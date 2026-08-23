#!/usr/bin/env bash
set -euo pipefail

if (( $# != 1 )); then
    printf 'usage: %s OUTPUT_SNAPSHOT\n' "$0" >&2
    exit 2
fi

output=$1
retired_file=docs/PARITY_RETIRED_SAVES.txt
retired_trace_file=docs/PARITY_RETIRED_TRACES.txt
seed_snapshot=parity-save-replays/seed1000000-final-20260818.snapshot
old_corpus=parity-save-replays/60s-random-input/schema12
corpora=(
    parity-save-replays/60s-random-input/schema14
    parity-save-replays/30s-random-input
    parity-save-replays/15-no-input
)
replacement_corpora=(
    parity-save-replays/60s-random-input/schema15-replacements-20260817
    parity-save-replays/30s-random-input/schema15-replacements-20260817
    parity-save-replays-legacy/10s-no-input-schema15-replacements-20260817
    parity-save-replays/60s-random-input/schema16-replacements-20260818
    parity-save-replays/60s-random-input/schema16-seed1000000-replacements-20260818
)

if [[ ! -f "$retired_file" ]]; then
    printf 'error: retired-save manifest does not exist: %s\n' "$retired_file" >&2
    exit 2
fi
if [[ ! -f "$retired_trace_file" ]]; then
    printf 'error: retired-trace manifest does not exist: %s\n' "$retired_trace_file" >&2
    exit 2
fi
if [[ ! -f "$seed_snapshot" ]]; then
    printf 'error: curated seed-1000000 snapshot does not exist: %s\n' "$seed_snapshot" >&2
    exit 2
fi
for corpus in "$old_corpus" "${corpora[@]}" "${replacement_corpora[@]}"; do
    if [[ ! -d "$corpus/traces" ]]; then
        printf 'error: trace corpus does not exist: %s/traces\n' "$corpus" >&2
        exit 2
    fi
done

declare -A retired_saves=()
while IFS= read -r save; do
    [[ -n "$save" ]] && retired_saves["$save"]=1
done < "$retired_file"

declare -A retired_traces=()
while IFS= read -r trace; do
    [[ -n "$trace" && "$trace" != \#* ]] && retired_traces["$trace"]=1
done < "$retired_trace_file"

output_dir=$(dirname "$output")
mkdir -p "$output_dir"
snapshot_tmp=$(mktemp "$output.tmp.XXXXXX")
trap 'rm -f "$snapshot_tmp"' EXIT

trace_is_complete() {
    local trace=$1
    local marker=${trace%-session-*}.complete
    # Replacement publishers use a checksum-bearing marker next to the
    # complete compressed filename. Older corpora use the replay stem.
    if [[ ! -f "$marker" && ! -f "$trace.complete" ]]; then
        printf 'warning: excluding trace without completion marker: %s\n' "$trace" >&2
        return 1
    fi
}

# Traces are addressed by their logical `.jsonl.zst` identity. A recording
# that was converted to the native format exists on disk only as
# `<identity>.parity.bitcode.zst`, so enumerate both spellings, strip the
# native suffix, and deduplicate.
list_logical_traces() {
    find "$1" -type f \
        \( -name '*.jsonl.zst' -o -name '*.jsonl.zst.parity.bitcode.zst' \) \
        -print0 | sed -z 's/\.parity\.bitcode\.zst$//' | sort -zu
}

while IFS= read -r -d '' trace; do
    relative=${trace#"$old_corpus/traces/"}
    save=${relative%/replay-*}
    if [[ ! -v "retired_saves[$save]" \
        && ! -v "retired_traces[$relative]" \
        && ! -v "retired_traces[$trace]" ]]; then
        trace_is_complete "$trace" || continue
        printf '%s\n' "$trace"
    fi
done < <(list_logical_traces "$old_corpus/traces") > "$snapshot_tmp"

for corpus in "${corpora[@]}"; do
    while IFS= read -r -d '' trace; do
        if [[ ! -v "retired_traces[$trace]" ]]; then
            trace_is_complete "$trace" || continue
            printf '%s\n' "$trace"
        fi
    done < <(list_logical_traces "$corpus/traces")
done | sort >> "$snapshot_tmp"

# Current schema-15/schema-16 recaptures supersede retired historical paths.
# An exact replacement artifact can itself be retired after validation, so the
# fully qualified deny-list still applies here.
for corpus in "${replacement_corpora[@]}"; do
    while IFS= read -r -d '' trace; do
        [[ ! -v "retired_traces[$trace]" ]] || continue
        trace_is_complete "$trace" || continue
        printf '%s\n' "$trace"
    done < <(list_logical_traces "$corpus/traces")
done | sort >> "$snapshot_tmp"

# The seed-1000000 campaign is curated separately because individual stale
# schema-14 recordings can be replaced by schema-16 captures. Include that
# already-audited authoritative set in the all-replay snapshot instead of
# rediscovering the raw seed corpus here.
while IFS= read -r trace; do
    [[ -n "$trace" ]] || continue
    trace_is_complete "$trace" || continue
    printf '%s\n' "$trace"
done < "$seed_snapshot" >> "$snapshot_tmp"

sort -u "$snapshot_tmp" -o "$snapshot_tmp"
mv "$snapshot_tmp" "$output"
trap - EXIT
printf 'wrote %s traces to %s\n' "$(wc -l < "$output")" "$output"
