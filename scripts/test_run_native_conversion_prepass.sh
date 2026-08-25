#!/usr/bin/env bash
set -euo pipefail

repository=$(pwd)
mkdir -p "$repository/.codex-tmp"
test_root=$(mktemp -d "$repository/.codex-tmp/native-conversion-prepass-test.XXXXXX")
workspace=$test_root
cleanup() {
    if [[ -n ${parallel_barrier:-} && -d "$parallel_barrier" ]]; then
        : >"$parallel_barrier/release"
    fi
    if [[ -n ${parallel_pid:-} ]]; then
        wait "$parallel_pid" 2>/dev/null || true
    fi
    rm -rf -- "$test_root"
}
trap cleanup EXIT

runner="$test_root/fake-runner"
invocations="$test_root/invocations"
: >"$invocations"
cat >"$runner" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
[[ "$1" == --convert ]]
trace=$2
quarantine="$trace.parity-conversion-source"
printf '%s\n' "$trace" >>"$FAKE_CONVERT_INVOCATIONS"
if [[ ! -e "$trace" && -e "$quarantine" ]]; then
    [[ -f "$trace.parity.bitcode.zst" ]]
    rm -f -- "$quarantine"
    printf 'recovered conversion of %s; deleted pending source\n' "$trace"
    exit 0
fi
if [[ ! -e "$trace" ]]; then
    [[ -f "$trace.parity.bitcode.zst" ]]
    printf 'recording %s was already converted\n' "$trace"
    exit 0
fi
if [[ "$trace" == *convert-fail* && ${FAKE_CONVERT_REPAIR:-0} != 1 ]]; then
    printf 'deliberate conversion failure\n' >&2
    exit 23
fi
if [[ "$trace" == *delete-fail* ]]; then
    printf 'native after destructive failure\n' >"$trace.parity.bitcode.zst"
    rm -f -- "$trace"
    printf 'deliberate post-delete failure\n' >&2
    exit 24
fi
if [[ "$trace" == *mutate-bundle* && -n ${FAKE_MUTATE_BUNDLE_WRAPPER:-} ]]; then
    printf 'tampered during conversion\n' >>"$FAKE_MUTATE_BUNDLE_WRAPPER"
fi
if [[ -n ${FAKE_CONVERT_BARRIER_DIR:-} ]]; then
    mkdir -p -- "$FAKE_CONVERT_BARRIER_DIR/active"
    : >"$FAKE_CONVERT_BARRIER_DIR/active/$BASHPID"
    while [[ ! -e "$FAKE_CONVERT_BARRIER_DIR/release" ]]; do
        sleep 0.05
    done
    rm -f -- "$FAKE_CONVERT_BARRIER_DIR/active/$BASHPID"
fi
printf 'native:%s\n' "$(<"$trace")" >"$trace.parity.bitcode.zst"
rm -f -- "$trace"
printf 'converted %s; deleted the recording\n' "$trace"
EOF
chmod +x "$runner"
runner_sha=$(sha256sum "$runner"); runner_sha=${runner_sha%% *}

make_corpus() {
    local corpus=$1
    mkdir -p "$corpus/traces/save" "$corpus/.capture-reservations"
    : >"$corpus/.capture-admission.lock"
    : >"$corpus/.distributed-collector.lock"
}

add_source() {
    local corpus=$1 name=$2
    printf 'source:%s\n' "$name" >"$corpus/traces/save/$name-session-0001.jsonl.zst"
    : >"$corpus/traces/save/$name.complete"
}

run_prepass() {
    local corpus=$1 audit=$2
    local selected_runner=${TEST_RUNNER:-$runner}
    local selected_sha=${TEST_RUNNER_SHA:-$runner_sha}
    shift 2
    env FAKE_CONVERT_INVOCATIONS="$invocations" \
        FAKE_LOADER_INVOCATIONS="${FAKE_LOADER_INVOCATIONS:-}" \
        FAKE_MUTATE_BUNDLE_WRAPPER="${FAKE_MUTATE_BUNDLE_WRAPPER:-}" \
        FAKE_CONVERT_BARRIER_DIR="${FAKE_CONVERT_BARRIER_DIR:-}" \
        NATIVE_CONVERT_JOBS=2 \
        NATIVE_CONVERT_TIMEOUT_SECONDS=3600 \
        NATIVE_CONVERT_OUTER_LOCK="$test_root/global.lock" \
        NATIVE_CONVERT_MIN_FREE_KIB=0 \
        NATIVE_CONVERT_MIN_AVAILABLE_KIB_PER_JOB=0 \
        NATIVE_CONVERT_TEST_ALLOW_DIRECT_RUNNER=1 \
        NATIVE_CONVERT_TEST_MEMINFO=/proc/meminfo \
        "$@" "$repository/scripts/run_native_conversion_prepass.sh" \
            "$workspace" "$corpus" "$selected_runner" "$selected_sha" "$audit"
}

# Eight jobs are allowed only in globally drained mode, are all admitted at
# once for independent traces, and still charge the configured memory reserve
# once per worker. Nine jobs and multi-corpus parallel conversion are rejected.
parallel_corpus="$test_root/corpus-parallel-eight"
parallel_audit="$test_root/audit-parallel-eight"
parallel_barrier="$test_root/parallel-eight-barrier"
make_corpus "$parallel_corpus"
mkdir -p "$parallel_barrier/active"
for index in {01..09}; do
    add_source "$parallel_corpus" "$index-parallel"
done
FAKE_CONVERT_BARRIER_DIR="$parallel_barrier" \
    run_prepass "$parallel_corpus" "$parallel_audit" NATIVE_CONVERT_JOBS=0008 &
parallel_pid=$!
parallel_count=0
for _ in {1..200}; do
    parallel_count=$(find "$parallel_barrier/active" -type f 2>/dev/null | wc -l)
    (( parallel_count == 8 )) && break
    sleep 0.05
done
if (( parallel_count != 8 )); then
    : >"$parallel_barrier/release"
    wait "$parallel_pid" || true
    parallel_pid=
    printf 'test failure: eight conversion workers were not admitted concurrently (saw %s)\n' \
        "$parallel_count" >&2
    exit 1
fi
: >"$parallel_barrier/release"
wait "$parallel_pid"
parallel_pid=
[[ -f "$parallel_audit/COMPLETE" ]]
[[ "$(wc -l <"$parallel_audit/native.SHA256SUMS")" == 9 ]]
grep -Fxq 'JOBS=8' "$parallel_audit/provenance.env"

limit_corpus="$test_root/corpus-job-limit"
make_corpus "$limit_corpus"
add_source "$limit_corpus" 01-limit
before=$(wc -l <"$invocations")
if run_prepass "$limit_corpus" "$test_root/audit-job-limit" NATIVE_CONVERT_JOBS=9; then
    printf 'test failure: accepted more than eight conversion jobs\n' >&2
    exit 1
fi
for huge_jobs in 18446744073709551616 999999999999999999999999999999999999999999; do
    if run_prepass "$limit_corpus" "$test_root/audit-job-limit-$huge_jobs" \
        NATIVE_CONVERT_JOBS="$huge_jobs"
    then
        printf 'test failure: accepted overflowing conversion job count %s\n' \
            "$huge_jobs" >&2
        exit 1
    fi
done
if run_prepass "$limit_corpus" "$test_root/audit-job-leading-zero-nine" \
    NATIVE_CONVERT_JOBS=00000000000000000000000000000000000009
then
    printf 'test failure: accepted over-limit conversion jobs hidden by leading zeroes\n' >&2
    exit 1
fi
if run_prepass "$limit_corpus" "$test_root/audit-other-corpora-parallel" \
    NATIVE_CONVERT_JOBS=8 NATIVE_CONVERT_ALLOW_OTHER_CORPORA=1
then
    printf 'test failure: accepted parallel conversion in multi-corpus mode\n' >&2
    exit 1
fi
[[ "$(wc -l <"$invocations")" == "$before" ]]

test_meminfo="$test_root/meminfo"
printf 'MemTotal:       100000 kB\nMemAvailable:    90000 kB\n' >"$test_meminfo"
# One 20,000-KiB worker fits the injected 90,000-KiB availability, while eight
# require 160,000 KiB. The fixture is accepted only with the direct-runner test
# escape hatch, so production admission remains tied to /proc/meminfo.
if run_prepass "$limit_corpus" "$test_root/audit-memory-limit" \
    NATIVE_CONVERT_JOBS=8 \
    NATIVE_CONVERT_MIN_AVAILABLE_KIB_PER_JOB=20000 \
    NATIVE_CONVERT_TEST_MEMINFO="$test_meminfo"
then
    printf 'test failure: eight jobs bypassed the per-worker memory gate\n' >&2
    exit 1
fi
[[ "$(wc -l <"$invocations")" == "$before" ]]

if run_prepass "$limit_corpus" "$test_root/audit-memory-overflow" \
    NATIVE_CONVERT_JOBS=8 \
    NATIVE_CONVERT_MIN_AVAILABLE_KIB_PER_JOB=1152921504606846976
then
    printf 'test failure: accepted a per-worker memory reserve that overflows eight-job arithmetic\n' >&2
    exit 1
fi
[[ "$(wc -l <"$invocations")" == "$before" ]]

disk_total_kib=$(df -Pk -- "$limit_corpus" | awk 'NR == 2 {print $2}')
if run_prepass "$limit_corpus" "$test_root/audit-disk-limit" \
    NATIVE_CONVERT_MIN_FREE_KIB="$((disk_total_kib + 1))"
then
    printf 'test failure: bypassed free-disk admission above filesystem capacity\n' >&2
    exit 1
fi
if run_prepass "$limit_corpus" "$test_root/audit-disk-overflow" \
    NATIVE_CONVERT_MIN_FREE_KIB=9223372036854775808
then
    printf 'test failure: accepted a free-disk reserve outside signed arithmetic\n' >&2
    exit 1
fi
[[ "$(wc -l <"$invocations")" == "$before" ]]
: >"$invocations"

corpus="$test_root/corpus-pass"
audit="$test_root/audit-pass"
make_corpus "$corpus"
add_source "$corpus" 01-pass
add_source "$corpus" 02-coexist
printf 'old-native\n' >"$corpus/traces/save/02-coexist-session-0001.jsonl.zst.parity.bitcode.zst"
: >"$corpus/traces/save/03-native.complete"
printf 'native-only\n' >"$corpus/traces/save/03-native-session-0001.jsonl.zst.parity.bitcode.zst"
printf 'preserve partial\n' >"$corpus/traces/save/.tmp-do-not-delete"
before=$(wc -l <"$invocations")
if run_prepass "$corpus" "$audit" NATIVE_CONVERT_TEST_ALLOW_DIRECT_RUNNER=0; then
    printf 'test failure: operational mode accepted a raw runner without protocol proof\n' >&2
    exit 1
fi
[[ "$(wc -l <"$invocations")" == "$before" ]]
run_prepass "$corpus" "$audit"
[[ "$(wc -l <"$audit/logical.snapshot")" == 3 ]]
[[ "$(wc -l <"$audit/sources.snapshot")" == 2 ]]
[[ "$(wc -l <"$audit/key-map.tsv")" == 3 ]]
[[ "$(wc -l <"$audit/native.SHA256SUMS")" == 3 ]]
[[ "$(wc -l <"$audit/failures.tsv")" == 1 ]]
[[ -f "$audit/COMPLETE" ]]
[[ "$(wc -l <"$invocations")" == 2 ]]
[[ ! -e "$corpus/traces/save/01-pass-session-0001.jsonl.zst" ]]
[[ -f "$corpus/traces/save/01-pass-session-0001.jsonl.zst.parity.bitcode.zst" ]]
[[ -f "$corpus/traces/save/.tmp-do-not-delete" ]]

# A completed resume verifies frozen inputs and status-zero postconditions
# without invoking the converter again.
run_prepass "$corpus" "$audit"
[[ "$(wc -l <"$invocations")" == 2 ]]

# A stale status-zero proof cannot bless a source that reappeared. The prior
# completion marker is retired before the failed recheck.
pass_logical="$corpus/traces/save/01-pass-session-0001.jsonl.zst"
printf 'unexpected restored source\n' >"$pass_logical"
if run_prepass "$corpus" "$audit"; then
    printf 'test failure: status-zero resume accepted a restored source\n' >&2
    exit 1
fi
[[ ! -e "$audit/COMPLETE" ]]
[[ -f "$audit/COMPLETE.previous" ]]
rm -f -- "$pass_logical"
run_prepass "$corpus" "$audit"
[[ -f "$audit/COMPLETE" ]]
[[ "$(wc -l <"$invocations")" == 2 ]]

# A crash after the first native manifest rename but before COMPLETE is
# recoverable only from the exact frozen path set and artifact hashes.
anchor_corpus="$test_root/corpus-first-anchor"
anchor_audit="$test_root/audit-first-anchor"
make_corpus "$anchor_corpus"
add_source "$anchor_corpus" 01-anchor
run_prepass "$anchor_corpus" "$anchor_audit"
rm -f -- "$anchor_audit/COMPLETE"
[[ ! -e "$anchor_audit/COMPLETE.previous" ]]
before=$(wc -l <"$invocations")
run_prepass "$anchor_corpus" "$anchor_audit"
[[ "$(wc -l <"$invocations")" == "$before" ]]
[[ -f "$anchor_audit/COMPLETE" && -f "$anchor_audit/COMPLETE.previous" ]]

# A completed audit verifies its old manifest before it can publish a new one.
printf 'tamper\n' >>"$corpus/traces/save/03-native-session-0001.jsonl.zst.parity.bitcode.zst"
if run_prepass "$corpus" "$audit"; then
    printf 'test failure: completed resume accepted native tampering\n' >&2
    exit 1
fi
[[ ! -e "$audit/COMPLETE" ]]
# Regenerating the manifest alongside the tampered artifact cannot replace the
# manifest digest retained in COMPLETE.previous.
while IFS= read -r logical; do
    sha256sum "$logical.parity.bitcode.zst"
done <"$audit/logical.snapshot" >"$audit/native.SHA256SUMS.regenerated"
mv -f -- "$audit/native.SHA256SUMS.regenerated" "$audit/native.SHA256SUMS"
if run_prepass "$corpus" "$audit"; then
    printf 'test failure: retained anchor accepted regenerated tamper manifest\n' >&2
    exit 1
fi
[[ ! -e "$audit/COMPLETE" ]]

# Nonzero attempts remain in the failure ledger without a completion marker;
# a repaired resume creates attempt two without deleting attempt one's log.
failed_corpus="$test_root/corpus-fail"
failed_audit="$test_root/audit-fail"
make_corpus "$failed_corpus"
add_source "$failed_corpus" 01-convert-fail
if run_prepass "$failed_corpus" "$failed_audit"; then
    printf 'test failure: accepted failed conversion\n' >&2
    exit 1
fi
grep -Fq $'23\t1\t' "$failed_audit/failures.tsv"
[[ ! -e "$failed_audit/COMPLETE" ]]
find "$failed_audit/logs" -type f -name '*.attempt-0001.log' -print -quit | grep -q .
FAKE_CONVERT_REPAIR=1 run_prepass "$failed_corpus" "$failed_audit"
[[ -f "$failed_audit/COMPLETE" ]]
find "$failed_audit/logs" -type f -name '*.attempt-0001.log' -print -quit | grep -q .
find "$failed_audit/logs" -type f -name '*.attempt-0002.log' -print -quit | grep -q .

# A crash after quarantine publication must be recovered by invoking the
# pinned converter again. Path observations alone never become success proof.
crash_corpus="$test_root/corpus-crash"
crash_audit="$test_root/audit-crash"
make_corpus "$crash_corpus"
add_source "$crash_corpus" 01-convert-fail
run_prepass "$crash_corpus" "$crash_audit" || true
crash_logical="$crash_corpus/traces/save/01-convert-fail-session-0001.jsonl.zst"
crash_status=$(find "$crash_audit/status" -type f -name '*.status' -print -quit)
crash_key=${crash_status##*/}; crash_key=${crash_key%.status}
crash_log_name="$crash_key.attempt-0002.log"
printf 'native after crash\n' >"$crash_logical.parity.bitcode.zst"
mv -- "$crash_logical" "$crash_logical.parity-conversion-source"
printf 'cached 1500 frames; interrupted before terminal output\n' \
    >"$crash_audit/logs/$crash_log_name.in-progress"
printf 'running\t2\t%s\n' "$crash_log_name" >"$crash_status"
before=$(wc -l <"$invocations")
run_prepass "$crash_corpus" "$crash_audit"
[[ "$(wc -l <"$invocations")" == "$((before + 1))" ]]
[[ ! -e "$crash_logical" ]]
[[ ! -e "$crash_logical.parity-conversion-source" ]]
[[ -f "$crash_logical.parity.bitcode.zst" ]]
grep -Fq $'0\t3\t' "$crash_status"
find "$crash_audit/logs" -type f -name '*.attempt-0003.log' -print -quit | grep -q .

# A destructive nonzero result remains visible in the status-driven ledger
# even though only its native side survives.
delete_corpus="$test_root/corpus-delete-fail"
delete_audit="$test_root/audit-delete-fail"
make_corpus "$delete_corpus"
add_source "$delete_corpus" 01-delete-fail
run_prepass "$delete_corpus" "$delete_audit" || true
grep -Fq $'24\t1\t0\t1\t' "$delete_audit/failures.tsv"
[[ ! -e "$delete_audit/COMPLETE" ]]

# An admitted capture, a malformed marker/trace ownership set, and a runner
# hash mismatch all fail before the converter is launched.
reserved_corpus="$test_root/corpus-reserved"
make_corpus "$reserved_corpus"
add_source "$reserved_corpus" 01-reserved
: >"$reserved_corpus/.capture-reservations/live.reserve"
before=$(wc -l <"$invocations")
if run_prepass "$reserved_corpus" "$test_root/audit-reserved"; then
    printf 'test failure: accepted active capture reservation\n' >&2
    exit 1
fi
[[ "$(wc -l <"$invocations")" == "$before" ]]

orphan_corpus="$test_root/corpus-orphan"
make_corpus "$orphan_corpus"
printf 'orphan\n' >"$orphan_corpus/traces/save/orphan-session-0001.jsonl.zst"
if run_prepass "$orphan_corpus" "$test_root/audit-orphan"; then
    printf 'test failure: accepted orphan logical trace\n' >&2
    exit 1
fi

duplicate_corpus="$test_root/corpus-duplicate"
make_corpus "$duplicate_corpus"
: >"$duplicate_corpus/traces/save/replay-001.complete"
printf 'one\n' >"$duplicate_corpus/traces/save/replay-001-session-0001.jsonl.zst"
printf 'two\n' >"$duplicate_corpus/traces/save/replay-001-session-0002.jsonl.zst"
if run_prepass "$duplicate_corpus" "$test_root/audit-duplicate"; then
    printf 'test failure: accepted two logical traces for one marker\n' >&2
    exit 1
fi

# A packaged runner executes its checksum-covered hermetic wrapper.
bundle="$test_root/bundle"
mkdir -p "$bundle/lib"
cp -- "$runner" "$bundle/original_parity_replay"
cat >"$bundle/original_parity_replay.remote" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
bundle_dir=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
exec "$bundle_dir/lib/ld-linux-x86-64.so.2" --library-path "$bundle_dir/lib" \
    "$bundle_dir/original_parity_replay" "$@"
EOF
cat >"$bundle/lib/ld-linux-x86-64.so.2" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
[[ "$1" == --library-path ]]
[[ ! -v LD_LIBRARY_PATH ]]
[[ "$2" == "$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)" ]]
[[ -z ${FAKE_LOADER_INVOCATIONS:-} ]] \
    || printf 'authenticated-wrapper\n' >>"$FAKE_LOADER_INVOCATIONS"
shift 2
exec "$@"
EOF
chmod +x "$bundle/original_parity_replay" "$bundle/original_parity_replay.remote" \
    "$bundle/lib/ld-linux-x86-64.so.2"
printf 'fake provenance\nNATIVE_CONVERSION_PROTOCOL=2\n' >"$bundle/PROVENANCE.txt"
printf '/lib64/ld-linux-x86-64.so.2 => %s/lib/ld-linux-x86-64.so.2 (0x1)\n' \
    "$bundle" >"$bundle/LOADER_LIST.txt"
(cd "$bundle" && sha256sum lib/ld-linux-x86-64.so.2 >LIB_SHA256SUMS \
    && sha256sum original_parity_replay original_parity_replay.remote \
        LIB_SHA256SUMS PROVENANCE.txt LOADER_LIST.txt >SHA256SUMS)
bundle_manifest_sha=$(sha256sum "$bundle/SHA256SUMS"); bundle_manifest_sha=${bundle_manifest_sha%% *}
bundle_lib_manifest_sha=$(sha256sum "$bundle/LIB_SHA256SUMS"); bundle_lib_manifest_sha=${bundle_lib_manifest_sha%% *}
bundle_sha=$(printf 'schema16-runner-bundle-v1\nSHA256SUMS=%s\nLIB_SHA256SUMS=%s\n' \
    "$bundle_manifest_sha" "$bundle_lib_manifest_sha" | sha256sum)
bundle_sha=${bundle_sha%% *}
bundle_corpus="$test_root/corpus-bundle"
make_corpus "$bundle_corpus"
add_source "$bundle_corpus" 01-bundle
loader_invocations="$test_root/loader-invocations"
: >"$loader_invocations"
before=$(wc -l <"$invocations")
if TEST_RUNNER="$bundle" TEST_RUNNER_SHA="$bundle_sha" \
    run_prepass "$bundle_corpus" "$test_root/audit-bundle-meminfo-fixture" \
        NATIVE_CONVERT_TEST_MEMINFO="$test_meminfo"
then
    printf 'test failure: packaged runner accepted test meminfo injection\n' >&2
    exit 1
fi
[[ "$(wc -l <"$invocations")" == "$before" ]]
TEST_RUNNER="$bundle" TEST_RUNNER_SHA="$bundle_sha" \
    FAKE_LOADER_INVOCATIONS="$loader_invocations" LD_LIBRARY_PATH=/poison \
    run_prepass "$bundle_corpus" "$test_root/audit-bundle"
grep -Fxq authenticated-wrapper "$loader_invocations"

bundle_trust_sha() {
    local candidate=$1 manifest_sha lib_manifest_sha result
    manifest_sha=$(sha256sum "$candidate/SHA256SUMS"); manifest_sha=${manifest_sha%% *}
    lib_manifest_sha=$(sha256sum "$candidate/LIB_SHA256SUMS"); lib_manifest_sha=${lib_manifest_sha%% *}
    result=$(printf 'schema16-runner-bundle-v1\nSHA256SUMS=%s\nLIB_SHA256SUMS=%s\n' \
        "$manifest_sha" "$lib_manifest_sha" | sha256sum)
    printf '%s\n' "${result%% *}"
}

expect_bundle_rejection() {
    local candidate=$1 label=$2 trust corpus_path before
    trust=$(bundle_trust_sha "$candidate")
    corpus_path="$test_root/corpus-bundle-$label"
    make_corpus "$corpus_path"
    add_source "$corpus_path" "01-$label"
    before=$(wc -l <"$invocations")
    if TEST_RUNNER="$candidate" TEST_RUNNER_SHA="$trust" \
        run_prepass "$corpus_path" "$test_root/audit-bundle-$label"
    then
        printf 'test failure: accepted malformed bundle: %s\n' "$label" >&2
        exit 1
    fi
    [[ "$(wc -l <"$invocations")" == "$before" ]]
}

relocate_bundle_proof() {
    local candidate=$1
    printf '/lib64/ld-linux-x86-64.so.2 => %s/lib/ld-linux-x86-64.so.2 (0x1)\n' \
        "$candidate" >"$candidate/LOADER_LIST.txt"
    (cd "$candidate" && sha256sum original_parity_replay \
        original_parity_replay.remote LIB_SHA256SUMS PROVENANCE.txt LOADER_LIST.txt \
        >SHA256SUMS)
}

symlink_bundle="$test_root/bundle-symlink"
cp -a -- "$bundle" "$symlink_bundle"
relocate_bundle_proof "$symlink_bundle"
ln -s /etc/passwd "$symlink_bundle/lib/escape"
expect_bundle_rejection "$symlink_bundle" symlink

root_extra_bundle="$test_root/bundle-root-extra"
cp -a -- "$bundle" "$root_extra_bundle"
relocate_bundle_proof "$root_extra_bundle"
: >"$root_extra_bundle/unexpected"
expect_bundle_rejection "$root_extra_bundle" root-extra

lib_extra_bundle="$test_root/bundle-lib-extra"
cp -a -- "$bundle" "$lib_extra_bundle"
relocate_bundle_proof "$lib_extra_bundle"
: >"$lib_extra_bundle/lib/unexpected.so"
expect_bundle_rejection "$lib_extra_bundle" lib-extra

loader_escape_bundle="$test_root/bundle-loader-escape"
cp -a -- "$bundle" "$loader_escape_bundle"
printf '/lib64/ld-linux-x86-64.so.2 => /outside/ld-linux.so.2 (0x1)\n' \
    >"$loader_escape_bundle/LOADER_LIST.txt"
(cd "$loader_escape_bundle" && sha256sum original_parity_replay \
    original_parity_replay.remote LIB_SHA256SUMS PROVENANCE.txt LOADER_LIST.txt \
    >SHA256SUMS)
expect_bundle_rejection "$loader_escape_bundle" loader-escape

old_protocol_bundle="$test_root/bundle-old-protocol"
cp -a -- "$bundle" "$old_protocol_bundle"
printf 'fake provenance\nNATIVE_CONVERSION_PROTOCOL=1\n' \
    >"$old_protocol_bundle/PROVENANCE.txt"
relocate_bundle_proof "$old_protocol_bundle"
expect_bundle_rejection "$old_protocol_bundle" old-protocol

multiple_protocol_bundle="$test_root/bundle-multiple-protocol"
cp -a -- "$bundle" "$multiple_protocol_bundle"
printf 'fake provenance\nNATIVE_CONVERSION_PROTOCOL=2\nNATIVE_CONVERSION_PROTOCOL=2\n' \
    >"$multiple_protocol_bundle/PROVENANCE.txt"
relocate_bundle_proof "$multiple_protocol_bundle"
expect_bundle_rejection "$multiple_protocol_bundle" multiple-protocol

malformed_protocol_bundle="$test_root/bundle-malformed-protocol"
cp -a -- "$bundle" "$malformed_protocol_bundle"
printf 'fake provenance\nNATIVE_CONVERSION_PROTOCOL=two\n' \
    >"$malformed_protocol_bundle/PROVENANCE.txt"
relocate_bundle_proof "$malformed_protocol_bundle"
expect_bundle_rejection "$malformed_protocol_bundle" malformed-protocol

# Mutation after initial authentication is caught before completion is
# published, even though conversion itself returned success.
mutable_bundle="$test_root/bundle-during-run"
cp -a -- "$bundle" "$mutable_bundle"
relocate_bundle_proof "$mutable_bundle"
mutable_trust=$(bundle_trust_sha "$mutable_bundle")
mutable_corpus="$test_root/corpus-bundle-during-run"
make_corpus "$mutable_corpus"
add_source "$mutable_corpus" 01-mutate-bundle
if TEST_RUNNER="$mutable_bundle" TEST_RUNNER_SHA="$mutable_trust" \
    FAKE_MUTATE_BUNDLE_WRAPPER="$mutable_bundle/original_parity_replay.remote" \
    run_prepass "$mutable_corpus" "$test_root/audit-bundle-during-run"
then
    printf 'test failure: accepted bundle mutation during conversion\n' >&2
    exit 1
fi
[[ ! -e "$test_root/audit-bundle-during-run/COMPLETE" ]]

if NATIVE_CONVERT_TEST_ALLOW_DIRECT_RUNNER=1 "$repository/scripts/run_native_conversion_prepass.sh" \
    "$workspace" "$corpus" "$runner" "${runner_sha%?}0" "$test_root/audit-sha"
then
    printf 'test failure: accepted incorrect runner hash\n' >&2
    exit 1
fi

printf 'native conversion prepass tests passed\n'
