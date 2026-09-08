#!/bin/sh
set -eu

if [ "$#" -ne 5 ]; then
    echo "usage: $0 RELEASE_ROOT EXPECTED_SOURCE_COMMIT EXPECTED_SHA256SUMS_SHA256 DEMO_RAW_ROOT FULL_RAW_ROOT" >&2
    exit 64
fi

test_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)

python3 "$test_directory/real-runtime-fence-e2e-selftest.py"
exec python3 "$test_directory/real-runtime-fence-e2e.py" \
    --release-root "$1" \
    --expected-source-commit "$2" \
    --expected-sha256s-sha256 "$3" \
    --demo-raw-root "$4" \
    --full-raw-root "$5"
