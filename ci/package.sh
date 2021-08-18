#!/bin/bash

set -euo pipefail
cd "$(dirname $(readlink -f "$0"))/.."

CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$(dirname $(readlink -f "$0"))/../target}

echo "Packaging for deployment..."

rm -Rf $CARGO_TARGET_DIR/travis-deployment $CARGO_TARGET_DIR/travis-deployment-tmp
mkdir -p $CARGO_TARGET_DIR/travis-deployment $CARGO_TARGET_DIR/travis-deployment-tmp

cp $CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/libbytehound.so $CARGO_TARGET_DIR/travis-deployment-tmp/
cp $CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/bytehound $CARGO_TARGET_DIR/travis-deployment-tmp/
cp $CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/bytehound-gather $CARGO_TARGET_DIR/travis-deployment-tmp/

cd $CARGO_TARGET_DIR/travis-deployment-tmp
tar -zcf ../travis-deployment/bytehound-x86_64-unknown-linux-gnu.tgz \
    libbytehound.so \
    bytehound \
    bytehound-gather

echo "Deployment package built!"
