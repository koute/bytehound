extern crate jemalloc_sys;
extern crate libc;

#[cfg(prefixed)]
#[test]
fn malloc_is_prefixed() {
    assert_ne!(jemalloc_sys::malloc as usize, libc::malloc as usize)
}

#[cfg(not(prefixed))]
#[test]
fn malloc_is_overridden() {
    assert_eq!(jemalloc_sys::malloc as usize, libc::malloc as usize)
}
