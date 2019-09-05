#!/bin/sh

set -euo pipefail
exec "$(dirname $(readlink -f "$0"))/run.sh" --interactive "$@"
