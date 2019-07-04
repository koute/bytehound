use std::env;

static mut PRECISE_TIMESTAMPS: bool = true;
static mut GRAB_BACKTRACES_ON_FREE: bool = false;
static mut ZERO_MEMORY: bool = false;
static mut TRACING_ENABLED_BY_DEFAULT: bool = true;

pub fn initialize() {
    let flag_precise_timestamps = env::var_os( "MEMORY_PROFILER_PRECISE_TIMESTAMPS" )
        .map( |value| value == "1" )
        .unwrap_or( false );

    if flag_precise_timestamps {
        info!( "Timestamp granularity: precise" );
    } else {
        info!( "Timestamp granularity: coarse" );
    }

    unsafe {
        PRECISE_TIMESTAMPS = flag_precise_timestamps;
    }

    let flag_backtraces_on_free = env::var_os( "MEMORY_PROFILER_GRAB_BACKTRACES_ON_FREE" )
        .map( |value| value == "1" )
        .unwrap_or( false );

    if flag_backtraces_on_free {
        info!( "Grab backtraces on `free`: yes" );
    } else {
        info!( "Grab backtraces on `free`: no" );
    }

    unsafe {
        GRAB_BACKTRACES_ON_FREE = flag_backtraces_on_free;
    }

    let flag_zero_memory = env::var_os( "MEMORY_PROFILER_ZERO_MEMORY" )
        .map( |value| value == "1" )
        .unwrap_or( false );

    if flag_zero_memory {
        info!( "Will always return zero'd memory: yes" );
    } else {
        info!( "Will always return zero'd memory: no" );
    }

    unsafe {
        ZERO_MEMORY = flag_zero_memory;
    }

    let tracing_enabled_by_default = env::var_os( "MEMORY_PROFILER_DISABLE_BY_DEFAULT" )
        .map( |value| value != "1" )
        .unwrap_or( true );

    if tracing_enabled_by_default {
        info!( "Tracing enabled by default: yes" );
    } else {
        info!( "Tracing enabled by default: no" );
    }

    unsafe {
        TRACING_ENABLED_BY_DEFAULT = tracing_enabled_by_default;
    }
}

#[inline]
pub fn precise_timestamps() -> bool {
    unsafe { PRECISE_TIMESTAMPS }
}

#[inline]
pub fn grab_backtraces_on_free() -> bool {
    unsafe { GRAB_BACKTRACES_ON_FREE }
}

#[inline]
pub fn zero_memory() -> bool {
    unsafe { ZERO_MEMORY }
}

#[inline]
pub fn tracing_enabled_by_default() -> bool {
    unsafe { TRACING_ENABLED_BY_DEFAULT }
}

#[inline]
pub fn crosscheck_unwind_results_with_libunwind() -> bool {
    false
}

pub fn chown_output_to() -> Option< u32 > {
    lazy_static! {
        static ref VALUE: Option< u32 > = {
            env::var( "MEMORY_PROFILER_CHOWN_OUTPUT_TO" ).ok()
                .and_then( |uid| uid.parse::< u32 >().ok() )
        };
    }

    *VALUE
}

pub fn should_write_binaries_to_output() -> bool {
    lazy_static! {
        static ref VALUE: bool = {
            env::var_os( "MEMORY_PROFILER_WRITE_BINARIES_TO_OUTPUT" )
                .map( |value| value == "1" )
                .unwrap_or( true )
        };
    }

    *VALUE
}

pub fn emit_partial_backtraces() -> bool {
    if !cfg!(debug_assertions) {
        return true;
    }

    lazy_static! {
        static ref VALUE: bool = {
            let value = env::var_os( "MEMORY_PROFILER_EMIT_PARTIAL_BACKTRACES" )
                .map( |value| value == "1" )
                .unwrap_or( true );

            if value {
                info!( "Will emit partial backtraces" );
            } else {
                info!( "Will NOT emit partial backtraces" );
            }

            value
        };
    }

    *VALUE
}

pub fn are_broadcasts_enabled() -> bool {
    lazy_static! {
        static ref VALUE: bool = {
            let flag = env::var_os( "MEMORY_PROFILER_ENABLE_BROADCAST" )
                .map( |value| value == "1" )
                .unwrap_or( false );

            if flag {
                info!( "Will send broadcasts" );
            } else {
                info!( "Will NOT send broadcasts" );
            }

            flag
        };
    }

    *VALUE
}

pub fn base_broadcast_port() -> u16 {
    lazy_static! {
        static ref VALUE: u16 = {
            let port = env::var( "MEMORY_PROFILER_BASE_BROADCAST_PORT" ).ok()
                .and_then( |port| port.parse::< u16 >().ok() )
                .unwrap_or( 8100 );

            info!( "Will use {} as a base broadcast port", port );
            port
        };
    }

    *VALUE
}
