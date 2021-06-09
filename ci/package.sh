#!/bin/bash

set -euo pipefail
cd "$(dirname $(readlink -f "$0"))/.."

CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$(dirname $(readlink -f "$0"))/../target}

echo "Packaging for deployment..."

rm -Rf $CARGO_TARGET_DIR/travis-deployment $CARGO_TARGET_DIR/travis-deployment-tmp
mkdir -p $CARGO_TARGET_DIR/travis-deployment $CARGO_TARGET_DIR/travis-deployment-tmp

cp $CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/libmemory_profiler.so $CARGO_TARGET_DIR/travis-deployment-tmp/
cp $CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/memory-profiler-cli $CARGO_TARGET_DIR/travis-deployment-tmp/
cp $CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/memory-profiler-gather $CARGO_TARGET_DIR/travis-deployment-tmp/

cd $CARGO_TARGET_DIR/travis-deployment-tmp
tar -zcf ../travis-deployment/memory-profiler-x86_64-unknown-linux-gnu.tgz \
    libmemory_profiler.so \
    memory-profiler-cli \
    memory-profiler-gather

echo "Deployment package built!"
