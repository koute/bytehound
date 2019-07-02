use std::cell::UnsafeCell;
use std::sync::Arc;
use std::collections::HashMap;
use std::mem;
use std::ptr;

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

pub static THROTTLE_FOR_THREAD: SpinLock< Option< HashMap< u32, ArcCounter > > > = spin_lock_new!( None );

impl Drop for Tls {
    fn drop( &mut self ) {
        self.on_application_thread = false;
        let mut throttle_for_thread_map = THROTTLE_FOR_THREAD.lock();
        let throttle_for_thread_map = throttle_for_thread_map.as_mut().unwrap();
        throttle_for_thread_map.remove( &self.thread_id );
    }
}

#[cold]
#[inline(never)]
fn construct_tls( cell: *mut (*mut Tls, bool) ) -> *mut Tls {
    let was_already_initialized = unsafe { (*cell).1 };
    if was_already_initialized {
        return ptr::null_mut();
    }

    unsafe {
        (*cell).1 = true;
    }

    let thread_id = syscall::gettid();
    let on_application_thread = *ON_APPLICATION_THREAD_DEFAULT.lock();
    let backtrace_cache = Arc::new( Cache::new() );
    let throttle_state = ArcCounter::new();
        let unwind_ctx = LocalUnwindContext::new();

    {
        let mut throttle_for_thread_map = THROTTLE_FOR_THREAD.lock();
        throttle_for_thread_map.get_or_insert_with( HashMap::new ).insert( thread_id, throttle_state.clone() );
    }

    let tls = Tls {
        thread_id,
        on_application_thread,
        backtrace_cache,
        throttle_state,
        unwind_ctx
    };

    // Currently Rust triggers an allocation when registering
    // the TLS destructor, so we do it manually ourselves to avoid
    // an infinite loop.

    let tls = Box::into_raw( Box::new( tls ) );
    unsafe {
        *cell = (tls, true);
        libc::pthread_setspecific( KEY, cell as *const libc::c_void );
    }

    tls
}

const EMPTY_TLS: UnsafeCell< (*mut Tls, bool) > = UnsafeCell::new( (0 as _, false) );

thread_local! {
    static TLS: UnsafeCell< (*mut Tls, bool) > = EMPTY_TLS;
}

static mut KEY: libc::pthread_key_t = 0;

unsafe extern fn destructor( cell: *mut libc::c_void ) {
    let cell = cell as *mut (*mut Tls, bool);
    let tls = ptr::replace( cell, (ptr::null_mut(), true) ).0;
    if !tls.is_null() {
        mem::drop( Box::from_raw( tls ) );
    }
}

pub unsafe fn initialize_tls() {
    libc::pthread_key_create( &mut KEY, Some( destructor ) );
}

pub fn get_tls() -> Option< &'static mut Tls > {
    unsafe {
        let mut ptr: *mut Tls = 0 as _;
        let _ = TLS.try_with( |cell| {
            let cell = cell.get();
            ptr = (*cell).0;
            if !(*cell).1 {
                ptr = construct_tls( cell );
            }
        });

        if ptr.is_null() {
            return None;
        }

        Some( &mut *ptr )
    }
}
