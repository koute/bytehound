use {
    serde_derive::{
        Deserialize
    },
    std::{
        ffi::{
            OsString,
            OsStr
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

fn target() -> Option< String > {
    std::env::var( "MEMORY_PROFILER_TEST_TARGET" ).ok()
}

fn build_root() -> PathBuf {
    if let Some( path ) = std::env::var_os( "CARGO_TARGET_DIR" ) {
        let path: PathBuf = path.into();
        if path.is_absolute() {
            path.canonicalize().unwrap()
        } else {
            repository_root().join( path ).canonicalize().unwrap()
        }
    } else {
        repository_root().join( "target" )
    }
}

fn preload_path() -> PathBuf {
    let path = if let Ok( path ) = std::env::var( "MEMORY_PROFILER_TEST_PRELOAD_PATH" ) {
        build_root().join( path ).join( "libbytehound.so" )
    } else {
        let target = match target() {
            Some( target ) => target,
            None => "x86_64-unknown-linux-gnu".to_owned()
        };

        let mut potential_paths = vec![
            build_root().join( &target ).join( "debug" ).join( "libbytehound.so" ),
            build_root().join( &target ).join( "release" ).join( "libbytehound.so" )
        ];

        if target == env!( "TARGET" ) {
            potential_paths.push( build_root().join( "debug" ).join( "libbytehound.so" ) );
            potential_paths.push( build_root().join( "release" ).join( "libbytehound.so" ) );
        }

        potential_paths.retain( |path| path.exists() );
        if potential_paths.is_empty() {
            panic!( "No libbytehound.so found!" );
        }

        if potential_paths.len() > 1 {
            panic!( "Multiple libbytehound.so found; specify the one which you want to use for tests with MEMORY_PROFILER_TEST_PRELOAD_PATH!" );
        }

        potential_paths.pop().unwrap()
    };

    assert!( path.exists(), "{:?} doesn't exist", path );
    path
}

fn cli_path() -> PathBuf {
    let path = if let Ok( path ) = std::env::var( "MEMORY_PROFILER_TEST_CLI_PATH" ) {
        build_root().join( path ).join( "bytehound" )
    } else {
        build_root().join( "x86_64-unknown-linux-gnu" ).join( "release" ).join( "bytehound" )
    };

    if !path.exists() {
        panic!( "Missing CLI binary at {:?}; compile it and try again", path );
    }

    path
}

fn target_toolchain_prefix() -> &'static str {
    let target = match target() {
        Some( target ) => target,
        None => return "".into()
    };

    match target.as_str() {
        "aarch64-unknown-linux-gnu" => "aarch64-linux-gnu-",
        "armv7-unknown-linux-gnueabihf" => "arm-linux-gnueabihf-",
        "mips64-unknown-linux-gnuabi64" => "mips64-linux-gnuabi64-",
        "x86_64-unknown-linux-gnu" => "x86_64-linux-gnu-",
        target => panic!( "Unknown target: '{}'", target )
    }
}

fn compiler_cc() -> String {
    format!( "{}gcc", target_toolchain_prefix() )
}

fn compiler_cxx() -> String {
    format!( "{}g++", target_toolchain_prefix() )
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
    pub extra_space: u32,
    pub chain_length: u32,
}

#[derive(Deserialize, Debug)]
struct ResponseAllocations {
    pub allocations: Vec< Allocation >,
    pub total_count: u64
}

#[derive(PartialEq, Deserialize, Debug)]
pub struct AllocationGroupData {
    pub size: u64,
    pub min_size: u64,
    pub max_size: u64,
    pub min_timestamp: Timeval,
    pub min_timestamp_relative: Timeval,
    pub min_timestamp_relative_p: f32,
    pub max_timestamp: Timeval,
    pub max_timestamp_relative: Timeval,
    pub max_timestamp_relative_p: f32,
    pub interval: Timeval,
    pub leaked_count: u64,
    pub allocated_count: u64
}

#[derive(PartialEq, Deserialize, Debug)]
pub struct AllocationGroup {
    pub all: AllocationGroupData,
    pub only_matched: AllocationGroupData,
    pub backtrace_id: u32,
    pub backtrace: Vec< Frame >
}

#[derive(Deserialize, Debug)]
pub struct ResponseAllocationGroups {
    pub allocations: Vec< AllocationGroup >,
    pub total_count: u64
}

struct Analysis {
    response: ResponseAllocations,
    groups: ResponseAllocationGroups
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

fn is_from_function_fuzzy( alloc: &Allocation, expected: &str ) -> bool {
    alloc.backtrace.iter().any( |frame| {
        frame.raw_function.as_ref().map( |symbol| {
            symbol.contains( expected )
        }).unwrap_or( false )
    })
}

impl Analysis {
    fn allocations_from_source< 'a >( &'a self, source: &'a str ) -> impl Iterator< Item = &Allocation > + 'a {
        self.response.allocations.iter().filter( move |alloc| is_from_source( alloc, source ) )
    }
}

fn assert_allocation_backtrace( alloc: &Allocation, expected: &[&str] ) {
    let mut actual: Vec< _ > = alloc.backtrace.iter().map( |frame| frame.raw_function.clone().unwrap_or( String::new() ) ).collect();
    actual.reverse();

    let matches = actual.len() >= expected.len() && actual.iter().zip( expected.iter() ).all( |(lhs, rhs)| lhs == rhs );
    if matches {
        return;
    }

    panic!( "Unexpected backtrace!\n\nActual:\n{}\n\nExpected to start with:\n{}\n", actual.join( "\n" ), expected.join( "\n" ) );
}

fn workdir() -> PathBuf {
    let workdir = build_root();
    std::fs::create_dir_all( &workdir ).unwrap();
    workdir
}

fn analyze( name: &str, path: impl AsRef< Path > ) -> Analysis {
    let cwd = workdir();

    let path = path.as_ref();
    assert_file_exists( path );

    static PORT: AtomicUsize = AtomicUsize::new( 8080 );
    let port = loop {
        let port = PORT.fetch_add( 1, Ordering::SeqCst );
        if std::net::TcpListener::bind( format!( "127.0.0.1:{}", port ) ).is_ok() {
            break port;
        }
    };

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

    let groups = attohttpc::get( &format!( "http://localhost:{}/data/last/allocation_groups", port ) ).send().unwrap();
    assert_eq!( groups.status(), attohttpc::StatusCode::OK );
    assert_eq!( *groups.headers().get( attohttpc::header::CONTENT_TYPE ).unwrap(), "application/json" );
    let groups: ResponseAllocationGroups = serde_json::from_str( &groups.text().unwrap() ).unwrap();

    Analysis { response, groups }
}

fn get_basename( path: &str ) -> &str {
    let index_slash = path.rfind( "/" ).map( |index| index + 1 ).unwrap_or( 0 );
    let index_dot = path.rfind( "." ).unwrap();
    &path[ index_slash..index_dot ]
}

fn compile_with_cargo( source: &str ) -> PathBuf {
    let source_path = repository_root().join( "integration-tests" ).join( "test-programs" ).join( source );
    let args: Vec< &str > = vec![ "build" ];
    let target_dir = build_root().join( "test-programs" );
    run_with_timeout(
        &source_path,
        "cargo",
        &args,
        &[
            ("CARGO_TARGET_DIR", target_dir.clone())
        ],
        Duration::from_secs( 180 )
    ).assert_success();

    target_dir.join( "debug" )
}

fn compile_with_rustc( source: &str ) -> PathBuf {
    let cwd = workdir();
    let source_path = repository_root().join( "integration-tests" ).join( "test-programs" ).join( source );
    let output_path = PathBuf::from( get_basename( source ) );

    run(
        &cwd,
        "rustc",
        &[
            source_path.as_os_str(),
            std::ffi::OsStr::new( "-o" ),
            output_path.as_os_str()
        ],
        EMPTY_ENV
    ).assert_success();

    output_path
}

fn compile_with_flags( source: &str, extra_flags: &[&str] ) {
    let cwd = workdir();
    let basename = get_basename( source );
    let source_path = repository_root().join( "integration-tests" ).join( "test-programs" ).join( source );
    let source_path = source_path.into_os_string().into_string().unwrap();

    let mut args: Vec< &str > = Vec::new();
    if !source.ends_with( ".c" ) {
        args.push( "-std=c++11" );
    }

    args.extend( &[
        "-fasynchronous-unwind-tables",
        "-Wno-unused-result",
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
            compiler_cc(),
            &args,
            EMPTY_ENV
        ).assert_success();
    } else {
        run(
            &cwd,
            compiler_cxx(),
            &args,
            EMPTY_ENV
        ).assert_success();
    }
}

fn compile( source: &str ) {
    compile_with_flags( source, &[] );
}

fn map_to_target(
    executable: impl AsRef< OsStr >,
    args: &[impl AsRef< OsStr >],
    envs: &[(impl AsRef< OsStr >, impl AsRef< OsStr >)]
) -> (OsString, Vec< OsString >, Vec< (OsString, OsString) >) {
    let mut executable = executable.as_ref().to_owned();
    let mut args: Vec< OsString > =
        args.iter().map( |arg| arg.as_ref().to_owned() ).collect();
    let mut envs: Vec< (OsString, OsString) > =
        envs.iter().map( |(key, value)| (key.as_ref().to_owned(), value.as_ref().to_owned()) ).collect();

    if let Some( runner ) = std::env::var_os( "MEMORY_PROFILER_TEST_RUNNER" ) {
        args = std::iter::once( executable ).chain( args.into_iter() ).collect();
        executable = runner;
        if let Some( index ) = envs.iter().position( |&(ref key, _)| key == "LD_PRELOAD" ) {
            let (_, value) = envs.remove( index );
            envs.push( ("TARGET_LD_PRELOAD".into(), value) );
        }
    }

    (executable, args, envs)
}

pub fn run_on_target< C, E, S, P, Q >( cwd: C, executable: E, args: &[S], envs: &[(P, Q)] ) -> CommandResult
    where C: AsRef< Path >,
          E: AsRef< OsStr >,
          S: AsRef< OsStr >,
          P: AsRef< OsStr >,
          Q: AsRef< OsStr >
{
    let (executable, args, envs) = map_to_target( executable, args, envs );
    run( cwd, executable, &args, &envs )
}

pub fn run_in_the_background_on_target< C, E, S, P, Q >( cwd: C, executable: E, args: &[S], envs: &[(P, Q)] ) -> ChildHandle
    where C: AsRef< Path >,
          E: AsRef< OsStr >,
          S: AsRef< OsStr >,
          P: AsRef< OsStr >,
          Q: AsRef< OsStr >
{
    let (executable, args, envs) = map_to_target( executable, args, envs );
    run_in_the_background( cwd, executable, &args, &envs )
}

#[test]
fn test_basic() {
    let cwd = workdir();

    compile( "basic.c" );

    run_on_target(
        &cwd,
        "./basic",
        EMPTY_ARGS,
        &[
            ("LD_PRELOAD", preload_path().into_os_string()),
            ("MEMORY_PROFILER_LOG", "debug".into()),
            ("MEMORY_PROFILER_OUTPUT", "memory-profiling-basic.dat".into())
        ]
    ).assert_success();

    check_allocations_basic_program(&cwd.join( "memory-profiling-basic.dat" ));
}

#[cfg(test)]
fn run_jemalloc_test( name: &str ) {
    let cwd = compile_with_cargo( &format!( "jemalloc/{}", name ) );

    run_on_target(
        &cwd,
        &format!( "./{}", name ),
        EMPTY_ARGS,
        &[
            ("LD_PRELOAD", preload_path().into_os_string()),
            ("MEMORY_PROFILER_LOG", "debug".into()),
            ("MEMORY_PROFILER_OUTPUT", format!( "memory-profiling-{}.dat", name ).into())
        ]
    ).assert_success();

    let analysis = analyze( name, cwd.join( format!( "memory-profiling-{}.dat", name ) ) );
    let mut iter = analysis.allocations_from_source( "main.rs" ).filter( |alloc| {
        is_from_function_fuzzy( alloc, "run_test" )
    });

    let a0 = iter.next().unwrap(); // (jemalloc) malloc, leaked
    let a1 = iter.next().unwrap(); // (jemalloc) malloc, freed
    let a2 = iter.next().unwrap(); // (jemalloc) malloc, freed through realloc
    let a3 = iter.next().unwrap(); // (jemalloc) realloc
    let a4 = iter.next().unwrap(); // (jemalloc) calloc, freed
    let a5 = iter.next().unwrap(); // (libc) malloc, freed through realloc
    let a6 = iter.next().unwrap(); // (libc) realloc, freed

    assert!( a0.deallocation.is_none() );
    assert!( a1.deallocation.is_some() );
    assert!( a2.deallocation.is_some() );
    assert!( a3.deallocation.is_none() );
    assert!( a4.deallocation.is_none() );
    assert!( a5.deallocation.is_some() );
    assert!( a6.deallocation.is_some() );

    assert!( a0.size < a1.size );
    assert!( a1.size < a2.size );
    assert!( a2.size < a3.size );
    assert!( a3.size < a4.size );
    assert_eq!( a5.size, 200 );
    assert_eq!( a6.size, 400 );

    assert_eq!( a0.thread, a1.thread );
    assert_eq!( a1.thread, a2.thread );
    assert_eq!( a2.thread, a3.thread );
    assert_eq!( a3.thread, a4.thread );
    assert_eq!( a4.thread, a5.thread );
    assert_eq!( a5.thread, a6.thread );

    assert_eq!( a0.chain_length, 1 );
    assert_eq!( a1.chain_length, 1 );
    assert_eq!( a2.chain_length, 2 );
    assert_eq!( a3.chain_length, 2 );
    assert_eq!( a4.chain_length, 1 );
    assert_eq!( a5.chain_length, 2 );
    assert_eq!( a6.chain_length, 2 );

    assert_eq!( iter.next(), None );
}

#[test]
fn test_jemalloc_v03_prefixed() {
    run_jemalloc_test( "jemalloc-v03" );
}

#[test]
fn test_jemalloc_v05_prefixed() {
    run_jemalloc_test( "jemalloc-v05" );
}

#[test]
fn test_jemalloc_v05_sallocx() {
    run_jemalloc_test( "jemalloc-v05-sallocx" );
}

#[test]
fn test_jemalloc_v05_unprefixed() {
    run_jemalloc_test( "jemalloc-v05-unprefixed" );
}

#[test]
fn test_alloc_in_tls() {
    let cwd = workdir();

    compile( "alloc-in-tls.cpp" );

    run_on_target(
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

fn test_start_stop_generic( kind: &str ) {
    let cwd = workdir();

    let output = format!( "start-stop_{}", kind );
    let define = format!( "VARIANT_{}", kind.to_uppercase() );
    compile_with_flags( "start-stop.c", &[ "-o", &output, "-D", &define, "-fPIC" ] );
    run_on_target(
        &cwd,
        &format!( "./{}", output ),
        EMPTY_ARGS,
        &[
            ("LD_PRELOAD", preload_path().into_os_string()),
            ("MEMORY_PROFILER_LOG", "debug".into()),
            ("MEMORY_PROFILER_OUTPUT", format!( "start-stop_{}.dat", kind ).into()),
            ("MEMORY_PROFILER_DISABLE_BY_DEFAULT", "1".into())
        ]
    ).assert_success();

    let analysis = analyze( &format!( "start-stop_{}", kind ), cwd.join( format!( "start-stop_{}.dat", kind ) ) );

    let mut iter = analysis.allocations_from_source( "start-stop.c" );
    let a0 = iter.next().unwrap();
    let a1 = iter.next().unwrap();
    let a2 = iter.next().unwrap();
    let a3 = iter.next().unwrap();
    let a4 = iter.next().unwrap();

    assert_eq!( a0.size, 10002 );
    assert_eq!( a1.size, 20002 );
    assert_eq!( a2.size, 10003 );
    assert_eq!( a3.size, 10004 );
    assert_eq!( a4.size, 20003 );

    assert_eq!( a0.thread, a2.thread );
    assert_ne!( a0.thread, a1.thread );
    assert_ne!( a3.thread, a4.thread );

    assert!( a0.deallocation.is_some() );
    assert!( a1.deallocation.is_none() );
    assert!( a2.deallocation.is_none() );
    assert!( a3.deallocation.is_none() );
    assert!( a4.deallocation.is_none() );

    assert_eq!( iter.next(), None );
}

#[test]
fn test_start_stop_sigusr1() {
    test_start_stop_generic( "sigusr1" );
}

#[test]
fn test_start_stop_api() {
    test_start_stop_generic( "api" );
}

#[test]
fn test_fork() {
    let cwd = workdir();

    compile( "fork.c" );

    run_on_target(
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
    let cwd = workdir();

    compile( "exit_1.c" );

    run_on_target(
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
    let cwd = workdir();

    compile( "exit_2.c" );

    run_on_target(
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
    let cwd = workdir();

    compile( "exit_3.c" );

    run_on_target(
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
    let cwd = workdir();

    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once( || compile( "gather.c" ) );

    static PORT: AtomicUsize = AtomicUsize::new( 8100 );
    let port = PORT.fetch_add( 1, Ordering::SeqCst );

    let child = run_in_the_background_on_target(
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
    let mut found = false;
    while timestamp.elapsed() < Duration::from_secs( 30 ) {
        if std::net::TcpStream::connect( format!( "127.0.0.1:{}", port ) ).is_ok() {
            found = true;
            break;
        }
        thread::sleep( Duration::from_millis( 100 ) );
    }

    if !found {
        panic!( "Couldn't connect to the embedded server" );
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

#[ignore]
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
    let cwd = workdir();
    compile_with_flags( "dlopen.c", &[ "-ldl" ] );
    compile_with_flags( "dlopen_so.c", &[ "-shared", "-fPIC" ] );

    run_on_target(
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

#[test]
fn test_throw() {
    let cwd = workdir();
    compile( "throw.cpp" );

    run_on_target(
        &cwd,
        "./throw",
        EMPTY_ARGS,
        &[
            ("LD_PRELOAD", preload_path().into_os_string()),
            ("MEMORY_PROFILER_LOG", "debug".into()),
            ("MEMORY_PROFILER_OUTPUT", "throw.dat".into())
        ]
    ).assert_success();

    let analysis = analyze( "throw", cwd.join( "throw.dat" ) );
    let a0 = analysis.response.allocations.iter().find( |alloc| alloc.size == 123456 ).unwrap();
    let a1 = analysis.response.allocations.iter().find( |alloc| alloc.size == 123457 ).unwrap();
    let a2 = analysis.response.allocations.iter().find( |alloc| alloc.size == 123458 ).unwrap();
    let a3 = analysis.response.allocations.iter().find( |alloc| alloc.size == 123459 ).unwrap();

    assert_allocation_backtrace( a0, &[
        "foobar_0",
        "foobar_1",
        "foobar_2",
        "foobar_3",
        "foobar_4",
        "foobar_5",
        "main"
    ]);

    assert_allocation_backtrace( a1, &[
        "foobar_3",
        "foobar_4",
        "foobar_5",
        "main"
    ]);

    assert_allocation_backtrace( a2, &[
        "foobar_5",
        "main"
    ]);

    assert_allocation_backtrace( a3, &[
        "main"
    ]);
}

#[test]
fn test_longjmp() {
    let cwd = workdir();
    compile( "longjmp.c" );

    run_on_target(
        &cwd,
        "./longjmp",
        EMPTY_ARGS,
        &[
            ("LD_PRELOAD", preload_path().into_os_string()),
            ("MEMORY_PROFILER_LOG", "debug".into()),
            ("MEMORY_PROFILER_OUTPUT", "longjmp.dat".into())
        ]
    ).assert_success();

    let analysis = analyze( "longjmp", cwd.join( "longjmp.dat" ) );
    let a0 = analysis.response.allocations.iter().find( |alloc| alloc.size == 123456 ).unwrap();
    let a1 = analysis.response.allocations.iter().find( |alloc| alloc.size == 123457 ).unwrap();
    let a2 = analysis.response.allocations.iter().find( |alloc| alloc.size == 123458 ).unwrap();
    let a3 = analysis.response.allocations.iter().find( |alloc| alloc.size == 123459 ).unwrap();

    assert_allocation_backtrace( a0, &[
        "foobar_0",
        "foobar_1",
        "foobar_2",
        "foobar_3",
        "foobar_4",
        "foobar_5",
        "main"
    ]);

    assert_allocation_backtrace( a1, &[
        "foobar_3",
        "foobar_4",
        "foobar_5",
        "main"
    ]);

    assert_allocation_backtrace( a2, &[
        "foobar_5",
        "main"
    ]);

    assert_allocation_backtrace( a3, &[
        "main"
    ]);
}

#[test]
fn test_backtrace() {
    let cwd = workdir();
    compile_with_flags( "backtrace.c", &[ "-rdynamic" ] );

    run_on_target(
        &cwd,
        "./backtrace",
        EMPTY_ARGS,
        &[
            ("LD_PRELOAD", preload_path().into_os_string()),
            ("MEMORY_PROFILER_LOG", "debug".into()),
            ("MEMORY_PROFILER_OUTPUT", "backtrace.dat".into())
        ]
    ).assert_success();

    let analysis = analyze( "backtrace", cwd.join( "backtrace.dat" ) );
    assert!( analysis.response.allocations.iter().any( |alloc| alloc.size == 123456 ) );
}

#[test]
fn test_return_opt_u128() {
    let cwd = workdir();
    compile_with_rustc( "return-opt-u128.rs" );

    run_on_target(
        &cwd,
        "./return-opt-u128",
        EMPTY_ARGS,
        &[
            ("LD_PRELOAD", preload_path().into_os_string()),
            ("MEMORY_PROFILER_LOG", "debug".into()),
            ("MEMORY_PROFILER_OUTPUT", "return-opt-u128.dat".into())
        ]
    ).assert_success();

    let analysis = analyze( "return-opt-u128", cwd.join( "return-opt-u128.dat" ) );
    assert!( analysis.response.allocations.iter().any( |alloc| alloc.size == 123456 ) );
}

#[test]
fn test_return_f64() {
    let cwd = workdir();
    compile_with_rustc( "return-f64.rs" );

    run_on_target(
        &cwd,
        "./return-f64",
        EMPTY_ARGS,
        &[
            ("LD_PRELOAD", preload_path().into_os_string()),
            ("MEMORY_PROFILER_LOG", "debug".into()),
            ("MEMORY_PROFILER_OUTPUT", "return-f64.dat".into())
        ]
    ).assert_success();

    let analysis = analyze( "return-f64", cwd.join( "return-f64.dat" ) );
    assert!( analysis.response.allocations.iter().any( |alloc| alloc.size == 123456 ) );
}

#[cfg(feature = "test-wasmtime")]
#[cfg(target_arch = "x86_64")]
#[test]
fn test_wasmtime_linking() {
    let cwd = compile_with_cargo( "wasmtime/linking" );

    run_on_target(
        &cwd,
        "./linking",
        EMPTY_ARGS,
        &[
            ("LD_PRELOAD", preload_path().into_os_string()),
            ("MEMORY_PROFILER_LOG", "debug".into()),
            ("MEMORY_PROFILER_OUTPUT", "linking.dat".into())
        ]
    ).assert_success();

    let analysis = analyze( "linking", cwd.join( "linking.dat" ) );
    let list: Vec< _ > = analysis.response.allocations.into_iter().filter( |alloc| is_from_function_fuzzy( alloc, "wasm_to_host_shim" ) ).collect();
    assert!( !list.is_empty() );
    for allocation in list {
        assert!( is_from_function( &allocation, "main" ) );
    }
}

#[cfg(feature = "test-wasmtime")]
#[cfg(target_arch = "x86_64")]
#[test]
fn test_wasmtime_interrupt() {
    let cwd = compile_with_cargo( "wasmtime/interrupt" );

    run_on_target(
        &cwd,
        "./interrupt",
        EMPTY_ARGS,
        &[
            ("LD_PRELOAD", preload_path().into_os_string()),
            ("MEMORY_PROFILER_LOG", "debug".into()),
            ("MEMORY_PROFILER_OUTPUT", "interrupt.dat".into())
        ]
    ).assert_success();

    let analysis = analyze( "interrupt", cwd.join( "interrupt.dat" ) );
    let list: Vec< _ > = analysis.response.allocations.into_iter().filter( |alloc| is_from_function_fuzzy( alloc, "trap_handler" ) ).collect();
    assert!( !list.is_empty() );
    for allocation in list {
        assert!( is_from_function( &allocation, "main" ) );
    }
}

#[test]
fn test_cull() {
    let cwd = workdir();

    compile( "cull.c" );

    run_on_target(
        &cwd,
        "./cull",
        EMPTY_ARGS,
        &[
            ("LD_PRELOAD", preload_path().into_os_string()),
            ("MEMORY_PROFILER_LOG", "debug".into()),
            ("MEMORY_PROFILER_OUTPUT", "memory-profiling-cull.dat".into()),
            ("MEMORY_PROFILER_CULL_TEMPORARY_ALLOCATIONS", "1".into()),
            ("MEMORY_PROFILER_TEMPORARY_ALLOCATION_LIFETIME_THRESHOLD", "100".into())
        ]
    ).assert_success();

    let analysis = analyze( "cull", cwd.join( "memory-profiling-cull.dat" ) );
    let mut iter = analysis.allocations_from_source( "cull.c" );

    let a0 = iter.next().unwrap();
    let a1 = iter.next().unwrap();
    assert!( a0.deallocation.is_some() );
    assert!( a1.deallocation.is_none() );
    assert_eq!( a0.size, 1235 );
    assert_eq!( a1.size, 1236 );

    let g0 = analysis.groups.allocations.iter().find( |group| group.backtrace_id == a0.backtrace_id ).unwrap();
    assert_eq!( g0.all.leaked_count, 1 );
    assert_eq!( g0.all.allocated_count, 2 );

    let a2 = iter.next().unwrap();
    let a3 = iter.next().unwrap();
    assert_eq!( a2.size, 2000 );
    assert_eq!( a3.size, 3000 );
    assert_eq!( a2.chain_length, 2 );
    assert_eq!( a3.chain_length, 2 );

    assert_eq!( iter.next(), None );
}

#[test]
fn test_cross_thread_alloc_culled() {
    let cwd = workdir();

    compile_with_flags( "cross-thread-alloc.c", &["-o", "cross-thread-alloc-culled"] );

    run_on_target(
        &cwd,
        "./cross-thread-alloc-culled",
        EMPTY_ARGS,
        &[
            ("LD_PRELOAD", preload_path().into_os_string()),
            ("MEMORY_PROFILER_LOG", "debug".into()),
            ("MEMORY_PROFILER_OUTPUT", "memory-profiling-cross-thread-alloc-culled.dat".into()),
            ("MEMORY_PROFILER_CULL_TEMPORARY_ALLOCATIONS", "1".into()),
            ("MEMORY_PROFILER_TEMPORARY_ALLOCATION_LIFETIME_THRESHOLD", "1000".into())
        ]
    ).assert_success();

    let analysis = analyze( "cross-thread-alloc-culled", cwd.join( "memory-profiling-cross-thread-alloc-culled.dat" ) );
    let mut iter = analysis.allocations_from_source( "cross-thread-alloc.c" ).filter( move |alloc| !is_from_function( alloc, "pthread_create" ) );
    assert!( iter.next().is_none() );
}

#[test]
fn test_cross_thread_alloc_non_culled() {
    let cwd = workdir();

    compile_with_flags( "cross-thread-alloc.c", &["-o", "cross-thread-alloc-non-culled"] );

    run_on_target(
        &cwd,
        "./cross-thread-alloc-non-culled",
        EMPTY_ARGS,
        &[
            ("LD_PRELOAD", preload_path().into_os_string()),
            ("MEMORY_PROFILER_LOG", "debug".into()),
            ("MEMORY_PROFILER_OUTPUT", "memory-profiling-cross-thread-alloc-non-culled.dat".into()),
            ("MEMORY_PROFILER_CULL_TEMPORARY_ALLOCATIONS", "1".into()),
            ("MEMORY_PROFILER_TEMPORARY_ALLOCATION_LIFETIME_THRESHOLD", "1".into())
        ]
    ).assert_success();

    let analysis = analyze( "cross-thread-alloc-non-culled", cwd.join( "memory-profiling-cross-thread-alloc-non-culled.dat" ) );
    let mut iter = analysis.allocations_from_source( "cross-thread-alloc.c" ).filter( move |alloc| !is_from_function( alloc, "pthread_create" ) );
    let a0 = iter.next().unwrap();
    let a1 = iter.next().unwrap();
    let a2 = iter.next().unwrap();

    assert_eq!( a0.size, 1234 );
    assert_eq!( a1.size, 1235 );
    assert_eq!( a2.size, 1236 );

    assert!( iter.next().is_none() );
}

#[test]
fn test_track_spawned_children() {
    let cwd = workdir();

    compile_with_flags( "basic.c", &["-o", "basic"] );
    compile_with_flags( "spawn-child.c", &["-o", "spawn-child"] );

    run_on_target(
        &cwd,
        "./spawn-child",
        EMPTY_ARGS,
        &[
            ("LD_PRELOAD", preload_path().into_os_string()),
            ("MEMORY_PROFILER_LOG", "debug".into()),
            ("MEMORY_PROFILER_TRACK_CHILD_PROCESSES", "1".into()),
            ("MEMORY_PROFILER_OUTPUT", "memory-profiling-%e.dat".into())
        ]
    ).assert_success();

    check_allocations_basic_program(&cwd.join( "memory-profiling-basic.dat" ));

    let analysis = analyze( "spawn-child", cwd.join( "memory-profiling-spawn-child.dat" ));
    let mut iter = analysis.allocations_from_source( "spawn-child.c" );

    let a0 = iter.next().unwrap();
    assert_eq!(a0.size, 10001);

    let a1 = iter.next().unwrap();
    assert_eq!(a1.size, 10003);
}

fn check_allocations_basic_program(path: &Path) {
    let analysis = analyze( "basic", path );
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

    assert_eq!( a0.chain_length, 1 );
    assert_eq!( a1.chain_length, 1 );
    assert_eq!( a2.chain_length, 2 );
    assert_eq!( a3.chain_length, 2 );
    assert_eq!( a4.chain_length, 1 );
    assert_eq!( a5.chain_length, 1 );

    assert_eq!( iter.next(), None );
}
