#!/usr/bin/env bash
set -euo pipefail

workspace=$(pwd)
mkdir -p "$workspace/.codex-tmp"
test_root=$(mktemp -d "$workspace/.codex-tmp/parity-release-sweep-test.XXXXXX")
trap 'rm -rf -- "$test_root"' EXIT

traces="$test_root/traces"
slots="$test_root/slots"
mkdir -p "$traces" "$slots"
printf 'pass\n' >"$traces/01-pass.jsonl.zst"
printf 'fail\n' >"$traces/02-fail.jsonl.zst"
printf 'after\n' >"$traces/03-after.jsonl.zst"
printf 'nonexact\n' >"$traces/04-nonexact.jsonl.zst"
# A converted trace: only the native artifact exists on disk, but the sweep
# still addresses it by its logical .jsonl.zst identity.
printf 'native\n' >"$traces/05-native.jsonl.zst.parity.bitcode.zst"

runner="$test_root/fake-runner"
invocation_log="$test_root/runner-invocations"
: >"$invocation_log"
cat >"$runner" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
trace=${!#}
name=${trace##*/}
printf '%s\n' "$name" >>"$FAKE_RUNNER_INVOCATIONS"
case "$name" in
    01-pass.jsonl.zst|03-after.jsonl.zst|05-native.jsonl.zst)
        if [[ "$name" == 01-pass.jsonl.zst \
            && -n ${FAKE_PARALLEL_BARRIER:-} ]]
        then
            : >"$FAKE_PARALLEL_BARRIER"
            sleep 0.2
        fi
        printf '%s\n' 'parity trace matched every recorded frame'
        ;;
    02-fail.jsonl.zst)
        if [[ ${FAKE_RUNNER_REPAIR:-0} == 1 ]]; then
            printf '%s\n' 'parity trace matched every recorded frame'
        else
            if [[ -n ${FAKE_PARALLEL_BARRIER:-} ]]; then
                for _attempt in {1..100}; do
                    [[ -f "$FAKE_PARALLEL_BARRIER" ]] && break
                    sleep 0.01
                done
            fi
            printf '%s\n' 'deliberate replay failure' >&2
            exit 23
        fi
        ;;
    04-nonexact.jsonl.zst)
        printf '%s\n' 'runner exited successfully without the exact marker'
        ;;
    *)
        printf 'unexpected fake trace: %s\n' "$trace" >&2
        exit 99
        ;;
esac
EOF
chmod +x "$runner"

fake_tools="$test_root/fake-tools"
mkdir -p "$fake_tools"
real_mktemp=$(command -v mktemp)
real_mv=$(command -v mv)
cat >"$fake_tools/mktemp" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
for argument in "$@"; do
    if [[ -n ${FAKE_MKTEMP_FAIL_PATTERN:-} \
        && "$argument" == *"$FAKE_MKTEMP_FAIL_PATTERN"* ]]
    then
        exit 88
    fi
done
exec "$REAL_MKTEMP" "$@"
EOF
cat >"$fake_tools/mv" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
for argument in "$@"; do
    if [[ -n ${FAKE_MV_FAIL_PATTERN:-} \
        && "$argument" == *"$FAKE_MV_FAIL_PATTERN"* ]]
    then
        exit 88
    fi
done
exec "$REAL_MV" "$@"
EOF
chmod +x "$fake_tools/mktemp" "$fake_tools/mv"

run_sweep_shape() {
    local audit=$1
    local shard=$2
    local shards=$3
    shift 3
    env \
        FAKE_RUNNER_INVOCATIONS="$invocation_log" \
        PARITY_SWEEP_SLOT_DIR="$slots" \
        PARITY_SWEEP_GLOBAL_CONCURRENCY=1 \
        "$@" \
        scripts/run_parity_release_sweep.sh \
            "$workspace" "$audit" "$runner" "$shard" "$shards"
}

run_sweep() {
    local audit=$1
    shift
    run_sweep_shape "$audit" 0 1 "$@"
}

status_for() {
    local audit=$1
    local trace=$2
    local relative=${trace#"$workspace"/}
    local key=${relative//\//__}
    printf '%s/status/%s.status\n' "$audit" "$key"
}

log_for() {
    local audit=$1
    local trace=$2
    local relative=${trace#"$workspace"/}
    local key=${relative//\//__}
    printf '%s/logs/%s.log\n' "$audit" "$key"
}

assert_no_temporaries() {
    local audit=$1
    if find "$audit" -type f -name '*.tmp.*' -print -quit | grep -q .; then
        printf 'test failure: temporary result remained under %s\n' "$audit" >&2
        return 1
    fi
}

make_single_trace_audit() {
    local audit=$1
    mkdir -p "$audit"
    printf '%s\n' "$traces/01-pass.jsonl.zst" >"$audit/traces.snapshot"
}

# A fresh fail-fast sweep publishes the failing result and does not launch the
# following trace. The full snapshot is input-only and remains byte-identical.
audit="$test_root/fail-fast"
mkdir -p "$audit"
printf '%s\n' \
    "$traces/01-pass.jsonl.zst" \
    "$traces/02-fail.jsonl.zst" \
    "$traces/03-after.jsonl.zst" >"$audit/traces.snapshot"
cp "$audit/traces.snapshot" "$audit/traces.snapshot.expected"
if run_sweep "$audit" PARITY_SWEEP_FAIL_FAST=1; then
    printf 'test failure: fail-fast sweep accepted a nonzero runner\n' >&2
    exit 1
fi
cmp "$audit/traces.snapshot.expected" "$audit/traces.snapshot"
[[ "$(<"$(status_for "$audit" "$traces/01-pass.jsonl.zst")")" == 0 ]]
[[ "$(<"$(status_for "$audit" "$traces/02-fail.jsonl.zst")")" == 23 ]]
[[ ! -e "$(status_for "$audit" "$traces/03-after.jsonl.zst")" ]]
[[ "$(wc -l <"$invocation_log")" == 2 ]]
assert_no_temporaries "$audit"

# Resume stops on the already-recorded failure before launching new work.
if run_sweep "$audit" PARITY_SWEEP_FAIL_FAST=1; then
    printf 'test failure: resume crossed an existing nonzero status\n' >&2
    exit 1
fi
[[ "$(wc -l <"$invocation_log")" == 2 ]]

# Once the diagnosed result is explicitly cleared, resume skips the prior pass
# and completes the repaired trace plus the remainder.
rm -f -- \
    "$(status_for "$audit" "$traces/02-fail.jsonl.zst")" \
    "$(log_for "$audit" "$traces/02-fail.jsonl.zst")"
run_sweep "$audit" PARITY_SWEEP_FAIL_FAST=1 FAKE_RUNNER_REPAIR=1
[[ "$(<"$(status_for "$audit" "$traces/02-fail.jsonl.zst")")" == 0 ]]
[[ "$(<"$(status_for "$audit" "$traces/03-after.jsonl.zst")")" == 0 ]]
[[ "$(wc -l <"$invocation_log")" == 4 ]]
cmp "$audit/traces.snapshot.expected" "$audit/traces.snapshot"
assert_no_temporaries "$audit"

# Resume treats a status-zero result without its exact-marker log as corrupt
# evidence and stops without launching anything else.
pass_log=$(log_for "$audit" "$traces/01-pass.jsonl.zst")
mv "$pass_log" "$pass_log.saved"
if run_sweep "$audit" PARITY_SWEEP_FAIL_FAST=1; then
    printf 'test failure: resume accepted status zero without its proof log\n' >&2
    exit 1
fi
[[ "$(wc -l <"$invocation_log")" == 4 ]]
mv "$pass_log.saved" "$pass_log"

# Fail-fast is deliberately unavailable to distributed or multi-slot sweeps;
# otherwise another process could launch work after this process records a
# failure but before learning about it.
shape_audit="$test_root/rejected-shape"
make_single_trace_audit "$shape_audit"
shape_invocations=$(wc -l <"$invocation_log")
if run_sweep_shape "$shape_audit" 0 2 PARITY_SWEEP_FAIL_FAST=1; then
    printf 'test failure: fail-fast accepted SHARDS=2\n' >&2
    exit 1
fi
if run_sweep_shape "$shape_audit" 1 2 PARITY_SWEEP_FAIL_FAST=1; then
    printf 'test failure: fail-fast accepted SHARD=1\n' >&2
    exit 1
fi
if run_sweep "$shape_audit" \
    PARITY_SWEEP_FAIL_FAST=1 PARITY_SWEEP_GLOBAL_CONCURRENCY=2
then
    printf 'test failure: fail-fast accepted global concurrency 2\n' >&2
    exit 1
fi
[[ "$(wc -l <"$invocation_log")" == "$shape_invocations" ]]

# With an explicit shared stop, independent shards may fail fast in parallel.
# The pass is deliberately in flight when the other shard fails; it completes
# and publishes, while the later item on its shard never starts.
parallel_audit="$test_root/parallel-fail-fast"
mkdir -p "$parallel_audit"
printf '%s\n' \
    "$traces/01-pass.jsonl.zst" \
    "$traces/02-fail.jsonl.zst" \
    "$traces/03-after.jsonl.zst" >"$parallel_audit/traces.snapshot"
parallel_stop="$parallel_audit/shared-stop"
parallel_barrier="$test_root/parallel-pass-started"
parallel_before=$(wc -l <"$invocation_log")
run_sweep_shape "$parallel_audit" 0 2 \
    PARITY_SWEEP_FAIL_FAST=1 \
    PARITY_SWEEP_GLOBAL_CONCURRENCY=2 \
    PARITY_SWEEP_FAIL_FAST_STOP="$parallel_stop" \
    PARITY_SWEEP_FAIL_FAST_TOKEN=0123456789abcdef0123456789abcdef \
    FAKE_PARALLEL_BARRIER="$parallel_barrier" &
parallel_pid0=$!
run_sweep_shape "$parallel_audit" 1 2 \
    PARITY_SWEEP_FAIL_FAST=1 \
    PARITY_SWEEP_GLOBAL_CONCURRENCY=2 \
    PARITY_SWEEP_FAIL_FAST_STOP="$parallel_stop" \
    PARITY_SWEEP_FAIL_FAST_TOKEN=0123456789abcdef0123456789abcdef \
    FAKE_PARALLEL_BARRIER="$parallel_barrier" &
parallel_pid1=$!
parallel_status0=0
parallel_status1=0
wait "$parallel_pid0" || parallel_status0=$?
wait "$parallel_pid1" || parallel_status1=$?
[[ "$parallel_status0" != 0 && "$parallel_status1" != 0 ]]
[[ "$(wc -l <"$invocation_log")" == "$((parallel_before + 2))" ]]
[[ "$(<"$(status_for "$parallel_audit" "$traces/01-pass.jsonl.zst")")" == 0 ]]
[[ "$(<"$(status_for "$parallel_audit" "$traces/02-fail.jsonl.zst")")" == 23 ]]
[[ ! -e "$(status_for "$parallel_audit" "$traces/03-after.jsonl.zst")" ]]
[[ -f "$parallel_stop" ]]
assert_no_temporaries "$parallel_audit"

# Setup and atomic publication failures are terminal in fail-fast mode.
bad_slot="$test_root/not-a-slot-directory"
printf 'not a directory\n' >"$bad_slot"
setup_audit="$test_root/setup-failure"
make_single_trace_audit "$setup_audit"
setup_invocations=$(wc -l <"$invocation_log")
if run_sweep "$setup_audit" \
    PARITY_SWEEP_FAIL_FAST=1 PARITY_SWEEP_SLOT_DIR="$bad_slot"
then
    printf 'test failure: fail-fast ignored runner-slot setup failure\n' >&2
    exit 1
fi
[[ "$(wc -l <"$invocation_log")" == "$setup_invocations" ]]

assert_injected_failure() {
    local name=$1
    local tool=$2
    local pattern=$3
    local audit="$test_root/publication-$name"
    local before
    make_single_trace_audit "$audit"
    before=$(wc -l <"$invocation_log")
    if run_sweep "$audit" \
        PARITY_SWEEP_FAIL_FAST=1 \
        PATH="$fake_tools:$PATH" \
        REAL_MKTEMP="$real_mktemp" \
        REAL_MV="$real_mv" \
        "$tool=$pattern"
    then
        printf 'test failure: fail-fast ignored injected %s failure\n' "$name" >&2
        exit 1
    fi
    [[ ! -e "$(status_for "$audit" "$traces/01-pass.jsonl.zst")" ]]
    assert_no_temporaries "$audit"
    if [[ "$name" == log-mktemp ]]; then
        [[ "$(wc -l <"$invocation_log")" == "$before" ]]
    else
        [[ "$(wc -l <"$invocation_log")" == "$((before + 1))" ]]
    fi
}

assert_injected_failure log-mktemp FAKE_MKTEMP_FAIL_PATTERN /logs/
assert_injected_failure status-mktemp FAKE_MKTEMP_FAIL_PATTERN /status/
assert_injected_failure log-mv FAKE_MV_FAIL_PATTERN /logs/
assert_injected_failure status-mv FAKE_MV_FAIL_PATTERN /status/

# A permanent ledger entry cannot hide malformed local proof. In particular,
# status parsing consumes the whole normalized file, not merely its first line.
ledger_audit="$test_root/ledger-local-proof"
make_single_trace_audit "$ledger_audit"
ledger_status=$(status_for "$ledger_audit" "$traces/01-pass.jsonl.zst")
ledger_log=$(log_for "$ledger_audit" "$traces/01-pass.jsonl.zst")
mkdir -p "${ledger_status%/*}" "${ledger_log%/*}"
printf '0\njunk\n' >"$ledger_status"
printf '%s\n' 'parity trace matched every recorded frame' >"$ledger_log"
corpus_relative=${workspace#"$workspace"/}
campaign_key_prefix=${corpus_relative//\//__}
trace_relative=${traces#"$workspace"/}/01-pass.jsonl.zst
trace_key=${trace_relative//\//__}
printf '%s__%s\n' "$campaign_key_prefix" "$trace_key" \
    >"$test_root/permanent.snapshot"
ledger_invocations=$(wc -l <"$invocation_log")
ledger_skip_audit="$test_root/ledger-skip"
make_single_trace_audit "$ledger_skip_audit"
run_sweep "$ledger_skip_audit" \
    PARITY_SWEEP_FAIL_FAST=1 \
    PARITY_PERMANENT_EOF_SNAPSHOT="$test_root/permanent.snapshot"
[[ "$(wc -l <"$invocation_log")" == "$ledger_invocations" ]]
[[ ! -e "$(status_for "$ledger_skip_audit" "$traces/01-pass.jsonl.zst")" ]]
if run_sweep "$ledger_audit" \
    PARITY_SWEEP_FAIL_FAST=1 \
    PARITY_PERMANENT_EOF_SNAPSHOT="$test_root/permanent.snapshot"
then
    printf 'test failure: permanent proof hid malformed local status\n' >&2
    exit 1
fi
[[ "$(wc -l <"$invocation_log")" == "$ledger_invocations" ]]

# Exit zero without exactly one anchored EOF marker is an integrity failure
# and stops before the next trace.
integrity_audit="$test_root/integrity"
mkdir -p "$integrity_audit"
printf '%s\n' \
    "$traces/04-nonexact.jsonl.zst" \
    "$traces/03-after.jsonl.zst" >"$integrity_audit/traces.snapshot"
if run_sweep "$integrity_audit" PARITY_SWEEP_FAIL_FAST=1; then
    printf 'test failure: fail-fast sweep accepted a missing EOF marker\n' >&2
    exit 1
fi
[[ "$(<"$(status_for "$integrity_audit" "$traces/04-nonexact.jsonl.zst")")" == integrity-eof-marker ]]
[[ ! -e "$(status_for "$integrity_audit" "$traces/03-after.jsonl.zst")" ]]
assert_no_temporaries "$integrity_audit"

# A missing trace receives an atomic `missing` status and also stops.
missing_audit="$test_root/missing"
mkdir -p "$missing_audit"
missing_trace="$traces/00-missing.jsonl.zst"
printf '%s\n' "$missing_trace" "$traces/03-after.jsonl.zst" \
    >"$missing_audit/traces.snapshot"
if run_sweep "$missing_audit" PARITY_SWEEP_FAIL_FAST=1; then
    printf 'test failure: fail-fast sweep accepted a missing trace\n' >&2
    exit 1
fi
[[ "$(<"$(status_for "$missing_audit" "$missing_trace")")" == missing ]]
[[ ! -e "$(status_for "$missing_audit" "$traces/03-after.jsonl.zst")" ]]
assert_no_temporaries "$missing_audit"

# A converted trace (native artifact only, addressed by its logical
# .jsonl.zst identity) is dispatched to the runner rather than marked missing.
native_audit="$test_root/native"
mkdir -p "$native_audit"
printf '%s\n' "$traces/05-native.jsonl.zst" >"$native_audit/traces.snapshot"
run_sweep "$native_audit" PARITY_SWEEP_FAIL_FAST=1
[[ "$(<"$(status_for "$native_audit" "$traces/05-native.jsonl.zst")")" == 0 ]]
assert_no_temporaries "$native_audit"

# Default mode preserves the historical collect-all behavior.
default_audit="$test_root/default"
mkdir -p "$default_audit"
printf '%s\n' "$traces/02-fail.jsonl.zst" "$traces/03-after.jsonl.zst" \
    >"$default_audit/traces.snapshot"
run_sweep "$default_audit"
[[ "$(<"$(status_for "$default_audit" "$traces/02-fail.jsonl.zst")")" == 23 ]]
[[ "$(<"$(status_for "$default_audit" "$traces/03-after.jsonl.zst")")" == 0 ]]
assert_no_temporaries "$default_audit"

printf 'parity release sweep fail-fast tests passed\n'
