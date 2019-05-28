use {
    reqwest::{
        header::{
            CONTENT_TYPE
        },
        StatusCode
    },
    serde_derive::{
        Deserialize
    },
    std::{
        path::{
            Path,
            PathBuf
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
    repository_root().join( "target" ).join( "x86_64-unknown-linux-gnu" ).join( "release" ).join( "libmemory_profiler.so" )
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

#[test]
fn test_basic() {
    let cwd = repository_root().join( "target" );
    run(
        &cwd,
        "gcc",
        &[
            "-fasynchronous-unwind-tables",
            "-O0",
            "-ggdb3",
            "../integration-tests/test-programs/basic.c",
            "-o",
            "basic"
        ],
        EMPTY_ENV
    ).assert_success();

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

    assert_file_exists( cwd.join( "memory-profiling-basic.dat" ) );

    let _child = run_in_the_background(
        &cwd,
        cli_path(),
        &["server", "memory-profiling-basic.dat"],
        &[("RUST_LOG", "server_core=debug,cli_core=debug,actix_net=info")]
    );

    let start = Instant::now();
    let mut found = false;
    while start.elapsed() < Duration::from_secs( 10 ) {
        thread::sleep( Duration::from_millis( 100 ) );
        if let Some( mut response ) = reqwest::get( "http://localhost:8080/list" ).ok() {
            assert_eq!( response.status(), StatusCode::OK );
            assert_eq!( *response.headers().get( CONTENT_TYPE ).unwrap(), "application/json" );
            let list: Vec< ResponseMetadata > = serde_json::from_str( &response.text().unwrap() ).unwrap();
            if !list.is_empty() {
                found = true;
                break;
            }
        }
    }

    assert!( found );

    let mut response = reqwest::get( "http://localhost:8080/data/last/allocations" ).unwrap();
    assert_eq!( response.status(), StatusCode::OK );
    assert_eq!( *response.headers().get( CONTENT_TYPE ).unwrap(), "application/json" );
    let response: ResponseAllocations = serde_json::from_str( &response.text().unwrap() ).unwrap();

    fn is_from_source( alloc: &Allocation, expected: &str ) -> bool {
        alloc.backtrace.iter().any( |frame| {
            frame.source.as_ref().map( |source| {
                source.ends_with( expected )
            }).unwrap_or( false )
        })
    }

    let mut iter = response.allocations.into_iter().filter( |alloc| is_from_source( &alloc, "basic.c" ) );
    let a0 = iter.next().unwrap(); // malloc, leaked
    let a1 = iter.next().unwrap(); // malloc, freed
    let a2 = iter.next().unwrap(); // malloc, freed through realloc
    let a3 = iter.next().unwrap(); // realloc
    let a4 = iter.next().unwrap(); // calloc, freed

    assert!( a0.deallocation.is_none() );
    assert!( a1.deallocation.is_some() );
    assert!( a2.deallocation.is_some() );
    assert!( a3.deallocation.is_none() );
    assert!( a4.deallocation.is_none() );

    assert!( a0.size < a1.size );
    assert!( a1.size < a2.size );
    assert!( a2.size < a3.size );
    assert!( a3.size < a4.size );

    assert_eq!( a0.thread, a1.thread );
    assert_eq!( a1.thread, a2.thread );
    assert_eq!( a2.thread, a3.thread );
    assert_eq!( a3.thread, a4.thread );

    assert_eq!( a0.backtrace.last().unwrap().line.unwrap() + 1, a1.backtrace.last().unwrap().line.unwrap() );

    assert_eq!( iter.next(), None );
}

#[test]
fn test_alloc_in_tls() {
    let cwd = repository_root().join( "target" );
    run(
        &cwd,
        "g++",
        &[
            "-fasynchronous-unwind-tables",
            "-O0",
            "-pthread",
            "-ggdb3",
            "../integration-tests/test-programs/alloc-in-tls.cpp",
            "-o",
            "alloc-in-tls"
        ],
        EMPTY_ENV
    ).assert_success();

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
