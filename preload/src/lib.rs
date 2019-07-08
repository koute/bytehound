#[macro_use]
extern crate log;

#[macro_use]
extern crate lazy_static;

#[cfg(feature = "sc")]
#[macro_use]
extern crate sc;

use std::thread;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::fs::read_link;
use std::collections::HashMap;

use std::os::unix::ffi::OsStrExt;

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
mod event;
mod init;
mod processing_thread;

use crate::arc_counter::ArcCounter;
use crate::event::InternalEvent;
use crate::spin_lock::{SpinLock, SpinLockGuard};
use crate::tls::{Tls, get_tls};
use crate::utils::read_file;

#[global_allocator]
static mut ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;

pub(crate) const PAGE_SIZE: usize = 4096;

lazy_static! {
    pub(crate) static ref PID: u32 = {
        let pid = unsafe { libc::getpid() } as u32;
        pid
    };
    pub(crate) static ref CMDLINE: Vec< u8 > = {
        read_file( "/proc/self/cmdline" ).unwrap()
    };
    pub(crate) static ref EXECUTABLE: Vec< u8 > = {
        let executable: Vec< u8 > = read_link( "/proc/self/exe" ).unwrap().as_os_str().as_bytes().into();
        executable
    };
}

pub(crate) static TRACING_ENABLED: AtomicBool = AtomicBool::new( false );

pub(crate) static ON_APPLICATION_THREAD_DEFAULT: SpinLock< bool > = SpinLock::new( false );

fn is_tracing_enabled() -> bool {
    TRACING_ENABLED.load( Ordering::Relaxed )
}

pub(crate) static RUNNING: AtomicBool = AtomicBool::new( true );

static THROTTLE_LIMIT: usize = 8192;

pub(crate) struct ThrottleHandle( ArcCounter );
impl ThrottleHandle {
    fn new( tls: &Tls ) -> Self {
        let state = &tls.throttle_state;
        while state.get() >= THROTTLE_LIMIT {
            thread::yield_now();
        }

        ThrottleHandle( state.clone() )
    }
}

pub(crate) struct AllocationLock {
    current_thread_id: u32,
    throttle_for_thread_map: SpinLockGuard< 'static, Option< HashMap< u32, ArcCounter > > >
}

impl AllocationLock {
    pub(crate) fn new() -> Self {
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
        crate::init::initialize();
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
    memory_profiler_raw_mmap,
    memory_profiler_raw_munmap,

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
