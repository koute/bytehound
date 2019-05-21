use std::mem::transmute;
use std::env;
use libc::{self, c_void, c_int, uintptr_t};
use perf_event_open::{Perf, EventSource, Event};

use crate::spin_lock::SpinLock;
use crate::opt;

type Context = *mut c_void;
type ReasonCode = c_int;
type Callback = extern "C" fn( Context, *mut c_void ) -> ReasonCode;

pub struct Backtrace {
    pub frames: Vec< u64 >,
    pub stale_count: Option< u32 >
}

const CACHE_SIZE: usize = 256;

lazy_static! {
    static ref BACKTRACE_CACHE: SpinLock< Vec< Vec< u64 > > > = {
        SpinLock::new( Vec::with_capacity( CACHE_SIZE ) )
    };
}

impl Backtrace {
    pub fn new() -> Self {
        let mut cache = BACKTRACE_CACHE.lock();
        Backtrace {
            frames: cache.pop().unwrap_or_else( Default::default ),
            stale_count: None
        }
    }

    pub fn is_empty( &self ) -> bool {
        self.frames.is_empty()
    }
}

impl Drop for Backtrace {
    fn drop( &mut self ) {
        let mut vec = std::mem::replace( &mut self.frames, Vec::new() );
        if vec.capacity() > 0 && vec.capacity() <= 1024 {
            vec.clear();
            let mut cache = BACKTRACE_CACHE.lock();
            if cache.len() >= CACHE_SIZE {
                return;
            }
            cache.push( vec );
        }
    }
}

extern "C" {
    fn _Unwind_Backtrace( callback: Callback, data: *mut c_void ) -> ReasonCode;
    fn _Unwind_GetIP( context: Context ) -> uintptr_t;
    fn _Unwind_VRS_Get( context: Context, regclass: _Unwind_VRS_RegClass, regno: u32, repr: _Unwind_VRS_DataRepresentation, valuep: *mut c_void ) -> _Unwind_VRS_Result;
}

#[allow(non_camel_case_types)]
#[repr(C)]
enum _Unwind_VRS_RegClass {
    _UVRSC_CORE = 0,
    _UVRSC_VFP = 1,
    _UVRSC_WMMXD = 3,
    _UVRSC_WMMXC = 4
}

#[allow(non_camel_case_types)]
#[repr(C)]
enum _Unwind_VRS_DataRepresentation {
    _UVRSD_UINT32 = 0,
    _UVRSD_VFPX = 1,
    _UVRSD_UINT64 = 3,
    _UVRSD_FLOAT = 4,
    _UVRSD_DOUBLE = 5
}

#[allow(non_camel_case_types)]
#[repr(C)]
enum _Unwind_VRS_Result {
    _UVRSR_OK = 0,
    _UVRSR_NOT_IMPLEMENTED = 1,
    _UVRSR_FAILED = 2
}

#[cfg(not(target_arch = "arm"))]
unsafe fn get_ip( context: Context ) -> uintptr_t {
    _Unwind_GetIP( context )
}

#[cfg(target_arch = "arm")]
unsafe fn get_gr( context: Context, index: c_int ) -> uintptr_t {
    let mut value: uintptr_t = 0;
    _Unwind_VRS_Get( context, _Unwind_VRS_RegClass::_UVRSC_CORE, index as u32, _Unwind_VRS_DataRepresentation::_UVRSD_UINT32, &mut value as *mut uintptr_t as *mut c_void );
    value
}

#[cfg(target_arch = "arm")]
unsafe fn get_ip( context: Context ) -> uintptr_t {
    get_gr( context, 15 ) & ( !(0x1 as uintptr_t) )
}

extern "C" fn on_backtrace( context: Context, data: *mut c_void ) -> ReasonCode {
    unsafe {
        let out: &mut Vec< u64 > = transmute( data );
        out.push( get_ip( context ) as u64 );
    }

    0
}

lazy_static! {
    static ref AS: SpinLock< ::nwind::LocalAddressSpace > = {
        let opts = ::nwind::LocalAddressSpaceOptions::new()
            .should_load_symbols( cfg!(feature = "logging") && log_enabled!( ::log::Level::Debug ) );

        let mut address_space = ::nwind::LocalAddressSpace::new_with_opts( opts ).unwrap();
        if let Ok( value ) = env::var( "MEMORY_PROFILER_USE_SHADOW_STACK" ) {
            if value == "0" {
                address_space.use_shadow_stack( false );
            } else if value == "1" {
                address_space.use_shadow_stack( true );
            }
        }

        SpinLock::new( address_space )
    };

    static ref PERF: SpinLock< Perf > = {
        let perf = Perf::build()
            .any_cpu()
            .event_source( EventSource::SwDummy )
            .open()
            .expect( "failed to initialize perf_event_open" );

        SpinLock::new( perf )
    };
}

#[inline(never)]
pub fn grab( out: &mut Backtrace ) {
    out.frames.clear();

    if false {
        unsafe {
            _Unwind_Backtrace( on_backtrace, transmute( &mut out.frames ) );
        }

        return;
    }

    let mut reload_address_space = false;
    {
        let mut perf = PERF.lock();
        if perf.are_events_pending() {
            for event in perf.iter() {
                match event.get() {
                    Event::Mmap2( ref event ) if event.filename != b"//anon" && event.inode != 0 && event.protection & libc::PROT_EXEC as u32 != 0 => {
                        debug!( "New executable region mmaped: {:?}", event );
                        reload_address_space = true;
                    },
                    Event::Lost( _ ) => {
                        debug!( "Lost events; forcing a reload" );
                        reload_address_space = true;
                    },
                    _ => {}
                }
            }
        }
    }

    let debug_crosscheck_unwind_results = opt::crosscheck_unwind_results_with_libunwind() && !AS.lock().is_shadow_stack_enabled();

    {
        let mut address_space = AS.lock();
        if reload_address_space {
            info!( "Reloading address space" );
            address_space.reload().unwrap();
        }

        if debug_crosscheck_unwind_results || !opt::emit_partial_backtraces() {
            address_space.unwind( |frame| {
                out.frames.push( frame.address );
                ::nwind::UnwindControl::Continue
            });
            out.stale_count = None;
        } else {
            let stale_count = address_space.unwind_through_fresh_frames( |frame| {
                out.frames.push( frame.address );
                ::nwind::UnwindControl::Continue
            });
            out.stale_count = stale_count.map( |value| value as u32 );
        }
    }

    if debug_crosscheck_unwind_results {
        let mut expected: Vec< u64 > = Vec::with_capacity( out.frames.len() );
        unsafe {
            _Unwind_Backtrace( on_backtrace, transmute( &mut expected ) );
        }

        if expected.last() == Some( &0 ) {
            expected.pop();
        }

        if out.frames[ 1.. ] != expected[ 1.. ] {
            info!( "/proc/self/maps:\n{}", String::from_utf8_lossy( &::std::fs::read( "/proc/self/maps" ).unwrap() ).trim() );

            let address_space = AS.lock();
            info!( "Expected: " );
            for &address in &expected {
                info!( "    {:?}", address_space.decode_symbol_once( address ) );
            }

            info!( "Actual: " );
            for &address in out.frames.iter() {
                info!( "    {:?}", address_space.decode_symbol_once( address ) );
            }

            panic!( "Wrong backtrace; expected: {:?}, got: {:?}", expected, out.frames );
        }
    }
}
