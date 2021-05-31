use std::mem;
use std::ptr;
use std::num::NonZeroUsize;

use libc::{
    c_void,
    c_int,
    size_t,
    off_t
};

use common::event;
use common::Timestamp;

use crate::InternalEvent;
use crate::event::{send_event, send_event_throttled};
use crate::global::{StrongThreadHandle, on_exit};
use crate::opt;
use crate::syscall;
use crate::timestamp::get_timestamp;
use crate::unwind::{self, Backtrace};

#[cfg(not(feature = "jemalloc"))]
extern "C" {
    #[link_name = "__libc_malloc"]
    fn malloc_real( size: size_t ) -> *mut c_void;
    #[link_name = "__libc_calloc"]
    fn calloc_real( count: size_t, element_size: size_t ) -> *mut c_void;
    #[link_name = "__libc_realloc"]
    fn realloc_real( ptr: *mut c_void, size: size_t ) -> *mut c_void;
    #[link_name = "__libc_free"]
    fn free_real( ptr: *mut c_void );
    #[link_name = "__libc_memalign"]
    fn memalign_real( alignment: size_t, size: size_t ) -> *mut c_void;
    #[link_name = "__libc_mallopt"]
    fn mallopt_real( params: c_int, value: c_int ) -> c_int;
}

#[cfg(feature = "jemalloc")]
extern "C" {
    #[link_name = "_rjem_malloc"]
    fn malloc_real( size: size_t ) -> *mut c_void;
    #[link_name = "_rjem_calloc"]
    fn calloc_real( count: size_t, element_size: size_t ) -> *mut c_void;
    #[link_name = "_rjem_realloc"]
    fn realloc_real( ptr: *mut c_void, size: size_t ) -> *mut c_void;
    #[link_name = "_rjem_free"]
    fn free_real( ptr: *mut c_void );
    #[link_name = "_rjem_memalign"]
    fn memalign_real( alignment: size_t, size: size_t ) -> *mut c_void;
}

#[cfg(feature = "jemalloc")]
unsafe fn mallopt_real( _: c_int, _: c_int ) -> c_int {
    1
}

extern "C" {
    #[link_name = "__libc_fork"]
    fn fork_real() -> libc::pid_t;
}

const USING_JEMALLOC: bool = cfg!( feature = "jemalloc" );

fn get_timestamp_if_enabled() -> Timestamp {
    if opt::get().precise_timestamps {
        get_timestamp()
    } else {
        Timestamp::min()
    }
}

#[no_mangle]
pub unsafe extern "C" fn memory_profiler_raw_mmap( addr: *mut c_void, length: size_t, prot: c_int, flags: c_int, fildes: c_int, off: off_t ) -> *mut c_void {
    syscall::mmap( addr, length, prot, flags, fildes, off )
}

#[no_mangle]
pub unsafe extern "C" fn memory_profiler_raw_munmap( addr: *mut c_void, length: size_t ) -> c_int {
    syscall::munmap( addr, length )
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn _exit( status: c_int ) {
    on_exit();
    syscall::exit( status as u32 );
}

#[allow(non_snake_case)]
#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn _Exit( status: c_int ) {
    _exit( status );
}

fn get_glibc_metadata( ptr: *mut c_void, size: usize ) -> (u32, u32, u64) {
    if USING_JEMALLOC {
        return (0, 0, 0);
    }

    // `libc` on mips64 doesn't export this
    extern "C" {
        fn malloc_usable_size( ptr: *mut libc::c_void) -> libc::size_t;
    }

    let raw_chunk_size = unsafe { *(ptr as *mut usize).offset( -1 ) };
    let flags = raw_chunk_size & 0b111;
    let chunk_size = raw_chunk_size & !0b111;

    let is_prev_in_use = flags & 1 != 0;
    let preceding_free_space = if !is_prev_in_use {
        unsafe { *(ptr as *mut usize).offset( -2 ) }
    } else {
        0
    };

    let is_mmapped = flags & 2 != 0;
    let extra_usable_space = chunk_size - size - mem::size_of::< usize >() * if is_mmapped { 2 } else { 1 };

    debug_assert_eq!(
        size + extra_usable_space,
        unsafe { malloc_usable_size( ptr ) },
        "chunk_size: {}, size: {}, malloc_usable_size: {}, extra_usable_space: {}",
        chunk_size,
        size,
        unsafe { malloc_usable_size( ptr ) },
        extra_usable_space,
    );

    (flags as u32, extra_usable_space as u32, preceding_free_space as u64)
}

#[inline(always)]
unsafe fn allocate( size: usize, is_calloc: bool ) -> *mut c_void {
    let thread = StrongThreadHandle::acquire();
    let ptr =
        if is_calloc || opt::get().zero_memory {
            calloc_real( size as size_t, 1 )
        } else {
            malloc_real( size as size_t )
        };

    let address = match NonZeroUsize::new( ptr as usize ) {
        Some( address ) => address,
        None => return ptr
    };

    let mut thread = if let Some( thread ) = thread { thread } else { return ptr };
    let mut backtrace = Backtrace::new();
    unwind::grab( &mut thread, &mut backtrace );

    let (mut flags, extra_usable_space, preceding_free_space) = get_glibc_metadata( ptr, size );
    if is_calloc {
        flags |= event::ALLOC_FLAG_CALLOC;
    }

    send_event_throttled( move || {
        InternalEvent::Alloc {
            ptr: address,
            size: size as usize,
            backtrace,
            flags,
            extra_usable_space,
            preceding_free_space,
            timestamp: get_timestamp_if_enabled(),
            thread: thread.decay()
        }
    });

    ptr
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn malloc( size: size_t ) -> *mut c_void {
    allocate( size, false )
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn calloc( count: size_t, element_size: size_t ) -> *mut c_void {
    let size = match (count as usize).checked_mul( element_size as usize ) {
        None => return ptr::null_mut(),
        Some( size ) => size as size_t
    };

    allocate( size, true )
}

#[inline(always)]
unsafe fn realloc_impl( old_ptr: *mut c_void, size: size_t ) -> *mut c_void {
    let old_address = match NonZeroUsize::new( old_ptr as usize ) {
        Some( old_address ) => old_address,
        None => return malloc( size )
    };

    if size == 0 {
        free( old_ptr );
        return ptr::null_mut();
    }

    let thread = StrongThreadHandle::acquire();
    let new_ptr = realloc_real( old_ptr, size );

    let mut thread = if let Some( thread ) = thread { thread } else { return new_ptr };
    let mut backtrace = Backtrace::new();
    unwind::grab( &mut thread, &mut backtrace );

    let timestamp = get_timestamp_if_enabled();

    if let Some( new_address ) = NonZeroUsize::new( new_ptr as usize ) {
        let (flags, extra_usable_space, preceding_free_space) = get_glibc_metadata( new_ptr, size );
        send_event_throttled( move || {
            InternalEvent::Realloc {
                old_ptr: old_address,
                new_ptr: new_address,
                size: size as usize,
                backtrace,
                flags,
                extra_usable_space,
                preceding_free_space,
                timestamp,
                thread: thread.decay()
            }
        });
    } else {
        send_event_throttled( || {
            InternalEvent::Free {
                ptr: old_address,
                backtrace,
                timestamp,
                thread: thread.decay()
            }
        });
    }

    new_ptr
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn realloc( old_ptr: *mut c_void, size: size_t ) -> *mut c_void {
    realloc_impl( old_ptr, size )
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn reallocarray( old_ptr: &mut c_void, count: size_t, element_size: size_t ) -> *mut c_void {
    let size = match (count as usize).checked_mul( element_size as usize ) {
        None => {
            *libc::__errno_location() = libc::ENOMEM;
            return ptr::null_mut()
        },
        Some( size ) => size as size_t
    };

    realloc_impl( old_ptr, size )
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn free( ptr: *mut c_void ) {
    let address = match NonZeroUsize::new( ptr as usize ) {
        Some( address ) => address,
        None => return
    };

    let thread = StrongThreadHandle::acquire();
    free_real( ptr );

    let mut thread = if let Some( thread ) = thread { thread } else { return };
    let mut backtrace = Backtrace::new();
    if opt::get().grab_backtraces_on_free {
        unwind::grab( &mut thread, &mut backtrace );
    }

    send_event_throttled( || {
        InternalEvent::Free {
            ptr: address,
            backtrace,
            timestamp: get_timestamp_if_enabled(),
            thread: thread.decay()
        }
    });
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn posix_memalign( memptr: *mut *mut c_void, alignment: size_t, size: size_t ) -> c_int {
    if memptr.is_null() {
        return libc::EINVAL;
    }

    let ptr_size = mem::size_of::< *const c_void >();
    if alignment % ptr_size != 0 || !(alignment / ptr_size).is_power_of_two() || alignment == 0 {
        return libc::EINVAL;
    }

    let thread = StrongThreadHandle::acquire();

    let pointer = memalign_real( alignment, size );
    *memptr = pointer;
    let address = match NonZeroUsize::new( pointer as usize ) {
        Some( address ) => address,
        None => return libc::ENOMEM
    };

    let mut thread = if let Some( thread ) = thread { thread } else { return 0 };
    let mut backtrace = Backtrace::new();
    unwind::grab( &mut thread, &mut backtrace );

    let (flags, extra_usable_space, preceding_free_space) = get_glibc_metadata( pointer, size );
    send_event_throttled( || {
        InternalEvent::Alloc {
            ptr: address,
            size: size as usize,
            backtrace,
            flags,
            extra_usable_space,
            preceding_free_space,
            timestamp: get_timestamp_if_enabled(),
            thread: thread.decay()
        }
    });

    0
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn mmap( addr: *mut c_void, length: size_t, prot: c_int, flags: c_int, fildes: c_int, off: off_t ) -> *mut c_void {
    let thread = StrongThreadHandle::acquire();

    let ptr = syscall::mmap( addr, length, prot, flags, fildes, off );
    if ptr == libc::MAP_FAILED || !opt::get().gather_mmap_calls {
        return ptr;
    }

    let mut thread = if let Some( thread ) = thread { thread } else { return ptr };
    let mut backtrace = Backtrace::new();
    unwind::grab( &mut thread, &mut backtrace );

    send_event_throttled( || InternalEvent::Mmap {
        pointer: ptr as usize,
        length: length as usize,
        requested_address: addr as usize,
        mmap_protection: prot as u32,
        mmap_flags: flags as u32,
        file_descriptor: fildes as u32,
        offset: off as u64,
        backtrace,
        timestamp: get_timestamp_if_enabled(),
        thread: thread.decay()
    });

    ptr
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn munmap( ptr: *mut c_void, length: size_t ) -> c_int {
    let thread = StrongThreadHandle::acquire();
    let result = syscall::munmap( ptr, length );

    if !opt::get().gather_mmap_calls {
        return result;
    }

    let mut thread = if let Some( thread ) = thread { thread } else { return result };
    let mut backtrace = Backtrace::new();
    unwind::grab( &mut thread, &mut backtrace );

    if !ptr.is_null() {
        send_event_throttled( || InternalEvent::Munmap {
            ptr: ptr as usize,
            len: length as usize,
            backtrace,
            timestamp: get_timestamp_if_enabled(),
            thread: thread.decay()
        });
    }

    result
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn mallopt( param: c_int, value: c_int ) -> c_int {
    let thread = StrongThreadHandle::acquire();
    let result = mallopt_real( param, value );

    let mut thread = if let Some( thread ) = thread { thread } else { return result };
    let mut backtrace = Backtrace::new();
    unwind::grab( &mut thread, &mut backtrace );

    send_event_throttled( || InternalEvent::Mallopt {
        param: param as i32,
        value: value as i32,
        result: result as i32,
        backtrace,
        timestamp: get_timestamp_if_enabled(),
        thread: thread.decay()
    });

    result
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn fork() -> libc::pid_t {
    let pid = fork_real();
    if pid == 0 {
        crate::global::on_fork();
    } else {
        info!( "Fork called; child PID: {}", pid );
    }

    pid
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn memalign( _alignment: size_t, _size: size_t ) -> *mut c_void {
    unimplemented!( "'memalign' is unimplemented!" );
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn aligned_alloc( _alignment: size_t, _size: size_t ) -> *mut c_void {
    unimplemented!( "'aligned_alloc' is unimplemented!" );
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn valloc( _size: size_t ) -> *mut c_void {
    unimplemented!( "'valloc' is unimplemented!" );
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn pvalloc( _size: size_t ) -> *mut c_void {
    unimplemented!( "'pvalloc' is unimplemented!" );
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn memory_profiler_set_marker( value: u32 ) {
    let thread = StrongThreadHandle::acquire();
    send_event( InternalEvent::SetMarker {
        value
    });

    mem::drop( thread );
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn memory_profiler_override_next_timestamp( timestamp: u64 ) {
    let thread = StrongThreadHandle::acquire();
    send_event_throttled( || InternalEvent::OverrideNextTimestamp {
        timestamp: Timestamp::from_usecs( timestamp )
    });

    mem::drop( thread );
}

fn sync() {
    let thread = StrongThreadHandle::acquire();
    crate::event::flush();
    crate::global::sync();
    mem::drop( thread );
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn memory_profiler_start() {
    debug!( "Start called..." );
    if crate::global::enable() {
        sync();
    }
    debug!( "Start finished" );
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn memory_profiler_stop() {
    debug!( "Stop called..." );
    if crate::global::disable() {
        sync();
    }
    debug!( "Stop finished" );
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn memory_profiler_sync() {
    debug!( "Sync called..." );
    sync();
    debug!( "Sync finished" );
}
