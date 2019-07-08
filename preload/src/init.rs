use std::env;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use crate::{RUNNING, TRACING_ENABLED, ON_APPLICATION_THREAD_DEFAULT};
use crate::event::{InternalEvent, send_event};
use crate::logger;
use crate::opt;
use crate::spin_lock::SpinLock;
use crate::thread_main;
use crate::tls::get_tls;
use crate::utils::generate_filename;

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
pub(crate) fn initialize() {
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
