#[cfg(prefixed)]
#[test]
fn malloc_is_prefixed() {
    assert_ne!(tikv_jemalloc_sys::malloc as usize, libc::malloc as usize)
}

#[cfg(not(prefixed))]
#[test]
fn malloc_is_overridden() {
    assert_eq!(tikv_jemalloc_sys::malloc as usize, libc::malloc as usize)
}
