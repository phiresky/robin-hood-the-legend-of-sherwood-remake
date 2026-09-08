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

binaryen_url='https://github.com/WebAssembly/binaryen/releases/download/version_132/binaryen-version_132-x86_64-linux.tar.gz'
binaryen_sha256='195ddc94f9bc89f45abdabb0b9eea86023d727ba90eac8b35b80f2544fc30572'
binaryen_length='117057408'
wabt_url='https://github.com/WebAssembly/wabt/releases/download/1.0.41/wabt-1.0.41-linux-x64.tar.gz'
wabt_sha256='83f8122e924745fcd70636e3594bc01c4c47f2d4c8f3c63b5d70d3f83a482677'
wabt_length='5076269'

mkdir -p "$destination/downloads" "$destination/binaryen" "$destination/wabt"

download_and_verify() {
    local url="$1"
    local expected_sha256="$2"
    local expected_length="$3"
    local output="$4"
    curl \
        --proto '=https' \
        --tlsv1.2 \
        --fail \
        --show-error \
        --silent \
        --location \
        --retry 5 \
        --retry-all-errors \
        --output "$output" \
        "$url"
    printf '%s  %s\n' "$expected_sha256" "$output" | sha256sum --check --strict -
    actual_length="$(wc -c < "$output")"
    if [[ "$actual_length" != "$expected_length" ]]; then
        echo "unexpected byte length for $url: $actual_length" >&2
        exit 1
    fi
}

download_and_verify \
    "$binaryen_url" \
    "$binaryen_sha256" \
    "$binaryen_length" \
    "$destination/downloads/binaryen.tar.gz"
download_and_verify \
    "$wabt_url" \
    "$wabt_sha256" \
    "$wabt_length" \
    "$destination/downloads/wabt.tar.gz"

tar --extract --gzip --file "$destination/downloads/binaryen.tar.gz" \
    --directory "$destination/binaryen" --strip-components=1 \
    --no-same-owner --no-same-permissions
tar --extract --gzip --file "$destination/downloads/wabt.tar.gz" \
    --directory "$destination/wabt" --strip-components=1 \
    --no-same-owner --no-same-permissions

wasm_opt="$destination/binaryen/bin/wasm-opt"
wasm_strip="$destination/wabt/bin/wasm-strip"
for executable in "$wasm_opt" "$wasm_strip"; do
    if [[ ! -f "$executable" || -L "$executable" || ! -x "$executable" ]]; then
        echo "pinned distribution did not contain an exact regular executable: $executable" >&2
        exit 1
    fi
done

if [[ "$($wasm_opt --version)" != 'wasm-opt version 132 (version_132)' ]]; then
    echo "pinned wasm-opt reported an unexpected version" >&2
    exit 1
fi
if [[ "$($wasm_strip --version)" != '1.0.41' ]]; then
    echo "pinned wasm-strip reported an unexpected version" >&2
    exit 1
fi

printf '%s\n' "$destination/binaryen/bin" "$destination/wabt/bin"
