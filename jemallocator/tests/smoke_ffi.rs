extern crate jemallocator;
extern crate jemalloc_sys;

// Work around https://github.com/alexcrichton/jemallocator/issues/19
#[global_allocator]
static A: jemallocator::Jemalloc = jemallocator::Jemalloc;

#[test]
fn smoke() {
    unsafe {
        let ptr = jemalloc_sys::malloc(4);
        *(ptr as *mut u32) = 0xDECADE;
        assert_eq!(*(ptr as *mut u32), 0xDECADE);
        jemalloc_sys::free(ptr);
    }
}
