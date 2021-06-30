#!/usr/bin/env sh

set -ex

: "${TARGET?The TARGET environment variable must be set.}"

echo "Running tests for target: ${TARGET}, Rust version=${TRAVIS_RUST_VERSION}"
export RUST_BACKTRACE=1
export RUST_TEST_THREADS=1
export RUST_TEST_NOCAPTURE=1
export CARGO_CMD=cross

# FIXME: workaround cargo breaking Travis-CI again:
# https://github.com/rust-lang/cargo/issues/5721
if [ "$TRAVIS" = "true" ]
then
    export TERM=dumb
fi

# Runs jemalloc tests when building jemalloc-sys (runs "make check"):
if [ "${NO_JEMALLOC_TESTS}" = "1" ]
then
    echo "jemalloc's tests are not run"
else
    export JEMALLOC_SYS_RUN_JEMALLOC_TESTS=1
fi

# Use cargo on native CI platforms:
case "${TARGET}" in
    "x86_64-unknown-linux-gnu") export CARGO_CMD=cargo ;;
    *"windows"*) export CARGO_CMD=cargo ;;
    *"apple"*) export CARGO_CMD=cargo ;;
esac

if [ "${CARGO_CMD}" = "cross" ]
then
    cargo install cross || echo "cross is already installed"
fi

if [ "${VALGRIND}" = "1" ]
then
    case "${TARGET}" in
        "x86_64-unknown-linux-gnu")
            export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER=valgrind
            ;;
        "x86_64-apple-darwin")
            export CARGO_TARGET_X86_64_APPLE_DARWIN_RUNNER=valgrind
            ;;
        *)
            echo "Specify how to run valgrind for TARGET=${TARGET}"
            exit 1
            ;;
    esac
fi

if [ "${TARGET}" = "x86_64-unknown-linux-gnu" ] || [ "${TARGET}" = "x86_64-apple-darwin" ]
then
    ${CARGO_CMD} build -vv --target "${TARGET}" 2>&1 | tee build_no_std.txt

    # Check that the no-std builds are not linked against a libc with default
    # features or the `use_std` feature enabled:
    ! grep -q "default" build_no_std.txt
    ! grep -q "use_std" build_no_std.txt

    RUST_SYS_ROOT=$(rustc --target="${TARGET}" --print sysroot)
    RUST_LLVM_NM="${RUST_SYS_ROOT}/lib/rustlib/${TARGET}/bin/llvm-nm"

    find target/ -iname '*jemalloc*.rlib' | while read -r rlib; do
        echo "${RUST_LLVM_NM} ${rlib}"
        ! $RUST_LLVM_NM "${rlib}" | grep "std"
    done
fi

${CARGO_CMD} test -vv --target "${TARGET}"

if [ "${JEMALLOC_SYS_GIT_DEV_BRANCH}" = "1" ]; then
    # FIXME: profiling tests broken on dev-branch
    # https://github.com/jemalloc/jemalloc/issues/1477
    :
else
    ${CARGO_CMD} test -vv --target "${TARGET}" --features profiling
fi

${CARGO_CMD} test -vv --target "${TARGET}" --features debug
${CARGO_CMD} test -vv --target "${TARGET}" --features stats
if [ "${JEMALLOC_SYS_GIT_DEV_BRANCH}" = "1" ]; then
    # FIXME: profiling tests broken on dev-branch
    # https://github.com/jemalloc/jemalloc/issues/1477
    :
else
    ${CARGO_CMD} test -vv --target "${TARGET}" --features 'debug profiling'
fi

${CARGO_CMD} test -vv --target "${TARGET}" \
             --features unprefixed_malloc_on_supported_platforms
${CARGO_CMD} test -vv --target "${TARGET}" --no-default-features
${CARGO_CMD} test -vv --target "${TARGET}" --no-default-features \
             --features background_threads_runtime_support

if [ "${NOBGT}" = "1" ]
then
    echo "enabling background threads by default at run-time is not tested"
else
    ${CARGO_CMD} test -vv --target "${TARGET}" --features background_threads
fi

${CARGO_CMD} test -vv --target "${TARGET}" --release
${CARGO_CMD} test -vv --target "${TARGET}" --manifest-path jemalloc-sys/Cargo.toml
${CARGO_CMD} test -vv --target "${TARGET}" \
             --manifest-path jemalloc-sys/Cargo.toml \
             --features unprefixed_malloc_on_supported_platforms

# FIXME: jemalloc-ctl fails in the following targets
case "${TARGET}" in
    "i686-unknown-linux-musl") ;;
    "x86_64-unknown-linux-musl") ;;
    *)

        ${CARGO_CMD} test -vv --target "${TARGET}" \
                     --manifest-path jemalloc-ctl/Cargo.toml \
                     --no-default-features
        # FIXME: cross fails to pass features to jemalloc-ctl
        # ${CARGO_CMD} test -vv --target "${TARGET}" \
        #             --manifest-path jemalloc-ctl \
        #             --no-default-features --features use_std
        ;;
esac

${CARGO_CMD} test -vv --target "${TARGET}" -p systest
${CARGO_CMD} test -vv --target "${TARGET}" \
             --manifest-path jemallocator-global/Cargo.toml
${CARGO_CMD} test -vv --target "${TARGET}" \
             --manifest-path jemallocator-global/Cargo.toml \
             --features force_global_jemalloc

if [ "${TRAVIS_RUST_VERSION}" = "nightly"  ]
then
    # The Alloc trait is unstable:
    ${CARGO_CMD} test -vv --target "${TARGET}" --features alloc_trait
fi
