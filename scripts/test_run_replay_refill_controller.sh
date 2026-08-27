#!/usr/bin/env bash
set -euo pipefail

repo=$(cd -- "$(dirname -- "$0")/.." && pwd)
test_root=$(mktemp -d "$repo/.agent-debug/test-replay-refill.XXXXXX")
trap 'rm -rf -- "$test_root"' EXIT
mkdir -p "$test_root/bin" "$test_root/workspace/scripts" "$test_root/bundle" \
    "$test_root/audit" "$test_root/tmux"

python3 - "$test_root/state.sqlite3" <<'PY'
import sqlite3,sys
db=sqlite3.connect(sys.argv[1])
db.executescript("""
create table schema_meta(key text primary key,value text);
create table corpora(corpus_id integer primary key,logical_root text);
create table replays(replay_id integer primary key,corpus_id integer);
create table runners(runner_id integer primary key,bundle_trust_sha256 text);
create table work_items(work_id integer primary key,operation text,replay_id integer,runner_id integer);
create table work_completions(work_id integer primary key);
insert into schema_meta values('current_runner_trust_sha256','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa');
insert into corpora values(1,'corpus');
insert into runners values(1,'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa');
""")
db.commit()
PY

cat >"$test_root/bin/tmux" <<'SH'
#!/usr/bin/env bash
case ${1:-} in
    has-session) exit 0 ;;
    kill-session) exit 1 ;;
    *) exit 0 ;;
esac
SH
cat >"$test_root/workspace/scripts/state.py" <<'PY'
#!/usr/bin/env python3
print('{}')
PY
cat >"$test_root/workspace/scripts/worker.sh" <<'SH'
#!/usr/bin/env bash
exit 0
SH
chmod +x "$test_root/bin/tmux" "$test_root/workspace/scripts/state.py" \
    "$test_root/workspace/scripts/worker.sh"

set +e
PATH="$test_root/bin:$PATH" \
REPLAY_REFILL_MAX_LANES=1 REPLAY_REFILL_MIN_LANES=1 \
REPLAY_REFILL_POLL_SECONDS=1 REPLAY_REFILL_ENQUEUE_SECONDS=60 \
REPLAY_REFILL_STATE_TOOL="$test_root/workspace/scripts/state.py" \
REPLAY_REFILL_WORKER_SCRIPT="$test_root/workspace/scripts/worker.sh" \
REPLAY_REFILL_TMUX_TMPDIR="$test_root/tmux" \
timeout 3s "$repo/scripts/run_replay_refill_controller.sh" \
    "$test_root/workspace" "$test_root/state.sqlite3" "$test_root/bundle" \
    aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
    bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb \
    corpus "$test_root/audit" refill-race
status=$?
set -e

[[ $status == 124 ]]
grep -q 'heartbeat target=1 desired=0 pending=0' \
    "$test_root/audit/refill-controller.log"
printf 'replay refill controller tests passed\n'
