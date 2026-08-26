#!/usr/bin/env bash
set -euo pipefail

repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
test_root=$(mktemp -d "$repo/.codex-tmp/corpus-work-supervisor-test.XXXXXX")
cleanup() {
    rm -rf -- "$test_root"
}
trap cleanup EXIT

db=$test_root/state.sqlite3
python3 "$repo/scripts/replay_state_db.py" init "$db" >/dev/null
python3 "$repo/scripts/replay_state_db.py" activate-corpus "$db" test/corpus \
    --expected 1 --location-host test --location-path "$test_root/corpus" >/dev/null

lease_state() {
    python3 - "$db" <<'PY'
import sqlite3
import sys

connection = sqlite3.connect(sys.argv[1])
row = connection.execute(
    "SELECT state FROM corpus_work_leases ORDER BY corpus_work_id DESC LIMIT 1"
).fetchone()
print(row[0] if row else "missing")
PY
}

CORPUS_WORK_HEARTBEAT_SECONDS=1 \
    "$repo/scripts/run_corpus_work_supervised.sh" \
    "$db" test/corpus convert worker-success test "$test_root/audit-success" \
    60 success -- bash -c 'sleep 2' >/dev/null
[[ "$(lease_state)" == completed ]]

set +e
CORPUS_WORK_HEARTBEAT_SECONDS=1 \
    "$repo/scripts/run_corpus_work_supervised.sh" \
    "$db" test/corpus convert worker-resource test "$test_root/audit-resource" \
    60 resource -- bash -c 'exit 137' >/dev/null
resource_rc=$?
set -e
[[ "$resource_rc" == 137 && "$(lease_state)" == failed ]]

child_pid_file=$test_root/child.pid
CORPUS_WORK_HEARTBEAT_SECONDS=1 \
    "$repo/scripts/run_corpus_work_supervised.sh" \
    "$db" test/corpus convert worker-signal test "$test_root/audit-signal" \
    60 signal -- bash -c 'printf "%s\n" "$$" >"$1"; exec sleep 300' bash "$child_pid_file" \
    >/dev/null &
supervisor_pid=$!
for attempt in $(seq 1 50); do
    [[ -s "$child_pid_file" ]] && break
    sleep 0.1
done
[[ -s "$child_pid_file" ]]
child_pid=$(<"$child_pid_file")
kill -TERM "$supervisor_pid"
set +e
wait "$supervisor_pid"
signal_rc=$?
set -e
[[ "$signal_rc" == 143 && "$(lease_state)" == abandoned ]]
if kill -0 "$child_pid" 2>/dev/null; then
    printf 'child process survived supervisor SIGTERM: %s\n' "$child_pid" >&2
    exit 1
fi

printf 'corpus work supervisor tests passed\n'
