# Common issues

## The profiler generates too much data, and I don't have enough disk space to hold it!

By default the profiler will gather every allocation that's made by your application.
If you don't need all of that data then you can set the [`MEMORY_PROFILER_CULL_TEMPORARY_ALLOCATIONS`](configuration.md#memory_profiler_cull_temporary_allocations)
environment variable to `1` before you start profiling. This will prevent the profiler from emitting
the majority of short-lived allocations which should cut down on how big the resulting file will be.

You can also adjust the [`MEMORY_PROFILER_TEMPORARY_ALLOCATION_LIFETIME_THRESHOLD`](configuration.md#memory_profiler_temporary_allocation_lifetime_threshold)
option to specify which allocations will be considered temporary by the profiler.

## The profiler crashes and/or is killed when I try to load my data file!

You're most likely trying to load a really big file, and you're running out of RAM.

You can use the `strip` subcommand to strip away unnecessary allocations making
it possible to load your data file for analysis even if you don't have enough RAM.

For example, here's how you'd strip away all of the allocations with a lifetime of less than 60 seconds:

```
$ ./memory-profiler-cli strip --threshold 60 -o stripped.dat original.dat
```

After running this command the `stripped.dat` will only contain allocations which
lived for at least 60 seconds or more.
