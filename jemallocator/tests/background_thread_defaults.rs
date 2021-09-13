//! Test background threads run-time default settings.

use tikv_jemallocator::Jemalloc;

#[global_allocator]
static A: Jemalloc = Jemalloc;

// Returns true if background threads are enabled.
fn background_threads() -> bool {
    tikv_jemalloc_ctl::opt::background_thread::read().unwrap()
}

#[test]
fn background_threads_runtime_defaults() {
    if !cfg!(feature = "background_threads_runtime_support") {
        // If the crate was compiled without background thread support,
        // then background threads are always disabled at run-time by default:
        assert_eq!(background_threads(), false);
        return;
    }

    if cfg!(feature = "background_threads") {
        // The crate was compiled with background-threads enabled by default:
        assert_eq!(background_threads(), true);
    } else {
        // The crate was compiled with background-threads disabled by default:
        assert_eq!(background_threads(), false);
    }
}
