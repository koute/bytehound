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
use crate::event::{InternalAllocationId, send_event, send_event_throttled};
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
    #[link_name = "_rjem_malloc_usable_size"]
    fn malloc_usable_size_real( ptr: *mut c_void ) -> size_t;
}

#[cfg(feature = "jemalloc")]
unsafe fn mallopt_real( _: c_int, _: c_int ) -> c_int {
    1
}

extern "C" {
    #[link_name = "__libc_fork"]
    fn fork_real() -> libc::pid_t;
}

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

#[derive(Debug)]
struct Metadata {
    flags: u32,
    preceding_free_space: usize,
    usable_size: usize
}

fn get_allocation_metadata( ptr: *mut c_void ) -> Metadata {
    #[cfg(feature = "jemalloc")]
    {
        return Metadata {
            flags: 0,
            preceding_free_space: 0,
            usable_size: unsafe { malloc_usable_size_real( ptr ) }
        }
    }

    #[cfg(not(feature = "jemalloc"))]
    {
        // `libc` on mips64 doesn't export this
        extern "C" {
            fn malloc_usable_size( ptr: *mut libc::c_void ) -> libc::size_t;
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
        let usable_size = chunk_size - mem::size_of::< usize >() * if is_mmapped { 2 } else { 1 };
        debug_assert_eq!( usable_size, unsafe { malloc_usable_size( ptr ) } );

        Metadata {
            flags: flags as u32,
            preceding_free_space,
            usable_size
        }
    }
}

unsafe fn tracking_pointer( pointer: *mut c_void, usable_size: usize ) -> *mut InternalAllocationId {
    let tracking_offset = usable_size - mem::size_of::< InternalAllocationId >();
    (pointer as *mut u8).add( tracking_offset ) as *mut InternalAllocationId
}

enum AllocationKind {
    Malloc,
    Calloc,
    Aligned( size_t )
}

#[inline(always)]
unsafe fn allocate( requested_size: usize, kind: AllocationKind ) -> *mut c_void {
    let effective_size = match requested_size.checked_add( mem::size_of::< InternalAllocationId >() ) {
        Some( size ) => size,
        None => return ptr::null_mut()
    };

    let thread = StrongThreadHandle::acquire();
    let pointer =
        match kind {
            AllocationKind::Malloc => {
                if opt::get().zero_memory {
                    calloc_real( effective_size as size_t, 1 )
                } else {
                    malloc_real( effective_size as size_t )
                }
            },
            AllocationKind::Calloc => calloc_real( effective_size as size_t, 1 ),
            AllocationKind::Aligned( alignment ) => {
                memalign_real( alignment, effective_size as size_t )
            }
        };

    let address = match NonZeroUsize::new( pointer as usize ) {
        Some( address ) => address,
        None => return pointer
    };

    let mut metadata = get_allocation_metadata( pointer );
    if !matches!( kind, AllocationKind::Calloc ) {
        std::ptr::write_bytes( pointer as *mut u8, 0xee, metadata.usable_size );
    }
    let tracking_pointer = tracking_pointer( pointer, metadata.usable_size );

    let mut thread = if let Some( thread ) = thread {
        thread
    } else {
        std::ptr::write_unaligned( tracking_pointer, InternalAllocationId::UNTRACKED );
        return pointer;
    };

    let id = thread.on_new_allocation();
    std::ptr::write_unaligned( tracking_pointer, id );

    let mut backtrace = Backtrace::new();
    unwind::grab( &mut thread, &mut backtrace );

    if matches!( kind, AllocationKind::Calloc ) {
        metadata.flags |= event::ALLOC_FLAG_CALLOC;
    }

    send_event_throttled( move || {
        InternalEvent::Alloc {
            id,
            address,
            size: requested_size as usize,
            usable_size: metadata.usable_size,
            preceding_free_space: metadata.preceding_free_space,
            flags: metadata.flags,
            backtrace,
            timestamp: get_timestamp_if_enabled(),
            thread: thread.decay()
        }
    });

    pointer
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn malloc( size: size_t ) -> *mut c_void {
    allocate( size, AllocationKind::Malloc )
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn calloc( count: size_t, element_size: size_t ) -> *mut c_void {
    let size = match count.checked_mul( element_size ) {
        None => return ptr::null_mut(),
        Some( size ) => size
    };

    allocate( size, AllocationKind::Calloc )
}

#[inline(always)]
unsafe fn realloc_impl( old_pointer: *mut c_void, requested_size: size_t ) -> *mut c_void {
    let old_address = match NonZeroUsize::new( old_pointer as usize ) {
        Some( old_address ) => old_address,
        None => return malloc( requested_size )
    };

    if requested_size == 0 {
        free( old_pointer );
        return ptr::null_mut();
    }

    let effective_size = match requested_size.checked_add( mem::size_of::< InternalAllocationId >() ) {
        Some( size ) => size,
        None => return ptr::null_mut()
    };

    let old_metadata = get_allocation_metadata( old_pointer );
    let old_tracking_pointer = tracking_pointer( old_pointer, old_metadata.usable_size );
    let id = std::ptr::read_unaligned( old_tracking_pointer );

    let thread = StrongThreadHandle::acquire();
    let new_pointer = realloc_real( old_pointer, effective_size );

    let mut thread = if let Some( thread ) = thread {
        thread
    } else {
        if new_pointer.is_null() {
            return ptr::null_mut();
        } else {
            let new_metadata = get_allocation_metadata( new_pointer );
            let new_tracking_pointer = tracking_pointer( new_pointer, new_metadata.usable_size );
            std::ptr::write_unaligned( new_tracking_pointer, InternalAllocationId::UNTRACKED );

            return new_pointer;
        }
    };

    let mut backtrace = Backtrace::new();
    unwind::grab( &mut thread, &mut backtrace );

    let timestamp = get_timestamp_if_enabled();
    if let Some( new_address ) = NonZeroUsize::new( new_pointer as usize ) {
        let new_metadata = get_allocation_metadata( new_pointer );
        let new_tracking_pointer = tracking_pointer( new_pointer, new_metadata.usable_size );
        std::ptr::write_unaligned( new_tracking_pointer, id );

        send_event_throttled( move || {
            InternalEvent::Realloc {
                id,
                old_address,
                new_address,
                new_size: requested_size as usize,
                new_usable_size: new_metadata.usable_size,
                new_preceding_free_space: new_metadata.preceding_free_space,
                new_flags: new_metadata.flags,
                backtrace,
                timestamp,
                thread: thread.decay()
            }
        });

        new_pointer
    } else {
        send_event_throttled( || {
            InternalEvent::Free {
                id,
                address: old_address,
                backtrace,
                timestamp,
                thread: thread.decay()
            }
        });

        ptr::null_mut()
    }
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
pub unsafe extern "C" fn free( pointer: *mut c_void ) {
    let address = match NonZeroUsize::new( pointer as usize ) {
        Some( address ) => address,
        None => return
    };

    let metadata = get_allocation_metadata( pointer );
    let tracking_pointer = tracking_pointer( pointer, metadata.usable_size );
    let id = std::ptr::read_unaligned( tracking_pointer );

    let thread = StrongThreadHandle::acquire();
    free_real( pointer );

    let mut thread = if let Some( thread ) = thread { thread } else { return };
    let mut backtrace = Backtrace::new();
    if opt::get().grab_backtraces_on_free {
        unwind::grab( &mut thread, &mut backtrace );
    }

    send_event_throttled( || {
        InternalEvent::Free {
            id,
            address,
            backtrace,
            timestamp: get_timestamp_if_enabled(),
            thread: thread.decay()
        }
    });
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn posix_memalign( memptr: *mut *mut c_void, alignment: size_t, requested_size: size_t ) -> c_int {
    if memptr.is_null() {
        return libc::EINVAL;
    }

    let ptr_size = mem::size_of::< *const c_void >();
    if alignment % ptr_size != 0 || !(alignment / ptr_size).is_power_of_two() || alignment == 0 {
        return libc::EINVAL;
    }

    let pointer = allocate( requested_size, AllocationKind::Aligned( alignment ) );
    *memptr = pointer;

    if pointer.is_null() {
        libc::ENOMEM
    } else {
        0
    }
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
