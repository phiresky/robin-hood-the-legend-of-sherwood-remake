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

distribution_url='https://static.crates.io/crates/wasm-bindgen-cli/wasm-bindgen-cli-0.2.127.crate'
distribution_sha256='6123f525ba36df42e57b67027637a78591e712a9f6a025ffd3d74298fa1c3f4c'
distribution_length='52654'
packaged_lock_sha256='420a0f944d2521032fbdc6a46d1519b7943ffe8254d019dc37ab8865428fceb9'
packaged_lock_length='59375'

mkdir -p "$destination/download" "$destination/source"
archive="$destination/download/wasm-bindgen-cli-0.2.127.crate"
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
printf '%s  %s\n' "$distribution_sha256" "$archive" | sha256sum --check --strict -
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
printf '%s  %s\n' "$packaged_lock_sha256" "$packaged_lock" | sha256sum --check --strict -
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
if [[ "$($executable --version)" != 'wasm-bindgen 0.2.127' ]]; then
    echo "pinned wasm-bindgen reported an unexpected version" >&2
    exit 1
fi

printf '%s\n' "$executable"
