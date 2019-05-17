# jemalloc-sys - Rust bindings to the `jemalloc` C library

> Note: the Rust allocator API is implemented for `jemalloc` in the
> [`jemallocator`](https://crates.io/crates/jemallocator) crate.

`jemalloc` is a general purpose memory allocation, its documentation
 can be found here:

* [API documentation][jemalloc_docs]
* [Wiki][jemalloc_wiki] (design documents, presentations, profiling, debugging, tuning, ...)

**Current jemalloc version**: 5.1.

# Feature flags

This crate provides following cargo feature flags:

* `profiling`: configure `jemalloc` with `--enable-prof`.
* `stats`: configure `jemalloc` with `--enable-stats`.
* `debug`: configure `jemalloc` with `--enable-debug`.
* `bg_thread` (enabled by default): when disabled, configure `jemalloc` with
  `--with-malloc-conf=background_thread:false`.
* `unprefixed_malloc_on_supported_platforms`:
  when disabled, configure `jemalloc` with `--with-jemalloc-prefix=_rjem_`.
  Enabling this causes symbols like `malloc` to be emitted without a prefix,
  overriding the ones defined by libc.
  This usually causes C and C++ code linked in the same program to use `jemalloc` as well.

  On some platforms prefixes are always used
  because unprefixing is known to cause segfaults due to allocator mismatches.

See [`jemalloc/INSTALL.md`](https://github.com/jemalloc/jemalloc/blob/dev/INSTALL.md#advanced-configuration).

# License

This project is licensed under either of

 * Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or
   http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or
   http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in jemallocator by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
