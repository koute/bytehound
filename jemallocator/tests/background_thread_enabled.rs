//! Test enabling / disabling background threads at run-time if the
//! library was compiled with background thread run-time support.
#![cfg(feature = "background_threads_runtime_support")]
#![cfg(not(feature = "unprefixed_malloc_on_supported_platforms"))]
#![cfg(not(target_env = "musl"))]

use tikv_jemallocator::Jemalloc;

#[global_allocator]
static A: Jemalloc = Jemalloc;

union U {
    x: &'static u8,
    y: &'static libc::c_char,
}

// Even if background threads are not enabled at run-time by default
// at configuration time, this enables them.
#[allow(non_upper_case_globals)]
#[export_name = "_rjem_mp_malloc_conf"]
pub static malloc_conf: Option<&'static libc::c_char> = Some(unsafe {
    U {
        x: &b"background_thread:true\0"[0],
    }
    .y
});

#[test]
fn background_threads_enabled() {
    // Background threads are unconditionally enabled at run-time by default.
    assert_eq!(
        tikv_jemalloc_ctl::opt::background_thread::read().unwrap(),
        true
    );
}
