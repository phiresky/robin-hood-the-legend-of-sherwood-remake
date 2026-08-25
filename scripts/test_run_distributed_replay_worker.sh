#!/usr/bin/env bash
set -euo pipefail

workspace=$(realpath -e -- "${0%/*}/..")
script=${DISTRIBUTED_REPLAY_SCRIPT:-$workspace/scripts/run_distributed_replay_worker.sh}
tool=$workspace/scripts/replay_state_db.py
mkdir -p -- "$workspace/.agent-debug"
test_root=$(mktemp -d "$workspace/.agent-debug/distributed-replay-test.XXXXXX")
cleanup() { rm -rf -- "$test_root"; }
trap cleanup EXIT

remote_bundle="$test_root/remote-bundle"
local_bundle="$test_root/local-bundle"
mkdir -p -- "$remote_bundle/lib"
printf '%s\n' \
    '#include <stdio.h>' \
    '#include <stdlib.h>' \
    '#include <string.h>' \
    '#include <unistd.h>' \
    'int main(int argc, char **argv) {' \
    '  const char *mode=getenv("FAKE_RUNNER_MODE");' \
    '  if (mode && strcmp(mode,"sleep")==0) sleep(30);' \
    '  if (mode && strcmp(mode,"nomarker")==0) return 0;' \
    '  if (mode && (strcmp(mode,"fail")==0 ||' \
    '      (strcmp(mode,"fail-first")==0 && argc > 1 && strstr(argv[argc-1],"replay-001")))) {' \
    '    fputs("first parity divergence after frame 42 (1 difference):\n", stderr);' \
    '    return 101;' \
    '  }' \
    '  puts("parity trace matched every recorded frame");' \
    '  return 0;' \
    '}' >"$test_root/fake_runner.c"
cc -O0 -o "$remote_bundle/original_parity_replay" "$test_root/fake_runner.c"
while IFS= read -r dependency; do
    cp -- "$dependency" "$remote_bundle/lib/${dependency##*/}"
done < <(ldd "$remote_bundle/original_parity_replay" \
    | sed -n -e 's/.*=> \(\/[^ ]*\).*/\1/p' -e 's/^[[:space:]]*\(\/[^ ]*ld-linux[^ ]*\).*/\1/p' \
    | LC_ALL=C sort -u)
loader="$remote_bundle/lib/ld-linux-x86-64.so.2"
printf '%s\n' '#!/usr/bin/env bash' \
    'here=$(CDPATH= cd -- "${0%/*}" && pwd)' \
    'exec "$here/lib/ld-linux-x86-64.so.2" --library-path "$here/lib" "$here/original_parity_replay" "$@"' \
    >"$remote_bundle/original_parity_replay.remote"
chmod +x "$remote_bundle/original_parity_replay.remote"
"$loader" --library-path "$remote_bundle/lib" --list \
    "$remote_bundle/original_parity_replay" >"$remote_bundle/LOADER_LIST.txt"
printf 'NATIVE_CONVERSION_PROTOCOL=2\n' >"$remote_bundle/PROVENANCE.txt"
(cd -- "$remote_bundle" && find lib -type f -print0 | LC_ALL=C sort -z \
    | xargs -0 sha256sum >LIB_SHA256SUMS)
(cd -- "$remote_bundle" && sha256sum LIB_SHA256SUMS LOADER_LIST.txt PROVENANCE.txt \
    original_parity_replay original_parity_replay.remote >SHA256SUMS)
main_manifest_sha=$(sha256sum "$remote_bundle/SHA256SUMS"); main_manifest_sha=${main_manifest_sha%% *}
lib_manifest_sha=$(sha256sum "$remote_bundle/LIB_SHA256SUMS"); lib_manifest_sha=${lib_manifest_sha%% *}
trust=$(printf 'schema16-runner-bundle-v1\nSHA256SUMS=%s\nLIB_SHA256SUMS=%s\n' \
    "$main_manifest_sha" "$lib_manifest_sha" | sha256sum); trust=${trust%% *}
raw_sha=$(sha256sum "$remote_bundle/original_parity_replay"); raw_sha=${raw_sha%% *}
wrapper_sha=$(sha256sum "$remote_bundle/original_parity_replay.remote"); wrapper_sha=${wrapper_sha%% *}
cp -a -- "$remote_bundle" "$local_bundle"
cmp -s -- "$remote_bundle/SHA256SUMS" "$local_bundle/SHA256SUMS"
cmp -s -- "$remote_bundle/LOADER_LIST.txt" "$local_bundle/LOADER_LIST.txt"

fake_ssh="$test_root/fake-ssh"
printf '%s\n' '#!/usr/bin/env bash' \
    'set -euo pipefail' \
    'if [[ ${1:-} == -F ]]; then shift 2; fi' \
    'shift' \
    'command=$1' \
    'if [[ "$command" == *" renew-work "* && -e ${FAKE_SSH_FAIL_RENEW:-/nonexistent} ]]; then exit 255; fi' \
    'if [[ "$command" == *" import-result "* && -e ${FAKE_SSH_FAIL_IMPORT:-/nonexistent} ]]; then exit 255; fi' \
    'exec bash -c "$command"' >"$fake_ssh"
chmod +x "$fake_ssh"

setup_case() {
    local name=$1
    case_root="$test_root/$name"
    remote_root="$case_root/remote"
    local_root="$case_root/local"
    remote_audit="$remote_root/audits/distributed"
    database="$remote_root/parity-save-replays/replay-state.sqlite3"
    corpus='parity-save-replays/60s-random-input/test-seed3'
    logical="$corpus/traces/save/replay-001-session-0001.jsonl.zst"
    marker="$corpus/traces/save/replay-001.complete"
    mkdir -p -- "$remote_root/${logical%/*}" "$remote_audit" \
        "$local_root/datadirs/fullgame_linux" "$remote_root/datadirs/fullgame_linux" \
        "$remote_root/parity-save-replays"
    printf 'native\nRHPRTRACEFOOTER!12345678901234567890' \
        >"$remote_root/$logical.parity.bitcode.zst"
    printf 'complete\n' >"$remote_root/$marker"
    native_sha=$(sha256sum "$remote_root/$logical.parity.bitcode.zst"); native_sha=${native_sha%% *}
    python3 - "$tool" "$database" "$corpus" "$remote_root/$corpus" "$logical" "$marker" \
        "$trust" "$raw_sha" "$main_manifest_sha" "$lib_manifest_sha" "$wrapper_sha" <<'PY'
import importlib.util,sys
from pathlib import Path
spec=importlib.util.spec_from_file_location("db",sys.argv[1]); db=importlib.util.module_from_spec(spec); spec.loader.exec_module(db)
connection=db.connect(Path(sys.argv[2]))
with connection:
    corpus_id=db.upsert_corpus(connection,sys.argv[3],trace_schema=16,expected=1)
    connection.execute("UPDATE corpora SET corpus_path=?,corpus_status='active' WHERE corpus_id=?",(sys.argv[4],corpus_id))
    db.upsert_replay(connection,sys.argv[5],sys.argv[6])
    db.upsert_runner(connection,{"RUNNER_BUNDLE_TRUST_SHA256":sys.argv[7],"RUNNER_RAW_SHA256":sys.argv[8],"RUNNER_BUNDLE_MANIFEST_SHA256":sys.argv[9],"RUNNER_LIB_MANIFEST_SHA256":sys.argv[10],"RUNNER_WRAPPER_SHA256":sys.argv[11]})
connection.close()
PY
    python3 "$tool" enqueue-corpus-replays "$database" "$corpus" \
        --runner-trust "$trust" --priority 100 >/dev/null
}

add_second_replay() {
    local logical2="$corpus/traces/save/replay-002-session-0001.jsonl.zst"
    local marker2="$corpus/traces/save/replay-002.complete"
    printf 'native-2\nRHPRTRACEFOOTER!12345678901234567890' \
        >"$remote_root/$logical2.parity.bitcode.zst"
    printf 'complete\n' >"$remote_root/$marker2"
    python3 - "$tool" "$database" "$logical2" "$marker2" <<'PY'
import importlib.util,sys
from pathlib import Path
spec=importlib.util.spec_from_file_location("db",sys.argv[1]); db=importlib.util.module_from_spec(spec); spec.loader.exec_module(db)
connection=db.connect(Path(sys.argv[2]))
with connection:
    db.upsert_replay(connection,sys.argv[3],sys.argv[4])
    connection.execute("UPDATE corpora SET expected_replays=2 WHERE logical_root=?",(sys.argv[3].split("/traces/",1)[0],))
connection.close()
PY
    python3 "$tool" enqueue-corpus-replays "$database" "$corpus" \
        --runner-trust "$trust" --priority 100 >/dev/null
}

run_worker() {
    DISTRIBUTED_REPLAY_ONESHOT=${TEST_ONESHOT:-1} \
    DISTRIBUTED_REPLAY_LEASE_SECONDS=6 \
    DISTRIBUTED_REPLAY_HEARTBEAT_SECONDS=1 \
    DISTRIBUTED_REPLAY_TIMEOUT_SECONDS=20 \
    DISTRIBUTED_REPLAY_REMOTE_WORKSPACE="$remote_root" \
    DISTRIBUTED_REPLAY_STATE_DB="$database" \
    DISTRIBUTED_REPLAY_STATE_TOOL="$tool" \
    DISTRIBUTED_REPLAY_REMOTE_SCRIPT="$script" \
    DISTRIBUTED_REPLAY_SSH="$fake_ssh" \
    "$script" local "$local_root" "$local_bundle" "$trust" "$raw_sha" \
        "$remote_audit" "$corpus" "local:$1" fake-host
}

run_remote_worker() {
    DISTRIBUTED_REPLAY_ONESHOT=1 \
    DISTRIBUTED_REPLAY_LEASE_SECONDS=6 \
    DISTRIBUTED_REPLAY_HEARTBEAT_SECONDS=1 \
    DISTRIBUTED_REPLAY_TIMEOUT_SECONDS=20 \
    DISTRIBUTED_REPLAY_REMOTE_WORKSPACE="$remote_root" \
    DISTRIBUTED_REPLAY_STATE_DB="$database" \
    DISTRIBUTED_REPLAY_STATE_TOOL="$tool" \
    DISTRIBUTED_REPLAY_REMOTE_SCRIPT="$script" \
    "$script" remote "$remote_root" "$remote_bundle" "$trust" "$raw_sha" \
        "$remote_audit" "$corpus" "remote:$1" -
}

setup_case exact
FAKE_RUNNER_MODE=exact run_worker exact
python3 - "$database" <<'PY'
import sqlite3,sys
c=sqlite3.connect(sys.argv[1])
assert c.execute("select count(*) from work_completions").fetchone()[0] == 1
assert c.execute("select outcome from replay_runs").fetchone()[0] == "exact_eof"
PY
[[ $(find "$remote_audit/results" -mindepth 1 -maxdepth 1 -type d | wc -l) == 1 ]]

setup_case remote_exact
FAKE_RUNNER_MODE=exact run_remote_worker exact
python3 - "$database" <<'PY'
import sqlite3,sys
c=sqlite3.connect(sys.argv[1])
assert c.execute("select count(*) from work_completions").fetchone()[0] == 1
assert c.execute("select outcome from replay_runs").fetchone()[0] == "exact_eof"
PY

setup_case recovery_namespace
# This is the pre-fix scratch location for the same reusable worker ID. Its
# nonzero result represents retained evidence from an older runner rollout.
# The current trust/audit namespace must neither upload nor interpret it.
legacy_result="$local_root/.agent-debug/distributed-replay-worker/local:namespace/audit/results/$(printf old-runner | sha256sum | cut -d' ' -f1)"
mkdir -p -- "$legacy_result"
printf '101\n' >"$legacy_result/status"
other_trust=$(printf '%064d' 0 | tr 0 f)
audit_identity=$(printf 'distributed-replay-worker-audit-v1\nAUDIT=%s\nCORPUS=%s\n' \
    "$remote_audit" "$corpus" | sha256sum); audit_identity=${audit_identity%% *}
other_runner_result="$local_root/.agent-debug/distributed-replay-worker/$other_trust/$audit_identity/local:namespace/audit/results/$(printf other-runner | sha256sum | cut -d' ' -f1)"
mkdir -p -- "$other_runner_result"
printf '101\n' >"$other_runner_result/status"
FAKE_RUNNER_MODE=exact run_worker namespace
[[ ! -e "$remote_audit/STOP.env" ]]
[[ -f "$legacy_result/status" ]]
[[ -f "$other_runner_result/status" ]]
[[ $(find "$remote_audit/results" -mindepth 1 -maxdepth 1 -type d | wc -l) == 1 ]]
python3 - "$database" <<'PY'
import sqlite3,sys
c=sqlite3.connect(sys.argv[1])
assert c.execute("select count(*) from work_completions").fetchone()[0] == 1
assert c.execute("select outcome from replay_runs").fetchone()[0] == "exact_eof"
PY

setup_case failure
add_second_replay
TEST_ONESHOT=0 FAKE_RUNNER_MODE=fail-first run_worker failure
[[ ! -e "$remote_audit/STOP.env" ]]
python3 - "$database" <<'PY'
import sqlite3,sys
c=sqlite3.connect(sys.argv[1])
assert c.execute("select count(*) from work_completions").fetchone()[0] == 2
assert c.execute("select outcome,count(*) from replay_runs group by outcome order by outcome").fetchall() == [("exact_eof",1),("mismatch",1)]
PY

setup_case integrity
set +e
FAKE_RUNNER_MODE=nomarker run_worker integrity
status=$?
set -e
[[ $status == 1 && -f "$remote_audit/STOP.env" ]]
python3 - "$database" <<'PY'
import sqlite3,sys
c=sqlite3.connect(sys.argv[1])
assert c.execute("select count(*) from work_completions").fetchone()[0] == 1
assert c.execute("select outcome from replay_runs").fetchone()[0] == "integrity_error"
PY

setup_case lease_loss
renew_failure="$case_root/fail-renew"
: >"$renew_failure"
set +e
FAKE_RUNNER_MODE=sleep FAKE_SSH_FAIL_RENEW="$renew_failure" run_worker lease
status=$?
set -e
[[ $status == 3 && -f "$remote_audit/STOP.env" ]]
python3 - "$database" <<'PY'
import sqlite3,sys
c=sqlite3.connect(sys.argv[1])
assert c.execute("select count(*) from work_completions").fetchone()[0] == 0
assert c.execute("select outcome from replay_runs").fetchone()[0] == "aborted"
PY
[[ $(find "$remote_audit/attempts" -mindepth 1 -maxdepth 1 -type d | wc -l) == 1 ]]

setup_case ssh_recovery
import_failure="$case_root/fail-import"
: >"$import_failure"
set +e
FAKE_RUNNER_MODE=exact FAKE_SSH_FAIL_IMPORT="$import_failure" run_worker recovery
status=$?
set -e
[[ $status == 3 ]]
python3 - "$database" <<'PY'
import sqlite3,sys
c=sqlite3.connect(sys.argv[1])
assert c.execute("select count(*) from work_completions").fetchone()[0] == 0
assert c.execute("select count(*) from replay_runs").fetchone()[0] == 0
PY
rm -- "$import_failure"
set +e
FAKE_RUNNER_MODE=fail run_worker recovery
status=$?
set -e
[[ $status == 1 ]]
python3 - "$database" <<'PY'
import sqlite3,sys
c=sqlite3.connect(sys.argv[1])
assert c.execute("select count(*) from work_completions").fetchone()[0] == 0
assert c.execute("select count(*) from replay_runs").fetchone()[0] == 1
assert c.execute("select outcome from replay_runs").fetchone()[0] == "exact_eof"
PY
[[ -e "$remote_audit/STOP.env" ]]

printf 'distributed replay worker tests passed\n'
