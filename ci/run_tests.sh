#!/bin/bash

set -euo pipefail
IFS=$'\n\t'

TEST_SUBSET=${TEST_SUBSET:-0}

export RUST_BACKTRACE=1

if [[ "$TEST_SUBSET" == 0 || "$TEST_SUBSET" == 1 ]]; then
    cargo test -p common
    cargo test -p memory-profiler
    cargo test -p cli-core
    cargo test -p server-core
fi

if [[ "$TEST_SUBSET" == 0 || "$TEST_SUBSET" == 2 ]]; then
    ./ci/build_for_deployment.sh
    cargo test -p integration-tests
    cargo build -p memory-profiler
    MEMORY_PROFILER_TEST_PRELOAD_PATH=debug cargo test -p integration-tests
fi
