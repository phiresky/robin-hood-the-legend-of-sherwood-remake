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
mkdir -p "$audit_dir/logs" "$audit_dir/status"

concurrency=${PARITY_SWEEP_CONCURRENCY:-$shards}
if [[ ! "$concurrency" =~ ^[1-9][0-9]*$ ]]; then
    printf 'error: PARITY_SWEEP_CONCURRENCY must be a positive integer\n' >&2
    exit 2
fi
mkdir -p "$audit_dir/.runner-slots"

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
        for ((slot = 0; slot < concurrency; slot += 1)); do
            exec {runner_slot_fd}>"$audit_dir/.runner-slots/$slot.lock"
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

mapfile -t traces < "$snapshot"
for ((index = shard; index < ${#traces[@]}; index += shards)); do
    trace=${traces[index]}
    relative=${trace#"$corpus_dir"/}
    key=${relative//\//__}
    log="$audit_dir/logs/$key.log"
    status="$audit_dir/status/$key.status"

    [[ -f "$status" ]] && continue
    if [[ ! -f "$trace" ]]; then
        write_status "$status" missing
        continue
    fi

    acquire_runner_slot
    if timeout --signal=TERM --kill-after=10s 900s \
        env ROBINHOOD_DATA_DIR="$workspace/datadirs/fullgame_linux" \
        "$runner" --no-auto-dump "$trace" > "$log" 2>&1
    then
        marker_count=$(grep -Fxc -- "$exact_eof_marker" "$log" || true)
        if [[ "$marker_count" == 1 ]]; then
            write_status "$status" 0
        else
            write_status "$status" "$integrity_status"
        fi
    else
        runner_status=$?
        write_status "$status" "$runner_status"
    fi
    exec {runner_slot_fd}>&-
done
