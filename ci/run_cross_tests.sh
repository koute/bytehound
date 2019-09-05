#!/bin/bash

set -euo pipefail
cd "$(dirname $(readlink -f "$0"))/.."

set +e
echo "$(rustc --version)" | grep -q "nightly"
if [ "$?" = "0" ]; then
    echo "Running on nightly!"
    EXTRA_ARGS="--features nightly"
else
    EXTRA_ARGS=""
fi
set -e

export MEMORY_PROFILER_TEST_TARGET=$1
export MEMORY_PROFILER_TEST_RUNNER=/usr/local/bin/runner
export CARGO_TARGET_DIR="target/cross"

cargo build --target=$MEMORY_PROFILER_TEST_TARGET -p memory-profiler $EXTRA_ARGS
MEMORY_PROFILER_TEST_PRELOAD_PATH=$MEMORY_PROFILER_TEST_TARGET/debug cargo test -p integration-tests

cargo build --target=$MEMORY_PROFILER_TEST_TARGET --release -p memory-profiler
MEMORY_PROFILER_TEST_PRELOAD_PATH=$MEMORY_PROFILER_TEST_TARGET/release cargo test -p integration-tests
