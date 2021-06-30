extern crate jemalloc_ctl;
extern crate jemallocator;
extern crate libc;

use jemalloc_ctl::{Access, AsName};
use jemallocator::Jemalloc;
use std::alloc::{GlobalAlloc, Layout};

#[global_allocator]
static A: Jemalloc = Jemalloc;

#[test]
fn smoke() {
    let layout = Layout::from_size_align(100, 8).unwrap();
    unsafe {
        let ptr = Jemalloc.alloc(layout.clone());
        assert!(!ptr.is_null());
        Jemalloc.dealloc(ptr, layout);
    }
}

#[test]
fn ctl_get_set() {
    let epoch: u64 = "epoch\0".name().read().unwrap();
    assert!(epoch > 0);
    "epoch\0".name().write(epoch).unwrap();
}

#[test]
#[should_panic]
fn ctl_panic_empty_get() {
    let _: u64 = "".name().read().unwrap();
}

#[test]
#[should_panic]
fn ctl_panic_empty_set() {
    let epoch: u64 = "epoch\0".name().read().unwrap();
    "".name().write(epoch).unwrap();
}

#[test]
#[should_panic]
fn ctl_panic_non_null_terminated_get() {
    let _: u64 = "epoch".name().read().unwrap();
}

#[test]
#[should_panic]
fn ctl_panic_non_null_terminated_set() {
    let epoch: u64 = "epoch\0".name().read().unwrap();
    "epoch".name().write(epoch).unwrap();
}
