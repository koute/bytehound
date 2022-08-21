#[global_allocator]
static GLOBAL: jemallocator::Jemalloc = jemallocator::Jemalloc;

use jemalloc_sys::sallocx;

fn main() {
    unsafe { jemalloc_common::run_test(); }
    unsafe {
        let a7 = jemalloc_common::alloc( 1 );
        assert_eq!( sallocx(a7 as _, 0), 8 );
        let a8 = jemalloc_common::alloc( 12 );
        assert_eq!( sallocx(a8 as _, 0), 8+8 );
    }
}
