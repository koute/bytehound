# jemalloc-sys - Rust bindings to the `jemalloc` C library

[![Travis-CI Status]][travis] [![Appveyor Status]][appveyor] [![Latest Version]][crates.io] [![docs]][docs.rs]

> Note: the Rust allocator API is implemented for `jemalloc` in the
> [`jemallocator`](https://crates.io/crates/jemallocator) crate.

## Documentation

* [Latest release (docs.rs)][docs.rs]
* [master branch`][master_docs]

`jemalloc` is a general purpose memory allocator, its documentation

 can be found here:

* [API documentation][jemalloc_docs]
* [Wiki][jemalloc_wiki] (design documents, presentations, profiling, debugging, tuning, ...)

[jemalloc_docs]: http://jemalloc.net/jemalloc.3.html
[jemalloc_wiki]: https://github.com/jemalloc/jemalloc/wiki

**Current jemalloc version**: 5.1.

## Platform support

See the platform support of the
[`jemallocator`](https://crates.io/crates/jemallocator) crate.

## Features

Most features correspond to `jemalloc` features - the reference is
[`jemalloc/INSTALL.md`][jemalloc_install].

### Cargo features

This crate provides following cargo feature flags:

* `profiling` (configure `jemalloc` with `--enable-prof`): Enable heap profiling
  and leak detection functionality. See jemalloc's "opt.prof" option
  documentation for usage details. When enabled, there are several approaches to
  backtracing, and the configure script chooses the first one in the following
  list that appears to function correctly:

  * `libunwind` (requires --enable-prof-libunwind)
  * `libgcc` (unless --disable-prof-libgcc)
  * `gcc intrinsics` (unless --disable-prof-gcc)

* `stats` (configure `jemalloc` with `--enable-stats`): Enable statistics
  gathering functionality. See the `jemalloc`'s "`opt.stats_print`" option
  documentation for usage details.
  
* `debug` (configure `jemalloc` with `--enable-debug`): Enable assertions and
  validation code. This incurs a substantial performance hit, but is very useful
  during application development.
  
* `background_threads_runtime_support` (enabled by default): enables
  background-threads run-time support when building `jemalloc-sys` on some POSIX
  targets supported by `jemalloc`. Background threads are disabled at run-time
  by default. This option allows dynamically enabling them at run-time.

* `background_threads` (disabled by default): enables background threads by
  default at run-time. When set to true, background threads are created on
  demand (the number of background threads will be no more than the number of
  CPUs or active arenas). Threads run periodically, and handle purging
  asynchronously. When switching off, background threads are terminated
  synchronously. Note that after `fork(2)` function, the state in the child
  process will be disabled regardless the state in parent process. See
  `stats.background_thread` for related stats. `opt.background_thread` can be
  used to set the default option. The background thread is only available on
  selected pthread-based platforms.

* `unprefixed_malloc_on_supported_platforms`: when disabled, configure
  `jemalloc` with `--with-jemalloc-prefix=_rjem_`. Enabling this causes symbols
  like `malloc` to be emitted without a prefix, overriding the ones defined by
  libc. This usually causes C and C++ code linked in the same program to use
  `jemalloc` as well. On some platforms prefixes are always used because
  unprefixing is known to cause segfaults due to allocator mismatches.
  
* `disable_initial_exec_tls` (disabled by default): when enabled, jemalloc is
  built with the `--disable-initial-exec-tls` option. It disables the 
  initial-exec TLS model for jemalloc's internal thread-local storage (on those 
  platforms that support explicit settings). This can allow jemalloc to be 
  dynamically loaded after program startup (e.g. using dlopen). If you encounter
  the error `yourlib.so: cannot allocate memory in static TLS block`, you'll 
  likely want to enable this.

### Environment variables

`jemalloc` options taking values are passed via environment variables using the
schema `JEMALLOC_SYS_{KEY}=VALUE` where the `KEY` names correspond to the
`./configure` options of `jemalloc` where the words are capitalized and the
hyphens `-` are replaced with underscores `_`(see
[`jemalloc/INSTALL.md`][jemalloc_install]):

* `JEMALLOC_SYS_WITH_MALLOC_CONF=<malloc_conf>`: Embed `<malloc_conf>` as a
  run-time options string that is processed prior to the `malloc_conf` global
  variable, the `/etc/malloc.conf` symlink, and the `MALLOC_CONF` environment
  variable (note: this variable might be prefixed as `_RJEM_MALLOC_CONF`). For
  example, to change the default decay time to 30 seconds:
  
  ```
  JEMALLOC_SYS_WITH_MALLOC_CONF=decay_ms:30000
  ```

* `JEMALLOC_SYS_WITH_LG_PAGE=<lg-page>`: Specify the base 2 log of the allocator
  page size, which must in turn be at least as large as the system page size. By
  default the configure script determines the host's page size and sets the
  allocator page size equal to the system page size, so this option need not be
  specified unless the system page size may change between configuration and
  execution, e.g. when cross compiling.
  
* `JEMALLOC_SYS_WITH_LG_HUGEPAGE=<lg-hugepage>`: Specify the base 2 log of the
  system huge page size. This option is useful when cross compiling, or when
  overriding the default for systems that do not explicitly support huge pages.
  
  
* `JEMALLOC_SYS_WITH_LG_QUANTUM=<lg-quantum>`: Specify the base 2 log of the
  minimum allocation alignment. jemalloc needs to know the minimum alignment
  that meets the following C standard requirement (quoted from the April 12,
  2011 draft of the C11 standard):
  
  > The pointer returned if the allocation succeeds is suitably aligned so that
  > it may be assigned to a pointer to any type of object with a fundamental
  > alignment requirement and then used to access such an object or an array of
  > such objects in the space allocated [...]

  This setting is architecture-specific, and although jemalloc includes known
  safe values for the most commonly used modern architectures, there is a
  wrinkle related to GNU libc (glibc) that may impact your choice of . On most
  modern architectures, this mandates 16-byte alignment (=4), but the glibc
  developers chose not to meet this requirement for performance reasons. An old
  discussion can be found at https://sourceware.org/bugzilla/show_bug.cgi?id=206
  . Unlike glibc, jemalloc does follow the C standard by default (caveat:
  jemalloc technically cheats for size classes smaller than the quantum), but
  the fact that Linux systems already work around this allocator noncompliance
  means that it is generally safe in practice to let jemalloc's minimum
  alignment follow glibc's lead. If you specify `JEMALLOC_SYS_WITH_LG_QUANTUM=3`
  during configuration, jemalloc will provide additional size classes that are
  not 16-byte-aligned (24, 40, and 56).

* `JEMALLOC_SYS_WITH_LG_VADDR=<lg-vaddr>`: Specify the number of significant
  virtual address bits. By default, the configure script attempts to detect
  virtual address size on those platforms where it knows how, and picks a
  default otherwise. This option may be useful when cross-compiling.

* `JEMALLOC_SYS_GIT_DEV_BRANCH`: when this environment variable is defined, the
  latest commit from `jemalloc`'s dev branch is fetched from
  `https://github.com/jemalloc/jemalloc` and built.

[jemalloc_install]: https://github.com/jemalloc/jemalloc/blob/dev/INSTALL.md#advanced-configuration

## License

This project is licensed under either of

 * Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or
   http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or
   http://opensource.org/licenses/MIT)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in `jemalloc-sys` by you, as defined in the Apache-2.0 license,
shall be dual licensed as above, without any additional terms or conditions.

[travis]: https://travis-ci.org/gnzlbg/jemallocator
[Travis-CI Status]: https://travis-ci.org/gnzlbg/jemallocator.svg?branch=master
[appveyor]: https://ci.appveyor.com/project/gnzlbg/jemallocator/branch/master
[Appveyor Status]: https://ci.appveyor.com/api/projects/status/github/gnzlbg/jemallocator?branch=master&svg=true
[Latest Version]: https://img.shields.io/crates/v/jemalloc-sys.svg
[crates.io]: https://crates.io/crates/jemalloc-ctl
[docs]: https://docs.rs/jemalloc-sys/badge.svg
[docs.rs]: https://docs.rs/jemalloc-sys/
[master_docs]: https://gnzlbg.github.io/jemallocator/jemalloc-sys
