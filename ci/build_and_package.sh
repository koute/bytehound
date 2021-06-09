#!/bin/bash

set -euo pipefail
cd "$(dirname $(readlink -f "$0"))/.."

./ci/build.sh
./ci/package.sh
