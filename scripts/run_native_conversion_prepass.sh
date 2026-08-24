#!/usr/bin/env bash
set -euo pipefail

# Normalize one drained parity corpus to native bitcode with durable per-trace
# evidence. For production, PINNED_RUNNER_OR_BUNDLE is an immutable bundle
# directory and TRUST_SHA256 is RUNNER_BUNDLE_SHA256_V1:
#   sha256("schema16-runner-bundle-v1\n" +
#          "SHA256SUMS=<sha256(SHA256SUMS)>\n" +
#          "LIB_SHA256SUMS=<sha256(LIB_SHA256SUMS)>\n")
# Direct executable + raw SHA is rejected unless the explicit
# NATIVE_CONVERT_TEST_ALLOW_DIRECT_RUNNER=1 fixture escape hatch is set. The
# default is a globally quiet host, two jobs, and a separate 7200-second
# conversion watchdog. NATIVE_CONVERT_ALLOW_OTHER_CORPORA=1 is corpus-safe but
# requires one idle-priority job; target admission/import locks are always held.

if (( $# != 5 )); then
    printf 'usage: %s WORKSPACE CORPUS PINNED_RUNNER_OR_BUNDLE TRUST_SHA256 AUDIT_DIR\n' "$0" >&2
    exit 2
fi

workspace_arg=$1
corpus_arg=$2
runner_arg=$3
expected_trust_sha=${4,,}
audit_arg=$5

jobs=${NATIVE_CONVERT_JOBS:-2}
timeout_seconds=${NATIVE_CONVERT_TIMEOUT_SECONDS:-7200}
allow_other_corpora=${NATIVE_CONVERT_ALLOW_OTHER_CORPORA:-0}
allow_direct_fixture=${NATIVE_CONVERT_TEST_ALLOW_DIRECT_RUNNER:-0}
minimum_free_kib=${NATIVE_CONVERT_MIN_FREE_KIB:-10485760}
minimum_available_kib_per_job=${NATIVE_CONVERT_MIN_AVAILABLE_KIB_PER_JOB:-8388608}

fail() {
    printf 'error: %s\n' "$*" >&2
    exit 2
}

sha256_file() {
    local result
    result=$(sha256sum -- "$1") || return 1
    printf '%s\n' "${result%% *}"
}

write_atomic() {
    local destination=$1 temporary
    temporary=$(mktemp "${destination}.tmp.XXXXXX") || return 1
    if ! cat >"$temporary" || ! mv -f -- "$temporary" "$destination"; then
        rm -f -- "$temporary"
        return 1
    fi
}

[[ "$jobs" =~ ^[1-9][0-9]*$ && "$jobs" -le 3 ]] \
    || fail 'NATIVE_CONVERT_JOBS must be between 1 and 3'
[[ "$timeout_seconds" =~ ^[0-9]+$ && "$timeout_seconds" -ge 3600 ]] \
    || fail 'NATIVE_CONVERT_TIMEOUT_SECONDS must be at least 3600'
[[ "$allow_other_corpora" == 0 || "$allow_other_corpora" == 1 ]] \
    || fail 'NATIVE_CONVERT_ALLOW_OTHER_CORPORA must be 0 or 1'
[[ "$allow_direct_fixture" == 0 || "$allow_direct_fixture" == 1 ]] \
    || fail 'NATIVE_CONVERT_TEST_ALLOW_DIRECT_RUNNER must be 0 or 1'
[[ "$minimum_free_kib" =~ ^[0-9]+$ ]] \
    || fail 'NATIVE_CONVERT_MIN_FREE_KIB must be an unsigned integer'
[[ "$minimum_available_kib_per_job" =~ ^[0-9]+$ ]] \
    || fail 'NATIVE_CONVERT_MIN_AVAILABLE_KIB_PER_JOB must be an unsigned integer'
[[ "$expected_trust_sha" =~ ^[0-9a-f]{64}$ ]] \
    || fail 'TRUST_SHA256 must contain 64 hexadecimal digits'

[[ -d "$workspace_arg" ]] || fail "workspace is not a directory: $workspace_arg"
workspace=$(realpath -e -- "$workspace_arg")
[[ -d "$corpus_arg/traces" ]] || fail "corpus has no traces directory: $corpus_arg"
corpus=$(realpath -e -- "$corpus_arg")
[[ "$corpus" == "$workspace"/* ]] || fail "corpus is outside workspace: $corpus"
runner_is_bundle=0
if [[ -d "$runner_arg" ]]; then
    runner_is_bundle=1
    runner_dir=$(realpath -e -- "$runner_arg")
    runner="$runner_dir/original_parity_replay"
else
    (( allow_direct_fixture == 1 )) \
        || fail 'operational conversion requires an authenticated protocol-2 runner bundle'
    [[ -x "$runner_arg" ]] || fail "runner is not executable: $runner_arg"
    runner=$(realpath -e -- "$runner_arg")
    runner_dir=${runner%/*}
fi

audit=$(realpath -m -- "$audit_arg")
audit_parent=$(dirname -- "$audit")
mkdir -p -- "$audit_parent" || fail "cannot create audit parent: $audit_parent"
audit_parent=$(realpath -e -- "$audit_parent")
audit="$audit_parent/$(basename -- "$audit")"
[[ "$audit" == "$workspace"/* ]] || fail "audit is outside workspace: $audit"
[[ "$audit" != "$corpus" && "$audit" != "$corpus"/* ]] \
    || fail 'audit must not be inside the corpus'

# A packaged runner executes its authenticated hermetic wrapper. The composite
# trust digest binds that wrapper, the raw binary, and every bundled library.
runner_exec=$runner
wrapper_sha=none
wrapper_path=
loader_path=
bundle_lib_dir=
bundle_manifest_sha=none
bundle_lib_manifest_sha=none
native_conversion_protocol=fixture-direct
runner_trust_sha=
actual_runner_sha=
if (( runner_is_bundle == 1 ))
then
    [[ -x "$runner" && -x "$runner_dir/original_parity_replay.remote" \
        && -x "$runner_dir/lib/ld-linux-x86-64.so.2" \
        && -f "$runner_dir/SHA256SUMS" && -f "$runner_dir/LIB_SHA256SUMS" \
        && -f "$runner_dir/PROVENANCE.txt" && -f "$runner_dir/LOADER_LIST.txt" ]] \
        || fail "packaged runner lacks checksum manifests: $runner_dir"
    mapfile -t protocol_values < <(
        sed -n 's/^NATIVE_CONVERSION_PROTOCOL=//p' "$runner_dir/PROVENANCE.txt"
    )
    [[ ${#protocol_values[@]} == 1 && "${protocol_values[0]}" == 2 ]] \
        || fail "bundle must authenticate exactly NATIVE_CONVERSION_PROTOCOL=2: $runner_dir"
    native_conversion_protocol=2
    if find "$runner_dir" -type l -print -quit | grep -q .; then
        fail "packaged runner contains a symlink: $runner_dir"
    fi
    for manifest in "$runner_dir/SHA256SUMS" "$runner_dir/LIB_SHA256SUMS"; do
        while IFS= read -r manifest_line; do
            [[ "$manifest_line" =~ ^[0-9a-fA-F]{64}[[:space:]][\ \*](.+)$ ]] \
                || fail "malformed bundle checksum entry: $manifest"
            manifest_path=${BASH_REMATCH[1]}
            [[ "$manifest_path" != /* && "$manifest_path" != ../* \
                && "$manifest_path" != */../* && "$manifest_path" != *'/..' \
                && "$manifest_path" != *$'\n'* ]] \
                || fail "unsafe bundle checksum path: $manifest_path"
        done <"$manifest"
    done
    if ! cmp -s \
        <(printf '%s\n' LIB_SHA256SUMS LOADER_LIST.txt PROVENANCE.txt \
            original_parity_replay original_parity_replay.remote | LC_ALL=C sort) \
        <(cut -d' ' -f3- "$runner_dir/SHA256SUMS" | LC_ALL=C sort -u)
    then
        fail "packaged root manifest does not have the exact required set: $runner_dir"
    fi
    if ! cmp -s \
        <(printf '%s\n' LIB_SHA256SUMS LOADER_LIST.txt PROVENANCE.txt SHA256SUMS \
            original_parity_replay original_parity_replay.remote | LC_ALL=C sort) \
        <(find "$runner_dir" -mindepth 1 -maxdepth 1 -type f -printf '%f\n' \
            | LC_ALL=C sort -u)
    then
        fail "packaged bundle root does not have the exact required file set: $runner_dir"
    fi
    [[ "$(find "$runner_dir" -mindepth 1 -maxdepth 1 -type d -printf '%f\n')" == lib ]] \
        || fail "packaged bundle root has an unexpected directory: $runner_dir"
    grep -Fq -- "=> $runner_dir/lib/ld-linux-x86-64.so.2 " "$runner_dir/LOADER_LIST.txt" \
        || fail "loader proof was not generated from final bundle path: $runner_dir"
    if ! awk -v prefix="$runner_dir/lib/" '
        /=>/ {
            resolved=$0
            sub(/^.*=>[[:space:]]*/, "", resolved)
            sub(/[[:space:]].*$/, "", resolved)
            if (index(resolved, prefix) != 1) exit 1
        }
    ' "$runner_dir/LOADER_LIST.txt"; then
        fail "loader proof resolves outside final bundle lib directory: $runner_dir"
    fi
    grep -Eq '^[0-9a-fA-F]{64} [ *]original_parity_replay$' "$runner_dir/SHA256SUMS" \
        && grep -Eq '^[0-9a-fA-F]{64} [ *]original_parity_replay\.remote$' "$runner_dir/SHA256SUMS" \
        && grep -Eq '^[0-9a-fA-F]{64} [ *]LIB_SHA256SUMS$' "$runner_dir/SHA256SUMS" \
        && grep -Eq '^[0-9a-fA-F]{64} [ *]PROVENANCE\.txt$' "$runner_dir/SHA256SUMS" \
        && grep -Eq '^[0-9a-fA-F]{64} [ *]LOADER_LIST\.txt$' "$runner_dir/SHA256SUMS" \
        && grep -Eq '^[0-9a-fA-F]{64} [ *]lib/ld-linux-x86-64\.so\.2$' \
            "$runner_dir/LIB_SHA256SUMS" \
        || fail "packaged manifests omit required trust inputs: $runner_dir"
    (cd "$runner_dir" && sha256sum --strict -c SHA256SUMS \
        && sha256sum --strict -c LIB_SHA256SUMS) \
        >/dev/null || fail "packaged runner checksum verification failed: $runner_dir"
    if ! cmp -s \
        <(cut -d' ' -f3- "$runner_dir/LIB_SHA256SUMS" | LC_ALL=C sort -u) \
        <(cd "$runner_dir" && find lib \( -type f -o -type l \) -print | LC_ALL=C sort -u)
    then
        fail "packaged library manifest does not cover the exact lib tree: $runner_dir"
    fi
    wrapper_path="$runner_dir/original_parity_replay.remote"
    wrapper_sha=$(sha256_file "$wrapper_path") \
        || fail 'cannot hash runner wrapper'
    runner_exec=$wrapper_path
    bundle_manifest_sha=$(sha256_file "$runner_dir/SHA256SUMS") \
        || fail 'cannot hash packaged runner manifest'
    bundle_lib_manifest_sha=$(sha256_file "$runner_dir/LIB_SHA256SUMS") \
        || fail 'cannot hash packaged library manifest'
    runner_trust_sha=$(printf 'schema16-runner-bundle-v1\nSHA256SUMS=%s\nLIB_SHA256SUMS=%s\n' \
        "$bundle_manifest_sha" "$bundle_lib_manifest_sha" | sha256sum)
    runner_trust_sha=${runner_trust_sha%% *}
    [[ "$runner_trust_sha" == "$expected_trust_sha" ]] \
        || fail "runner bundle trust mismatch: expected $expected_trust_sha, got $runner_trust_sha"
    actual_runner_sha=$(sha256_file "$runner") || fail "cannot hash runner: $runner"
else
    actual_runner_sha=$(sha256_file "$runner") || fail "cannot hash runner: $runner"
    runner_trust_sha=$actual_runner_sha
    [[ "$runner_trust_sha" == "$expected_trust_sha" ]] \
        || fail "runner hash mismatch: expected $expected_trust_sha, got $runner_trust_sha"
fi
if (( runner_is_bundle == 0 )) && file -b -- "$runner" | grep -q '^ELF '; then
    command -v readelf >/dev/null || fail 'readelf is required to verify the ELF loader'
    loader_path=$(readelf -l -- "$runner" \
        | sed -n 's/.*Requesting program interpreter: \([^]]*\)].*/\1/p')
    [[ -n "$loader_path" && -x "$loader_path" ]] \
        || fail "ELF interpreter is missing or not executable: ${loader_path:-unknown}"
    if ldd -- "$runner" | grep -Fq 'not found'; then
        fail "runner has an unresolved shared library: $runner"
    fi
fi

global_outer_lock=none
if (( allow_other_corpora == 1 )); then
    (( jobs == 1 )) || fail 'concurrent-campaign mode requires NATIVE_CONVERT_JOBS=1'
    nice_level=19
    ionice_class=3
    ionice_level=
else
    nice_level=10
    ionice_class=2
    ionice_level=7
    global_outer_lock=${NATIVE_CONVERT_OUTER_LOCK:-/tmp/robin-parity-runner.lock}
    mkdir -p -- "${global_outer_lock%/*}" || fail 'cannot create global outer-lock directory'
    exec {global_outer_lock_fd}>"$global_outer_lock" || fail 'cannot open global outer lock'
    flock -n "$global_outer_lock_fd" \
        || fail "global parity/capture drain lock is held: $global_outer_lock"
fi

# Serialize this audit and all prepasses for this corpus.
audit_lock="${audit}.lock"
exec {audit_lock_fd}>"$audit_lock" || fail "cannot open audit lock: $audit_lock"
flock -n "$audit_lock_fd" || fail "another process owns audit lock: $audit_lock"
corpus_digest=$(printf '%s' "${corpus#"$workspace"/}" | sha256sum)
corpus_digest=${corpus_digest%% *}
mkdir -p -- "$workspace/.git/native-conversion-locks" \
    || fail 'cannot create native-conversion lock directory'
exec {corpus_lock_fd}>"$workspace/.git/native-conversion-locks/$corpus_digest.lock" \
    || fail 'cannot open corpus conversion lock'
flock -n "$corpus_lock_fd" || fail "another conversion owns corpus: $corpus"

# Capture admission and distributed import are the two writers of a campaign.
# Hold their corpus-scoped locks before proving that no admitted capture remains.
exec {admission_lock_fd}>"$corpus/.capture-admission.lock" \
    || fail 'cannot open capture admission lock'
flock -n "$admission_lock_fd" || fail "capture admission is active: $corpus"
exec {collector_lock_fd}>"$corpus/.distributed-collector.lock" \
    || fail 'cannot open distributed collector lock'
flock -n "$collector_lock_fd" || fail "distributed collector is active: $corpus"
if [[ -d "$corpus/.capture-reservations" ]] \
    && find "$corpus/.capture-reservations" -type f -name '*.reserve' \
        -print -quit | grep -q .
then
    fail "campaign has a capture reservation: $corpus"
fi

processes_matching() {
    local scope=$1 pid command
    for process in /proc/[0-9]*/cmdline; do
        pid=${process#/proc/}; pid=${pid%/cmdline}
        [[ "$pid" != "$$" ]] || continue
        command=$(tr '\0' ' ' <"$process" 2>/dev/null) || continue
        case "$command" in
            *original_parity_replay*' --convert '*|*' -PARITYTRACE '*|*run_schema16_distributed_capture.sh*|*rsync*) ;;
            *) continue ;;
        esac
        if [[ "$scope" == global || "$command" == *"$corpus"* ]]; then
            printf '%s\t%s\n' "$pid" "$command"
        fi
    done
}

# The held admission and collector locks are the correctness boundary for this
# corpus. The global process inventory is necessarily observational because
# legacy producers have no shared cross-corpus admission lock; use it only as
# a resource-contention guard, and record any explicit waiver in provenance.
target_writers=$(processes_matching target)
[[ -z "$target_writers" ]] \
    || fail "target corpus still has an active writer:\n$target_writers"
if (( allow_other_corpora == 0 )); then
    global_writers=$(processes_matching global)
    [[ -z "$global_writers" ]] \
        || fail "capture/conversion activity is not globally drained:\n$global_writers"
fi

available_kib=$(df -Pk -- "$corpus" | awk 'NR == 2 {print $4}')
[[ "$available_kib" =~ ^[0-9]+$ && "$available_kib" -ge "$minimum_free_kib" ]] \
    || fail "insufficient free disk KiB: ${available_kib:-unknown}"
memory_available_kib=$(awk '/^MemAvailable:/ {print $2}' /proc/meminfo)
required_memory_kib=$((jobs * minimum_available_kib_per_job))
[[ "$memory_available_kib" =~ ^[0-9]+$ \
    && "$memory_available_kib" -ge "$required_memory_kib" ]] \
    || fail "insufficient available memory KiB: ${memory_available_kib:-unknown} < $required_memory_kib"

# COMPLETE is a current-state commit marker, not an eternal claim. Preserve
# its prior contents before rechecking a previously completed audit.
if [[ -f "$audit/COMPLETE" ]]; then
    mv -f -- "$audit/COMPLETE" "$audit/COMPLETE.previous" \
        || fail 'cannot retire prior completion marker'
fi
if [[ -f "$audit/native.SHA256SUMS" && ! -f "$audit/COMPLETE.previous" ]]; then
    # Recover the sole first-publication crash window: the immutable native
    # manifest was renamed, but its small anchor was not. Accept it only when
    # its path set is exactly the frozen logical snapshot and every byte still
    # matches; this state cannot arise after an anchored completion.
    while IFS= read -r manifest_line; do
        [[ "$manifest_line" =~ ^[0-9a-f]{64}[[:space:]][[:space:]](/.*)$ ]] \
            || fail 'unanchored native manifest has a malformed entry'
    done <"$audit/native.SHA256SUMS"
    if ! cmp -s \
        <(sed -E 's/^[0-9a-f]{64}  //' "$audit/native.SHA256SUMS" | LC_ALL=C sort) \
        <(while IFS= read -r logical; do \
            printf '%s\n' "$logical.parity.bitcode.zst"; \
        done <"$audit/logical.snapshot" | LC_ALL=C sort)
    then
        fail 'unanchored native manifest path set differs from frozen logical snapshot'
    fi
    sha256sum -c "$audit/native.SHA256SUMS" >/dev/null \
        || fail 'unanchored native manifest does not match artifact bytes'
    {
        printf 'RECOVERED_FIRST_PUBLICATION_UTC=%q\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
        printf 'NATIVE_MANIFEST_SHA256=%s\n' "$(sha256_file "$audit/native.SHA256SUMS")"
    } | write_atomic "$audit/COMPLETE.previous" \
        || fail 'cannot recover first-publication completion anchor'
fi
if [[ -f "$audit/native.SHA256SUMS" || -f "$audit/COMPLETE.previous" ]]; then
    [[ -f "$audit/native.SHA256SUMS" && -f "$audit/COMPLETE.previous" ]] \
        || fail 'completed audit lacks its manifest or retained completion anchor'
    mapfile -t prior_manifest_values < <(
        sed -n 's/^NATIVE_MANIFEST_SHA256=//p' "$audit/COMPLETE.previous"
    )
    [[ ${#prior_manifest_values[@]} == 1 \
        && "${prior_manifest_values[0]}" =~ ^[0-9a-f]{64}$ ]] \
        || fail 'prior completion marker has no unique native-manifest hash'
    [[ "$(sha256_file "$audit/native.SHA256SUMS")" == "${prior_manifest_values[0]}" ]] \
        || fail 'prior native manifest changed after completion'
    sha256sum -c "$audit/native.SHA256SUMS" >/dev/null \
        || fail 'native artifact changed after completion'
fi

work_dir=$(mktemp -d "${TMPDIR:-/tmp}/native-conversion-prepass.XXXXXX") \
    || fail 'cannot create work directory'
audit_tmp=
cleanup() {
    rm -rf -- "$work_dir"
    [[ -z "$audit_tmp" ]] || rm -rf -- "$audit_tmp"
}
trap cleanup EXIT

build_logical_snapshot() {
    local destination=$1 path logical marker base owner
    local physical="$work_dir/physical" owners="$work_dir/owners" markers="$work_dir/markers"
    : >"$physical"; : >"$owners"; : >"$markers"
    while IFS= read -r -d '' path; do
        [[ "$path" != *$'\n'* ]] || { printf 'error: newline in trace path: %q\n' "$path" >&2; return 1; }
        logical=${path%.parity.bitcode.zst}
        printf '%s\n' "$logical" >>"$physical"
    done < <(find "$corpus/traces" -type f \
        \( -name '*.jsonl.zst' -o -name '*.jsonl.zst.parity.bitcode.zst' \) -print0)
    LC_ALL=C sort -u "$physical" -o "$destination"
    [[ -s "$destination" ]] || { printf 'error: corpus has no logical traces: %s\n' "$corpus" >&2; return 1; }

    while IFS= read -r logical; do
        base=${logical##*/}; base=${base%.jsonl.zst}; owner=${base%%-session-*}
        marker="${logical%/*}/$owner.complete"
        [[ -f "$marker" ]] || { printf 'error: logical trace has no completion marker: %s\n' "$logical" >&2; return 1; }
        printf '%s\n' "$marker" >>"$owners"
    done <"$destination"
    duplicate_owner=$(LC_ALL=C sort "$owners" | uniq -d | head -n 1)
    [[ -z "$duplicate_owner" ]] || {
        printf 'error: completion marker owns more than one logical trace: %s\n' \
            "$duplicate_owner" >&2
        return 1
    }
    LC_ALL=C sort -u "$owners" -o "$owners"
    while IFS= read -r -d '' marker; do
        [[ "$marker" != *$'\n'* ]] \
            || { printf 'error: newline in completion-marker path: %q\n' "$marker" >&2; return 1; }
        printf '%s\n' "$marker" >>"$markers"
    done < <(find "$corpus/traces" -type f -name '*.complete' -print0)
    LC_ALL=C sort -u "$markers" -o "$markers"
    diff -u -- "$markers" "$owners" >&2 || {
        printf 'error: completion-marker/logical-trace ownership is not a bijection\n' >&2
        return 1
    }
}

logical_current="$work_dir/logical.snapshot"
build_logical_snapshot "$logical_current" || exit 2
expected_logical=unknown
if [[ -f "$corpus/campaign.env" ]]; then
    mapfile -t expected_values < <(
        sed -n 's/^EXPECTED_LOGICAL_REPLAYS=//p' "$corpus/campaign.env"
    )
    if (( ${#expected_values[@]} != 1 )) || [[ ! "${expected_values[0]}" =~ ^[1-9][0-9]*$ ]]; then
        fail "campaign.env must contain one positive EXPECTED_LOGICAL_REPLAYS: $corpus"
    fi
    expected_logical=${expected_values[0]}
    [[ "$(wc -l <"$logical_current")" == "$expected_logical" ]] \
        || fail "logical trace count does not match campaign expectation: $expected_logical"
fi

metadata="$work_dir/provenance.env"
{
    printf 'WORKSPACE=%q\n' "$workspace"
    printf 'CORPUS=%q\n' "$corpus"
    printf 'RUNNER=%q\n' "$runner"
    printf 'RUNNER_SHA256=%s\n' "$actual_runner_sha"
    printf 'RUNNER_TRUST_SHA256=%s\n' "$runner_trust_sha"
    printf 'RUNNER_IS_BUNDLE=%s\n' "$runner_is_bundle"
    printf 'TEST_ALLOW_DIRECT_RUNNER=%s\n' "$allow_direct_fixture"
    printf 'NATIVE_CONVERSION_PROTOCOL=%s\n' "$native_conversion_protocol"
    printf 'RUNNER_EXEC=%q\n' "$runner_exec"
    printf 'WRAPPER_SHA256=%s\n' "$wrapper_sha"
    printf 'LOADER=%q\n' "${loader_path:-none}"
    printf 'BUNDLE_MANIFEST_SHA256=%s\n' "$bundle_manifest_sha"
    printf 'BUNDLE_LIB_MANIFEST_SHA256=%s\n' "$bundle_lib_manifest_sha"
    printf 'JOBS=%s\n' "$jobs"
    printf 'TIMEOUT_SECONDS=%s\n' "$timeout_seconds"
    printf 'ALLOW_OTHER_CORPORA=%s\n' "$allow_other_corpora"
    printf 'GLOBAL_DRAIN_GUARD=observational-process-snapshot\n'
    printf 'TARGET_DRAIN_GUARD=locked-admission-and-collector\n'
    printf 'GLOBAL_OUTER_LOCK=%q\n' "$global_outer_lock"
    printf 'NICE=%s\n' "$nice_level"
    printf 'IONICE_CLASS=%s\n' "$ionice_class"
    printf 'IONICE_LEVEL=%s\n' "$ionice_level"
    printf 'EXPECTED_LOGICAL_REPLAYS=%s\n' "$expected_logical"
} >"$metadata"

if [[ ! -e "$audit" ]]; then
    audit_tmp=$(mktemp -d "${audit}.tmp.XXXXXX") || fail 'cannot create audit staging directory'
    mkdir -p "$audit_tmp/logs" "$audit_tmp/status" || fail 'cannot initialize audit staging directory'
    cp -- "$logical_current" "$audit_tmp/logical.snapshot"
    cp -- "$metadata" "$audit_tmp/provenance.env"
    : >"$audit_tmp/sources.snapshot"
    : >"$audit_tmp/key-map.tsv"
    while IFS= read -r logical; do
        relative=${logical#"$workspace"/}
        key=$(printf '%s' "$relative" | sha256sum); key=${key%% *}
        printf '%s\t%s\n' "$key" "$logical" >>"$audit_tmp/key-map.tsv"
        [[ ! -f "$logical" ]] || printf '%s\n' "$logical" >>"$audit_tmp/sources.snapshot"
    done <"$logical_current"
    if [[ "$(cut -f1 "$audit_tmp/key-map.tsv" | LC_ALL=C sort -u | wc -l)" \
        != "$(wc -l <"$audit_tmp/key-map.tsv")" ]]
    then
        rm -rf -- "$audit_tmp"
        fail 'SHA-256 key collision in logical snapshot'
    fi
    (cd "$audit_tmp" && sha256sum logical.snapshot sources.snapshot \
        key-map.tsv provenance.env) >"$audit_tmp/INPUT_SHA256SUMS"
    printf 'CREATED_UTC=%q\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >"$audit_tmp/created.env"
    {
        printf 'AVAILABLE_DISK_KIB=%s\n' "$available_kib"
        printf 'AVAILABLE_MEMORY_KIB=%s\n' "$memory_available_kib"
        printf 'LOGICAL_COUNT=%s\n' "$(wc -l <"$audit_tmp/logical.snapshot")"
        printf 'SOURCE_COUNT=%s\n' "$(wc -l <"$audit_tmp/sources.snapshot")"
    } >"$audit_tmp/preflight.env"
    mv -- "$audit_tmp" "$audit" || fail "cannot publish audit: $audit"
    audit_tmp=
else
    [[ -d "$audit" && -f "$audit/logical.snapshot" && -f "$audit/sources.snapshot" \
        && -f "$audit/key-map.tsv" && -f "$audit/provenance.env" \
        && -f "$audit/INPUT_SHA256SUMS" ]] \
        || fail "existing audit is incomplete: $audit"
    cmp -s "$logical_current" "$audit/logical.snapshot" \
        || fail "logical corpus drift from frozen audit: $audit"
    cmp -s "$metadata" "$audit/provenance.env" \
        || fail "invocation provenance drift from frozen audit: $audit"
    (cd "$audit" && sha256sum -c INPUT_SHA256SUMS) >/dev/null \
        || fail "frozen audit inputs fail checksums: $audit"
fi

run_one() {
    local logical=$1 relative key status_file native quarantine attempt=1 prior_status prior_attempt prior_log
    local log_final log_in_progress rc=0
    local -a converter_command
    relative=${logical#"$workspace"/}
    key=$(printf '%s' "$relative" | sha256sum); key=${key%% *}
    status_file="$audit/status/$key.status"
    native="$logical.parity.bitcode.zst"
    quarantine="$logical.parity-conversion-source"
    if [[ -f "$status_file" ]]; then
        mapfile -t status_lines <"$status_file" || return 1
        [[ ${#status_lines[@]} == 1 ]] || return 1
        IFS=$'\t' read -r prior_status prior_attempt prior_log <<<"${status_lines[0]}" \
            || return 1
        [[ -n "$prior_status" && "$prior_attempt" =~ ^[0-9]+$ && -n "$prior_log" ]] \
            || return 1
        if [[ "$prior_status" == 0 ]]; then
            [[ ! -e "$logical" && ! -e "$quarantine" && -f "$native" \
                && -f "$audit/logs/$prior_log" ]] || {
                printf 'error: status-zero resume invariant failed: %s\n' "$logical" >&2
                return 1
            }
            return 0
        fi
        # A `running` status proves only that the prior invocation was
        # interrupted. Protocol 2 owns deterministic recovery of every
        # canonical/quarantine/native transaction state, so never infer a
        # commit from pathname or log observations here: reserve a new attempt
        # and invoke the authenticated converter again under its trace lock.
        attempt=$((prior_attempt + 1))
    fi
    while true; do
        printf -v attempt_label '%04d' "$attempt"
        log_name="$key.attempt-$attempt_label.log"
        log_final="$audit/logs/$log_name"
        log_in_progress="$log_final.in-progress"
        [[ -e "$log_final" || -e "$log_in_progress" ]] || break
        attempt=$((attempt + 1))
    done
    [[ ! -e "$log_final" && ! -e "$log_in_progress" ]] || return 1
    (set -o noclobber; : >"$log_in_progress") 2>/dev/null || return 1
    printf 'running\t%s\t%s\n' "$attempt" "$log_name" \
        | write_atomic "$status_file" || return 1
    if [[ -n "$loader_path" ]]; then
        converter_command=(env -u LD_LIBRARY_PATH "$loader_path" --library-path "$bundle_lib_dir" \
            "$runner_exec" --convert "$logical")
    else
        converter_command=(env -u LD_LIBRARY_PATH "$runner_exec" --convert "$logical")
    fi
    if [[ "$ionice_class" == 2 ]]; then
        timeout --signal=TERM --kill-after=30s "${timeout_seconds}s" \
            nice -n "$nice_level" ionice -c "$ionice_class" -n "$ionice_level" \
            "${converter_command[@]}" >"$log_in_progress" 2>&1 || rc=$?
    else
        timeout --signal=TERM --kill-after=30s "${timeout_seconds}s" \
            nice -n "$nice_level" ionice -c "$ionice_class" \
            "${converter_command[@]}" >"$log_in_progress" 2>&1 || rc=$?
    fi
    if (( rc == 0 )) \
        && { [[ -e "$logical" ]] || [[ -e "$quarantine" ]] || [[ ! -f "$native" ]]; }
    then
        rc=65
        printf 'postcondition failure: source/quarantine remains or native is absent\n' \
            >>"$log_in_progress"
    fi
    mv -- "$log_in_progress" "$log_final" || return 1
    printf '%s\t%s\t%s\n' "$rc" "$attempt" "$log_name" \
        | write_atomic "$status_file" || return 1
    (( rc == 0 ))
}
export -f run_one sha256_file
export workspace audit runner_exec timeout_seconds nice_level ionice_class ionice_level
export loader_path bundle_lib_dir

active=0
worker_failure=0
while IFS= read -r logical; do
    run_one "$logical" &
    active=$((active + 1))
    if (( active >= jobs )); then
        wait -n || worker_failure=1
        active=$((active - 1))
    fi
done <"$audit/sources.snapshot"
while (( active > 0 )); do
    wait -n || worker_failure=1
    active=$((active - 1))
done

failures_tmp=$(mktemp "$audit/failures.tsv.tmp.XXXXXX") || fail 'cannot stage failure ledger'
printf 'status\tattempt\tsource_exists\tnative_exists\ttrace\tlog\n' >"$failures_tmp"
while IFS= read -r logical; do
    relative=${logical#"$workspace"/}
    key=$(printf '%s' "$relative" | sha256sum); key=${key%% *}
    status_file="$audit/status/$key.status"
    if [[ -f "$status_file" ]]; then
        read -r rc attempt log_name <"$status_file" || { rc=malformed; attempt=; log_name=; }
    else
        rc=missing; attempt=; log_name=
    fi
    source_exists=0; native_exists=0
    [[ ! -e "$logical" ]] || source_exists=1
    [[ ! -f "$logical.parity.bitcode.zst" ]] || native_exists=1
    if [[ "$rc" != 0 || "$source_exists" != 0 || "$native_exists" != 1 ]]; then
        printf '%s\t%s\t%s\t%s\t%s\t%s\n' \
            "$rc" "$attempt" "$source_exists" "$native_exists" "$logical" "$log_name" \
            >>"$failures_tmp"
    fi
done <"$audit/sources.snapshot"
mv -f -- "$failures_tmp" "$audit/failures.tsv" || fail 'cannot publish failure ledger'

if (( worker_failure != 0 )) || [[ "$(wc -l <"$audit/failures.tsv")" != 1 ]]; then
    printf 'conversion prepass incomplete; see %s\n' "$audit/failures.tsv" >&2
    exit 1
fi

[[ "$(sha256_file "$runner")" == "$actual_runner_sha" ]] \
    || fail 'runner changed during conversion'
if [[ "$wrapper_sha" != none ]]; then
    [[ "$(sha256_file "$wrapper_path")" == "$wrapper_sha" ]] \
        || fail 'runner wrapper changed during conversion'
    (cd "$runner_dir" && sha256sum -c SHA256SUMS && sha256sum -c LIB_SHA256SUMS) \
        >/dev/null || fail 'packaged runner inputs changed during conversion'
    final_bundle_manifest_sha=$(sha256_file "$runner_dir/SHA256SUMS") || exit 2
    final_bundle_lib_manifest_sha=$(sha256_file "$runner_dir/LIB_SHA256SUMS") || exit 2
    final_runner_trust_sha=$(printf \
        'schema16-runner-bundle-v1\nSHA256SUMS=%s\nLIB_SHA256SUMS=%s\n' \
        "$final_bundle_manifest_sha" "$final_bundle_lib_manifest_sha" | sha256sum)
    final_runner_trust_sha=${final_runner_trust_sha%% *}
    [[ "$final_runner_trust_sha" == "$runner_trust_sha" ]] \
        || fail 'runner bundle trust digest changed during conversion'
fi

build_logical_snapshot "$logical_current" || exit 2
cmp -s "$logical_current" "$audit/logical.snapshot" \
    || fail 'logical corpus drifted during conversion'
native_manifest_tmp=$(mktemp "$audit/native.SHA256SUMS.tmp.XXXXXX") \
    || fail 'cannot stage native manifest'
: >"$native_manifest_tmp"
while IFS= read -r logical; do
    [[ ! -e "$logical" && ! -e "$logical.parity-conversion-source" \
        && -f "$logical.parity.bitcode.zst" ]] \
        || { rm -f -- "$native_manifest_tmp"; fail "final all-native invariant failed: $logical"; }
    digest=$(sha256_file "$logical.parity.bitcode.zst") \
        || { rm -f -- "$native_manifest_tmp"; fail "cannot hash native trace: $logical"; }
    printf '%s  %s\n' "$digest" "$logical.parity.bitcode.zst" >>"$native_manifest_tmp"
done <"$audit/logical.snapshot"
if [[ -f "$audit/native.SHA256SUMS" ]]; then
    cmp -s -- "$native_manifest_tmp" "$audit/native.SHA256SUMS" \
        || { rm -f -- "$native_manifest_tmp"; fail 'native manifest differs from prior completed audit'; }
    rm -f -- "$native_manifest_tmp"
else
    mv -- "$native_manifest_tmp" "$audit/native.SHA256SUMS" \
        || fail 'cannot publish native manifest'
fi
{
    printf 'COMPLETED_UTC=%q\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    printf 'LOGICAL_COUNT=%s\n' "$(wc -l <"$audit/logical.snapshot")"
    printf 'SOURCE_COUNT=%s\n' "$(wc -l <"$audit/sources.snapshot")"
    printf 'NATIVE_MANIFEST_SHA256=%s\n' "$(sha256_file "$audit/native.SHA256SUMS")"
} | write_atomic "$audit/COMPLETE" || fail 'cannot publish completion marker'

printf 'native conversion prepass complete: %s\n' "$audit"
