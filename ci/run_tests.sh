#!/bin/bash

set -euo pipefail
cd "$(dirname $(readlink -f "$0"))/.."

TEST_SUBSET=${TEST_SUBSET:-0}
TEST_TARGET=${TEST_TARGET:-}

export RUST_BACKTRACE=1

if [[ "$TEST_SUBSET" == 0 || "$TEST_SUBSET" == 1 ]]; then
    cargo test -p common
    cargo test -p memory-profiler
    cargo test -p cli-core
    cargo test -p server-core
fi

if [[ "$TEST_SUBSET" == 0 || "$TEST_SUBSET" == 2 ]]; then
    ./ci/build_for_deployment.sh
    MEMORY_PROFILER_TEST_PRELOAD_PATH=x86_64-unknown-linux-gnu/release cargo test -p integration-tests

    cargo build -p memory-profiler
    MEMORY_PROFILER_TEST_PRELOAD_PATH=debug cargo test -p integration-tests
fi

if [[ "$TEST_SUBSET" == 0 || "$TEST_SUBSET" == 3 ]]; then
    cargo build --release --target=x86_64-unknown-linux-gnu -p memory-profiler-cli

    if [[ "$TEST_TARGET" == "" || "$TEST_TARGET" == "aarch64-unknown-linux-gnu" ]]; then
        rustup target add aarch64-unknown-linux-gnu
    fi
    if [[ "$TEST_TARGET" == "" || "$TEST_TARGET" == "armv7-unknown-linux-gnueabihf" ]]; then
        rustup target add armv7-unknown-linux-gnueabihf
    fi
    if [[ "$TEST_TARGET" == "" || "$TEST_TARGET" == "mips64-unknown-linux-gnuabi64" ]]; then
        rustup target add mips64-unknown-linux-gnuabi64
    fi

    if [[ "$TEST_TARGET" == "" || "$TEST_TARGET" == "aarch64-unknown-linux-gnu" ]]; then
        ./ci/docker/run.sh ci/run_cross_tests.sh aarch64-unknown-linux-gnu
    fi
    if [[ "$TEST_TARGET" == "" || "$TEST_TARGET" == "armv7-unknown-linux-gnueabihf" ]]; then
        ./ci/docker/run.sh ci/run_cross_tests.sh armv7-unknown-linux-gnueabihf
    fi
    if [[ "$TEST_TARGET" == "" || "$TEST_TARGET" == "mips64-unknown-linux-gnuabi64" ]]; then
        ./ci/docker/run.sh ci/run_cross_tests.sh mips64-unknown-linux-gnuabi64
    fi
fi
