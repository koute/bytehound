use tikv_jemallocator::Jemalloc;

#[global_allocator]
static A: Jemalloc = Jemalloc;

#[test]
fn smoke() {
    let a = Box::new(3_u32);
    assert!(unsafe { tikv_jemallocator::usable_size(&*a) } >= 4);
}
