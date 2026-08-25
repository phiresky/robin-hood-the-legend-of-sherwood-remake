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
if [[ " $* " == *' --convert '* ]]; then
    if [[ ${trace##*/} == *convert-fail* \
        && ${FAKE_CONVERTER_REPAIR:-0} != 1 ]]; then
        printf '%s\n' 'deliberate conversion failure' >&2
        exit 29
    fi
    python3 - "$trace" "$trace.parity.bitcode.zst" <<'PY'
from pathlib import Path
import hashlib
import os
import sys

source = Path(sys.argv[1])
native = Path(sys.argv[2])
stat = source.stat()
digest = hashlib.sha256(source.read_bytes()).hexdigest()
native.write_text(
    f"fake-native:length={stat.st_size}:modified={stat.st_mtime_ns}:sha256={digest}\n"
)
os.unlink(source)
PY
    exit 0
fi
printf '%s\n' "$trace" >>"$FAKE_RUNNER_INVOCATIONS"
if [[ -n ${FAKE_RUNNER_STATE_DIR:-} ]]; then
    mkdir -p "$FAKE_RUNNER_STATE_DIR"
    exec 8>"$FAKE_RUNNER_STATE_DIR/lock"
    flock 8
    active=0
    [[ ! -f "$FAKE_RUNNER_STATE_DIR/active" ]] \
        || active=$(<"$FAKE_RUNNER_STATE_DIR/active")
    active=$((active + 1))
    printf '%s\n' "$active" >"$FAKE_RUNNER_STATE_DIR/active"
    maximum=0
    [[ ! -f "$FAKE_RUNNER_STATE_DIR/maximum" ]] \
        || maximum=$(<"$FAKE_RUNNER_STATE_DIR/maximum")
    if (( active > maximum )); then
        printf '%s\n' "$active" >"$FAKE_RUNNER_STATE_DIR/maximum"
    fi
    flock -u 8
    decrement_active() {
        flock 8
        active=$(<"$FAKE_RUNNER_STATE_DIR/active")
        printf '%s\n' "$((active - 1))" >"$FAKE_RUNNER_STATE_DIR/active"
        flock -u 8
    }
    trap decrement_active EXIT
fi
case "${trace##*/}" in
    *01-parallel-fail-session-*.jsonl.zst)
        if [[ ${FAKE_RUNNER_REPAIR:-0} != 1 ]]; then
            for _attempt in {1..100}; do
                [[ -f "$FAKE_RUNNER_STATE_DIR/inflight-started" ]] && break
                sleep 0.01
            done
            printf '%s\n' 'deliberate parallel replay failure' >&2
            exit 31
        fi
        ;;
    *02-parallel-inflight-session-*.jsonl.zst|*05-interrupt-session-*.jsonl.zst)
        : >"$FAKE_RUNNER_STATE_DIR/inflight-started"
        sleep "${FAKE_RUNNER_SLEEP:-0.3}"
        ;;
    *03-parallel-after-session-*.jsonl.zst|*04-parallel-after-session-*.jsonl.zst)
        ;;
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

add_native_trace() {
    local campaign=$1 stem=$2 logical
    mkdir -p "$campaign/traces/save"
    logical="$campaign/traces/save/$stem-session-0001.jsonl.zst"
    printf 'fake native trace\n' >"$logical.parity.bitcode.zst"
    : >"$campaign/traces/save/$stem.complete"
}

add_coexisting_trace() {
    local campaign=$1 stem=$2 logical
    mkdir -p "$campaign/traces/save"
    logical="$campaign/traces/save/$stem-session-0001.jsonl.zst"
    printf 'fake legacy trace\n' >"$logical"
    printf 'fake native trace\n' >"$logical.parity.bitcode.zst"
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
        SCHEMA16_FINAL_RUNNER_MODE="${FAKE_RUNNER_MODE:-direct}" \
        SCHEMA16_FINAL_RUNNER_BUNDLE_SHA256="${FAKE_BUNDLE_SHA:-}" \
        SCHEMA16_FINAL_OUTER_LOCK="$test_root/final-validation.lock" \
        "$validator" "$workspace" "$selected_runner" "$selected_sha" "$@"
}

run_bundle_validation() {
    FAKE_RUNNER_MODE=bundle FAKE_BUNDLE_SHA=$bundle_identity run_validation "$@"
}

run_bundle_with_identity() {
    local identity=$1
    shift
    FAKE_RUNNER_MODE=bundle FAKE_BUNDLE_SHA=$identity run_validation "$@"
}

run_validation_repaired() {
    FAKE_RUNNER_REPAIR=1 run_validation "$@"
}

run_validation_parallel_state() {
    local concurrency=$1 state_dir=$2 selected_runner=$3 selected_sha=$4
    shift 4
    env \
        FAKE_RUNNER_STATE_DIR="$state_dir" \
        SCHEMA16_FINAL_SWEEP_CONCURRENCY="$concurrency" \
        FAKE_RUNNER_INVOCATIONS="$invocations" \
        SCHEMA16_FINAL_RUNNER_MODE=direct \
        SCHEMA16_FINAL_OUTER_LOCK="$test_root/final-validation.lock" \
        "$validator" "$workspace" "$selected_runner" "$selected_sha" "$@"
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
held_outer=${SCHEMA16_TEST_OUTER_LOCK:-/tmp/robin-parity-runner.lock}
slot_dir="$workspace/.git/parity-runner-slots"
mkdir -p "$slot_dir"
exec {held_outer_fd}>"$held_outer"
flock "$held_outer_fd"
before=$(wc -l <"$invocations")
env \
    FAKE_RUNNER_INVOCATIONS="$invocations" \
    SCHEMA16_FINAL_RUNNER_MODE=direct \
    SCHEMA16_FINAL_OUTER_LOCK="$held_outer" \
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

# Conversion failure is preserved outside the not-yet-created audit and stops
# before any replay. A repaired resume is idempotent and completes normally.
conversion=$(make_campaign schema16-seed2900000-conversion-test 2900000 1)
add_trace "$conversion" 01-convert-fail
conversion_audit=$(audit_for "$test_root/audit-conversion" "$conversion" "$runner_sha")
before=$(wc -l <"$invocations")
expect_failure_without_run "$before" \
    run_validation "$test_root/audit-conversion" "$runner" "$runner_sha" "$conversion"
[[ ! -e "$conversion_audit" ]]
grep -Fxq '29' "$conversion_audit.conversion.status"
[[ -f "$conversion/traces/save/01-convert-fail-session-0001.jsonl.zst" ]]
FAKE_CONVERTER_REPAIR=1 run_validation \
    "$test_root/audit-conversion" "$runner" "$runner_sha" "$conversion"
[[ "$(wc -l <"$invocations")" == "$((before + 1))" ]]
[[ ! -e "$conversion/traces/save/01-convert-fail-session-0001.jsonl.zst" ]]
[[ -f "$conversion/traces/save/01-convert-fail-session-0001.jsonl.zst.parity.bitcode.zst" ]]

# Logical trace identity is independent of its physical representation. Native
# bitcode-only, legacy JSONL, and the lazy-conversion coexistence window each
# contribute exactly one manifest entry. The pinned converter normalizes the
# latter two before the immutable audit is initialized, including the source
# length, nanosecond mtime, and content hash in the resulting native bytes.
formats=$(make_campaign schema16-seed3000000-format-test 3000000 3)
add_trace "$formats" 01-legacy
add_native_trace "$formats" 02-native
add_coexisting_trace "$formats" 03-coexist
formats_before=$(wc -l <"$invocations")
run_validation "$test_root/audit-formats" "$runner" "$runner_sha" "$formats"
[[ "$(wc -l <"$invocations")" == "$((formats_before + 3))" ]]
formats_audit=$(audit_for "$test_root/audit-formats" "$formats" "$runner_sha")
native_logical="$formats/traces/save/02-native-session-0001.jsonl.zst"
coexist_logical="$formats/traces/save/03-coexist-session-0001.jsonl.zst"
grep -Fxq "$(sha256sum -- "$native_logical.parity.bitcode.zst" | cut -d' ' -f1)  $native_logical" \
    "$formats_audit/traces.sha256"
[[ ! -e "$formats/traces/save/01-legacy-session-0001.jsonl.zst" ]]
[[ ! -e "$coexist_logical" ]]
grep -Fxq "$(sha256sum -- "$coexist_logical.parity.bitcode.zst" | cut -d' ' -f1)  $coexist_logical" \
    "$formats_audit/traces.sha256"

# Replacing native bytes at the same physical path invalidates the frozen
# logical identity.
cp "$native_logical.parity.bitcode.zst" "$test_root/native.saved"
printf 'mutated native bytes\n' >"$native_logical.parity.bitcode.zst"
before=$(wc -l <"$invocations")
expect_failure_without_run "$before" \
    run_validation "$test_root/audit-formats" "$runner" "$runner_sha" "$formats"
[[ ! -f "$formats_audit/parity-verdict.env" ]]
mv "$test_root/native.saved" "$native_logical.parity.bitcode.zst"
run_validation "$test_root/audit-formats" "$runner" "$runner_sha" "$formats"
[[ "$(wc -l <"$invocations")" == "$before" ]]

cp "$coexist_logical.parity.bitcode.zst" "$test_root/coexist-native.saved"
printf 'fake legacy trace\n' >"$coexist_logical"
python3 - "$coexist_logical" <<'PY'
from pathlib import Path
import os
import sys

path = Path(sys.argv[1])
stat = path.stat()
os.utime(path, ns=(stat.st_atime_ns, stat.st_mtime_ns + 1_000_000_000))
PY
expect_failure_without_run "$before" \
    run_validation "$test_root/audit-formats" "$runner" "$runner_sha" "$formats"
[[ "$(wc -l <"$invocations")" == "$before" ]]
mv "$test_root/coexist-native.saved" "$coexist_logical.parity.bitcode.zst"
run_validation "$test_root/audit-formats" "$runner" "$runner_sha" "$formats"
[[ "$(wc -l <"$invocations")" == "$before" ]]

mv "$coexist_logical.parity.bitcode.zst" "$test_root/coexist-native.saved"
expect_failure_without_run "$before" \
    run_validation "$test_root/audit-formats" "$runner" "$runner_sha" "$formats"
[[ "$(wc -l <"$invocations")" == "$before" ]]
mv "$test_root/coexist-native.saved" "$coexist_logical.parity.bitcode.zst"
run_validation "$test_root/audit-formats" "$runner" "$runner_sha" "$formats"
[[ "$(wc -l <"$invocations")" == "$before" ]]

printf 'new legacy source beside native cache\n' >"$native_logical"
cp "$native_logical.parity.bitcode.zst" "$test_root/native-before-source.saved"
expect_failure_without_run "$before" \
    run_validation "$test_root/audit-formats" "$runner" "$runner_sha" "$formats"
[[ "$(wc -l <"$invocations")" == "$before" ]]
mv "$test_root/native-before-source.saved" "$native_logical.parity.bitcode.zst"
run_validation "$test_root/audit-formats" "$runner" "$runner_sha" "$formats"
[[ "$(wc -l <"$invocations")" == "$before" ]]

# A packaged runner is pinned and audited by its caller-authenticated composite
# identity while retaining the raw ELF/script hash as separate provenance.
# Direct runners above remain supported for focused local fixtures.
bundle="$test_root/fake-runner-bundle"
mkdir -p "$bundle/lib"
cp "$runner" "$bundle/original_parity_replay"
printf '\n# distinct bundled runner\n' >>"$bundle/original_parity_replay"
chmod +x "$bundle/original_parity_replay"
cat >"$bundle/original_parity_replay.remote" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
bundle_dir=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
exec "$bundle_dir/lib/ld-linux-x86-64.so.2" --library-path "$bundle_dir/lib" \
    "$bundle_dir/original_parity_replay" "$@"
EOF
chmod +x "$bundle/original_parity_replay.remote"
cat >"$bundle/lib/ld-linux-x86-64.so.2" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
[[ "$1" == --library-path ]]
[[ ! -v LD_LIBRARY_PATH ]]
shift 2
exec "$@"
EOF
chmod +x "$bundle/lib/ld-linux-x86-64.so.2"
printf 'fake provenance\nNATIVE_CONVERSION_PROTOCOL=2\n' >"$bundle/PROVENANCE.txt"
printf '/lib64/ld-linux-x86-64.so.2 => %s/lib/ld-linux-x86-64.so.2 (0x1)\n' \
    "$bundle" >"$bundle/LOADER_LIST.txt"
(
    cd "$bundle"
    sha256sum lib/ld-linux-x86-64.so.2 >LIB_SHA256SUMS
    sha256sum original_parity_replay original_parity_replay.remote LIB_SHA256SUMS \
        PROVENANCE.txt LOADER_LIST.txt >SHA256SUMS
)
bundle_sha=$(sha256sum -- "$bundle/original_parity_replay")
bundle_sha=${bundle_sha%% *}
bundle_main_manifest_sha=$(sha256sum -- "$bundle/SHA256SUMS" | cut -d' ' -f1)
bundle_lib_manifest_sha=$(sha256sum -- "$bundle/LIB_SHA256SUMS" | cut -d' ' -f1)
bundle_identity=$(printf 'schema16-runner-bundle-v1\nSHA256SUMS=%s\nLIB_SHA256SUMS=%s\n' \
    "$bundle_main_manifest_sha" "$bundle_lib_manifest_sha" \
    | sha256sum | cut -d' ' -f1)
bundle_campaign=$(make_campaign schema16-seed2950000-bundle-test 2950000 1)
add_native_trace "$bundle_campaign" 01-bundle-pass
legacy_bundle_pin="$workspace/.git/schema16-final-runners/$bundle_sha"
mkdir -p "$legacy_bundle_pin"
cp "$bundle/original_parity_replay" "$legacy_bundle_pin/original_parity_replay"
printf '%s  original_parity_replay\n' "$bundle_sha" >"$legacy_bundle_pin/runner.sha256"
bundle_before=$(wc -l <"$invocations")
run_bundle_validation "$test_root/audit-bundle" "$bundle" "$bundle_sha" "$bundle_campaign"
[[ "$(wc -l <"$invocations")" == "$((bundle_before + 1))" ]]
bundle_audit=$(audit_for "$test_root/audit-bundle" "$bundle_campaign" "$bundle_identity")
pinned_bundle="$workspace/.git/schema16-final-runners/$bundle_identity"
[[ -x "$pinned_bundle/original_parity_replay.remote" ]]
[[ -f "$legacy_bundle_pin/runner.sha256" ]]
grep -Fxq "RUNNER=$pinned_bundle/original_parity_replay.remote" \
    "$bundle_audit/parity-verdict.env"
grep -Fxq "RUNNER_SHA256=$bundle_sha" "$bundle_audit/parity-verdict.env"
grep -Fxq 'NATIVE_CONVERSION_PROTOCOL=2' "$bundle_audit/validation.env"
grep -Fxq 'NATIVE_CONVERSION_PROTOCOL=2' "$bundle_audit/parity-verdict.env"

cp "$bundle/PROVENANCE.txt" "$test_root/provenance.saved"
cp "$bundle/SHA256SUMS" "$test_root/protocol-manifest.saved"
printf 'fake provenance\nNATIVE_CONVERSION_PROTOCOL=1\n' >"$bundle/PROVENANCE.txt"
(
    cd "$bundle"
    sha256sum original_parity_replay original_parity_replay.remote LIB_SHA256SUMS \
        PROVENANCE.txt LOADER_LIST.txt >SHA256SUMS
)
old_protocol_manifest_sha=$(sha256sum -- "$bundle/SHA256SUMS" | cut -d' ' -f1)
old_protocol_identity=$(printf 'schema16-runner-bundle-v1\nSHA256SUMS=%s\nLIB_SHA256SUMS=%s\n' \
    "$old_protocol_manifest_sha" "$bundle_lib_manifest_sha" \
    | sha256sum | cut -d' ' -f1)
before=$(wc -l <"$invocations")
expect_failure_without_run "$before" \
    run_bundle_with_identity "$old_protocol_identity" \
        "$test_root/audit-bundle" "$bundle" "$bundle_sha" "$bundle_campaign"
mv "$test_root/provenance.saved" "$bundle/PROVENANCE.txt"
mv "$test_root/protocol-manifest.saved" "$bundle/SHA256SUMS"

printf 'unmanifested runtime input\n' >"$bundle/lib/libunexpected.so"
expect_failure_without_run "$before" \
    run_bundle_validation "$test_root/audit-bundle" "$bundle" "$bundle_sha" "$bundle_campaign"
rm -f -- "$bundle/lib/libunexpected.so"

mv "$bundle/lib/ld-linux-x86-64.so.2" "$test_root/loader.saved"
expect_failure_without_run "$before" \
    run_bundle_validation "$test_root/audit-bundle" "$bundle" "$bundle_sha" "$bundle_campaign"
[[ ! -f "$bundle_audit/parity-verdict.env" ]]
mv "$test_root/loader.saved" "$bundle/lib/ld-linux-x86-64.so.2"

cp "$bundle/original_parity_replay.remote" "$test_root/wrapper.saved"
printf '\n# tampered wrapper\n' >>"$bundle/original_parity_replay.remote"
expect_failure_without_run "$before" \
    run_bundle_validation "$test_root/audit-bundle" "$bundle" "$bundle_sha" "$bundle_campaign"
mv "$test_root/wrapper.saved" "$bundle/original_parity_replay.remote"
chmod +x "$bundle/original_parity_replay.remote"

cp "$bundle/SHA256SUMS" "$test_root/bundle-manifest.saved"
cp "$bundle/original_parity_replay.remote" "$test_root/wrapper.saved"
printf '\n# re-manifested malicious wrapper\n' >>"$bundle/original_parity_replay.remote"
(
    cd "$bundle"
    sha256sum original_parity_replay original_parity_replay.remote LIB_SHA256SUMS \
        PROVENANCE.txt LOADER_LIST.txt >SHA256SUMS
)
expect_failure_without_run "$before" \
    run_bundle_validation "$test_root/audit-bundle" "$bundle" "$bundle_sha" "$bundle_campaign"
mv "$test_root/wrapper.saved" "$bundle/original_parity_replay.remote"
chmod +x "$bundle/original_parity_replay.remote"
mv "$test_root/bundle-manifest.saved" "$bundle/SHA256SUMS"

mv "$bundle/original_parity_replay.remote" "$test_root/wrapper.saved"
ln -s "$test_root/wrapper.saved" "$bundle/original_parity_replay.remote"
expect_failure_without_run "$before" \
    run_bundle_validation "$test_root/audit-bundle" "$bundle" "$bundle_sha" "$bundle_campaign"
rm -f -- "$bundle/original_parity_replay.remote"
mv "$test_root/wrapper.saved" "$bundle/original_parity_replay.remote"
chmod +x "$bundle/original_parity_replay.remote"

run_bundle_validation "$test_root/audit-bundle" "$bundle" "$bundle_sha" "$bundle_campaign"
[[ "$(wc -l <"$invocations")" == "$before" ]]

chmod u+w "$pinned_bundle/lib/ld-linux-x86-64.so.2"
printf '\n# corrupted pinned loader\n' >>"$pinned_bundle/lib/ld-linux-x86-64.so.2"
expect_failure_without_run "$before" \
    run_bundle_validation "$test_root/audit-bundle" "$bundle" "$bundle_sha" "$bundle_campaign"
[[ ! -f "$bundle_audit/parity-verdict.env" ]]
cp "$bundle/lib/ld-linux-x86-64.so.2" \
    "$pinned_bundle/lib/ld-linux-x86-64.so.2"
chmod +x "$pinned_bundle/lib/ld-linux-x86-64.so.2"
run_bundle_validation "$test_root/audit-bundle" "$bundle" "$bundle_sha" "$bundle_campaign"
[[ "$(wc -l <"$invocations")" == "$before" ]]

# A later shard can publish the real trigger while an earlier trace is still
# waiting for its claim. Failure classification must authenticate that
# published nonzero proof instead of blaming the intentionally unstarted entry.
sparse_campaign=$(make_campaign schema16-seed2960000-sparse-failure-test 2960000 2)
add_trace "$sparse_campaign" 01-pass
add_trace "$sparse_campaign" 02-fail
sparse_before=$(wc -l <"$invocations")
if SCHEMA16_FINAL_SWEEP_CONCURRENCY=1 \
    run_validation unused "$runner" "$runner_sha" "$sparse_campaign"
then
    printf 'test failure: sparse fixture setup accepted its failure\n' >&2
    exit 1
fi
sparse_audit=$(audit_for unused "$sparse_campaign" "$runner_sha")
sparse_pass="$sparse_campaign/traces/save/01-pass-session-0001.jsonl.zst"
sparse_fail="$sparse_campaign/traces/save/02-fail-session-0001.jsonl.zst"
rm -f -- \
    "$(status_for "$sparse_audit" "$sparse_pass")" \
    "$(log_for "$sparse_audit" "$sparse_pass")" \
    "$(status_for "$sparse_audit" "$sparse_fail")" \
    "$(log_for "$sparse_audit" "$sparse_fail")"
sparse_full_key=${sparse_pass//\//__}
exec {sparse_lock_fd}>"$sparse_audit/.trace-locks/$sparse_full_key.lock"
flock "$sparse_lock_fd"
env \
    FAKE_RUNNER_INVOCATIONS="$invocations" \
    SCHEMA16_FINAL_SWEEP_CONCURRENCY=2 \
    SCHEMA16_FINAL_RUNNER_MODE=direct \
    SCHEMA16_FINAL_OUTER_LOCK="$test_root/final-validation.lock" \
    "$validator" "$workspace" "$runner" "$runner_sha" "$sparse_campaign" \
        >"$test_root/sparse-validation.log" 2>&1 &
locked_pid=$!
sparse_failure_published=0
for _attempt in {1..200}; do
    if [[ -f "$(status_for "$sparse_audit" "$sparse_fail")" ]]; then
        sparse_failure_published=1
        break
    fi
    sleep 0.01
done
[[ "$sparse_failure_published" == 1 ]]
flock -u "$sparse_lock_fd"
exec {sparse_lock_fd}>&-
if wait "$locked_pid"; then
    printf 'test failure: sparse parallel failure was accepted\n' >&2
    exit 1
fi
locked_pid=
[[ "$(wc -l <"$invocations")" == "$((sparse_before + 3))" ]]
[[ ! -e "$(status_for "$sparse_audit" "$sparse_pass")" ]]
grep -Fxq 'CLASSIFICATION=nonzero-or-malformed-status' \
    "$sparse_audit/parity-last-failure.env"
grep -Fxq 'STATUS=23' "$sparse_audit/parity-last-failure.env"
grep -Fxq "TRACE=$sparse_fail" "$sparse_audit/parity-last-failure.env"
cp "$sparse_audit/.parallel-fail-fast-stop" "$test_root/sparse-stop.saved"
cp "$sparse_audit/sweep-launch.env" "$test_root/sparse-launch.saved"
cp "$(log_for "$sparse_audit" "$sparse_fail")" "$test_root/sparse-fail-log.saved"

# Without a canonical shared-stop artifact, the later nonzero proof has no
# authenticated fail-fast precedence over the earlier missing proof.
printf 'not-a-canonical-stop\n' >"$sparse_audit/.parallel-fail-fast-stop"
sparse_invocations=$(wc -l <"$invocations")
if SCHEMA16_FINAL_SWEEP_CONCURRENCY=2 \
    run_validation unused "$runner" "$runner_sha" "$sparse_campaign"
then
    printf 'test failure: invalid sparse stop admitted a resume\n' >&2
    exit 1
fi
[[ "$(wc -l <"$invocations")" == "$sparse_invocations" ]]
grep -Fxq 'CLASSIFICATION=missing-status' \
    "$sparse_audit/parity-last-failure.env"
grep -Fxq "TRACE=$sparse_pass" "$sparse_audit/parity-last-failure.env"

cp "$test_root/sparse-stop.saved" "$sparse_audit/.parallel-fail-fast-stop"
sed 's/^FAIL_FAST_BATCH_TOKEN=.*/FAIL_FAST_BATCH_TOKEN=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/' \
    "$test_root/sparse-launch.saved" >"$sparse_audit/sweep-launch.env"
if SCHEMA16_FINAL_SWEEP_CONCURRENCY=2 \
    run_validation unused "$runner" "$runner_sha" "$sparse_campaign"
then
    printf 'test failure: mismatched sparse launch token was authenticated\n' >&2
    exit 1
fi
grep -Fxq 'CLASSIFICATION=missing-status' \
    "$sparse_audit/parity-last-failure.env"
grep -Fxq "TRACE=$sparse_pass" "$sparse_audit/parity-last-failure.env"
cp "$test_root/sparse-launch.saved" "$sparse_audit/sweep-launch.env"

# A valid stop token cannot elevate malformed or half-published evidence.
cp "$test_root/sparse-stop.saved" "$sparse_audit/.parallel-fail-fast-stop"
printf '23\njunk\n' >"$(status_for "$sparse_audit" "$sparse_fail")"
if SCHEMA16_FINAL_SWEEP_CONCURRENCY=2 \
    run_validation unused "$runner" "$runner_sha" "$sparse_campaign"
then
    printf 'test failure: malformed sparse trigger was authenticated\n' >&2
    exit 1
fi
grep -Fxq 'CLASSIFICATION=missing-status' \
    "$sparse_audit/parity-last-failure.env"
grep -Fxq "TRACE=$sparse_pass" "$sparse_audit/parity-last-failure.env"
printf '999\n' >"$(status_for "$sparse_audit" "$sparse_fail")"
if SCHEMA16_FINAL_SWEEP_CONCURRENCY=2 \
    run_validation unused "$runner" "$runner_sha" "$sparse_campaign"
then
    printf 'test failure: out-of-range sparse trigger was authenticated\n' >&2
    exit 1
fi
grep -Fxq 'CLASSIFICATION=missing-status' \
    "$sparse_audit/parity-last-failure.env"
grep -Fxq "TRACE=$sparse_pass" "$sparse_audit/parity-last-failure.env"
printf '23\n' >"$(status_for "$sparse_audit" "$sparse_fail")"
rm -f -- "$(log_for "$sparse_audit" "$sparse_fail")"
if SCHEMA16_FINAL_SWEEP_CONCURRENCY=2 \
    run_validation unused "$runner" "$runner_sha" "$sparse_campaign"
then
    printf 'test failure: half-published sparse trigger was authenticated\n' >&2
    exit 1
fi
grep -Fxq 'CLASSIFICATION=missing-status' \
    "$sparse_audit/parity-last-failure.env"
grep -Fxq "TRACE=$sparse_pass" "$sparse_audit/parity-last-failure.env"
cp "$test_root/sparse-fail-log.saved" "$(log_for "$sparse_audit" "$sparse_fail")"

# Parallel fail-fast permits already-running work to publish, but starts no
# later shard item after the first failure. Resume rejects the preserved
# failure without launching anything; after explicit repair it skips the prior
# exact proof and safely resumes with a different resource concurrency.
parallel_campaign=$(make_campaign schema16-seed2970000-parallel-test 2970000 4)
add_trace "$parallel_campaign" 01-parallel-fail
add_trace "$parallel_campaign" 02-parallel-inflight
add_trace "$parallel_campaign" 03-parallel-after
add_trace "$parallel_campaign" 04-parallel-after
parallel_state="$test_root/parallel-state"
mkdir -p "$parallel_state"
parallel_before=$(wc -l <"$invocations")
if run_validation_parallel_state 2 "$parallel_state" \
    "$runner" "$runner_sha" "$parallel_campaign"
then
    printf 'test failure: parallel validation accepted a failing trace\n' >&2
    exit 1
fi
if [[ "$(wc -l <"$invocations")" != "$((parallel_before + 2))" ]]; then
    printf 'test failure: a post-stop parallel trace was invoked\n' >&2
    tail -n 4 "$invocations" >&2
    exit 1
fi
[[ "$(<"$parallel_state/maximum")" == 2 ]]
parallel_audit=$(audit_for unused "$parallel_campaign" "$runner_sha")
parallel_failed="$parallel_campaign/traces/save/01-parallel-fail-session-0001.jsonl.zst"
parallel_inflight="$parallel_campaign/traces/save/02-parallel-inflight-session-0001.jsonl.zst"
parallel_after3="$parallel_campaign/traces/save/03-parallel-after-session-0001.jsonl.zst"
parallel_after4="$parallel_campaign/traces/save/04-parallel-after-session-0001.jsonl.zst"
[[ "$(<"$(status_for "$parallel_audit" "$parallel_failed")")" == 31 ]]
parallel_inflight_status=$(status_for "$parallel_audit" "$parallel_inflight")
if [[ "$(<"$parallel_inflight_status")" != 0 ]]; then
    printf 'test failure: in-flight trace did not publish exact status\n' >&2
    cat -- "$(log_for "$parallel_audit" "$parallel_inflight")" >&2
    exit 1
fi
[[ ! -e "$(status_for "$parallel_audit" "$parallel_after3")" ]]
[[ ! -e "$(status_for "$parallel_audit" "$parallel_after4")" ]]
[[ -f "$parallel_audit/.parallel-fail-fast-stop" ]]
if run_validation_parallel_state 2 "$parallel_state" \
    "$runner" "$runner_sha" "$parallel_campaign"
then
    printf 'test failure: parallel resume crossed a preserved failure\n' >&2
    exit 1
fi
[[ "$(wc -l <"$invocations")" == "$((parallel_before + 2))" ]]
rm -f -- \
    "$(status_for "$parallel_audit" "$parallel_failed")" \
    "$(log_for "$parallel_audit" "$parallel_failed")"
FAKE_RUNNER_REPAIR=1 run_validation_parallel_state 3 "$parallel_state" \
    "$runner" "$runner_sha" "$parallel_campaign"
[[ "$(wc -l <"$invocations")" == "$((parallel_before + 5))" ]]
[[ "$(grep -Fxc -- "$parallel_inflight" "$invocations")" == 1 ]]
[[ ! -e "$parallel_audit/.parallel-fail-fast-stop" ]]
grep -Fxq 'SWEEP_CONCURRENCY=3' "$parallel_audit/sweep-launch.env"
grep -Fxq 'SWEEP_CONCURRENCY=3' "$parallel_audit/parity-verdict.env"
! grep -Fq 'SWEEP_CONCURRENCY=' "$parallel_audit/validation.env"
if find "$parallel_audit" -type f -name '*.tmp.*' -print -quit | grep -q .; then
    printf 'test failure: parallel audit retained a temporary file\n' >&2
    exit 1
fi

# TERM reaps every worker process group. An interrupted in-flight trace leaves
# neither a verdict nor a partially-published status/log or private temp file.
interrupt_campaign=$(make_campaign schema16-seed2980000-interrupt-test 2980000 1)
add_trace "$interrupt_campaign" 05-interrupt
interrupt_state="$test_root/interrupt-state"
mkdir -p "$interrupt_state"
env \
    FAKE_RUNNER_SLEEP=30 \
    FAKE_RUNNER_STATE_DIR="$interrupt_state" \
    FAKE_RUNNER_INVOCATIONS="$invocations" \
    SCHEMA16_FINAL_SWEEP_CONCURRENCY=2 \
    SCHEMA16_FINAL_RUNNER_MODE=direct \
    SCHEMA16_FINAL_OUTER_LOCK="$test_root/final-validation.lock" \
    "$validator" "$workspace" "$runner" "$runner_sha" "$interrupt_campaign" \
        >"$test_root/interrupt-validation.log" 2>&1 &
locked_pid=$!
interrupt_started=0
for _attempt in {1..200}; do
    if [[ -f "$interrupt_state/inflight-started" ]]; then
        interrupt_started=1
        break
    fi
    sleep 0.01
done
[[ "$interrupt_started" == 1 ]]
kill -TERM "$locked_pid"
if wait "$locked_pid"; then
    printf 'test failure: interrupted validation exited successfully\n' >&2
    exit 1
fi
locked_pid=
interrupt_audit=$(audit_for unused "$interrupt_campaign" "$runner_sha")
[[ ! -f "$interrupt_audit/parity-verdict.env" ]]
[[ -z "$(find "$interrupt_audit/logs" "$interrupt_audit/status" -type f -print -quit)" ]]
[[ -z "$(find "$interrupt_audit" -type f -name '*.tmp.*' -print -quit)" ]]
if INTERRUPT_TARGET="$interrupt_campaign" INTERRUPT_RUNNER="$runner" python3 - <<'PY'
import os
from pathlib import Path

target = os.environ["INTERRUPT_TARGET"].encode()
runner = os.environ["INTERRUPT_RUNNER"].encode()
ancestors = set()
pid = os.getpid()
while pid > 1:
    ancestors.add(pid)
    try:
        fields = Path(f"/proc/{pid}/stat").read_text().split()
    except (FileNotFoundError, PermissionError):
        break
    pid = int(fields[3])
for entry in Path("/proc").iterdir():
    if not entry.name.isdigit() or int(entry.name) in ancestors:
        continue
    try:
        command = (entry / "cmdline").read_bytes()
    except (FileNotFoundError, PermissionError, ProcessLookupError):
        continue
    if target in command and runner in command:
        raise SystemExit(0)
raise SystemExit(1)
PY
then
    printf 'test failure: interrupted validation left a runner process\n' >&2
    exit 1
fi

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
ordered_failed_count=$(wc -l <"$invocations")
(( ordered_failed_count >= ordered_before + 1 \
    && ordered_failed_count <= ordered_before + 2 ))
! grep -Fq -- "$seed4/" "$invocations"
seed3_audit=$(audit_for "$ordered_audit" "$seed3" "$runner_sha")
[[ -f "$seed3_audit/parity-last-failure.env" ]]
grep -Eq 'CLASSIFICATION=(nonzero-or-malformed-status|missing-status)' \
    "$seed3_audit/parity-last-failure.env"
snapshot_before=$(sha256sum -- "$seed3_audit/traces.snapshot")

# Resume stops at preserved failure without launching work. Clearing only that
# diagnosed result lets seed3 finish, then and only then starts seed4.
if run_validation "$ordered_audit" "$runner" "$runner_sha" "$seed3" "$seed4"; then
    printf 'test failure: resume accepted preserved seed3 failure\n' >&2
    exit 1
fi
[[ "$(wc -l <"$invocations")" == "$ordered_failed_count" ]]
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
first_physical="$first_trace.parity.bitcode.zst"
cp "$first_physical" "$test_root/trace.saved"
printf 'same path, different compressed bytes\n' >"$first_physical"
before=$(wc -l <"$invocations")
expect_failure_without_run "$before" \
    run_validation_repaired "$ordered_audit" "$runner" "$runner_sha" "$seed3" "$seed4"
[[ ! -f "$seed3_audit/parity-verdict.env" ]]
[[ ! -f "$seed4_audit/parity-verdict.env" ]]
[[ -f "$seed3_audit/parity-verdict.previous.env" ]]
mv "$test_root/trace.saved" "$first_physical"
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
    "$audit_parent" "$workspace" "$formats" "$seed3" "$seed4" <<'PY'
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
        trace.relative_to(workspace).as_posix()
        .removesuffix(".parity.bitcode.zst")
        .replace("/", "__")
        for trace in (campaign / "traces").rglob("*.jsonl.zst*")
        if trace.name.endswith((".jsonl.zst", ".jsonl.zst.parity.bitcode.zst"))
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
