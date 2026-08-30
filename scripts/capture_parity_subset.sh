#!/usr/bin/env bash
set -euo pipefail

# Re-record a chosen subset of parity replays with an explicit input seed per
# replay, instead of re-deriving seeds from a base like
# original-code/scripts/capture_parity_save_replays.sh does. Corpora that were
# filled in across several shard runs used different PARITY_INPUT_SEED_BASE
# values, so the only way to reproduce an existing trace is to read the seed
# its header recorded and pass it back in.
#
# Usage:
#   scripts/capture_parity_subset.sh <manifest> <output-dir>
#
# The manifest is TSV with one replay per line:
#   <save-path-relative-to-save-dir>\t<replay-index>\t<input-seed>
#
# Environment:
#   SAVE_DIR=./reference-saves      ROBINHOOD_DATA_DIR=./datadirs/fullgame_linux
#   ROBIN_BINARY=./original-code/build/native-full/robin
#   PARITY_CONVERTER=./target/release/original_parity_replay
#   PARITY_FRAMES=1500  PARITY_SEED=1  WATCHDOG_SECONDS=2700
#   CAPTURE_JOBS=10  COMPRESS=1  HEADFUL=0  FORCE=0
#
# COMPRESS=1 (the default) converts each finished JSONL recording directly
# into the native .parity.bitcode.zst artifact via the Rust runner's
# --convert, which audits losslessness and deletes the JSONL. There is no
# interim zstd-compressed JSONL any more; ZSTD_THREADS/ZSTD_LEVEL are
# accepted but ignored for compatibility with existing wrappers.

manifest="${1:?usage: capture_parity_subset.sh <manifest> <output-dir>}"
output_dir="${2:?usage: capture_parity_subset.sh <manifest> <output-dir>}"

save_dir="${SAVE_DIR:-$PWD/reference-saves}"
data_dir="${ROBINHOOD_DATA_DIR:-$PWD/datadirs/fullgame_linux}"
binary="${ROBIN_BINARY:-$PWD/original-code/build/native-full/robin}"
frames="${PARITY_FRAMES:-1500}"
seed="${PARITY_SEED:-1}"
watchdog_seconds="${WATCHDOG_SECONDS:-2700}"
capture_jobs="${CAPTURE_JOBS:-10}"
converter="${PARITY_CONVERTER:-$PWD/target/release/original_parity_replay}"
compress="${COMPRESS:-1}"
headful="${HEADFUL:-0}"

save_dir="${save_dir%/}"
output_dir="${output_dir%/}"
data_dir="${data_dir%/}"

for required in "$manifest" ; do
    if [[ ! -f "$required" ]]; then
        printf 'error: manifest does not exist: %s\n' "$required" >&2
        exit 2
    fi
done
for required_dir in "$save_dir" "$data_dir"; do
    if [[ ! -d "$required_dir" ]]; then
        printf 'error: directory does not exist: %s\n' "$required_dir" >&2
        exit 2
    fi
done
if [[ ! -x "$binary" ]]; then
    printf 'error: original-game binary is not executable: %s\n' "$binary" >&2
    exit 2
fi
if [[ "$compress" == 1 && ! -x "$converter" ]]; then
    printf 'error: COMPRESS=1 requires the parity converter (build with `cargo build -p robin_parity --release --bin original_parity_replay`, or set PARITY_CONVERTER): %s\n' \
        "$converter" >&2
    exit 2
fi

mkdir -p -- "$output_dir/traces" "$output_dir/logs" "$output_dir/incomplete-traces"

captured=0
failed=0
skipped=0

capture_one_replay() {
    local relative_path="$1" replay_index="$2" input_seed="$3"
    local save_file replay_name trace_base log_path trace_stem complete_marker
    local capture_status failure_reason captured_trace failed_archive
    local -a sdl_env captured_traces failed_traces previous_traces

    save_file="$save_dir/$relative_path"
    replay_name="replay-$(printf '%03d' "$replay_index")"
    trace_base="$output_dir/traces/$relative_path/$replay_name.jsonl"
    log_path="$output_dir/logs/$relative_path/$replay_name.log"
    trace_stem="${trace_base%.jsonl}"
    complete_marker="$trace_stem.complete"
    mkdir -p -- "$(dirname -- "$trace_base")" "$(dirname -- "$log_path")"

    if [[ ! -f "$save_file" ]]; then
        printf 'warning: save does not exist: %s\n' "$save_file" >&2
        return 20
    fi
    if [[ "${FORCE:-0}" != 1 && -e "$complete_marker" ]]; then
        printf 'skip     %s/%s (completion marker)\n' "$relative_path" "$replay_name"
        return 10
    fi

    # Keep traces/ holding completed runs only: park anything a previous
    # attempt left behind before starting a new one.
    shopt -s nullglob
    previous_traces=("$trace_stem"-session-*.jsonl "$trace_stem"-session-*.jsonl.zst
        "$trace_stem"-session-*.jsonl.zst.parity.bitcode.zst)
    shopt -u nullglob
    if (( ${#previous_traces[@]} != 0 )) || [[ -e "$complete_marker" ]]; then
        failed_archive="$output_dir/incomplete-traces/$relative_path/$replay_name/attempt-$(date -u +%Y%m%dT%H%M%SZ)-$BASHPID"
        mkdir -p -- "$failed_archive"
        if (( ${#previous_traces[@]} != 0 )); then
            mv -- "${previous_traces[@]}" "$failed_archive/"
        fi
        if [[ -e "$complete_marker" ]]; then
            mv -- "$complete_marker" "$failed_archive/previous.complete"
        fi
    fi

    printf 'capture  %s/%s seed=%s\n' "$relative_path" "$replay_name" "$input_seed"
    capture_status=0
    failure_reason=""
    sdl_env=( SDL_AUDIODRIVER=dummy )
    if [[ "$headful" == 0 ]]; then
        sdl_env+=( SDL_VIDEODRIVER=dummy )
    fi

    if timeout --signal=TERM --kill-after=10s "${watchdog_seconds}s" \
        env \
            ROBINHOOD_DATA_DIR="$data_dir" \
            "${sdl_env[@]}" \
            "$binary" \
            -PARITYSAVE "$save_file" \
            -PARITYTRACE "$trace_base" \
            -PARITYSEED "$seed" \
            -PARITYFRAMES "$frames" \
            -PARITYRANDOMINPUT "$input_seed" \
            >"$log_path" 2>&1
    then
        shopt -s nullglob
        captured_traces=("$trace_stem"-session-*.jsonl)
        shopt -u nullglob
        if (( ${#captured_traces[@]} == 0 )); then
            capture_status=65
            failure_reason="no trace was produced"
        else
            for captured_trace in "${captured_traces[@]}"; do
                if [[ "$(tail -n 1 -- "$captured_trace")" != *'"type":"rng_suffix"'* ]]; then
                    capture_status=65
                    failure_reason="trace is incomplete: $captured_trace"
                    break
                fi
            done
            if (( capture_status == 0 )) && [[ "$compress" == 1 ]]; then
                for captured_trace in "${captured_traces[@]}"; do
                    # --convert audits every line for losslessness, publishes
                    # the native .parity.bitcode.zst artifact, and deletes the
                    # JSONL recording only after re-verifying the artifact.
                    if ! "$converter" --convert "$captured_trace" >>"$log_path" 2>&1; then
                        capture_status=65
                        failure_reason="unable to convert trace: $captured_trace"
                        break
                    fi
                done
            fi
        fi
    else
        capture_status=$?
        failure_reason="game exited with status $capture_status"
    fi

    if (( capture_status == 0 )); then
        : >"$complete_marker"
        return 0
    fi

    shopt -s nullglob
    failed_traces=("$trace_stem"-session-*.jsonl "$trace_stem"-session-*.jsonl.zst
        "$trace_stem"-session-*.jsonl.zst.parity.bitcode.zst)
    shopt -u nullglob
    if (( ${#failed_traces[@]} != 0 )); then
        failed_archive="$output_dir/incomplete-traces/$relative_path/$replay_name/failed-$(date -u +%Y%m%dT%H%M%SZ)-$BASHPID"
        mkdir -p -- "$failed_archive"
        mv -- "${failed_traces[@]}" "$failed_archive/"
    fi
    printf 'warning: capture failed: %s: %s/%s (see %s)\n' \
        "$failure_reason" "$relative_path" "$replay_name" "$log_path" >&2
    return 20
}

active_jobs=0

reap_one_capture() {
    local capture_result
    if wait -n; then
        capture_result=0
    else
        capture_result=$?
    fi
    active_jobs=$((active_jobs - 1))
    case "$capture_result" in
        0) captured=$((captured + 1)) ;;
        10) skipped=$((skipped + 1)) ;;
        *) failed=$((failed + 1)) ;;
    esac
}

stop_capture_jobs() {
    local -a child_pids
    trap - INT TERM
    mapfile -t child_pids < <(jobs -pr)
    if (( ${#child_pids[@]} != 0 )); then
        kill -- "${child_pids[@]}" 2>/dev/null || true
        wait "${child_pids[@]}" 2>/dev/null || true
    fi
    exit 130
}

trap stop_capture_jobs INT TERM

while IFS=$'\t' read -r relative_path replay_index input_seed; do
    if [[ -z "${relative_path:-}" || "$relative_path" == \#* ]]; then
        continue
    fi
    capture_one_replay "$relative_path" "$replay_index" "$input_seed" &
    active_jobs=$((active_jobs + 1))
    if (( active_jobs >= capture_jobs )); then
        reap_one_capture
    fi
done < "$manifest"

while (( active_jobs != 0 )); do
    reap_one_capture
done
trap - INT TERM

printf 'done: %d captured, %d failed, %d skipped\n' "$captured" "$failed" "$skipped"
if (( failed != 0 )); then
    exit 1
fi
