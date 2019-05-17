#!/usr/bin/env sh

set -ex

: ${TARGET?"The TARGET environment variable must be set."}

echo "Running tests for target: ${TARGET}, Rust version=${TRAVIS_RUST_VERSION}"
export RUST_BACKTRACE=1
export RUST_TEST_THREADS=1
export RUST_TEST_NOCAPTURE=1
export CARGO_CMD=cross

# Runs jemalloc tests using "make check":
#export JEMALLOC_SYS_RUN_TESTS=1

# Use cargo on native CI platforms:
if [[ ${TARGET} = *"windows"* ]] || \
       [[ ${TARGET} = *"x86_64-unknown-linux-gnu"* ]] || [[ ${TARGET} = *"i686-unknown-linux-gnu"* ]] || [[ ${TARGET} = *"i586-unknown-linux-gnu"* ]] \
       || [[ ${TARGET} = *"apple"* ]]; then
    export CARGO_CMD=cargo
else
    cargo install cross || echo "cross is already installed"
fi

if [[ ${TARGET} = *"x86_64-unknown-linux-gnu"* ]]; then
    export JEMALLOC_SYS_VERIFY_CONFIGURE=1
fi

# Make sure TARGET is installed when using cargo:
if [[ ${CARGO_CMD} == "cargo" ]]; then
    rustup target add ${TARGET} || true
fi

${CARGO_CMD} test -vv --target $TARGET
${CARGO_CMD} test -vv --target $TARGET --features profiling
${CARGO_CMD} test -vv --target $TARGET --features debug
${CARGO_CMD} test -vv --target $TARGET --features stats
${CARGO_CMD} test -vv --target $TARGET --features 'debug profiling'
${CARGO_CMD} test -vv --target $TARGET --features unprefixed_malloc_on_supported_platforms
${CARGO_CMD} test -vv --target $TARGET --release
# FIXME: something broke in the toolchain leading to these
# two commands producing a stack overflow inside of cargo:
# https://github.com/alexcrichton/jemallocator/issues/61
#${CARGO_CMD} test -vv --target $TARGET -p jemalloc-sys
#${CARGO_CMD} test -vv --target $TARGET -p jemalloc-sys --features unprefixed_malloc_on_supported_platforms

if [[ ${TRAVIS_RUST_VERSION} == "nightly"  ]]; then
    # feature(global_allocator) is unstable:
    ${CARGO_CMD} test -vv --target $TARGET -p systest
    # The Alloc trait is unstable:
    ${CARGO_CMD} test -vv --target $TARGET --features alloc_trait
fi
