#!/bin/bash

set -euo pipefail

CWD="$(pwd)"
cd "$(dirname $(readlink -f "$0"))"

TARGET_LIST=$(rustup target list | grep "(installed)" | cut -d " " -f 1)

docker build \
    -f Dockerfile.hybrid \
    -t crossenv \
    --build-arg TARGET_LIST="$TARGET_LIST" \
    ./

EXTRA_ARGS=""

set +u
if [ "$1" == "--interactive" ]; then
    EXTRA_ARGS="$EXTRA_ARGS --entrypoint /bin/bash"
    shift
fi
set -u

docker run \
    --rm \
    --tty \
    --interactive \
    $EXTRA_ARGS \
    -v ~/.rustup:/home/user/.rustup \
    -v ~/.cargo/bin:/home/user/.cargo/bin \
    -v ~/.cargo/git:/home/user/.cargo/git \
    -v ~/.cargo/registry:/home/user/.cargo/registry \
    -v "$CWD:/home/user/cwd" \
    -w /home/user/cwd \
    crossenv \
    "$@"
