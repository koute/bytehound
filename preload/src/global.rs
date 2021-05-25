use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::ops::Deref;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;

use crate::arc_lite::ArcLite;
use crate::event::{InternalEvent, send_event};
use crate::spin_lock::{SpinLock, SpinLockGuard};
use crate::syscall;
use crate::unwind::{ThreadUnwindState, prepare_to_start_unwinding};
use crate::timestamp::Timestamp;

pub type RawThreadHandle = ArcLite< ThreadData >;

struct ThreadRegistry {
    pub enabled_for_new_threads: bool,
    threads: Option< HashMap< u32, RawThreadHandle > >,
    dead_thread_queue: Vec< (Timestamp, RawThreadHandle) >
}

unsafe impl Send for ThreadRegistry {}

impl ThreadRegistry {
    fn threads( &mut self ) -> &mut HashMap< u32, RawThreadHandle > {
        self.threads.get_or_insert_with( HashMap::new )
    }
}

const STATE_UNINITIALIZED: usize = 0;
const STATE_DISABLED: usize = 1;
const STATE_STARTING: usize = 2;
const STATE_ENABLED: usize = 3;
const STATE_STOPPING: usize = 4;
const STATE_PERMANENTLY_DISABLED: usize = 5;
static STATE: AtomicUsize = AtomicUsize::new( STATE_UNINITIALIZED );

static THREAD_RUNNING: AtomicBool = AtomicBool::new( false );
static ENABLED_BY_USER: AtomicBool = AtomicBool::new( false );

static THREAD_REGISTRY: SpinLock< ThreadRegistry > = SpinLock::new( ThreadRegistry {
    enabled_for_new_threads: false,
    threads: None,
    dead_thread_queue: Vec::new()
});

static PROCESSING_THREAD_HANDLE: SpinLock< Option< std::thread::JoinHandle< () > > > = SpinLock::new( None );

pub fn toggle() {
    if STATE.load( Ordering::SeqCst ) == STATE_PERMANENTLY_DISABLED {
        return;
    }

    let value = !ENABLED_BY_USER.load( Ordering::SeqCst );
    if value {
        info!( "Tracing will be toggled ON" );
    } else {
        info!( "Tracing will be toggled OFF" );
    }

    ENABLED_BY_USER.store( value, Ordering::SeqCst );
}

pub fn enable() -> bool {
    if STATE.load( Ordering::SeqCst ) == STATE_PERMANENTLY_DISABLED {
        return false;
    }

    ENABLED_BY_USER.compare_exchange( false, true, Ordering::SeqCst, Ordering::SeqCst ).is_ok()
}

pub fn disable() -> bool {
    if STATE.load( Ordering::SeqCst ) == STATE_PERMANENTLY_DISABLED {
        return false;
    }

    ENABLED_BY_USER.compare_exchange( true, false, Ordering::SeqCst, Ordering::SeqCst ).is_ok()
}

fn is_busy() -> bool {
    let state = STATE.load( Ordering::SeqCst );
    if state == STATE_STARTING || state == STATE_STOPPING {
        return true;
    }

    let is_enabled = ENABLED_BY_USER.load( Ordering::SeqCst );
    let is_thread_running = THREAD_RUNNING.load( Ordering::SeqCst );
    if !is_enabled && is_thread_running && state == STATE_ENABLED {
        return true;
    }

    false
}

fn try_sync_processing_thread_destruction() {
    let mut handle = PROCESSING_THREAD_HANDLE.lock();
    let state = STATE.load( Ordering::SeqCst );
    if state == STATE_STOPPING || state == STATE_DISABLED {
        if let Some( handle ) = handle.take() {
            let _ = handle.join();
        }
    }
}

pub fn sync() {
    try_sync_processing_thread_destruction();

    while is_busy() {
        thread::sleep( std::time::Duration::from_millis( 1 ) );
    }

    try_sync_processing_thread_destruction();
}

pub extern fn on_exit() {
    if STATE.load( Ordering::SeqCst ) == STATE_PERMANENTLY_DISABLED {
        return;
    }

    info!( "Exit hook called" );

    ENABLED_BY_USER.store( false, Ordering::SeqCst );
    send_event( InternalEvent::Exit );

    let mut count = 0;
    while THREAD_RUNNING.load( Ordering::SeqCst ) == true && count < 2000 {
        unsafe {
            libc::usleep( 25 * 1000 );
            count += 1;
        }
    }

    info!( "Exit hook finished" );
}

pub unsafe extern fn on_fork() {
    STATE.store( STATE_PERMANENTLY_DISABLED, Ordering::SeqCst );
    ENABLED_BY_USER.store( false, Ordering::SeqCst );
    THREAD_RUNNING.store( false, Ordering::SeqCst );
    THREAD_REGISTRY.force_unlock(); // In case we were forked when the lock was held.
    {
        let tid = syscall::gettid();
        let mut registry = THREAD_REGISTRY.lock();
        registry.enabled_for_new_threads = false;
        registry.threads().retain( |&thread_id, _| {
            thread_id == tid
        });
    }

    TLS.with( |tls| tls.set_enabled( false ) );
}

fn spawn_processing_thread() {
    info!( "Spawning event processing thread..." );

    let mut thread_handle = PROCESSING_THREAD_HANDLE.lock();
    assert!( !THREAD_RUNNING.load( Ordering::SeqCst ) );

    let new_handle = thread::Builder::new().name( "mem-prof".into() ).spawn( move || {
        TLS.with( |tls| {
            unsafe {
                *tls.is_internal.get() = true;
            }
            assert!( !tls.is_enabled() );
        });

        THREAD_RUNNING.store( true, Ordering::SeqCst );

        let result = std::panic::catch_unwind( || {
            crate::processing_thread::thread_main();
        });

        if result.is_err() {
            ENABLED_BY_USER.store( false, Ordering::SeqCst );
        }

        let mut thread_registry = THREAD_REGISTRY.lock();
        thread_registry.enabled_for_new_threads = false;
        for tls in thread_registry.threads().values() {
            if tls.is_internal() {
                continue;
            }

            debug!( "Disabling thread {:04x}...", tls.thread_id );
            tls.set_enabled( false );
            tls.unwind_cache.clear();
        }

        STATE.store( STATE_DISABLED, Ordering::SeqCst );
        info!( "Tracing was disabled" );

        THREAD_RUNNING.store( false, Ordering::SeqCst );

        if let Err( err ) = result {
            std::panic::resume_unwind( err );
        }
    }).expect( "failed to start the main memory profiler thread" );

    while THREAD_RUNNING.load( Ordering::SeqCst ) == false {
        thread::yield_now();
    }

    *thread_handle = Some( new_handle );
}

#[cold]
#[inline(never)]
fn try_enable( state: usize ) -> bool {
    if state == STATE_UNINITIALIZED {
        STATE.store( STATE_DISABLED, Ordering::SeqCst );
        crate::init::startup();
    }

    if !ENABLED_BY_USER.load( Ordering::Relaxed ) {
        return false;
    }

    if STATE.compare_exchange( STATE_DISABLED, STATE_STARTING, Ordering::SeqCst, Ordering::SeqCst ).is_err() {
        return false;
    }

    static LOCK: SpinLock< () > = SpinLock::new(());
    let mut _lock = match LOCK.try_lock() {
        Some( guard ) => guard,
        None => {
            return false;
        }
    };

    {
        let thread_registry = THREAD_REGISTRY.lock();
        assert!( !thread_registry.enabled_for_new_threads );
    }

    prepare_to_start_unwinding();
    spawn_processing_thread();

    {
        let mut thread_registry = THREAD_REGISTRY.lock();
        thread_registry.enabled_for_new_threads = true;
        for tls in thread_registry.threads().values() {
            if tls.is_internal() {
                continue;
            }

            debug!( "Enabling thread {:04x}...", tls.thread_id );
            tls.set_enabled( true );
        }
    }

    STATE.store( STATE_ENABLED, Ordering::SeqCst );
    info!( "Tracing was enabled" );

    true
}

pub fn try_disable_if_requested() {
    if ENABLED_BY_USER.load( Ordering::Relaxed ) {
        return;
    }

    if STATE.compare_exchange( STATE_ENABLED, STATE_STOPPING, Ordering::SeqCst, Ordering::SeqCst ).is_err() {
        return;
    }

    send_event( InternalEvent::Exit );
}

const THROTTLE_LIMIT: usize = 8192;

#[cold]
#[inline(never)]
fn throttle( tls: &RawThreadHandle ) {
    while ArcLite::get_refcount_relaxed( tls ) >= THROTTLE_LIMIT {
        thread::yield_now();
    }
}

/// A handle to per-thread storage; you can't do anything with it.
///
/// Can be sent to other threads.
pub struct WeakThreadHandle( RawThreadHandle );
unsafe impl Send for WeakThreadHandle {}
unsafe impl Sync for WeakThreadHandle {}

impl WeakThreadHandle {
    pub fn tid( &self ) -> u32 {
        self.0.thread_id
    }
}

/// A handle to per-thread storage.
///
/// Can only be aquired for the current thread, and cannot be sent to other threads.
pub struct StrongThreadHandle( Option< RawThreadHandle > );

impl StrongThreadHandle {
    #[cold]
    #[inline(never)]
    fn acquire_slow() -> Option< Self > {
        let current_thread_id = syscall::gettid();
        let mut registry = THREAD_REGISTRY.lock();
        if let Some( thread ) = registry.threads().get( &current_thread_id ) {
            debug!( "Acquired a dead thread: {:04X}", current_thread_id );
            Some( StrongThreadHandle( Some( thread.clone() ) ) )
        } else {
            warn!( "Failed to acquire a handle for thread: {:04X}", current_thread_id );
            None
        }
    }

    #[inline(always)]
    pub fn acquire() -> Option< Self > {
        let state = STATE.load( Ordering::Relaxed );
        if state != STATE_ENABLED {
            if !try_enable( state ) {
                return None;
            }
        }

        let tls = TLS.with( |tls| {
            if ArcLite::get_refcount_relaxed( tls ) >= THROTTLE_LIMIT {
                throttle( tls );
            }

            if !tls.is_enabled() {
                None
            } else {
                tls.set_enabled( false );
                Some( tls.0.clone() )
            }
        });

        match tls {
            Some( Some( tls ) ) => {
                Some( StrongThreadHandle( Some( tls ) ) )
            },
            Some( None ) => {
                None
            },
            None => {
                Self::acquire_slow()
            }
        }
    }

    pub fn decay( mut self ) -> WeakThreadHandle {
        let tls = match self.0.take() {
            Some( tls ) => tls,
            None => unsafe { std::hint::unreachable_unchecked() }
        };

        tls.set_enabled( true );
        WeakThreadHandle( tls )
    }

    pub fn unwind_state( &mut self ) -> &mut ThreadUnwindState {
        let tls = match self.0.as_ref() {
            Some( tls ) => tls,
            None => unsafe { std::hint::unreachable_unchecked() }
        };

        unsafe {
            &mut *tls.unwind_state.get()
        }
    }

    pub fn unwind_cache( &self ) -> &Arc< crate::unwind::Cache > {
        let tls = match self.0.as_ref() {
            Some( tls ) => tls,
            None => unsafe { std::hint::unreachable_unchecked() }
        };

        &tls.unwind_cache
    }
}

impl Drop for StrongThreadHandle {
    fn drop( &mut self ) {
        if let Some( tls ) = self.0.take() {
            tls.set_enabled( true );
        }
    }
}

pub struct AllocationLock {
    current_thread_id: u32,
    registry_lock: SpinLockGuard< 'static, ThreadRegistry >
}

impl AllocationLock {
    pub fn new() -> Self {
        let mut registry_lock = THREAD_REGISTRY.lock();
        let current_thread_id = syscall::gettid();
        let threads = registry_lock.threads();
        for (&thread_id, tls) in threads.iter_mut() {
            if thread_id == current_thread_id {
                continue;
            }

            if tls.is_internal() {
                continue;
            }
            unsafe {
                ArcLite::add( tls, THROTTLE_LIMIT );
            }
        }

        std::sync::atomic::fence( Ordering::SeqCst );

        for (&thread_id, tls) in threads.iter_mut() {
            if thread_id == current_thread_id {
                continue;
            }

            if tls.is_internal() {
                continue;
            }
            while ArcLite::get_refcount_relaxed( tls ) != THROTTLE_LIMIT {
                thread::yield_now();
            }
        }

        std::sync::atomic::fence( Ordering::SeqCst );

        AllocationLock {
            current_thread_id,
            registry_lock
        }
    }
}

impl Drop for AllocationLock {
    fn drop( &mut self ) {
        for (&thread_id, tls) in self.registry_lock.threads().iter_mut() {
            if thread_id == self.current_thread_id {
                continue;
            }

            unsafe {
                ArcLite::sub( tls, THROTTLE_LIMIT );
            }
        }
    }
}

pub struct ThreadData {
    thread_id: u32,
    is_internal: UnsafeCell< bool >,
    enabled: AtomicBool,
    unwind_cache: Arc< crate::unwind::Cache >,
    unwind_state: UnsafeCell< ThreadUnwindState >
}

impl ThreadData {
    #[inline(always)]
    pub fn is_enabled( &self ) -> bool {
        self.enabled.load( Ordering::Relaxed )
    }

    #[inline(always)]
    pub fn is_internal( &self ) -> bool {
        unsafe {
            *self.is_internal.get()
        }
    }

    fn set_enabled( &self, value: bool ) {
        self.enabled.store( value, Ordering::Relaxed )
    }
}

struct ThreadSentinel( RawThreadHandle );

impl Deref for ThreadSentinel {
    type Target = RawThreadHandle;
    fn deref( &self ) -> &Self::Target {
        &self.0
    }
}

impl Drop for ThreadSentinel {
    fn drop( &mut self ) {
        let mut registry = THREAD_REGISTRY.lock();
        if let Some( thread ) = registry.threads().get( &self.thread_id ) {
            let thread = thread.clone();
            registry.dead_thread_queue.push( (crate::timestamp::get_timestamp(), thread) );
        }

        debug!( "Thread dropped: {:04X}", self.thread_id );
    }
}

thread_local_reentrant! {
    static TLS: ThreadSentinel = |callback| {
        let thread_id = syscall::gettid();
        let mut registry = THREAD_REGISTRY.lock();

        let tls = ThreadData {
            thread_id,
            is_internal: UnsafeCell::new( false ),
            enabled: AtomicBool::new( registry.enabled_for_new_threads ),
            unwind_cache: Arc::new( crate::unwind::Cache::new() ),
            unwind_state: UnsafeCell::new( ThreadUnwindState::new() )
        };

        let tls = ArcLite::new( tls );
        registry.threads().insert( thread_id, tls.clone() );

        callback( ThreadSentinel( tls ) )
    };
}

pub fn garbage_collect_dead_threads( now: Timestamp ) {
    use std::collections::hash_map::Entry;

    let mut registry = THREAD_REGISTRY.lock();
    let registry = &mut *registry;

    if registry.dead_thread_queue.is_empty() {
        return;
    }

    let count = registry.dead_thread_queue.iter()
        .take_while( |&(time_of_death, _)| time_of_death.as_secs() + 3 < now.as_secs() )
        .count();

    if count == 0 {
        return;
    }

    let threads = registry.threads.get_or_insert_with( HashMap::new );
    for (_, thread) in registry.dead_thread_queue.drain( ..count ) {
        if let Entry::Occupied( entry ) = threads.entry( thread.thread_id ) {
            if RawThreadHandle::ptr_eq( entry.get(), &thread ) {
                entry.remove_entry();
            }
        }
    }
}
