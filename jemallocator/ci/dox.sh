#!/usr/bin/env sh

set -ex

export RUSTDOCFLAGS="--cfg jemallocator_docs"
cargo doc --features alloc_trait
cargo doc -p jemalloc-sys
cargo doc -p jemalloc-ctl
