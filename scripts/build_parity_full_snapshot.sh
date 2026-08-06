#!/usr/bin/env bash
set -euo pipefail

if (( $# != 1 )); then
    printf 'usage: %s OUTPUT_SNAPSHOT\n' "$0" >&2
    exit 2
fi

output=$1
retired_file=docs/PARITY_RETIRED_SAVES.txt
old_corpus=parity-random-save-replays-60s-15x
corpora=(
    parity-random-save-replays-60s-15x-schema14
    parity-save-replays-schema12
    parity-random-save-replays
)

if [[ ! -f "$retired_file" ]]; then
    printf 'error: retired-save manifest does not exist: %s\n' "$retired_file" >&2
    exit 2
fi
for corpus in "$old_corpus" "${corpora[@]}"; do
    if [[ ! -d "$corpus/traces" ]]; then
        printf 'error: trace corpus does not exist: %s/traces\n' "$corpus" >&2
        exit 2
    fi
done

declare -A retired_saves=()
while IFS= read -r save; do
    [[ -n "$save" ]] && retired_saves["$save"]=1
done < "$retired_file"

output_dir=$(dirname "$output")
mkdir -p "$output_dir"
snapshot_tmp=$(mktemp "$output.tmp.XXXXXX")
trap 'rm -f "$snapshot_tmp"' EXIT

while IFS= read -r -d '' trace; do
    relative=${trace#"$old_corpus/traces/"}
    save=${relative%/replay-*}
    [[ -v "retired_saves[$save]" ]] || printf '%s\n' "$trace"
done < <(find "$old_corpus/traces" -type f -name '*.jsonl.zst' -print0 | sort -z) > "$snapshot_tmp"

for corpus in "${corpora[@]}"; do
    find "$corpus/traces" -type f -name '*.jsonl.zst' -print
done | sort >> "$snapshot_tmp"

mv "$snapshot_tmp" "$output"
trap - EXIT
printf 'wrote %s traces to %s\n' "$(wc -l < "$output")" "$output"
