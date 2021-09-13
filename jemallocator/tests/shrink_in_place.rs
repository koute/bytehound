#![cfg_attr(feature = "alloc_trait", feature(allocator_api))]

use tikv_jemallocator::Jemalloc;

#[global_allocator]
static A: Jemalloc = Jemalloc;

#[test]
#[cfg(feature = "alloc_trait")]
fn shrink_in_place() {
    unsafe {
        use std::alloc::{Alloc, Layout};

        // allocate a "large" block of memory:
        let orig_sz = 10 * 4096;
        let orig_l = Layout::from_size_align(orig_sz, 1).unwrap();
        let ptr = Jemalloc.alloc(orig_l).unwrap();

        // try to shrink it in place to 1 byte - if this succeeds,
        // the size-class of the new allocation should be different
        // than that of the original allocation:
        let new_sz = 1;
        if let Ok(()) = Jemalloc.shrink_in_place(ptr, orig_l, new_sz) {
            // test that deallocating with the new layout succeeds:
            let new_l = Layout::from_size_align(new_sz, 1).unwrap();
            Jemalloc.dealloc(ptr, new_l);
        } else {
            // if shrink in place failed - deallocate with the old layout
            Jemalloc.dealloc(ptr, orig_l);
        }
    }
}
