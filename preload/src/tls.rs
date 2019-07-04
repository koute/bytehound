use std::sync::Arc;
use std::collections::HashMap;

use nwind::LocalUnwindContext;

use crate::arc_counter::ArcCounter;
use crate::unwind::Cache;
use crate::spin_lock::SpinLock;
use crate::syscall;
use crate::ON_APPLICATION_THREAD_DEFAULT;

pub struct Tls {
    pub thread_id: u32,
    pub on_application_thread: bool,
    pub backtrace_cache: Arc< Cache >,
    pub throttle_state: ArcCounter,
    pub unwind_ctx: LocalUnwindContext
}

pub static THROTTLE_FOR_THREAD: SpinLock< Option< HashMap< u32, ArcCounter > > > = SpinLock::new( None );

impl Drop for Tls {
    fn drop( &mut self ) {
        self.on_application_thread = false;
        let mut throttle_for_thread_map = THROTTLE_FOR_THREAD.lock();
        let throttle_for_thread_map = throttle_for_thread_map.as_mut().unwrap();
        throttle_for_thread_map.remove( &self.thread_id );
    }
}

thread_local_reentrant! {
    static TLS: Tls = {
        let thread_id = syscall::gettid();
        let on_application_thread = *ON_APPLICATION_THREAD_DEFAULT.lock();
        let backtrace_cache = Arc::new( Cache::new() );
        let throttle_state = ArcCounter::new();
        let unwind_ctx = LocalUnwindContext::new();

        {
            let mut throttle_for_thread_map = THROTTLE_FOR_THREAD.lock();
            throttle_for_thread_map.get_or_insert_with( HashMap::new ).insert( thread_id, throttle_state.clone() );
        }

        Tls {
            thread_id,
            on_application_thread,
            backtrace_cache,
            throttle_state,
            unwind_ctx
        }
    };
}

#[inline]
pub fn get_tls() -> Option< &'static mut Tls > {
    unsafe {
        TLS.get()
    }
}
