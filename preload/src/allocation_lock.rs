use std::thread;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::Ordering;
use std::collections::HashMap;

use crate::arc_counter::ArcCounter;
use crate::spin_lock::SpinLockGuard;
use crate::syscall;
use crate::tls::{Tls, get_tls};

static THROTTLE_LIMIT: usize = 8192;

fn is_tracing_enabled() -> bool {
    crate::TRACING_ENABLED.load( Ordering::Relaxed )
}

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

pub struct RecursionLock< 'a > {
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
