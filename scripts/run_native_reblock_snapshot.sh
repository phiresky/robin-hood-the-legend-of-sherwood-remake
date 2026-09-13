#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=scripts/lib/parity_common.sh
source "$(dirname -- "$(realpath -- "${BASH_SOURCE[0]}")")/lib/parity_common.sh"

# Atomically rewrite one immutable snapshot of native parity traces with the
# block geometry compiled into an authenticated runner.  The runner itself
# owns the hard-link/binding recovery protocol and proves semantic equality
# before publishing each replacement.  This driver adds frozen membership,
# durable per-artifact attempts, bounded concurrency, and completion evidence.

if (( $# != 6 )); then
    printf 'usage: %s WORKSPACE CORPUS RUNNER_BUNDLE TRUST_SHA256 AUDIT_DIR EXPECTED_COUNT\n' "$0" >&2
    exit 2
fi

workspace=$(realpath -e -- "$1")
corpus=$(realpath -e -- "$2")
bundle=$(realpath -e -- "$3")
expected_trust=${4,,}
audit=$(realpath -m -- "$5")
expected_count=$6
jobs=${NATIVE_REBLOCK_JOBS:-8}
timeout_seconds=${NATIVE_REBLOCK_TIMEOUT_SECONDS:-7200}
outer_lock=${NATIVE_REBLOCK_OUTER_LOCK:-/srv/robinhood/locks/robin-parity-runner.lock}


[[ "$corpus" == "$workspace"/* ]] || fail 'corpus is outside workspace'
if [[ -d "$corpus/traces" ]]; then
    trace_root="$corpus/traces"
else
    trace_root="$corpus"
fi
find "$trace_root" -type f -name '*.parity.bitcode.zst' -print -quit | grep -q . \
    || fail 'corpus has no native traces'
[[ "$audit" == "$workspace"/* && "$audit" != "$corpus" && "$audit" != "$corpus"/* ]] || fail 'unsafe audit path'
[[ "$expected_trust" =~ ^[0-9a-f]{64}$ ]] || fail 'invalid trust digest'
[[ "$expected_count" =~ ^[1-9][0-9]*$ ]] || fail 'invalid expected count'
[[ "$jobs" =~ ^[1-8]$ ]] || fail 'NATIVE_REBLOCK_JOBS must be 1..8'
[[ "$timeout_seconds" =~ ^[0-9]+$ && "$timeout_seconds" -ge 3600 ]] || fail 'invalid timeout'
runner="$bundle/original_parity_replay.remote"
# This driver runs wherever the authoritative bundle is deployed and is not told
# the path its LOADER_LIST.txt was generated at, so it uses the relocation proof.
verify_runner_bundle "$bundle" --relocated || exit 2
verify_runner_bundle_identity "$bundle" "$expected_trust" || exit 2
actual_trust=$expected_trust
raw_sha=$(sha256_file "$bundle/original_parity_replay") || fail 'cannot hash raw runner'

mkdir -p -- "${outer_lock%/*}" "${audit%/*}"
exec {outer_fd}>"$outer_lock"
flock -n "$outer_fd" || fail "outer parity lock is held: $outer_lock"
corpus_key=$(printf '%s' "${corpus#"$workspace"/}" | sha256sum); corpus_key=${corpus_key%% *}
lock_dir="$workspace/.git/native-conversion-locks"
mkdir -p -- "$lock_dir"
exec {corpus_fd}>"$lock_dir/$corpus_key.lock"
flock -n "$corpus_fd" || fail 'another conversion owns the corpus'
exec {admission_fd}>"$corpus/.capture-admission.lock"
flock -n "$admission_fd" || fail 'capture admission is active'
exec {collector_fd}>"$corpus/.distributed-collector.lock"
flock -n "$collector_fd" || fail 'distributed collection is active'

if [[ ! -e "$audit" ]]; then
    staging=$(mktemp -d "${audit}.tmp.XXXXXX")
    mkdir -p "$staging/logs" "$staging/status"
    find "$trace_root" -type f -name '*.parity.bitcode.zst' -print0 \
        | LC_ALL=C sort -z >"$staging/native-paths.nul"
    count=$(tr -cd '\0' <"$staging/native-paths.nul" | wc -c)
    [[ "$count" == "$expected_count" ]] || fail "native snapshot count $count != $expected_count"
    while IFS= read -r -d '' path; do sha256sum --zero -- "$path"; done \
        <"$staging/native-paths.nul" >"$staging/native-before.sha256z"
    {
        printf 'CREATED_UTC=%q\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
        printf 'WORKSPACE=%q\nCORPUS=%q\nBUNDLE=%q\n' "$workspace" "$corpus" "$bundle"
        printf 'RUNNER_RAW_SHA256=%s\nRUNNER_TRUST_SHA256=%s\n' "$raw_sha" "$actual_trust"
        printf 'JOBS=%s\nEXPECTED_COUNT=%s\n' "$jobs" "$expected_count"
        printf 'NATIVE_PATHS_SHA256=%s\n' "$(sha256sum "$staging/native-paths.nul" | cut -d' ' -f1)"
        printf 'NATIVE_BEFORE_MANIFEST_SHA256=%s\n' "$(sha256sum "$staging/native-before.sha256z" | cut -d' ' -f1)"
    } >"$staging/provenance.env"
    mv -- "$staging" "$audit"
else
    [[ -d "$audit" && -f "$audit/native-paths.nul" && -f "$audit/native-before.sha256z" \
        && -f "$audit/provenance.env" && -d "$audit/logs" && -d "$audit/status" ]] \
        || fail 'existing audit is incomplete'
    if [[ -f "$audit/MANIFEST.sha256" ]]; then
        [[ -f "$audit/COMPLETE" ]] || fail 'sealed audit has no completion record'
        (cd "$audit" && sha256sum --strict -c MANIFEST.sha256) >/dev/null \
            || fail 'existing reblock audit seal is invalid'
        printf 'native reblock snapshot already complete: %s\n' "$audit"
        exit 0
    fi
    count=$(tr -cd '\0' <"$audit/native-paths.nul" | wc -c)
    [[ "$count" == "$expected_count" ]] || fail 'frozen native snapshot count changed'
    grep -Fxq "RUNNER_RAW_SHA256=$raw_sha" "$audit/provenance.env" || fail 'runner raw hash drift'
    grep -Fxq "RUNNER_TRUST_SHA256=$actual_trust" "$audit/provenance.env" || fail 'runner trust drift'
fi

run_one() {
    local native=$1 key status first_attempt=1 rc=0
    local attempt_prior_status attempt_prior_number attempt_prior_log
    local attempt_number attempt_log_name attempt_log_final attempt_log_in_progress
    key=$(attempt_key "$workspace" "$native") || return 1
    status="$audit/status/$key.status"
    if [[ -f "$status" ]]; then
        attempt_read_status "$status" || return 1
        if [[ "$attempt_prior_status" == 0 ]]; then
            [[ -f "$native" && ! -e "$native.parity-reblock-source-v66" \
                && ! -e "$native.parity-reblock-binding-v66.json" \
                && ! -e "$native.parity-reblock-source-v67" \
                && ! -e "$native.parity-reblock-binding-v67.json" ]] || return 1
            return 0
        fi
        first_attempt=$((attempt_prior_number + 1))
    fi
    attempt_begin "$audit/logs" "$key" "$first_attempt" "$status" || return 1
    timeout --signal=TERM --kill-after=30s "${timeout_seconds}s" \
        nice -n 10 ionice -c 2 -n 7 env -u LD_LIBRARY_PATH \
        "$runner" --reblock "$native" >"$attempt_log_in_progress" 2>&1 || rc=$?
    if (( rc == 0 )) && { [[ ! -f "$native" || -e "$native.parity-reblock-source-v66" \
        || -e "$native.parity-reblock-binding-v66.json" \
        || -e "$native.parity-reblock-source-v67" \
        || -e "$native.parity-reblock-binding-v67.json" ]]; }; then
        rc=65
        printf 'postcondition failed: canonical or recovery state invalid\n' \
            >>"$attempt_log_in_progress"
    fi
    attempt_finish "$status" "$rc" || return 1
    (( rc == 0 ))
}
export -f run_one sha256_file write_atomic attempt_key attempt_read_status \
    attempt_begin attempt_finish
export workspace audit runner timeout_seconds

active=0
failed=0
while IFS= read -r -d '' native; do
    run_one "$native" &
    active=$((active + 1))
    if (( active >= jobs )); then wait -n || failed=1; active=$((active - 1)); fi
done <"$audit/native-paths.nul"
while (( active > 0 )); do wait -n || failed=1; active=$((active - 1)); done
(( failed == 0 )) || fail 'one or more reblock workers failed'

while IFS= read -r -d '' native; do
    relative=${native#"$workspace"/}; key=$(printf '%s' "$relative" | sha256sum); key=${key%% *}
    IFS=$'\t' read -r rc _ _ <"$audit/status/$key.status" || fail "missing status: $native"
    [[ "$rc" == 0 && -f "$native" && ! -e "$native.parity-reblock-source-v66" \
        && ! -e "$native.parity-reblock-binding-v66.json" \
        && ! -e "$native.parity-reblock-source-v67" \
        && ! -e "$native.parity-reblock-binding-v67.json" ]] || fail "incomplete reblock: $native"
done <"$audit/native-paths.nul"
find "$trace_root" -type f \
    \( -name '.parity-reblock-*' -o -name '*.parity-reblock-source-v*' \
        -o -name '*.parity-reblock-binding-v*.json' \) -print -quit | grep -q . \
    && fail 'reblock temporary artifacts remain'
while IFS= read -r -d '' path; do sha256sum --zero -- "$path"; done \
    <"$audit/native-paths.nul" >"$audit/native-after.sha256z"
{
    printf 'COMPLETED_UTC=%q\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    printf 'COUNT=%s\n' "$expected_count"
    printf 'BEFORE_MANIFEST_SHA256=%s\n' "$(sha256sum "$audit/native-before.sha256z" | cut -d' ' -f1)"
    printf 'AFTER_MANIFEST_SHA256=%s\n' "$(sha256sum "$audit/native-after.sha256z" | cut -d' ' -f1)"
} | write_atomic "$audit/COMPLETE"
(
    cd "$audit"
    sha256sum -- provenance.env native-paths.nul native-before.sha256z \
        native-after.sha256z COMPLETE
    find logs status -type f -print0 | LC_ALL=C sort -z \
        | xargs -0 -r sha256sum --
) | write_atomic "$audit/MANIFEST.sha256"
printf 'native reblock snapshot complete: %s\n' "$audit"
