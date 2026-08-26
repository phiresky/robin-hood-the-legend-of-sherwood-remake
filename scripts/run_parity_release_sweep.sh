#!/usr/bin/env bash
set -u

if (( $# < 4 || $# > 5 )); then
    printf 'usage: %s CORPUS_DIR AUDIT_DIR RUNNER SHARD [SHARDS]\n' "$0" >&2
    exit 2
fi

corpus_dir=$1
audit_dir=$2
runner=$3
shard=$4
shards=${5:-1}

if [[ ! "$shard" =~ ^[0-9]+$ || ! "$shards" =~ ^[1-9][0-9]*$ || "$shard" -ge "$shards" ]]; then
    printf 'error: require 0 <= SHARD < SHARDS\n' >&2
    exit 2
fi
if [[ ! -x "$runner" ]]; then
    printf 'error: runner is not executable: %s\n' "$runner" >&2
    exit 2
fi

workspace=$(pwd)
snapshot="$audit_dir/traces.snapshot"

# Recordings are independent and each runner has its own process state. Runner
# processes from distinct audits nevertheless share this host-wide slot pool
# so callers can bound aggregate CPU and memory use. PARITY_SWEEP_CONCURRENCY
# remains a compatibility alias; coordinated callers should use the global
# spelling consistently. Parallel fail-fast shards coordinate through an
# explicit shared stop file: no new trace starts after a failure publishes it,
# while already-running traces are allowed to finish and publish their proof.
global_concurrency=${PARITY_SWEEP_GLOBAL_CONCURRENCY:-${PARITY_SWEEP_CONCURRENCY:-1}}
if [[ ! "$global_concurrency" =~ ^[1-9][0-9]*$ ]]; then
    printf 'error: PARITY_SWEEP_GLOBAL_CONCURRENCY must be a positive integer\n' >&2
    exit 2
fi
fail_fast=${PARITY_SWEEP_FAIL_FAST:-0}
if [[ "$fail_fast" != 0 && "$fail_fast" != 1 ]]; then
    printf 'error: PARITY_SWEEP_FAIL_FAST must be 0 or 1\n' >&2
    exit 2
fi
fail_fast_stop=${PARITY_SWEEP_FAIL_FAST_STOP:-}
fail_fast_token=${PARITY_SWEEP_FAIL_FAST_TOKEN:-}
fail_fast_gate=
if [[ "$fail_fast" == 1 \
    && ( "$shard" != 0 || "$shards" != 1 || "$global_concurrency" != 1 ) \
    && -z "$fail_fast_stop" ]]
then
    printf '%s\n' \
        'error: parallel PARITY_SWEEP_FAIL_FAST=1 requires PARITY_SWEEP_FAIL_FAST_STOP' \
        >&2
    exit 2
fi
if [[ -n "$fail_fast_stop" ]]; then
    if [[ "$fail_fast_stop" != /* || "$fail_fast_stop" == *$'\n'* ]]; then
        printf 'error: PARITY_SWEEP_FAIL_FAST_STOP must be an absolute newline-free path\n' >&2
        exit 2
    fi
    if [[ ! "$fail_fast_token" =~ ^[0-9a-f]{32,64}$ ]]; then
        printf 'error: PARITY_SWEEP_FAIL_FAST_TOKEN must contain 32-64 lowercase hexadecimal digits\n' >&2
        exit 2
    fi
    mkdir -p -- "${fail_fast_stop%/*}" || exit 2
    fail_fast_gate="${fail_fast_stop}.start-gate.lock"
fi
if ! mkdir -p "$audit_dir/logs" "$audit_dir/status" "$audit_dir/.trace-locks"; then
    if [[ "$fail_fast" == 1 ]]; then
        exit 1
    fi
fi
runner_slot_dir=${PARITY_SWEEP_SLOT_DIR:-$workspace/.git/parity-runner-slots}
if ! mkdir -p "$runner_slot_dir"; then
    if [[ "$fail_fast" == 1 ]]; then
        exit 1
    fi
fi

exact_eof_marker='parity trace matched every recorded frame'
integrity_status='integrity-eof-marker'
run_log=
cleanup_private_log() {
    if [[ -n "$run_log" ]]; then
        rm -f -- "$run_log"
    fi
}
trap cleanup_private_log EXIT
trap 'cleanup_private_log; exit 130' INT TERM

write_status() {
    local destination=$1
    local value=$2
    local temporary

    temporary=$(mktemp "${destination}.tmp.XXXXXX") || return 1
    if ! printf '%s\n' "$value" > "$temporary"; then
        rm -f -- "$temporary"
        return 1
    fi
    if ! mv -f -- "$temporary" "$destination"; then
        rm -f -- "$temporary"
        return 1
    fi
}

acquire_runner_slot() {
    local slot
    while true; do
        if [[ "$fail_fast" == 1 && -n "$fail_fast_stop" && -e "$fail_fast_stop" ]]; then
            return 1
        fi
        for ((slot = 0; slot < global_concurrency; slot += 1)); do
            exec {runner_slot_fd}>"$runner_slot_dir/$slot.lock"
            if flock -n "$runner_slot_fd"; then
                return 0
            fi
            exec {runner_slot_fd}>&-
        done
        sleep 1
    done
}

publish_fail_fast_stop() {
    local temporary
    [[ "$fail_fast" == 1 && -n "$fail_fast_stop" ]] || return 0
    exec {stop_gate_fd}>"$fail_fast_gate" || return 1
    flock "$stop_gate_fd" || { exec {stop_gate_fd}>&-; return 1; }
    if [[ -e "$fail_fast_stop" ]]; then
        exec {stop_gate_fd}>&-
        return 0
    fi
    temporary=$(mktemp "${fail_fast_stop}.tmp.XXXXXX") \
        || { exec {stop_gate_fd}>&-; return 1; }
    {
        printf 'FAIL_FAST_BATCH_TOKEN=%s\n' "$fail_fast_token"
        printf 'FAILED_UTC=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    } >"$temporary" \
        || { rm -f -- "$temporary"; exec {stop_gate_fd}>&-; return 1; }
    # A hard link is an atomic create-without-replace. Multiple failing shards
    # may race here; exactly one publishes and all observe the same stop path.
    ln -- "$temporary" "$fail_fast_stop" 2>/dev/null || [[ -e "$fail_fast_stop" ]] \
        || { rm -f -- "$temporary"; exec {stop_gate_fd}>&-; return 1; }
    rm -f -- "$temporary"
    exec {stop_gate_fd}>&-
}

stop_after_failure() {
    if ! publish_fail_fast_stop; then
        printf 'error: unable to publish shared fail-fast stop: %s\n' \
            "$fail_fast_stop" >&2
        exit 70
    fi
    exit 1
}

status_is_exact_eof() {
    local status=$1
    local log=$2
    local value marker_count

    [[ -f "$status" && -f "$log" ]] || return 1
    value=$(<"$status") || return 1
    [[ "$value" == 0 ]] || return 1
    marker_count=$(grep -Fxc -- "$exact_eof_marker" "$log" || true)
    [[ "$marker_count" == 1 ]]
}

if [[ ! -f "$snapshot" ]]; then
    printf 'error: trace snapshot does not exist: %s\n' "$snapshot" >&2
    exit 2
fi

# A resumed campaign can encounter traces that already reached exact EOF in
# another audit.  Status filenames in this sweep are relative to `corpus_dir`,
# while the permanent ledger uses repository-relative campaign keys.  Import
# that ledger explicitly so the spelling difference can never cause a known
# EOF replay to run again.
permanent_eof_snapshot=${PARITY_PERMANENT_EOF_SNAPSHOT:-}
declare -A permanent_eof_keys=()
if [[ -n "$permanent_eof_snapshot" ]]; then
    if [[ ! -f "$permanent_eof_snapshot" ]]; then
        printf 'error: permanent EOF snapshot does not exist: %s\n' \
            "$permanent_eof_snapshot" >&2
        exit 2
    fi
    while IFS= read -r permanent_key; do
        [[ -n "$permanent_key" ]] || continue
        permanent_eof_keys["$permanent_key"]=1
    done < "$permanent_eof_snapshot"
fi

corpus_relative=${corpus_dir#"$workspace"/}
campaign_key_prefix=${corpus_relative//\//__}

mapfile -t traces < "$snapshot"
for ((index = shard; index < ${#traces[@]}; index += shards)); do
    if [[ "$fail_fast" == 1 && -n "$fail_fast_stop" && -e "$fail_fast_stop" ]]; then
        exit 1
    fi
    trace=${traces[index]}
    relative=${trace#"$corpus_dir"/}
    key=${relative//\//__}
    # Older launches used a corpus root of `.` and therefore retained the
    # leading `parity-save-replays/` in status/log keys.  Treat that spelling
    # as the same logical trace so a resumed sweep does not rerun already
    # completed work under a second filename namespace.
    full_key=${trace//\//__}
    canonical_key="${campaign_key_prefix}__${key}"
    log="$audit_dir/logs/$key.log"
    status="$audit_dir/status/$key.status"

    # Two watcher instances can briefly overlap during a restart.  Sharding
    # prevents duplicate work within one invocation, but it cannot stop both
    # invocations from observing the same absent status and writing the same
    # log concurrently.  Serialize each logical trace across the whole audit,
    # then repeat the status check while holding the claim.
    exec {trace_lock_fd}>"$audit_dir/.trace-locks/$full_key.lock"
    flock "$trace_lock_fd"

    if [[ "$fail_fast" == 1 && -n "$fail_fast_stop" && -e "$fail_fast_stop" ]]; then
        exec {trace_lock_fd}>&-
        exit 1
    fi

    existing_status=
    existing_log=
    if [[ -f "$status" ]]; then
        existing_status=$status
        existing_log=$log
    elif [[ -f "$audit_dir/status/$full_key.status" ]]; then
        existing_status="$audit_dir/status/$full_key.status"
        existing_log="$audit_dir/logs/$full_key.log"
    fi
    if [[ -n "$existing_status" ]]; then
        exec {trace_lock_fd}>&-
        if [[ "$fail_fast" == 1 ]] \
            && ! status_is_exact_eof "$existing_status" "$existing_log"
        then
            stop_after_failure
        fi
        continue
    fi
    # Permanent evidence may skip a runner invocation, but it must never hide
    # corrupt local evidence in this audit. The local status/log check above is
    # therefore intentionally performed first while holding the trace claim.
    if [[ -n "$permanent_eof_snapshot" \
        && -n "${permanent_eof_keys[$canonical_key]+present}" ]]
    then
        exec {trace_lock_fd}>&-
        continue
    fi
    # A converted trace exists only as its native artifact; the runner
    # resolves the logical .jsonl.zst path to it on its own.
    if [[ ! -f "$trace" && ! -f "$trace.parity.bitcode.zst" ]]; then
        if ! write_status "$status" missing; then
            exec {trace_lock_fd}>&-
            if [[ "$fail_fast" == 1 ]]; then
                exit 1
            fi
            continue
        fi
        exec {trace_lock_fd}>&-
        if [[ "$fail_fast" == 1 ]]; then
            stop_after_failure
        fi
        continue
    fi

    # Keep the in-flight output private.  A distributed sweep may mirror a
    # completed log for the same trace into this audit while this runner is
    # active.  If the runner writes directly to the published pathname, that
    # atomic rsync replacement makes the EOF check inspect the mirrored file
    # instead of the inode this runner actually wrote, producing a false
    # integrity-eof-marker result.
    if ! run_log=$(mktemp "${log}.tmp.XXXXXX"); then
        printf 'error: unable to create private parity log for: %s\n' "$log" >&2
        exec {trace_lock_fd}>&-
        if [[ "$fail_fast" == 1 ]]; then
            stop_after_failure
        fi
        continue
    fi

    if ! acquire_runner_slot; then
        rm -f -- "$run_log"
        exec {trace_lock_fd}>&-
        exit 1
    fi
    runner_command_status=0
    if [[ "$fail_fast" == 1 && -n "$fail_fast_stop" ]]; then
        exec {start_gate_fd}>"$fail_fast_gate"
        flock "$start_gate_fd"
        if [[ -e "$fail_fast_stop" ]]; then
            exec {start_gate_fd}>&-
            exec {runner_slot_fd}>&-
            rm -f -- "$run_log"
            run_log=
            exec {trace_lock_fd}>&-
            exit 1
        fi
        timeout --foreground --signal=TERM --kill-after=10s 900s \
            env ROBINHOOD_DATA_DIR="$workspace/datadirs/fullgame_linux" \
            "$runner" --no-auto-dump "$trace" > "$run_log" 2>&1 \
            {start_gate_fd}>&- &
        runner_pid=$!
        # Publication of a failure cannot cross this boundary: the runner is
        # already started before its shard releases the shared start gate.
        exec {start_gate_fd}>&-
        wait "$runner_pid" || runner_command_status=$?
    else
        timeout --foreground --signal=TERM --kill-after=10s 900s \
            env ROBINHOOD_DATA_DIR="$workspace/datadirs/fullgame_linux" \
            "$runner" --no-auto-dump "$trace" > "$run_log" 2>&1 \
            || runner_command_status=$?
    fi
    if (( runner_command_status == 0 )); then
        marker_count=$(grep -Fxc -- "$exact_eof_marker" "$run_log" || true)
        if [[ "$marker_count" == 1 ]]; then
            runner_status=0
        else
            runner_status=$integrity_status
        fi
    else
        runner_status=$runner_command_status
    fi
    stop_publication_failed=0
    if [[ "$fail_fast" == 1 && "$runner_status" != 0 ]]; then
        publish_fail_fast_stop || stop_publication_failed=1
    fi
    result_published=0
    if mv -f -- "$run_log" "$log" \
        && write_status "$status" "$runner_status"
    then
        result_published=1
        run_log=
    else
        printf 'error: unable to publish parity result: %s\n' "$log" >&2
        rm -f -- "$run_log"
    fi
    if [[ "$fail_fast" == 1 && "$result_published" != 1 \
        && "$runner_status" == 0 ]]
    then
        publish_fail_fast_stop || stop_publication_failed=1
    fi
    exec {runner_slot_fd}>&-
    exec {trace_lock_fd}>&-
    if [[ "$fail_fast" == 1 \
        && ( "$result_published" != 1 || "$runner_status" != 0 ) ]]
    then
        if (( stop_publication_failed != 0 )); then
            printf 'error: unable to publish shared fail-fast stop: %s\n' \
                "$fail_fast_stop" >&2
            exit 70
        fi
        exit 1
    fi
done

if [[ "$fail_fast" == 1 && -n "$fail_fast_stop" && -e "$fail_fast_stop" ]]; then
    exit 1
fi
