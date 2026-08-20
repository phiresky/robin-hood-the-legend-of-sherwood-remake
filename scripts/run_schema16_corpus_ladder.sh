#!/usr/bin/env bash
set -euo pipefail

# Sequentially record and validate schema16 random-input corpora.  Seed 2m is
# already owned by seed2m-recorder-10k, so this supervisor waits for that tmux
# job instead of starting a duplicate writer.  Later corpora are recorded here
# one at a time.  A zero-failure complete corpus terminates the ladder; any
# non-zero verdict advances the random-input base by one million.

workspace=${SCHEMA16_LADDER_WORKSPACE:-/home/phire/robinhood}
corpus_root="$workspace/parity-save-replays/60s-random-input"
audit_dir=${SCHEMA16_LADDER_AUDIT_DIR:-/home/phire/.cache/sccache/robinhood-parity-audits/autonomous-watch-7bb79e1d}
recorder=${SCHEMA16_LADDER_RECORDER:-$corpus_root/schema16-seed2000000-20260818/recorder/robin-schema16-3d5cc341-PENDING}
expected_recorder_sha=${SCHEMA16_LADDER_RECORDER_SHA:-3d5cc341d3f57202d1aaf42f518ec3a76da576d4a239bc660b799c9b0be73138}
first_seed_base=${SCHEMA16_LADDER_FIRST_SEED_BASE:-2000000}
seed_step=${SCHEMA16_LADDER_SEED_STEP:-1000000}
replays_per_save=${SCHEMA16_LADDER_REPLAYS_PER_SAVE:-40}
expected_saves=${SCHEMA16_LADDER_EXPECTED_SAVES:-243}
poll_seconds=${SCHEMA16_LADDER_POLL_SECONDS:-300}
capture_jobs=${SCHEMA16_LADDER_CAPTURE_JOBS:-8}
zstd_level=${SCHEMA16_LADDER_ZSTD_LEVEL:-16}
capture_externally=${SCHEMA16_LADDER_CAPTURE_EXTERNALLY:-0}
initial_recorder_session=${SCHEMA16_LADDER_INITIAL_RECORDER_SESSION:-seed2m-recorder-10k}
stop_file=${SCHEMA16_LADDER_STOP_FILE:-$corpus_root/.schema16-corpus-ladder.stop}
expected_replays=$((replays_per_save * expected_saves))

if [[ ! "$first_seed_base" =~ ^[0-9]+$ || ! "$seed_step" =~ ^[1-9][0-9]*$ ]]; then
    printf 'error: seed bases must be unsigned integers and step must be positive\n' >&2
    exit 2
fi
if [[ ! "$poll_seconds" =~ ^[1-9][0-9]*$ ]]; then
    printf 'error: SCHEMA16_LADDER_POLL_SECONDS must be positive\n' >&2
    exit 2
fi
if [[ ! "$capture_jobs" =~ ^[1-9][0-9]*$ || "$capture_jobs" -gt 10 ]]; then
    printf 'error: SCHEMA16_LADDER_CAPTURE_JOBS must be from 1 through 10\n' >&2
    exit 2
fi
if [[ ! "$zstd_level" =~ ^([1-9]|1[0-9]|2[0-2])$ ]]; then
    printf 'error: SCHEMA16_LADDER_ZSTD_LEVEL must be from 1 through 22\n' >&2
    exit 2
fi
if [[ "$capture_externally" != 0 && "$capture_externally" != 1 ]]; then
    printf 'error: SCHEMA16_LADDER_CAPTURE_EXTERNALLY must be 0 or 1\n' >&2
    exit 2
fi
if [[ ! -x "$recorder" ]]; then
    printf 'error: recorder is not executable: %s\n' "$recorder" >&2
    exit 2
fi
actual_recorder_sha=$(sha256sum -- "$recorder")
actual_recorder_sha=${actual_recorder_sha%% *}
if [[ "$actual_recorder_sha" != "$expected_recorder_sha" ]]; then
    printf 'error: recorder hash mismatch: expected %s, got %s\n' \
        "$expected_recorder_sha" "$actual_recorder_sha" >&2
    exit 2
fi

cd -- "$workspace"

find_campaign() {
    local seed_base=$1 campaign env_file value
    while IFS= read -r -d '' env_file; do
        value=$(sed -n 's/^PARITY_INPUT_SEED_BASE=//p' "$env_file" | head -n 1)
        if [[ "$value" == "$seed_base" ]]; then
            campaign=${env_file%/campaign.env}
            printf '%s\n' "$campaign"
            return 0
        fi
    done < <(find "$corpus_root" -mindepth 2 -maxdepth 2 -type f \
        -path '*/schema16-seed*/campaign.env' -print0 | sort -z)
    return 1
}

create_campaign() {
    local seed_base=$1 campaign created_utc
    campaign="$corpus_root/schema16-seed${seed_base}-$(date -u +%Y%m%d)"
    mkdir -p -- "$campaign" "$campaign/traces" "$campaign/logs"
    created_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    {
        printf 'CAMPAIGN_CREATED_UTC=%s\n' "$created_utc"
        printf 'CAMPAIGN_STATE=recording_autonomous_ladder\n'
        printf 'PARITY_TRACE_SCHEMA=16\n'
        printf 'PARITY_RANDOM_REPLAYS=%s\n' "$replays_per_save"
        printf 'PARITY_FRAMES=1500\n'
        printf 'PARITY_INPUT_SEED_BASE=%s\n' "$seed_base"
        printf 'PARITY_INPUT_SEED_FIRST=%s\n' "$seed_base"
        printf 'PARITY_INPUT_SEED_LAST=%s\n' "$((seed_base + replays_per_save - 1))"
        printf 'PARITY_SEED=1\n'
        printf 'SHERWOOD_LIMIT=30\n'
        printf 'SHERWOOD_SAMPLE_SEED=1\n'
        printf 'SHARD_COUNT=1\n'
        printf 'CAPTURE_JOBS=%s\n' "$capture_jobs"
        printf 'GLOBAL_ORIGINAL_PROCESS_LIMIT=%s\n' "$capture_jobs"
        printf 'ZSTD_LEVEL=%s\n' "$zstd_level"
        printf 'EXPECTED_SELECTED_SAVES=%s\n' "$expected_saves"
        printf 'EXPECTED_LOGICAL_REPLAYS=%s\n' "$expected_replays"
        printf 'EXPECTED_FRAMES_PER_REPLAY=1500\n'
        printf 'ROBIN_BINARY=%s\n' "$recorder"
        printf 'ROBIN_BINARY_SHA256=%s\n' "$expected_recorder_sha"
    } >"$campaign/campaign.env"
    printf '%s\n' "$campaign"
}

completed_replays() {
    local campaign=$1
    find "$campaign/traces" -type f -name '*.complete' 2>/dev/null | wc -l
}

run_capture() {
    local campaign=$1 seed_base=$2
    printf '%s capture start seed=%s campaign=%s\n' "$(date -Is)" "$seed_base" "$campaign"
    nice -n 0 ionice -c 3 env \
        PARITY_TRACE_SCHEMA=16 \
        PARITY_RANDOM_REPLAYS="$replays_per_save" \
        PARITY_FRAMES=1500 \
        PARITY_INPUT_SEED_BASE="$seed_base" \
        PARITY_SEED=1 \
        SHERWOOD_LIMIT=30 \
        SHERWOOD_SAMPLE_SEED=1 \
        SHARD_COUNT=1 \
        SHARD_INDEX=0 \
        CAPTURE_JOBS="$capture_jobs" \
        COMPRESS=1 \
        ZSTD_THREADS=1 \
        ZSTD_LEVEL="$zstd_level" \
        HEADFUL=0 \
        SKIP_BUILD=1 \
        WATCHDOG_SECONDS=2700 \
        CAPTURE_MIN_FREE_KIB=31457280 \
        CAPTURE_RESERVE_KIB=9437184 \
        CAPTURE_EMERGENCY_FREE_KIB=32505856 \
        ROBIN_BINARY="$recorder" \
        ROBIN_LIBRARY_DIR=/lib/i386-linux-gnu \
        ROBINHOOD_DATA_DIR="$workspace/datadirs/fullgame_linux" \
        "$workspace/original-code/scripts/capture_parity_save_replays.sh" \
        "$workspace/reference-saves" \
        "$campaign" \
        "$workspace/datadirs/fullgame_linux" \
        2>&1 | tee -a "$campaign/capture-ladder.log"
}

status_path_for_trace() {
    local trace=$1 relative key
    relative=${trace#"$workspace"/}
    key=${relative//\//__}
    printf '%s/status/%s.status\n' "$audit_dir" "$key"
}

write_verdict_if_ready() {
    local campaign=$1 trace status value
    local total=0 completed=0 passed=0 failed=0 temporary
    local failures_tmp="$campaign/parity-failures.tsv.tmp"
    : >"$failures_tmp"
    while IFS= read -r -d '' trace; do
        total=$((total + 1))
        status=$(status_path_for_trace "$trace")
        if [[ ! -f "$status" ]]; then
            continue
        fi
        completed=$((completed + 1))
        read -r value <"$status" || value=unreadable
        if [[ "$value" == 0 ]]; then
            passed=$((passed + 1))
        else
            failed=$((failed + 1))
            printf '%s\t%s\n' "$value" "$trace" >>"$failures_tmp"
        fi
    done < <(find "$campaign/traces" -type f -name '*.jsonl.zst' -print0 | sort -z)

    if (( total != expected_replays || completed != expected_replays )); then
        rm -f -- "$failures_tmp"
        return 1
    fi
    mv -f -- "$failures_tmp" "$campaign/parity-failures.tsv"
    temporary="$campaign/parity-verdict.env.tmp"
    {
        printf 'VERIFIED_UTC=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
        printf 'RUNNER=%s\n' "$(sed -n 's/^RUNNER=//p' "$audit_dir/summary.env" 2>/dev/null | head -n 1)"
        printf 'TOTAL=%s\n' "$total"
        printf 'PASSED=%s\n' "$passed"
        printf 'FAILED=%s\n' "$failed"
        printf 'EXACT_PARITY=%s\n' "$([[ "$failed" == 0 ]] && printf 1 || printf 0)"
    } >"$temporary"
    mv -f -- "$temporary" "$campaign/parity-verdict.env"
    printf '%s\n' "$failed"
}

seed_base=$first_seed_base
printf '%s schema16 corpus ladder started at seed %s; stop file %s\n' \
    "$(date -Is)" "$seed_base" "$stop_file"
while [[ ! -e "$stop_file" ]]; do
    if ! campaign=$(find_campaign "$seed_base"); then
        campaign=$(create_campaign "$seed_base")
    fi

    complete=$(completed_replays "$campaign")
    if (( complete < expected_replays )); then
        if [[ "$capture_externally" == 1 ]]; then
            printf '%s seed=%s external capture=%s/%s; waiting\n' \
                "$(date -Is)" "$seed_base" "$complete" "$expected_replays"
            sleep "$poll_seconds"
            continue
        fi
        if [[ "$seed_base" == "$first_seed_base" ]] \
            && tmux has-session -t "$initial_recorder_session" 2>/dev/null
        then
            printf '%s seed=%s capture=%s/%s; external recorder still active\n' \
                "$(date -Is)" "$seed_base" "$complete" "$expected_replays"
            sleep "$poll_seconds"
            continue
        fi
        if ! run_capture "$campaign" "$seed_base"; then
            printf '%s seed=%s capture exited nonzero; retrying incomplete work after poll\n' \
                "$(date -Is)" "$seed_base" >&2
            sleep "$poll_seconds"
            continue
        fi
        continue
    fi

    if failed=$(write_verdict_if_ready "$campaign"); then
        if (( failed == 0 )); then
            printf '%s seed=%s achieved %s/%s exact parity; ladder complete\n' \
                "$(date -Is)" "$seed_base" "$expected_replays" "$expected_replays"
            exit 0
        fi
        printf '%s seed=%s complete with %s failure(s); advancing seed base\n' \
            "$(date -Is)" "$seed_base" "$failed"
        seed_base=$((seed_base + seed_step))
        continue
    fi

    printf '%s seed=%s capture complete; waiting for %s parity verdicts\n' \
        "$(date -Is)" "$seed_base" "$expected_replays"
    sleep "$poll_seconds"
done

printf '%s schema16 corpus ladder stopped by %s\n' "$(date -Is)" "$stop_file"
