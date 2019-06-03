#[macro_use]
extern crate log;

#[macro_use]
extern crate lazy_static;

#[macro_use]
extern crate sc;

use std::ptr;
use std::mem;
use std::thread;
use std::env;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::fs::{self, File, remove_file, read_link};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::net::{TcpListener, TcpStream, UdpSocket, IpAddr, SocketAddr};
use std::time::Duration;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::path::{Path, PathBuf};
use std::cmp::min;
use std::ffi::CStr;
use std::fmt::Write as FmtWrite;
use std::collections::{HashMap, HashSet};

use std::os::unix::io::AsRawFd;
use std::os::unix::ffi::OsStrExt;

use std::io::{
    self,
    Read,
    Write,
    Seek,
    SeekFrom
};

use libc::{
    c_void,
    c_int,
    size_t,
    off_t
};

use common::speedy::{Writable, Readable, Endianness};

mod unwind;
mod timestamp;
#[macro_use]
mod spin_lock;
mod channel;
mod utils;
mod arch;
mod logger;
mod opt;
mod syscall;
mod raw_file;
mod arc_counter;
mod tls;

use common::event::{self, DataId, Event, HeaderBody, AllocBody, FramesInvalidated, HEADER_FLAG_IS_LITTLE_ENDIAN};
use common::lz4_stream::Lz4Writer;
use common::request::{
    PROTOCOL_VERSION,
    Request,
    Response,
    BroadcastHeader
};
use common::get_local_ips;

use nwind::proc_maps::Region;
use nwind::proc_maps::parse as parse_maps;

use crate::timestamp::{Timestamp, get_timestamp};
use crate::unwind::Backtrace;
use crate::channel::Channel;
use crate::utils::{
    read_file,
    copy,
    temporarily_change_umask
};
use crate::spin_lock::{SpinLock, SpinLockGuard};
use crate::arc_counter::ArcCounter;
use crate::tls::{Tls, get_tls};

extern "C" {
    #[link_name = "__libc_malloc"]
    fn malloc_real( size: size_t ) -> *mut c_void;
    #[link_name = "__libc_calloc"]
    fn calloc_real( count: size_t, element_size: size_t ) -> *mut c_void;
    #[link_name = "__libc_realloc"]
    fn realloc_real( ptr: *mut c_void, size: size_t ) -> *mut c_void;
    #[link_name = "__libc_free"]
    fn free_real( ptr: *mut c_void );
    #[link_name = "__libc_memalign"]
    fn memalign_real( alignment: size_t, size: size_t ) -> *mut c_void;
    #[link_name = "__libc_mallopt"]
    fn mallopt_real( params: c_int, value: c_int ) -> c_int;

    #[link_name = "__libc_fork"]
    fn fork_real() -> libc::pid_t;
}

#[global_allocator]
static mut ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;

const PAGE_SIZE: usize = 4096;

use std::hash::Hash;
fn get_hash< T: Hash >( value: T ) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::Hasher;
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

lazy_static! {
    static ref EVENT_CHANNEL: Channel< InternalEvent > = Channel::new();
    static ref INITIAL_TIMESTAMP: Timestamp = get_timestamp();
    static ref UUID: DataId = {
        let pid = *PID;
        let cmdline = &*CMDLINE;
        let executable = &*EXECUTABLE;

        let mut timespec = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0
        };

        unsafe {
            libc::clock_gettime( libc::CLOCK_REALTIME, &mut timespec );
        }

        let a = get_hash( &pid as *const _ as usize ) ^ get_hash( pid ) ^ get_hash( timespec.tv_sec );
        let b = get_hash( cmdline ) ^ get_hash( executable ) ^ get_hash( timespec.tv_nsec );

        DataId::new( a, b )
    };
    static ref PID: u32 = {
        let pid = unsafe { libc::getpid() } as u32;
        pid
    };
    static ref CMDLINE: Vec< u8 > = {
        read_file( "/proc/self/cmdline" ).unwrap()
    };
    static ref EXECUTABLE: Vec< u8 > = {
        let executable: Vec< u8 > = read_link( "/proc/self/exe" ).unwrap().as_os_str().as_bytes().into();
        executable
    };
}

static LISTENER_PORT: AtomicUsize = AtomicUsize::new( 0 );
static TRACING_ENABLED: AtomicBool = AtomicBool::new( false );

pub(crate) static ON_APPLICATION_THREAD_DEFAULT: SpinLock< bool > = spin_lock_new!( false );

fn get_timestamp_if_enabled() -> Timestamp {
    if opt::precise_timestamps() {
        get_timestamp()
    } else {
        Timestamp::min()
    }
}

enum InternalEvent {
    Alloc {
        ptr: usize,
        size: usize,
        backtrace: Backtrace,
        thread: u32,
        flags: u32,
        extra_usable_space: u32,
        preceding_free_space: u64,
        timestamp: Timestamp,
        throttle: ThrottleHandle
    },
    Realloc {
        old_ptr: usize,
        new_ptr: usize,
        size: usize,
        backtrace: Backtrace,
        thread: u32,
        flags: u32,
        extra_usable_space: u32,
        preceding_free_space: u64,
        timestamp: Timestamp,
        throttle: ThrottleHandle
    },
    Free {
        ptr: usize,
        backtrace: Backtrace,
        thread: u32,
        timestamp: Timestamp,
        throttle: ThrottleHandle
    },
    Exit,
    GrabMemoryDump,
    SetMarker {
        value: u32
    },
    Mmap {
        pointer: usize,
        requested_address: usize,
        length: usize,
        mmap_protection: u32,
        mmap_flags: u32,
        offset: u64,
        backtrace: Backtrace,
        thread: u32,
        file_descriptor: u32,
        timestamp: Timestamp,
        throttle: ThrottleHandle
    },
    Munmap {
        ptr: usize,
        len: usize,
        backtrace: Backtrace,
        thread: u32,
        timestamp: Timestamp,
        throttle: ThrottleHandle
    },
    Mallopt {
        param: i32,
        value: i32,
        result: i32,
        backtrace: Backtrace,
        thread: u32,
        timestamp: Timestamp,
        throttle: ThrottleHandle
    },
    OverrideNextTimestamp {
        timestamp: Timestamp
    },
    Stop
}

struct Output {
    file: Option< (PathBuf, File) >,
    clients: Vec< Client >
}

impl Output {
    fn new() -> Self {
        Output {
            file: None,
            clients: Vec::new()
        }
    }

    fn new_file< P: Into< PathBuf > >( path: P ) -> io::Result< Output > {
        let path: PathBuf = path.into();
        let fp = {
            let _handle = temporarily_change_umask( 0o777 );
            fs::OpenOptions::new()
                .read( true )
                .write( true )
                .create( true )
                .truncate( true )
                .mode( 0o777 )
                .open( &path )?
        };

        // In the unlikely case of a race condition when setting the umask.
        let _ = fp.set_permissions( fs::Permissions::from_mode( 0o777 ) );

        Ok( Output { file: Some( (path, fp) ), clients: Vec::new() } )
    }

    fn get_file( &mut self ) -> Option< &mut File > {
        self.file.as_mut().map( |(_, file)| file )
    }

    fn is_none( &self ) -> bool {
        self.file.is_none() && self.clients.is_empty()
    }
}

fn poll_clients( poll_fds: &mut Vec< libc::pollfd >, output: &mut Lz4Writer< Output > ) {
    poll_fds.clear();

    for client in output.inner().clients.iter() {
        poll_fds.push( libc::pollfd {
            fd: client.stream.as_raw_fd(),
            events: libc::POLLIN | libc::POLLHUP,
            revents: 0
        });
    }

    let ok = unsafe { libc::poll( poll_fds.as_ptr() as *mut _, poll_fds.len() as _, 0 ) };
    if ok == -1 {
        let err = io::Error::last_os_error();
        if err.kind() != io::ErrorKind::Interrupted {
            error!( "Poll failed: {}", err );
            return;
        }
    }

    for (index, poll_fd) in poll_fds.iter().enumerate() {
        let pollin = poll_fd.revents & libc::POLLIN != 0;
        let pollhup = poll_fd.revents & libc::POLLHUP != 0;

        let client = &mut output.inner_mut_without_flush().clients[ index ];
        if pollhup {
            info!( "A client was disconnected" );
            client.running = false;
            continue;
        }

        if !pollin {
            continue;
        }

        trace!( "Reading a client's request..." );
        let request = match Request::read_from_stream( Endianness::LittleEndian, &mut client.stream ) {
            Ok( request ) => request,
            Err( error ) => {
                info!( "Failed to read a client request: {}", error );
                client.running = false;
                continue;
            }
        };
        trace!( "Finished reading the request from client" );

        match request {
            Request::StartStreaming => {
                let output = &mut output.inner_mut().unwrap();
                let client = &mut output.clients[ index ];
                if let Err( error ) = client.start_streaming( &mut output.file ) {
                    info!( "Failed to start streaming to a client: {}", error );
                    client.running = false;
                } else {
                    client.streaming = true;
                }
            },
            Request::TriggerMemoryDump => {
                debug!( "Received a TriggerMemoryDump request" );
                send_event( InternalEvent::GrabMemoryDump );
            },
            Request::Ping => {
                trace!( "Received a Ping request" );
                if let Err( error ) = Response::Pong.write_to_stream( Endianness::LittleEndian, &mut client.stream ) {
                    info!( "Failed to respond to a client ping: {}", error );
                    client.running = false;
                }
            }
        }
    }

    output.inner_mut_without_flush().clients.retain( |client| client.running );
}

impl io::Write for Output {
    fn write( &mut self, data: &[u8] ) -> io::Result< usize > {
        if let Some( (_, ref mut fp) ) = self.file {
            fp.write_all( data )?
        }

        for mut client in self.clients.iter_mut() {
            if !client.running || !client.streaming {
                continue;
            }

            let result = client.write_all( data );
            if let Err( error ) = result {
                client.running = false;
                warn!( "Write to client failed: {}", error );
            }
        }

        Ok( data.len() )
    }

    fn flush( &mut self ) -> io::Result< () > {
        if let Some( (_, ref mut fp) ) = self.file {
            fp.flush()?
        }

        Ok(())
    }
}

impl< 'a > io::Write for &'a mut Client {
    fn write( &mut self, data: &[u8] ) -> io::Result< usize > {
        let length = data.len();
        let response = Response::Data( data.into() );
        response.write_to_stream( Endianness::LittleEndian, &mut self.stream ).map( |_| length )
    }

    fn flush( &mut self ) -> io::Result< () > {
        self.stream.flush()
    }
}

struct Client {
    stream: TcpStream,
    running: bool,
    streaming: bool
}

impl Client {
    fn new( stream: TcpStream ) -> io::Result< Self > {
        let mut client = Client {
            stream,
            running: true,
            streaming: false
        };

        Response::Start( broadcast_header() ).write_to_stream( Endianness::LittleEndian, &mut client.stream )?;
        Ok( client )
    }

    fn stream_initial_data( &mut self, path: &Path, file: &mut File ) -> io::Result< () > {
        if !opt::should_write_binaries_to_output() {
            info!( "Streaming the binaries which were suppressed in the original output file..." );
            let mut serializer = Lz4Writer::new( &mut *self );
            write_header( &mut serializer )?;
            write_binaries( &mut serializer )?;
            serializer.flush()?;
        }

        info!( "Streaming initial data..." );
        file.seek( SeekFrom::Start( 0 ) )?;
        copy( file, &mut *self )?;

        Response::FinishedInitialStreaming.write_to_stream( Endianness::LittleEndian, &mut self.stream )?;

        if let Err( error ) = remove_file( &path ) {
            warn!( "Failed to remove {:?}: {}", path, error );
        }

        info!( "Finished streaming initial data" );
        Ok(())
    }

    fn start_streaming( &mut self, output: &mut Option< (PathBuf, File) > ) -> io::Result< () > {
        // First client which connects to us gets streamed all of the data
        // which we've gathered up until this point.

        if let Some( (path, mut fp) ) = output.take() {
            match self.stream_initial_data( &path, &mut fp ) {
                Ok(()) => return Ok(()),
                Err( error ) => {
                    fp.seek( SeekFrom::End( 0 ) )?;
                    *output = Some( (path, fp) );
                    return Err( error );
                }
            }
        }

        {
            let mut serializer = Lz4Writer::new( &mut *self );
            write_header( &mut serializer )?;
            write_maps( &mut serializer )?;
            write_binaries( &mut serializer )?;
            serializer.flush()?;
        }

        Response::FinishedInitialStreaming
            .write_to_stream( Endianness::LittleEndian, &mut self.stream )?;

        Ok(())
    }
}

impl Drop for Client {
    fn drop( &mut self ) {
        info!( "Removing client..." );
    }
}

fn read_maps() -> io::Result< Vec< Region > > {
    let maps = read_file( "/proc/self/maps" )?;
    let maps_str = String::from_utf8_lossy( &maps );
    let regions = parse_maps( &maps_str );
    Ok( regions )
}

fn mmap_file< P: AsRef< Path >, R, F: FnOnce( &[u8] ) -> R >( path: P, callback: F ) -> io::Result< R > {
    let fp = File::open( &path )?;
    let mmap = unsafe { memmap::Mmap::map( &fp ) }?;
    let slice = mmap.deref();
    Ok( callback( slice ) )
}

fn write_file< U: Write >( mut serializer: &mut U, path: &str, bytes: &[u8] ) -> io::Result< () > {
    Event::File {
        timestamp: get_timestamp(),
        path: path.into(),
        contents: bytes.into()
    }.write_to_stream( Endianness::LittleEndian, &mut serializer )
}

fn write_binaries< U: Write >( mut serializer: &mut U ) -> io::Result< () > {
    let regions = read_maps()?;
    let mut files = HashSet::new();
    for region in regions {
        if region.is_shared || !region.is_executable || region.name.is_empty() {
            continue;
        }

        if region.name == "[heap]" || region.name == "[stack]" || region.name == "[vdso]" {
            continue;
        }

        if files.contains( &region.name ) {
            continue;
        }

        files.insert( region.name );
    }

    serializer.flush()?;
    for filename in files {
        debug!( "Writing '{}'...", filename );
        match mmap_file( &filename, |bytes| write_file( &mut serializer, &filename, bytes ) ) {
            Ok( result ) => {
                result?
            },
            Err( error ) => {
                debug!( "Failed to mmap '{}': {}", filename, error );
            }
        }
    }

    Ok(())
}

fn write_maps< U: Write >( serializer: &mut U ) -> io::Result< Vec< u8 > > {
    let maps = read_file( "/proc/self/maps" )?;
    Event::File { timestamp: get_timestamp(), path: "/proc/self/maps".into(), contents: maps.clone().into() }.write_to_stream( Endianness::LittleEndian, serializer )?;
    Ok( maps )
}

fn new_header_body() -> io::Result< HeaderBody > {
    let id = *UUID;
    let (timestamp, wall_clock_secs, wall_clock_nsecs) = get_wall_clock();

    let mut flags = 0;
    if arch::IS_LITTLE_ENDIAN {
        flags |= HEADER_FLAG_IS_LITTLE_ENDIAN;
    }

    Ok( HeaderBody {
        id,
        initial_timestamp: *INITIAL_TIMESTAMP,
        timestamp,
        wall_clock_secs,
        wall_clock_nsecs,
        pid: *PID,
        cmdline: CMDLINE.clone(),
        executable: EXECUTABLE.clone(),
        arch: arch::TARGET_ARCH.to_string(),
        flags,
        pointer_size: mem::size_of::< usize >() as u8
    })
}

fn broadcast_header() -> BroadcastHeader {
    let id = *UUID;
    let (timestamp, wall_clock_secs, wall_clock_nsecs) = get_wall_clock();

    BroadcastHeader {
        id,
        initial_timestamp: *INITIAL_TIMESTAMP,
        timestamp,
        wall_clock_secs,
        wall_clock_nsecs,
        pid: *PID,
        listener_port: LISTENER_PORT.load( Ordering::SeqCst ) as u16,
        cmdline: CMDLINE.clone(),
        executable: EXECUTABLE.clone(),
        arch: arch::TARGET_ARCH.to_string(),
        protocol_version: PROTOCOL_VERSION
    }
}

fn write_header< U: Write >( serializer: &mut U ) -> io::Result< () > {
    Event::Header( new_header_body()? ).write_to_stream( Endianness::LittleEndian, serializer )
}

fn get_wall_clock() -> (Timestamp, u64, u64) {
    let timestamp = get_timestamp();
    let mut timespec = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0
    };

    unsafe {
        libc::clock_gettime( libc::CLOCK_REALTIME, &mut timespec );
    }

    (timestamp, timespec.tv_sec as u64, timespec.tv_nsec as u64)
}

fn write_wallclock< U: Write >( serializer: &mut U ) -> io::Result< () > {
    let (timestamp, sec, nsec) = get_wall_clock();
    Event::WallClock { timestamp, sec, nsec }.write_to_stream( Endianness::LittleEndian, serializer )
}

fn write_uptime< U: Write >( serializer: &mut U ) -> io::Result< () > {
    let uptime = fs::read( "/proc/uptime" )?;
    write_file( serializer, "/proc/uptime", &uptime )
}

extern "C" {
    static environ: *const *const libc::c_char;
}

fn write_environ< U: Write >( mut serializer: U ) -> io::Result< () > {
    unsafe {
        let mut ptr = environ;
        while !(*ptr).is_null() {
            let string = CStr::from_ptr( *ptr );
            Event::Environ {
                entry: string.to_bytes().into()
            }.write_to_stream( Endianness::LittleEndian, &mut serializer )?;

            ptr = ptr.offset( 1 );
        }
    }

    Ok(())
}

fn write_backtrace< U: Write >( serializer: &mut U, thread: u32, backtrace: Backtrace, next_backtrace_id: &mut u64 ) -> io::Result< u64 > {
    if backtrace.is_empty() {
        return Ok( 0 );
    }

    let id = *next_backtrace_id;
    *next_backtrace_id = id + 1;

    let frames_invalidated = match backtrace.stale_count {
        None => FramesInvalidated::All,
        Some( value ) => FramesInvalidated::Some( value )
    };

    if mem::size_of::< usize >() == mem::size_of::< u32 >() {
        let frames: &[usize] = backtrace.frames.as_slice();
        let frames: &[u32] = unsafe { std::slice::from_raw_parts( frames.as_ptr() as *const u32, frames.len() ) };
        Event::PartialBacktrace32 {
            id,
            thread,
            frames_invalidated,
            addresses: frames.into()
        }.write_to_stream( Endianness::LittleEndian, serializer )?;
    } else if mem::size_of::< usize >() == mem::size_of::< u64 >() {
        let frames: &[usize] = backtrace.frames.as_slice();
        let frames: &[u64] = unsafe { std::slice::from_raw_parts( frames.as_ptr() as *const u64, frames.len() ) };
        Event::PartialBacktrace {
            id,
            thread,
            frames_invalidated,
            addresses: frames.into()
        }.write_to_stream( Endianness::LittleEndian, serializer )?;
    } else {
        unreachable!();
    }

    Ok( id )
}

fn is_accessible< U: Read + Seek >( mut fp: U, address: u64 ) -> bool {
    if let Err( _ ) = fp.seek( SeekFrom::Start( address ) ) {
        return false;
    }

    let mut dummy: [u8; 1] = [0];
    match fp.read( &mut dummy ) {
        Ok( 1 ) => true,
        _ => false
    }
}

fn memory_dump_body< U: Write >( mut serializer: &mut U ) -> io::Result< () > {
    let mut buffer = Vec::new();
    buffer.resize( 1024 * 128, 0 );
    let mut buffer = buffer.into_boxed_slice();
    let maps = write_maps( serializer )?;
    let maps = String::from_utf8_lossy( &maps );
    let maps = parse_maps( &maps );
    let mut fp = File::open( "/proc/self/mem" )?;
    let page_size = PAGE_SIZE as u64;


    for region in maps {
        if !region.is_write && region.inode != 0 {
            continue;
        }

        let mut end = {
            let total_length = (region.end - region.start) / page_size;

            let mut start = 0;
            let mut end = total_length;
            loop {
                if start == end {
                    break;
                }

                let current = start + (end - start) / 2;
                let accessible = is_accessible( &mut fp, region.start + current * page_size + page_size - 1 );
                if !accessible {
                    end = current;
                } else {
                    start = current + 1;
                }
            }

            region.start + end * page_size
        };

        loop {
            let chunk_size = min( buffer.len() as u64, end - region.start );
            if chunk_size == 0 {
                break;
            }

            let address = end - chunk_size;
            fp.seek( SeekFrom::Start( address ) )?;
            fp.read_exact( &mut buffer[ 0..chunk_size as usize ] )?;
            let data = &buffer[ 0..chunk_size as usize ];
            Event::MemoryDump {
                address,
                length: chunk_size as u64,
                data: data.into()
            }.write_to_stream( Endianness::LittleEndian, &mut serializer )?;

            end -= chunk_size;
        }

/*
        let mut page: [u8; 4096] = [0; 4096];
        while address > region.start {
            fp.seek( SeekFrom::Start( address - page_size ) )?;
            fp.read_exact( &mut page )?;

            if page.iter().all( |&byte| byte == 0 ) {
                address -= page_size;
            } else {
                break;
            }
        }

        fp.seek( SeekFrom::Start( region.start ) )?;
        let mut current = region.start;

        while current < address {
            let chunk_size = min( buffer.len(), (address - current) as usize );
            fp.read_exact( &mut buffer[ 0..chunk_size ] )?;
            let data = &buffer[ 0..chunk_size ];
            Event::MemoryDump {
                address: current,
                length: chunk_size as u64,
                data
            }.write_to_stream( LittleEndian, serializer )?;
            current += chunk_size as u64;
        }
*/
    }

    serializer.flush()?;
    Ok(())
}

fn write_memory_dump< U: Write >( serializer: &mut U ) -> io::Result< () > {
    info!( "Writing a memory dump..." );
    serializer.flush()?;

    assert!( !get_tls().unwrap().on_application_thread );

    let pid = unsafe { libc::fork() };
    if pid == 0 {
        let result = memory_dump_body( serializer );
        unsafe {
            syscall!( EXIT, if result.is_err() { 1 } else { 0 } );
        }
    } else {
        info!( "Waiting for child to finish..." );
        unsafe {
            libc::waitpid( pid, ptr::null_mut(), 0 );
        }
    }

    info!( "Memory dump finished" );
    Ok(())
}

fn write_included_files< U: Write >( serializer: &mut U ) -> io::Result< () > {
    let pattern = match env::var( "MEMORY_PROFILER_INCLUDE_FILE" ) {
        Ok( pattern ) => pattern,
        Err( _ ) => return Ok(())
    };

    info!( "Will write any files matching the pattern: {:?}", pattern );
    match glob::glob( &pattern ) {
        Ok( paths ) => {
            let mut any = false;
            for path in paths {
                any = true;
                let path = match path {
                    Ok( path ) => path,
                    Err( _ ) => continue
                };

                info!( "Writing file: {:?}...", path );
                match mmap_file( &path, |bytes| write_file( serializer, &path.to_string_lossy(), bytes ) ) {
                    Ok( result ) => {
                        result?;
                    },
                    Err( error ) => {
                        warn!( "Failed to read {:?}: {}", path, error );
                        continue;
                    }
                }

                serializer.flush()?;
            }

            if !any {
                info!( "No files matched the pattern!" );
            }
        },
        Err( error ) => {
            error!( "Glob of {:?} failed: {}", pattern, error );
        }
    }

    Ok(())
}

fn create_listener() -> io::Result< TcpListener > {
    let mut listener: io::Result< TcpListener >;
    let mut port = opt::base_broadcast_port();
    loop {
        listener = TcpListener::bind( format!( "0.0.0.0:{}", port ) );
        if listener.is_ok() {
            info!( "Created a TCP listener on port {}", port );
            break;
        }

        port += 1;
        if port > port + 100 {
            error!( "Failed to create a TCP listener" );
            break;
        }
    }

    let listener = listener?;
    listener.set_nonblocking( true )?;
    Ok( listener )
}


fn send_broadcast_to( target: IpAddr ) -> Result< (), io::Error > {
    let socket = UdpSocket::bind( SocketAddr::new( target, 0 ) )?;
    socket.set_broadcast( true )?;

    let mut message = Vec::new();
    broadcast_header().write_to_stream( Endianness::LittleEndian, &mut message ).unwrap();

    socket.send_to( &message, "255.255.255.255:43512" )?;
    Ok(())
}

fn send_broadcast() -> Result< (), io::Error > {
    use std::iter::once;
    use std::net::{Ipv4Addr, IpAddr};

    let wildcard: IpAddr = Ipv4Addr::new( 0, 0, 0, 0 ).into();
    let mut output = Ok(());
    for ip in get_local_ips().into_iter().chain( once( wildcard ) ) {
        let result = send_broadcast_to( ip );
        if result.is_err() {
            output = result;
        }
    }

    output
}

fn generate_filename( pattern: &str ) -> String {
    let mut output = String::new();
    let mut seen_percent = false;
    for ch in pattern.chars() {
        if !seen_percent && ch == '%' {
            seen_percent = true;
            continue;
        }

        if seen_percent {
            seen_percent = false;
            match ch {
                '%' => {
                    output.push( ch );
                },
                'p' => {
                    let pid = *PID;
                    write!( &mut output, "{}", pid ).unwrap();
                },
                't' => {
                    let timestamp = unsafe { libc::time( ptr::null_mut() ) };
                    write!( &mut output, "{}", timestamp ).unwrap();
                },
                'e' => {
                    let executable = String::from_utf8_lossy( &*EXECUTABLE );
                    let executable = &executable[ executable.rfind( "/" ).map( |index| index + 1 ).unwrap_or( 0 ).. ];
                    write!( &mut output, "{}", executable ).unwrap();
                },
                _ => {}
            }
        } else {
            output.push( ch );
        }
    }

    output
}

fn thread_main() {
    assert!( !get_tls().unwrap().on_application_thread );

    info!( "Starting event thread..." );
    let mut output_writer = Lz4Writer::new( Output::new() );
    let mut saved_error: Option< io::Error > = None;

    macro_rules! save_error {
        ($e:expr) => {
            if saved_error.is_none() {
                match $e {
                    Ok( value ) => Some( value ),
                    Err( error ) => {
                        saved_error = Some( error );
                        None
                    }
                }
            } else {
                None
            }
        }
    }

    let mut events = Vec::new();

    let output_path;
    if let Ok( path ) = env::var( "MEMORY_PROFILER_OUTPUT" ) {
        output_path = generate_filename( &path );
    } else {
        output_path = generate_filename( "memory-profiling_%e_%t_%p.dat" );
    };

    let send_broadcasts = opt::are_broadcasts_enabled();
    if output_path != "" {
        if let Some( mut output ) = save_error!( Output::new_file( &output_path ) ) {
            info!( "File '{}' opened for writing", output_path );
            if let Some( uid ) = opt::chown_output_to() {
                let gid = unsafe { libc::getgid() };
                let errcode = unsafe { libc::fchown( output.get_file().unwrap().as_raw_fd(), uid, gid ) };
                if errcode != 0 {
                    let err = io::Error::last_os_error();
                    warn!( "Couldn't chown '{}' to {}: {}", output_path, uid, err );
                } else {
                    info!( "File '{}' was chown'd to {}", output_path, uid );
                }
            }

            output_writer.replace_inner( output ).unwrap();
        } else {
            error!( "Couldn't open '{}' for writing", output_path );
        }
    }

    let mut listener = create_listener();
    LISTENER_PORT.store( listener.as_ref().ok().and_then( |listener| listener.local_addr().ok() ).map( |addr| addr.port() ).unwrap_or( 0 ) as usize, Ordering::SeqCst );

    {
        let serializer = &mut output_writer;

        if !serializer.inner().is_none() {
            info!( "Writing initial header..." );
            save_error!( write_header( &mut *serializer ) );

            info!( "Writing wall clock..." );
            save_error!( write_wallclock( &mut *serializer ) );

            info!( "Writing uptime..." );
            save_error!( write_uptime( &mut *serializer ) );

            save_error!( write_included_files( &mut *serializer ) );

            info!( "Writing environ..." );
            save_error!( write_environ( &mut *serializer ) );

            info!( "Writing maps..." );
            save_error!( write_maps( &mut *serializer ) );
            save_error!( serializer.flush() );

            if opt::should_write_binaries_to_output() {
                info!( "Writing binaries..." );
                save_error!( write_binaries( &mut *serializer ) );
            }

            info!( "Flushing..." );
            save_error!( serializer.flush_and_reset_buffers() );
        }
    }

    let mut next_backtrace_id = 0;
    let mut last_flush_timestamp = get_timestamp();
    let mut coarse_timestamp = get_timestamp();
    let mut running = true;
    let mut allocation_lock_for_memory_dump = None;
    let mut last_broadcast = coarse_timestamp;
    let mut timestamp_override = None;
    let mut stopped = false;
    let mut poll_fds = Vec::new();
    'main_loop: loop {
        EVENT_CHANNEL.timed_recv_all( &mut events, Duration::from_millis( 250 ) );

        coarse_timestamp = get_timestamp();
        if (coarse_timestamp - last_broadcast).as_secs() >= 1 {
            last_broadcast = coarse_timestamp;
            if send_broadcasts {
                let _ = send_broadcast();
            }

            if let Ok( ref mut listener ) = listener {
                match listener.accept() {
                    Ok( (stream, _) ) => {
                        match Client::new( stream ) {
                            Ok( client ) => {
                                output_writer.inner_mut_without_flush().clients.push( client );
                            },
                            Err( error ) => {
                                info!( "Failed to initialize client: {}", error );
                            }
                        }
                    },
                    Err( ref error ) if error.kind() == io::ErrorKind::WouldBlock => {},
                    Err( _ ) => {}
                }
            }

            poll_clients( &mut poll_fds, &mut output_writer );
        }

        if events.is_empty() && !running {
            break;
        }

        if saved_error.is_some() {
            // We've encountered some errors; keep running, but drop all events.
            events.clear();
            allocation_lock_for_memory_dump = None;
            continue;
        }

        if events.is_empty() {
            if let Some( _lock ) = allocation_lock_for_memory_dump.take() {
                if !output_writer.inner().is_none() {
                    save_error!( write_memory_dump( &mut output_writer ) );
                }
            }
        }

        let serializer = &mut output_writer;
        let skip = stopped || serializer.inner().is_none();
        for event in events.drain(..) {
            match event {
                InternalEvent::Alloc { ptr, size, backtrace, thread, flags, extra_usable_space, preceding_free_space, mut timestamp, throttle } => {
                    if skip {
                        continue;
                    }

                    if timestamp == Timestamp::min() {
                        timestamp = coarse_timestamp;
                    }

                    timestamp = timestamp_override.take().unwrap_or( timestamp );
                    if let Some( backtrace ) = save_error!( write_backtrace( &mut *serializer, thread, backtrace, &mut next_backtrace_id ) ) {
                        mem::drop( throttle );
                        let event = Event::Alloc {
                            timestamp,
                            allocation: AllocBody {
                                pointer: ptr as u64,
                                size: size as u64,
                                backtrace,
                                thread,
                                flags,
                                extra_usable_space,
                                preceding_free_space
                            }
                        };
                        save_error!( event.write_to_stream( Endianness::LittleEndian, &mut *serializer ) );
                    }
                },
                InternalEvent::Realloc { old_ptr, new_ptr, size, backtrace, thread, flags, extra_usable_space, preceding_free_space, mut timestamp, throttle } => {
                    if skip {
                        continue;
                    }

                    if timestamp == Timestamp::min() {
                        timestamp = coarse_timestamp;
                    }

                    timestamp = timestamp_override.take().unwrap_or( timestamp );
                    if let Some( backtrace ) = save_error!( write_backtrace( &mut *serializer, thread, backtrace, &mut next_backtrace_id ) ) {
                        mem::drop( throttle );
                        let event = Event::Realloc {
                            timestamp, old_pointer: old_ptr as u64,
                            allocation: AllocBody {
                                pointer: new_ptr as u64,
                                size: size as u64,
                                backtrace, thread,
                                flags,
                                extra_usable_space,
                                preceding_free_space
                            }
                        };
                        save_error!( event.write_to_stream( Endianness::LittleEndian, &mut *serializer ) );
                    }
                },
                InternalEvent::Free { ptr, backtrace, thread, mut timestamp, throttle } => {
                    if skip {
                        continue;
                    }

                    if timestamp == Timestamp::min() {
                        timestamp = coarse_timestamp;
                    }

                    timestamp = timestamp_override.take().unwrap_or( timestamp );
                    if let Some( backtrace ) = save_error!( write_backtrace( &mut *serializer, thread, backtrace, &mut next_backtrace_id ) ) {
                        mem::drop( throttle );
                        let event = Event::Free { timestamp, pointer: ptr as u64, backtrace, thread };
                        save_error!( event.write_to_stream( Endianness::LittleEndian, &mut *serializer ) );
                    }
                },
                InternalEvent::Mmap { pointer, length, backtrace, thread, requested_address, mmap_protection, mmap_flags, file_descriptor, offset, mut timestamp, throttle } => {
                    if skip {
                        continue;
                    }

                    if timestamp == Timestamp::min() {
                        timestamp = coarse_timestamp;
                    }

                    timestamp = timestamp_override.take().unwrap_or( timestamp );
                    if let Some( backtrace ) = save_error!( write_backtrace( &mut *serializer, thread, backtrace, &mut next_backtrace_id ) ) {
                        mem::drop( throttle );
                        let event = Event::MemoryMap {
                            timestamp,
                            pointer: pointer as u64,
                            length: length as u64,
                            backtrace,
                            thread,
                            requested_address: requested_address as u64,
                            mmap_protection,
                            mmap_flags,
                            file_descriptor,
                            offset
                        };

                        save_error!( event.write_to_stream( Endianness::LittleEndian, &mut *serializer ) );
                    }
                },
                InternalEvent::Munmap { ptr, len, backtrace, thread, mut timestamp, throttle } => {
                    if skip {
                        continue;
                    }

                    if timestamp == Timestamp::min() {
                        timestamp = coarse_timestamp;
                    }

                    let timestamp = timestamp_override.take().unwrap_or( timestamp );
                    if let Some( backtrace ) = save_error!( write_backtrace( &mut *serializer, thread, backtrace, &mut next_backtrace_id ) ) {
                        mem::drop( throttle );
                        let event = Event::MemoryUnmap { timestamp, pointer: ptr as u64, length: len as u64, backtrace, thread };
                        save_error!( event.write_to_stream( Endianness::LittleEndian, &mut *serializer ) );
                    }
                },
                InternalEvent::Mallopt { param, value, result, mut timestamp, backtrace, thread, throttle } => {
                    if skip {
                        continue;
                    }

                    if timestamp == Timestamp::min() {
                        timestamp = coarse_timestamp;
                    }

                    let timestamp = timestamp_override.take().unwrap_or( timestamp );
                    if let Some( backtrace ) = save_error!( write_backtrace( &mut *serializer, thread, backtrace, &mut next_backtrace_id ) ) {
                        mem::drop( throttle );
                        let event = Event::Mallopt { timestamp, param, value, result, backtrace, thread };
                        save_error!( event.write_to_stream( Endianness::LittleEndian, &mut *serializer ) );
                    }
                },
                InternalEvent::Exit => {
                    running = false;
                },
                InternalEvent::GrabMemoryDump => {
                    // Block any further allocations.
                    if allocation_lock_for_memory_dump.is_none() {
                        debug!( "Locking allocations to prepare for a memory dump" );
                        allocation_lock_for_memory_dump = Some( AllocationLock::new() );
                    }
                },
                InternalEvent::SetMarker { value } => {
                    if skip {
                        continue;
                    }

                    let event = Event::Marker { value };
                    save_error!( event.write_to_stream( Endianness::LittleEndian, &mut *serializer ) );
                },
                InternalEvent::OverrideNextTimestamp { timestamp } => {
                    timestamp_override = Some( timestamp );
                },
                InternalEvent::Stop => {
                    stopped = true;
                    save_error!( serializer.flush() );
                }
            }
        }

        if (coarse_timestamp - last_flush_timestamp).as_secs() > 30 {
            last_flush_timestamp = get_timestamp();
            save_error!( serializer.flush() );
        }
    }

    let _ = output_writer.flush();
    info!( "Event thread finished" );
}

fn is_tracing_enabled() -> bool {
    TRACING_ENABLED.load( Ordering::Relaxed )
}

fn send_event( event: InternalEvent ) {
    EVENT_CHANNEL.send( event );
}

#[inline(always)]
fn send_event_throttled< F: FnOnce() -> InternalEvent >( callback: F ) {
    EVENT_CHANNEL.chunked_send_with( 64, callback );
}

static RUNNING: AtomicBool = AtomicBool::new( true );

extern fn on_exit() {
    info!( "Exit hook called" );

    TRACING_ENABLED.store( false, Ordering::SeqCst );

    send_event( InternalEvent::Exit );
    let mut count = 0;
    while RUNNING.load( Ordering::SeqCst ) == true && count < 2000 {
        unsafe {
            libc::usleep( 25 * 1000 );
            count += 1;
        }
    }

    info!( "Exit hook finished" );
}

#[inline(never)]
fn initialize() {
    static FLAG: AtomicBool = AtomicBool::new( false );
    if FLAG.compare_and_swap( false, true, Ordering::SeqCst ) == true {
        return;
    }

    assert!( !get_tls().unwrap().on_application_thread );

    static mut SYSCALL_LOGGER: logger::SyscallLogger = logger::SyscallLogger::empty();
    static mut FILE_LOGGER: logger::FileLogger = logger::FileLogger::empty();
    let log_level = if let Ok( value ) = env::var( "MEMORY_PROFILER_LOG" ) {
        match value.as_str() {
            "trace" => log::LevelFilter::Trace,
            "debug" => log::LevelFilter::Debug,
            "info" => log::LevelFilter::Info,
            "warn" => log::LevelFilter::Warn,
            "error" => log::LevelFilter::Error,
            _ => log::LevelFilter::Off
        }
    } else {
        log::LevelFilter::Off
    };

    let pid = unsafe { libc::getpid() };

    if let Ok( value ) = env::var( "MEMORY_PROFILER_LOGFILE" ) {
        let path = generate_filename( &value );
        let rotate_at = env::var( "MEMORY_PROFILER_LOGFILE_ROTATE_WHEN_BIGGER_THAN" ).ok().and_then( |value| value.parse().ok() );

        unsafe {
            if let Ok(()) = FILE_LOGGER.initialize( path, rotate_at, log_level, pid ) {
                log::set_logger( &FILE_LOGGER ).unwrap();
            }
        }
    } else {
        unsafe {
            SYSCALL_LOGGER.initialize( log_level, pid );
            log::set_logger( &SYSCALL_LOGGER ).unwrap();
        }
    }

    log::set_max_level( log_level );

    info!( "Initializing..." );

    unsafe {
        crate::tls::initialize_tls();
    }

    let tracing_enabled =
        if let Ok( value ) = env::var( "MEMORY_PROFILER_DISABLE_BY_DEFAULT" ) {
            if value == "1" {
                info!( "Disabling tracing by default" );
                false
            } else {
                true
            }
        } else {
            true
        };

    opt::initialize();

    let _ = *INITIAL_TIMESTAMP;
    let _ = *UUID;

    info!( "Setting atexit hook..." );
    unsafe {
        let result = libc::atexit( on_exit );
        if result != 0 {
            error!( "Cannot set the at-exit hook" );
        }
    }

    info!( "Spawning main thread..." );
    let flag = Arc::new( SpinLock::new( false ) );
    let flag_clone = flag.clone();
    thread::Builder::new().name( "mem-prof".into() ).spawn( move || {
        assert!( !get_tls().unwrap().on_application_thread );

        *flag_clone.lock() = true;
        thread_main();
        RUNNING.store( false, Ordering::SeqCst );
    }).expect( "failed to start the main memory profiler thread" );

    while *flag.lock() == false {
        thread::yield_now();
    }

    info!( "Setting signal handler..." );
    extern "C" fn sigusr_handler( _: libc::c_int ) {
        let value = !TRACING_ENABLED.load( Ordering::SeqCst );
        if value {
            info!( "Enabling tracing in response to SIGUSR" );
        } else {
            info!( "Disabling tracing in response to SIGUSR" );
        }

        TRACING_ENABLED.store( value, Ordering::SeqCst );
    }

    TRACING_ENABLED.store( tracing_enabled, Ordering::SeqCst );
    unsafe {
        libc::signal( libc::SIGUSR1, sigusr_handler as libc::sighandler_t );
        libc::signal( libc::SIGUSR2, sigusr_handler as libc::sighandler_t );
    }

    *ON_APPLICATION_THREAD_DEFAULT.lock() = true;
    info!( "Initialization done!" );

    get_tls().unwrap().on_application_thread = true;

    env::remove_var( "LD_PRELOAD" );
}

#[cfg(target_arch = "arm")]
#[no_mangle]
pub unsafe extern "C" fn sys_mmap( addr: *mut c_void, length: size_t, prot: c_int, flags: c_int, fildes: c_int, off: off_t ) -> *mut c_void {
    syscall!( MMAP2, addr, length, prot, flags, fildes, off / (PAGE_SIZE as off_t) ) as *mut c_void
}

#[cfg(not(target_arch = "arm"))]
#[no_mangle]
pub unsafe extern "C" fn sys_mmap( addr: *mut c_void, length: size_t, prot: c_int, flags: c_int, fildes: c_int, off: off_t ) -> *mut c_void {
    syscall!( MMAP, addr, length, prot, flags, fildes, off ) as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn sys_munmap( addr: *mut c_void, length: size_t ) -> c_int {
    syscall!( MUNMAP, addr, length ) as c_int
}

#[no_mangle]
pub unsafe extern "C" fn _exit( status: c_int ) {
    on_exit();
    syscall!( EXIT, status );
}

static THROTTLE_LIMIT: usize = 4096;

struct ThrottleHandle( ArcCounter );
impl ThrottleHandle {
    fn new( tls: &Tls ) -> Self {
        let state = &tls.throttle_state;
        while state.get() >= THROTTLE_LIMIT {
            thread::yield_now();
        }

        ThrottleHandle( state.clone() )
    }
}

struct AllocationLock {
    current_thread_id: u32,
    throttle_for_thread_map: SpinLockGuard< 'static, Option< HashMap< u32, ArcCounter > > >
}

impl AllocationLock {
    fn new() -> Self {
        let mut throttle_for_thread_map = crate::tls::THROTTLE_FOR_THREAD.lock();
        let current_thread_id = utils::get_thread_id_raw();
        for (&thread_id, counter) in throttle_for_thread_map.as_mut().unwrap().iter_mut() {
            if thread_id == current_thread_id {
                continue;
            }

            unsafe {
                counter.add( THROTTLE_LIMIT );
            }
        }

        AllocationLock {
            current_thread_id,
            throttle_for_thread_map
        }
    }
}

impl Drop for AllocationLock {
    fn drop( &mut self ) {
        for (&thread_id, counter) in self.throttle_for_thread_map.as_mut().unwrap().iter_mut() {
            if thread_id == self.current_thread_id {
                continue;
            }

            unsafe {
                counter.sub( THROTTLE_LIMIT );
            }
        }
    }
}

struct RecursionLock< 'a > {
    tls: &'a mut Tls
}

impl< 'a > RecursionLock< 'a > {
    fn new( tls: &'a mut Tls ) -> Self {
        tls.on_application_thread = false;
        RecursionLock {
            tls
        }
    }
}

impl< 'a > Drop for RecursionLock< 'a > {
    fn drop( &mut self ) {
        self.tls.on_application_thread = true;
    }
}

impl< 'a > Deref for RecursionLock< 'a > {
    type Target = Tls;
    fn deref( &self ) -> &Self::Target {
        self.tls
    }
}

impl< 'a > DerefMut for RecursionLock< 'a > {
    fn deref_mut( &mut self ) -> &mut Self::Target {
        self.tls
    }
}

#[inline(always)]
fn acquire_lock() -> Option< (RecursionLock< 'static >, ThrottleHandle) > {
    let mut is_enabled = is_tracing_enabled();
    if !is_enabled {
        initialize();
        is_enabled = is_tracing_enabled();
        if !is_enabled {
            return None;
        }
    }

    let tls = get_tls()?;
    if !tls.on_application_thread {
        None
    } else {
        let throttle = ThrottleHandle::new( &tls );
        Some( (RecursionLock::new( tls ), throttle) )
    }
}

// `libc` on mips64 doesn't export this
extern "C" {
    fn malloc_usable_size( ptr: *mut libc::c_void) -> libc::size_t;
}

fn get_glibc_metadata( ptr: *mut c_void, size: usize ) -> (u32, u32, u64) {
    let raw_chunk_size = unsafe { *(ptr as *mut usize).offset( -1 ) };
    let flags = raw_chunk_size & 0b111;
    let chunk_size = raw_chunk_size & !0b111;

    let is_prev_in_use = flags & 1 != 0;
    let preceding_free_space = if !is_prev_in_use {
        unsafe { *(ptr as *mut usize).offset( -2 ) }
    } else {
        0
    };

    let is_mmapped = flags & 2 != 0;
    let extra_usable_space = chunk_size - size - mem::size_of::< usize >() * if is_mmapped { 2 } else { 1 };

    debug_assert_eq!(
        size + extra_usable_space,
        unsafe { malloc_usable_size( ptr ) },
        "chunk_size: {}, size: {}, malloc_usable_size: {}, extra_usable_space: {}",
        chunk_size,
        size,
        unsafe { malloc_usable_size( ptr ) },
        extra_usable_space,
    );

    (flags as u32, extra_usable_space as u32, preceding_free_space as u64)
}

#[inline(always)]
unsafe fn allocate( size: usize, is_calloc: bool ) -> *mut c_void {
    let lock = acquire_lock();
    let ptr =
        if is_calloc || opt::zero_memory() {
            calloc_real( size as size_t, 1 )
        } else {
            malloc_real( size as size_t )
        };

    if ptr.is_null() {
        return ptr;
    }

    let (mut tls, throttle) = if let Some( lock ) = lock { lock } else { return ptr };
    let mut backtrace = Backtrace::new();
    unwind::grab( &mut tls, &mut backtrace );

    let (mut flags, extra_usable_space, preceding_free_space) = get_glibc_metadata( ptr, size );
    if is_calloc {
        flags |= event::ALLOC_FLAG_CALLOC;
    }

    let thread = tls.thread_id;
    send_event_throttled( move || {
        InternalEvent::Alloc {
            ptr: ptr as usize,
            size: size as usize,
            backtrace,
            thread,
            flags,
            extra_usable_space,
            preceding_free_space,
            timestamp: get_timestamp_if_enabled(),
            throttle
        }
    });

    mem::drop( tls );
    ptr
}

#[cfg(not(test))]
#[no_mangle]
pub unsafe extern "C" fn malloc( size: size_t ) -> *mut c_void {
    allocate( size, false )
}

#[cfg(not(test))]
#[no_mangle]
pub unsafe extern "C" fn calloc( count: size_t, element_size: size_t ) -> *mut c_void {
    let size = match (count as usize).checked_mul( element_size as usize ) {
        None => return ptr::null_mut(),
        Some( size ) => size as size_t
    };

    allocate( size, true )
}

#[cfg(not(test))]
#[no_mangle]
pub unsafe extern "C" fn realloc( old_ptr: *mut c_void, size: size_t ) -> *mut c_void {
    if old_ptr.is_null() {
        return malloc( size );
    }

    if size == 0 {
        free( old_ptr );
        return ptr::null_mut();
    }

    let lock = acquire_lock();
    let new_ptr = realloc_real( old_ptr, size );

    let (mut tls, throttle) = if let Some( lock ) = lock { lock } else { return new_ptr };
    let mut backtrace = Backtrace::new();
    unwind::grab( &mut tls, &mut backtrace );

    let thread = tls.thread_id;
    let timestamp = get_timestamp_if_enabled();

    if !new_ptr.is_null() {
        let (flags, extra_usable_space, preceding_free_space) = get_glibc_metadata( new_ptr, size );
        send_event_throttled( move || {
            InternalEvent::Realloc {
                old_ptr: old_ptr as usize,
                new_ptr: new_ptr as usize,
                size: size as usize,
                backtrace,
                thread,
                flags,
                extra_usable_space,
                preceding_free_space,
                timestamp,
                throttle
            }
        });
    } else {
        send_event_throttled( || {
            InternalEvent::Free {
                ptr: old_ptr as usize,
                backtrace,
                thread,
                timestamp,
                throttle
            }
        });
    }

    mem::drop( tls );
    new_ptr
}

#[cfg(not(test))]
#[no_mangle]
pub unsafe extern "C" fn free( ptr: *mut c_void ) {
    if ptr.is_null() {
        return;
    }

    let lock = acquire_lock();
    free_real( ptr );

    let (mut tls, throttle) = if let Some( lock ) = lock { lock } else { return };
    let mut backtrace = Backtrace::new();
    if opt::grab_backtraces_on_free() {
        unwind::grab( &mut tls, &mut backtrace );
    }

    let thread = tls.thread_id;
    send_event_throttled( || {
        InternalEvent::Free {
            ptr: ptr as usize,
            backtrace,
            thread,
            timestamp: get_timestamp_if_enabled(),
            throttle
        }
    });

    mem::drop( tls );
}

#[cfg(not(test))]
#[no_mangle]
pub unsafe extern "C" fn posix_memalign( memptr: *mut *mut c_void, alignment: size_t, size: size_t ) -> c_int {
    if memptr.is_null() {
        return libc::EINVAL;
    }

    let ptr_size = mem::size_of::< *const c_void >();
    if alignment % ptr_size != 0 || !(alignment / ptr_size).is_power_of_two() || alignment == 0 {
        return libc::EINVAL;
    }

    let lock = acquire_lock();

    let pointer = memalign_real( alignment, size );
    *memptr = pointer;
    if pointer.is_null() {
        return libc::ENOMEM;
    }

    let (mut tls, throttle) = if let Some( lock ) = lock { lock } else { return 0 };
    let mut backtrace = Backtrace::new();
    unwind::grab( &mut tls, &mut backtrace );

    let (flags, extra_usable_space, preceding_free_space) = get_glibc_metadata( pointer, size );
    let thread = tls.thread_id;
    send_event_throttled( || {
        InternalEvent::Alloc {
            ptr: pointer as usize,
            size: size as usize,
            backtrace,
            thread,
            flags,
            extra_usable_space,
            preceding_free_space,
            timestamp: get_timestamp_if_enabled(),
            throttle
        }
    });

    mem::drop( tls );
    0
}

#[no_mangle]
pub unsafe extern "C" fn mmap( addr: *mut c_void, length: size_t, prot: c_int, flags: c_int, fildes: c_int, off: off_t ) -> *mut c_void {
    let lock = acquire_lock();

    let ptr = sys_mmap( addr, length, prot, flags, fildes, off );
    if ptr == libc::MAP_FAILED {
        return ptr;
    }

    let (mut tls, throttle) = if let Some( lock ) = lock { lock } else { return ptr };
    let mut backtrace = Backtrace::new();
    unwind::grab( &mut tls, &mut backtrace );

    let thread = tls.thread_id;
    send_event_throttled( || InternalEvent::Mmap {
        pointer: ptr as usize,
        length: length as usize,
        requested_address: addr as usize,
        mmap_protection: prot as u32,
        mmap_flags: flags as u32,
        file_descriptor: fildes as u32,
        offset: off as u64,
        backtrace,
        thread,
        timestamp: get_timestamp_if_enabled(),
        throttle
    });

    mem::drop( tls );
    ptr
}

#[no_mangle]
pub unsafe extern "C" fn munmap( ptr: *mut c_void, length: size_t ) -> c_int {
    let lock = acquire_lock();
    let result = sys_munmap( ptr, length );

    let (mut tls, throttle) = if let Some( lock ) = lock { lock } else { return result };
    let mut backtrace = Backtrace::new();
    unwind::grab( &mut tls, &mut backtrace );

    if !ptr.is_null() {
        let thread = tls.thread_id;
        send_event_throttled( || InternalEvent::Munmap {
            ptr: ptr as usize,
            len: length as usize,
            backtrace,
            thread,
            timestamp: get_timestamp_if_enabled(),
            throttle
        });
    }

    mem::drop( tls );
    result
}

#[no_mangle]
pub unsafe extern "C" fn mallopt( param: c_int, value: c_int ) -> c_int {
    let lock = acquire_lock();
    let result = mallopt_real( param, value );

    let (mut tls, throttle) = if let Some( lock ) = lock { lock } else { return result };
    let mut backtrace = Backtrace::new();
    unwind::grab( &mut tls, &mut backtrace );

    let thread = tls.thread_id;
    send_event_throttled( || InternalEvent::Mallopt {
        param: param as i32,
        value: value as i32,
        result: result as i32,
        backtrace,
        thread,
        timestamp: get_timestamp_if_enabled(),
        throttle
    });

    mem::drop( tls );
    result
}

#[no_mangle]
pub unsafe extern "C" fn fork() -> libc::pid_t {
    let pid = fork_real();
    if pid == 0 {
        let mut tls = get_tls();
        let tls = tls.as_mut().unwrap();
        tls.on_application_thread = false;
        *ON_APPLICATION_THREAD_DEFAULT.lock() = false;
    } else {
        info!( "Fork called; child PID: {}", pid );
    }

    pid
}

#[cfg(not(test))]
#[no_mangle]
pub unsafe extern "C" fn memalign( _alignment: size_t, _size: size_t ) -> *mut c_void {
    unimplemented!( "'memalign' is unimplemented!" );
}

#[cfg(not(test))]
#[no_mangle]
pub unsafe extern "C" fn aligned_alloc( _alignment: size_t, _size: size_t ) -> *mut c_void {
    unimplemented!( "'aligned_alloc' is unimplemented!" );
}

#[cfg(not(test))]
#[no_mangle]
pub unsafe extern "C" fn valloc( _size: size_t ) -> *mut c_void {
    unimplemented!( "'valloc' is unimplemented!" );
}

#[cfg(not(test))]
#[no_mangle]
pub unsafe extern "C" fn pvalloc( _size: size_t ) -> *mut c_void {
    unimplemented!( "'pvalloc' is unimplemented!" );
}

#[no_mangle]
pub unsafe extern "C" fn memory_profiler_set_marker( value: u32 ) {
    let lock = acquire_lock();
    send_event( InternalEvent::SetMarker {
        value
    });

    mem::drop( lock );
}

#[no_mangle]
pub unsafe extern "C" fn memory_profiler_override_next_timestamp( timestamp: u64 ) {
    let lock = acquire_lock();
    send_event_throttled( || InternalEvent::OverrideNextTimestamp {
        timestamp: Timestamp::from_usecs( timestamp )
    });
    mem::drop( lock );
}

#[no_mangle]
pub unsafe extern "C" fn memory_profiler_stop() {
    let lock = acquire_lock();
    send_event( InternalEvent::Stop );
    mem::drop( lock );
}
