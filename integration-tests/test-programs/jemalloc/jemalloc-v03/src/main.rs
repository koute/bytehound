#[global_allocator]
static GLOBAL: jemallocator::Jemalloc = jemallocator::Jemalloc;

fn main() {
    unsafe {
        jemalloc_common::run_test()
    }
}
