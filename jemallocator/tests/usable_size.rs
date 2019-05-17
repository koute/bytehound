extern crate jemallocator;

use jemallocator::Jemalloc;

#[global_allocator]
static A: Jemalloc = Jemalloc;

#[test]
fn smoke() {
    let a = Box::new(3_u32);
    assert!(unsafe { jemallocator::usable_size(&*a) } >= 4);
}
