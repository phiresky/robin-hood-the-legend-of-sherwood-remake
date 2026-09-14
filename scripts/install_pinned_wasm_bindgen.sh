#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "usage: $0 ABSENT_ABSOLUTE_DESTINATION" >&2
    exit 2
fi

destination="$1"
if [[ "$destination" != /* || -e "$destination" ]]; then
    echo "destination must be an absent absolute path: $destination" >&2
    exit 2
fi

distribution_url='https://static.crates.io/crates/wasm-bindgen-cli/wasm-bindgen-cli-0.2.128.crate'
distribution_sha256='2e29140c04e81832902b70e5d37b9ce1bfa811c8fe4563bea08f76df8388cf20'
distribution_length='53205'
packaged_lock_sha256='bd306494454acd7c409b37950530527495c0c9cffdbf21e449ffa4bd4454fc7a'
packaged_lock_length='59779'

mkdir -p "$destination/download" "$destination/source"
archive="$destination/download/wasm-bindgen-cli-0.2.128.crate"
curl \
    --proto '=https' \
    --tlsv1.2 \
    --fail \
    --show-error \
    --silent \
    --location \
    --retry 5 \
    --retry-all-errors \
    --output "$archive" \
    "$distribution_url"
printf '%s  %s\n' "$distribution_sha256" "$archive" | sha256sum --check --strict - >&2
actual_length="$(wc -c < "$archive")"
if [[ "$actual_length" != "$distribution_length" ]]; then
    echo "unexpected wasm-bindgen-cli distribution byte length: $actual_length" >&2
    exit 1
fi

tar --extract --gzip --file "$archive" \
    --directory "$destination/source" --strip-components=1 \
    --no-same-owner --no-same-permissions

packaged_lock="$destination/source/Cargo.lock"
if [[ ! -f "$packaged_lock" || -L "$packaged_lock" ]]; then
    echo "pinned wasm-bindgen-cli distribution has no regular Cargo.lock" >&2
    exit 1
fi
printf '%s  %s\n' "$packaged_lock_sha256" "$packaged_lock" | sha256sum --check --strict - >&2
actual_lock_length="$(wc -c < "$packaged_lock")"
if [[ "$actual_lock_length" != "$packaged_lock_length" ]]; then
    echo "unexpected packaged Cargo.lock byte length: $actual_lock_length" >&2
    exit 1
fi

cargo install \
    --locked \
    --path "$destination/source" \
    --root "$destination"

executable="$destination/bin/wasm-bindgen"
if [[ ! -f "$executable" || -L "$executable" || ! -x "$executable" ]]; then
    echo "pinned source build did not produce a regular wasm-bindgen executable" >&2
    exit 1
fi
if [[ "$($executable --version)" != 'wasm-bindgen 0.2.128' ]]; then
    echo "pinned wasm-bindgen reported an unexpected version" >&2
    exit 1
fi

printf '%s\n' "$executable"
