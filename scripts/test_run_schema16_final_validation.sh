#!/usr/bin/env bash
set -euo pipefail

repository=$(pwd)
mkdir -p "$repository/.codex-tmp"
test_root=$(mktemp -d "$repository/.codex-tmp/schema16-final-validation-test.XXXXXX")
locked_pid=
cleanup() {
    if [[ -n "$locked_pid" ]]; then
        kill "$locked_pid" 2>/dev/null || true
        wait "$locked_pid" 2>/dev/null || true
    fi
    rm -rf -- "$test_root"
}
trap cleanup EXIT

workspace="$test_root/workspace"
mkdir -p "$workspace/scripts" "$workspace/.git"
ln -s "$repository/scripts/run_parity_release_sweep.sh" \
    "$workspace/scripts/run_parity_release_sweep.sh"
validator="$repository/scripts/run_schema16_final_validation.sh"
invocations="$test_root/runner-invocations"
: >"$invocations"

runner="$test_root/fake-runner"
cat >"$runner" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
trace=${!#}
printf '%s\n' "$trace" >>"$FAKE_RUNNER_INVOCATIONS"
case "${trace##*/}" in
    *02-fail-session-*.jsonl.zst)
        if [[ ${FAKE_RUNNER_REPAIR:-0} != 1 ]]; then
            printf '%s\n' 'deliberate replay failure' >&2
            exit 23
        fi
        ;;
esac
printf '%s\n' 'parity trace matched every recorded frame'
EOF
chmod +x "$runner"
runner_sha=$(sha256sum -- "$runner")
runner_sha=${runner_sha%% *}

make_campaign() {
    local name=$1 seed=$2 expected=$3
    local campaign="$workspace/parity-save-replays/60s-random-input/$name"
    mkdir -p "$campaign/traces"
    {
        printf 'PARITY_TRACE_SCHEMA=16\n'
        printf 'PARITY_INPUT_SEED_BASE=%s\n' "$seed"
        printf 'EXPECTED_LOGICAL_REPLAYS=%s\n' "$expected"
    } >"$campaign/campaign.env"
    printf '%s\n' "$campaign"
}

add_trace() {
    local campaign=$1 stem=$2
    mkdir -p "$campaign/traces/save"
    printf 'fake trace\n' >"$campaign/traces/save/$stem-session-0001.jsonl.zst"
    : >"$campaign/traces/save/$stem.complete"
}

audit_for() {
    local campaign=$2 sha=$3 relative digest label
    relative=${campaign#"$workspace"/}
    digest=$(printf '%s' "$relative" | sha256sum)
    digest=${digest%% *}
    label=$(printf '%s' "${campaign##*/}" | tr -c 'A-Za-z0-9._-' '_')
    label=${label:0:48}
    printf '%s/parity-save-replays/audits/schema16-final-%s-path-%s-runner-%s\n' \
        "$workspace" "$label" "$digest" "$sha"
}

status_for() {
    local audit=$1 trace=$2 relative key
    relative=${trace#"$workspace"/}
    key=${relative//\//__}
    printf '%s/status/%s.status\n' "$audit" "$key"
}

log_for() {
    local audit=$1 trace=$2 relative key
    relative=${trace#"$workspace"/}
    key=${relative//\//__}
    printf '%s/logs/%s.log\n' "$audit" "$key"
}

run_validation() {
    local selected_runner=$2 selected_sha=$3
    shift 3
    env \
        FAKE_RUNNER_INVOCATIONS="$invocations" \
        SCHEMA16_FINAL_OUTER_LOCK="$test_root/final-validation.lock" \
        "$validator" "$workspace" "$selected_runner" "$selected_sha" "$@"
}

run_validation_repaired() {
    FAKE_RUNNER_REPAIR=1 run_validation "$@"
}

expect_failure_without_run() {
    local before=$1
    shift
    if "$@"; then
        printf 'test failure: command unexpectedly succeeded: %q\n' "$*" >&2
        exit 1
    fi
    [[ "$(wc -l <"$invocations")" == "$before" ]]
}

# Structural preflight requires a bijection between completion markers and zst
# traces, not merely two independently matching totals.
bare=$(make_campaign bare 3000000 1)
mkdir -p "$bare/traces/save"
printf 'bare\n' >"$bare/traces/save/bare-session-0001.jsonl.zst"
before=$(wc -l <"$invocations")
expect_failure_without_run "$before" \
    run_validation "$test_root/audit-bare" "$runner" "$runner_sha" "$bare"

orphan=$(make_campaign orphan 3000000 1)
mkdir -p "$orphan/traces/save"
: >"$orphan/traces/save/orphan.complete"
expect_failure_without_run "$before" \
    run_validation "$test_root/audit-orphan" "$runner" "$runner_sha" "$orphan"

compensating=$(make_campaign compensating 3000000 2)
mkdir -p "$compensating/traces/save"
: >"$compensating/traces/save/one.complete"
: >"$compensating/traces/save/two.complete"
printf 'one\n' >"$compensating/traces/save/one-session-0001.jsonl.zst"
printf 'two\n' >"$compensating/traces/save/one-session-0002.jsonl.zst"
expect_failure_without_run "$before" \
    run_validation "$test_root/audit-compensating" \
        "$runner" "$runner_sha" "$compensating"

# A wrong trusted hash is rejected before any audit or replay work.
wrong_sha=$(printf '0%.0s' {1..64})
expect_failure_without_run "$before" \
    run_validation "$test_root/audit-wrong-sha" "$runner" "$wrong_sha" "$bare"

reserved=$(make_campaign reserved 2400000 1)
add_trace "$reserved" 01-pass
mkdir -p "$reserved/.capture-reservations"
: >"$reserved/.capture-reservations/save.reserve"
expect_failure_without_run "$before" \
    run_validation "$test_root/audit-reserved" "$runner" "$runner_sha" "$reserved"

# The validator takes its outer lock before entering the release sweep, whose
# slot zero is the repository-wide parity slot. Neither blocked boundary may
# allow the fake runner to start.
locked_campaign=$(make_campaign schema16-seed2500000-locked-test 2500000 1)
add_trace "$locked_campaign" 01-pass
locked_audit="$test_root/audit-locked"
held_outer=/tmp/robin-parity-runner.lock
slot_dir="$workspace/.git/parity-runner-slots"
mkdir -p "$slot_dir"
exec {held_outer_fd}>"$held_outer"
flock "$held_outer_fd"
before=$(wc -l <"$invocations")
env \
    FAKE_RUNNER_INVOCATIONS="$invocations" \
    "$validator" "$workspace" "$runner" "$runner_sha" "$locked_campaign" \
    >"$test_root/locked-validation.log" 2>&1 &
locked_pid=$!
sleep 0.1
[[ "$(wc -l <"$invocations")" == "$before" ]]
exec {held_slot_fd}>"$slot_dir/0.lock"
flock "$held_slot_fd"
flock -u "$held_outer_fd"
exec {held_outer_fd}>&-

validator_holds_outer=0
for _attempt in {1..100}; do
    exec {probe_outer_fd}>"$held_outer"
    if ! flock -n "$probe_outer_fd"; then
        validator_holds_outer=1
        exec {probe_outer_fd}>&-
        break
    fi
    exec {probe_outer_fd}>&-
    sleep 0.02
done
[[ "$validator_holds_outer" == 1 ]]
[[ "$(wc -l <"$invocations")" == "$before" ]]
flock -u "$held_slot_fd"
exec {held_slot_fd}>&-
wait "$locked_pid"
locked_pid=
[[ "$(wc -l <"$invocations")" == "$((before + 1))" ]]

# Seed 3 failure is published and classified, and seed 4 is never started.
seed3=$(make_campaign schema16-seed3000000-final-test 3000000 2)
add_trace "$seed3" 01-pass
add_trace "$seed3" 02-fail
seed4=$(make_campaign schema16-seed4000000-final-test 4000000 2)
add_trace "$seed4" 01-pass
add_trace "$seed4" 03-after
ordered_audit="$test_root/audit-ordered"
ordered_before=$(wc -l <"$invocations")
if run_validation "$ordered_audit" "$runner" "$runner_sha" "$seed3" "$seed4"; then
    printf 'test failure: ordered validation crossed seed3 failure\n' >&2
    exit 1
fi
[[ "$(wc -l <"$invocations")" == "$((ordered_before + 2))" ]]
! grep -Fq -- "$seed4/" "$invocations"
seed3_audit=$(audit_for "$ordered_audit" "$seed3" "$runner_sha")
[[ -f "$seed3_audit/parity-last-failure.env" ]]
grep -Fq 'CLASSIFICATION=nonzero-or-malformed-status' \
    "$seed3_audit/parity-last-failure.env"
snapshot_before=$(sha256sum -- "$seed3_audit/traces.snapshot")

# Resume stops at preserved failure without launching work. Clearing only that
# diagnosed result lets seed3 finish, then and only then starts seed4.
if run_validation "$ordered_audit" "$runner" "$runner_sha" "$seed3" "$seed4"; then
    printf 'test failure: resume accepted preserved seed3 failure\n' >&2
    exit 1
fi
[[ "$(wc -l <"$invocations")" == "$((ordered_before + 2))" ]]
failed_trace="$seed3/traces/save/02-fail-session-0001.jsonl.zst"
rm -f -- \
    "$(status_for "$seed3_audit" "$failed_trace")" \
    "$(log_for "$seed3_audit" "$failed_trace")"
FAKE_RUNNER_REPAIR=1 run_validation \
    "$ordered_audit" "$runner" "$runner_sha" "$seed3" "$seed4"
[[ "$(wc -l <"$invocations")" == "$((ordered_before + 5))" ]]
[[ "$(sha256sum -- "$seed3_audit/traces.snapshot")" == "$snapshot_before" ]]
seed4_audit=$(audit_for "$ordered_audit" "$seed4" "$runner_sha")
audit_parent="$workspace/parity-save-replays/audits"
[[ "${seed3_audit%/*}" == "$audit_parent" ]]
[[ "${seed4_audit%/*}" == "$audit_parent" ]]
[[ ! -e "$audit_parent/campaigns" && ! -e "$audit_parent/runners" ]]
grep -Fxq 'EXACT_PARITY=1' "$seed3_audit/parity-verdict.env"
grep -Fxq 'EXACT_PARITY=1' "$seed4_audit/parity-verdict.env"
grep -Fxq "RUNNER_SHA256=$runner_sha" "$seed4_audit/parity-verdict.env"
grep -Fxq "TRACE_IDENTITIES_SHA256=$(sha256sum -- "$seed4_audit/traces.sha256" | cut -d' ' -f1)" \
    "$seed4_audit/parity-verdict.env"

# Replacing compressed bytes at the same canonical path invalidates the frozen
# identity before resume can trust the old status/log proof.
first_trace="$seed3/traces/save/01-pass-session-0001.jsonl.zst"
cp "$first_trace" "$test_root/trace.saved"
printf 'same path, different compressed bytes\n' >"$first_trace"
before=$(wc -l <"$invocations")
expect_failure_without_run "$before" \
    run_validation_repaired "$ordered_audit" "$runner" "$runner_sha" "$seed3" "$seed4"
[[ ! -f "$seed3_audit/parity-verdict.env" ]]
[[ ! -f "$seed4_audit/parity-verdict.env" ]]
[[ -f "$seed3_audit/parity-verdict.previous.env" ]]
mv "$test_root/trace.saved" "$first_trace"
run_validation_repaired "$ordered_audit" "$runner" "$runner_sha" "$seed3" "$seed4"
[[ "$(wc -l <"$invocations")" == "$before" ]]

# A frozen manifest is immutable on resume, even if every existing proof is
# exact. Restore it after the negative check for the proof-corruption cases.
cp "$seed3_audit/traces.snapshot" "$test_root/snapshot.saved"
sed -n '1p' "$seed3_audit/traces.snapshot" >>"$seed3_audit/traces.snapshot"
before=$(wc -l <"$invocations")
expect_failure_without_run "$before" \
    run_validation_repaired "$ordered_audit" "$runner" "$runner_sha" "$seed3" "$seed4"
[[ ! -f "$seed3_audit/parity-verdict.env" ]]
[[ ! -f "$seed4_audit/parity-verdict.env" ]]
mv "$test_root/snapshot.saved" "$seed3_audit/traces.snapshot"

# Existing proof must have canonical status bytes and exactly one anchored EOF
# marker. The lower-level sweep catches each before launching another replay.
first_status=$(status_for "$seed3_audit" "$first_trace")
first_log=$(log_for "$seed3_audit" "$first_trace")
cp "$first_status" "$test_root/status.saved"
cp "$first_log" "$test_root/log.saved"

printf '0\njunk\n' >"$first_status"
expect_failure_without_run "$before" \
    run_validation_repaired "$ordered_audit" "$runner" "$runner_sha" "$seed3" "$seed4"
[[ ! -f "$seed3_audit/parity-verdict.env" ]]
[[ -f "$seed3_audit/parity-verdict.previous.env" ]]
cp "$test_root/status.saved" "$first_status"

rm -f -- "$first_log"
expect_failure_without_run "$before" \
    run_validation_repaired "$ordered_audit" "$runner" "$runner_sha" "$seed3" "$seed4"
cp "$test_root/log.saved" "$first_log"

: >"$first_log"
expect_failure_without_run "$before" \
    run_validation_repaired "$ordered_audit" "$runner" "$runner_sha" "$seed3" "$seed4"
{
    printf '%s\n' 'parity trace matched every recorded frame'
    printf '%s\n' 'parity trace matched every recorded frame'
} >"$first_log"
expect_failure_without_run "$before" \
    run_validation_repaired "$ordered_audit" "$runner" "$runner_sha" "$seed3" "$seed4"
cp "$test_root/log.saved" "$first_log"

printf '17\n' >"$first_status"
expect_failure_without_run "$before" \
    run_validation_repaired "$ordered_audit" "$runner" "$runner_sha" "$seed3" "$seed4"
cp "$test_root/status.saved" "$first_status"

# Exact final set verification rejects an unrelated status even though every
# manifest entry itself has valid proof.
printf '0\n' >"$seed3_audit/status/unrelated.status"
expect_failure_without_run "$before" \
    run_validation_repaired "$ordered_audit" "$runner" "$runner_sha" "$seed3" "$seed4"
rm -f -- "$seed3_audit/status/unrelated.status"

# A second runner build gets a disjoint SHA-scoped audit; it cannot inherit the
# first runner's exact statuses. Corruption of the pinned copy is also fatal.
runner2="$test_root/fake-runner-2"
cp "$runner" "$runner2"
printf '\n# distinct pinned build\n' >>"$runner2"
chmod +x "$runner2"
runner2_sha=$(sha256sum -- "$runner2")
runner2_sha=${runner2_sha%% *}
before=$(wc -l <"$invocations")
FAKE_RUNNER_REPAIR=1 run_validation \
    "$ordered_audit" "$runner2" "$runner2_sha" "$seed3" "$seed4"
[[ "$(wc -l <"$invocations")" == "$((before + 4))" ]]
[[ -d "$(audit_for "$ordered_audit" "$seed3" "$runner2_sha")" ]]

pinned2="$workspace/.git/schema16-final-runners/$runner2_sha/original_parity_replay"
chmod 0755 "$pinned2"
printf '\n# corrupted after validation\n' >>"$pinned2"
before=$(wc -l <"$invocations")
expect_failure_without_run "$before" \
    run_validation_repaired "$ordered_audit" "$runner2" "$runner2_sha" "$seed3" "$seed4"
runner2_seed3_audit=$(audit_for "$ordered_audit" "$seed3" "$runner2_sha")
runner2_seed4_audit=$(audit_for "$ordered_audit" "$seed4" "$runner2_sha")
[[ ! -f "$runner2_seed3_audit/parity-verdict.env" ]]
[[ ! -f "$runner2_seed4_audit/parity-verdict.env" ]]
[[ -f "$runner2_seed3_audit/parity-verdict.previous.env" ]]
[[ -f "$runner2_seed4_audit/parity-verdict.previous.env" ]]

# Exercise the updater's own discovery primitive over the immediate audit
# children. Canonical campaign-prefixed trace keys are discovered; audit-local
# metadata and current/previous verdict files never become ledger keys.
python3 - \
    "$repository/scripts/update_permanent_eof_ledgers.py" \
    "$audit_parent" "$workspace" "$seed3" "$seed4" <<'PY'
from __future__ import annotations

import importlib.util
from pathlib import Path
import sys

module_path = Path(sys.argv[1])
audit_parent = Path(sys.argv[2])
workspace = Path(sys.argv[3])
campaigns = [Path(value) for value in sys.argv[4:]]
spec = importlib.util.spec_from_file_location("update_permanent_eof_ledgers", module_path)
assert spec is not None and spec.loader is not None
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

audit_roots = sorted(path for path in audit_parent.iterdir() if path.is_dir())
keys = module.exact_keys(audit_roots)
for campaign in campaigns:
    prefix = campaign.relative_to(workspace).as_posix().replace("/", "__") + "__"
    expected = {
        trace.relative_to(workspace).as_posix().replace("/", "__")
        for trace in (campaign / "traces").rglob("*.jsonl.zst")
    }
    discovered = {key for key in keys if key.startswith(prefix)}
    assert discovered == expected, (campaign, discovered, expected)

assert not any("parity-verdict" in key or "validation.env" in key for key in keys)
PY

if find "$workspace/parity-save-replays/audits" "$workspace/.git/schema16-final-runners" \
    -type f -name '*.tmp.*' -print -quit | grep -q .
then
    printf 'test failure: temporary validation artifact remained\n' >&2
    exit 1
fi

printf 'schema16 final validation tests passed\n'
