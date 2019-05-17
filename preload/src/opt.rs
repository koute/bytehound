use std::env;

#[inline]
pub fn grab_backtraces_on_free() -> bool {
    lazy_static! {
        static ref VALUE: bool = {
            let flag = env::var_os( "MEMORY_PROFILER_GRAB_BACKTRACES_ON_FREE" )
                .map( |value| value == "1" )
                .unwrap_or( false );

            if flag {
                info!( "Will grab backtraces on `free()`" );
            }

            flag
        };
    }

    *VALUE
}

#[inline]
pub fn zero_memory() -> bool {
    lazy_static! {
        static ref VALUE: bool = {
            let flag = env::var_os( "MEMORY_PROFILER_ZERO_MEMORY" )
                .map( |value| value == "1" )
                .unwrap_or( false );

            if flag {
                info!( "Will always return zero'd memory" );
            }

            flag
        };
    }

    *VALUE
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
