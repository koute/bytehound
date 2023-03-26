use std::cell::UnsafeCell;
use std::ops::Deref;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;

use crate::allocation_tracker::AllocationTracker;
use crate::arc_lite::ArcLite;
use crate::event::{send_event, InternalAllocationId, InternalEvent};
use crate::spin_lock::{SpinLock, SpinLockGuard};
use crate::timestamp::Timestamp;
use crate::unwind::{prepare_to_start_unwinding, ThreadUnwindState};
use crate::{opt, syscall};
use thread_local_reentrant::AccessError as TlsAccessError;

pub type RawThreadHandle = ArcLite<ThreadData>;

struct ThreadRegistry {
    enabled_for_new_threads: bool,
    threads_by_system_id: crate::utils::HashMap<u32, RawThreadHandle>,
    new_dead_thread_queue: Vec<(Timestamp, RawThreadHandle)>,
    thread_counter: u64,
}

unsafe impl Send for ThreadRegistry {}

impl ThreadRegistry {
    fn threads_by_system_id(&mut self) -> &mut crate::utils::HashMap<u32, RawThreadHandle> {
        &mut self.threads_by_system_id
    }
}

const STATE_UNINITIALIZED: usize = 0;
const STATE_INITIALIZING_STAGE_1: usize = 1;
const STATE_PARTIALLY_INITIALIZED: usize = 2;
const STATE_INITIALIZING_STAGE_2: usize = 3;
const STATE_DISABLED: usize = 4;
const STATE_STARTING: usize = 5;
const STATE_ENABLED: usize = 6;
const STATE_STOPPING: usize = 7;
const STATE_PERMANENTLY_DISABLED: usize = 8;
static STATE: AtomicUsize = AtomicUsize::new(STATE_UNINITIALIZED);

static THREAD_RUNNING: AtomicBool = AtomicBool::new(false);

const DESIRED_STATE_DISABLED: usize = 0;
const DESIRED_STATE_SUSPENDED: usize = 1;
const DESIRED_STATE_ENABLED: usize = 2;
static DESIRED_STATE: AtomicUsize = AtomicUsize::new(DESIRED_STATE_DISABLED);

static THREAD_REGISTRY: SpinLock<ThreadRegistry> = SpinLock::new(ThreadRegistry {
    enabled_for_new_threads: false,
    threads_by_system_id: crate::utils::empty_hashmap(),
    new_dead_thread_queue: Vec::new(),
    thread_counter: 1,
});

#[inline(never)]
fn lock_thread_registry<R>(callback: impl FnOnce(&mut ThreadRegistry) -> R) -> R {
    callback(&mut THREAD_REGISTRY.lock())
}

static PROCESSING_THREAD_HANDLE: SpinLock<Option<libc::pthread_t>> = SpinLock::new(None);
static mut PROCESSING_THREAD_TID: u32 = 0;

pub static mut SYM_REGISTER_FRAME: Option<unsafe extern "C" fn(fde: *const u8)> = None;
pub static mut SYM_DEREGISTER_FRAME: Option<unsafe extern "C" fn(fde: *const u8)> = None;

pub static mut INITIAL_TIMESTAMP: Timestamp = Timestamp::from_secs(0);

static NEXT_MAP_ID: AtomicU64 = AtomicU64::new(1);

pub fn next_map_id() -> u64 {
    NEXT_MAP_ID.fetch_add(1, Ordering::Relaxed)
}

pub static MMAP_REGISTRY: Mutex<crate::smaps::MapsRegistry> =
    Mutex::new(crate::smaps::MapsRegistry::new());
static mut PR_SET_VMA_ANON_NAME_SUPPORTED: bool = true;

static mut DUMMY_MEMFD: i32 = -1;

#[inline(always)]
pub fn is_pr_set_vma_anon_name_supported() -> bool {
    unsafe { crate::global::PR_SET_VMA_ANON_NAME_SUPPORTED }
}

#[inline(always)]
pub fn dummy_memfd() -> i32 {
    unsafe { DUMMY_MEMFD }
}

#[cfg(feature = "jemalloc")]
#[inline]
pub fn using_unprefixed_jemalloc() -> bool {
    true
}

#[cfg(not(feature = "jemalloc"))]
static USING_UNPREFIXED_JEMALLOC: AtomicBool = AtomicBool::new(false);

#[cfg(not(feature = "jemalloc"))]
#[inline]
pub fn using_unprefixed_jemalloc() -> bool {
    USING_UNPREFIXED_JEMALLOC.load(Ordering::Relaxed)
}

pub fn toggle() {
    if STATE.load(Ordering::SeqCst) == STATE_PERMANENTLY_DISABLED {
        return;
    }

    let value = DESIRED_STATE.load(Ordering::SeqCst);
    let new_value = match value {
        DESIRED_STATE_DISABLED => {
            info!("Tracing will be toggled ON (for the first time)");
            DESIRED_STATE_ENABLED
        }
        DESIRED_STATE_SUSPENDED => {
            info!("Tracing will be toggled ON");
            DESIRED_STATE_ENABLED
        }
        DESIRED_STATE_ENABLED => {
            info!("Tracing will be toggled OFF");
            DESIRED_STATE_SUSPENDED
        }
        _ => unreachable!(),
    };

    DESIRED_STATE.store(new_value, Ordering::SeqCst);
}

pub fn enable() -> bool {
    if STATE.load(Ordering::SeqCst) == STATE_PERMANENTLY_DISABLED {
        return false;
    }

    DESIRED_STATE.swap(DESIRED_STATE_ENABLED, Ordering::SeqCst) != DESIRED_STATE_ENABLED
}

pub fn disable() -> bool {
    if STATE.load(Ordering::SeqCst) == STATE_PERMANENTLY_DISABLED {
        return false;
    }

    DESIRED_STATE.swap(DESIRED_STATE_SUSPENDED, Ordering::SeqCst) == DESIRED_STATE_ENABLED
}

fn is_busy() -> bool {
    let state = STATE.load(Ordering::SeqCst);
    if state == STATE_STARTING || state == STATE_STOPPING {
        return true;
    }

    let requested_state = DESIRED_STATE.load(Ordering::SeqCst);
    let is_thread_running = THREAD_RUNNING.load(Ordering::SeqCst);
    if requested_state == DESIRED_STATE_DISABLED && is_thread_running && state == STATE_ENABLED {
        return true;
    }

    false
}

fn try_sync_processing_thread_destruction() {
    let mut handle = PROCESSING_THREAD_HANDLE.lock();
    let state = STATE.load(Ordering::SeqCst);
    if state == STATE_STOPPING || state == STATE_DISABLED {
        if let Some(handle) = handle.take() {
            unsafe {
                libc::pthread_join(handle, std::ptr::null_mut());
            }
        }
    }
}

pub fn sync() {
    try_sync_processing_thread_destruction();

    while is_busy() {
        thread::sleep(std::time::Duration::from_millis(1));
    }

    try_sync_processing_thread_destruction();
}

pub extern "C" fn on_exit() {
    if STATE.load(Ordering::SeqCst) == STATE_PERMANENTLY_DISABLED {
        return;
    }

    info!("Exit hook called");

    DESIRED_STATE.store(DESIRED_STATE_DISABLED, Ordering::SeqCst);
    send_event(InternalEvent::Exit);

    let mut count = 0;
    while THREAD_RUNNING.load(Ordering::SeqCst) == true && count < 2000 {
        unsafe {
            libc::usleep(25 * 1000);
            count += 1;
        }
    }

    info!("Exit hook finished");
}

pub unsafe extern "C" fn on_fork() {
    STATE.store(STATE_PERMANENTLY_DISABLED, Ordering::SeqCst);
    DESIRED_STATE.store(DESIRED_STATE_DISABLED, Ordering::SeqCst);
    THREAD_RUNNING.store(false, Ordering::SeqCst);
    THREAD_REGISTRY.force_unlock(); // In case we were forked when the lock was held.
    {
        let tid = syscall::gettid();
        let mut registry = THREAD_REGISTRY.lock();
        registry.enabled_for_new_threads = false;
        registry
            .threads_by_system_id()
            .retain(|&_, thread| thread.thread_id == tid);
    }

    let _ = TLS.try_with(|tls| tls.set_enabled(false));
}

fn spawn_processing_thread() {
    info!("Will spawn the event processing thread...");

    let mut thread_handle = PROCESSING_THREAD_HANDLE.lock();
    assert!(!THREAD_RUNNING.load(Ordering::SeqCst));

    extern "C" fn thread_main(_: *mut libc::c_void) -> *mut libc::c_void {
        info!("Processing thread created!");

        unsafe {
            PROCESSING_THREAD_TID = syscall::gettid();
        }

        THREAD_RUNNING.store(true, Ordering::SeqCst);

        TLS.try_with(|tls| {
            unsafe {
                *tls.is_internal.get() = true;
            }
            assert!(!tls.is_enabled());
        })
        .unwrap();

        let result = std::panic::catch_unwind(|| {
            crate::processing_thread::thread_main();
        });

        if result.is_err() {
            DESIRED_STATE.store(DESIRED_STATE_DISABLED, Ordering::SeqCst);
        }

        lock_thread_registry(|thread_registry| {
            thread_registry.enabled_for_new_threads = false;
            for tls in thread_registry.threads_by_system_id().values() {
                if tls.is_internal() {
                    continue;
                }

                debug!("Disabling thread {:04x}...", tls.thread_id);
                tls.set_enabled(false);
            }

            STATE.store(STATE_DISABLED, Ordering::SeqCst);
            info!("Tracing was disabled");

            THREAD_RUNNING.store(false, Ordering::SeqCst);
        });

        if let Err(err) = result {
            std::panic::resume_unwind(err);
        }

        std::ptr::null_mut()
    }

    info!("Creating the event processing thread...");
    let mut thread: libc::pthread_t;
    unsafe {
        thread = std::mem::zeroed();
        if libc::pthread_create(
            &mut thread,
            std::ptr::null(),
            thread_main,
            std::ptr::null_mut(),
        ) < 0
        {
            panic!(
                "failed to start the main memory profiler thread: {}",
                std::io::Error::last_os_error()
            );
        }
        if libc::pthread_setname_np(thread, b"mem-prof".as_ptr() as *const libc::c_char) < 0 {
            warn!(
                "Failed to set the name of the processing thread: {}",
                std::io::Error::last_os_error()
            );
        }
    }

    info!("Waiting for the event processing thread...");
    while THREAD_RUNNING.load(Ordering::SeqCst) == false {
        thread::yield_now();
    }

    *thread_handle = Some(thread);
    info!("Event processing thread created!");
}

#[cfg(target_arch = "x86_64")]
fn find_internal_syms<const N: usize>(names: &[&str; N]) -> [usize; N] {
    let mut addresses = [0; N];

    unsafe {
        use goblin::elf::section_header::{SHT_DYNSYM, SHT_SYMTAB};
        use goblin::elf::sym::sym64::Sym;
        use goblin::elf64::header::Header;
        use goblin::elf64::section_header::SectionHeader;

        let self_path = b"/proc/self/exe\0".as_ptr() as _;
        let mut fd = crate::syscall::open_raw_cstr(self_path, libc::O_RDONLY, 0);
        if fd < 0 {
            warn!(
                "failed to open /proc/self/exe: {}",
                std::io::Error::from_raw_os_error(fd)
            );
            let path = libc::getauxval(libc::AT_EXECFN) as *const libc::c_char;
            if !path.is_null() {
                fd = crate::syscall::open_raw_cstr(path, libc::O_RDONLY, 0);
                if fd < 0 {
                    panic!(
                        "failed to open {:?}: {}",
                        std::ffi::CStr::from_ptr(path),
                        std::io::Error::from_raw_os_error(fd)
                    );
                }
            } else {
                panic!("couldn't open /proc/self/exe");
            }
        }

        let mut buf: libc::stat64 = std::mem::zeroed();
        if libc::fstat64(fd as _, &mut buf) != 0 {
            panic!(
                "couldn't fstat the executable: {}",
                std::io::Error::last_os_error()
            );
        }

        let size = buf.st_size as usize;
        let executable = syscall::mmap(
            std::ptr::null_mut(),
            size,
            libc::PROT_READ,
            libc::MAP_PRIVATE,
            fd,
            0,
        );
        assert_ne!(executable, libc::MAP_FAILED);

        let elf_header = *(executable as *const Header);
        let address_offset = libc::getauxval(libc::AT_PHDR) as usize - elf_header.e_phoff as usize;

        assert_eq!(
            elf_header.e_shentsize as usize,
            std::mem::size_of::<SectionHeader>()
        );
        let section_headers = std::slice::from_raw_parts(
            ((executable as *const u8).add(elf_header.e_shoff as usize)) as *const SectionHeader,
            elf_header.e_shnum as usize,
        );

        for section_header in section_headers {
            if section_header.sh_type != SHT_SYMTAB && section_header.sh_type != SHT_DYNSYM {
                continue;
            }
            let strtab_key = section_header.sh_link as usize;
            let strtab_section_header = section_headers[strtab_key];
            let strtab_bytes = std::slice::from_raw_parts(
                (executable as *const u8).add(strtab_section_header.sh_offset as usize),
                strtab_section_header.sh_size as usize,
            );

            let syms = std::slice::from_raw_parts(
                (executable as *const u8).add(section_header.sh_offset as usize) as *const Sym,
                section_header.sh_size as usize / std::mem::size_of::<Sym>(),
            );

            for sym in syms {
                let bytes = &strtab_bytes[sym.st_name as usize..];
                let name = &bytes[..bytes
                    .iter()
                    .position(|&byte| byte == 0)
                    .unwrap_or(bytes.len())];
                for (target_name, output_address) in names.iter().zip(addresses.iter_mut()) {
                    if *output_address != 0 {
                        continue;
                    }
                    if name == target_name.as_bytes() {
                        if let Some(address) = address_offset.checked_add(sym.st_value as usize) {
                            info!("Found '{}' at: 0x{:016X}", target_name, address);
                            *output_address = address;
                            break;
                        }
                    }
                }
            }
        }

        let errcode = syscall::munmap(executable, size);
        if errcode < 0 {
            warn!(
                "munmap failed: {}",
                std::io::Error::from_raw_os_error(errcode)
            );
        }

        let errcode = syscall::close(fd);
        if errcode < 0 {
            warn!(
                "close failed: {}",
                std::io::Error::from_raw_os_error(errcode)
            );
        }
    }

    addresses
}

#[cfg(target_arch = "x86_64")]
fn hook_jemalloc() {
    let names = [
        "_rjem_malloc",
        "_rjem_mallocx",
        "_rjem_calloc",
        "_rjem_sdallocx",
        "_rjem_realloc",
        "_rjem_rallocx",
        "_rjem_nallocx",
        "_rjem_xallocx",
        "_rjem_malloc_usable_size",
        "_rjem_mallctl",
        "_rjem_posix_memalign",
        "_rjem_aligned_alloc",
        "_rjem_free",
        "_rjem_sallocx",
        "_rjem_dallocx",
        "_rjem_mallctlnametomib",
        "_rjem_mallctlbymib",
        "_rjem_malloc_stats_print",
        "_rjem_memalign",
        "_rjem_valloc",
        "malloc",
        "mallocx",
        "calloc",
        "sdallocx",
        "realloc",
        "rallocx",
        "nallocx",
        "xallocx",
        "malloc_usable_size",
        "mallctl",
        "posix_memalign",
        "aligned_alloc",
        "free",
        "sallocx",
        "dallocx",
        "mallctlnametomib",
        "mallctlbymib",
        "malloc_stats_print",
        "memalign",
        "valloc",
    ];

    let replacements = [
        // Prefixed jemalloc.
        crate::api::_rjem_malloc as usize,
        crate::api::_rjem_mallocx as usize,
        crate::api::_rjem_calloc as usize,
        crate::api::_rjem_sdallocx as usize,
        crate::api::_rjem_realloc as usize,
        crate::api::_rjem_rallocx as usize,
        crate::api::_rjem_nallocx as usize,
        crate::api::_rjem_xallocx as usize,
        crate::api::_rjem_malloc_usable_size as usize,
        crate::api::_rjem_mallctl as usize,
        crate::api::_rjem_posix_memalign as usize,
        crate::api::_rjem_aligned_alloc as usize,
        crate::api::_rjem_free as usize,
        crate::api::_rjem_sallocx as usize,
        crate::api::_rjem_dallocx as usize,
        crate::api::_rjem_mallctlnametomib as usize,
        crate::api::_rjem_mallctlbymib as usize,
        crate::api::_rjem_malloc_stats_print as usize,
        crate::api::_rjem_memalign as usize,
        crate::api::_rjem_valloc as usize,
        // Unprefixed jemalloc.
        crate::api::_rjem_malloc as usize,
        crate::api::_rjem_mallocx as usize,
        crate::api::_rjem_calloc as usize,
        crate::api::_rjem_sdallocx as usize,
        crate::api::_rjem_realloc as usize,
        crate::api::_rjem_rallocx as usize,
        crate::api::_rjem_nallocx as usize,
        crate::api::_rjem_xallocx as usize,
        crate::api::_rjem_malloc_usable_size as usize,
        crate::api::_rjem_mallctl as usize,
        crate::api::_rjem_posix_memalign as usize,
        crate::api::_rjem_aligned_alloc as usize,
        crate::api::_rjem_free as usize,
        crate::api::_rjem_sallocx as usize,
        crate::api::_rjem_dallocx as usize,
        crate::api::_rjem_mallctlnametomib as usize,
        crate::api::_rjem_mallctlbymib as usize,
        crate::api::_rjem_malloc_stats_print as usize,
        crate::api::_rjem_memalign as usize,
        crate::api::_rjem_valloc as usize,
    ];

    let addresses = find_internal_syms(&names);
    if addresses.iter().all(|&address| address == 0) {
        return;
    }

    let index_mallocx = names.iter().position(|name| *name == "mallocx").unwrap();
    let index_sdallocx = names.iter().position(|name| *name == "sdallocx").unwrap();
    let index_rallocx = names.iter().position(|name| *name == "rallocx").unwrap();

    let enable_extended_hooks = addresses[index_mallocx] != 0
        || addresses[index_sdallocx] != 0
        || addresses[index_rallocx] != 0;

    let extended_hooks_offset = names
        .iter()
        .position(|name| !name.starts_with("_rjem_"))
        .unwrap();

    if enable_extended_hooks {
        info!("Attaching prefixed jemalloc hooks...");
        hook_symbols(&names, &addresses, &replacements);
    } else {
        info!("Attaching unprefixed jemalloc hooks...");
        hook_symbols(
            &names[..extended_hooks_offset],
            &addresses[..extended_hooks_offset],
            &replacements[..extended_hooks_offset],
        );

        #[cfg(not(feature = "jemalloc"))]
        USING_UNPREFIXED_JEMALLOC.store(true, Ordering::SeqCst);
    }
}

#[cfg(target_arch = "x86_64")]
fn hook_symbols(names: &[&str], addresses: &[usize], replacements: &[usize]) {
    assert_eq!(names.len(), replacements.len());
    assert_eq!(names.len(), addresses.len());

    for ((&name, &replacement), &address) in names.iter().zip(replacements).zip(addresses) {
        if address == 0 {
            info!("Symbol not found: \"{}\"", name);
            continue;
        }

        if replacement == address {
            panic!(
                "tried to replace a symbol with itself: symbol='{}', address=0x{:016X}",
                name, replacement
            );
        }

        let page_1 = address as usize & !(4096 - 1);
        let page_2 = (address as usize + 14) & !(4096 - 1);
        let page = page_1 as *mut libc::c_void;
        let length = if page_1 == page_2 { 4096 } else { 8192 };

        unsafe {
            if libc::mprotect(
                page,
                length,
                libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC,
            ) < 0
            {
                panic!("mprotect failed: {}", std::io::Error::last_os_error());
            }

            // Write a `jmp` instruction with a RIP-relative addressing mode, with a zero displacement.
            let mut p = address as *mut u8;
            std::ptr::write_unaligned(p, 0xFF);
            p = p.add(1);
            std::ptr::write_unaligned(p, 0x25);
            p = p.add(1);
            std::ptr::write_unaligned(p, 0x00);
            p = p.add(1);
            std::ptr::write_unaligned(p, 0x00);
            p = p.add(1);
            std::ptr::write_unaligned(p, 0x00);
            p = p.add(1);
            std::ptr::write_unaligned(p, 0x00);
            p = p.add(1);
            std::ptr::write_unaligned(p as *mut usize, replacement);

            if libc::mprotect(page, length, libc::PROT_READ | libc::PROT_EXEC) < 0 {
                warn!("mprotect failed: {}", std::io::Error::last_os_error());
            }
        }
    }
}

fn resolve_original_syms() {
    unsafe {
        let register_frame = libc::dlsym(
            libc::RTLD_NEXT,
            b"__register_frame\0".as_ptr() as *const libc::c_char,
        );
        let deregister_frame = libc::dlsym(
            libc::RTLD_NEXT,
            b"__deregister_frame\0".as_ptr() as *const libc::c_char,
        );
        if register_frame.is_null() || deregister_frame.is_null() {
            if register_frame.is_null() {
                warn!("Failed to find `__register_frame` symbol");
            }
            if deregister_frame.is_null() {
                warn!("Failed to find `__deregister_frame` symbol");
            }
            return;
        }

        crate::global::SYM_REGISTER_FRAME = Some(std::mem::transmute(register_frame));
        crate::global::SYM_DEREGISTER_FRAME = Some(std::mem::transmute(deregister_frame));
    }
}

fn check_set_vma_anon_name() {
    if crate::opt::get().disable_pr_set_vma_anon_name {
        warn!("PR_SET_VMA_ANON_NAME forcibly disabled!");
        unsafe {
            PR_SET_VMA_ANON_NAME_SUPPORTED = false;
        }

        return;
    }

    unsafe {
        let pointer = crate::syscall::mmap(
            std::ptr::null_mut(),
            4096,
            0,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        );
        assert_ne!(pointer, libc::MAP_FAILED);

        let is_supported = crate::syscall::pr_set_vma_anon_name(pointer, 4096, b"test\0");
        crate::syscall::munmap(pointer, 4096);

        if !is_supported {
            warn!( "PR_SET_VMA_ANON_NAME is not supported (Linux 5.17+ required); will try to emulate in userspace" );
            PR_SET_VMA_ANON_NAME_SUPPORTED = false;
        }
    }
}

fn initialize_stage_1() {
    unsafe {
        INITIAL_TIMESTAMP = crate::timestamp::get_timestamp();
    }

    crate::init::initialize_logger();
    info!("Version: {}", env!("CARGO_PKG_VERSION"));

    unsafe {
        crate::opt::initialize();
    }

    check_set_vma_anon_name();
    if !is_pr_set_vma_anon_name_supported() {
        unsafe {
            let fd = libc::memfd_create(b"bytehound_padding\0".as_ptr().cast(), libc::MFD_CLOEXEC);
            if fd < 0 {
                error!("Failed to create a memfd for a dummy map!");
                libc::abort();
            }

            info!("Dummy memfd created: fd = {}", fd);
            DUMMY_MEMFD = fd;
        }
    }

    if !crate::opt::get().disabled_by_default {
        toggle();
    }

    #[cfg(target_arch = "x86_64")]
    hook_jemalloc();

    #[cfg(target_arch = "x86_64")]
    hook_private_mmap();

    info!("Stage 1 initialization finished");
}

#[cfg(target_arch = "x86_64")]
fn hook_private_mmap() {
    use std::ops::ControlFlow;

    let mut address_mmap = std::ptr::null_mut();
    let mut address_munmap = std::ptr::null_mut();
    crate::elf::ObjectInfo::each(|info| {
        if info.name_contains("libc.so") {
            if let Some(address) = info.dlsym("__mmap") {
                address_mmap = address;
            }
            if let Some(address) = info.dlsym("__munmap") {
                address_munmap = address;
            }

            return ControlFlow::Break(());
        }

        ControlFlow::Continue(())
    });

    if !address_mmap.is_null() {
        info!("Found __mmap at: 0x{:016X}", address_mmap as usize);
    }

    if !address_munmap.is_null() {
        info!("Found __munmap at: 0x{:016X}", address_munmap as usize);
    }

    hook_symbols(
        &["__mmap", "__munmap"],
        &[address_mmap as usize, address_munmap as usize],
        &[crate::api::__mmap as usize, crate::api::__munmap as usize],
    );
}

fn initialize_stage_2() {
    info!("Initializing stage 2...");

    crate::init::initialize_atexit_hook();
    crate::init::initialize_signal_handlers();

    if !opt::get().track_child_processes {
        std::env::remove_var("LD_PRELOAD");
    }

    info!("Stage 2 initialization finished");
}

static ALLOW_STAGE_2: AtomicBool = AtomicBool::new(false);

#[used]
#[link_section = ".init_array.00099"]
static INIT_ARRAY: unsafe extern "C" fn(libc::c_int, *mut *mut u8, *mut *mut u8) = {
    unsafe extern "C" fn function(_argc: libc::c_int, _argv: *mut *mut u8, _envp: *mut *mut u8) {
        ALLOW_STAGE_2.store(true, Ordering::SeqCst);
        try_enable(STATE.load(Ordering::Relaxed));
    }
    function
};

#[cold]
#[inline(never)]
fn try_enable(mut state: usize) -> bool {
    if state == STATE_UNINITIALIZED {
        if STATE
            .compare_exchange(
                STATE_UNINITIALIZED,
                STATE_INITIALIZING_STAGE_1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_ok()
        {
            initialize_stage_1();
            STATE.store(STATE_PARTIALLY_INITIALIZED, Ordering::SeqCst);
            state = STATE_PARTIALLY_INITIALIZED;
        } else {
            return false;
        }
    }

    if state == STATE_PARTIALLY_INITIALIZED {
        if !ALLOW_STAGE_2.load(Ordering::SeqCst) {
            return false;
        }

        if STATE
            .compare_exchange(
                STATE_PARTIALLY_INITIALIZED,
                STATE_INITIALIZING_STAGE_2,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_ok()
        {
            initialize_stage_2();
            STATE.store(STATE_DISABLED, Ordering::SeqCst);
        } else {
            return false;
        }
    }

    if DESIRED_STATE.load(Ordering::SeqCst) == DESIRED_STATE_DISABLED {
        return false;
    }

    if STATE
        .compare_exchange(
            STATE_DISABLED,
            STATE_STARTING,
            Ordering::SeqCst,
            Ordering::SeqCst,
        )
        .is_err()
    {
        return false;
    }

    lock_thread_registry(|thread_registry| {
        assert!(!thread_registry.enabled_for_new_threads);
    });

    prepare_to_start_unwinding();
    spawn_processing_thread();

    lock_thread_registry(|thread_registry| {
        thread_registry.enabled_for_new_threads = true;
        for tls in thread_registry.threads_by_system_id().values() {
            if tls.is_internal() {
                continue;
            }

            debug!("Enabling thread {:04x}...", tls.thread_id);
            tls.set_enabled(true);
        }
    });

    resolve_original_syms();

    crate::allocation_tracker::initialize();

    STATE.store(STATE_ENABLED, Ordering::SeqCst);
    info!("Tracing was enabled");

    true
}

pub fn try_disable_if_requested() {
    if DESIRED_STATE.load(Ordering::SeqCst) != DESIRED_STATE_DISABLED {
        return;
    }

    if STATE
        .compare_exchange(
            STATE_ENABLED,
            STATE_STOPPING,
            Ordering::SeqCst,
            Ordering::SeqCst,
        )
        .is_err()
    {
        return;
    }

    send_event(InternalEvent::Exit);
}

const THROTTLE_LIMIT: usize = 8192;

#[cold]
#[inline(never)]
fn throttle(tls: &RawThreadHandle) {
    while ArcLite::get_refcount_relaxed(tls) >= THROTTLE_LIMIT {
        thread::yield_now();
    }
}

pub fn is_actively_running() -> bool {
    DESIRED_STATE.load(Ordering::Relaxed) == DESIRED_STATE_ENABLED
}

/// A handle to per-thread storage; you can't do anything with it.
///
/// Can be sent to other threads.
pub struct WeakThreadHandle(RawThreadHandle);
unsafe impl Send for WeakThreadHandle {}
unsafe impl Sync for WeakThreadHandle {}

impl WeakThreadHandle {
    pub fn system_tid(&self) -> u32 {
        self.0.thread_id
    }

    pub fn unwind_state(&self) -> (&UnsafeCell<bool>, &UnsafeCell<ThreadUnwindState>) {
        (&self.0.is_unwinding, &self.0.unwind_state)
    }
}

/// A handle to per-thread storage.
///
/// Can only be aquired for the current thread, and cannot be sent to other threads.
pub struct StrongThreadHandle(Option<RawThreadHandle>);

impl StrongThreadHandle {
    #[inline(always)]
    pub fn acquire() -> Option<Self> {
        let state = STATE.load(Ordering::Relaxed);
        if state != STATE_ENABLED {
            if !try_enable(state) {
                return None;
            }
        }

        let tls = TLS.try_with(|tls| {
            if !tls.is_enabled() {
                None
            } else {
                if ArcLite::get_refcount_relaxed(tls) >= THROTTLE_LIMIT {
                    throttle(tls);
                }
                tls.set_enabled(false);
                Some(tls.0.clone())
            }
        });

        match tls {
            Ok(Some(tls)) => Some(StrongThreadHandle(Some(tls))),
            Ok(None) | Err(TlsAccessError::Uninitialized) => None,
            Err(TlsAccessError::Destroyed) => {
                acquire_slow().map(|tls| StrongThreadHandle(Some(tls)))
            }
        }
    }

    pub fn decay(mut self) -> WeakThreadHandle {
        let tls = match self.0.take() {
            Some(tls) => tls,
            None => unsafe { std::hint::unreachable_unchecked() },
        };

        tls.set_enabled(true);
        WeakThreadHandle(tls)
    }

    pub fn unwind_state(&mut self) -> (&UnsafeCell<bool>, &UnsafeCell<ThreadUnwindState>) {
        let tls = match self.0.as_ref() {
            Some(tls) => tls,
            None => unsafe { std::hint::unreachable_unchecked() },
        };

        (&tls.is_unwinding, &tls.unwind_state)
    }

    pub fn on_new_allocation(&mut self) -> InternalAllocationId {
        let tls = match self.0.as_ref() {
            Some(tls) => tls,
            None => unsafe { std::hint::unreachable_unchecked() },
        };

        let counter = tls.allocation_counter.get();
        let allocation;
        unsafe {
            allocation = *counter;
            *counter += 1;
        }

        InternalAllocationId::new(tls.internal_thread_id, allocation)
    }

    pub fn system_tid(&self) -> u32 {
        let tls = match self.0.as_ref() {
            Some(tls) => tls,
            None => unsafe { std::hint::unreachable_unchecked() },
        };

        tls.thread_id
    }

    pub fn unique_tid(&self) -> u64 {
        let tls = match self.0.as_ref() {
            Some(tls) => tls,
            None => unsafe { std::hint::unreachable_unchecked() },
        };

        tls.internal_thread_id
    }

    pub fn allocation_tracker(&self) -> &AllocationTracker {
        let tls = match self.0.as_ref() {
            Some(tls) => tls,
            None => unsafe { std::hint::unreachable_unchecked() },
        };

        &tls.allocation_tracker
    }

    pub(crate) fn zombie_events(&self) -> &SpinLock<Vec<InternalEvent>> {
        let tls = match self.0.as_ref() {
            Some(tls) => tls,
            None => unsafe { std::hint::unreachable_unchecked() },
        };

        &tls.zombie_events
    }

    pub fn is_dead(&self) -> bool {
        let tls = match self.0.as_ref() {
            Some(tls) => tls,
            None => unsafe { std::hint::unreachable_unchecked() },
        };

        tls.is_dead.load(Ordering::Relaxed)
    }
}

impl Drop for StrongThreadHandle {
    fn drop(&mut self) {
        if let Some(tls) = self.0.take() {
            tls.set_enabled(true);
        }
    }
}

pub enum ThreadHandleKind {
    Strong(StrongThreadHandle),
    Weak(WeakThreadHandle),
}

impl ThreadHandleKind {
    pub fn system_tid(&self) -> u32 {
        match self {
            ThreadHandleKind::Strong(StrongThreadHandle(ref handle)) => handle.as_ref().unwrap(),
            ThreadHandleKind::Weak(WeakThreadHandle(ref handle)) => handle,
        }
        .thread_id
    }
}

#[cold]
#[inline(never)]
fn acquire_slow() -> Option<RawThreadHandle> {
    let current_thread_id = syscall::gettid();
    lock_thread_registry(|thread_registry| {
        if let Some(thread) = thread_registry
            .threads_by_system_id()
            .get(&current_thread_id)
        {
            debug!("Acquired a dead thread: {:04X}", current_thread_id);
            Some(thread.clone())
        } else {
            warn!(
                "Failed to acquire a handle for thread: {:04X}",
                current_thread_id
            );
            None
        }
    })
}

#[inline(always)]
pub fn acquire_any_thread_handle() -> Option<ThreadHandleKind> {
    let mut state = STATE.load(Ordering::Relaxed);
    if state != STATE_ENABLED {
        if !try_enable(state) {
            state = STATE.load(Ordering::Relaxed);
            match state {
                STATE_UNINITIALIZED | STATE_INITIALIZING_STAGE_1 => return None,
                _ => {
                    if DESIRED_STATE.load(Ordering::SeqCst) != DESIRED_STATE_ENABLED {
                        return None;
                    }
                }
            }
        }
    }

    let tls = TLS.try_with(|tls| {
        if !tls.is_enabled() {
            ThreadHandleKind::Weak(WeakThreadHandle(tls.0.clone()))
        } else {
            if ArcLite::get_refcount_relaxed(tls) >= THROTTLE_LIMIT {
                throttle(tls);
            }

            tls.set_enabled(false);
            ThreadHandleKind::Strong(StrongThreadHandle(Some(tls.0.clone())))
        }
    });

    match tls {
        Ok(tls) => Some(tls),
        Err(TlsAccessError::Uninitialized) => None,
        Err(TlsAccessError::Destroyed) => Some(ThreadHandleKind::Strong(StrongThreadHandle(Some(
            acquire_slow()?,
        )))),
    }
}

pub struct AllocationLock {
    current_thread_id: u32,
    registry_lock: SpinLockGuard<'static, ThreadRegistry>,
}

impl AllocationLock {
    pub fn new() -> Self {
        let mut registry_lock = THREAD_REGISTRY.lock();
        let current_thread_id = syscall::gettid();
        let threads = registry_lock.threads_by_system_id();
        for (&thread_id, tls) in threads.iter_mut() {
            if thread_id == current_thread_id {
                continue;
            }

            if tls.is_internal() {
                continue;
            }
            unsafe {
                ArcLite::add(tls, THROTTLE_LIMIT);
            }
        }

        std::sync::atomic::fence(Ordering::SeqCst);

        for (&thread_id, tls) in threads.iter_mut() {
            if thread_id == current_thread_id {
                continue;
            }

            if tls.is_internal() {
                continue;
            }
            while ArcLite::get_refcount_relaxed(tls) != THROTTLE_LIMIT {
                thread::yield_now();
            }
        }

        std::sync::atomic::fence(Ordering::SeqCst);

        AllocationLock {
            current_thread_id,
            registry_lock,
        }
    }
}

impl Drop for AllocationLock {
    fn drop(&mut self) {
        for (&thread_id, tls) in self.registry_lock.threads_by_system_id().iter_mut() {
            if thread_id == self.current_thread_id {
                continue;
            }

            unsafe {
                ArcLite::sub(tls, THROTTLE_LIMIT);
            }
        }
    }
}

pub struct ThreadData {
    thread_id: u32,
    internal_thread_id: u64,
    is_internal: UnsafeCell<bool>,
    enabled: AtomicBool,
    is_dead: AtomicBool,
    is_unwinding: UnsafeCell<bool>,
    unwind_state: UnsafeCell<ThreadUnwindState>,
    allocation_counter: UnsafeCell<u64>,
    allocation_tracker: AllocationTracker,
    zombie_events: SpinLock<Vec<InternalEvent>>,
}

impl ThreadData {
    #[inline(always)]
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    #[inline(always)]
    pub fn is_internal(&self) -> bool {
        unsafe { *self.is_internal.get() || PROCESSING_THREAD_TID == self.thread_id }
    }

    fn set_enabled(&self, value: bool) {
        self.enabled.store(value, Ordering::Relaxed)
    }
}

struct ThreadSentinel(RawThreadHandle);

impl Deref for ThreadSentinel {
    type Target = RawThreadHandle;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for ThreadSentinel {
    fn drop(&mut self) {
        self.is_dead.store(true, Ordering::SeqCst);

        let is_enabled = self.enabled.load(Ordering::SeqCst);
        self.enabled.store(false, Ordering::SeqCst);

        lock_thread_registry(|thread_registry| {
            if let Some(thread) = thread_registry.threads_by_system_id().get(&self.thread_id) {
                let thread = thread.clone();
                thread_registry
                    .new_dead_thread_queue
                    .push((crate::timestamp::get_timestamp(), thread));
            }
        });

        debug!("Thread dropped: {:04X}", self.thread_id);
        self.enabled.store(is_enabled, Ordering::SeqCst);
    }
}

thread_local_reentrant! {
    static TLS: ThreadSentinel = |callback| {
        let thread_id = syscall::gettid();
        let tls = lock_thread_registry( |registry| {
            let internal_thread_id = registry.thread_counter;
            registry.thread_counter += 1;

            let tls = ThreadData {
                thread_id,
                internal_thread_id,
                is_internal: UnsafeCell::new( false ),
                is_dead: AtomicBool::new( false ),
                enabled: AtomicBool::new( registry.enabled_for_new_threads && thread_id != unsafe { PROCESSING_THREAD_TID } ),
                is_unwinding: UnsafeCell::new( false ),
                unwind_state: UnsafeCell::new( ThreadUnwindState::new() ),
                allocation_counter: UnsafeCell::new( 1 ),
                allocation_tracker: crate::allocation_tracker::on_thread_created( internal_thread_id ),
                zombie_events: SpinLock::new( Vec::new() )
            };

            let tls = ArcLite::new( tls );
            registry.threads_by_system_id().insert( thread_id, tls.clone() );

            tls
        });

        callback( ThreadSentinel( tls ) )
    };
}

#[derive(Default)]
pub struct ThreadGarbageCollector {
    buffer: Vec<(Timestamp, RawThreadHandle)>,
    dead_threads: Vec<(Timestamp, RawThreadHandle)>,
}

impl ThreadGarbageCollector {
    pub(crate) fn run(
        &mut self,
        now: Timestamp,
        events: &mut crate::channel::ChannelBuffer<InternalEvent>,
    ) {
        use crate::utils::Entry;

        lock_thread_registry(|thread_registry| {
            std::mem::swap(&mut thread_registry.new_dead_thread_queue, &mut self.buffer);
        });

        for (timestamp, thread) in self.buffer.drain(..) {
            crate::allocation_tracker::on_thread_destroyed(thread.internal_thread_id);
            events.extend(thread.zombie_events.lock().drain(..));
            self.dead_threads.push((timestamp, thread));
        }

        if self.dead_threads.is_empty() {
            return;
        }

        let count = self
            .dead_threads
            .iter()
            .take_while(|&(time_of_death, _)| time_of_death.as_secs() + 3 < now.as_secs())
            .count();

        if count == 0 {
            return;
        }

        for (_, thread) in self.dead_threads.drain(..count) {
            lock_thread_registry(|thread_registry| {
                let mut entry_by_system_id = None;
                if let Entry::Occupied(entry) =
                    thread_registry.threads_by_system_id.entry(thread.thread_id)
                {
                    if RawThreadHandle::ptr_eq(entry.get(), &thread) {
                        entry_by_system_id = Some(entry.remove_entry());
                    }
                }

                entry_by_system_id
            });
        }
    }
}
