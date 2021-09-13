//! Sets `jemalloc` as the `#[global_allocator]` on targets that support it.
//!
//! Just add `jemallocator-global` as a dependency:
//!
//! ```toml
//! # Cargo.toml
//! [dependencies]
//! jemallocator-global = "0.4.0"
//! ```
//!
//! and `jemalloc` will be used as the `#[global_allocator]` on targets that
//! support it.
//!
//! To unconditionally set `jemalloc` as the `#[global_allocator]` enable the
//! `force_global_jemalloc` cargo feature.

#[macro_use]
extern crate cfg_if;

cfg_if! {
    if #[cfg(any(
        feature = "force_global_jemalloc",
        target_os = "linux",
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd"
    ))] {
        /// Sets `jemalloc` as the `#[global_allocator]`.
        #[global_allocator]
        pub static JEMALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
    }
}

#[cfg(test)]
mod tests {
    // Test that jemallocator-global is enabled automatically in those targets in
    // which it should be enabled:

    macro_rules! check {
        () => {
            #[test]
            fn foo() {
                let _ = super::JEMALLOC;
            }
        };
        ($os_name:tt) => {
            #[cfg(target_os = $os_name)]
            check!();
        };
        ($($os_name:tt),*) => {
            $(check!($os_name);)*
        }
    }

    // If the `force_global_jemalloc` feature is enabled, then it
    // should always be set as the global allocator:
    #[cfg(feature = "force_global_jemalloc")]
    check!();

    // If the `force_global_jemalloc` feature is not enabled, then in the
    // following targets it should be automatically enabled anyways:
    #[cfg(not(feature = "force_global_jemalloc"))]
    check!("linux", "android", "macos", "ios", "freebsd", "netbsd", "openbsd");
}
