#!/usr/bin/env bash
set -euo pipefail

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

fail() { printf 'error: %s\n' "$*" >&2; exit 2; }
write_atomic() {
    local destination=$1 temporary
    temporary=$(mktemp "${destination}.tmp.XXXXXX") || return 1
    if ! cat >"$temporary" || ! mv -f -- "$temporary" "$destination"; then
        rm -f -- "$temporary"
        return 1
    fi
}

[[ "$corpus" == "$workspace"/* && -d "$corpus/traces" ]] || fail 'corpus is outside workspace or lacks traces'
[[ "$audit" == "$workspace"/* && "$audit" != "$corpus" && "$audit" != "$corpus"/* ]] || fail 'unsafe audit path'
[[ "$expected_trust" =~ ^[0-9a-f]{64}$ ]] || fail 'invalid trust digest'
[[ "$expected_count" =~ ^[1-9][0-9]*$ ]] || fail 'invalid expected count'
[[ "$jobs" =~ ^[1-8]$ ]] || fail 'NATIVE_REBLOCK_JOBS must be 1..8'
[[ "$timeout_seconds" =~ ^[0-9]+$ && "$timeout_seconds" -ge 3600 ]] || fail 'invalid timeout'
runner="$bundle/original_parity_replay.remote"
[[ -x "$runner" && -x "$bundle/original_parity_replay" && -f "$bundle/SHA256SUMS" \
    && -f "$bundle/LIB_SHA256SUMS" ]] || fail 'incomplete runner bundle'
find "$bundle" -type l -print -quit | grep -q . && fail 'runner bundle contains a symlink'
(cd "$bundle" && sha256sum --strict -c SHA256SUMS && sha256sum --strict -c LIB_SHA256SUMS) \
    >/dev/null || fail 'runner bundle checksum verification failed'
manifest_sha=$(sha256sum -- "$bundle/SHA256SUMS"); manifest_sha=${manifest_sha%% *}
lib_manifest_sha=$(sha256sum -- "$bundle/LIB_SHA256SUMS"); lib_manifest_sha=${lib_manifest_sha%% *}
actual_trust=$(printf 'schema16-runner-bundle-v1\nSHA256SUMS=%s\nLIB_SHA256SUMS=%s\n' \
    "$manifest_sha" "$lib_manifest_sha" | sha256sum); actual_trust=${actual_trust%% *}
[[ "$actual_trust" == "$expected_trust" ]] || fail "runner trust mismatch: $actual_trust"
raw_sha=$(sha256sum -- "$bundle/original_parity_replay"); raw_sha=${raw_sha%% *}

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
    find "$corpus/traces" -type f -name '*.parity.bitcode.zst' -print0 \
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
    count=$(tr -cd '\0' <"$audit/native-paths.nul" | wc -c)
    [[ "$count" == "$expected_count" ]] || fail 'frozen native snapshot count changed'
    grep -Fxq "RUNNER_RAW_SHA256=$raw_sha" "$audit/provenance.env" || fail 'runner raw hash drift'
    grep -Fxq "RUNNER_TRUST_SHA256=$actual_trust" "$audit/provenance.env" || fail 'runner trust drift'
fi

run_one() {
    local native=$1 relative key status prior attempt=1 log_name log_tmp log_final rc=0
    relative=${native#"$workspace"/}
    key=$(printf '%s' "$relative" | sha256sum); key=${key%% *}
    status="$audit/status/$key.status"
    if [[ -f "$status" ]]; then
        IFS=$'\t' read -r prior attempt log_name <"$status" || return 1
        if [[ "$prior" == 0 ]]; then
            [[ -f "$native" && ! -e "$native.parity-reblock-source-v66" \
                && ! -e "$native.parity-reblock-binding-v66.json" ]] || return 1
            return 0
        fi
        attempt=$((attempt + 1))
    fi
    printf -v label '%04d' "$attempt"
    log_name="$key.attempt-$label.log"
    log_final="$audit/logs/$log_name"
    log_tmp="$log_final.in-progress"
    [[ ! -e "$log_final" && ! -e "$log_tmp" ]] || return 1
    : >"$log_tmp"
    printf 'running\t%s\t%s\n' "$attempt" "$log_name" | write_atomic "$status" || return 1
    timeout --signal=TERM --kill-after=30s "${timeout_seconds}s" \
        nice -n 10 ionice -c 2 -n 7 env -u LD_LIBRARY_PATH \
        "$runner" --reblock "$native" >"$log_tmp" 2>&1 || rc=$?
    if (( rc == 0 )) && { [[ ! -f "$native" || -e "$native.parity-reblock-source-v66" \
        || -e "$native.parity-reblock-binding-v66.json" ]]; }; then
        rc=65
        printf 'postcondition failed: canonical or recovery state invalid\n' >>"$log_tmp"
    fi
    mv -- "$log_tmp" "$log_final"
    printf '%s\t%s\t%s\n' "$rc" "$attempt" "$log_name" | write_atomic "$status"
    (( rc == 0 ))
}
export -f run_one write_atomic
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
        && ! -e "$native.parity-reblock-binding-v66.json" ]] || fail "incomplete reblock: $native"
done <"$audit/native-paths.nul"
find "$corpus/traces" -type f -name '.parity-reblock-*' -print -quit | grep -q . \
    && fail 'reblock temporary artifacts remain'
while IFS= read -r -d '' path; do sha256sum --zero -- "$path"; done \
    <"$audit/native-paths.nul" >"$audit/native-after.sha256z"
{
    printf 'COMPLETED_UTC=%q\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    printf 'COUNT=%s\n' "$expected_count"
    printf 'BEFORE_MANIFEST_SHA256=%s\n' "$(sha256sum "$audit/native-before.sha256z" | cut -d' ' -f1)"
    printf 'AFTER_MANIFEST_SHA256=%s\n' "$(sha256sum "$audit/native-after.sha256z" | cut -d' ' -f1)"
} | write_atomic "$audit/COMPLETE"
printf 'native reblock snapshot complete: %s\n' "$audit"
