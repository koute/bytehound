#!/bin/bash

set -euo pipefail
cd "$(dirname $(readlink -f "$0"))/.."

source ./ci/check_if_nightly.sh

cd preload
cargo build --release --target=x86_64-unknown-linux-gnu $FEATURES_NIGHTLY
cd ..

cd cli
cargo build --release --target=x86_64-unknown-linux-gnu
cd ..

cd gather
cargo build --release --target=x86_64-unknown-linux-gnu
cd ..
