// Copyright 2019 Octavian Oncescu

#![no_std]

//! A drop-in global allocator wrapper around the [mimalloc](https://github.com/microsoft/mimalloc) allocator.
//! Mimalloc is a general purpose, performance oriented allocator built by Microsoft.
//!
//! ## Usage
//! ```rust,ignore
//! use mimalloc::MiMalloc;
//!
//! #[global_allocator]
//! static GLOBAL: MiMalloc = MiMalloc;
//! ```
//!
//! ## Usage without secure mode
//! By default this library builds mimalloc in secure mode. This means that
//! heap allocations are encrypted, but this results in a 3% increase in overhead.
//!
//! To disable secure mode, in `Cargo.toml`:
//! ```rust,ignore
//! [dependencies]
//! mimalloc = { version = "*", default-features = false }
//! ```

extern crate libmimalloc_sys as ffi;

use core::alloc::{GlobalAlloc, Layout};
use core::ffi::c_void;
use ffi::*;

// `MI_MAX_ALIGN_SIZE` is 16 unless manually overridden:
// https://github.com/microsoft/mimalloc/blob/15220c68/include/mimalloc-types.h#L22
//
// If it changes on us, we should consider either manually overriding it, or
// expose it from the -sys crate (in order to catch updates)
const MI_MAX_ALIGN_SIZE: usize = 16;

// Note: this doesn't take a layout directly because doing so would be wrong for
// reallocation
#[inline]
fn may_use_unaligned_api(size: usize, alignment: usize) -> bool {
    // Required by `GlobalAlloc`. Note that while allocators aren't allowed to
    // unwind in rust, this is only in debug mode, and can only happen if the
    // caller already caused UB by passing in an invalid layout.
    debug_assert!(size != 0 && alignment.is_power_of_two());

    // This logic is based on the discussion [here]. We don't bother with the
    // 3rd suggested test due to it being high cost (calling `mi_good_size`)
    // compared to the other checks, and also feeling like it relies on too much
    // implementation-specific behavior.
    //
    // [here]: https://github.com/microsoft/mimalloc/issues/314#issuecomment-708541845

    (alignment <= MI_MAX_ALIGN_SIZE && size >= alignment)
        || (alignment == size && alignment <= 4096)
}

/// Drop-in mimalloc global allocator.
///
/// ## Usage
/// ```rust,ignore
/// use mimalloc::MiMalloc;
///
/// #[global_allocator]
/// static GLOBAL: MiMalloc = MiMalloc;
/// ```
pub struct MiMalloc;

unsafe impl GlobalAlloc for MiMalloc {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if may_use_unaligned_api(layout.size(), layout.align()) {
            mi_malloc(layout.size()) as *mut u8
        } else {
            mi_malloc_aligned(layout.size(), layout.align()) as *mut u8
        }
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if may_use_unaligned_api(layout.size(), layout.align()) {
            mi_zalloc(layout.size()) as *mut u8
        } else {
            mi_zalloc_aligned(layout.size(), layout.align()) as *mut u8
        }
    }

    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        mi_free(ptr as *mut c_void);
    }

    #[inline]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if may_use_unaligned_api(new_size, layout.align()) {
            mi_realloc(ptr as *mut c_void, new_size) as *mut u8
        } else {
            mi_realloc_aligned(ptr as *mut c_void, new_size, layout.align()) as *mut u8
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_frees_allocated_memory() {
        unsafe {
            let layout = Layout::from_size_align(8, 8).unwrap();
            let alloc = MiMalloc;

            let ptr = alloc.alloc(layout);
            alloc.dealloc(ptr, layout);
        }
    }

    #[test]
    fn it_frees_allocated_big_memory() {
        unsafe {
            let layout = Layout::from_size_align(1 << 20, 32).unwrap();
            let alloc = MiMalloc;

            let ptr = alloc.alloc(layout);
            alloc.dealloc(ptr, layout);
        }
    }

    #[test]
    fn it_frees_zero_allocated_memory() {
        unsafe {
            let layout = Layout::from_size_align(8, 8).unwrap();
            let alloc = MiMalloc;

            let ptr = alloc.alloc_zeroed(layout);
            alloc.dealloc(ptr, layout);
        }
    }

    #[test]
    fn it_frees_zero_allocated_big_memory() {
        unsafe {
            let layout = Layout::from_size_align(1 << 20, 32).unwrap();
            let alloc = MiMalloc;

            let ptr = alloc.alloc_zeroed(layout);
            alloc.dealloc(ptr, layout);
        }
    }

    #[test]
    fn it_frees_reallocated_memory() {
        unsafe {
            let layout = Layout::from_size_align(8, 8).unwrap();
            let alloc = MiMalloc;

            let ptr = alloc.alloc(layout);
            let ptr = alloc.realloc(ptr, layout, 16);
            alloc.dealloc(ptr, layout);
        }
    }

    #[test]
    fn it_frees_reallocated_big_memory() {
        unsafe {
            let layout = Layout::from_size_align(1 << 20, 32).unwrap();
            let alloc = MiMalloc;

            let ptr = alloc.alloc(layout);
            let ptr = alloc.realloc(ptr, layout, 2 << 20);
            alloc.dealloc(ptr, layout);
        }
    }
}
