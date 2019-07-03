#!/bin/bash

set -euo pipefail

set +e
echo "$(rustc --version)" | grep -q "nightly"
if [ "$?" = "0" ]; then
    export IS_NIGHTLY=1
else
    export IS_NIGHTLY=0
fi
set -e

if [ "$IS_NIGHTLY" = "1" ]; then
    cargo build --release --target=x86_64-unknown-linux-gnu -p memory-profiler --features nightly
else
    cargo build --release --target=x86_64-unknown-linux-gnu -p memory-profiler
fi

cargo build --release --target=x86_64-unknown-linux-gnu -p memory-profiler-cli
cargo build --release --target=x86_64-unknown-linux-gnu -p memory-profiler-gather

echo "Building artifacts for deployment..."

rm -Rf target/travis-deployment target/travis-deployment-tmp
mkdir -p target/travis-deployment target/travis-deployment-tmp

cp target/x86_64-unknown-linux-gnu/release/libmemory_profiler.so target/travis-deployment-tmp/
cp target/x86_64-unknown-linux-gnu/release/memory-profiler-cli target/travis-deployment-tmp/
cp target/x86_64-unknown-linux-gnu/release/memory-profiler-gather target/travis-deployment-tmp/

echo "Packing..."

cd target/travis-deployment-tmp
tar -zcf ../travis-deployment/memory-profiler-x86_64-unknown-linux-gnu.tgz \
    libmemory_profiler.so \
    memory-profiler-cli \
    memory-profiler-gather

echo "Deployment package built!"
