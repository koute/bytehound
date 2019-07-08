#[macro_use]
extern crate log;

#[macro_use]
extern crate lazy_static;

#[cfg(feature = "sc")]
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
use std::path::{Path, PathBuf};
use std::fmt::Write as FmtWrite;
use std::collections::HashMap;

use std::os::unix::io::AsRawFd;
use std::os::unix::ffi::OsStrExt;

use std::io::{
    self,
    Write,
    Seek,
    SeekFrom
};

use common::speedy::{Writable, Readable, Endianness};

#[macro_use]
mod thread_local;
mod unwind;
mod timestamp;
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
mod writers;
mod writer_memory;
mod api;

use common::event::{DataId, Event, HeaderBody, AllocBody, HEADER_FLAG_IS_LITTLE_ENDIAN};
use common::lz4_stream::Lz4Writer;
use common::request::{
    PROTOCOL_VERSION,
    Request,
    Response,
    BroadcastHeader
};
use common::get_local_ips;

use crate::timestamp::{Timestamp, get_timestamp, get_wall_clock};
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

#[global_allocator]
static mut ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;

pub(crate) const PAGE_SIZE: usize = 4096;

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

static TRACING_ENABLED: AtomicBool = AtomicBool::new( false );

pub(crate) static ON_APPLICATION_THREAD_DEFAULT: SpinLock< bool > = SpinLock::new( false );

fn generate_data_id() -> DataId {
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
}

pub(crate) enum InternalEvent {
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

    fn set_file( &mut self, fp: File, path: PathBuf ) {
        self.file = Some( (path, fp) );
    }

    fn is_none( &self ) -> bool {
        self.file.is_none() && self.clients.is_empty()
    }
}

fn poll_clients( id: DataId, initial_timestamp: Timestamp, poll_fds: &mut Vec< libc::pollfd >, output: &mut Lz4Writer< Output > ) {
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
                if let Err( error ) = client.start_streaming( id, initial_timestamp, &mut output.file ) {
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
        if let Some( (ref path, ref mut fp) ) = self.file {
            if let Err( error ) = fp.write_all( data ) {
                warn!( "Write to {:?} failed: {}", path, error );
                self.file = None;
            }
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
        if let Some( (ref path, ref mut fp) ) = self.file {
            if let Err( error ) = fp.flush() {
                warn!( "Flush of {:?} failed: {}", path, error );
                self.file = None;
            }
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
    fn new( id: DataId, initial_timestamp: Timestamp, listener_port: u16, stream: TcpStream ) -> io::Result< Self > {
        let mut client = Client {
            stream,
            running: true,
            streaming: false
        };

        Response::Start( broadcast_header( id, initial_timestamp, listener_port ) ).write_to_stream( Endianness::LittleEndian, &mut client.stream )?;
        Ok( client )
    }

    fn stream_initial_data( &mut self, id: DataId, initial_timestamp: Timestamp, path: &Path, file: &mut File ) -> io::Result< () > {
        if !opt::get().write_binaries_to_output {
            info!( "Streaming the binaries which were suppressed in the original output file..." );
            let mut serializer = Lz4Writer::new( &mut *self );
            writers::write_header( id, initial_timestamp, &mut serializer )?;
            writers::write_binaries( &mut serializer )?;
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

    fn start_streaming( &mut self, id: DataId, initial_timestamp: Timestamp, output: &mut Option< (PathBuf, File) > ) -> io::Result< () > {
        // First client which connects to us gets streamed all of the data
        // which we've gathered up until this point.

        if let Some( (path, mut fp) ) = output.take() {
            match self.stream_initial_data( id, initial_timestamp, &path, &mut fp ) {
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
            writers::write_header( id, initial_timestamp, &mut serializer )?;
            writers::write_maps( &mut serializer )?;
            writers::write_binaries( &mut serializer )?;
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

pub(crate) fn new_header_body( id: DataId, initial_timestamp: Timestamp ) -> io::Result< HeaderBody > {
    let (timestamp, wall_clock_secs, wall_clock_nsecs) = get_wall_clock();

    let mut flags = 0;
    if arch::IS_LITTLE_ENDIAN {
        flags |= HEADER_FLAG_IS_LITTLE_ENDIAN;
    }

    Ok( HeaderBody {
        id,
        initial_timestamp,
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

fn broadcast_header( id: DataId, initial_timestamp: Timestamp, listener_port: u16 ) -> BroadcastHeader {
    let (timestamp, wall_clock_secs, wall_clock_nsecs) = get_wall_clock();

    BroadcastHeader {
        id,
        initial_timestamp,
        timestamp,
        wall_clock_secs,
        wall_clock_nsecs,
        pid: *PID,
        listener_port,
        cmdline: CMDLINE.clone(),
        executable: EXECUTABLE.clone(),
        arch: arch::TARGET_ARCH.to_string(),
        protocol_version: PROTOCOL_VERSION
    }
}

fn create_listener() -> io::Result< TcpListener > {
    let mut listener: io::Result< TcpListener >;
    let mut port = opt::get().base_server_port;
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


fn send_broadcast_to( id: DataId, initial_timestamp: Timestamp, listener_port: u16, target: IpAddr ) -> Result< (), io::Error > {
    let socket = UdpSocket::bind( SocketAddr::new( target, 0 ) )?;
    socket.set_broadcast( true )?;

    let mut message = Vec::new();
    broadcast_header( id, initial_timestamp, listener_port ).write_to_stream( Endianness::LittleEndian, &mut message ).unwrap();

    socket.send_to( &message, "255.255.255.255:43512" )?;
    Ok(())
}

fn send_broadcast( id: DataId, initial_timestamp: Timestamp, listener_port: u16 ) -> Result< (), io::Error > {
    use std::iter::once;
    use std::net::Ipv4Addr;

    let wildcard: IpAddr = Ipv4Addr::new( 0, 0, 0, 0 ).into();
    let mut output = Ok(());
    for ip in get_local_ips().into_iter().chain( once( wildcard ) ) {
        let result = send_broadcast_to( id, initial_timestamp, listener_port, ip );
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

fn initialize_output_file() -> Option< (File, PathBuf) > {
    let output_path = generate_filename( &opt::get().output_path_pattern );
    if output_path == "" {
        return None;
    }

    let fp = {
        let _handle = temporarily_change_umask( 0o777 );
        fs::OpenOptions::new()
            .read( true )
            .write( true )
            .create( true )
            .truncate( true )
            .mode( 0o777 )
            .open( &output_path )
    };

    let fp = match fp {
        Ok( fp ) => fp,
        Err( error ) => {
            error!( "Couldn't open '{}' for writing: {}", output_path, error );
            return None;
        }
    };

    // In the unlikely case of a race condition when setting the umask.
    let _ = fp.set_permissions( fs::Permissions::from_mode( 0o777 ) );

    info!( "File '{}' opened for writing", output_path );
    if let Some( uid ) = opt::get().chown_output_to {
        let gid = unsafe { libc::getgid() };
        let errcode = unsafe { libc::fchown( fp.as_raw_fd(), uid, gid ) };
        if errcode != 0 {
            let err = io::Error::last_os_error();
            warn!( "Couldn't chown '{}' to {}: {}", output_path, uid, err );
        } else {
            info!( "File '{}' was chown'd to {}", output_path, uid );
        }
    }

    Some( (fp, output_path.into()) )
}

fn thread_main() {
    assert!( !get_tls().unwrap().on_application_thread );

    info!( "Starting event thread..." );

    let uuid = generate_data_id();
    let initial_timestamp = get_timestamp();
    info!( "Data ID: {}", uuid );

    let mut output_writer = Lz4Writer::new( Output::new() );
    if let Some( (fp, path) ) = initialize_output_file() {
        let mut fp = Lz4Writer::new( fp );
        match writers::write_initial_data( uuid, initial_timestamp, &mut fp ) {
            Ok(()) => {
                let fp = fp.into_inner().unwrap();

                let mut output = Output::new();
                output.set_file( fp, path );
                output_writer.replace_inner( output ).unwrap();
            },
            Err( error ) => {
                warn!( "Failed to write initial data: {}", error );
            }
        }
    }

    let mut listener = create_listener();
    let listener_port = listener.as_ref().ok().and_then( |listener| listener.local_addr().ok() ).map( |addr| addr.port() ).unwrap_or( 0 );

    let mut events = Vec::new();
    let send_broadcasts = opt::get().enable_broadcasts;
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
                let _ = send_broadcast( uuid, initial_timestamp, listener_port );
            }

            if let Ok( ref mut listener ) = listener {
                match listener.accept() {
                    Ok( (stream, _) ) => {
                        match Client::new( uuid, initial_timestamp, listener_port, stream ) {
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

            poll_clients( uuid, initial_timestamp, &mut poll_fds, &mut output_writer );
        }

        if events.is_empty() && !running {
            break;
        }

        if events.is_empty() {
            if let Some( _lock ) = allocation_lock_for_memory_dump.take() {
                if !output_writer.inner().is_none() {
                    let _ = writer_memory::write_memory_dump( &mut output_writer );
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
                    if let Ok( backtrace ) = writers::write_backtrace( &mut *serializer, thread, backtrace, &mut next_backtrace_id ) {
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
                        let _ = event.write_to_stream( Endianness::LittleEndian, &mut *serializer );
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
                    if let Ok( backtrace ) = writers::write_backtrace( &mut *serializer, thread, backtrace, &mut next_backtrace_id ) {
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
                        let _ = event.write_to_stream( Endianness::LittleEndian, &mut *serializer );
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
                    if let Ok( backtrace ) = writers::write_backtrace( &mut *serializer, thread, backtrace, &mut next_backtrace_id ) {
                        mem::drop( throttle );
                        let event = Event::Free { timestamp, pointer: ptr as u64, backtrace, thread };
                        let _ = event.write_to_stream( Endianness::LittleEndian, &mut *serializer );
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
                    if let Ok( backtrace ) = writers::write_backtrace( &mut *serializer, thread, backtrace, &mut next_backtrace_id ) {
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

                        let _ = event.write_to_stream( Endianness::LittleEndian, &mut *serializer );
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
                    if let Ok( backtrace ) = writers::write_backtrace( &mut *serializer, thread, backtrace, &mut next_backtrace_id ) {
                        mem::drop( throttle );
                        let event = Event::MemoryUnmap { timestamp, pointer: ptr as u64, length: len as u64, backtrace, thread };
                        let _ = event.write_to_stream( Endianness::LittleEndian, &mut *serializer );
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
                    if let Ok( backtrace ) = writers::write_backtrace( &mut *serializer, thread, backtrace, &mut next_backtrace_id ) {
                        mem::drop( throttle );
                        let event = Event::Mallopt { timestamp, param, value, result, backtrace, thread };
                        let _ = event.write_to_stream( Endianness::LittleEndian, &mut *serializer );
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
                    let _ = event.write_to_stream( Endianness::LittleEndian, &mut *serializer );
                },
                InternalEvent::OverrideNextTimestamp { timestamp } => {
                    timestamp_override = Some( timestamp );
                },
                InternalEvent::Stop => {
                    stopped = true;
                    let _ = serializer.flush();
                }
            }
        }

        if (coarse_timestamp - last_flush_timestamp).as_secs() > 30 {
            last_flush_timestamp = get_timestamp();
            let _ = serializer.flush();
        }
    }

    let _ = output_writer.flush();
    info!( "Event thread finished" );
}

fn is_tracing_enabled() -> bool {
    TRACING_ENABLED.load( Ordering::Relaxed )
}

pub(crate) fn send_event( event: InternalEvent ) {
    EVENT_CHANNEL.send( event );
}

#[inline(always)]
pub(crate) fn send_event_throttled< F: FnOnce() -> InternalEvent >( callback: F ) {
    EVENT_CHANNEL.chunked_send_with( 64, callback );
}

static RUNNING: AtomicBool = AtomicBool::new( true );

pub(crate) extern fn on_exit() {
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

fn initialize_logger() {
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
}

fn initialize_atexit_hook() {
    info!( "Setting atexit hook..." );
    unsafe {
        let result = libc::atexit( on_exit );
        if result != 0 {
            error!( "Cannot set the at-exit hook" );
        }
    }
}

fn initialize_processing_thread() {
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
}

fn initialize_signal_handlers() {
    extern "C" fn sigusr_handler( _: libc::c_int ) {
        let value = !TRACING_ENABLED.load( Ordering::SeqCst );
        if value {
            info!( "Enabling tracing in response to SIGUSR" );
        } else {
            info!( "Disabling tracing in response to SIGUSR" );
        }

        TRACING_ENABLED.store( value, Ordering::SeqCst );
    }

    if opt::get().register_sigusr1 {
        info!( "Registering SIGUSR1 handler..." );
        unsafe {
            libc::signal( libc::SIGUSR1, sigusr_handler as libc::sighandler_t );
        }
    }

    if opt::get().register_sigusr2 {
        info!( "Registering SIGUSR2 handler..." );
        unsafe {
            libc::signal( libc::SIGUSR2, sigusr_handler as libc::sighandler_t );
        }
    }
}

#[inline(never)]
fn initialize() {
    static FLAG: AtomicBool = AtomicBool::new( false );
    if FLAG.compare_and_swap( false, true, Ordering::SeqCst ) == true {
        return;
    }

    assert!( !get_tls().unwrap().on_application_thread );

    initialize_logger();
    info!( "Initializing..." );

    unsafe {
        opt::initialize();
    }

    initialize_atexit_hook();
    initialize_processing_thread();

    TRACING_ENABLED.store( !opt::get().disabled_by_default, Ordering::SeqCst );
    initialize_signal_handlers();

    *ON_APPLICATION_THREAD_DEFAULT.lock() = true;
    info!( "Initialization done!" );

    get_tls().unwrap().on_application_thread = true;

    env::remove_var( "LD_PRELOAD" );
}

static THROTTLE_LIMIT: usize = 8192;

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
        let current_thread_id = syscall::gettid();
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
pub(crate) fn acquire_lock() -> Option< (RecursionLock< 'static >, ThrottleHandle) > {
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

#[cfg(not(test))]
pub use crate::api::{
    sys_mmap,
    sys_munmap,

    _exit,
    fork,

    malloc,
    calloc,
    realloc,
    free,
    posix_memalign,
    mmap,
    munmap,
    mallopt,
    memalign,
    aligned_alloc,
    valloc,
    pvalloc,

    memory_profiler_set_marker,
    memory_profiler_override_next_timestamp,
    memory_profiler_stop
};
