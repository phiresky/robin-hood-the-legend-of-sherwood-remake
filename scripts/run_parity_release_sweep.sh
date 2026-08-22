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
mkdir -p "$audit_dir/logs" "$audit_dir/status" "$audit_dir/.trace-locks"

# Runner processes from distinct audit directories must share the same slot
# pool.  Original's asynchronous pathfinder is sensitive to several parity
# runners competing on the host, so an audit-local lock can produce a result
# that a serial replay cannot reproduce.  Keep the safe default globally
# serial.  PARITY_SWEEP_CONCURRENCY remains a compatibility alias, but callers
# that deliberately want parallelism should set the global spelling to the
# same value for every participating sweep.
global_concurrency=${PARITY_SWEEP_GLOBAL_CONCURRENCY:-${PARITY_SWEEP_CONCURRENCY:-1}}
if [[ ! "$global_concurrency" =~ ^[1-9][0-9]*$ ]]; then
    printf 'error: PARITY_SWEEP_GLOBAL_CONCURRENCY must be a positive integer\n' >&2
    exit 2
fi
runner_slot_dir=${PARITY_SWEEP_SLOT_DIR:-$workspace/.git/parity-runner-slots}
mkdir -p "$runner_slot_dir"

exact_eof_marker='parity trace matched every recorded frame'
integrity_status='integrity-eof-marker'

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

    if [[ -n "$permanent_eof_snapshot" \
        && -n "${permanent_eof_keys[$canonical_key]+present}" ]]
    then
        continue
    fi

    # Two watcher instances can briefly overlap during a restart.  Sharding
    # prevents duplicate work within one invocation, but it cannot stop both
    # invocations from observing the same absent status and writing the same
    # log concurrently.  Serialize each logical trace across the whole audit,
    # then repeat the status check while holding the claim.
    exec {trace_lock_fd}>"$audit_dir/.trace-locks/$full_key.lock"
    flock "$trace_lock_fd"

    if [[ -f "$status" || -f "$audit_dir/status/$full_key.status" ]]; then
        exec {trace_lock_fd}>&-
        continue
    fi
    if [[ ! -f "$trace" ]]; then
        write_status "$status" missing
        exec {trace_lock_fd}>&-
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
        continue
    fi

    acquire_runner_slot
    if timeout --signal=TERM --kill-after=10s 900s \
        env ROBINHOOD_DATA_DIR="$workspace/datadirs/fullgame_linux" \
        "$runner" --no-auto-dump "$trace" > "$run_log" 2>&1
    then
        marker_count=$(grep -Fxc -- "$exact_eof_marker" "$run_log" || true)
        if [[ "$marker_count" == 1 ]]; then
            runner_status=0
        else
            runner_status=$integrity_status
        fi
    else
        runner_status=$?
    fi
    if mv -f -- "$run_log" "$log"; then
        write_status "$status" "$runner_status"
    else
        printf 'error: unable to publish parity log: %s\n' "$log" >&2
        rm -f -- "$run_log"
    fi
    exec {runner_slot_fd}>&-
    exec {trace_lock_fd}>&-
done
