# Getting started

## Download prebuilt binaries

You can download a precompiled binary release of the profiler from [here](https://github.com/koute/bytehound/releases).

## Build from source

Alternatively you can build everything from sources yourself.

Make sure you have the following installed:

1. Rust nightly
2. Full GCC toolchain
3. [Yarn](https://yarnpkg.com) package manager

Then you can build the profiler:

```
$ cargo build --release -p bytehound-preload
$ cargo build --release -p bytehound-cli
```

...and grab the binaries from from `target/release/libbytehound.so` and `target/release/bytehound`.

## Gathering data

You can gather the profiling data by attaching the profiler to your application using `LD_PRELOAD`.
Just put the `libbytehound.so` in the same directory as your program and then run the following:

```
$ export MEMORY_PROFILER_LOG=info
$ LD_PRELOAD=./libbytehound.so ./your_application
```

You can further configure the profiler [through environment variables](./configuration.md),
although often that is not be necessary.

## Analysis

After you've gathered your data you can load it for analysis:

```
$ ./bytehound server memory-profiling_*.dat
```

Then open your web browser and point it at `http://localhost:8080` to access the GUI.

If the profiler crashes when loading the data you most likely don't have
enough RAM to load the whole thing into memory; see the [common issues](./common_issues.md)
section for how to handle such situation.
