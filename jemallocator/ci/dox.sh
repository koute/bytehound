#!/usr/bin/env sh

set -ex

export RUSTDOCFLAGS="--cfg jemallocator_docs"
cargo doc --features alloc_trait
cargo doc -p tikv-jemalloc-sys
cargo doc -p tikv-jemalloc-ctl
