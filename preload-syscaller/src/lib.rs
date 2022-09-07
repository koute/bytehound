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
mod arc_lite;
mod writers;
mod writer_memory;
mod api;
mod event;
mod init;
mod processing_thread;
mod global;
mod ordered_map;
mod nohash;
mod allocation_tracker;

use crate::event::InternalEvent;
use crate::utils::read_file;

#[global_allocator]
static mut ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

pub(crate) const PAGE_SIZE: usize = 4096;

lazy_static! {
    pub(crate) static ref PID: u32 = {
        let pid = crate::syscall::getpid() as u32;
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

pub use crate::api::{
    __register_frame,
    __deregister_frame,

    _exit,
    _Exit,
    fork,

    malloc,
    calloc,
    realloc,
    reallocarray,
    free,
    posix_memalign,
    malloc_usable_size,
    mmap,
    munmap,
    mallopt,
    memalign,
    aligned_alloc,
    valloc,
    pvalloc,

    memory_profiler_set_marker,
    memory_profiler_override_next_timestamp,
    memory_profiler_start,
    memory_profiler_stop,
    memory_profiler_sync
};
