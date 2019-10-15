[![Build Status](https://api.travis-ci.org/koute/memory-profiler.svg)](https://travis-ci.org/koute/memory-profiler)

# A memory profiler for Linux

## Features

   * Can be used to analyze memory leaks, see where exactly the memory is being
     consumed, identify temporary allocations and investigate excessive memory fragmentation
   * Gathers every allocation and deallocation, along with full stack traces
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

1. Install GCC, Rust and the Yarn package manager (for building the GUI)
2. Build it:

        $ cargo build --release -p memory-profiler
        $ cargo build --release -p memory-profiler-cli

3. Grab the binaries from `target/release/libmemory_profiler.so` and `target/release/memory-profiler-cli`

## Usage

### Basic usage

    $ LD_PRELOAD=./libmemory_profiler.so ./your_application
    $ ./memory-profiler-cli server memory-profiling_*.dat

Then open your Web browser and point it at `http://localhost:8080` to access the GUI.

If you'd rather not use the GUI you can also make use of the REST API exposed by the server.
For example:

   * Generate a flamegraph of leaked allocations:

         $ curl "http://localhost:8080/data/last/export/flamegraph?lifetime=only_leaked" > flame.svg

   * Export the leaked allocations as an ASCII tree:

         $ curl "http://localhost:8080/data/last/allocation_ascii_tree?lifetime=only_leaked"

   * Export the biggest three allocations made by the application to JSON: (You should pipe the output to `json_reformat` for human readable output.)

         $ curl "http://localhost:8080/data/last/allocations?sort_by=size&order=dsc&count=3"

   * Export the biggest three call sites with at least 10 allocations where at least 50% are leaked:

         $ curl "http://localhost:8080/data/last/allocation_groups?group_allocations_min=10&group_leaked_allocations_min=50%&sort_by=all.size&count=3"

## REST API exposed by `memory-profiler-cli server`

Available endpoints:

   * A list of loaded data files:

         /list

   * JSON containing a list of matched allocations:

         /data/<id>/allocations?<allocation_filter>&sort_by=<sort_by>&order=<order>&count=<count>&skip=<skip>

   * JSON whose each entry corresponds to a group of matched allocations from a single, unique backtrace:

         /data/<id>/allocation_groups?<allocation_filter>&sort_by=<group_sort_by>&order=<order>&count=<count>&skip=<skip>

   * An ASCII tree with matched allocations:

         /data/<id>/allocation_ascii_tree?<allocation_filter>`

   * Exports matched allocations as a flamegraph:

         /data/<id>/export/flamegraph?<allocation_filter>

   * Exports matched allocations into a format accepted by [flamegraph.pl]:

         /data/<id>/export/flamegraph.pl?<allocation_filter>

   * Exports matched allocations into a format accepted by [Heaptrack GUI]:

         /data/<id>/export/heaptrack?<allocation_filter>

   * JSON containing a list of `mmap` calls:

         /data/<id>/mmaps

   * JSON containing a list of `mallopt` calls:

         /data/<id>/mallopts

[flamegraph.pl]: https://github.com/brendangregg/FlameGraph/blob/master/flamegraph.pl

The `<id>` can either be an actual ID of a loaded data file which you can get by querying
the `/list` endpoint, or can be equal to `last` which will use the last loaded data file.

The `<allocation_filter>` can be composed of any of the following parameters:

   * `from`, `to` - a timestamp in seconds or a percentage (of total runtime)
                    specifying the chronological range of matched allocations
   * `lifetime` - an enum specifying the lifetime of matched allocations:
      * `all` - matches every allocation (default)
      * `only_leaked` - matches only leaked allocations
      * `only_not_deallocated_in_current_range` - matches allocation which were not deallocated in the interval specified by `from`/`to`
      * `only_deallocated_in_current_range` - matches allocations which were deallocated in the interval specified by `from`/`to`
      * `only_temporary` - matches only temporary allocations
      * `only_whole_group_leaked` - matches only allocations whose whole group (that is - every allocation from a given call site) was leaked
   * `address_min`, `address_max` - an integer with a minimum/maximum address of matched allocations
   * `size_min`, `size_max` - an integer with a minimum/maximum size of matched allocations in bytes
   * `lifetime_min`, `lifetime_max` - an integer with a minimum/maximum lifetime of matched allocations in seconds
   * `backtrace_depth_min`, `backtrace_depth_max`
   * `function_regex` - a regexp which needs to match with one of the functions in the backtrace of the matched allocation
   * `source_regex` - a regexp which needs to match with one of the source files in the backtrace of the matched allocation
   * `negative_function_regex` - a regexp which needs to NOT match with all of the functions in the backtrace of the matched allocation
   * `negative_source_regex` - a regexp which needs to NOT match with all of the source files in the backtrace of the matched allocation
   * `group_interval_min`, `group_interval_max` - a minimum/maximum interval in seconds or a percentage (of total runtime)
                                                  between the first and the last allocation from the same call site
   * `group_allocations_min`, `group_allocations_max` - an integer with a minimum/maximum number of allocations
                                                        from the same call site
   * `group_leaked_allocations_min`, `group_leaked_allocations_max` - an integer or a percentage of all allocations
                                                                      which were leaked from the same call site

The `<sort_by>` for allocations can be one of:

   * `timestamp`
   * `address`
   * `size`

The `<group_sort_by>` for allocation groups can be one of:

   * `only_matched.min_timestamp`
   * `only_matched.max_timestamp`
   * `only_matched.interval`
   * `only_matched.allocated_count`
   * `only_matched.leaked_count`
   * `only_matched.size`
   * `all.min_timestamp`
   * `all.max_timestamp`
   * `all.interval`
   * `all.allocated_count`
   * `all.leaked_count`
   * `all.size`

The `only_matched.*` variants will sort by aggregate values derived only from allocations
which were matched by the `allocation_filter`, while the `all.*` variants will sort
by values derived from every allocation in a given group.

The `<order>` specifies the ordering of the results and can be either `asc` or `dsc`.

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

### `MEMORY_PROFILER_DISABLE_BY_DEFAULT`

Default: `0`

When set to `1` the tracing will be disabled be default at startup.

### `MEMORY_PROFILER_REGISTER_SIGUSR1`

Default: `1`

When set to `1` the profiler will register a `SIGUSR1` signal handler
which can be used to toggle (enable or disable) profiling.

If disabled and reenabled a new data file will be created according
to the pattern set in `MEMORY_PROFILER_OUTPUT`.

### `MEMORY_PROFILER_REGISTER_SIGUSR2`

Default: `1`

When set to `1` the profiler will register a `SIGUSR2` signal handler
which can be used to toggle (enable or disable) profiling.

### `MEMORY_PROFILER_ENABLE_SERVER`

Default: `0`

When set to `1` the profiled process will start an embedded server which can
be used to stream the profiling data through TCP using `memory-profiler-cli gather` and `memory-profiler-gather`.

This server will only be started when the profiling is enabled.

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
