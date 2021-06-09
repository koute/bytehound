#!/bin/bash

set -euo pipefail

cd "$(dirname $(readlink -f "$0"))"
CWD="$(pwd)/../.."

EXTRA_ARGS=""
IMAGE_TAG=crossenv

IS_INTERACTIVE=0
TARGET_LIST="aarch64-unknown-linux-gnu armv7-unknown-linux-gnueabihf mips64-unknown-linux-gnuabi64"
USE_HOST_RUSTC=0
CARGO_TARGET_DIR="/home/user/cwd/target-docker"

set +u
while true
do
    if [ "$1" == "--interactive" ]; then
        EXTRA_ARGS="$EXTRA_ARGS --entrypoint /bin/bash -v /:/mnt/host"
        IS_INTERACTIVE=1
        IMAGE_TAG="$IMAGE_TAG-interactive"
        shift
    elif [ "$1" == "--use-host-rustc" ]; then
        TARGET_LIST=$(rustup target list | grep "(installed)" | cut -d " " -f 1)
        USE_HOST_RUSTC=1
        CARGO_TARGET_DIR="/home/user/cwd/target"
        IMAGE_TAG="$IMAGE_TAG-host-rustc"
        shift
    else
        break
    fi
done
set -u

docker build \
    -f Dockerfile.hybrid \
    -t $IMAGE_TAG \
    --build-arg TARGET_LIST="$TARGET_LIST" \
    --build-arg UID="$(id -u)" \
    --build-arg GID="$(id -g)" \
    --build-arg IS_INTERACTIVE="$IS_INTERACTIVE" \
    --build-arg CARGO_TARGET_DIR="$CARGO_TARGET_DIR" \
    --build-arg USE_HOST_RUSTC="$USE_HOST_RUSTC" \
    ./

if [ "$USE_HOST_RUSTC" == "1" ]; then
    EXTRA_ARGS="$EXTRA_ARGS -v $HOME/.rustup:/home/user/.rustup -v $HOME/.cargo/bin:/home/user/.cargo/bin -v $HOME/.cargo/git:/home/user/.cargo/git -v $HOME/.cargo/registry:/home/user/.cargo/registry"
fi

docker run \
    --rm \
    --tty \
    --interactive \
    $EXTRA_ARGS \
    -v "$CWD:/home/user/cwd" \
    -w /home/user/cwd \
    $IMAGE_TAG \
    "$@"
