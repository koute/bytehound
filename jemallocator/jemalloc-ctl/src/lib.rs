//! `jemalloc` control and introspection.
//!
//! `jemalloc` offers a powerful introspection and control interface through the `mallctl` function.
//! It can be used to tune the allocator, take heap dumps, and retrieve statistics. This crate
//! provides a typed API over that interface.
//!
//! While `mallctl` takes a string to specify an operation (e.g. `stats.allocated` or
//! `stats.arenas.15.muzzy_decay_ms`), the overhead of repeatedly parsing those strings is not
//! ideal. Fortunately, `jemalloc` offers the ability to translate the string ahead of time into a
//! "Management Information Base" (MIB) to speed up future lookups.
//!
//! This crate provides a type for each `mallctl` operation. Calling
//! `$op::{read(), write(x), update(x)}` on the type calls `mallctl` with the
//! string-based API. If the operation will be repeatedly performed, a MIB for
//! the operation can be obtained using `$op.mib()`.
//!
//! # Examples
//!
//! Repeatedly printing allocation statistics:
//!
//! ```no_run
//! use std::thread;
//! use std::time::Duration;
//! use tikv_jemalloc_ctl::{stats, epoch};
//!
//! #[global_allocator]
//! static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
//!
//! fn main() {
//!     loop {
//!         // many statistics are cached and only updated when the epoch is advanced.
//!         epoch::advance().unwrap();
//!
//!         let allocated = stats::allocated::read().unwrap();
//!         let resident = stats::resident::read().unwrap();
//!         println!("{} bytes allocated/{} bytes resident", allocated, resident);
//!         thread::sleep(Duration::from_secs(10));
//!     }
//! }
//! ```
//!
//! Doing the same with the MIB-based API:
//!
//! ```no_run
//! use std::thread;
//! use std::time::Duration;
//! use tikv_jemalloc_ctl::{stats, epoch};
//!
//! #[global_allocator]
//! static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
//!
//! fn main() {
//!     let e = epoch::mib().unwrap();
//!     let allocated = stats::allocated::mib().unwrap();
//!     let resident = stats::resident::mib().unwrap();
//!     loop {
//!         // many statistics are cached and only updated when the epoch is advanced.
//!         e.advance().unwrap();
//!
//!         let allocated = allocated.read().unwrap();
//!         let resident = resident.read().unwrap();
//!         println!("{} bytes allocated/{} bytes resident", allocated, resident);
//!         thread::sleep(Duration::from_secs(10));
//!     }
//! }
//! ```
#![deny(missing_docs, broken_intra_doc_links)]
#![cfg_attr(not(feature = "use_std"), no_std)]
#![cfg_attr(feature = "cargo-clippy", allow(clippy::module_name_repetitions))]

#[cfg(test)]
#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

use crate::std::{fmt, mem, num, ops, ptr, result, slice, str};
#[cfg(not(feature = "use_std"))]
use core as std;
#[cfg(feature = "use_std")]
use std;

#[macro_use]
mod macros;

pub mod arenas;
pub mod config;
mod error;
mod keys;
pub mod opt;
pub mod raw;
pub mod stats;
#[cfg(feature = "use_std")]
pub mod stats_print;
pub mod thread;

pub use error::{Error, Result};
pub use keys::{Access, AsName, Mib, MibStr, Name};

option! {
    version[ str: b"version\0", str: 1 ] => &'static str |
    ops: r |
    docs:
    /// `jemalloc` version string.
    ///
    /// # Example
    ///
    /// ```
    /// # #[global_allocator]
    /// # static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
    /// #
    /// # fn main() {
    /// use tikv_jemalloc_ctl::version;
    /// println!("jemalloc version {}", version::read().unwrap());
    /// let version_mib = version::mib().unwrap();
    /// println!("jemalloc version {}", version_mib.read().unwrap());
    /// # }
    /// ```
    mib_docs: /// See [`version`].
}

option! {
    background_thread[ str: b"background_thread\0", non_str: 1 ] => bool |
    ops: r,w,u |
    docs:
    /// State of internal background worker threads.
    ///
    /// When enabled, background threads are created on demand (the number of
    /// background threads will be no more than the number of CPUs or active
    /// arenas). Threads run periodically and handle purging asynchronously.
    ///
    /// ```
    /// # #[global_allocator]
    /// # static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
    /// #
    /// # fn main() {
    /// # #[cfg(not(target_os = "macos"))] {
    /// #
    /// use tikv_jemalloc_ctl::background_thread;
    /// let bg = background_thread::mib().unwrap();
    /// let s = bg.read().unwrap();
    /// println!("background_threads enabled: {}", s);
    /// let p = background_thread::update(!s).unwrap();
    /// println!("background_threads enabled: {} => {}", p, bg.read().unwrap());
    /// assert_eq!(p, s);
    /// background_thread::write(s).unwrap();
    /// println!("background_threads enabled: {}", bg.read().unwrap());
    /// assert_eq!(p, s);
    /// #
    /// # } // #[cfg(..)]
    /// # }
    /// ```
    mib_docs: /// See [`background_thread`].
}

option! {
    max_background_threads[ str: b"max_background_threads\0", non_str: 1 ] => libc::size_t |
    ops: r, w, u |
    docs:
    /// Maximum number of background threads that will be created.
    ///
    /// ```
    /// # #[global_allocator]
    /// # static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
    /// #
    /// # fn main() {
    /// # #[cfg(not(target_os = "macos"))] {
    /// #
    /// use tikv_jemalloc_ctl::max_background_threads;
    /// let m = max_background_threads::mib().unwrap();
    /// println!("max_background_threads: {}", m.read().unwrap());
    /// m.write(0).unwrap();
    /// assert_eq!(m.read().unwrap(), 0);
    /// #
    /// # } // #[cfg(..)]
    /// # }
    /// ```
    mib_docs: /// See [`max_background_threads`].
}

option! {
    epoch[ str: b"epoch\0", non_str: 1 ] => u64 |
    ops: r, w, u |
    docs:
    /// `jemalloc` epoch.
    ///
    /// Many of the statistics tracked by `jemalloc` are cached. The epoch
    /// controls when they are refreshed.
    ///
    /// # Example
    ///
    /// Advancing the epoch:
    ///
    /// ```
    /// # #[global_allocator]
    /// # static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
    /// #
    /// # fn main() {
    /// #
    /// use tikv_jemalloc_ctl::epoch;
    /// let e = epoch::mib().unwrap();
    /// let a = e.advance().unwrap();
    /// let b = e.advance().unwrap();
    /// assert_eq!(a + 1, b);
    ///
    /// let o = e.update(0).unwrap();
    /// assert_eq!(o, e.read().unwrap());
    /// # }
    mib_docs: /// See [`epoch`].
}

impl epoch {
    /// Advances the epoch returning its old value - see [`epoch`].
    pub fn advance() -> crate::error::Result<u64> {
        Self::update(1)
    }
}

impl epoch_mib {
    /// Advances the epoch returning its old value - see [`epoch`].
    pub fn advance(self) -> crate::error::Result<u64> {
        self.0.update(1)
    }
}
