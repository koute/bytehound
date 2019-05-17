This is a fork of https://github.com/alexcrichton/jemallocator that includes a modified copy
of jemalloc which instead of using `mmap` and `munmap` uses our own wrappers for those functions.

# jemallocator

[![Build Status](https://travis-ci.org/alexcrichton/jemallocator.svg?branch=master)](https://travis-ci.org/alexcrichton/jemallocator) [![Build Status](https://ci.appveyor.com/api/projects/status/github/alexcrichton/jemallocator?branch=master&svg=true)](https://ci.appveyor.com/project/alexcrichton/jemallocator/branch/master)

[Documentation](https://docs.rs/jemallocator)

A Rust allocator crate which links to [jemalloc](http://jemalloc.net/)
and provides a `Jemalloc` unit type for use with the `#[global_allocator]` attribute.

Usage:

```toml
# Cargo.toml
[dependencies]
jemallocator = "0.1.8"
```

Rust:

```rust
extern crate jemallocator;

#[global_allocator]
static ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;
```

And that's it! Once you've defined this `static` then jemalloc will be used for
all allocations requested by Rust code in the same program.


# Feature flags

This crate has some Cargo feature flags:

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
for inclusion in `jemallocator` by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
