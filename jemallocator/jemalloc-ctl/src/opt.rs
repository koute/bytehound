//! `jemalloc`'s run-time configuration.
//!
//! These settings are controlled by the `MALLOC_CONF` environment variable.

option! {
    abort[ str: b"opt.abort\0", non_str: 2 ] => bool |
    ops: r |
    docs:
    /// Whether `jemalloc` calls `abort(3)` on most warnings.
    ///
    /// This is disabled by default unless `--enable-debug` was specified during
    /// build configuration.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[global_allocator]
    /// # static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
    /// #
    /// # fn main() {
    /// use tikv_jemalloc_ctl::opt;
    /// let abort = opt::abort::mib().unwrap();
    /// println!("abort on warning: {}", abort.read().unwrap());
    /// # }
    /// ```
    mib_docs: /// See [`abort`].
}

option! {
    dss[ str: b"opt.dss\0", str: 2 ] => &'static str |
    ops: r |
    docs:
    /// The `dss` (`sbrk(2)`) allocation precedence as related to `mmap(2)`
    /// allocation.
    ///
    /// The following settings are supported if `sbrk(2)` is supported by the
    /// operating system: "disabled", "primary", and "secondary"; otherwise only
    /// "disabled" is supported. The default is "secondary" if `sbrk(2)` is
    /// supported by the operating system; "disabled" otherwise.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[global_allocator]
    /// # static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
    /// #
    /// # fn main() {
    /// use tikv_jemalloc_ctl::opt;
    /// let dss = opt::dss::read().unwrap();
    /// println!("dss priority: {}", dss);
    /// # }
    /// ```
    mib_docs: /// See [`dss`].
}

option! {
    narenas[ str: b"opt.narenas\0", non_str: 2 ] => libc::c_uint |
    ops: r |
    docs:
    /// Maximum number of arenas to use for automatic multiplexing of threads
    /// and arenas.
    ///
    /// The default is four times the number of CPUs, or one if there is a
    /// single CPU.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[global_allocator]
    /// # static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
    /// #
    /// # fn main() {
    /// use tikv_jemalloc_ctl::opt;
    /// let narenas = opt::narenas::read().unwrap();
    /// println!("number of arenas: {}", narenas);
    /// # }
    /// ```
    mib_docs: /// See [`narenas`].
}

option! {
    junk[ str: b"opt.junk\0", str: 2 ] => &'static str |
    ops: r |
    docs:
    /// `jemalloc`'s junk filling mode.
    ///
    /// Requires `--enable-fill` to have been specified during build
    /// configuration.
    ///
    /// If set to "alloc", each byte of uninitialized allocated memory will be
    /// set to `0x5a`. If set to "free", each byte of deallocated memory will be set
    /// to `0x5a`. If set to "true", both allocated and deallocated memory will be
    /// initialized, and if set to "false" junk filling will be disabled. This is
    /// intended for debugging and will impact performance negatively.
    ///
    /// The default is "false", unless `--enable-debug` was specified during
    /// build configuration, in
    /// which case the default is "true".
    ///
    /// # Examples
    ///
    /// ```
    /// # #[global_allocator]
    /// # static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
    /// #
    /// # fn main() {
    /// use tikv_jemalloc_ctl::opt;
    /// let junk = opt::junk::read().unwrap();
    /// println!("junk filling: {}", junk);
    /// # }
    /// ```
    mib_docs: /// See [`junk`].
}

option! {
    zero[ str: b"opt.zero\0", non_str: 2 ] => bool |
    ops: r |
    docs:
    /// `jemalloc`'s zeroing behavior.
    ///
    /// Requires `--enable-fill` to have been specified during build
    /// configuration.
    ///
    /// If enabled, `jemalloc` will initialize each byte of uninitialized
    /// allocated memory to 0. This is intended for debugging and will impact
    /// performance negatively. It is disabled by default.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[global_allocator]
    /// # static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
    /// #
    /// # fn main() {
    /// use tikv_jemalloc_ctl::opt;
    /// let zero = opt::zero::read().unwrap();
    /// println!("zeroing: {}", zero);
    /// # }
    /// ```
    mib_docs: /// See [`zero`].
}

option! {
    tcache[ str: b"opt.tcache\0", non_str: 2 ] => bool |
    ops: r |
    docs:
    /// Thread-local allocation caching behavior.
    ///
    /// Thread-specific caching allows many allocations to be satisfied without
    /// performing any thread synchronization, at the cost of increased memory
    /// use. This is enabled by default.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[global_allocator]
    /// # static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
    /// #
    /// # fn main() {
    /// use tikv_jemalloc_ctl::opt;
    /// let tcache = opt::tcache::read().unwrap();
    /// println!("thread-local caching: {}", tcache);
    /// # }
    /// ```
    mib_docs: /// See [`tcache`].
}

option! {
    lg_tcache_max[ str: b"opt.lg_tcache_max\0", non_str: 2 ] => libc::size_t |
    ops: r |
    docs:
    /// Maximum size class (log base 2) to cache in the thread-specific cache
    /// (`tcache`).
    ///
    /// At a minimum, all small size classes are cached, and at a maximum all
    /// large size classes are cached. The default maximum is 32 KiB (2^15).
    ///
    /// # Examples
    ///
    /// ```
    /// # #[global_allocator]
    /// # static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
    /// #
    /// # fn main() {
    /// use tikv_jemalloc_ctl::opt;
    /// let lg_tcache_max = opt::lg_tcache_max::read().unwrap();
    /// println!("max cached allocation size: {}", 1 << lg_tcache_max);
    /// # }
    /// ```
    mib_docs: /// See [`lg_tcache_max`].
}

option! {
    background_thread[ str: b"opt.background_thread\0", non_str: 2 ] => bool |
    ops: r |
    docs:
    /// `jemalloc`'s default initialization behavior for background threads.
    ///
    /// `jemalloc` automatically spawns background worker threads on
    /// initialization (first `jemalloc` call) if this option is enabled. By
    /// default this option is disabled - `malloc_conf=background_thread:true`
    /// changes its default.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[global_allocator]
    /// # static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
    /// #
    /// # fn main() {
    /// use tikv_jemalloc_ctl::opt;
    /// let background_thread = opt::background_thread::read().unwrap();
    /// println!("background threads since initialization: {}", background_thread);
    /// # }
    /// ```
    mib_docs: /// See [`background_thread`].
}
