#!/bin/bash

set -euo pipefail
IFS=$'\n\t'

export RUST_BACKTRACE=1

set +e
echo "$(rustc --version)" | grep -q "nightly"
if [ "$?" = "0" ]; then
    export IS_NIGHTLY=1
else
    export IS_NIGHTLY=0
fi
set -e

cargo test -p common
if [ "$IS_NIGHTLY" = "1" ]; then
    cargo test -p memory-profiler
fi
cargo test -p cli-core
cargo test -p server-core

if [ "$IS_NIGHTLY" = "1" ]; then
    ./ci/build_for_deployment.sh
    cargo test -p integration-tests
fi
