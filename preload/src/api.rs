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
    #[link_name = "_rjem_mp_malloc"]
    fn malloc_real( size: size_t ) -> *mut c_void;
    #[link_name = "_rjem_mp_calloc"]
    fn calloc_real( count: size_t, element_size: size_t ) -> *mut c_void;
    #[link_name = "_rjem_mp_realloc"]
    fn realloc_real( ptr: *mut c_void, size: size_t ) -> *mut c_void;
    #[link_name = "_rjem_mp_free"]
    fn free_real( ptr: *mut c_void );
    #[link_name = "_rjem_mp_memalign"]
    fn memalign_real( alignment: size_t, size: size_t ) -> *mut c_void;
    #[link_name = "_rjem_mp_malloc_usable_size"]
    fn malloc_usable_size_real( ptr: *mut c_void ) -> size_t;
}

extern "C" {
    #[link_name = "_rjem_mp_mallocx"]
    fn jem_mallocx_real( size: size_t, flags: c_int ) -> *mut c_void;
    #[link_name = "_rjem_mp_calloc"]
    fn jem_calloc_real( count: size_t, element_size: size_t ) -> *mut c_void;
    #[link_name = "_rjem_mp_sdallocx"]
    fn jem_sdallocx_real( pointer: *mut c_void, _size: size_t, _flags: c_int );
    #[link_name = "_rjem_mp_rallocx"]
    fn jem_rallocx_real( old_pointer: *mut c_void, size: size_t, _flags: c_int ) -> *mut c_void;
    #[link_name = "_rjem_mp_xallocx"]
    fn jem_xallocx_real( pointer: *mut c_void, size: size_t, extra: size_t, _flags: c_int ) -> size_t;
    #[link_name = "_rjem_mp_nallocx"]
    fn jem_nallocx_real( size: size_t, _flags: c_int ) -> size_t;
    #[link_name = "_rjem_mp_malloc_usable_size"]
    fn jem_malloc_usable_size_real( pointer: *const c_void ) -> size_t;
    #[link_name = "_rjem_mp_mallctlnametomib"]
    fn jem_mallctlnametomib_real( name: *const libc::c_char, mibp: *mut size_t, miblenp: *mut size_t ) -> c_int;
    #[link_name = "_rjem_mp_mallctlbymib"]
    fn jem_mallctlbymib_real( mib: *const size_t, miblen: size_t, oldp: *mut c_void, oldpenp: *mut size_t, newp: *mut c_void, newlen: size_t ) -> c_int;
    #[link_name = "_rjem_mp_malloc_stats_print"]
    fn jem_malloc_stats_print_real( write_cb: Option< unsafe extern "C" fn( *mut c_void, *const libc::c_char ) >, cbopaque: *mut c_void, opts: *const libc::c_char );
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
            usable_size: unsafe { jem_malloc_usable_size_real( ptr ) }
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

    let mut thread = StrongThreadHandle::acquire();
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

    if !crate::global::is_actively_running() {
        thread = None;
    }

    let address = match NonZeroUsize::new( pointer as usize ) {
        Some( address ) => address,
        None => return pointer
    };

    let mut metadata = get_allocation_metadata( pointer );
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
    debug_assert!( id.is_valid() );

    let mut thread = StrongThreadHandle::acquire();
    let new_pointer = realloc_real( old_pointer, effective_size );
    if id.is_untracked() && !crate::global::is_actively_running() {
        thread = None;
    }

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
    debug_assert!( id.is_valid() );

    let mut thread = StrongThreadHandle::acquire();
    free_real( pointer );

    if id.is_untracked() && !crate::global::is_actively_running() {
        thread = None;
    }

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
pub unsafe extern "C" fn _rjem_malloc( requested_size: size_t ) -> *mut c_void {
    _rjem_mallocx( requested_size, 0 )
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn _rjem_mallocx( requested_size: size_t, flags: c_int ) -> *mut c_void {
    let effective_size = match requested_size.checked_add( mem::size_of::< InternalAllocationId >() ) {
        Some( size ) => size,
        None => return ptr::null_mut()
    };

    let mut thread = StrongThreadHandle::acquire();
    let pointer = jem_mallocx_real( effective_size, flags );

    if !crate::global::is_actively_running() {
        thread = None;
    }

    let address = match NonZeroUsize::new( pointer as usize ) {
        Some( address ) => address,
        None => return pointer
    };

    let usable_size = jem_malloc_usable_size_real( pointer );
    debug_assert!( usable_size >= effective_size );
    let tracking_pointer = tracking_pointer( pointer, usable_size );

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
    send_event_throttled( move || {
        InternalEvent::Alloc {
            id,
            address,
            size: requested_size as usize,
            usable_size,
            preceding_free_space: 0,
            flags: event::ALLOC_FLAG_JEMALLOC,
            backtrace,
            timestamp: get_timestamp_if_enabled(),
            thread: thread.decay()
        }
    });

    pointer
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn _rjem_calloc( count: size_t, element_size: size_t ) -> *mut c_void {
    let requested_size = match count.checked_mul( element_size ) {
        None => return ptr::null_mut(),
        Some( size ) => size
    };

    let effective_size = match requested_size.checked_add( mem::size_of::< InternalAllocationId >() ) {
        Some( size ) => size,
        None => return ptr::null_mut()
    };

    let mut thread = StrongThreadHandle::acquire();
    let pointer = jem_calloc_real( 1, effective_size );

    if !crate::global::is_actively_running() {
        thread = None;
    }

    let address = match NonZeroUsize::new( pointer as usize ) {
        Some( address ) => address,
        None => return pointer
    };

    let usable_size = jem_malloc_usable_size_real( pointer );
    debug_assert!( usable_size >= effective_size );
    let tracking_pointer = tracking_pointer( pointer, usable_size );

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
    send_event_throttled( move || {
        InternalEvent::Alloc {
            id,
            address,
            size: requested_size as usize,
            usable_size,
            preceding_free_space: 0,
            flags: event::ALLOC_FLAG_JEMALLOC | event::ALLOC_FLAG_CALLOC,
            backtrace,
            timestamp: get_timestamp_if_enabled(),
            thread: thread.decay()
        }
    });

    pointer
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn _rjem_sdallocx( pointer: *mut c_void, requested_size: size_t, flags: c_int ) {
    let address = match NonZeroUsize::new( pointer as usize ) {
        Some( address ) => address,
        None => return
    };

    let effective_size = match requested_size.checked_add( mem::size_of::< InternalAllocationId >() ) {
        Some( size ) => size,
        None => return
    };

    let usable_size = jem_malloc_usable_size_real( pointer );
    debug_assert!( usable_size >= effective_size );
    let tracking_pointer = tracking_pointer( pointer, usable_size );
    let id = std::ptr::read_unaligned( tracking_pointer );
    debug_assert!( id.is_valid() );

    let mut thread = StrongThreadHandle::acquire();
    jem_sdallocx_real( pointer, effective_size, flags );

    if id.is_untracked() && !crate::global::is_actively_running() {
        thread = None;
    }

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
pub unsafe extern "C" fn _rjem_realloc( old_pointer: *mut c_void, requested_size: size_t ) -> *mut c_void {
    _rjem_rallocx( old_pointer, requested_size, 0 )
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn _rjem_rallocx( old_pointer: *mut c_void, requested_size: size_t, flags: c_int ) -> *mut c_void {
    let old_address = match NonZeroUsize::new( old_pointer as usize ) {
        Some( old_address ) => old_address,
        None => return _rjem_mallocx( requested_size, flags )
    };

    let effective_size = match requested_size.checked_add( mem::size_of::< InternalAllocationId >() ) {
        Some( size ) => size,
        None => return ptr::null_mut()
    };

    let old_usable_size = jem_malloc_usable_size_real( old_pointer );
    let old_tracking_pointer = tracking_pointer( old_pointer, old_usable_size );
    let id = std::ptr::read_unaligned( old_tracking_pointer );
    debug_assert!( id.is_valid() );

    let mut thread = StrongThreadHandle::acquire();
    let new_pointer = jem_rallocx_real( old_pointer, effective_size, flags );
    if id.is_untracked() && !crate::global::is_actively_running() {
        thread = None;
    }

    let mut thread = if let Some( thread ) = thread {
        thread
    } else {
        if new_pointer.is_null() {
            return ptr::null_mut();
        } else {
            let new_usable_size = jem_malloc_usable_size_real( new_pointer );
            debug_assert!( new_usable_size >= effective_size );
            let new_tracking_pointer = tracking_pointer( new_pointer, new_usable_size );
            std::ptr::write_unaligned( new_tracking_pointer, InternalAllocationId::UNTRACKED );

            return new_pointer;
        }
    };

    let mut backtrace = Backtrace::new();
    unwind::grab( &mut thread, &mut backtrace );

    let timestamp = get_timestamp_if_enabled();
    if let Some( new_address ) = NonZeroUsize::new( new_pointer as usize ) {
        let new_usable_size = jem_malloc_usable_size_real( new_pointer );
        debug_assert!( new_usable_size >= effective_size );
        let new_tracking_pointer = tracking_pointer( new_pointer, new_usable_size );
        std::ptr::write_unaligned( new_tracking_pointer, id );

        send_event_throttled( move || {
            InternalEvent::Realloc {
                id,
                old_address,
                new_address,
                new_size: requested_size as usize,
                new_usable_size,
                new_preceding_free_space: 0,
                new_flags: event::ALLOC_FLAG_JEMALLOC,
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
pub unsafe extern "C" fn _rjem_xallocx( pointer: *mut c_void, requested_size: size_t, extra: size_t, flags: c_int ) -> size_t {
    let address = match NonZeroUsize::new( pointer as usize ) {
        Some( address ) => address,
        None => return 0
    };

    let effective_size = match requested_size.checked_add( mem::size_of::< InternalAllocationId >() ) {
        Some( size ) => size,
        None => return _rjem_malloc_usable_size( pointer )
    };

    let old_usable_size = jem_malloc_usable_size_real( pointer );
    let old_tracking_pointer = tracking_pointer( pointer, old_usable_size );
    let id = std::ptr::read_unaligned( old_tracking_pointer );
    debug_assert!( id.is_valid() );

    let mut thread = StrongThreadHandle::acquire();
    let new_effective_size = jem_xallocx_real( pointer, effective_size, extra, flags );
    let new_requested_size = new_effective_size.checked_sub( mem::size_of::< InternalAllocationId >() ).expect( "_rjem_xallocx: underflow" );
    if id.is_untracked() && !crate::global::is_actively_running() {
        thread = None;
    }

    let mut thread = if let Some( thread ) = thread {
        thread
    } else {
        let new_usable_size = jem_malloc_usable_size_real( pointer );
        debug_assert!( new_usable_size >= effective_size );
        let new_tracking_pointer = tracking_pointer( pointer, new_usable_size );
        std::ptr::write_unaligned( new_tracking_pointer, InternalAllocationId::UNTRACKED );

        return new_requested_size;
    };

    let mut backtrace = Backtrace::new();
    unwind::grab( &mut thread, &mut backtrace );

    let timestamp = get_timestamp_if_enabled();

    let new_usable_size = jem_malloc_usable_size_real( pointer );
    debug_assert!( new_usable_size >= effective_size );
    let new_tracking_pointer = tracking_pointer( pointer, new_usable_size );
    std::ptr::write_unaligned( new_tracking_pointer, id );

    send_event_throttled( move || {
        InternalEvent::Realloc {
            id,
            old_address: address,
            new_address: address,
            new_size: new_requested_size as usize,
            new_usable_size,
            new_preceding_free_space: 0,
            new_flags: event::ALLOC_FLAG_JEMALLOC,
            backtrace,
            timestamp,
            thread: thread.decay()
        }
    });

    new_requested_size
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn _rjem_nallocx( requested_size: size_t, flags: c_int ) -> size_t {
    let effective_size = match requested_size.checked_add( mem::size_of::< InternalAllocationId >() ) {
        Some( size ) => size,
        None => return 0
    };

    jem_nallocx_real( effective_size, flags ).checked_sub( mem::size_of::< InternalAllocationId >() ).expect( "_rjem_nallocx: underflow" )
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn _rjem_malloc_usable_size( pointer: *const c_void ) -> size_t {
    let usable_size = jem_malloc_usable_size_real( pointer );
    match usable_size.checked_sub( mem::size_of::< InternalAllocationId >() ) {
        Some( size ) => size,
        None => panic!( "_rjem_malloc_usable_size: underflow (pointer=0x{:016X}, usable_size={})", pointer as usize , usable_size )
    }
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn _rjem_mallctl( name: *const libc::c_char, _oldp: *mut c_void, _oldlenp: *mut size_t, _newp: *mut c_void, _newlen: size_t ) -> c_int {
    warn!( "unimplemented: rjem_mallctl called: name={:?}", std::ffi::CStr::from_ptr( name ) );

    0
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn _rjem_posix_memalign( _pointer: *mut *mut c_void, _alignment: size_t, _size: size_t ) -> c_int {
    todo!( "_rjem_posix_memalign" );
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn _rjem_aligned_alloc( _alignment: size_t, _size: size_t ) -> *mut c_void {
    todo!( "_rjem_aligned_alloc" );
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn _rjem_free( _pointer: *mut c_void ) {
    todo!( "_rjem_free" );
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn _rjem_sallocx( _pointer: *const c_void, _flags: c_int ) -> size_t {
    todo!( "_rjem_dallocx" );
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn _rjem_dallocx( _pointer: *mut c_void, _flags: c_int ) {
    todo!( "_rjem_dallocx" );
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn _rjem_mallctlnametomib( name: *const libc::c_char, mibp: *mut size_t, miblenp: *mut size_t ) -> c_int {
    jem_mallctlnametomib_real( name, mibp, miblenp )
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn _rjem_mallctlbymib(
    mib: *const size_t,
    miblen: size_t,
    oldp: *mut c_void,
    oldpenp: *mut size_t,
    newp: *mut c_void,
    newlen: size_t,
) -> c_int {
    jem_mallctlbymib_real( mib, miblen, oldp, oldpenp, newp, newlen )
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn _rjem_malloc_stats_print(
    write_cb: Option< unsafe extern "C" fn( *mut c_void, *const libc::c_char ) >,
    cbopaque: *mut c_void,
    opts: *const libc::c_char,
) {
    jem_malloc_stats_print_real( write_cb, cbopaque, opts )
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
    let mut thread = StrongThreadHandle::acquire();
    if !opt::get().gather_mmap_calls {
        thread = None;
    }

    let mut thread = if let Some( thread ) = thread {
        thread
    } else {
        return syscall::mmap( addr, length, prot, flags, fildes, off );
    };

    let mut backtrace = Backtrace::new();
    unwind::grab( &mut thread, &mut backtrace );

    let _lock = crate::global::MMAP_LOCK.lock();
    let ptr = syscall::mmap( addr, length, prot, flags, fildes, off );
    if ptr == libc::MAP_FAILED {
        return ptr;
    }

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
    let mut thread = StrongThreadHandle::acquire();
    if !opt::get().gather_mmap_calls {
        thread = None;
    }

    let mut thread = if let Some( thread ) = thread {
        thread
    } else {
        return syscall::munmap( ptr, length );
    };

    let mut backtrace = Backtrace::new();
    unwind::grab( &mut thread, &mut backtrace );

    let _lock = crate::global::MMAP_LOCK.lock();
    let result = syscall::munmap( ptr, length );

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

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn __register_frame( fde: *const u8 ) {
    debug!( "Registering new frame: 0x{:016X}", fde as usize );

    if let Some( original ) = crate::global::SYM_REGISTER_FRAME {
        original( fde )
    } else {
        error!( "__register_frame call ignored since we couldn't find the original symbol" );
    }

    let thread = StrongThreadHandle::acquire();
    unwind::register_frame_by_pointer( fde );
    std::mem::drop( thread );
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn __deregister_frame( fde: *const u8 ) {
    debug!( "Deregistering new frame: 0x{:016X}", fde as usize );

    if let Some( original ) = crate::global::SYM_DEREGISTER_FRAME {
        original( fde )
    } else {
        error!( "__deregister_frame call ignored since we couldn't find the original symbol" );
    }

    let thread = StrongThreadHandle::acquire();
    unwind::deregister_frame_by_pointer( fde );
    std::mem::drop( thread );
}
