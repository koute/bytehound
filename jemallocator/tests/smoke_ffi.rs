// Work around https://github.com/gnzlbg/jemallocator/issues/19
#[global_allocator]
static A: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[test]
fn smoke() {
    unsafe {
        let ptr = tikv_jemalloc_sys::malloc(4);
        *(ptr as *mut u32) = 0xDECADE;
        assert_eq!(*(ptr as *mut u32), 0xDECADE);
        tikv_jemalloc_sys::free(ptr);
    }
}
