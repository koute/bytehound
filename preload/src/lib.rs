#[macro_use]
extern crate log;

#[macro_use]
extern crate lazy_static;

#[cfg(feature = "sc")]
#[macro_use]
extern crate sc;

#[macro_use]
extern crate thread_local_reentrant;

use std::fs::read_link;

use std::os::unix::ffi::OsStrExt;

#[macro_use]
mod macros;
mod allocation_tracker;
mod api;
mod arc_lite;
mod arch;
mod channel;
mod elf;
mod event;
mod global;
mod init;
mod logger;
mod nohash;
mod opt;
mod ordered_map;
mod processing_thread;
mod raw_file;
mod smaps;
mod spin_lock;
mod syscall;
mod timestamp;
mod unwind;
mod utils;
mod writer_memory;
mod writers;

use crate::event::InternalEvent;
use crate::utils::read_file;

#[global_allocator]
static mut GLOBAL_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub(crate) const PAGE_SIZE: usize = 4096;

lazy_static! {
    pub(crate) static ref PID: u32 = {
        let pid = crate::syscall::getpid() as u32;
        pid
    };
    pub(crate) static ref CMDLINE: Vec<u8> = { read_file("/proc/self/cmdline").unwrap() };
    pub(crate) static ref EXECUTABLE: Vec<u8> = {
        let executable: Vec<u8> = read_link("/proc/self/exe")
            .unwrap()
            .as_os_str()
            .as_bytes()
            .into();
        executable
    };
}

pub use crate::api::{
    _Exit, __deregister_frame, __register_frame, _exit, aligned_alloc, bytehound_jemalloc_raw_mmap,
    bytehound_jemalloc_raw_munmap, bytehound_mimalloc_raw_mmap, bytehound_mimalloc_raw_mprotect,
    bytehound_mimalloc_raw_munmap, calloc, fork, free, malloc, malloc_usable_size, mallopt,
    memalign, memory_profiler_override_next_timestamp, memory_profiler_set_marker,
    memory_profiler_start, memory_profiler_stop, memory_profiler_sync, mmap, mmap64, munmap,
    posix_memalign, pvalloc, realloc, reallocarray, valloc,
};
