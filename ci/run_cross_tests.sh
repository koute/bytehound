#!/bin/bash

set -euo pipefail
cd "$(dirname $(readlink -f "$0"))/.."

source ./ci/check_if_nightly.sh

export MEMORY_PROFILER_TEST_TARGET=$1
export MEMORY_PROFILER_TEST_RUNNER=/usr/local/bin/runner

cd preload
cargo build --target=$MEMORY_PROFILER_TEST_TARGET $FEATURES_NIGHTLY
cd ..

cd integration-tests
MEMORY_PROFILER_TEST_PRELOAD_PATH=$MEMORY_PROFILER_TEST_TARGET/debug cargo test --no-default-features
cd ..

cd preload
cargo build --target=$MEMORY_PROFILER_TEST_TARGET $FEATURES_NIGHTLY --release
cd ..

cd integration-tests
MEMORY_PROFILER_TEST_PRELOAD_PATH=$MEMORY_PROFILER_TEST_TARGET/release cargo test --no-default-features
cd ..
