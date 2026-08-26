#!/usr/bin/env bash
set -euo pipefail

workspace=$(pwd)
script=${INCREMENTAL_EOF_SCRIPT:-$workspace/scripts/run_incremental_eof_checks.sh}
test_root=$(mktemp -d "$workspace/.codex-tmp/incremental-eof-test.XXXXXX")
trap 'rm -rf -- "$test_root"' EXIT
export REPLAY_STATE_DB="$test_root/replay-state.sqlite3"

fake_tools="$test_root/tools"
mkdir -p "$fake_tools"
cat >"$fake_tools/nice" <<'TOOL'
#!/usr/bin/env bash
[[ "$1" == -n && "$2" == 19 ]] || exit 90
shift 2
exec "$@"
TOOL
cat >"$fake_tools/ionice" <<'TOOL'
#!/usr/bin/env bash
[[ "$1" == -c && "$2" == 3 ]] || exit 91
shift 2
exec "$@"
TOOL
cat >"$fake_tools/mktemp" <<'TOOL'
#!/usr/bin/env bash
set -euo pipefail
output=$(/usr/bin/mktemp "$@")
printf '%s\n' "$output"
if [[ "$*" == *'.runner.log.tmp.'* \
    && -n "${TEST_PHASE_FLIP_ON_RUNNER_MKTEMP:-}" ]]
then
    printf 'PHASE=drain-and-verify-seed3\n' \
        >"$TEST_PHASE_FLIP_ON_RUNNER_MKTEMP"
fi
TOOL
cat >"$fake_tools/sha256sum" <<'TOOL'
#!/usr/bin/env bash
set -euo pipefail
if [[ -n "${TEST_SHA_DELAY_PATTERN:-}" ]]; then
    for argument in "$@"; do
        if [[ $argument == *"$TEST_SHA_DELAY_PATTERN"* ]]; then
            [[ -z "${TEST_SHA_DELAY_STARTED:-}" ]] || : >"$TEST_SHA_DELAY_STARTED"
            while [[ ! -e "${TEST_SHA_DELAY_RELEASE:?}" ]]; do sleep 0.05; done
            break
        fi
    done
fi
exec /usr/bin/sha256sum "$@"
TOOL
chmod +x "$fake_tools/nice" "$fake_tools/ionice" "$fake_tools/mktemp" \
    "$fake_tools/sha256sum"

make_bundle() {
    local root=$1
    mkdir -p "$root/lib"
    cat >"$root/original_parity_replay" <<'RUNNER'
#!/usr/bin/env bash
exit 0
RUNNER
    cat >"$root/original_parity_replay.remote" <<'RUNNER'
#!/usr/bin/env bash
set -euo pipefail
trace=${@: -1}
[[ -z "${TEST_INVOCATIONS:-}" ]] || printf '%s\n' "$trace" >>"$TEST_INVOCATIONS"
[[ -z "${TEST_STARTED_FILE:-}" ]] || : >"$TEST_STARTED_FILE"
active_marker=
if [[ -n "${TEST_ACTIVE_DIR:-}" ]]; then
    mkdir -p "$TEST_ACTIVE_DIR"
    active_marker="$TEST_ACTIVE_DIR/$$"
    : >"$active_marker"
    trap 'rm -f -- "$active_marker"' EXIT
    trap 'exit 143' TERM
    trap 'exit 129' HUP
fi
if [[ -n "${TEST_WAIT_FILE:-}" ]]; then
    [[ -z "${TEST_IGNORE_TERM:-}" ]] || trap '' TERM
    while [[ ! -e "$TEST_WAIT_FILE" ]]; do sleep 0.05; done
fi
if [[ -n "${TEST_MUTATE_NATIVE:-}" ]]; then
    printf 'changed\n' >>"$trace.parity.bitcode.zst"
fi
if [[ -n "${TEST_RUNNER_FAIL_PATTERN:-}" \
    && $trace == *"$TEST_RUNNER_FAIL_PATTERN"* ]]; then
    exit 7
fi
case "${TEST_RUNNER_MODE:-exact}" in
exact) printf '%s\n' 'parity trace matched every recorded frame' ;;
missing-eof) printf '%s\n' 'runner exited without marker' ;;
*) exit 7 ;;
esac
RUNNER
    printf 'NATIVE_CONVERSION_PROTOCOL=2\n' >"$root/PROVENANCE.txt"
    printf 'ld-linux.so => %s/lib/ld-linux-x86-64.so.2 (0x0)\n' "$root" \
        >"$root/LOADER_LIST.txt"
    printf 'lib\n' >"$root/lib/ld-linux-x86-64.so.2"
    chmod +x "$root/original_parity_replay" "$root/original_parity_replay.remote"
    (cd "$root" && sha256sum lib/ld-linux-x86-64.so.2 >LIB_SHA256SUMS \
        && sha256sum original_parity_replay original_parity_replay.remote \
            PROVENANCE.txt LOADER_LIST.txt LIB_SHA256SUMS >SHA256SUMS)
}

bundle="$test_root/bundle"
make_bundle "$bundle"
runner_sha=$(sha256sum "$bundle/original_parity_replay"); runner_sha=${runner_sha%% *}
main_sha=$(sha256sum "$bundle/SHA256SUMS"); main_sha=${main_sha%% *}
lib_sha=$(sha256sum "$bundle/LIB_SHA256SUMS"); lib_sha=${lib_sha%% *}
trust_sha=$(printf 'schema16-runner-bundle-v1\nSHA256SUMS=%s\nLIB_SHA256SUMS=%s\n' \
    "$main_sha" "$lib_sha" | sha256sum); trust_sha=${trust_sha%% *}
meminfo="$test_root/meminfo"
loadavg="$test_root/loadavg"
printf 'MemAvailable: 99999999 kB\n' >"$meminfo"
printf '1.00 0.00 0.00 1/1 1\n' >"$loadavg"

make_case() {
    local name=$1 root="$test_root/$1"
    mkdir -p "$root/campaign/traces/save" "$root/orchestrator" "$root/audit"
    printf 'PHASE=wait-seed3-natural-exit\n' >"$root/orchestrator/state.env"
    printf 'native-%s\nRHPRTRACEFOOTER!12345678901234567890' "$name" \
        >"$root/campaign/traces/save/replay-001-session-0001.jsonl.zst.parity.bitcode.zst"
    : >"$root/campaign/traces/save/replay-001.complete"
}

run_case() {
    local name=$1
    shift
    env PATH="$fake_tools:$PATH" INCREMENTAL_EOF_ONESHOT=1 \
        INCREMENTAL_EOF_MEMINFO_PATH="$meminfo" \
        INCREMENTAL_EOF_LOADAVG_PATH="$loadavg" \
        INCREMENTAL_EOF_NATIVE_LOCK_DIR="$test_root/native-locks" \
        INCREMENTAL_EOF_SLOT_DIR="$test_root/slots" \
        INCREMENTAL_EOF_OUTER_LOCK="$test_root/outer.lock" "$@" \
        "$script" "$workspace" "$bundle" "$trust_sha" "$runner_sha" \
        "$test_root/$name/orchestrator" "$test_root/$name/audit" \
        "$test_root/$name/campaign"
}

make_case exact
run_case exact
result=$(find "$test_root/exact/audit/results" -mindepth 1 -maxdepth 1 -type d)
[[ -n "$result" && "$(<"$result/status")" == 0 ]]
[[ "$(grep -Fxc 'parity trace matched every recorded frame' "$result/log")" == 1 ]]
grep -Fxq "RUNNER_RAW_SHA256=$runner_sha" "$result/attestation.env"
grep -Fxq "RUNNER_BUNDLE_TRUST_SHA256=$trust_sha" "$result/attestation.env"
[[ "$(sha256sum "$result/log" | cut -d' ' -f1)" \
    == "$(sed -n 's/^LOG_SHA256=//p' "$result/attestation.env")" ]]

# Exact authenticated proof reuse does not rerun. Tampering any sealed input
# makes resume fail closed rather than silently trusting status zero.
make_case reuse
invocations="$test_root/reuse.invocations"
run_case reuse TEST_INVOCATIONS="$invocations"
run_case reuse TEST_INVOCATIONS="$invocations"
[[ "$(wc -l <"$invocations")" == 1 ]]
result=$(find "$test_root/reuse/audit/results" -mindepth 1 -maxdepth 1 -type d)
printf 'tamper\n' >>"$result/log"
if run_case reuse TEST_INVOCATIONS="$invocations"; then
    printf 'test failure: tampered sealed proof was reused\n' >&2
    exit 1
fi
[[ "$(wc -l <"$invocations")" == 1 ]]

# A completed marker is mandatory; an unmarked native artifact is ignored.
make_case unmarked
rm "$test_root/unmarked/campaign/traces/save/replay-001.complete"
run_case unmarked
[[ -z "$(find "$test_root/unmarked/audit/results" -mindepth 1 -maxdepth 1 -type d -print -quit)" ]]

# Exit zero without one anchored EOF is a published integrity failure and stop.
make_case missing
if run_case missing TEST_RUNNER_MODE=missing-eof; then
    printf 'test failure: missing EOF was accepted\n' >&2
    exit 1
fi
result=$(find "$test_root/missing/audit/results" -mindepth 1 -maxdepth 1 -type d)
[[ "$(<"$result/status")" == integrity-eof-marker ]]
[[ -f "$test_root/missing/audit/STOP.env" ]]

# Native bytes changing during the run invalidate the tuple even at exact EOF.
make_case mutate
if run_case mutate TEST_MUTATE_NATIVE=1; then
    printf 'test failure: changed native bytes were accepted\n' >&2
    exit 1
fi
result=$(find "$test_root/mutate/audit/results" -mindepth 1 -maxdepth 1 -type d)
[[ "$(<"$result/status")" == integrity-native-changed ]]
pre=$(sed -n 's/^NATIVE_SHA256_PRE=//p' "$result/attestation.env")
post=$(sed -n 's/^NATIVE_SHA256_POST=//p' "$result/attestation.env")
[[ "$pre" != "$post" ]]

# A closed phase and a drain marker both stop admission without runner output.
make_case phase
printf 'PHASE=drain-and-verify-seed3\n' >"$test_root/phase/orchestrator/state.env"
run_case phase
[[ -z "$(find "$test_root/phase/audit/results" -mindepth 1 -maxdepth 1 -type d -print -quit)" ]]
grep -Fxq 'REASON=admission-closed' "$test_root/phase/audit/controller-finished.env"
make_case drain
: >"$test_root/drain/campaign/.capture.drain"
run_case drain
[[ -z "$(find "$test_root/drain/audit/results" -mindepth 1 -maxdepth 1 -type d -print -quit)" ]]

# Close admission during private-log allocation, after the earlier bundle/hash
# work. The final pre-setsid check must remove the private file and start no
# runner at all.
make_case prelaunch-close
prelaunch_invocations="$test_root/prelaunch.invocations"
run_case prelaunch-close \
    TEST_PHASE_FLIP_ON_RUNNER_MKTEMP="$test_root/prelaunch-close/orchestrator/state.env" \
    TEST_INVOCATIONS="$prelaunch_invocations"
[[ ! -e "$prelaunch_invocations" ]]
[[ -z "$(find "$test_root/prelaunch-close/audit/results" \
    -mindepth 1 -maxdepth 1 -type d -print -quit)" ]]
[[ -z "$(find "$test_root/prelaunch-close/audit" -maxdepth 1 \
    -type f -name '.runner.log.tmp.*' -print -quit)" ]]
grep -Fxq 'REASON=admission-closed' \
    "$test_root/prelaunch-close/audit/controller-finished.env"

# A phase transition aborts an admitted child as a recorded non-proof; the
# second completed recording is never admitted.
make_case draining-child
printf 'native-second\nRHPRTRACEFOOTER!12345678901234567890' \
    >"$test_root/draining-child/campaign/traces/save/replay-002-session-0001.jsonl.zst.parity.bitcode.zst"
: >"$test_root/draining-child/campaign/traces/save/replay-002.complete"
started="$test_root/started"
release="$test_root/release"
env PATH="$fake_tools:$PATH" INCREMENTAL_EOF_ONESHOT=0 \
    INCREMENTAL_EOF_POLL_SECONDS=1 INCREMENTAL_EOF_MEMINFO_PATH="$meminfo" \
    INCREMENTAL_EOF_LOADAVG_PATH="$loadavg" \
    INCREMENTAL_EOF_NATIVE_LOCK_DIR="$test_root/native-locks" \
    INCREMENTAL_EOF_SLOT_DIR="$test_root/slots" \
    INCREMENTAL_EOF_OUTER_LOCK="$test_root/outer.lock" TEST_STARTED_FILE="$started" \
    TEST_WAIT_FILE="$release" "$script" "$workspace" "$bundle" "$trust_sha" \
    "$runner_sha" "$test_root/draining-child/orchestrator" \
    "$test_root/draining-child/audit" "$test_root/draining-child/campaign" &
controller_pid=$!
for _ in {1..100}; do [[ -e "$started" ]] && break; sleep 0.05; done
[[ -e "$started" ]]
# All coordination locks required by production are held for the admitted
# attempt. They were acquired nonblocking, so the controller never queues in
# front of production; the phase watchdog below releases them promptly.
relative=${test_root#"$workspace"/}/draining-child/campaign
corpus_sha=$(printf '%s' "$relative" | sha256sum); corpus_sha=${corpus_sha%% *}
for lock in "$test_root/outer.lock" "$test_root/native-locks/$corpus_sha.lock" \
    "$test_root/draining-child/campaign/.distributed-collector.lock" \
    "$test_root/slots/0.lock"
do
    exec {probe_fd}>"$lock"
    if flock -n "$probe_fd"; then
        printf 'test failure: admitted child did not hold %s\n' "$lock" >&2
        exit 1
    fi
    exec {probe_fd}>&-
done
printf 'PHASE=drain-and-verify-seed3\n' \
    >"$test_root/draining-child/orchestrator/state.env"
sleep 2
wait "$controller_pid"
[[ "$(find "$test_root/draining-child/audit/results" -mindepth 1 -maxdepth 1 -type d | wc -l)" == 0 ]]
attempt=$(find "$test_root/draining-child/audit/attempts" -mindepth 1 -maxdepth 1 -type d)
[[ -n "$attempt" && "$(<"$attempt/status")" == aborted-phase-transition ]]
[[ "$(grep -Fxc 'parity trace matched every recorded frame' "$attempt/log" || true)" == 0 ]]
if kill -0 "$controller_pid" 2>/dev/null; then
    printf 'test failure: phase-aborted controller is still alive\n' >&2
    exit 1
fi
grep -Fxq 'REASON=admission-closed' \
    "$test_root/draining-child/audit/controller-finished.env"

# Controller signals terminate the owned process group, publish only an
# operational aborted attempt, and release every coordination lock. The fake
# runner ignores TERM so the one-second test grace also exercises escalation.
make_case signal-child
signal_started="$test_root/signal-started"
signal_release="$test_root/signal-release"
env PATH="$fake_tools:$PATH" INCREMENTAL_EOF_ONESHOT=0 \
    INCREMENTAL_EOF_ABORT_GRACE_SECONDS=1 \
    INCREMENTAL_EOF_MEMINFO_PATH="$meminfo" \
    INCREMENTAL_EOF_LOADAVG_PATH="$loadavg" \
    INCREMENTAL_EOF_NATIVE_LOCK_DIR="$test_root/native-locks" \
    INCREMENTAL_EOF_SLOT_DIR="$test_root/slots" \
    INCREMENTAL_EOF_OUTER_LOCK="$test_root/outer.lock" \
    TEST_STARTED_FILE="$signal_started" TEST_WAIT_FILE="$signal_release" \
    TEST_IGNORE_TERM=1 "$script" "$workspace" "$bundle" "$trust_sha" \
    "$runner_sha" "$test_root/signal-child/orchestrator" \
    "$test_root/signal-child/audit" "$test_root/signal-child/campaign" &
signal_controller_pid=$!
for _ in {1..100}; do [[ -e "$signal_started" ]] && break; sleep 0.05; done
[[ -e "$signal_started" ]]
kill -TERM "$signal_controller_pid"
wait "$signal_controller_pid"
[[ -z "$(find "$test_root/signal-child/audit/results" -mindepth 1 \
    -maxdepth 1 -type d -print -quit)" ]]
signal_attempt=$(find "$test_root/signal-child/audit/attempts" -mindepth 1 \
    -maxdepth 1 -type d)
[[ -n "$signal_attempt" \
    && "$(<"$signal_attempt/status")" == aborted-controller-signal ]]
grep -Fxq 'REASON=signal-or-oneshot' \
    "$test_root/signal-child/audit/controller-finished.env"
exec {probe_fd}>"$test_root/outer.lock"
flock -n "$probe_fd"
exec {probe_fd}>&-

# Four independent recordings can run simultaneously. Incremental readers use
# shared outer/corpus/collector locks, so every production writer's exclusive
# probe still fails, and each replay owns a distinct global runner slot.
make_case parallel
for replay in 002 003 004; do
    printf 'native-%s\nRHPRTRACEFOOTER!12345678901234567890' "$replay" \
        >"$test_root/parallel/campaign/traces/save/replay-$replay-session-0001.jsonl.zst.parity.bitcode.zst"
    : >"$test_root/parallel/campaign/traces/save/replay-$replay.complete"
done
parallel_active="$test_root/parallel-active"
parallel_release="$test_root/parallel-release"
mkdir -p "$parallel_active"
env PATH="$fake_tools:$PATH" INCREMENTAL_EOF_ONESHOT=1 \
    INCREMENTAL_EOF_CONCURRENCY=4 INCREMENTAL_EOF_MEMINFO_PATH="$meminfo" \
    INCREMENTAL_EOF_LOADAVG_PATH="$loadavg" \
    INCREMENTAL_EOF_NATIVE_LOCK_DIR="$test_root/native-locks" \
    INCREMENTAL_EOF_SLOT_DIR="$test_root/slots" \
    INCREMENTAL_EOF_OUTER_LOCK="$test_root/outer.lock" \
    TEST_ACTIVE_DIR="$parallel_active" TEST_WAIT_FILE="$parallel_release" \
    "$script" "$workspace" "$bundle" "$trust_sha" "$runner_sha" \
    "$test_root/parallel/orchestrator" "$test_root/parallel/audit" \
    "$test_root/parallel/campaign" &
parallel_controller_pid=$!
for _ in {1..200}; do
    active_count=$(find "$parallel_active" -type f 2>/dev/null | wc -l)
    if (( active_count == 4 )); then
        break
    fi
    sleep 0.05
done
if (( active_count != 4 )); then
    kill -TERM "$parallel_controller_pid" 2>/dev/null || true
    wait "$parallel_controller_pid" || true
    printf 'test failure: expected four simultaneous incremental runners, saw %s\n' \
        "$active_count" >&2
    exit 1
fi
relative=${test_root#"$workspace"/}/parallel/campaign
corpus_sha=$(printf '%s' "$relative" | sha256sum); corpus_sha=${corpus_sha%% *}
for lock in "$test_root/outer.lock" "$test_root/native-locks/$corpus_sha.lock" \
    "$test_root/parallel/campaign/.distributed-collector.lock" \
    "$test_root/slots/0.lock" "$test_root/slots/1.lock" \
    "$test_root/slots/2.lock" "$test_root/slots/3.lock"
do
    exec {probe_fd}>"$lock"
    if flock -n "$probe_fd"; then
        printf 'test failure: parallel replay did not protect %s\n' "$lock" >&2
        exit 1
    fi
    exec {probe_fd}>&-
done
: >"$parallel_release"
wait "$parallel_controller_pid"
[[ $(find "$test_root/parallel/audit/results" -mindepth 1 -maxdepth 1 \
    -type d | wc -l) == 4 ]]
grep -Fxq 'CONCURRENCY=4' "$test_root/parallel/audit/provenance.env"
grep -Fxq 'MEMORY_PER_JOB_KIB=6291456' "$test_root/parallel/audit/provenance.env"
[[ $(find "$test_root/parallel/audit" -maxdepth 1 -type d \
    -name '.result.tmp.*' | wc -l) == 0 ]]

# HUP reaches the controller and its asynchronous worker shells, but not the
# setsid replay groups. Worker-local traps publish the shared stop sentinel;
# each worker then terminates and reaps its own replay before releasing locks.
make_case parallel-hup
for replay in 002 003 004; do
    printf 'native-%s\nRHPRTRACEFOOTER!12345678901234567890' "$replay" \
        >"$test_root/parallel-hup/campaign/traces/save/replay-$replay-session-0001.jsonl.zst.parity.bitcode.zst"
    : >"$test_root/parallel-hup/campaign/traces/save/replay-$replay.complete"
done
hup_active="$test_root/hup-active"
hup_release="$test_root/hup-release"
mkdir -p "$hup_active"
setsid env PATH="$fake_tools:$PATH" INCREMENTAL_EOF_ONESHOT=0 \
    INCREMENTAL_EOF_CONCURRENCY=4 INCREMENTAL_EOF_ABORT_GRACE_SECONDS=1 \
    INCREMENTAL_EOF_MEMINFO_PATH="$meminfo" INCREMENTAL_EOF_LOADAVG_PATH="$loadavg" \
    INCREMENTAL_EOF_NATIVE_LOCK_DIR="$test_root/native-locks" \
    INCREMENTAL_EOF_SLOT_DIR="$test_root/slots" \
    INCREMENTAL_EOF_OUTER_LOCK="$test_root/outer.lock" \
    TEST_ACTIVE_DIR="$hup_active" TEST_WAIT_FILE="$hup_release" \
    "$script" "$workspace" "$bundle" "$trust_sha" "$runner_sha" \
    "$test_root/parallel-hup/orchestrator" "$test_root/parallel-hup/audit" \
    "$test_root/parallel-hup/campaign" &
hup_controller_pid=$!
for _ in {1..200}; do
    hup_count=$(find "$hup_active" -type f | wc -l)
    if (( hup_count == 4 )); then break; fi
    sleep 0.05
done
[[ $hup_count == 4 ]]
mapfile -t hup_runner_pids < <(find "$hup_active" -type f -printf '%f\n')
kill -HUP -- "-$hup_controller_pid"
wait "$hup_controller_pid"
for pid in "${hup_runner_pids[@]}"; do
    if kill -0 "$pid" 2>/dev/null; then
        printf 'test failure: HUP left runner pid %s alive\n' "$pid" >&2
        exit 1
    fi
done
[[ -z $(find "$hup_active" -type f -print -quit) ]]
[[ -z $(find "$test_root/parallel-hup/audit" -maxdepth 1 \
    -type f -name '.runner.log.tmp.*' -print -quit) ]]
relative=${test_root#"$workspace"/}/parallel-hup/campaign
corpus_sha=$(printf '%s' "$relative" | sha256sum); corpus_sha=${corpus_sha%% *}
for lock in "$test_root/outer.lock" "$test_root/native-locks/$corpus_sha.lock" \
    "$test_root/parallel-hup/campaign/.distributed-collector.lock" \
    "$test_root/slots/0.lock" "$test_root/slots/1.lock" \
    "$test_root/slots/2.lock" "$test_root/slots/3.lock"
do
    exec {probe_fd}>"$lock"
    flock -n "$probe_fd"
    exec {probe_fd}>&-
done

# STOP.env is a serialized replay start gate. The second worker is delayed in
# hashing until the first publishes a semantic failure; it must never invoke
# the runner, and restarting the stopped audit must also invoke nothing.
make_case stop-gate
printf 'native-002\nRHPRTRACEFOOTER!12345678901234567890' \
    >"$test_root/stop-gate/campaign/traces/save/replay-002-session-0001.jsonl.zst.parity.bitcode.zst"
: >"$test_root/stop-gate/campaign/traces/save/replay-002.complete"
stop_invocations="$test_root/stop-gate.invocations"
delay_started="$test_root/stop-gate.delay-started"
delay_release="$test_root/stop-gate.delay-release"
env PATH="$fake_tools:$PATH" INCREMENTAL_EOF_ONESHOT=1 \
    INCREMENTAL_EOF_CONCURRENCY=2 INCREMENTAL_EOF_MEMINFO_PATH="$meminfo" \
    INCREMENTAL_EOF_LOADAVG_PATH="$loadavg" \
    INCREMENTAL_EOF_NATIVE_LOCK_DIR="$test_root/native-locks" \
    INCREMENTAL_EOF_SLOT_DIR="$test_root/slots" \
    INCREMENTAL_EOF_OUTER_LOCK="$test_root/outer.lock" \
    TEST_INVOCATIONS="$stop_invocations" TEST_RUNNER_FAIL_PATTERN=replay-001 \
    TEST_SHA_DELAY_PATTERN=replay-002 TEST_SHA_DELAY_STARTED="$delay_started" \
    TEST_SHA_DELAY_RELEASE="$delay_release" \
    "$script" "$workspace" "$bundle" "$trust_sha" "$runner_sha" \
    "$test_root/stop-gate/orchestrator" "$test_root/stop-gate/audit" \
    "$test_root/stop-gate/campaign" &
stop_controller_pid=$!
for _ in {1..200}; do
    [[ -e "$delay_started" && -e "$test_root/stop-gate/audit/STOP.env" ]] && break
    sleep 0.05
done
[[ -e "$delay_started" && -e "$test_root/stop-gate/audit/STOP.env" ]]
: >"$delay_release"
stop_status=0
wait "$stop_controller_pid" || stop_status=$?
[[ $stop_status == 1 ]]
[[ $(wc -l <"$stop_invocations") == 1 ]]
grep -Fq 'replay-001' "$stop_invocations"
if run_case stop-gate INCREMENTAL_EOF_CONCURRENCY=2 \
    TEST_INVOCATIONS="$stop_invocations"; then
    printf 'test failure: stopped audit resumed\n' >&2
    exit 1
fi
[[ $(wc -l <"$stop_invocations") == 1 ]]

# An existing internal/tamper failure is recovered before work admission,
# publishes BATCH_FATAL.env, and rejects every later resume before setsid.
make_case fatal-gate
run_case fatal-gate INCREMENTAL_EOF_CONCURRENCY=2
fatal_result=$(find "$test_root/fatal-gate/audit/results" -mindepth 1 \
    -maxdepth 1 -type d)
printf 'tamper\n' >>"$fatal_result/log"
printf 'native-002\nRHPRTRACEFOOTER!12345678901234567890' \
    >"$test_root/fatal-gate/campaign/traces/save/replay-002-session-0001.jsonl.zst.parity.bitcode.zst"
: >"$test_root/fatal-gate/campaign/traces/save/replay-002.complete"
fatal_invocations="$test_root/fatal-gate.invocations"
fatal_status=0
env PATH="$fake_tools:$PATH" INCREMENTAL_EOF_ONESHOT=1 \
    INCREMENTAL_EOF_CONCURRENCY=2 INCREMENTAL_EOF_MEMINFO_PATH="$meminfo" \
    INCREMENTAL_EOF_LOADAVG_PATH="$loadavg" \
    INCREMENTAL_EOF_NATIVE_LOCK_DIR="$test_root/native-locks" \
    INCREMENTAL_EOF_SLOT_DIR="$test_root/slots" \
    INCREMENTAL_EOF_OUTER_LOCK="$test_root/outer.lock" \
    TEST_INVOCATIONS="$fatal_invocations" \
    "$script" "$workspace" "$bundle" "$trust_sha" "$runner_sha" \
    "$test_root/fatal-gate/orchestrator" "$test_root/fatal-gate/audit" \
    "$test_root/fatal-gate/campaign" || fatal_status=$?
[[ $fatal_status == 2 ]]
[[ -e "$test_root/fatal-gate/audit/BATCH_FATAL.env" ]]
[[ ! -e "$fatal_invocations" ]]
fatal_status=0
run_case fatal-gate INCREMENTAL_EOF_CONCURRENCY=2 \
    TEST_INVOCATIONS="$fatal_invocations" || fatal_status=$?
[[ $fatal_status == 2 && ! -e "$fatal_invocations" ]]

# Reused proofs do not consume slots or inflate memory reservations. Three
# authenticated reuses plus one new trace still run with memory for one job.
make_case reuse-slots
run_case reuse-slots INCREMENTAL_EOF_CONCURRENCY=4 \
    INCREMENTAL_EOF_MIN_MEMORY_KIB=100 INCREMENTAL_EOF_MEMORY_PER_JOB_KIB=100
for replay in 002 003; do
    cp "$test_root/reuse-slots/campaign/traces/save/replay-001-session-0001.jsonl.zst.parity.bitcode.zst" \
        "$test_root/reuse-slots/campaign/traces/save/replay-$replay-session-0001.jsonl.zst.parity.bitcode.zst"
    : >"$test_root/reuse-slots/campaign/traces/save/replay-$replay.complete"
done
# Establish independent proofs for 002 and 003 before adding the fourth trace.
run_case reuse-slots INCREMENTAL_EOF_CONCURRENCY=4 \
    INCREMENTAL_EOF_MIN_MEMORY_KIB=100 INCREMENTAL_EOF_MEMORY_PER_JOB_KIB=100
printf 'native-004\nRHPRTRACEFOOTER!12345678901234567890' \
    >"$test_root/reuse-slots/campaign/traces/save/replay-004-session-0001.jsonl.zst.parity.bitcode.zst"
: >"$test_root/reuse-slots/campaign/traces/save/replay-004.complete"
printf 'MemAvailable: 100 kB\n' >"$meminfo"
run_case reuse-slots INCREMENTAL_EOF_CONCURRENCY=4 \
    INCREMENTAL_EOF_MIN_MEMORY_KIB=100 INCREMENTAL_EOF_MEMORY_PER_JOB_KIB=100
[[ $(find "$test_root/reuse-slots/audit/results" -mindepth 1 -maxdepth 1 \
    -type d | wc -l) == 4 ]]

# Recovered internal/tamper status 2 outranks a pending semantic failure in
# either trace order. Recovery happens before worker admission, so the pending
# replay must never reach the runner or publish a semantic STOP.
printf 'MemAvailable: 99999999 kB\n' >"$meminfo"
for order in internal-first semantic-first; do
    make_case "mixed-$order"
    if [[ $order == semantic-first ]]; then
        mv "$test_root/mixed-$order/campaign/traces/save/replay-001-session-0001.jsonl.zst.parity.bitcode.zst" \
            "$test_root/mixed-$order/campaign/traces/save/replay-002-session-0001.jsonl.zst.parity.bitcode.zst"
        mv "$test_root/mixed-$order/campaign/traces/save/replay-001.complete" \
            "$test_root/mixed-$order/campaign/traces/save/replay-002.complete"
    fi
    run_case "mixed-$order" INCREMENTAL_EOF_CONCURRENCY=2
    mixed_result=$(find "$test_root/mixed-$order/audit/results" -mindepth 1 \
        -maxdepth 1 -type d)
    printf 'tamper\n' >>"$mixed_result/log"
    if [[ $order == internal-first ]]; then
        new_replay=002
    else
        new_replay=001
    fi
    printf 'native-new\nRHPRTRACEFOOTER!12345678901234567890' \
        >"$test_root/mixed-$order/campaign/traces/save/replay-$new_replay-session-0001.jsonl.zst.parity.bitcode.zst"
    : >"$test_root/mixed-$order/campaign/traces/save/replay-$new_replay.complete"
    mixed_invocations="$test_root/mixed-$order.invocations"
    mixed_status=0
    env PATH="$fake_tools:$PATH" INCREMENTAL_EOF_ONESHOT=1 \
        INCREMENTAL_EOF_CONCURRENCY=2 INCREMENTAL_EOF_MEMINFO_PATH="$meminfo" \
        INCREMENTAL_EOF_LOADAVG_PATH="$loadavg" \
        INCREMENTAL_EOF_NATIVE_LOCK_DIR="$test_root/native-locks" \
        INCREMENTAL_EOF_SLOT_DIR="$test_root/slots" \
        INCREMENTAL_EOF_OUTER_LOCK="$test_root/outer.lock" \
        TEST_INVOCATIONS="$mixed_invocations" \
        TEST_RUNNER_FAIL_PATTERN="replay-$new_replay" \
        "$script" "$workspace" "$bundle" "$trust_sha" "$runner_sha" \
        "$test_root/mixed-$order/orchestrator" \
        "$test_root/mixed-$order/audit" "$test_root/mixed-$order/campaign" \
        || mixed_status=$?
    [[ $mixed_status == 2 ]]
    [[ -e "$test_root/mixed-$order/audit/BATCH_FATAL.env" ]]
    [[ ! -e "$test_root/mixed-$order/audit/STOP.env" ]]
    [[ ! -e "$mixed_invocations" ]]
done

# The memory threshold grows with each concurrently admitted job. With room for
# only two reservations, jobs three and four remain unstarted and are audited
# as closed gates rather than oversubscribing the host.
make_case dynamic-gate
for replay in 002 003 004; do
    printf 'native-%s\nRHPRTRACEFOOTER!12345678901234567890' "$replay" \
        >"$test_root/dynamic-gate/campaign/traces/save/replay-$replay-session-0001.jsonl.zst.parity.bitcode.zst"
    : >"$test_root/dynamic-gate/campaign/traces/save/replay-$replay.complete"
done
printf 'MemAvailable: 250 kB\n' >"$meminfo"
run_case dynamic-gate INCREMENTAL_EOF_CONCURRENCY=4 \
    INCREMENTAL_EOF_MIN_MEMORY_KIB=100 INCREMENTAL_EOF_MEMORY_PER_JOB_KIB=100
[[ $(find "$test_root/dynamic-gate/audit/results" -mindepth 1 -maxdepth 1 \
    -type d | wc -l) == 2 ]]
grep -Eq $'\t(3\t300|4\t400)\tclosed$' \
    "$test_root/dynamic-gate/audit/resource-gate.tsv"

# A closed resource gate starts nothing and leaves an auditable sample.
make_case gate
printf 'MemAvailable: 1 kB\n' >"$meminfo"
run_case gate
[[ -z "$(find "$test_root/gate/audit/results" -mindepth 1 -maxdepth 1 -type d -print -quit)" ]]
grep -q $'\tclosed$' "$test_root/gate/audit/resource-gate.tsv"

printf 'incremental EOF controller tests passed\n'
