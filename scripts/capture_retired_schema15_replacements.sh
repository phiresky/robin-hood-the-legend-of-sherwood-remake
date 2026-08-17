#!/usr/bin/env bash
set -euo pipefail

# Re-record only the retired traces that can be replaced by a fresh current-
# Original schema-15 oracle. Undefined-behavior, missing-input, intentional RNG
# policy, and already-superseded traces are deliberately excluded.

workspace_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
original_dir="$workspace_dir/original-code"
ledger="$workspace_dir/docs/PARITY_RETIRED_TRACES.txt"
binary="${ROBIN_BINARY:-$original_dir/build/native-full/robin}"
data_dir="${ROBINHOOD_DATA_DIR:-$workspace_dir/datadirs/fullgame_linux}"
capture_jobs="${CAPTURE_JOBS:-3}"
watchdog_seconds="${WATCHDOG_SECONDS:-2700}"

replacement_60="$workspace_dir/parity-save-replays/60s-random-input/schema15-replacements-20260817"
replacement_30="$workspace_dir/parity-save-replays/30s-random-input/schema15-replacements-20260817"
replacement_10="$workspace_dir/parity-save-replays-legacy/10s-no-input-schema15-replacements-20260817"
result_file="$replacement_60/batch-results.tsv"
result_lock="$replacement_60/.batch-results.lock"
pause_file="$replacement_60/.batch.pause"

usage() {
    printf 'usage: %s [--dry-run] [--worker TRACE]\n' "$0"
}

classify_retired_paths() {
    awk '
        function classify(context) {
            if (context ~ /ShootType|LowerBow|OUT_OF_RANGE/) return "blocked";
            if (context ~ /old_elevation/) return "blocked";
            if (context ~ /sprite timing UB/) return "blocked";
            if (context ~ /sword-strike record omits/) return "blocked";
            if (context ~ /beggar pay click/) return "blocked";
            if (context ~ /Auxiliary audio RNG policy/) return "blocked";
            return "direct";
        }
        /^#/ {
            if (had_path) {
                context = $0;
                had_path = 0;
            } else {
                context = context " " $0;
            }
            next;
        }
        NF {
            print classify(context) "\t" $0;
            had_path = 1;
        }
    ' "$ledger"
}

normalize_trace_path() {
    local retired_rel="$1"
    case "$retired_rel" in
        parity-save-replays/*|parity-save-replays-legacy/*)
            printf '%s/%s\n' "$workspace_dir" "$retired_rel"
            ;;
        *)
            printf '%s/parity-save-replays/60s-random-input/schema12/traces/%s\n' \
                "$workspace_dir" "$retired_rel"
            ;;
    esac
}

is_already_superseded() {
    local source_trace="$1"
    local relative="${source_trace#"$workspace_dir/"}"
    local schema14

    if [[ "$relative" == parity-save-replays/60s-random-input/schema12/traces/* ]]; then
        schema14="$workspace_dir/${relative/schema12/schema14}"
        [[ -f "$schema14" ]] && return 0
    fi

    # The short replay is a prefix of the same seed's longer schema-14 capture.
    if [[ "$relative" == \
        parity-save-replays/30s-random-input/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_026/replay-001-session-0001.jsonl.zst ]]
    then
        return 0
    fi

    case "$relative" in
        parity-save-replays/60s-random-input/schema12/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_013/replay-006-session-0001.jsonl.zst|\
        parity-save-replays/60s-random-input/schema12/traces/Savegame_nicouzouf/Profile_001/Savegame_020/replay-014-session-0001.jsonl.zst|\
        parity-save-replays/60s-random-input/schema12/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_004/replay-003-session-0001.jsonl.zst)
            return 0
            ;;
    esac
    return 1
}

resolve_capture() {
    local source_trace="$1"
    local relative="${source_trace#"$workspace_dir/"}"
    local after_traces save_relative base

    after_traces="${relative#*/traces/}"
    case "$relative" in
        parity-save-replays/60s-random-input/*)
            frames=1500
            output_root="$replacement_60"
            save_relative="${after_traces%/*}"
            ;;
        parity-save-replays/30s-random-input/*)
            frames=750
            output_root="$replacement_30"
            save_relative="${after_traces%/*}"
            ;;
        parity-save-replays-legacy/10s-no-input-schema12/*)
            frames=250
            output_root="$replacement_10"
            base="${after_traces%-session-0001.jsonl.zst}"
            save_relative="$base"
            ;;
        *)
            printf 'unsupported retired corpus path: %s\n' "$relative" >&2
            return 1
            ;;
    esac

    save_file="$workspace_dir/reference-saves/$save_relative"
    destination="$output_root/traces/$after_traces"
    destination_base="${destination%-session-0001.jsonl.zst}"
    raw_trace="$destination_base-session-0001.jsonl"
    compressed_trace="$raw_trace.zst"
    complete_marker="$compressed_trace.complete"
    log_file="$output_root/logs/${after_traces//\//__}.log"

    rng_seed=1
    input_seed=""
    if [[ -f "$source_trace" ]]; then
        local header
        # Reading only the header intentionally closes the decompressor pipe
        # early. Ignore zstd's resulting SIGPIPE while retaining the line.
        header="$({ zstd -dc --long=31 -- "$source_trace" 2>/dev/null | {
            IFS= read -r line
            printf '%s' "$line"
        }; } || true)"
        [[ -n "$header" ]] || {
            printf 'missing retired trace header: %s\n' "$source_trace" >&2
            return 1
        }
        rng_seed="$(jq -er '.rng_seed' <<<"$header")"
        input_seed="$(jq -r '.random_input_seed // empty' <<<"$header")"
    elif [[ "$relative" != parity-save-replays-legacy/10s-no-input-schema12/* ]]; then
        printf 'missing retired trace header: %s\n' "$source_trace" >&2
        return 1
    fi
}

append_result() {
    local status="$1"
    local source_trace="$2"
    local output_trace="$3"
    local message="$4"
    local result_fd

    mkdir -p -- "$(dirname -- "$result_file")"
    exec {result_fd}>>"$result_lock"
    flock "$result_fd"
    printf '%s\t%s\t%s\t%s\n' \
        "$status" \
        "${source_trace#"$workspace_dir/"}" \
        "${output_trace#"$workspace_dir/"}" \
        "$message" >>"$result_file"
    flock -u "$result_fd"
    exec {result_fd}>&-
}

capture_one() {
    local source_trace="$1"
    local attempt_dir attempt_base produced_raw final_frame expected_final
    local -a random_input_args=()

    while [[ -e "$pause_file" ]]; do
        sleep 2
    done

    resolve_capture "$source_trace"
    if [[ -f "$complete_marker" ]]; then
        append_result skip "$source_trace" "$compressed_trace" already_complete
        return 0
    fi
    if [[ ! -f "$save_file" ]]; then
        append_result fail "$source_trace" "$compressed_trace" missing_save
        return 1
    fi

    mkdir -p -- "$output_root/logs" "$output_root/.attempts" \
        "$(dirname -- "$compressed_trace")"
    attempt_dir="$(mktemp -d "$output_root/.attempts/capture.XXXXXX")"
    attempt_base="$attempt_dir/trace.jsonl"
    if [[ -n "$input_seed" ]]; then
        random_input_args=(-PARITYRANDOMINPUT "$input_seed")
    fi

    env \
        ROBINHOOD_DATA_DIR="$data_dir" \
        SDL_AUDIODRIVER=dummy \
        SDL_VIDEODRIVER=dummy \
        timeout --signal=TERM --kill-after=5s "$watchdog_seconds" \
        "$binary" \
        -PARITYSAVE "$save_file" \
        -PARITYTRACE "$attempt_base" \
        -PARITYSEED "$rng_seed" \
        -PARITYFRAMES "$frames" \
        "${random_input_args[@]}" >"$log_file" 2>&1 || true

    produced_raw="$attempt_dir/trace-session-0001.jsonl"
    # launcher.cpp returns after handing control to the game process. Wait for
    # the recorder's terminal suffix rather than treating launcher exit as
    # capture completion. A file that makes no progress for two minutes is a
    # failed capture; the outer watchdog remains the absolute upper bound.
    local deadline=$((SECONDS + watchdog_seconds))
    local last_size=-1
    local unchanged_since=$SECONDS
    while (( SECONDS < deadline )); do
        if [[ -f "$produced_raw" ]]; then
            local current_size
            current_size="$(stat -c %s "$produced_raw")"
            if [[ "$current_size" -ne "$last_size" ]]; then
                last_size="$current_size"
                unchanged_since=$SECONDS
            elif (( SECONDS - unchanged_since >= 120 )); then
                append_result fail "$source_trace" "$compressed_trace" stalled_output
                return 1
            fi
            if tail -n 1 "$produced_raw" 2>/dev/null | jq -e \
                --argjson frames "$frames" \
                '.type == "rng_suffix" and .frame_count == $frames' >/dev/null 2>&1
            then
                break
            fi
        elif (( SECONDS - unchanged_since >= 120 )); then
            append_result fail "$source_trace" "$compressed_trace" missing_output
            return 1
        fi
        sleep 1
    done
    if [[ ! -f "$produced_raw" ]] || ! tail -n 1 "$produced_raw" | jq -e \
        --argjson frames "$frames" \
        '.type == "rng_suffix" and .frame_count == $frames' >/dev/null
    then
        append_result fail "$source_trace" "$compressed_trace" capture_timeout
        return 1
    fi
    if ! sed -n '1p' "$produced_raw" | jq -e \
        --argjson rng "$rng_seed" \
        --arg input "$input_seed" \
        '.type == "header" and .schema == 15 and .rng_seed == $rng and
         (($input == "" and (.random_input_seed == null)) or
          ($input != "" and .random_input_seed == ($input | tonumber)))' >/dev/null
    then
        append_result fail "$source_trace" "$compressed_trace" bad_header
        return 1
    fi
    if [[ "$(rg -c '"type":"frame"' "$produced_raw")" -ne "$frames" ]]; then
        append_result fail "$source_trace" "$compressed_trace" bad_frame_count
        return 1
    fi
    if ! tail -n 1 "$produced_raw" | jq -e --argjson frames "$frames" \
        '.type == "rng_suffix" and .frame_count == $frames' >/dev/null
    then
        append_result fail "$source_trace" "$compressed_trace" bad_terminator
        return 1
    fi
    final_frame="$(tail -n 1 "$produced_raw" | jq -er '.final_frame')"
    expected_final="$(sed -n '1p' "$produced_raw" | jq -er '.initial_frame // empty' 2>/dev/null || true)"
    if [[ -n "$expected_final" && "$final_frame" -ne $((expected_final + frames)) ]]; then
        append_result fail "$source_trace" "$compressed_trace" bad_final_frame
        return 1
    fi

    zstd -1 -q -T1 --long=31 -f "$produced_raw" -o "$attempt_dir/trace.jsonl.zst"
    zstd -t -q --long=31 "$attempt_dir/trace.jsonl.zst"
    mv -- "$attempt_dir/trace.jsonl.zst" "$compressed_trace"
    printf 'schema=15 frames=%s rng_seed=%s input_seed=%s final_frame=%s sha256=%s\n' \
        "$frames" "$rng_seed" "${input_seed:-none}" "$final_frame" \
        "$(sha256sum "$compressed_trace" | cut -d ' ' -f 1)" >"$complete_marker"
    find "$attempt_dir" -depth -delete
    append_result ok "$source_trace" "$compressed_trace" "frames=$frames"
}

if [[ "${1:-}" == --worker ]]; then
    [[ $# -eq 2 ]] || { usage >&2; exit 2; }
    capture_one "$2"
    exit
fi

dry_run=0
if [[ "${1:-}" == --dry-run ]]; then
    dry_run=1
    shift
fi
[[ $# -eq 0 ]] || { usage >&2; exit 2; }
[[ "$capture_jobs" =~ ^[1-9][0-9]*$ ]] || {
    printf 'CAPTURE_JOBS must be a positive integer\n' >&2
    exit 2
}
[[ -x "$binary" ]] || { printf 'missing Original binary: %s\n' "$binary" >&2; exit 2; }

candidate_file="$(mktemp)"
trap 'rm -f -- "$candidate_file"' EXIT
while IFS=$'\t' read -r classification retired_rel; do
    [[ "$classification" == direct ]] || continue
    source_trace="$(normalize_trace_path "$retired_rel")"
    is_already_superseded "$source_trace" && continue
    printf '%s\n' "$source_trace" >>"$candidate_file"
done < <(classify_retired_paths)

candidate_count="$(wc -l <"$candidate_file")"
if [[ "$candidate_count" -ne 95 ]]; then
    printf 'refusing unexpected direct-candidate count: got %s, expected 95\n' \
        "$candidate_count" >&2
    exit 2
fi
if [[ "$dry_run" == 1 ]]; then
    cat "$candidate_file"
    exit
fi

mkdir -p -- "$replacement_60"
printf 'status\tretired_trace\treplacement_trace\tdetail\n' >"$result_file"
xargs -P "$capture_jobs" -n 1 "$0" --worker <"$candidate_file"
printf 'schema-15 replacement batch complete: %s candidates\n' "$candidate_count"
