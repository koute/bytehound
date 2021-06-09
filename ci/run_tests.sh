#!/bin/bash

set -euo pipefail
cd "$(dirname $(readlink -f "$0"))/.."

source ./ci/check_if_nightly.sh

export RUST_BACKTRACE=1

cd common
cargo test --target=x86_64-unknown-linux-gnu
cd ..

cd preload
cargo test --target=x86_64-unknown-linux-gnu $FEATURES_NIGHTLY
cd ..

cd cli-core
cargo test --target=x86_64-unknown-linux-gnu
cd ..

cd server-core
cargo test --target=x86_64-unknown-linux-gnu
cd ..

./ci/build.sh

ci/run_cross_tests.sh x86_64-unknown-linux-gnu
ci/run_cross_tests.sh aarch64-unknown-linux-gnu
ci/run_cross_tests.sh armv7-unknown-linux-gnueabihf
ci/run_cross_tests.sh mips64-unknown-linux-gnuabi64
