#!/usr/bin/env bash
set -euo pipefail

# Keep a digest-bound distributed replay pool supplied as conversion publishes
# new native artifacts.  SQLite work keys and renewable claims prevent duplicate
# execution across this controller and any independent workers.

if (( $# != 8 )); then
    printf 'usage: %s WORKSPACE DATABASE BUNDLE TRUST_SHA RUNNER_SHA CORPUS AUDIT LANE_PREFIX\n' "$0" >&2
    exit 2
fi

workspace=$(realpath -e -- "$1")
database=$(realpath -e -- "$2")
bundle=$(realpath -e -- "$3")
trust_sha=${4,,}
runner_sha=${5,,}
corpus=$6
audit=$7
lane_prefix=$8

max_lanes=${REPLAY_REFILL_MAX_LANES:-20}
min_lanes=${REPLAY_REFILL_MIN_LANES:-8}
poll_seconds=${REPLAY_REFILL_POLL_SECONDS:-10}
enqueue_seconds=${REPLAY_REFILL_ENQUEUE_SECONDS:-120}
scale_up_available_kib=${REPLAY_REFILL_SCALE_UP_AVAILABLE_KIB:-41943040}
scale_down_available_kib=${REPLAY_REFILL_SCALE_DOWN_AVAILABLE_KIB:-25165824}
state_tool=${REPLAY_REFILL_STATE_TOOL:-$workspace/scripts/replay_state_db.py}
worker_script=${REPLAY_REFILL_WORKER_SCRIPT:-$workspace/scripts/run_distributed_replay_worker.sh}
tmux_tmpdir=${REPLAY_REFILL_TMUX_TMPDIR:-$workspace/tmux}
timeout_seconds=${REPLAY_REFILL_TIMEOUT_SECONDS:-1800}

[[ "$trust_sha" =~ ^[0-9a-f]{64}$ && "$runner_sha" =~ ^[0-9a-f]{64}$ ]] || exit 2
[[ "$corpus" != /* && "$corpus" != *..* && "$corpus" != *$'\n'* ]] || exit 2
[[ "$lane_prefix" =~ ^[A-Za-z0-9_.:-]+$ ]] || exit 2
for number in "$max_lanes" "$min_lanes" "$poll_seconds" "$enqueue_seconds" \
    "$scale_up_available_kib" "$scale_down_available_kib" "$timeout_seconds"; do
    [[ "$number" =~ ^[0-9]+$ ]] || exit 2
done
(( min_lanes >= 0 && max_lanes > 0 && max_lanes >= min_lanes && poll_seconds > 0 \
    && enqueue_seconds >= poll_seconds \
    && scale_up_available_kib > scale_down_available_kib )) || exit 2
[[ -x "$state_tool" && -x "$worker_script" ]] || exit 2

mkdir -p -- "$audit" "$tmux_tmpdir"
log="$audit/refill-controller.log"
exec >>"$log" 2>&1

timestamp() { date -u +%Y-%m-%dT%H:%M:%SZ; }
session_name() { printf '%s-%02d' "$lane_prefix" "$1"; }

current_trust() {
    python3 - "$database" <<'PY'
import sqlite3,sys
db=sqlite3.connect(sys.argv[1])
row=db.execute("select value from schema_meta where key='current_runner_trust_sha256'").fetchone()
print("" if row is None else row[0])
PY
}

pending_work() {
    python3 - "$database" "$corpus" "$trust_sha" <<'PY'
import sqlite3,sys
db=sqlite3.connect(sys.argv[1])
row=db.execute(
    """select count(*) from work_items wi
       join replays r using(replay_id)
       join corpora c using(corpus_id)
       join runners ru using(runner_id)
       left join work_completions done using(work_id)
       where wi.operation='replay' and done.work_id is null
         and c.logical_root=? and ru.bundle_trust_sha256=?""",
    (sys.argv[2],sys.argv[3]),
).fetchone()
print(row[0])
PY
}

enqueue_ready() {
    python3 "$state_tool" enqueue-corpus-replays "$database" "$corpus" \
        --runner-trust "$trust_sha" --priority 250 --workspace "$workspace"
}

start_lane() {
    local lane=$1 session command
    session=$(session_name "$lane")
    printf -v command '%q ' env \
        "DISTRIBUTED_REPLAY_STATE_DB=$database" \
        "DISTRIBUTED_REPLAY_TIMEOUT_SECONDS=$timeout_seconds" \
        "$worker_script" remote "$workspace" "$bundle" "$trust_sha" "$runner_sha" \
        "$audit" "$corpus" "remote:refill:$lane_prefix:$lane" none
    TMUX_TMPDIR="$tmux_tmpdir" tmux new-session -d -s "$session" \
        "cd $(printf '%q' "$workspace") && $command"
    printf '%s started lane=%s session=%s\n' "$(timestamp)" "$lane" "$session"
}

stop_lane() {
    local lane=$1 session
    session=$(session_name "$lane")
    if TMUX_TMPDIR="$tmux_tmpdir" tmux has-session -t "$session" 2>/dev/null; then
        # A finite worker may finish after has-session and before kill-session.
        # That disappearance is the desired state, not a controller failure.
        if TMUX_TMPDIR="$tmux_tmpdir" tmux kill-session -t "$session" 2>/dev/null; then
            printf '%s drained lane=%s session=%s\n' "$(timestamp)" "$lane" "$session"
        fi
    fi
}

printf '%s controller-start corpus=%s min=%s max=%s\n' \
    "$(timestamp)" "$corpus" "$min_lanes" "$max_lanes"
last_enqueue=0
enqueue_pid=
enqueue_output=
target=$min_lanes
while true; do
    configured_trust=$(current_trust)
    if [[ "$configured_trust" != "$trust_sha" ]]; then
        printf '%s current-runner-changed configured=%s current=%s; exiting\n' \
            "$(timestamp)" "$trust_sha" "$configured_trust"
        exit 0
    fi
    if [[ -e "$audit/STOP.env" || -e "$audit/BATCH_FATAL.env" ]]; then
        printf '%s audit-integrity-stop; exiting\n' "$(timestamp)"
        exit 1
    fi

    now=$(date +%s)
    if [[ -n "$enqueue_pid" ]] && ! kill -0 "$enqueue_pid" 2>/dev/null; then
        if wait "$enqueue_pid"; then
            printf '%s enqueue %s\n' "$(timestamp)" "$(<"$enqueue_output")"
        else
            printf '%s enqueue-failed %s\n' "$(timestamp)" "$(<"$enqueue_output")"
        fi
        rm -f -- "$enqueue_output"
        enqueue_pid=
        enqueue_output=
    fi
    if [[ -z "$enqueue_pid" ]] && (( now - last_enqueue >= enqueue_seconds )); then
        enqueue_output=$(mktemp "$audit/.enqueue.XXXXXX")
        enqueue_ready >"$enqueue_output" 2>&1 &
        enqueue_pid=$!
        last_enqueue=$now
        printf '%s enqueue-started pid=%s\n' "$(timestamp)" "$enqueue_pid"
    fi

    available_kib=$(awk '/^MemAvailable:/ {print $2}' /proc/meminfo)
    if (( available_kib < scale_down_available_kib )); then
        target=$min_lanes
    elif (( available_kib >= scale_up_available_kib )); then
        target=$max_lanes
    fi

    pending=$(pending_work)
    desired=$target
    if (( pending < desired )); then desired=$pending; fi

    for ((lane=desired; lane<max_lanes; lane++)); do stop_lane "$lane"; done
    for ((lane=0; lane<desired; lane++)); do
        session=$(session_name "$lane")
        if ! TMUX_TMPDIR="$tmux_tmpdir" tmux has-session -t "$session" 2>/dev/null; then
            start_lane "$lane"
        fi
    done
    printf '%s heartbeat target=%s desired=%s pending=%s available_kib=%s\n' \
        "$(timestamp)" "$target" "$desired" "$pending" "$available_kib"
    sleep "$poll_seconds"
done
