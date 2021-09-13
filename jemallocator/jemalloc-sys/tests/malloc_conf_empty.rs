#[test]
fn malloc_conf_empty() {
    unsafe {
        assert!(tikv_jemalloc_sys::malloc_conf.is_none());
    }
}
