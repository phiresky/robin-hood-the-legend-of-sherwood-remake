#!/usr/bin/env bash
set -euo pipefail

# Run one corpus-wide operation under an authoritative SQLite lease.  The
# wrapped command is responsible for its own artifact-level crash recovery;
# this supervisor only keeps the corpus lease truthful and terminal.

if (( $# < 10 )) || [[ "$9" != -- ]]; then
    printf 'usage: %s DATABASE LOGICAL_ROOT OPERATION WORKER_ID HOST AUDIT_PATH LEASE_SECONDS DETAIL -- COMMAND [ARG ...]\n' "$0" >&2
    exit 2
fi

database=$1
logical_root=$2
operation=$3
worker_id=$4
host=$5
audit_path=$6
lease_seconds=$7
detail=$8
shift 9

[[ "$lease_seconds" =~ ^[0-9]+$ ]] && (( lease_seconds >= 60 )) \
    || { printf 'error: lease seconds must be an integer of at least 60\n' >&2; exit 2; }
(( $# > 0 )) || { printf 'error: wrapped command is required\n' >&2; exit 2; }

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
state=abandoned
finish_detail='supervisor exited before wrapped command completed'
heartbeat_pid=
claim_token=
child_pid=

terminate_child_group() {
    local attempt
    [[ -n "$child_pid" ]] || return 0
    kill -TERM -- "-$child_pid" 2>/dev/null || true
    for attempt in $(seq 1 30); do
        kill -0 "$child_pid" 2>/dev/null || { child_pid=; return 0; }
        sleep 1
    done
    kill -KILL -- "-$child_pid" 2>/dev/null || true
    wait "$child_pid" 2>/dev/null || true
    child_pid=
}

handle_signal() {
    local signal=$1 rc=$2
    finish_detail="supervisor interrupted by SIG$signal"
    terminate_child_group
    exit "$rc"
}

finish_lease() {
    local rc=$?
    trap - EXIT INT TERM HUP
    if [[ -n "$heartbeat_pid" ]]; then
        kill "$heartbeat_pid" 2>/dev/null || true
        wait "$heartbeat_pid" 2>/dev/null || true
    fi
    if [[ -n "$claim_token" ]]; then
        python3 "$script_dir/replay_state_db.py" finish-corpus-work \
            "$database" "$claim_token" "$state" --detail "$finish_detail" \
            || printf 'error: could not finish corpus-work lease %s\n' "$claim_token" >&2
    fi
    exit "$rc"
}
trap finish_lease EXIT
trap 'handle_signal INT 130' INT
trap 'handle_signal TERM 143' TERM
trap 'handle_signal HUP 129' HUP

claim_json=$(python3 "$script_dir/replay_state_db.py" claim-corpus-work \
    "$database" "$logical_root" "$operation" "$worker_id" \
    --host "$host" --audit-path "$audit_path" --detail "$detail" \
    --lease-seconds "$lease_seconds")
claim_token=$(python3 -c 'import json, sys; print(json.load(sys.stdin)["claim_token"])' \
    <<<"$claim_json")
printf '%s\n' "$claim_json"

heartbeat_seconds=${CORPUS_WORK_HEARTBEAT_SECONDS:-$((lease_seconds / 3))}
[[ "$heartbeat_seconds" =~ ^[0-9]+$ ]] \
    && (( heartbeat_seconds >= 1 && heartbeat_seconds < lease_seconds )) \
    || { printf 'error: heartbeat seconds must be positive and less than the lease\n' >&2; exit 2; }
owner_pid=$$
(
    while sleep "$heartbeat_seconds"; do
        if ! python3 "$script_dir/replay_state_db.py" renew-corpus-work \
            "$database" "$claim_token" --lease-seconds "$lease_seconds"
        then
            kill -TERM "$owner_pid" 2>/dev/null || true
            exit 1
        fi
    done
) &
heartbeat_pid=$!

set +e
setsid -- "$@" &
child_pid=$!
wait "$child_pid"
command_rc=$?
child_pid=
set -e
if (( command_rc == 0 )); then
    state=completed
    finish_detail="wrapped command completed: $detail"
else
    state=failed
    finish_detail="wrapped command exited $command_rc: $detail"
fi
exit "$command_rc"
