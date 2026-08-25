#!/usr/bin/env bash
set -euo pipefail

# Incrementally attest exact EOF for completed native recordings while capture
# is still active.  This is deliberately separate from final corpus validation:
# it never freezes a corpus-wide manifest. It takes the shared outer/native
# locks only by nonblocking admission and holds them for its single attempt, so
# a production phase transition can trigger the watchdog instead of queueing
# more work. Each immutable (logical path, native digest, runner bundle) tuple
# is published as one atomically-renamed result directory.

if (( $# < 7 )); then
    printf 'usage: %s WORKSPACE RUNNER_BUNDLE BUNDLE_TRUST_SHA RUNNER_SHA ORCHESTRATOR_AUDIT AUDIT_DIR CAMPAIGN [CAMPAIGN ...]\n' "$0" >&2
    exit 2
fi

workspace_arg=$1
bundle_arg=$2
bundle_trust_sha=${3,,}
runner_sha=${4,,}
orchestrator_audit_arg=$5
audit_arg=$6
shift 6
campaign_args=("$@")

poll_seconds=${INCREMENTAL_EOF_POLL_SECONDS:-30}
timeout_seconds=${INCREMENTAL_EOF_TIMEOUT_SECONDS:-900}
abort_grace_seconds=${INCREMENTAL_EOF_ABORT_GRACE_SECONDS:-10}
min_memory_kib=${INCREMENTAL_EOF_MIN_MEMORY_KIB:-25165824}
memory_per_job_kib=${INCREMENTAL_EOF_MEMORY_PER_JOB_KIB:-6291456}
max_load1=${INCREMENTAL_EOF_MAX_LOAD1:-40}
concurrency=${INCREMENTAL_EOF_CONCURRENCY:-1}
oneshot=${INCREMENTAL_EOF_ONESHOT:-0}
admitted_phase=${INCREMENTAL_EOF_ADMITTED_PHASE:-wait-seed3-natural-exit}
allow_drained=${INCREMENTAL_EOF_ALLOW_DRAINED:-0}
nice_level=${INCREMENTAL_EOF_NICE_LEVEL:-19}
ionice_class=${INCREMENTAL_EOF_IONICE_CLASS:-3}
ionice_level=${INCREMENTAL_EOF_IONICE_LEVEL:-7}
outer_lock=${INCREMENTAL_EOF_OUTER_LOCK:-/tmp/robin-parity-runner.lock}
slot_dir=${INCREMENTAL_EOF_SLOT_DIR:-$workspace_arg/.git/parity-runner-slots}
native_lock_dir=${INCREMENTAL_EOF_NATIVE_LOCK_DIR:-$workspace_arg/.git/native-conversion-locks}
meminfo_path=${INCREMENTAL_EOF_MEMINFO_PATH:-/proc/meminfo}
loadavg_path=${INCREMENTAL_EOF_LOADAVG_PATH:-/proc/loadavg}
exact_eof_marker='parity trace matched every recorded frame'

fail() {
    printf 'error: %s\n' "$*" >&2
    exit 2
}

sha256_file() {
    local value
    value=$(sha256sum -- "$1") || return 1
    printf '%s\n' "${value%% *}"
}

write_atomic() {
    local destination=$1 temporary
    temporary=$(mktemp "${destination}.tmp.XXXXXX") || return 1
    if ! cat >"$temporary" || ! mv -f -- "$temporary" "$destination"; then
        rm -f -- "$temporary"
        return 1
    fi
}

is_uint() {
    [[ "$1" =~ ^[0-9]+$ ]]
}

normalize_bounded_uint() {
    local value=$1 limit=$2
    is_uint "$value" || return 1
    while [[ ${#value} -gt 1 && $value == 0* ]]; do
        value=${value#0}
    done
    if (( ${#value} > ${#limit} )) \
        || { (( ${#value} == ${#limit} )) && [[ $value > $limit ]]; }
    then
        return 1
    fi
    printf '%s\n' "$value"
}

is_nonnegative_number() {
    [[ "$1" =~ ^[0-9]+([.][0-9]+)?$ ]]
}

float_le() {
    LC_ALL=C awk -v value="$1" -v limit="$2" \
        'BEGIN { exit !(value + 0 <= limit + 0) }'
}

path_has_newline() {
    [[ "$1" == *$'\n'* ]]
}

runner_bundle_digest() {
    local bundle=$1 main_sha lib_sha value
    main_sha=$(sha256_file "$bundle/SHA256SUMS") || return 1
    lib_sha=$(sha256_file "$bundle/LIB_SHA256SUMS") || return 1
    value=$(printf 'schema16-runner-bundle-v1\nSHA256SUMS=%s\nLIB_SHA256SUMS=%s\n' \
        "$main_sha" "$lib_sha" | sha256sum) || return 1
    printf '%s\n' "${value%% *}"
}

verify_bundle() {
    local manifest line path
    [[ -x "$bundle/original_parity_replay" \
        && -x "$bundle/original_parity_replay.remote" \
        && -f "$bundle/SHA256SUMS" \
        && -f "$bundle/LIB_SHA256SUMS" \
        && -f "$bundle/PROVENANCE.txt" ]] \
        || fail "incomplete runner bundle: $bundle"
    [[ "$(sha256_file "$bundle/original_parity_replay")" == "$runner_sha" ]] \
        || fail 'raw runner hash mismatch'
    [[ "$(runner_bundle_digest "$bundle")" == "$bundle_trust_sha" ]] \
        || fail 'runner bundle trust digest mismatch'
    mapfile -t protocol_values < <(sed -n 's/^NATIVE_CONVERSION_PROTOCOL=//p' \
        "$bundle/PROVENANCE.txt")
    [[ ${#protocol_values[@]} == 1 && "${protocol_values[0]}" == 2 ]] \
        || fail 'runner bundle does not authenticate native conversion protocol 2'
    if find "$bundle" -type l -print -quit | grep -q .; then
        fail "runner bundle contains a symlink: $bundle"
    fi
    for manifest in "$bundle/SHA256SUMS" "$bundle/LIB_SHA256SUMS"; do
        while IFS= read -r line; do
            [[ "$line" =~ ^[0-9a-fA-F]{64}[[:space:]][\ \*](.+)$ ]] \
                || fail "malformed bundle checksum entry: $manifest"
            path=${BASH_REMATCH[1]}
            [[ "$path" != /* && "$path" != ../* && "$path" != */../* \
                && "$path" != *'/..' && "$path" != *$'\n'* ]] \
                || fail "unsafe bundle checksum path: $path"
        done <"$manifest"
    done
    diff -u -- \
        <(find "$bundle/lib" -type f -printf 'lib/%P\n' | LC_ALL=C sort) \
        <(sed -n 's/^[0-9a-fA-F]\{64\} [ *]//p' "$bundle/LIB_SHA256SUMS" \
            | LC_ALL=C sort) >/dev/null \
        || fail 'library manifest does not exactly cover bundle lib tree'
    diff -u -- \
        <(printf '%s\n' LIB_SHA256SUMS LOADER_LIST.txt PROVENANCE.txt \
            original_parity_replay original_parity_replay.remote | LC_ALL=C sort) \
        <(sed -n 's/^[0-9a-fA-F]\{64\} [ *]//p' "$bundle/SHA256SUMS" \
            | LC_ALL=C sort) >/dev/null \
        || fail 'main manifest does not exactly cover bundle root files'
    diff -u -- \
        <(printf '%s\n' LIB_SHA256SUMS LOADER_LIST.txt PROVENANCE.txt SHA256SUMS \
            original_parity_replay original_parity_replay.remote | LC_ALL=C sort) \
        <(find "$bundle" -maxdepth 1 -type f -printf '%f\n' | LC_ALL=C sort) \
        >/dev/null || fail 'runner bundle root file set is not canonical'
    diff -u -- <(printf 'lib\n') \
        <(find "$bundle" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' \
            | LC_ALL=C sort) >/dev/null \
        || fail 'runner bundle has an unexpected root directory'
    grep -Fq -- "=> $bundle/lib/ld-linux-x86-64.so.2 " "$bundle/LOADER_LIST.txt" \
        || fail 'runner loader proof is not bound to this bundle path'
    awk -v prefix="$bundle/lib/" '
        /=>/ {
            resolved=$0
            sub(/^.*=>[[:space:]]*/, "", resolved)
            sub(/[[:space:]].*$/, "", resolved)
            if (index(resolved, prefix) != 1) exit 1
        }
    ' "$bundle/LOADER_LIST.txt" \
        || fail 'runner loader proof resolves outside authenticated lib tree'
    grep -Eq '^[0-9a-fA-F]{64} [ *]original_parity_replay$' "$bundle/SHA256SUMS" \
        && grep -Eq '^[0-9a-fA-F]{64} [ *]original_parity_replay\.remote$' "$bundle/SHA256SUMS" \
        && grep -Eq '^[0-9a-fA-F]{64} [ *]LIB_SHA256SUMS$' "$bundle/SHA256SUMS" \
        && grep -Eq '^[0-9a-fA-F]{64} [ *]PROVENANCE\.txt$' "$bundle/SHA256SUMS" \
        && grep -Eq '^[0-9a-fA-F]{64} [ *]LOADER_LIST\.txt$' "$bundle/SHA256SUMS" \
        && grep -Eq '^[0-9a-fA-F]{64} [ *]lib/ld-linux-x86-64\.so\.2$' "$bundle/LIB_SHA256SUMS" \
        || fail 'bundle manifests omit required runtime inputs'
    (cd -- "$bundle" && sha256sum --strict -c SHA256SUMS \
        && sha256sum --strict -c LIB_SHA256SUMS) >/dev/null \
        || fail 'runner bundle checksum verification failed'
}

read_phase() {
    local state="$orchestrator_audit/state.env" value
    [[ -f "$state" ]] || return 1
    mapfile -t phase_values < <(sed -n 's/^PHASE=//p' "$state")
    (( ${#phase_values[@]} == 1 )) || return 1
    value=${phase_values[0]}
    # write_phase currently emits shell-escaped values; the admitted phase has
    # no metacharacters, so require its canonical literal spelling.
    [[ "$value" == "$admitted_phase" ]]
}

admission_open() {
    local campaign
    read_phase || return 1
    for campaign in "${campaigns[@]}"; do
        if (( allow_drained == 0 )); then
            [[ ! -e "$campaign/.capture.drain" ]] || return 1
        fi
    done
}

resource_gate_open() {
    local admitted_jobs=${1:-1} memory load required_memory
    memory=$(awk '$1 == "MemAvailable:" && $2 ~ /^[0-9]+$/ {print $2; exit}' \
        "$meminfo_path") || return 1
    load=$(awk '{print $1; exit}' "$loadavg_path") || return 1
    required_memory=$((min_memory_kib + (admitted_jobs - 1) * memory_per_job_kib))
    is_uint "$memory" && is_nonnegative_number "$load" \
        && (( memory >= required_memory )) && float_le "$load" "$max_load1"
}

append_gate_sample() {
    local result=$1 admitted_jobs=${2:-1} memory load required_memory
    memory=$(awk '$1 == "MemAvailable:" {print $2; exit}' "$meminfo_path" 2>/dev/null || true)
    load=$(awk '{print $1; exit}' "$loadavg_path" 2>/dev/null || true)
    required_memory=$((min_memory_kib + (admitted_jobs - 1) * memory_per_job_kib))
    exec {gate_log_fd}>"$audit/resource-gate.lock" || return 1
    flock "$gate_log_fd" || { exec {gate_log_fd}>&-; return 1; }
    printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
        "${memory:-NA}" "${load:-NA}" "$admitted_jobs" "$required_memory" \
        "$result" >>"$audit/resource-gate.tsv"
    exec {gate_log_fd}>&-
}

list_candidates() {
    local campaign marker marker_dir marker_name stem native logical
    local -a markers=() natives=()
    for campaign in "${campaigns[@]}"; do
        mapfile -d '' -t markers < <(
            find "$campaign/traces" -type f -name '*.complete' -print0 | LC_ALL=C sort -z
        )
        for marker in "${markers[@]}"; do
            marker_dir=${marker%/*}
            marker_name=${marker##*/}
            stem=${marker_name%.complete}
            natives=()
            if [[ -f "$marker_dir/$stem.jsonl.zst.parity.bitcode.zst" ]]; then
                natives+=("$marker_dir/$stem.jsonl.zst.parity.bitcode.zst")
            fi
            while IFS= read -r -d '' native; do
                natives+=("$native")
            done < <(
                find "$marker_dir" -maxdepth 1 -type f \
                    -name "$stem-session-*.jsonl.zst.parity.bitcode.zst" \
                    -print0 | LC_ALL=C sort -z
            )
            for native in "${natives[@]}"; do
                logical=${native%.parity.bitcode.zst}
                printf '%s\0%s\0%s\0' "$campaign" "$marker" "$logical"
            done
        done
    done
}

verify_result() {
    local result=$1 native_sha=$2 expected_trace=$3 expected_marker=$4
    local marker_count status_value attested_native log_sha actual_log_sha trace_value
    local expected_identity expected_logical_sha expected_marker_relative expected_marker_sha
    [[ -d "$result" && -f "$result/status" && -f "$result/log" \
        && -f "$result/attestation.env" && -f "$result/trace.path" \
        && -f "$result/MANIFEST.sha256" ]] || return 1
    diff -u -- \
        <(printf '%s\n' MANIFEST.sha256 attestation.env log status trace.path \
            | LC_ALL=C sort) \
        <(find "$result" -mindepth 1 -maxdepth 1 -type f -printf '%f\n' \
            | LC_ALL=C sort) >/dev/null || return 1
    [[ -z "$(find "$result" -mindepth 1 -maxdepth 1 ! -type f -print -quit)" ]] \
        || return 1
    mapfile -t status_values <"$result/status" || return 1
    (( ${#status_values[@]} == 1 )) || return 1
    status_value=${status_values[0]}
    [[ "$status_value" == 0 ]] || return 1
    marker_count=$(grep -Fxc -- "$exact_eof_marker" "$result/log" || true)
    [[ "$marker_count" == 1 ]] || return 1
    mapfile -t attested_values < <(sed -n 's/^NATIVE_SHA256_POST=//p' \
        "$result/attestation.env")
    (( ${#attested_values[@]} == 1 )) || return 1
    attested_native=${attested_values[0]}
    [[ "$attested_native" == "$native_sha" ]] || return 1
    mapfile -t trace_values <"$result/trace.path" || return 1
    (( ${#trace_values[@]} == 1 )) || return 1
    trace_value=${trace_values[0]}
    [[ "$trace_value" == "$expected_trace" ]] || return 1
    mapfile -t log_values < <(sed -n 's/^LOG_SHA256=//p' "$result/attestation.env")
    (( ${#log_values[@]} == 1 )) || return 1
    log_sha=${log_values[0]}
    actual_log_sha=$(sha256_file "$result/log") || return 1
    [[ "$log_sha" == "$actual_log_sha" ]] || return 1
    expected_logical_sha=$(printf '%s' "$expected_trace" | sha256sum); expected_logical_sha=${expected_logical_sha%% *}
    expected_identity=$(printf 'schema16-incremental-eof-v1\nLOGICAL=%s\nNATIVE_SHA256=%s\nRUNNER_BUNDLE_TRUST_SHA256=%s\n' \
        "$expected_trace" "$native_sha" "$bundle_trust_sha" | sha256sum)
    expected_identity=${expected_identity%% *}
    [[ "${result##*/}" == "$expected_identity" ]] || return 1
    expected_marker_relative=${expected_marker#"$workspace"/}
    expected_marker_sha=$(sha256_file "$expected_marker") || return 1
    local -a exact_lines=(
        'FORMAT=schema16-incremental-eof-v1'
        "LOGICAL_PATH_SHA256=$expected_logical_sha"
        "IDENTITY_SHA256=$expected_identity"
        "COMPLETION_MARKER=$expected_marker_relative"
        "COMPLETION_MARKER_SHA256=$expected_marker_sha"
        "NATIVE_SHA256_PRE=$native_sha"
        "NATIVE_SHA256_POST=$native_sha"
        "RUNNER_RAW_SHA256=$runner_sha"
        "RUNNER_BUNDLE_TRUST_SHA256=$bundle_trust_sha"
        "RUNNER_BUNDLE_MANIFEST_SHA256=$bundle_manifest_sha"
        "RUNNER_LIB_MANIFEST_SHA256=$bundle_lib_manifest_sha"
        "RUNNER_WRAPPER_SHA256=$runner_wrapper_sha"
        "DATA_DIR=$data_dir"
        "DATA_DIR_PATH_SHA256=$data_dir_path_sha"
        'COMMAND=original_parity_replay.remote --no-auto-dump LOGICAL_TRACE'
        "TIMEOUT_SECONDS=$timeout_seconds"
        "NICE_LEVEL=$nice_level"
        "IONICE_CLASS=$ionice_class"
        "IONICE_LEVEL=$ionice_level"
        'RUNNER_COMMAND_STATUS=0'
        'EXACT_EOF_MARKER_COUNT=1'
        "LOG_SHA256=$actual_log_sha"
    )
    local expected_line
    for expected_line in "${exact_lines[@]}"; do
        [[ "$(grep -Fxc -- "$expected_line" "$result/attestation.env" || true)" == 1 ]] \
            || return 1
    done
    (cd -- "$result" && sha256sum --strict -c MANIFEST.sha256 >/dev/null) \
        || return 1
}

wait_for_child() {
    local pid=$1 status=0
    while true; do
        status=0
        wait "$pid" || status=$?
        if kill -0 "$pid" 2>/dev/null; then
            stop_requested=1
            continue
        fi
        return "$status"
    done
}

publish_result() {
    local logical=$1 marker=$2 native=$3 native_pre=$4 native_post=$5 command_status=$6
    local marker_count=$7 result_status=$8 started=$9 finished=${10}
    local logical_relative logical_sha identity_sha result tmp log_sha
    local marker_relative marker_sha
    logical_relative=${logical#"$workspace"/}
    marker_relative=${marker#"$workspace"/}
    marker_sha=$(sha256_file "$marker" 2>/dev/null || printf missing)
    logical_sha=$(printf '%s' "$logical_relative" | sha256sum)
    logical_sha=${logical_sha%% *}
    identity_sha=$(printf 'schema16-incremental-eof-v1\nLOGICAL=%s\nNATIVE_SHA256=%s\nRUNNER_BUNDLE_TRUST_SHA256=%s\n' \
        "$logical_relative" "$native_pre" "$bundle_trust_sha" | sha256sum)
    identity_sha=${identity_sha%% *}
    if [[ "$result_status" == aborted-* ]]; then
        result="$audit/attempts/$identity_sha.$(date -u +%Y%m%dT%H%M%SZ).$$"
    else
        result="$audit/results/$identity_sha"
    fi
    if [[ -e "$result" ]]; then
        record_replay_result "$result"
        return 0
    fi
    tmp=$(mktemp -d "$audit/.result.tmp.XXXXXX") || return 1
    printf '%s\n' "$logical_relative" >"$tmp/trace.path"
    mv -f -- "$private_log" "$tmp/log"
    private_log=
    printf '%s\n' "$result_status" >"$tmp/status"
    log_sha=$(sha256_file "$tmp/log") || { rm -rf -- "$tmp"; return 1; }
    {
        printf 'FORMAT=schema16-incremental-eof-v1\n'
        printf 'STARTED_UTC=%s\nFINISHED_UTC=%s\n' "$started" "$finished"
        printf 'LOGICAL_PATH_SHA256=%s\nIDENTITY_SHA256=%s\n' "$logical_sha" "$identity_sha"
        printf 'COMPLETION_MARKER=%s\nCOMPLETION_MARKER_SHA256=%s\n' \
            "$marker_relative" "$marker_sha"
        printf 'NATIVE_SHA256_PRE=%s\nNATIVE_SHA256_POST=%s\n' "$native_pre" "$native_post"
        printf 'RUNNER_RAW_SHA256=%s\nRUNNER_BUNDLE_TRUST_SHA256=%s\n' \
            "$runner_sha" "$bundle_trust_sha"
        printf 'RUNNER_BUNDLE_MANIFEST_SHA256=%s\nRUNNER_LIB_MANIFEST_SHA256=%s\n' \
            "$bundle_manifest_sha" "$bundle_lib_manifest_sha"
        printf 'RUNNER_WRAPPER_SHA256=%s\n' "$runner_wrapper_sha"
        printf 'DATA_DIR=%s\nDATA_DIR_PATH_SHA256=%s\n' \
            "$data_dir" "$data_dir_path_sha"
        printf 'COMMAND=original_parity_replay.remote --no-auto-dump LOGICAL_TRACE\n'
        printf 'TIMEOUT_SECONDS=%s\nNICE_LEVEL=%s\nIONICE_CLASS=%s\nIONICE_LEVEL=%s\n' \
            "$timeout_seconds" "$nice_level" "$ionice_class" "$ionice_level"
        printf 'RUNNER_COMMAND_STATUS=%s\nEXACT_EOF_MARKER_COUNT=%s\n' \
            "$command_status" "$marker_count"
        printf 'LOG_SHA256=%s\n' "$log_sha"
    } >"$tmp/attestation.env"
    (cd -- "$tmp" && sha256sum attestation.env log status trace.path >MANIFEST.sha256)
    # The directory rename is the publication boundary: readers see either no
    # tuple or the complete status/log/path/attestation tuple.
    if ! mv -- "$tmp" "$result"; then
        rm -rf -- "$tmp"
        [[ -d "$result" ]] || return 1
    fi
    record_replay_result "$result"
}

record_replay_result() {
    local result=$1
    python3 "$replay_state_tool" import-result "$replay_state_db" "$result" \
        --audit-root "$audit" --workspace "$workspace" --host "$(hostname)" \
        >/dev/null \
        || fail "cannot commit replay result to authoritative database: $result"
}

release_run_locks() {
    local variable fd
    for variable in runner_slot_fd collector_lock_fd native_lock_fd outer_lock_fd trace_lock_fd; do
        if [[ -v "$variable" ]]; then
            fd=${!variable}
            eval "exec ${fd}>&-"
            unset "$variable"
        fi
    done
}

acquire_runner_slot() {
    local slot
    for ((slot = 0; slot < concurrency; slot += 1)); do
        exec {runner_slot_fd}>"$slot_dir/$slot.lock" || return 1
        if flock -n "$runner_slot_fd"; then
            runner_slot=$slot
            return 0
        fi
        exec {runner_slot_fd}>&-
        unset runner_slot_fd
    done
    return 1
}

publish_stop() {
    local finished=$1 identity_sha=$2 result_status=$3
    exec {stop_lock_fd}>"$audit/STOP.lock" || return 1
    flock "$stop_lock_fd" || { exec {stop_lock_fd}>&-; return 1; }
    if [[ ! -e "$audit/STOP.env" ]]; then
        {
            printf 'FAILED_UTC=%s\n' "$finished"
            printf 'IDENTITY_SHA256=%s\nSTATUS=%s\n' "$identity_sha" "$result_status"
        } | write_atomic "$audit/STOP.env" \
            || { exec {stop_lock_fd}>&-; return 1; }
    fi
    exec {stop_lock_fd}>&-
}

publish_batch_fatal() {
    local status=$1 logical=$2
    exec {fatal_lock_fd}>"$audit/STOP.lock" || return 1
    flock "$fatal_lock_fd" || { exec {fatal_lock_fd}>&-; return 1; }
    if [[ ! -e "$audit/BATCH_FATAL.env" ]]; then
        {
            printf 'FAILED_UTC=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
            printf 'STATUS=%s\nLOGICAL=%s\n' "$status" "${logical#"$workspace"/}"
        } | write_atomic "$audit/BATCH_FATAL.env" \
            || { exec {fatal_lock_fd}>&-; return 1; }
    fi
    exec {fatal_lock_fd}>&-
}

run_one() {
    local campaign=$1 marker=$2 logical=$3 native="$3.parity.bitcode.zst"
    local logical_relative logical_sha corpus_relative corpus_sha footer
    local native_pre native_post identity_sha result started finished marker_count
    local command_status=0 result_status runner_pid phase_aborted=0 signal_aborted=0
    local existing_exact
    local peer_failure_aborted=0 batch_failure_aborted=0 admitted_jobs
    local marker_pre marker_post abort_deadline
    [[ ! -e "$audit/BATCH_FATAL.env" ]] || return 7
    [[ ! -e "$audit/STOP.env" ]] || return 6
    [[ -f "$native" ]] || return 0
    path_has_newline "$logical" && fail "newline in logical trace path: $logical"
    logical_relative=${logical#"$workspace"/}
    logical_sha=$(printf '%s' "$logical_relative" | sha256sum)
    logical_sha=${logical_sha%% *}
    exec {trace_lock_fd}>"$audit/.trace-locks/$logical_sha.lock"
    if ! flock -n "$trace_lock_fd"; then
        release_run_locks
        return 0
    fi
    # Coordinate with final validation, conversion, distributed import, and
    # other replay sweeps without ever waiting in front of production work.
    exec {outer_lock_fd}>"$outer_lock" || { release_run_locks; return 4; }
    flock -s -n "$outer_lock_fd" \
        || { release_run_locks; return 4; }
    corpus_relative=${campaign#"$workspace"/}
    corpus_sha=$(printf '%s' "$corpus_relative" | sha256sum); corpus_sha=${corpus_sha%% *}
    exec {native_lock_fd}>"$native_lock_dir/$corpus_sha.lock" \
        || { release_run_locks; return 4; }
    flock -s -n "$native_lock_fd" \
        || { release_run_locks; return 4; }
    exec {collector_lock_fd}>"$campaign/.distributed-collector.lock" \
        || { release_run_locks; return 4; }
    flock -s -n "$collector_lock_fd" \
        || { release_run_locks; return 4; }
    admission_open \
        || { release_run_locks; return 3; }
    [[ -f "$marker" && -f "$native" && ! -e "$logical" \
        && ! -e "$logical.parity-conversion-source" ]] \
        || { release_run_locks; return 0; }
    footer=$(tail -c 36 -- "$native" 2>/dev/null | head -c 16 || true)
    [[ "$footer" == RHPRTRACEFOOTER! ]] \
        || { release_run_locks; return 0; }
    marker_pre=$(sha256_file "$marker") || { release_run_locks; return 0; }
    native_pre=$(sha256_file "$native") || { release_run_locks; return 0; }
    identity_sha=$(printf 'schema16-incremental-eof-v1\nLOGICAL=%s\nNATIVE_SHA256=%s\nRUNNER_BUNDLE_TRUST_SHA256=%s\n' \
        "$logical_relative" "$native_pre" "$bundle_trust_sha" | sha256sum)
    identity_sha=${identity_sha%% *}
    result="$audit/results/$identity_sha"
    if [[ -e "$result" ]]; then
        verify_result "$result" "$native_pre" "$logical_relative" "$marker" \
            || fail "existing incremental proof is not exact/authentic: $result"
        release_run_locks
        return 0
    fi
    existing_exact=$(python3 "$replay_state_tool" has-attested-exact \
        "$replay_state_db" "$logical_relative" \
        --runner-trust "$bundle_trust_sha" --native-sha256 "$native_pre") \
        || fail "cannot query exact replay evidence: $logical_relative"
    if [[ "$existing_exact" == 1 ]]; then
        release_run_locks
        return 0
    fi
    [[ "$existing_exact" == 0 ]] \
        || fail "malformed exact replay query result: $existing_exact"

    # Reused proofs consume no runner slot or memory reservation. A new replay
    # reserves according to the lowest globally available slot, which is a
    # conservative count of already-running replay processes even when other
    # controllers share the slot directory.
    acquire_runner_slot \
        || { release_run_locks; return 4; }
    admitted_jobs=$((runner_slot + 1))

    # This is the final admission boundary. The watchdog below aborts an
    # in-flight child as a non-proof if production moves into its drain phase.
    admission_open || { release_run_locks; return 3; }
    resource_gate_open "$admitted_jobs" \
        || { append_gate_sample closed "$admitted_jobs"; release_run_locks; return 4; }
    append_gate_sample admitted "$admitted_jobs"
    verify_bundle
    native_pre=$(sha256_file "$native") || { release_run_locks; return 0; }
    identity_sha=$(printf 'schema16-incremental-eof-v1\nLOGICAL=%s\nNATIVE_SHA256=%s\nRUNNER_BUNDLE_TRUST_SHA256=%s\n' \
        "$logical_relative" "$native_pre" "$bundle_trust_sha" | sha256sum)
    identity_sha=${identity_sha%% *}
    private_log=$(mktemp "$audit/.runner.log.tmp.XXXXXX") \
        || fail "cannot create private log for $logical"
    # Bundle verification and private-log allocation can take long enough for
    # production to close admission. Recheck at the last possible boundary;
    # after this succeeds the next operation creates the runner process group.
    if ! admission_open; then
        rm -f -- "$private_log"
        private_log=
        release_run_locks
        return 3
    fi
    if [[ -e "$controller_stop_file" ]]; then
        rm -f -- "$private_log"
        private_log=
        release_run_locks
        return 5
    fi
    # STOP.lock is both the first-failure publisher lock and the final replay
    # start gate. A worker delayed in hashing/bundle verification cannot cross
    # this boundary after a sibling has published STOP.env.
    exec {start_gate_fd}>"$audit/STOP.lock" \
        || { rm -f -- "$private_log"; private_log=; release_run_locks; return 4; }
    flock "$start_gate_fd" \
        || { rm -f -- "$private_log"; private_log=; release_run_locks; return 4; }
    if [[ -e "$audit/BATCH_FATAL.env" ]]; then
        rm -f -- "$private_log"
        private_log=
        exec {start_gate_fd}>&-
        release_run_locks
        return 7
    fi
    if [[ -e "$audit/STOP.env" ]]; then
        rm -f -- "$private_log"
        private_log=
        exec {start_gate_fd}>&-
        release_run_locks
        return 6
    fi
    if [[ -e "$controller_stop_file" ]]; then
        rm -f -- "$private_log"
        private_log=
        exec {start_gate_fd}>&-
        release_run_locks
        return 5
    fi
    if ! admission_open; then
        rm -f -- "$private_log"
        private_log=
        exec {start_gate_fd}>&-
        release_run_locks
        return 3
    fi
    started=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    if [[ "$ionice_class" == 2 ]]; then
        setsid timeout --foreground --signal=TERM --kill-after=10s "${timeout_seconds}s" \
            nice -n "$nice_level" ionice -c "$ionice_class" -n "$ionice_level" env \
            ROBINHOOD_DATA_DIR="$workspace/datadirs/fullgame_linux" \
            "$bundle/original_parity_replay.remote" --no-auto-dump "$logical" \
            {start_gate_fd}>&- >"$private_log" 2>&1 &
    else
        setsid timeout --foreground --signal=TERM --kill-after=10s "${timeout_seconds}s" \
            nice -n "$nice_level" ionice -c "$ionice_class" env \
            ROBINHOOD_DATA_DIR="$workspace/datadirs/fullgame_linux" \
            "$bundle/original_parity_replay.remote" --no-auto-dump "$logical" \
            {start_gate_fd}>&- >"$private_log" 2>&1 &
    fi
    runner_pid=$!
    current_child=$runner_pid
    exec {start_gate_fd}>&-
    while kill -0 "$runner_pid" 2>/dev/null; do
        if [[ -e "$controller_stop_file" ]]; then
            signal_aborted=1
            kill -TERM -- "-$runner_pid" 2>/dev/null || true
            break
        elif [[ -e "$audit/BATCH_FATAL.env" ]]; then
            batch_failure_aborted=1
            kill -TERM -- "-$runner_pid" 2>/dev/null || true
            break
        elif [[ -e "$audit/STOP.env" ]]; then
            peer_failure_aborted=1
            kill -TERM -- "-$runner_pid" 2>/dev/null || true
            break
        elif ! admission_open; then
            phase_aborted=1
            kill -TERM -- "-$runner_pid" 2>/dev/null || true
            break
        fi
        [[ -r "/proc/$runner_pid/stat" \
            && "$(awk '{print $3}' "/proc/$runner_pid/stat" 2>/dev/null)" == Z ]] && break
        sleep 1
    done
    if (( phase_aborted == 1 || signal_aborted == 1 || peer_failure_aborted == 1 \
        || batch_failure_aborted == 1 )); then
        abort_deadline=$((SECONDS + abort_grace_seconds))
        while kill -0 "$runner_pid" 2>/dev/null; do
            [[ -r "/proc/$runner_pid/stat" \
                && "$(awk '{print $3}' "/proc/$runner_pid/stat" 2>/dev/null)" == Z ]] \
                && break
            if (( SECONDS >= abort_deadline )); then
                kill -KILL -- "-$runner_pid" 2>/dev/null || true
                break
            fi
            sleep 1
        done
    fi
    wait_for_child "$runner_pid" || command_status=$?
    current_child=
    finished=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    native_post=$(sha256_file "$native" 2>/dev/null || printf missing)
    marker_post=$(sha256_file "$marker" 2>/dev/null || printf missing)
    verify_bundle
    marker_count=$(grep -Fxc -- "$exact_eof_marker" "$private_log" || true)
    if (( signal_aborted == 1 )); then
        result_status=aborted-controller-signal
    elif (( batch_failure_aborted == 1 )); then
        result_status=aborted-batch-failure
    elif (( peer_failure_aborted == 1 )); then
        result_status=aborted-peer-failure
    elif (( phase_aborted == 1 )); then
        result_status=aborted-phase-transition
    elif [[ "$native_post" != "$native_pre" || "$marker_post" != "$marker_pre" \
        || -e "$logical" || -e "$logical.parity-conversion-source" ]]; then
        result_status=integrity-native-changed
    elif (( command_status != 0 )); then
        result_status=$command_status
    elif [[ "$marker_count" != 1 ]]; then
        result_status=integrity-eof-marker
    else
        result_status=0
    fi
    publish_result "$logical" "$marker" "$native" "$native_pre" "$native_post" \
        "$command_status" "$marker_count" "$result_status" "$started" "$finished" \
        || fail "cannot atomically publish result for $logical"
    release_run_locks
    if (( signal_aborted == 1 )); then
        return 5
    fi
    if (( phase_aborted == 1 )); then
        return 3
    fi
    if (( peer_failure_aborted == 1 )); then
        return 6
    fi
    if (( batch_failure_aborted == 1 )); then
        return 7
    fi
    if [[ "$result_status" != 0 ]]; then
        publish_stop "$finished" "$identity_sha" "$result_status" \
            || fail 'cannot publish incremental stop'
        return 1
    fi
}

is_uint "$poll_seconds" && (( poll_seconds > 0 )) || fail 'poll seconds must be positive'
is_uint "$timeout_seconds" && (( timeout_seconds > 0 )) || fail 'timeout seconds must be positive'
is_uint "$abort_grace_seconds" || fail 'abort grace seconds must be unsigned'
min_memory_kib=$(normalize_bounded_uint "$min_memory_kib" 9223372036854775807) \
    || fail 'minimum memory must fit signed 64-bit KiB arithmetic'
memory_per_job_kib=$(normalize_bounded_uint "$memory_per_job_kib" 1152921504606846975) \
    || fail 'per-job memory exceeds safe concurrency arithmetic'
is_nonnegative_number "$max_load1" || fail 'maximum load must be nonnegative'
is_uint "$concurrency" && (( concurrency >= 1 && concurrency <= 16 )) \
    || fail 'concurrency must be between 1 and 16'
(( min_memory_kib <= 9223372036854775807 \
    - (concurrency - 1) * memory_per_job_kib )) \
    || fail 'configured concurrency memory arithmetic overflows'
[[ "$oneshot" == 0 || "$oneshot" == 1 ]] || fail 'oneshot must be 0 or 1'
[[ "$admitted_phase" =~ ^[a-z0-9-]+$ ]] || fail 'admitted phase is malformed'
[[ "$allow_drained" == 0 || "$allow_drained" == 1 ]] \
    || fail 'allow-drained flag must be 0 or 1'
[[ "$nice_level" =~ ^([0-9]|1[0-9])$ ]] || fail 'nice level must be 0 through 19'
[[ "$ionice_class" == 2 || "$ionice_class" == 3 ]] \
    || fail 'ionice class must be best-effort (2) or idle (3)'
[[ "$ionice_level" =~ ^[0-7]$ ]] || fail 'ionice level must be 0 through 7'
[[ "$bundle_trust_sha" =~ ^[0-9a-f]{64}$ && "$runner_sha" =~ ^[0-9a-f]{64}$ ]] \
    || fail 'runner hashes must be 64 lowercase hexadecimal digits'

workspace=$(realpath -e -- "$workspace_arg") || fail 'workspace does not exist'
replay_state_tool=${REPLAY_STATE_TOOL:-${0%/*}/replay_state_db.py}
replay_state_tool=$(realpath -e -- "$replay_state_tool") \
    || fail 'replay-state database tool does not exist'
[[ -f "$replay_state_tool" ]] || fail 'replay-state database tool is not a file'
replay_state_db=${REPLAY_STATE_DB:-$workspace/parity-save-replays/replay-state.sqlite3}
replay_state_db=$(realpath -m -- "$replay_state_db")
[[ "$replay_state_db" == "$workspace"/* ]] \
    || fail 'replay-state database must be below workspace'
bundle=$(realpath -e -- "$bundle_arg") || fail 'runner bundle does not exist'
orchestrator_audit=$(realpath -e -- "$orchestrator_audit_arg") \
    || fail 'orchestrator audit does not exist'
mkdir -p -- "$audit_arg"
audit=$(realpath -e -- "$audit_arg") || fail 'audit does not exist'
campaigns=()
for campaign_arg in "${campaign_args[@]}"; do
    campaign=$(realpath -e -- "$campaign_arg") || fail "campaign does not exist: $campaign_arg"
    [[ "$campaign" == "$workspace"/* && -d "$campaign/traces" ]] \
        || fail "campaign must be below workspace and contain traces: $campaign"
    campaigns+=("$campaign")
done
[[ "$audit" == "$workspace"/* && "$orchestrator_audit" == "$workspace"/* ]] \
    || fail 'audit directories must be below workspace'
mkdir -p -- "$audit/results" "$audit/attempts" "$audit/.trace-locks"
python3 "$replay_state_tool" init "$replay_state_db" >/dev/null \
    || fail 'cannot initialize authoritative replay-state database'
if ! python3 "$replay_state_tool" import-audit "$replay_state_db" "$audit" \
    --workspace "$workspace" --host "$(hostname)" >/dev/null
then
    publish_batch_fatal integrity-audit-import "$audit" \
        || fail 'cannot publish audit-recovery integrity failure'
    fail 'cannot recover existing audit into authoritative replay-state database'
fi
mkdir -p -- "$native_lock_dir" "$slot_dir" \
    "${outer_lock%/*}"
exec {controller_fd}>"$audit/controller.lock"
flock -n "$controller_fd" || fail "another incremental controller owns $audit"
[[ ! -e "$audit/BATCH_FATAL.env" ]] || {
    printf 'error: incremental audit is stopped by existing BATCH_FATAL.env: %s\n' \
        "$audit/BATCH_FATAL.env" >&2
    exit 2
}
[[ ! -e "$audit/STOP.env" ]] || {
    printf 'error: incremental audit is stopped by existing STOP.env: %s\n' \
        "$audit/STOP.env" >&2
    exit 1
}
verify_bundle
bundle_manifest_sha=$(sha256_file "$bundle/SHA256SUMS")
bundle_lib_manifest_sha=$(sha256_file "$bundle/LIB_SHA256SUMS")
runner_wrapper_sha=$(sha256_file "$bundle/original_parity_replay.remote")
data_dir=$(realpath -e -- "$workspace/datadirs/fullgame_linux") \
    || fail 'fullgame_linux data directory does not exist'
data_dir_path_sha=$(printf '%s' "$data_dir" | sha256sum); data_dir_path_sha=${data_dir_path_sha%% *}
script_sha=$(sha256_file "$0")

provenance="$audit/provenance.env"
provenance_tmp=$(mktemp "$audit/provenance.env.tmp.XXXXXX") || fail 'cannot stage provenance'
{
    printf 'FORMAT=schema16-incremental-eof-controller-v1\n'
    printf 'SCRIPT_SHA256=%s\nWORKSPACE=%s\nRUNNER_BUNDLE=%s\n' \
        "$script_sha" "$workspace" "$bundle"
    printf 'REPLAY_STATE_DB=%s\nREPLAY_STATE_TOOL_SHA256=%s\n' \
        "$replay_state_db" "$(sha256_file "$replay_state_tool")"
    printf 'RUNNER_RAW_SHA256=%s\nRUNNER_BUNDLE_TRUST_SHA256=%s\n' \
        "$runner_sha" "$bundle_trust_sha"
    printf 'RUNNER_BUNDLE_MANIFEST_SHA256=%s\nRUNNER_LIB_MANIFEST_SHA256=%s\n' \
        "$bundle_manifest_sha" "$bundle_lib_manifest_sha"
    printf 'RUNNER_WRAPPER_SHA256=%s\nORCHESTRATOR_AUDIT=%s\n' \
        "$runner_wrapper_sha" "$orchestrator_audit"
    printf 'CONCURRENCY=%s\nNICE_LEVEL=%s\nIONICE_CLASS=%s\nIONICE_LEVEL=%s\n' \
        "$concurrency" "$nice_level" "$ionice_class" "$ionice_level"
    printf 'MIN_MEMORY_KIB=%s\nMEMORY_PER_JOB_KIB=%s\nMAX_LOAD1=%s\n' \
        "$min_memory_kib" "$memory_per_job_kib" "$max_load1"
    printf 'OUTER_LOCK=%s\nNATIVE_LOCK_DIR=%s\nRUNNER_SLOT_DIR=%s\n' \
        "$outer_lock" "$native_lock_dir" "$slot_dir"
    printf 'MEMINFO_PATH=%s\nLOADAVG_PATH=%s\nPOLL_SECONDS=%s\nTIMEOUT_SECONDS=%s\n' \
        "$meminfo_path" "$loadavg_path" "$poll_seconds" "$timeout_seconds"
    printf 'ABORT_GRACE_SECONDS=%s\n' "$abort_grace_seconds"
    printf 'ADMITTED_PHASE=%s\n' "$admitted_phase"
    printf 'ALLOW_DRAINED=%s\n' "$allow_drained"
    printf 'CAMPAIGN_COUNT=%s\n' "${#campaigns[@]}"
    for ((campaign_index = 0; campaign_index < ${#campaigns[@]}; campaign_index += 1)); do
        printf 'CAMPAIGN_%s=%s\n' "$campaign_index" "${campaigns[campaign_index]}"
    done
} >"$provenance_tmp"
if [[ -f "$provenance" ]]; then
    cmp -s -- "$provenance_tmp" "$provenance" \
        || fail "incremental audit provenance mismatch: $provenance"
    rm -f -- "$provenance_tmp"
else
    mv -- "$provenance_tmp" "$provenance"
fi
[[ -f "$audit/resource-gate.tsv" ]] \
    || printf 'UTC\tMEM_AVAILABLE_KIB\tLOAD1\tADMITTED_JOBS\tREQUIRED_MEMORY_KIB\tRESULT\n' \
        >"$audit/resource-gate.tsv"

stop_requested=0
signal_requested=0
current_child=
private_log=
worker_pids=()
controller_pid=$BASHPID
controller_stop_file="$audit/.controller-stop.$$"
controller_signal() {
    stop_requested=1
    signal_requested=1
    : >"$controller_stop_file"
}
trap controller_signal INT TERM HUP
cleanup() {
    [[ -z "$private_log" ]] || rm -f -- "$private_log"
    if (( BASHPID == controller_pid )); then
        rm -f -- "$controller_stop_file"
    fi
}
trap cleanup EXIT

worker_signal() {
    : >"$controller_stop_file"
}

worker_cleanup() {
    local worker_exit_status=$? deadline
    case $worker_exit_status in
        0|1|3|4|5|6|7) ;;
        *) publish_batch_fatal "$worker_exit_status" \
            "${worker_logical:-unknown}" || true ;;
    esac
    if [[ -n "${current_child:-}" ]] && kill -0 "$current_child" 2>/dev/null; then
        kill -TERM -- "-$current_child" 2>/dev/null || true
        deadline=$((SECONDS + abort_grace_seconds))
        while kill -0 "$current_child" 2>/dev/null; do
            if (( SECONDS >= deadline )); then
                kill -KILL -- "-$current_child" 2>/dev/null || true
                break
            fi
            sleep 1
        done
        wait "$current_child" 2>/dev/null || true
        current_child=
    fi
    [[ -z "${private_log:-}" ]] || rm -f -- "$private_log"
}

wait_batch() {
    local pid status=0 normalized priority batch_status=0 batch_priority=0
    for pid in "${worker_pids[@]}"; do
        # Poll until the worker has exited or is a zombie, then reap it once.
        # This keeps controller INT/TERM/HUP from interrupting a blocking wait
        # and turning a still-owned asynchronous worker into status 127/143.
        while kill -0 "$pid" 2>/dev/null; do
            if [[ -r "/proc/$pid/stat" \
                && "$(awk '{print $3}' "/proc/$pid/stat" 2>/dev/null)" == Z ]]; then
                break
            fi
            sleep 1
        done
        status=0
        wait "$pid" || status=$?
        if (( status != 0 )); then
            if (( status >= 128 )) && [[ -e "$controller_stop_file" ]]; then
                normalized=5
                priority=3
            else case $status in
                2) normalized=2; priority=5 ;;
                1|6) normalized=1; priority=4 ;;
                5) normalized=5; priority=3 ;;
                3) normalized=3; priority=2 ;;
                4) normalized=4; priority=1 ;;
                *) normalized=2; priority=5 ;;
            esac
            fi
            if (( priority > batch_priority )); then
                batch_status=$normalized
                batch_priority=$priority
            fi
        fi
    done
    worker_pids=()
    return "$batch_status"
}

while (( stop_requested == 0 )); do
    admission_open || break
    found=0
    while IFS= read -r -d '' campaign \
        && IFS= read -r -d '' marker \
        && IFS= read -r -d '' logical
    do
        found=1
        (
            signal_requested=0
            current_child=
            private_log=
            worker_logical=$logical
            trap worker_signal INT TERM HUP
            trap worker_cleanup EXIT
            run_one "$campaign" "$marker" "$logical"
        ) &
        worker_pids+=("$!")
        if (( ${#worker_pids[@]} == concurrency )); then
            run_status=0
            wait_batch || run_status=$?
            (( run_status == 0 )) || {
                if (( run_status == 3 || run_status == 5 )); then
                    stop_requested=1
                    break
                elif (( run_status == 6 )); then
                    exit 1
                elif (( run_status == 4 )); then
                    break
                fi
                exit "$run_status"
            }
        fi
        (( stop_requested == 0 )) || break
    done < <(list_candidates)
    if (( ${#worker_pids[@]} != 0 )); then
        run_status=0
        wait_batch || run_status=$?
        (( run_status == 0 )) || {
            if (( run_status == 3 || run_status == 5 )); then
                stop_requested=1
            elif (( run_status == 6 )); then
                exit 1
            elif (( run_status != 4 )); then
                exit "$run_status"
            fi
        }
    fi
    (( stop_requested == 0 )) || break
    (( oneshot == 0 )) || break
    admission_open || break
    sleep "$poll_seconds" &
    sleep_pid=$!
    wait "$sleep_pid" || true
done

{
    printf 'STOPPED_UTC=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    if admission_open; then
        printf 'REASON=signal-or-oneshot\n'
    else
        printf 'REASON=admission-closed\n'
    fi
    printf 'IN_FLIGHT_CHILD=none\n'
} | write_atomic "$audit/controller-finished.env" \
    || fail 'cannot publish controller completion'
