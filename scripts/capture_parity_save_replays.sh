#!/usr/bin/env bash
set -euo pipefail

# Load every v48 RHSG/GSHR save through -PARITYSAVE and record 10 seconds
# (250 simulation frames at 25 Hz) without input.
#
# Usage:
#   scripts/capture_parity_save_replays.sh [save-dir] [output-dir] [data-dir]
#
# The default output is ./parity-save-replays in the invocation directory.
#
# Environment overrides:
#   PARITY_FRAMES=250  PARITY_SEED=1  WATCHDOG_SECONDS=60
#   ROBIN_BINARY=original-code/build/native-full/robin
#   SKIP_BUILD=1      FORCE=1
#   SHARD_COUNT=1     SHARD_INDEX=0

invocation_dir="$PWD"
repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd -- "$repo_dir"

save_dir="${1:-${PARITY_SAVE_DIR:-$repo_dir/reference-saves}}"
output_dir="${2:-$invocation_dir/parity-save-replays}"
data_dir="${3:-${ROBINHOOD_DATA_DIR:-$repo_dir/datadirs/fullgame_linux}}"
if [[ "$save_dir" != /* ]]; then
    save_dir="$invocation_dir/$save_dir"
fi
if [[ "$output_dir" != /* ]]; then
    output_dir="$invocation_dir/$output_dir"
fi
if [[ "$data_dir" != /* ]]; then
    data_dir="$invocation_dir/$data_dir"
fi
save_dir="${save_dir%/}"
output_dir="${output_dir%/}"
data_dir="${data_dir%/}"
frames="${PARITY_FRAMES:-250}"
seed="${PARITY_SEED:-1}"
watchdog_seconds="${WATCHDOG_SECONDS:-60}"
binary="${ROBIN_BINARY:-original-code/build/native-full/robin}"
shard_count="${SHARD_COUNT:-1}"
shard_index="${SHARD_INDEX:-0}"

if [[ ! -d "$save_dir" ]]; then
    printf 'error: save directory does not exist: %s\n' "$save_dir" >&2
    exit 2
fi
if [[ ! -d "$data_dir" ]]; then
    printf 'error: game data directory does not exist: %s\n' "$data_dir" >&2
    exit 2
fi
if [[ ! "$frames" =~ ^[1-9][0-9]*$ ]]; then
    printf 'error: PARITY_FRAMES must be a positive integer\n' >&2
    exit 2
fi
if [[ ! "$shard_count" =~ ^[1-9][0-9]*$
    || ! "$shard_index" =~ ^[0-9]+$
    || "$shard_index" -ge "$shard_count" ]]
then
    printf 'error: require SHARD_COUNT > 0 and 0 <= SHARD_INDEX < SHARD_COUNT\n' >&2
    exit 2
fi

if [[ "${SKIP_BUILD:-0}" != 1 ]]; then
    (
        cd original-code
        mkdir -p build/native-full/compiler-tmp
        TMPDIR="$PWD/build/native-full/compiler-tmp" cmake --build --preset native-full
    )
fi
if [[ ! -x "$binary" ]]; then
    printf 'error: original-game binary is not executable: %s\n' "$binary" >&2
    exit 2
fi

mkdir -p -- "$output_dir/traces" "$output_dir/logs"

captured=0
failed=0
skipped=0
save_index=0

while IFS= read -r -d '' save_file; do
    magic_hex="$(od -An -tx1 -N4 -- "$save_file" 2>/dev/null | tr -d ' \n' || true)"
    if [[ "$magic_hex" != 52485347 && "$magic_hex" != 47534852 ]]; then
        continue
    fi

    header_words="$(od -An -tu4 -j4 -N12 -- "$save_file" 2>/dev/null || true)"
    if ! read -r header_version _mission_id stream_version <<<"$header_words"; then
        printf 'warning: skipping truncated save header: %s\n' "$save_file" >&2
        skipped=$((skipped + 1))
        continue
    fi
    if [[ "${header_version:-}" != 48 || "${stream_version:-}" != 48 ]]; then
        printf 'warning: skipping non-v48 save: %s\n' "$save_file" >&2
        skipped=$((skipped + 1))
        continue
    fi

    current_index="$save_index"
    save_index=$((save_index + 1))
    if (( current_index % shard_count != shard_index )); then
        continue
    fi

    relative_path="${save_file#"$save_dir"/}"
    trace_base="$output_dir/traces/$relative_path.jsonl"
    trace_stem="${trace_base%.jsonl}"
    complete_marker="$trace_stem.complete"
    log_path="$output_dir/logs/$relative_path.log"
    mkdir -p -- "$(dirname -- "$trace_base")" "$(dirname -- "$log_path")"

    if [[ "${FORCE:-0}" != 1 && -e "$complete_marker" ]]; then
        printf 'skip     %s (capture already completed)\n' "$relative_path"
        skipped=$((skipped + 1))
        continue
    fi

    printf 'capture  %s\n' "$relative_path"
    if timeout --signal=TERM --kill-after=10s "${watchdog_seconds}s" \
        env \
            ROBINHOOD_DATA_DIR="$data_dir" \
            SDL_VIDEODRIVER=dummy \
            SDL_AUDIODRIVER=dummy \
            "$binary" \
            -PARITYSAVE "$save_file" \
            -PARITYTRACE "$trace_base" \
            -PARITYSEED "$seed" \
            -PARITYFRAMES "$frames" \
            >"$log_path" 2>&1
    then
        : >"$complete_marker"
        captured=$((captured + 1))
    else
        status=$?
        printf 'warning: capture failed with status %d: %s (see %s)\n' \
            "$status" "$relative_path" "$log_path" >&2
        failed=$((failed + 1))
    fi
done < <(find "$save_dir" -type f -print0 | sort -z)

printf 'shard %d/%d done: %d captured, %d failed, %d skipped\n' \
    "$shard_index" "$shard_count" "$captured" "$failed" "$skipped"
if (( failed != 0 )); then
    exit 1
fi
