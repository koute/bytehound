#!/bin/bash

set +e
echo "$(rustc --version)" | grep -q "nightly"
if [ "$?" = "0" ]; then
    FEATURES_NIGHTLY="--features nightly"
else
    FEATURES_NIGHTLY=""
fi
set -e
