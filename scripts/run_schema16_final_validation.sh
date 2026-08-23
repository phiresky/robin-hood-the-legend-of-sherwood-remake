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

audit_parent="$workspace/parity-save-replays/audits"
outer_lock=${SCHEMA16_FINAL_OUTER_LOCK:-/tmp/robin-parity-runner.lock}
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
        digest=$(sha256_file "$trace") || { rm -f -- "$unsorted"; return 1; }
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

[[ -d "$workspace" ]] || fail "workspace is not a directory: $workspace"
workspace=$(realpath -e -- "$workspace")
[[ -x "$runner" ]] || fail "runner is not executable: $runner"
runner=$(realpath -e -- "$runner")
[[ "$expected_runner_sha" =~ ^[0-9a-fA-F]{64}$ ]] \
    || fail 'RUNNER_SHA256 must contain 64 hexadecimal digits'
expected_runner_sha=${expected_runner_sha,,}
actual_runner_sha=$(sha256_file "$runner") || fail "cannot hash runner: $runner"
[[ "$actual_runner_sha" == "$expected_runner_sha" ]] \
    || fail "runner hash mismatch: expected $expected_runner_sha, got $actual_runner_sha"
[[ -x "$workspace/scripts/run_parity_release_sweep.sh" ]] \
    || fail "missing release sweep under workspace: $workspace"

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
        "$audit_parent/schema16-final-${campaign_label}-path-$path_digest-runner-$actual_runner_sha"
    )
done
for audit in "${campaign_audits[@]}"; do
    mkdir -p -- "${audit%/*}"
    if [[ -f "$audit/parity-verdict.env" ]]; then
        mv -f -- "$audit/parity-verdict.env" "$audit/parity-verdict.previous.env"
    fi
done

runner_dir="$workspace/.git/schema16-final-runners/$actual_runner_sha"
pinned_runner="$runner_dir/original_parity_replay"
mkdir -p -- "$workspace/.git/schema16-final-runners"
if [[ ! -e "$runner_dir" ]]; then
    runner_tmp=$(mktemp -d \
        "$workspace/.git/schema16-final-runners/.runner-$actual_runner_sha.tmp.XXXXXX")
    cp -p -- "$runner" "$runner_tmp/original_parity_replay"
    chmod 0555 "$runner_tmp/original_parity_replay"
    printf '%s  original_parity_replay\n' "$actual_runner_sha" >"$runner_tmp/runner.sha256"
    mv -- "$runner_tmp" "$runner_dir"
fi
[[ -x "$pinned_runner" ]] || fail "pinned runner is not executable: $pinned_runner"
[[ "$(sha256_file "$pinned_runner")" == "$actual_runner_sha" ]] \
    || fail "pinned runner hash mismatch: $pinned_runner"
cmp -s -- "$runner_dir/runner.sha256" \
    <(printf '%s  original_parity_replay\n' "$actual_runner_sha") \
    || fail "pinned runner metadata mismatch: $runner_dir/runner.sha256"

work_dir=$(mktemp -d "${TMPDIR:-/tmp}/schema16-final-validation.XXXXXX")
trap 'rm -rf -- "$work_dir"' EXIT
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
        printf 'SNAPSHOT_SHA256=%s\n' "$snapshot_sha"
        printf 'TRACE_IDENTITIES_SHA256=%s\n' "$identities_sha"
    } >"$metadata"
    initialize_or_verify_audit \
        "$audit" "$campaign" "$manifest" "$identities" "$metadata" \
        "$snapshot_sha" "$identities_sha" || exit 2

    if ! (
        cd -- "$workspace"
        env \
            PARITY_SWEEP_FAIL_FAST=1 \
            PARITY_SWEEP_GLOBAL_CONCURRENCY=1 \
            PARITY_SWEEP_SLOT_DIR="$workspace/.git/parity-runner-slots" \
            scripts/run_parity_release_sweep.sh \
                "$workspace" "$audit" "$pinned_runner" 0 1
    ); then
        classify_failure "$audit"
        exit 1
    fi

    [[ "$(sha256_file "$pinned_runner")" == "$actual_runner_sha" ]] \
        || fail "pinned runner changed during validation: $pinned_runner"
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
        printf 'RUNNER=%s\n' "$pinned_runner"
        printf 'RUNNER_SHA256=%s\n' "$actual_runner_sha"
        printf 'SNAPSHOT_SHA256=%s\n' "$snapshot_sha"
        printf 'TRACE_IDENTITIES_SHA256=%s\n' "$identities_sha"
    } >"$verdict_tmp"
    mv -f -- "$verdict_tmp" "$audit/parity-verdict.env"
    printf '%s seed=%s exact validation complete: %s/%s\n' \
        "$(date -Is)" "$seed_base" "$expected" "$expected"
done

printf '%s ordered schema16 final validation complete (%s campaign(s))\n' \
    "$(date -Is)" "${#campaigns[@]}"
