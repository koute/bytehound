#!/bin/bash

set -euo pipefail
cd "$(dirname $(readlink -f "$0"))"

echo "Building the simulation..."
cargo build
cd ..

echo "Building the profiler..."
cargo build -p bytehound-preload

echo "Building the CLI..."
cargo build -p bytehound-cli

echo "Profiling the simulation..."
export MEMORY_PROFILER_OUTPUT=simulation/memory-profiling-simulation-raw.dat
LD_PRELOAD=target/debug/libbytehound.so simulation/target/debug/simulation

target/debug/bytehound postprocess --anonymize=partial -o simulation/memory-profiling-simulation.dat simulation/memory-profiling-simulation-raw.dat
rm -f simulation/memory-profiling-simulation-raw.dat

echo "Profiling data generated: memory-profiling-simulation.dat"
