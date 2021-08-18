#!/bin/bash

set -euo pipefail
cd "$(dirname $(readlink -f "$0"))"

echo "Building the simulation..."
cargo build
cd ..

echo "Building the profiler..."
cargo build -p bytehound-preload

echo "Profiling the simulation..."
export MEMORY_PROFILER_OUTPUT=simulation/memory-profiling-simulation.dat
LD_PRELOAD=target/debug/libbytehound.so simulation/target/debug/simulation

echo "Profiling data generated: memory-profiling-simulation.dat"
