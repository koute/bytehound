# A memory profiler for Linux

## Features

   * Can be used to analyze memory leaks, see where exactly the memory is being
     consumed, identify temporary allocations and investigate excessive memory fragmentation
   * Gathers every allocation and deallocation, along with full stack traces
   * Can dynamically cull temporary allocations allowing you to profile over a long
     period of time
   * Uses a custom, tailor-made stack unwinding implementation which makes it
     a lot cheaper than other similar tools, potentially up to orders of magnitude
     faster in some cases
   * Can export the data it gathered into various different formats; it can
     export the data as JSON (so you can analyze it yourself if you want), as
     Heaptrack (so you can use the excellent [Heaptrack GUI] for analysis)
     and as a flamegraph
   * Has its own Web-based GUI which can be used for analysis
   * Can dynamically stream the profiling data to another machine instead
     of saving it locally, which is useful for profiling on memory-constrained systems
   * Supports AMD64, ARM, AArch64 and MIPS64 architectures (where MIPS64 requires a tiny out-of-tree kernel patch for `perf_event_open`)

[Heaptrack GUI]: https://github.com/KDE/heaptrack

## Screenshots

<p align="center">
    <img src="screenshot_gui_graphs.png">
</p>

<p align="center">
    <img src="screenshot_gui_allocations.png">
</p>

## Building

1. Install GCC, Rust nightly and the Yarn package manager (for building the GUI)
2. Build it:

        $ cargo build --release -p memory-profiler
        $ cargo build --release -p memory-profiler-cli

3. Grab the binaries from `target/release/libmemory_profiler.so` and `target/release/memory-profiler-cli`

## Usage

### Basic usage

    $ export MEMORY_PROFILER_LOG=warn
    $ LD_PRELOAD=./libmemory_profiler.so ./your_application
    $ ./memory-profiler-cli server memory-profiling_*.dat

Then open your Web browser and point it at `http://localhost:8080` to access the GUI.

## Environment variables used by `libmemory_profiler.so`

### `MEMORY_PROFILER_OUTPUT`

Default: `memory-profiling_%e_%t_%p.dat`

A path to a file to which the data will be written to.

This environment variable supports placeholders which will be replaced at
runtime with the following:
   * `%p` -> PID of the process
   * `%t` -> number of seconds since UNIX epoch
   * `%e` -> name of the executable
   * `%n` -> auto-incrementing counter (0, 1, .., 9, 10, etc.)

### `MEMORY_PROFILER_LOG`

Default: unset

The log level to use; possible values:
   * `trace`
   * `debug`
   * `info`
   * `warn`
   * `error`

Unset by default, which disables logging altogether.

### `MEMORY_PROFILER_LOGFILE`

Default: unset

Path to the file to which the logs will be written to; if unset the logs will
be emitted to stderr (if they're enabled with `MEMORY_PROFILER_LOG`).

This supports placeholders similar to `MEMORY_PROFILER_OUTPUT` (except `%n`).

### `MEMORY_PROFILER_CULL_TEMPORARY_ALLOCATIONS`

Default: `0`

When set to `1` the profiler will cull temporary allocations
and omit them from the output.

Use this if you only care about memory leaks or you want
to do long term profiling over several days.

### `MEMORY_PROFILER_TEMPORARY_ALLOCATION_LIFETIME_THRESHOLD`

Default: `10000`

The minimum lifetime of an allocation, in milliseconds, to **not** be
considered a temporary allocation, and hence not get culled.

Only makes sense when `MEMORY_PROFILER_CULL_TEMPORARY_ALLOCATIONS` is turned on.

### `MEMORY_PROFILER_TEMPORARY_ALLOCATION_PENDING_THRESHOLD`

Default: `65536`

The maximum number of allocations to be kept in memory when tracking which
allocations are temporary and which are not.

Every allocation whose lifetime hasn't yet crossed the temporary allocation interval
will be temporarily kept in a buffer, and removed from it once it either gets deallocated
or its lifetime crosses the temporary allocation interval.

If the number of allocations stored in this buffer exceeds the value set here the buffer will be
cleared and all of the allocations contained within will be written to disk, regardless of their lifetime.

Only makes sense when `MEMORY_PROFILER_CULL_TEMPORARY_ALLOCATIONS` is turned on.

### `MEMORY_PROFILER_DISABLE_BY_DEFAULT`

Default: `0`

When set to `1` the tracing will be disabled be default at startup.

### `MEMORY_PROFILER_REGISTER_SIGUSR1`

Default: `1`

When set to `1` the profiler will register a `SIGUSR1` signal handler
which can be used to toggle (enable or disable) profiling.

### `MEMORY_PROFILER_REGISTER_SIGUSR2`

Default: `1`

When set to `1` the profiler will register a `SIGUSR2` signal handler
which can be used to toggle (enable or disable) profiling.

### `MEMORY_PROFILER_ENABLE_SERVER`

Default: `0`

When set to `1` the profiled process will start an embedded server which can
be used to stream the profiling data through TCP using `memory-profiler-cli gather` and `memory-profiler-gather`.

This server will only be started when profiling is first enabled.

### `MEMORY_PROFILER_BASE_SERVER_PORT`

Default: `8100`

TCP port of the embedded server on which the profiler will listen on.

If the profiler won't be able to bind a socket to this port it will
try to find the next free port to bind to. It will succesively probe
the ports in a linear fashion, e.g. 8100, 8101, 8102, etc.,
up to 100 times before giving up.

Requires `MEMORY_PROFILER_ENABLE_SERVER` to be set to `1`.

### `MEMORY_PROFILER_ENABLE_BROADCAST`

Default: `0`

When set to `1` the profiled process will send UDP broadcasts announcing that
it's being profiled. This is used by `memory-profiler-cli gather` and `memory-profiler-gather`
to automatically discover `memory-profiler` instances to which to connect.

Requires `MEMORY_PROFILER_ENABLE_SERVER` to be set to `1`.

### `MEMORY_PROFILER_PRECISE_TIMESTAMPS`

Default: `0`

Decides whenever timestamps will be gathered for every event, or only for chunks of events.
When enabled the timestamps will be more precise at a cost of extra CPU usage.

### `MEMORY_PROFILER_WRITE_BINARIES_TO_OUTPUT`

Default: `1`

Controls whenever the profiler will embed the profiled application (and all of the libraries
used by the application) inside of the profiling data it writes to disk.

This makes it possible to later decode the profiling data without having to manually
hunt down the original binaries.

### `MEMORY_PROFILER_ZERO_MEMORY`

Default: `0`

Decides whenever `malloc` will behave like `calloc` and fill the memory it returns with zeros.

### `MEMORY_PROFILER_BACKTRACE_CACHE_SIZE`

Default: `32768`

Controls the size of the internal backtrace cache used to deduplicate emitted stack traces.

### `MEMORY_PROFILER_GATHER_MMAP_CALLS`

Default: `0`

Controls whenever the profiler will also gather calls to `mmap` and `munmap`.

(Those are *not* treated as allocations and are only available under the `/mmaps` API endpoint.)

### `MEMORY_PROFILER_USE_SHADOW_STACK`

Default: `1`

Whenever to use a more intrusive, faster unwinding algorithm; enabled by default.

Setting it to `0` will on average significantly slow down unwinding. This option
is provided only for debugging purposes.

## Enabling full debug logs

By default the profiler is compiled with most of its debug logs disabled for performance reasons.
To reenable them be sure to recompile it with the `debug-logs` feature, e.g. like this:

    $ cd preload
    $ cargo build --release --features debug-logs

## License

Licensed under either of

  * Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
  * MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
