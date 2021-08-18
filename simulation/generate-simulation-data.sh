#!/bin/bash

set -euo pipefail
cd "$(dirname $(readlink -f "$0"))"

echo "Building the simulation..."
cargo build
cd ..

echo "Building the profiler..."
cargo build -p memory-profiler

echo "Profiling the simulation..."
export MEMORY_PROFILER_OUTPUT=simulation/memory-profiling-simulation.dat
LD_PRELOAD=target/debug/libmemory_profiler.so simulation/target/debug/simulation

echo "Profiling data generated: memory-profiling-simulation.dat"
