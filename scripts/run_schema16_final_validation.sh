#!/usr/bin/env bash
set -euo pipefail

# Validate completed schema-16 campaigns in the order supplied. This is kept
# separate from the live capture ladder: it freezes each completed corpus and
# proves every trace with one immutable parity-runner build before continuing
# to the next campaign.

if (( $# < 4 )); then
    printf 'usage: %s WORKSPACE RUNNER RUNNER_SHA256 CAMPAIGN [CAMPAIGN ...]\n' "$0" >&2
    exit 2
fi

workspace=$1
runner=$2
expected_runner_sha=$3
shift 3
campaigns=("$@")
runner_mode=${SCHEMA16_FINAL_RUNNER_MODE:-}
expected_bundle_sha=${SCHEMA16_FINAL_RUNNER_BUNDLE_SHA256:-}
detected_cpus=$(nproc 2>/dev/null || printf '1\n')
if [[ ! "$detected_cpus" =~ ^[1-9][0-9]*$ ]]; then
    detected_cpus=1
fi
default_sweep_concurrency=$detected_cpus
(( default_sweep_concurrency > 16 )) && default_sweep_concurrency=16
sweep_concurrency=${SCHEMA16_FINAL_SWEEP_CONCURRENCY:-$default_sweep_concurrency}
[[ "$sweep_concurrency" =~ ^[1-9][0-9]*$ ]] \
    || { printf 'error: SCHEMA16_FINAL_SWEEP_CONCURRENCY must be a positive integer\n' >&2; exit 2; }
(( sweep_concurrency <= 64 )) \
    || { printf 'error: SCHEMA16_FINAL_SWEEP_CONCURRENCY must not exceed 64\n' >&2; exit 2; }

audit_parent="$workspace/parity-save-replays/audits"
outer_lock=${SCHEMA16_FINAL_OUTER_LOCK:-/tmp/robin-parity-runner.lock}
exact_eof_marker='parity trace matched every recorded frame'
unset LD_LIBRARY_PATH

fail() {
    printf 'error: %s\n' "$*" >&2
    exit 2
}

sha256_file() {
    local value
    value=$(sha256sum -- "$1") || return 1
    printf '%s\n' "${value%% *}"
}

verify_runner_bundle() {
    local bundle=$1 loader_proof_root=${2:-$1} manifest line path
    [[ -x "$bundle/original_parity_replay" \
        && -x "$bundle/original_parity_replay.remote" \
        && -x "$bundle/lib/ld-linux-x86-64.so.2" \
        && -f "$bundle/SHA256SUMS" \
        && -f "$bundle/LIB_SHA256SUMS" \
        && -f "$bundle/PROVENANCE.txt" \
        && -f "$bundle/LOADER_LIST.txt" ]] \
        || { printf 'error: incomplete parity runner bundle: %s\n' "$bundle" >&2; return 1; }
    mapfile -t protocol_values < <(
        sed -n 's/^NATIVE_CONVERSION_PROTOCOL=//p' "$bundle/PROVENANCE.txt"
    )
    [[ ${#protocol_values[@]} == 1 && "${protocol_values[0]}" == 2 ]] \
        || { printf 'error: bundle must authenticate NATIVE_CONVERSION_PROTOCOL=2: %s\n' \
            "$bundle" >&2; return 1; }
    if find "$bundle" -type l -print -quit | grep -q .; then
        printf 'error: runner bundle contains a symlink: %s\n' "$bundle" >&2
        return 1
    fi
    for manifest in "$bundle/SHA256SUMS" "$bundle/LIB_SHA256SUMS"; do
        while IFS= read -r line; do
            [[ "$line" =~ ^[0-9a-fA-F]{64}[[:space:]][\ \*](.+)$ ]] \
                || { printf 'error: malformed bundle checksum entry: %s\n' "$manifest" >&2; return 1; }
            path=${BASH_REMATCH[1]}
            [[ "$path" != /* && "$path" != ../* && "$path" != */../* \
                && "$path" != *'/..' && "$path" != *$'\n'* ]] \
                || { printf 'error: unsafe bundle checksum path: %s\n' "$path" >&2; return 1; }
        done <"$manifest"
    done
    if ! diff -u -- \
        <(find "$bundle/lib" -type f -printf 'lib/%P\n' | LC_ALL=C sort) \
        <(sed -n 's/^[0-9a-fA-F]\{64\} [ *]//p' "$bundle/LIB_SHA256SUMS" \
            | LC_ALL=C sort) >&2
    then
        printf 'error: library manifest does not exactly cover bundle lib tree: %s\n' \
            "$bundle" >&2
        return 1
    fi
    if ! diff -u -- \
        <(find "$bundle" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' \
            | LC_ALL=C sort) \
        <(printf 'lib\n') >&2
    then
        printf 'error: runner bundle has an unexpected root directory: %s\n' \
            "$bundle" >&2
        return 1
    fi
    if ! diff -u -- \
        <(printf '%s\n' LIB_SHA256SUMS LOADER_LIST.txt PROVENANCE.txt \
            original_parity_replay original_parity_replay.remote | LC_ALL=C sort) \
        <(sed -n 's/^[0-9a-fA-F]\{64\} [ *]//p' "$bundle/SHA256SUMS" \
            | LC_ALL=C sort) >&2
    then
        printf 'error: main manifest does not exactly cover bundle root files: %s\n' \
            "$bundle" >&2
        return 1
    fi
    if ! diff -u -- \
        <(printf '%s\n' LIB_SHA256SUMS LOADER_LIST.txt PROVENANCE.txt SHA256SUMS \
            original_parity_replay original_parity_replay.remote | LC_ALL=C sort) \
        <(find "$bundle" -maxdepth 1 -type f -printf '%f\n' | LC_ALL=C sort) >&2
    then
        printf 'error: runner bundle root file set is not canonical: %s\n' \
            "$bundle" >&2
        return 1
    fi
    grep -Fq -- "=> $loader_proof_root/lib/ld-linux-x86-64.so.2 " \
        "$bundle/LOADER_LIST.txt" \
        || { printf 'error: loader proof is not from authenticated final bundle path: %s\n' \
            "$loader_proof_root" >&2; return 1; }
    if ! awk -v prefix="$loader_proof_root/lib/" '
        /=>/ {
            resolved=$0
            sub(/^.*=>[[:space:]]*/, "", resolved)
            sub(/[[:space:]].*$/, "", resolved)
            if (index(resolved, prefix) != 1) exit 1
        }
    ' "$bundle/LOADER_LIST.txt"; then
        printf 'error: loader proof resolves outside authenticated lib tree: %s\n' \
            "$loader_proof_root" >&2
        return 1
    fi
    grep -Eq '^[0-9a-fA-F]{64} [ *]original_parity_replay$' "$bundle/SHA256SUMS" \
        && grep -Eq '^[0-9a-fA-F]{64} [ *]original_parity_replay\.remote$' "$bundle/SHA256SUMS" \
        && grep -Eq '^[0-9a-fA-F]{64} [ *]LIB_SHA256SUMS$' "$bundle/SHA256SUMS" \
        && grep -Eq '^[0-9a-fA-F]{64} [ *]PROVENANCE\.txt$' "$bundle/SHA256SUMS" \
        && grep -Eq '^[0-9a-fA-F]{64} [ *]LOADER_LIST\.txt$' "$bundle/SHA256SUMS" \
        && grep -Eq '^[0-9a-fA-F]{64} [ *]lib/ld-linux-x86-64\.so\.2$' "$bundle/LIB_SHA256SUMS" \
        || { printf 'error: bundle manifests omit required runtime inputs: %s\n' "$bundle" >&2; return 1; }
    (cd -- "$bundle" \
        && sha256sum --strict -c SHA256SUMS \
        && sha256sum --strict -c LIB_SHA256SUMS) >/dev/null \
        || { printf 'error: parity runner bundle checksum failure: %s\n' "$bundle" >&2; return 1; }
}

runner_bundle_digest() {
    local bundle=$1 main_sha lib_sha value
    main_sha=$(sha256_file "$bundle/SHA256SUMS") || return 1
    lib_sha=$(sha256_file "$bundle/LIB_SHA256SUMS") || return 1
    value=$(printf 'schema16-runner-bundle-v1\nSHA256SUMS=%s\nLIB_SHA256SUMS=%s\n' \
        "$main_sha" "$lib_sha" | sha256sum) || return 1
    printf '%s\n' "${value%% *}"
}

# Bind the stable logical `.jsonl.zst` identity to its normalized native bytes.
# Any legacy source here means normalization raced with another writer or did
# not complete, so never freeze that unstable representation.
trace_identity_digest() {
    local logical=$1 native="$1.parity.bitcode.zst"
    if [[ -e "$logical" ]]; then
        printf 'error: legacy trace remains after native normalization: %s\n' \
            "$logical" >&2
        return 1
    elif [[ -f "$native" ]]; then
        sha256_file "$native"
    else
        printf 'error: logical trace has no normalized native artifact: %s\n' \
            "$logical" >&2
        return 1
    fi
}

normalize_manifest_to_native() {
    local manifest=$1 audit=$2 trace native conversion_log conversion_status status
    conversion_log="$audit.conversion.log"
    conversion_status="$audit.conversion.status"
    : >"$conversion_log"
    while IFS= read -r trace; do
        native="$trace.parity.bitcode.zst"
        if [[ -f "$trace" ]]; then
            status=0
            "$pinned_runner_exec" --convert "$trace" >>"$conversion_log" 2>&1 \
                || status=$?
            printf '%s\n' "$status" >"$conversion_status"
            if (( status != 0 )); then
                printf 'error: native trace conversion failed with status %s: %s (see %s)\n' \
                    "$status" "$trace" "$conversion_log" >&2
                return 1
            fi
            [[ ! -e "$trace" && -f "$native" ]] || {
                printf 'error: converter did not replace legacy trace with native artifact: %s\n' \
                    "$trace" >&2
                return 1
            }
        elif [[ ! -f "$native" ]]; then
            printf 'error: logical trace has no legacy or native artifact: %s\n' \
                "$trace" >&2
            return 1
        fi
    done <"$manifest"
}

read_campaign_uint() {
    local campaign=$1 key=$2
    local -a values=()
    mapfile -t values < <(sed -n "s/^${key}=//p" "$campaign/campaign.env")
    if (( ${#values[@]} != 1 )) || [[ ! "${values[0]}" =~ ^[0-9]+$ ]]; then
        printf 'error: %s must contain exactly one unsigned %s\n' \
            "$campaign/campaign.env" "$key" >&2
        return 1
    fi
    printf '%s\n' "${values[0]}"
}

path_has_newline() {
    [[ "$1" == *$'\n'* ]]
}

build_complete_manifest() {
    local campaign=$1 expected=$2 destination=$3
    local marker stem trace
    local -a markers=() traces=() matches=() complete_traces=()

    mapfile -d '' -t markers < <(
        find "$campaign/traces" -type f -name '*.complete' -print0 | LC_ALL=C sort -z
    )
    # Logical .jsonl.zst identities: a converted recording exists only as
    # <identity>.parity.bitcode.zst, so match both and strip the suffix.
    mapfile -d '' -t traces < <(
        find "$campaign/traces" -type f \( -name '*.jsonl.zst' \
            -o -name '*.jsonl.zst.parity.bitcode.zst' \) -print0 \
            | sed -z 's/\.parity\.bitcode\.zst$//' | LC_ALL=C sort -zu
    )

    if (( ${#markers[@]} != expected || ${#traces[@]} != expected )); then
        printf 'error: %s has markers=%s zst=%s expected=%s\n' \
            "$campaign" "${#markers[@]}" "${#traces[@]}" "$expected" >&2
        return 1
    fi

    for marker in "${markers[@]}"; do
        if path_has_newline "$marker"; then
            printf 'error: newline in completion-marker path: %q\n' "$marker" >&2
            return 1
        fi
        stem=${marker%.complete}
        matches=()
        while IFS= read -r -d '' trace; do
            matches+=("$trace")
        done < <(find "${stem%/*}" -maxdepth 1 -type f \
            \( -name "${stem##*/}-session-*.jsonl.zst" \
            -o -name "${stem##*/}-session-*.jsonl.zst.parity.bitcode.zst" \) \
            -print0 | sed -z 's/\.parity\.bitcode\.zst$//' | LC_ALL=C sort -zu)
        if (( ${#matches[@]} != 1 )); then
            printf 'error: completion marker must own exactly one zst trace: %s (found %s)\n' \
                "$marker" "${#matches[@]}" >&2
            return 1
        fi
        complete_traces+=("${matches[0]}")
    done

    for trace in "${traces[@]}"; do
        if path_has_newline "$trace"; then
            printf 'error: newline in trace path: %q\n' "$trace" >&2
            return 1
        fi
    done
    printf '%s\n' "${complete_traces[@]}" | LC_ALL=C sort -u >"$destination"
    if [[ "$(wc -l <"$destination")" != "$expected" ]]; then
        printf 'error: canonical complete manifest is not unique for %s\n' "$campaign" >&2
        return 1
    fi
    if ! diff -u -- <(printf '%s\n' "${traces[@]}") "$destination" >&2; then
        printf 'error: sorted zst inventory differs from complete manifest for %s\n' \
            "$campaign" >&2
        return 1
    fi
}

build_trace_identities() {
    local manifest=$1 destination=$2 trace digest
    local unsorted="${destination}.unsorted"

    : >"$unsorted"
    while IFS= read -r trace; do
        digest=$(trace_identity_digest "$trace") \
            || { rm -f -- "$unsorted"; return 1; }
        printf '%s  %s\n' "$digest" "$trace" >>"$unsorted" \
            || { rm -f -- "$unsorted"; return 1; }
    done <"$manifest"
    if ! LC_ALL=C sort -- "$unsorted" >"$destination"; then
        rm -f -- "$unsorted" "$destination"
        return 1
    fi
    rm -f -- "$unsorted"
}

require_campaign_drained() {
    local campaign=$1
    if [[ -d "$campaign/.capture-reservations" ]] \
        && find "$campaign/.capture-reservations" -type f -name '*.reserve' \
            -print -quit | grep -q .
    then
        printf 'error: campaign still has an active capture reservation: %s\n' \
            "$campaign" >&2
        return 1
    fi
}

trace_key() {
    local trace=$1 relative
    relative=${trace#"$workspace"/}
    [[ "$relative" != "$trace" ]] || return 1
    printf '%s\n' "${relative//\//__}"
}

initialize_or_verify_audit() {
    local audit=$1 campaign=$2 manifest=$3 identities=$4 metadata_tmp=$5
    local snapshot_sha=$6 identities_sha=$7 temporary

    if [[ ! -e "$audit" ]]; then
        temporary=$(mktemp -d "${audit}.tmp.XXXXXX") || return 1
        if ! mkdir -p "$temporary/logs" "$temporary/status" "$temporary/.trace-locks" \
            || ! cp -- "$manifest" "$temporary/traces.snapshot" \
            || ! cp -- "$identities" "$temporary/traces.sha256" \
            || ! cp -- "$metadata_tmp" "$temporary/validation.env"
        then
            rm -rf -- "$temporary"
            return 1
        fi
        if ! mv -- "$temporary" "$audit"; then
            rm -rf -- "$temporary"
            return 1
        fi
        return 0
    fi

    [[ -d "$audit" && -f "$audit/traces.snapshot" \
        && -f "$audit/traces.sha256" && -f "$audit/validation.env" ]] \
        || { printf 'error: incomplete existing audit: %s\n' "$audit" >&2; return 1; }
    cmp -s -- "$manifest" "$audit/traces.snapshot" \
        || { printf 'error: frozen manifest drift for %s\n' "$campaign" >&2; return 1; }
    cmp -s -- "$identities" "$audit/traces.sha256" \
        || { printf 'error: frozen trace identity drift for %s\n' "$campaign" >&2; return 1; }
    cmp -s -- "$metadata_tmp" "$audit/validation.env" \
        || { printf 'error: frozen audit metadata drift for %s\n' "$campaign" >&2; return 1; }
    [[ "$(sha256_file "$audit/traces.snapshot")" == "$snapshot_sha" ]] \
        || { printf 'error: frozen snapshot hash mismatch for %s\n' "$campaign" >&2; return 1; }
    [[ "$(sha256_file "$audit/traces.sha256")" == "$identities_sha" ]] \
        || { printf 'error: frozen trace-identity hash mismatch for %s\n' "$campaign" >&2; return 1; }
}

classify_failure() {
    local audit=$1 fallback=${2:-audit-proof-set-mismatch}
    local trace key status log value marker_count classification=missing-status
    local temporary="$audit/parity-last-failure.env.tmp"

    trace=
    value=
    while IFS= read -r trace; do
        key=$(trace_key "$trace") || continue
        status="$audit/status/$key.status"
        log="$audit/logs/$key.log"
        if [[ ! -f "$status" ]]; then
            classification=missing-status
            break
        fi
        if ! cmp -s -- "$status" <(printf '0\n'); then
            value=$(<"$status") || value=unreadable
            classification=nonzero-or-malformed-status
            break
        fi
        if [[ ! -f "$log" ]]; then
            classification=missing-log
            break
        fi
        marker_count=$(grep -Fxc -- "$exact_eof_marker" "$log" || true)
        if [[ "$marker_count" != 1 ]]; then
            classification="eof-marker-count-$marker_count"
            break
        fi
        trace=
    done <"$audit/traces.snapshot"
    if [[ -z "$trace" ]]; then
        classification=$fallback
    fi

    {
        printf 'FAILED_UTC=%q\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
        printf 'CLASSIFICATION=%q\n' "$classification"
        printf 'STATUS=%q\n' "$value"
        printf 'TRACE=%q\n' "$trace"
    } >"$temporary"
    mv -f -- "$temporary" "$audit/parity-last-failure.env"
    printf 'error: final validation stopped: %s (%s)\n' "$trace" "$classification" >&2
}

verify_exact_audit() {
    local audit=$1 expected=$2 work_dir=$3 trace key status log marker_count
    local expected_statuses="$work_dir/expected-statuses"
    local expected_logs="$work_dir/expected-logs"
    local actual_statuses="$work_dir/actual-statuses"
    local actual_logs="$work_dir/actual-logs"

    : >"$expected_statuses"
    : >"$expected_logs"
    while IFS= read -r trace; do
        key=$(trace_key "$trace") || return 1
        printf '%s/status/%s.status\n' "$audit" "$key" >>"$expected_statuses"
        printf '%s/logs/%s.log\n' "$audit" "$key" >>"$expected_logs"
    done <"$audit/traces.snapshot"
    LC_ALL=C sort -o "$expected_statuses" "$expected_statuses"
    LC_ALL=C sort -o "$expected_logs" "$expected_logs"
    find "$audit/status" -maxdepth 1 -type f -print | LC_ALL=C sort >"$actual_statuses"
    find "$audit/logs" -maxdepth 1 -type f -print | LC_ALL=C sort >"$actual_logs"
    cmp -s -- "$expected_statuses" "$actual_statuses" \
        || { printf 'error: status set differs from frozen manifest: %s\n' "$audit" >&2; return 1; }
    cmp -s -- "$expected_logs" "$actual_logs" \
        || { printf 'error: log set differs from frozen manifest: %s\n' "$audit" >&2; return 1; }
    [[ "$(wc -l <"$expected_statuses")" == "$expected" ]] || return 1

    while IFS= read -r trace; do
        key=$(trace_key "$trace") || return 1
        status="$audit/status/$key.status"
        log="$audit/logs/$key.log"
        cmp -s -- "$status" <(printf '0\n') \
            || { printf 'error: non-exact status proof: %s\n' "$status" >&2; return 1; }
        marker_count=$(grep -Fxc -- "$exact_eof_marker" "$log" || true)
        [[ "$marker_count" == 1 ]] \
            || { printf 'error: expected one EOF marker in %s, found %s\n' \
                "$log" "$marker_count" >&2; return 1; }
    done <"$audit/traces.snapshot"
}

verify_existing_audit_proofs() {
    local audit=$1 trace key status log marker_count actual
    local -A allowed_statuses=() allowed_logs=()
    while IFS= read -r trace; do
        key=$(trace_key "$trace") || return 1
        status="$audit/status/$key.status"
        log="$audit/logs/$key.log"
        allowed_statuses["$status"]=1
        allowed_logs["$log"]=1
        [[ -e "$status" || -e "$log" ]] || continue
        [[ -f "$status" && -f "$log" ]] || return 1
        cmp -s -- "$status" <(printf '0\n') || return 1
        marker_count=$(grep -Fxc -- "$exact_eof_marker" "$log" || true)
        [[ "$marker_count" == 1 ]] || return 1
    done <"$audit/traces.snapshot"
    while IFS= read -r -d '' actual; do
        [[ -n "${allowed_statuses[$actual]+present}" ]] || return 1
    done < <(find "$audit/status" -maxdepth 1 -type f -print0)
    while IFS= read -r -d '' actual; do
        [[ -n "${allowed_logs[$actual]+present}" ]] || return 1
    done < <(find "$audit/logs" -maxdepth 1 -type f -print0)
}

[[ -d "$workspace" ]] || fail "workspace is not a directory: $workspace"
workspace=$(realpath -e -- "$workspace")
runner_arg_is_dir=0
if [[ -d "$runner" ]]; then
    runner_arg_is_dir=1
    runner=$(realpath -e -- "$runner")/original_parity_replay
fi
[[ -x "$runner" ]] || fail "runner is not executable: $runner"
runner=$(realpath -e -- "$runner")
[[ "$expected_runner_sha" =~ ^[0-9a-fA-F]{64}$ ]] \
    || fail 'RUNNER_SHA256 must contain 64 hexadecimal digits'
expected_runner_sha=${expected_runner_sha,,}
actual_runner_sha=$(sha256_file "$runner") || fail "cannot hash runner: $runner"
[[ "$actual_runner_sha" == "$expected_runner_sha" ]] \
    || fail "runner hash mismatch: expected $expected_runner_sha, got $actual_runner_sha"
runner_source_dir=${runner%/*}
runner_is_bundle=0
runner_wrapper_sha=none
runner_bundle_manifest_sha=none
runner_lib_manifest_sha=none
native_conversion_protocol=fixture-direct
case "$runner_mode" in
bundle)
    [[ "$expected_bundle_sha" =~ ^[0-9a-fA-F]{64}$ ]] \
        || fail 'bundle mode requires SCHEMA16_FINAL_RUNNER_BUNDLE_SHA256'
    expected_bundle_sha=${expected_bundle_sha,,}
    runner_is_bundle=1
    runner_identity=$expected_bundle_sha
    ;;
direct)
    [[ -z "$expected_bundle_sha" ]] \
        || fail 'direct mode does not accept SCHEMA16_FINAL_RUNNER_BUNDLE_SHA256'
    (( runner_arg_is_dir == 0 )) || fail 'direct runner must be an executable file'
    [[ ! -e "$runner_source_dir/original_parity_replay.remote" \
        && ! -e "$runner_source_dir/SHA256SUMS" \
        && ! -e "$runner_source_dir/LIB_SHA256SUMS" \
        && ! -e "$runner_source_dir/lib" ]] \
        || fail 'direct mode refuses a packaged runner; select bundle mode'
    runner_identity=$actual_runner_sha
    ;;
*) fail 'SCHEMA16_FINAL_RUNNER_MODE must be exactly direct or bundle' ;;
esac
if (( runner_is_bundle == 0 )) && file -b -- "$runner" | grep -q '^ELF '; then
    command -v readelf >/dev/null || fail 'readelf is required to verify the ELF loader'
    direct_loader=$(readelf -l -- "$runner" \
        | sed -n 's/.*Requesting program interpreter: \([^]]*\)].*/\1/p')
    [[ -n "$direct_loader" && -x "$direct_loader" ]] \
        || fail "ELF interpreter is missing or not executable: ${direct_loader:-unknown}"
    if ldd -- "$runner" | grep -Fq 'not found'; then
        fail "direct runner has an unresolved shared library: $runner"
    fi
fi
[[ -x "$workspace/scripts/run_parity_release_sweep.sh" ]] \
    || fail "missing release sweep under workspace: $workspace"
command -v setsid >/dev/null || fail 'setsid is required for parallel worker cleanup'

mkdir -p -- "$audit_parent" "$(dirname -- "$outer_lock")"
exec {outer_lock_fd}>"$outer_lock"
flock "$outer_lock_fd"

# Resolve every target before inspecting the pinned runner or any prior audit
# metadata. A current verdict is a commit marker, so move all such markers out
# of the way first; any later integrity failure must leave no current 100%.
declare -a normalized_campaigns=() campaign_audits=()
for campaign_arg in "${campaigns[@]}"; do
    [[ -d "$campaign_arg/traces" && -f "$campaign_arg/campaign.env" ]] \
        || fail "campaign is incomplete: $campaign_arg"
    campaign=$(realpath -e -- "$campaign_arg")
    [[ "$campaign" == "$workspace"/* ]] \
        || fail "campaign is outside workspace: $campaign"
    campaign_relative=${campaign#"$workspace"/}
    path_digest=$(printf '%s' "$campaign_relative" | sha256sum)
    path_digest=${path_digest%% *}
    campaign_label=$(printf '%s' "${campaign##*/}" | tr -c 'A-Za-z0-9._-' '_')
    campaign_label=${campaign_label:0:48}
    normalized_campaigns+=("$campaign")
    campaign_audits+=(
        "$audit_parent/schema16-final-${campaign_label}-path-$path_digest-runner-$runner_identity"
    )
done
for audit in "${campaign_audits[@]}"; do
    mkdir -p -- "${audit%/*}"
    if [[ -f "$audit/parity-verdict.env" ]]; then
        mv -f -- "$audit/parity-verdict.env" "$audit/parity-verdict.previous.env"
    fi
done

if (( runner_is_bundle == 1 )); then
    verify_runner_bundle "$runner_source_dir" || exit 2
    native_conversion_protocol=2
    runner_wrapper_sha=$(sha256_file "$runner_source_dir/original_parity_replay.remote") \
        || exit 2
    runner_bundle_manifest_sha=$(sha256_file "$runner_source_dir/SHA256SUMS") \
        || exit 2
    runner_lib_manifest_sha=$(sha256_file "$runner_source_dir/LIB_SHA256SUMS") \
        || exit 2
    [[ "$(runner_bundle_digest "$runner_source_dir")" == "$expected_bundle_sha" ]] \
        || fail "runner bundle trust digest mismatch: $runner_source_dir"
fi

runner_dir="$workspace/.git/schema16-final-runners/$runner_identity"
pinned_runner="$runner_dir/original_parity_replay"
pinned_runner_exec="$pinned_runner"
mkdir -p -- "$workspace/.git/schema16-final-runners"
if [[ ! -e "$runner_dir" ]]; then
    runner_tmp=$(mktemp -d \
        "$workspace/.git/schema16-final-runners/.runner-$actual_runner_sha.tmp.XXXXXX")
    if (( runner_is_bundle == 1 )); then
        cp -a --no-preserve=ownership -- "$runner_source_dir/." "$runner_tmp/"
        verify_runner_bundle "$runner_tmp" "$runner_source_dir" || exit 2
    else
        cp -p -- "$runner" "$runner_tmp/original_parity_replay"
        chmod 0555 "$runner_tmp/original_parity_replay"
        printf '%s  original_parity_replay\n' "$actual_runner_sha" >"$runner_tmp/runner.sha256"
    fi
    mv -- "$runner_tmp" "$runner_dir"
fi
[[ -x "$pinned_runner" ]] || fail "pinned runner is not executable: $pinned_runner"
[[ "$(sha256_file "$pinned_runner")" == "$actual_runner_sha" ]] \
    || fail "pinned runner hash mismatch: $pinned_runner"
if (( runner_is_bundle == 1 )); then
    verify_runner_bundle "$runner_dir" "$runner_source_dir" || exit 2
    [[ "$(sha256_file "$runner_dir/original_parity_replay.remote")" == "$runner_wrapper_sha" \
        && "$(sha256_file "$runner_dir/SHA256SUMS")" == "$runner_bundle_manifest_sha" \
        && "$(sha256_file "$runner_dir/LIB_SHA256SUMS")" == "$runner_lib_manifest_sha" ]] \
        || fail "pinned runner bundle provenance mismatch: $runner_dir"
    pinned_runner_exec="$runner_dir/original_parity_replay.remote"
else
    cmp -s -- "$runner_dir/runner.sha256" \
        <(printf '%s  original_parity_replay\n' "$actual_runner_sha") \
        || fail "pinned runner metadata mismatch: $runner_dir/runner.sha256"
fi

work_dir=$(mktemp -d "${TMPDIR:-/tmp}/schema16-final-validation.XXXXXX")
sweep_pids=()
declare -A sweep_live_pids=()
active_audit=
cleanup_validation() {
    local status=$? pid attempt alive temporary
    trap - EXIT INT TERM
    for pid in "${!sweep_live_pids[@]}"; do
        kill -TERM -- "-$pid" 2>/dev/null || kill "$pid" 2>/dev/null || true
    done
    for attempt in {1..50}; do
        alive=0
        for pid in "${!sweep_live_pids[@]}"; do
            if kill -0 -- "-$pid" 2>/dev/null; then
                alive=1
            fi
        done
        if (( alive == 0 )); then
            break
        fi
        sleep 0.1
    done
    for pid in "${!sweep_live_pids[@]}"; do
        kill -KILL -- "-$pid" 2>/dev/null || true
    done
    for pid in "${!sweep_live_pids[@]}"; do
        wait "$pid" 2>/dev/null || true
    done
    if [[ -n "$active_audit" && -d "$active_audit" ]]; then
        while IFS= read -r -d '' temporary; do
            rm -f -- "$temporary"
        done < <(find "$active_audit/logs" "$active_audit/status" \
            -type f -name '*.tmp.*' -print0 2>/dev/null)
    fi
    rm -rf -- "$work_dir"
    exit "$status"
}
trap cleanup_validation EXIT
trap 'exit 130' INT TERM
previous_seed=-1

for campaign_index in "${!normalized_campaigns[@]}"; do
    campaign=${normalized_campaigns[campaign_index]}
    audit=${campaign_audits[campaign_index]}
    expected=$(read_campaign_uint "$campaign" EXPECTED_LOGICAL_REPLAYS) || exit 2
    seed_base=$(read_campaign_uint "$campaign" PARITY_INPUT_SEED_BASE) || exit 2
    trace_schema=$(read_campaign_uint "$campaign" PARITY_TRACE_SCHEMA) || exit 2
    (( expected > 0 )) || fail "campaign expected count must be positive: $campaign"
    [[ "$trace_schema" == 16 ]] || fail "campaign trace schema is not 16: $campaign"
    (( seed_base > previous_seed )) \
        || fail "campaign seed bases must be strictly increasing: $seed_base"
    previous_seed=$seed_base

    require_campaign_drained "$campaign" || exit 2
    manifest="$work_dir/manifest-$seed_base"
    build_complete_manifest "$campaign" "$expected" "$manifest" || exit 2
    # A normal replay lazily creates a native cache beside legacy JSONL, which
    # would change the physical representation after the audit was frozen.
    # Normalize legacy and coexistence inputs first using the pinned converter;
    # `--convert` independently audits readback before deleting the source.
    normalize_manifest_to_native "$manifest" "$audit" || exit 2
    require_campaign_drained "$campaign" || exit 2
    build_complete_manifest "$campaign" "$expected" "$manifest" || exit 2
    snapshot_sha=$(sha256_file "$manifest") || exit 2
    identities="$work_dir/identities-$seed_base"
    build_trace_identities "$manifest" "$identities" || exit 2
    identities_sha=$(sha256_file "$identities") || exit 2
    metadata="$work_dir/metadata-$seed_base"
    {
        printf 'CAMPAIGN=%s\n' "$campaign"
        printf 'PARITY_INPUT_SEED_BASE=%s\n' "$seed_base"
        printf 'PARITY_TRACE_SCHEMA=16\n'
        printf 'EXPECTED_LOGICAL_REPLAYS=%s\n' "$expected"
        printf 'RUNNER_SHA256=%s\n' "$actual_runner_sha"
        printf 'RUNNER_WRAPPER_SHA256=%s\n' "$runner_wrapper_sha"
        printf 'RUNNER_BUNDLE_MANIFEST_SHA256=%s\n' "$runner_bundle_manifest_sha"
        printf 'RUNNER_LIB_MANIFEST_SHA256=%s\n' "$runner_lib_manifest_sha"
        printf 'NATIVE_CONVERSION_PROTOCOL=%s\n' "$native_conversion_protocol"
        printf 'SNAPSHOT_SHA256=%s\n' "$snapshot_sha"
        printf 'TRACE_IDENTITIES_SHA256=%s\n' "$identities_sha"
    } >"$metadata"
    initialize_or_verify_audit \
        "$audit" "$campaign" "$manifest" "$identities" "$metadata" \
        "$snapshot_sha" "$identities_sha" || exit 2

    # Refuse to launch any new work when a resume already contains a corrupt or
    # non-exact proof. During a fresh run, the first failing worker publishes a
    # shared stop file; no later trace starts, while in-flight workers finish
    # and atomically publish their own results before this parent continues.
    if ! verify_existing_audit_proofs "$audit"; then
        classify_failure "$audit"
        exit 1
    fi
    fail_fast_stop="$audit/.parallel-fail-fast-stop"
    active_audit=$audit
    rm -f -- "$fail_fast_stop"
    launch_tmp=$(mktemp "$audit/sweep-launch.env.tmp.XXXXXX") || exit 2
    {
        printf 'LAUNCHED_UTC=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
        printf 'SWEEP_CONCURRENCY=%s\n' "$sweep_concurrency"
        printf 'FAIL_FAST_SEMANTICS=in_flight_complete_no_new_start_after_stop\n'
    } >"$launch_tmp"
    mv -f -- "$launch_tmp" "$audit/sweep-launch.env"
    sweep_failed=0
    sweep_pids=()
    sweep_live_pids=()
    for ((sweep_shard = 0; sweep_shard < sweep_concurrency; sweep_shard += 1)); do
        (
            cd -- "$workspace"
            exec setsid env \
                PARITY_SWEEP_FAIL_FAST=1 \
                PARITY_SWEEP_FAIL_FAST_STOP="$fail_fast_stop" \
                PARITY_SWEEP_GLOBAL_CONCURRENCY="$sweep_concurrency" \
                PARITY_SWEEP_SLOT_DIR="$workspace/.git/parity-runner-slots" \
                scripts/run_parity_release_sweep.sh \
                    "$workspace" "$audit" "$pinned_runner_exec" \
                    "$sweep_shard" "$sweep_concurrency" \
                    {outer_lock_fd}>&-
        ) &
        sweep_pids+=("$!")
        sweep_live_pids["$!"]=1
    done
    sweep_remaining=${#sweep_pids[@]}
    while (( sweep_remaining > 0 )); do
        sweep_status=0
        sweep_finished_pid=
        wait -n -p sweep_finished_pid || sweep_status=$?
        sweep_remaining=$((sweep_remaining - 1))
        if [[ -n "$sweep_finished_pid" ]]; then
            unset 'sweep_live_pids[$sweep_finished_pid]'
        fi
        if (( sweep_status != 0 )); then
            sweep_failed=1
        fi
        # Status 70 means a worker could not publish the coordination stop.
        # Abort every worker group rather than allowing uncoordinated starts.
        if (( sweep_status == 70 )); then
            for sweep_pid in "${!sweep_live_pids[@]}"; do
                kill -TERM -- "-$sweep_pid" 2>/dev/null || true
            done
        fi
    done
    sweep_pids=()
    sweep_live_pids=()
    active_audit=
    if (( sweep_failed != 0 )); then
        classify_failure "$audit"
        exit 1
    fi
    rm -f -- "$fail_fast_stop"

    [[ "$(sha256_file "$pinned_runner")" == "$actual_runner_sha" ]] \
        || fail "pinned runner changed during validation: $pinned_runner"
    if (( runner_is_bundle == 1 )); then
        verify_runner_bundle "$runner_dir" "$runner_source_dir" || exit 2
        [[ "$(sha256_file "$runner_dir/original_parity_replay.remote")" == "$runner_wrapper_sha" \
            && "$(sha256_file "$runner_dir/SHA256SUMS")" == "$runner_bundle_manifest_sha" \
            && "$(sha256_file "$runner_dir/LIB_SHA256SUMS")" == "$runner_lib_manifest_sha" ]] \
            || fail "pinned runner bundle changed during validation: $runner_dir"
    fi
    [[ "$(sha256_file "$audit/traces.snapshot")" == "$snapshot_sha" ]] \
        || fail "frozen snapshot changed during validation: $audit/traces.snapshot"

    verify_dir="$work_dir/verify-$seed_base"
    mkdir -p "$verify_dir"
    if ! verify_exact_audit "$audit" "$expected" "$verify_dir"; then
        classify_failure "$audit"
        exit 1
    fi

    # Capture can be distributed, so repeat the full structural and compressed
    # byte-identity proof immediately before publishing the verdict. Path
    # stability alone is not enough: an in-place replacement must invalidate it.
    require_campaign_drained "$campaign" || exit 2
    terminal_manifest="$work_dir/terminal-manifest-$seed_base"
    build_complete_manifest "$campaign" "$expected" "$terminal_manifest" || exit 2
    cmp -s -- "$terminal_manifest" "$audit/traces.snapshot" \
        || fail "campaign inventory changed during validation: $campaign"
    terminal_identities="$work_dir/terminal-identities-$seed_base"
    build_trace_identities "$terminal_manifest" "$terminal_identities" || exit 2
    cmp -s -- "$terminal_identities" "$audit/traces.sha256" \
        || fail "campaign trace bytes changed during validation: $campaign"
    [[ "$(sha256_file "$terminal_identities")" == "$identities_sha" ]] \
        || fail "terminal trace-identity hash mismatch: $campaign"

    verdict_tmp=$(mktemp "$audit/parity-verdict.env.tmp.XXXXXX") || exit 2
    {
        printf 'VERIFIED_UTC=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
        printf 'CAMPAIGN=%s\n' "$campaign"
        printf 'PARITY_INPUT_SEED_BASE=%s\n' "$seed_base"
        printf 'TOTAL=%s\n' "$expected"
        printf 'PASSED=%s\n' "$expected"
        printf 'FAILED=0\n'
        printf 'EXACT_PARITY=1\n'
        printf 'RUNNER=%s\n' "$pinned_runner_exec"
        printf 'RUNNER_SHA256=%s\n' "$actual_runner_sha"
        printf 'RUNNER_WRAPPER_SHA256=%s\n' "$runner_wrapper_sha"
        printf 'RUNNER_BUNDLE_MANIFEST_SHA256=%s\n' "$runner_bundle_manifest_sha"
        printf 'RUNNER_LIB_MANIFEST_SHA256=%s\n' "$runner_lib_manifest_sha"
        printf 'NATIVE_CONVERSION_PROTOCOL=%s\n' "$native_conversion_protocol"
        printf 'SWEEP_CONCURRENCY=%s\n' "$sweep_concurrency"
        printf 'SNAPSHOT_SHA256=%s\n' "$snapshot_sha"
        printf 'TRACE_IDENTITIES_SHA256=%s\n' "$identities_sha"
    } >"$verdict_tmp"
    mv -f -- "$verdict_tmp" "$audit/parity-verdict.env"
    printf '%s seed=%s exact validation complete: %s/%s\n' \
        "$(date -Is)" "$seed_base" "$expected" "$expected"
done

printf '%s ordered schema16 final validation complete (%s campaign(s))\n' \
    "$(date -Is)" "${#campaigns[@]}"
