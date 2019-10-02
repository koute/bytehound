#!/bin/bash

set -euo pipefail

CWD="$(pwd)"
cd "$(dirname $(readlink -f "$0"))"

TARGET_LIST=$(rustup target list | grep "(installed)" | cut -d " " -f 1)
EXTRA_ARGS=""
IS_INTERACTIVE=0
IMAGE_TAG=crossenv

set +u
if [ "$1" == "--interactive" ]; then
    EXTRA_ARGS="$EXTRA_ARGS --entrypoint /bin/bash -v /:/mnt/host"
    IS_INTERACTIVE=1
    IMAGE_TAG=crossenv-interactive
    shift
fi
set -u

docker build \
    -f Dockerfile.hybrid \
    -t $IMAGE_TAG \
    --build-arg TARGET_LIST="$TARGET_LIST" \
    --build-arg UID="$(id -u)" \
    --build-arg GID="$(id -g)" \
    --build-arg IS_INTERACTIVE="$IS_INTERACTIVE" \
    ./

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
    $IMAGE_TAG \
    "$@"
