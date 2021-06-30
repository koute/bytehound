#![cfg_attr(feature = "alloc_trait", feature(allocator_api))]

extern crate jemallocator;
use jemallocator::Jemalloc;

#[global_allocator]
static A: Jemalloc = Jemalloc;

#[test]
#[cfg(feature = "alloc_trait")]
fn shrink_in_place() {
    unsafe {
        use std::alloc::{Alloc, Layout};

        // allocate 7 bytes which end up in the 8 byte size-class as long as
        // jemalloc's default size classes are used:
        let orig_sz = 7;
        let orig_l = Layout::from_size_align(orig_sz, 1).unwrap();
        let ptr = Jemalloc.alloc(orig_l).unwrap();

        // try to grow it in place by 1 byte - it should grow without problems:
        let new_sz = orig_sz + 1;
        assert!(Jemalloc.grow_in_place(ptr, orig_l, new_sz).is_ok());
        let new_l = Layout::from_size_align(orig_sz + 1, 1).unwrap();

        // trying to do it again fails because it would require moving the
        // allocation to a different size class which jemalloc's xallocx does not
        // do:
        let new_sz = new_sz + 1;
        assert!(Jemalloc.grow_in_place(ptr, new_l, new_sz).is_err());

        Jemalloc.dealloc(ptr, new_l)
    }
}
