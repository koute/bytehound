use crate::global::on_exit;
use crate::logger;
use crate::opt;
use crate::utils::generate_filename;

pub static mut SYSCALL_LOGGER: logger::SyscallLogger = logger::SyscallLogger::empty();
pub static mut FILE_LOGGER: logger::FileLogger = logger::FileLogger::empty();

pub fn initialize_logger() {
    let log_level = if let Some( value ) = unsafe { crate::syscall::getenv( b"MEMORY_PROFILER_LOG" ) } {
        match value.to_str() {
            Some( "trace" ) => log::LevelFilter::Trace,
            Some( "debug" ) => log::LevelFilter::Debug,
            Some( "info" ) => log::LevelFilter::Info,
            Some( "warn" ) => log::LevelFilter::Warn,
            Some( "error" ) => log::LevelFilter::Error,
            _ => log::LevelFilter::Off
        }
    } else {
        log::LevelFilter::Off
    };

    let pid = crate::syscall::getpid();
    unsafe {
        SYSCALL_LOGGER.initialize( log_level, pid );
    }

    if let Some( value ) = unsafe { crate::syscall::getenv( b"MEMORY_PROFILER_LOGFILE" ) } {
        let path = generate_filename( value.as_slice(), None );
        let rotate_at = unsafe { crate::syscall::getenv( b"MEMORY_PROFILER_LOGFILE_ROTATE_WHEN_BIGGER_THAN" ) }.and_then( |value| value.to_str()?.parse().ok() );

        unsafe {
            if let Ok(()) = FILE_LOGGER.initialize( path, rotate_at, log_level, pid ) {
                log::set_logger( &FILE_LOGGER ).unwrap();
            }
        }
    } else {
        unsafe {
            log::set_logger( &SYSCALL_LOGGER ).unwrap();
        }
    }

    log::set_max_level( log_level );
}

pub fn initialize_atexit_hook() {
    info!( "Setting atexit hook..." );
    unsafe {
        let result = libc::atexit( on_exit );
        if result != 0 {
            error!( "Cannot set the at-exit hook" );
        }
    }
}

pub fn initialize_signal_handlers() {
    extern "C" fn sigusr_handler( signal: libc::c_int ) {
        let signal_name = match signal {
            libc::SIGUSR1 => "SIGUSR1",
            libc::SIGUSR2 => "SIGUSR2",
            _ => "???"
        };

        info!( "Signal handler triggered with signal: {} ({})", signal_name, signal );
        crate::global::toggle();
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
