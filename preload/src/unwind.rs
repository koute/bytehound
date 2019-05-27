use std::mem::{self, transmute};
use std::env;
use std::sync::{Arc, Weak};
use libc::{self, c_void, c_int, uintptr_t};
use perf_event_open::{Perf, EventSource, Event};

use crate::spin_lock::SpinLock;
use crate::opt;

type Context = *mut c_void;
type ReasonCode = c_int;
type Callback = extern "C" fn( Context, *mut c_void ) -> ReasonCode;

struct CacheEntry {
    frames: *mut u64,
    capacity: usize,
    cache: Weak< Cache >
}

impl CacheEntry {
    #[inline(always)]
    fn pack( mut frames: Vec< u64 >, cache: Weak< Cache > ) -> Self {
        let entry = CacheEntry {
            frames: frames.as_mut_ptr(),
            capacity: frames.capacity(),
            cache
        };

        mem::forget( frames );
        entry
    }

    #[inline(always)]
    fn unpack( self ) -> (Vec< u64 >, Weak< Cache >) {
        let frames = unsafe { Vec::from_raw_parts( self.frames, 0, self.capacity ) };
        (frames, self.cache)
    }
}

struct Cache {
    entries: SpinLock< Vec< CacheEntry > >
}

impl Cache {
    fn new() -> Self {
        Cache {
            entries: SpinLock::new( Vec::new() )
        }
    }
}

impl Drop for Cache {
    fn drop( &mut self ) {
        let mut entries = self.entries.lock();
        for entry in entries.drain( .. ) {
            mem::drop( entry.unpack() );
        }
    }
}

pub struct Backtrace {
    pub frames: Vec< u64 >,
    pub stale_count: Option< u32 >,
    cache: Weak< Cache >
}

thread_local! {
    static BACKTRACE_CACHE: Arc< Cache > = Arc::new( Cache::new() );
}

impl Backtrace {
    fn reserve_from_cache( &mut self ) {
        let _ = BACKTRACE_CACHE.try_with( |cache_arc| {
            let mut entries = cache_arc.entries.lock();
            if let Some( entry ) = entries.pop() {
                let (frames, cache) = entry.unpack();
                self.frames = frames;
                self.cache = cache;
            } else {
                self.cache = Arc::downgrade( cache_arc );
            }
        });
    }

    pub fn new() -> Self {
        Backtrace {
            frames: Vec::new(),
            stale_count: None,
            cache: Weak::new()
        }
    }

    pub fn is_empty( &self ) -> bool {
        self.frames.is_empty()
    }
}

impl Drop for Backtrace {
    fn drop( &mut self ) {
        let frames = mem::replace( &mut self.frames, Vec::new() );
        let cache_weak = mem::replace( &mut self.cache, Weak::new() );
        if let Some( cache ) = cache_weak.upgrade() {
            let mut entries = cache.entries.lock();
            entries.push( CacheEntry::pack( frames, cache_weak ) );
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
    out.reserve_from_cache();
    debug_assert!( out.frames.is_empty() );

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
