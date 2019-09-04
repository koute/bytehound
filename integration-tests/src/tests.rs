use {
    serde_derive::{
        Deserialize
    },
    std::{
        ffi::{
            OsString
        },
        path::{
            Path,
            PathBuf
        },
        sync::{
            atomic::{
                AtomicUsize,
                Ordering
            }
        },
        thread,
        time::{
            Duration,
            Instant
        }
    },
    crate::{
        utils::*
    }
};

fn repository_root() -> PathBuf {
    Path::new( env!( "CARGO_MANIFEST_DIR" ) ).join( ".." ).canonicalize().unwrap()
}

fn preload_path() -> PathBuf {
    let path = if let Ok( path ) = std::env::var( "MEMORY_PROFILER_TEST_PRELOAD_PATH" ) {
        repository_root().join( "target" ).join( path ).join( "libmemory_profiler.so" )
    } else {
        repository_root().join( "target" ).join( "x86_64-unknown-linux-gnu" ).join( "release" ).join( "libmemory_profiler.so" )
    };

    assert!( path.exists(), "{:?} doesn't exist", path );
    path
}

fn cli_path() -> PathBuf {
    repository_root().join( "target" ).join( "x86_64-unknown-linux-gnu" ).join( "release" ).join( "memory-profiler-cli" )
}

#[derive(Deserialize)]
struct ResponseMetadata {
    pub id: String,
    pub executable: String,
    pub architecture: String
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize, Debug, Hash)]
#[serde(transparent)]
pub struct Secs( u64 );

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize, Debug, Hash)]
#[serde(transparent)]
pub struct FractNanos( u32 );

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize, Debug)]
pub struct Timeval {
    pub secs: Secs,
    pub fract_nsecs: FractNanos
}

#[derive(PartialEq, Deserialize, Debug)]
pub struct Deallocation {
    pub timestamp: Timeval,
    pub thread: u32
}

#[derive(PartialEq, Deserialize, Debug)]
pub struct Frame {
    pub address: u64,
    pub address_s: String,
    pub count: u64,
    pub library: Option< String >,
    pub function: Option< String >,
    pub raw_function: Option< String >,
    pub source: Option< String >,
    pub line: Option< u32 >,
    pub column: Option< u32 >,
    pub is_inline: bool
}

#[derive(PartialEq, Deserialize, Debug)]
pub struct Allocation {
    pub address: u64,
    pub address_s: String,
    pub timestamp: Timeval,
    pub timestamp_relative: Timeval,
    pub timestamp_relative_p: f32,
    pub thread: u32,
    pub size: u64,
    pub backtrace_id: u32,
    pub deallocation: Option< Deallocation >,
    pub backtrace: Vec< Frame >,
    pub is_mmaped: bool,
    pub in_main_arena: bool,
    pub extra_space: u32
}

#[derive(Deserialize, Debug)]
struct ResponseAllocations {
    pub allocations: Vec< Allocation >,
    pub total_count: u64
}

struct Analysis {
    response: ResponseAllocations
}

fn is_from_source( alloc: &Allocation, expected: &str ) -> bool {
    alloc.backtrace.iter().any( |frame| {
        frame.source.as_ref().map( |source| {
            source.ends_with( expected )
        }).unwrap_or( false )
    })
}

fn is_from_function( alloc: &Allocation, expected: &str ) -> bool {
    alloc.backtrace.iter().any( |frame| {
        frame.raw_function.as_ref().map( |symbol| {
            symbol == expected
        }).unwrap_or( false )
    })
}

impl Analysis {
    fn allocations_from_source< 'a >( &'a self, source: &'a str ) -> impl Iterator< Item = &Allocation > + 'a {
        self.response.allocations.iter().filter( move |alloc| is_from_source( alloc, source ) )
    }
}

fn analyze( name: &str, path: impl AsRef< Path > ) -> Analysis {
    let cwd = repository_root().join( "target" );

    let path = path.as_ref();
    assert_file_exists( path );

    static PORT: AtomicUsize = AtomicUsize::new( 8080 );
    let port = PORT.fetch_add( 1, Ordering::SeqCst );

    let _child = run_in_the_background(
        &cwd,
        cli_path(),
        &[OsString::from( "server" ), path.as_os_str().to_owned(), OsString::from( "--port" ), OsString::from( format!( "{}", port ) )],
        &[("RUST_LOG", "server_core=debug,cli_core=debug,actix_net=info")]
    );

    let start = Instant::now();
    let mut found = false;
    while start.elapsed() < Duration::from_secs( 10 ) {
        thread::sleep( Duration::from_millis( 100 ) );
        if let Some( response ) = attohttpc::get( &format!( "http://localhost:{}/list", port ) ).send().ok() {
            assert_eq!( response.status(), attohttpc::StatusCode::OK );
            assert_eq!( *response.headers().get( attohttpc::header::CONTENT_TYPE ).unwrap(), "application/json" );
            let list: Vec< ResponseMetadata > = serde_json::from_str( &response.text().unwrap() ).unwrap();
            if !list.is_empty() {
                assert_eq!( list[ 0 ].executable.split( "/" ).last().unwrap(), name );
                found = true;
                break;
            }
        }
    }

    assert!( found );

    let response = attohttpc::get( &format!( "http://localhost:{}/data/last/allocations", port ) ).send().unwrap();
    assert_eq!( response.status(), attohttpc::StatusCode::OK );
    assert_eq!( *response.headers().get( attohttpc::header::CONTENT_TYPE ).unwrap(), "application/json" );
    let response: ResponseAllocations = serde_json::from_str( &response.text().unwrap() ).unwrap();

    Analysis { response }
}

fn get_basename( path: &str ) -> &str {
    let index_slash = path.rfind( "/" ).map( |index| index + 1 ).unwrap_or( 0 );
    let index_dot = path.rfind( "." ).unwrap();
    &path[ index_slash..index_dot ]
}

fn compile_with_flags( source: &str, extra_flags: &[&str] ) {
    let cwd = repository_root().join( "target" );
    let basename = get_basename( source );
    let source_path = PathBuf::from( "../integration-tests/test-programs" ).join( source );
    let source_path = source_path.into_os_string().into_string().unwrap();

    let mut args: Vec< &str > = Vec::new();
    if !source.ends_with( ".c" ) {
        args.push( "-std=c++11" );
    }

    args.extend( &[
        "-fasynchronous-unwind-tables",
        "-O0",
        "-pthread",
        "-ggdb3",
        &source_path,
        "-o",
        basename
    ]);

    args.extend( extra_flags );
    if source.ends_with( ".c" ) {
        run(
            &cwd,
            "gcc",
            &args,
            EMPTY_ENV
        ).assert_success();
    } else {
        run(
            &cwd,
            "g++",
            &args,
            EMPTY_ENV
        ).assert_success();
    }
}

fn compile( source: &str ) {
    compile_with_flags( source, &[] );
}

#[test]
fn test_basic() {
    let cwd = repository_root().join( "target" );

    compile( "basic.c" );

    run(
        &cwd,
        "./basic",
        EMPTY_ARGS,
        &[
            ("LD_PRELOAD", preload_path().into_os_string()),
            ("MEMORY_PROFILER_LOG", "debug".into()),
            ("MEMORY_PROFILER_OUTPUT", "memory-profiling-basic.dat".into())
        ]
    ).assert_success();

    let analysis = analyze( "basic", cwd.join( "memory-profiling-basic.dat" ) );
    let mut iter = analysis.allocations_from_source( "basic.c" );

    let a0 = iter.next().unwrap(); // malloc, leaked
    let a1 = iter.next().unwrap(); // malloc, freed
    let a2 = iter.next().unwrap(); // malloc, freed through realloc
    let a3 = iter.next().unwrap(); // realloc
    let a4 = iter.next().unwrap(); // calloc, freed
    let a5 = iter.next().unwrap(); // posix_memalign, leaked

    assert!( a0.deallocation.is_none() );
    assert!( a1.deallocation.is_some() );
    assert!( a2.deallocation.is_some() );
    assert!( a3.deallocation.is_none() );
    assert!( a4.deallocation.is_none() );
    assert!( a5.deallocation.is_none() );

    assert_eq!( a5.address % 65536, 0 );

    assert!( a0.size < a1.size );
    assert!( a1.size < a2.size );
    assert!( a2.size < a3.size );
    assert!( a3.size < a4.size );
    assert!( a4.size < a5.size );

    assert_eq!( a0.thread, a1.thread );
    assert_eq!( a1.thread, a2.thread );
    assert_eq!( a2.thread, a3.thread );
    assert_eq!( a3.thread, a4.thread );
    assert_eq!( a4.thread, a5.thread );

    assert_eq!( a0.backtrace.last().unwrap().line.unwrap() + 1, a1.backtrace.last().unwrap().line.unwrap() );

    assert_eq!( iter.next(), None );
}

#[test]
fn test_alloc_in_tls() {
    let cwd = repository_root().join( "target" );

    compile( "alloc-in-tls.cpp" );

    run(
        &cwd,
        "./alloc-in-tls",
        EMPTY_ARGS,
        &[
            ("LD_PRELOAD", preload_path().into_os_string()),
            ("MEMORY_PROFILER_LOG", "debug".into()),
            ("MEMORY_PROFILER_OUTPUT", "memory-profiling-alloc-in-tls.dat".into())
        ]
    ).assert_success();

    assert_file_exists( cwd.join( "memory-profiling-alloc-in-tls.dat" ) );
}

#[test]
fn test_start_stop() {
    let cwd = repository_root().join( "target" );

    compile( "start-stop.c" );

    run(
        &cwd,
        "./start-stop",
        EMPTY_ARGS,
        &[
            ("LD_PRELOAD", preload_path().into_os_string()),
            ("MEMORY_PROFILER_LOG", "debug".into()),
            ("MEMORY_PROFILER_OUTPUT", "start-stop_%n.dat".into()),
            ("MEMORY_PROFILER_DISABLE_BY_DEFAULT", "1".into())
        ]
    ).assert_success();

    let analysis_1 = analyze( "start-stop", cwd.join( "start-stop_0.dat" ) );
    let analysis_2 = analyze( "start-stop", cwd.join( "start-stop_1.dat" ) );

    {
        let mut iter = analysis_1.allocations_from_source( "start-stop.c" );
        let a0 = iter.next().unwrap();
        let a1 = iter.next().unwrap();
        let a2 = iter.next().unwrap();

        assert_eq!( a0.size, 10002 );
        assert_eq!( a1.size, 20002 );
        assert_eq!( a2.size, 10003 );

        assert_eq!( a0.thread, a2.thread );
        assert_ne!( a0.thread, a1.thread );

        assert_eq!( iter.next(), None );
    }

    {
        let mut iter = analysis_2.allocations_from_source( "start-stop.c" );
        let a0 = iter.next().unwrap();
        let a1 = iter.next().unwrap();

        assert_eq!( a0.size, 10004 );
        assert_eq!( a1.size, 20003 );

        assert_ne!( a0.thread, a1.thread );

        assert_eq!( iter.next(), None );
    }
}

#[test]
fn test_fork() {
    let cwd = repository_root().join( "target" );

    compile( "fork.c" );

    run(
        &cwd,
        "./fork",
        EMPTY_ARGS,
        &[
            ("LD_PRELOAD", preload_path().into_os_string()),
            ("MEMORY_PROFILER_LOG", "debug".into()),
            ("MEMORY_PROFILER_OUTPUT", "fork_%n.dat".into())
        ]
    ).assert_success();

    assert_file_exists( cwd.join( "fork_0.dat" ) );
    assert_file_missing( cwd.join( "fork_1.dat" ) );

    let analysis = analyze( "fork", cwd.join( "fork_0.dat" ) );

    let mut iter = analysis.allocations_from_source( "fork.c" ).filter( |alloc| {
        !is_from_function( alloc, "allocate_dtv" ) &&
        !is_from_function( alloc, "_dl_allocate_tls" )
    });
    let a0 = iter.next().unwrap();
    let a1 = iter.next().unwrap();
    let a2 = iter.next().unwrap();
    let a3 = iter.next().unwrap();
    let a4 = iter.next().unwrap();

    assert_eq!( a0.size, 10001 );
    assert_eq!( a1.size, 20001 );
    assert_eq!( a2.size, 10002 );
    assert_eq!( a3.size, 20002 );
    assert_eq!( a4.size, 10003 );

    assert_eq!( a0.thread, a2.thread );
    assert_eq!( a2.thread, a4.thread );
    assert_eq!( a1.thread, a3.thread );
    assert_ne!( a0.thread, a1.thread );

    assert_eq!( iter.next(), None );
}

#[test]
fn test_normal_exit() {
    let cwd = repository_root().join( "target" );

    compile( "exit_1.c" );

    run(
        &cwd,
        "./exit_1",
        EMPTY_ARGS,
        &[
            ("LD_PRELOAD", preload_path().into_os_string()),
            ("MEMORY_PROFILER_LOG", "debug".into()),
            ("MEMORY_PROFILER_OUTPUT", "exit_1.dat".into())
        ]
    ).assert_success();

    let analysis = analyze( "exit_1", cwd.join( "exit_1.dat" ) );

    let mut iter = analysis.allocations_from_source( "exit_1.c" );
    let a0 = iter.next().unwrap();
    assert_eq!( a0.size, 11001 );
    assert_eq!( iter.next(), None );
}

#[test]
fn test_immediate_exit_unistd() {
    let cwd = repository_root().join( "target" );

    compile( "exit_2.c" );

    run(
        &cwd,
        "./exit_2",
        EMPTY_ARGS,
        &[
            ("LD_PRELOAD", preload_path().into_os_string()),
            ("MEMORY_PROFILER_LOG", "debug".into()),
            ("MEMORY_PROFILER_OUTPUT", "exit_2.dat".into())
        ]
    ).assert_success();

    let analysis = analyze( "exit_2", cwd.join( "exit_2.dat" ) );

    let mut iter = analysis.allocations_from_source( "exit_2.c" );
    let a0 = iter.next().unwrap();
    assert_eq!( a0.size, 12001 );
    assert_eq!( iter.next(), None );
}

#[test]
fn test_immediate_exit_stdlib() {
    let cwd = repository_root().join( "target" );

    compile( "exit_3.c" );

    run(
        &cwd,
        "./exit_3",
        EMPTY_ARGS,
        &[
            ("LD_PRELOAD", preload_path().into_os_string()),
            ("MEMORY_PROFILER_LOG", "debug".into()),
            ("MEMORY_PROFILER_OUTPUT", "exit_3.dat".into())
        ]
    ).assert_success();

    let analysis = analyze( "exit_3", cwd.join( "exit_3.dat" ) );

    let mut iter = analysis.allocations_from_source( "exit_3.c" );
    let a0 = iter.next().unwrap();
    assert_eq!( a0.size, 13001 );
    assert_eq!( iter.next(), None );
}

struct GatherTestHandle< 'a > {
    pid: u32,
    is_graceful: &'a mut bool
}

impl< 'a > GatherTestHandle< 'a > {
    fn kill( self ) {
        *self.is_graceful = false;
        unsafe { libc::kill( self.pid as _, libc::SIGUSR2 ); }
    }

    fn early_return( self ) {
        unsafe { libc::kill( self.pid as _, libc::SIGINT ); }
    }

    fn next( &self ) {
        unsafe { libc::kill( self.pid as _, libc::SIGUSR1 ); }
    }

    fn sleep( &self ) {
        thread::sleep( Duration::from_millis( 1000 ) );
    }
}

fn test_gather_generic( expected_allocations: usize, callback: impl FnOnce( GatherTestHandle ) ) {
    let cwd = repository_root().join( "target" );

    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once( || compile( "gather.c" ) );

    static PORT: AtomicUsize = AtomicUsize::new( 8100 );
    let port = PORT.fetch_add( 1, Ordering::SeqCst );

    let child = run_in_the_background(
        &cwd,
        "./gather",
        EMPTY_ARGS,
        &[
            ("LD_PRELOAD", preload_path().into_os_string()),
            ("MEMORY_PROFILER_LOG", "debug".into()),
            ("MEMORY_PROFILER_OUTPUT", format!( "gather_{}.dat", port ).into()),
            ("MEMORY_PROFILER_REGISTER_SIGUSR1", "0".into()),
            ("MEMORY_PROFILER_REGISTER_SIGUSR2", "0".into()),
            ("MEMORY_PROFILER_ENABLE_SERVER", "1".into()),
            ("MEMORY_PROFILER_BASE_SERVER_PORT", format!( "{}", port ).into())
        ]
    );

    let timestamp = Instant::now();
    while timestamp.elapsed() < Duration::from_secs( 10 ) {
        if std::net::TcpStream::connect( format!( "127.0.0.1:{}", port ) ).is_ok() {
            break;
        }
        thread::sleep( Duration::from_millis( 100 ) );
    }

    let tmp_path = cwd.join( "tmp" ).join( format!( "test-gather-{}", port ) );
    if tmp_path.exists() {
        std::fs::remove_dir_all( &tmp_path ).unwrap();
    }

    std::fs::create_dir_all( &tmp_path ).unwrap();

    let gather = run_in_the_background(
        &tmp_path,
        cli_path(),
        &[OsString::from( "gather" ), OsString::from( format!( "127.0.0.1:{}", port ) )],
        &[("RUST_LOG", "server_core=debug,cli_core=debug,actix_net=info")]
    );

    thread::sleep( Duration::from_millis( 1000 ) );

    let mut is_graceful = true;
    let handle = GatherTestHandle {
        pid: child.pid(),
        is_graceful: &mut is_graceful
    };

    callback( handle );

    if is_graceful {
        child.wait().assert_success();
    } else {
        child.wait().assert_failure();
    }

    gather.wait().assert_success();

    let outputs = dir_entries( tmp_path ).unwrap();
    assert_eq!( outputs.len(), 1, "Unexpected outputs: {:?}", outputs );

    let analysis = analyze( "gather", outputs.into_iter().next().unwrap() );
    let mut iter = analysis.allocations_from_source( "gather.c" );

    assert!( expected_allocations >= 1 && expected_allocations <= 3 );
    if expected_allocations >= 1 {
        let a0 = iter.next().expect( "Expected at least one allocation; got none" );
        assert_eq!( a0.size, 10001 );
    }

    if expected_allocations >= 2 {
        let a1 = iter.next().expect( "Expected at least two allocations; got only one" );
        assert_eq!( a1.size, 10002 );
    }

    if expected_allocations >= 3 {
        let a2 = iter.next().expect( "Expected at least three allocations; got only two" );
        assert_eq!( a2.size, 10003 );
    }

    assert_eq!( iter.next(), None, "Too many allocations" );
}

#[test]
fn test_gather_full_graceful() {
    test_gather_generic( 3, |handle| {
        handle.next();
        handle.sleep();
        handle.next();
        handle.early_return();
    });
}

#[test]
fn test_gather_initial_graceful() {
    test_gather_generic( 1, |handle| {
        handle.early_return();
    });
}

#[test]
fn test_gather_initial_killed() {
    test_gather_generic( 1, |handle| {
        handle.kill();
    });
}

#[test]
fn test_gather_partial_graceful() {
    test_gather_generic( 2, |handle| {
        handle.next();
        handle.early_return();
    });
}

#[test]
fn test_gather_partial_killed() {
    test_gather_generic( 1, |handle| {
        handle.next();
        handle.sleep();
        handle.kill();
    });
}

#[test]
fn test_dlopen() {
    let cwd = repository_root().join( "target" );
    compile_with_flags( "dlopen.c", &[ "-ldl" ] );
    compile_with_flags( "dlopen_so.c", &[ "-shared" ] );

    run(
        &cwd,
        "./dlopen",
        EMPTY_ARGS,
        &[
            ("LD_PRELOAD", preload_path().into_os_string()),
            ("MEMORY_PROFILER_LOG", "debug".into()),
            ("MEMORY_PROFILER_OUTPUT", "dlopen.dat".into())
        ]
    ).assert_success();

    let analysis = analyze( "dlopen", cwd.join( "dlopen.dat" ) );
    assert!( analysis.response.allocations.iter().any( |alloc| alloc.size == 123123 ) );

    let mut iter = analysis.allocations_from_source( "dlopen_so.c" );
    let a0 = iter.next().unwrap();
    assert_eq!( a0.size, 123123 );
    assert_eq!( iter.next(), None );
}
