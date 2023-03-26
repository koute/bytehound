use libc::{self, c_int, c_void, uintptr_t};
use nwind::{LocalAddressSpace, LocalAddressSpaceOptions, LocalUnwindContext, UnwindControl};
use perf_event_open::{Event, EventSource, Perf};
use std::mem::{self, transmute};
use std::sync::atomic::{AtomicU64, AtomicUsize};
use std::sync::{RwLock, RwLockReadGuard};

use crate::global::StrongThreadHandle;
use crate::nohash::NoHash;
use crate::opt;
use crate::spin_lock::SpinLock;

#[repr(C)]
pub struct BacktraceHeader {
    pub key: u64,
    pub id: AtomicU64,
    counter: AtomicUsize,
    length: usize,
}

pub struct Backtrace(std::ptr::NonNull<BacktraceHeader>);
unsafe impl Send for Backtrace {}
unsafe impl Sync for Backtrace {}

impl Backtrace {
    pub fn ptr_eq(lhs: &Backtrace, rhs: &Backtrace) -> bool {
        lhs.0.as_ptr() == rhs.0.as_ptr()
    }

    pub fn key(&self) -> u64 {
        self.header().key
    }

    pub fn id(&self) -> Option<u64> {
        let id = self.header().id.load(std::sync::atomic::Ordering::Relaxed);
        if id == 0 {
            None
        } else {
            Some(id)
        }
    }

    pub fn set_id(&self, value: u64) {
        self.header()
            .id
            .store(value, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn frames(&self) -> &[usize] {
        let length = self.header().length;
        unsafe {
            let ptr = (self.0.as_ptr() as *const BacktraceHeader as *const u8)
                .add(std::mem::size_of::<BacktraceHeader>()) as *const usize;
            std::slice::from_raw_parts(ptr, length)
        }
    }
}

impl Clone for Backtrace {
    #[inline]
    fn clone(&self) -> Self {
        self.header()
            .counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Backtrace(self.0.clone())
    }
}

impl Drop for Backtrace {
    #[inline]
    fn drop(&mut self) {
        unsafe {
            if self
                .header()
                .counter
                .fetch_sub(1, std::sync::atomic::Ordering::Release)
                != 1
            {
                return;
            }

            std::sync::atomic::fence(std::sync::atomic::Ordering::Acquire);
            self.drop_slow();
        }
    }
}

impl Backtrace {
    fn new(key: u64, backtrace: &[usize]) -> Self {
        unsafe {
            let length = backtrace.len();
            let layout = std::alloc::Layout::from_size_align(
                std::mem::size_of::<BacktraceHeader>() + std::mem::size_of::<usize>() * length,
                8,
            )
            .unwrap();
            let memory = std::alloc::alloc(layout) as *mut BacktraceHeader;
            std::ptr::write(
                memory,
                BacktraceHeader {
                    key,
                    id: AtomicU64::new(0),
                    counter: AtomicUsize::new(1),
                    length,
                },
            );
            std::ptr::copy_nonoverlapping(
                backtrace.as_ptr(),
                (memory as *mut u8).add(std::mem::size_of::<BacktraceHeader>()) as *mut usize,
                length,
            );

            Backtrace(std::ptr::NonNull::new_unchecked(memory))
        }
    }

    #[inline(never)]
    unsafe fn drop_slow(&mut self) {
        let length = self.header().length;
        let layout = std::alloc::Layout::from_size_align(
            std::mem::size_of::<BacktraceHeader>() + std::mem::size_of::<usize>() * length,
            8,
        )
        .unwrap();
        std::alloc::dealloc(self.0.as_ptr() as *mut u8, layout);
    }

    #[inline]
    fn header(&self) -> &BacktraceHeader {
        unsafe { self.0.as_ref() }
    }
}

pub struct ThreadUnwindState {
    unwind_ctx: LocalUnwindContext,
    last_dl_state: (u64, u64),
    current_backtrace: Vec<usize>,
    buffer: Vec<usize>,
    cache: lru::LruCache<u64, Backtrace, NoHash>,
}

impl ThreadUnwindState {
    pub fn new() -> Self {
        ThreadUnwindState {
            unwind_ctx: LocalUnwindContext::new(),
            last_dl_state: (0, 0),
            current_backtrace: Vec::new(),
            buffer: Vec::new(),
            cache: lru::LruCache::with_hasher(
                crate::opt::get().backtrace_cache_size_level_1,
                NoHash,
            ),
        }
    }
}

type Context = *mut c_void;
type ReasonCode = c_int;
type Callback = extern "C" fn(Context, *mut c_void) -> ReasonCode;

extern "C" {
    fn _Unwind_Backtrace(callback: Callback, data: *mut c_void) -> ReasonCode;
    fn _Unwind_GetIP(context: Context) -> uintptr_t;
    fn _Unwind_VRS_Get(
        context: Context,
        regclass: _Unwind_VRS_RegClass,
        regno: u32,
        repr: _Unwind_VRS_DataRepresentation,
        valuep: *mut c_void,
    ) -> _Unwind_VRS_Result;
}

#[allow(non_camel_case_types)]
#[repr(C)]
enum _Unwind_VRS_RegClass {
    _UVRSC_CORE = 0,
    _UVRSC_VFP = 1,
    _UVRSC_WMMXD = 3,
    _UVRSC_WMMXC = 4,
}

#[allow(non_camel_case_types)]
#[repr(C)]
enum _Unwind_VRS_DataRepresentation {
    _UVRSD_UINT32 = 0,
    _UVRSD_VFPX = 1,
    _UVRSD_UINT64 = 3,
    _UVRSD_FLOAT = 4,
    _UVRSD_DOUBLE = 5,
}

#[allow(non_camel_case_types)]
#[repr(C)]
enum _Unwind_VRS_Result {
    _UVRSR_OK = 0,
    _UVRSR_NOT_IMPLEMENTED = 1,
    _UVRSR_FAILED = 2,
}

#[cfg(not(target_arch = "arm"))]
unsafe fn get_ip(context: Context) -> uintptr_t {
    _Unwind_GetIP(context)
}

#[cfg(target_arch = "arm")]
unsafe fn get_gr(context: Context, index: c_int) -> uintptr_t {
    let mut value: uintptr_t = 0;
    _Unwind_VRS_Get(
        context,
        _Unwind_VRS_RegClass::_UVRSC_CORE,
        index as u32,
        _Unwind_VRS_DataRepresentation::_UVRSD_UINT32,
        &mut value as *mut uintptr_t as *mut c_void,
    );
    value
}

#[cfg(target_arch = "arm")]
unsafe fn get_ip(context: Context) -> uintptr_t {
    get_gr(context, 15) & (!(0x1 as uintptr_t))
}

extern "C" fn on_backtrace(context: Context, data: *mut c_void) -> ReasonCode {
    unsafe {
        let out: &mut Vec<usize> = transmute(data);
        out.push(get_ip(context) as usize);
    }

    0
}

lazy_static! {
    static ref AS: RwLock<LocalAddressSpace> = {
        let opts = LocalAddressSpaceOptions::new()
            .should_load_symbols(cfg!(feature = "debug-logs") && log_enabled!(::log::Level::Debug));

        let mut address_space = LocalAddressSpace::new_with_opts(opts).unwrap();
        address_space.use_shadow_stack(opt::get().enable_shadow_stack);
        RwLock::new(address_space)
    };
}

pub unsafe fn register_frame_by_pointer(fde: *const u8) {
    AS.write().unwrap().register_fde_from_pointer(fde)
}

pub fn deregister_frame_by_pointer(fde: *const u8) {
    AS.write().unwrap().unregister_fde_from_pointer(fde)
}

static mut PERF: Option<SpinLock<Perf>> = None;

pub fn prepare_to_start_unwinding() {
    static FLAG: SpinLock<bool> = SpinLock::new(false);
    let mut flag = FLAG.lock();
    if *flag {
        return;
    }
    *flag = true;

    if !opt::get().use_perf_event_open {
        return;
    }

    if unsafe { PERF.is_some() } {
        return;
    }

    let perf = Perf::build()
        .any_cpu()
        .event_source(EventSource::SwDummy)
        .open();

    match perf {
        Ok(perf) => unsafe {
            PERF = Some(SpinLock::new(perf));
        },
        Err(error) => {
            warn!("Failed to initialize perf_event_open: {}", error);
        }
    }
}

fn reload() {
    let mut address_space = AS.write().unwrap();
    info!("Reloading address space");
    let timestamp = crate::timestamp::get_timestamp();
    let update = address_space.reload().unwrap();
    crate::event::send_event(crate::event::InternalEvent::AddressSpaceUpdated {
        timestamp,
        maps: update.maps,
        new_binaries: update.new_binaries,
    });
}

fn reload_if_necessary_perf_event_open(
    perf: &SpinLock<Perf>,
) -> RwLockReadGuard<'static, LocalAddressSpace> {
    if unsafe { perf.unsafe_as_ref().are_events_pending() } {
        let mut perf = perf.lock();
        let mut reload_address_space = false;
        for event in perf.iter() {
            match event.get() {
                Event::Mmap2(ref event)
                    if event.filename != b"//anon"
                        && event.inode != 0
                        && event.protection & libc::PROT_EXEC as u32 != 0 =>
                {
                    debug!("New executable region mmaped: {:?}", event);
                    reload_address_space = true;
                }
                Event::Lost(_) => {
                    debug!("Lost events; forcing a reload");
                    reload_address_space = true;
                }
                _ => {}
            }
        }

        if reload_address_space {
            reload();
        }
    }

    AS.read().unwrap()
}

fn reload_if_necessary_dl_iterate_phdr(
    last_state: &mut (u64, u64),
) -> RwLockReadGuard<'static, LocalAddressSpace> {
    let dl_state = get_dl_state();
    if *last_state != dl_state {
        *last_state = dl_state;
        reload();
    }

    AS.read().unwrap()
}

fn get_dl_state() -> (u64, u64) {
    unsafe extern "C" fn callback(
        info: *mut libc::dl_phdr_info,
        _: libc::size_t,
        data: *mut libc::c_void,
    ) -> libc::c_int {
        let out = &mut *(data as *mut (u64, u64));
        out.0 = (*info).dlpi_adds;
        out.1 = (*info).dlpi_subs;
        1
    }

    unsafe {
        let mut out: (u64, u64) = (0, 0);
        libc::dl_iterate_phdr(Some(callback), &mut out as *mut _ as *mut libc::c_void);
        out
    }
}

#[inline(never)]
#[cold]
fn on_broken_unwinding(last_backtrace_depth: usize, stale_frame_count: usize) {
    error!(
        "Unwinding is totally broken; last backtrace was {} frames long, and yet we've apparently popped {} frames since last unwind",
        last_backtrace_depth,
        stale_frame_count
    );

    unsafe {
        libc::abort();
    }
}

#[inline(always)]
pub fn grab(tls: &mut StrongThreadHandle) -> Backtrace {
    unsafe {
        let (is_unwinding, unwind_state) = tls.unwind_state();
        *is_unwinding.get() = true;
        let backtrace = grab_with_unwind_state(&mut *unwind_state.get());
        *is_unwinding.get() = false;
        backtrace
    }
}

#[inline(always)]
pub fn grab_from_any(tls: &mut crate::global::ThreadHandleKind) -> Option<Backtrace> {
    match tls {
        crate::global::ThreadHandleKind::Strong(tls) => Some(grab(tls)),
        crate::global::ThreadHandleKind::Weak(tls) => unsafe {
            let (is_unwinding, unwind_state) = tls.unwind_state();
            if *is_unwinding.get() {
                return None;
            }

            *is_unwinding.get() = true;
            let backtrace = grab_with_unwind_state(&mut *unwind_state.get());
            *is_unwinding.get() = false;
            Some(backtrace)
        },
    }
}

#[inline(never)]
fn grab_with_unwind_state(unwind_state: &mut ThreadUnwindState) -> Backtrace {
    let unwind_ctx = &mut unwind_state.unwind_ctx;

    let address_space = unsafe {
        if let Some(ref perf) = PERF {
            reload_if_necessary_perf_event_open(perf)
        } else {
            reload_if_necessary_dl_iterate_phdr(&mut unwind_state.last_dl_state)
        }
    };

    let stale_count;
    let debug_crosscheck_unwind_results =
        opt::crosscheck_unwind_results_with_libunwind() && !address_space.is_shadow_stack_enabled();
    if debug_crosscheck_unwind_results || !opt::emit_partial_backtraces() {
        stale_count = unwind_state.current_backtrace.len();

        let buffer = &mut unwind_state.buffer;
        buffer.clear();

        address_space.unwind(unwind_ctx, |address| {
            buffer.push(address);
            UnwindControl::Continue
        });
    } else {
        let buffer = &mut unwind_state.buffer;
        buffer.clear();

        let stale_count_opt = address_space.unwind_through_fresh_frames(unwind_ctx, |address| {
            buffer.push(address);
            UnwindControl::Continue
        });

        let last_backtrace_depth = unwind_state.current_backtrace.len();
        let mut new_backtrace_depth = buffer.len();

        if let Some(stale_frame_count) = stale_count_opt {
            if stale_frame_count > last_backtrace_depth {
                on_broken_unwinding(last_backtrace_depth, stale_frame_count);
            } else {
                new_backtrace_depth += last_backtrace_depth - stale_frame_count;
                if cfg!(feature = "debug-logs") {
                    debug!(
                        "Finished unwinding; backtrace depth: {} (fresh = {}, non-fresh = {})",
                        new_backtrace_depth,
                        buffer.len(),
                        last_backtrace_depth - stale_frame_count
                    );
                }
            }
        } else {
            if cfg!(feature = "debug-logs") {
                debug!(
                    "Finished unwinding; backtrace depth: {}",
                    new_backtrace_depth
                );
            }
        }

        stale_count = stale_count_opt.unwrap_or(unwind_state.current_backtrace.len());
    }

    mem::drop(address_space);

    let remaining = unwind_state.current_backtrace.len() - stale_count;
    unwind_state.current_backtrace.truncate(remaining);
    unwind_state
        .current_backtrace
        .reserve(unwind_state.buffer.len());

    const PRIME: u64 = 1099511628211;
    let mut key: u64 = 0;
    for &frame in &unwind_state.current_backtrace {
        key = key.wrapping_mul(PRIME);
        key ^= frame as u64;
    }
    for &frame in unwind_state.buffer.iter().rev() {
        key = key.wrapping_mul(PRIME);
        key ^= frame as u64;
        unwind_state.current_backtrace.push(frame);
    }
    unwind_state.buffer.clear();

    let backtrace = match unwind_state.cache.get_mut(&key) {
        None => {
            if cfg!(debug_assertions) {
                if unwind_state.cache.len() >= unwind_state.cache.cap() {
                    debug!("1st level backtrace cache overflow");
                }
            }

            let entry = Backtrace::new(key, &unwind_state.current_backtrace);
            unwind_state.cache.put(key, entry.clone());

            entry
        }
        Some(entry) => {
            if entry.frames() == unwind_state.current_backtrace {
                entry.clone()
            } else {
                info!("1st level backtrace cache conflict detected!");

                let new_entry = Backtrace::new(key, &unwind_state.current_backtrace);
                *entry = new_entry.clone();

                new_entry
            }
        }
    };

    if debug_crosscheck_unwind_results {
        let mut expected: Vec<usize> = Vec::with_capacity(backtrace.frames().len());
        unsafe {
            _Unwind_Backtrace(on_backtrace, transmute(&mut expected));
        }

        if expected.last() == Some(&0) {
            expected.pop();
        }

        expected.reverse();
        if backtrace.frames()[..backtrace.frames().len() - 1] != expected[..expected.len() - 1] {
            info!(
                "/proc/self/maps:\n{}",
                String::from_utf8_lossy(&::std::fs::read("/proc/self/maps").unwrap()).trim()
            );

            let address_space = AS.read().unwrap();
            info!("Expected: ({} frames)", expected.len());
            for (nth, &address) in expected.iter().enumerate() {
                info!(
                    "({:02})    {:?}",
                    nth,
                    address_space.decode_symbol_once(address)
                );
            }

            info!("Actual: ({} frames)", backtrace.frames().len());
            for (nth, &address) in backtrace.frames().iter().enumerate() {
                info!(
                    "({:02})    {:?}",
                    nth,
                    address_space.decode_symbol_once(address)
                );
            }

            panic!(
                "Wrong backtrace; expected: {:?}, got: {:?}",
                expected,
                backtrace.frames()
            );
        }
    }

    backtrace
}
