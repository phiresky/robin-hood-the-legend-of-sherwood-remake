#!/usr/bin/env bash
set -euo pipefail

# After the authenticated seed-2/3/4 campaign has produced three complete
# non-exact corpus proofs, capture, normalize and validate fresh schema-16
# corpora serially from seed base 5,000,000 onward. Semantic failures retain a
# complete collect-all audit and advance by one million. Operational, setup or
# integrity failures stop immediately and are resumed in place.

if (( $# != 8 )); then
    printf 'usage: %s WORKSPACE RECORDER RECORDER_SHA RUNNER_BUNDLE BUNDLE_TRUST_SHA RUNNER_SHA EXISTING_AUDIT AUDIT_ROOT\n' "$0" >&2
    exit 2
fi

workspace_arg=$1
recorder_arg=$2
recorder_sha=${3,,}
bundle_arg=$4
bundle_trust_sha=${5,,}
runner_sha=${6,,}
existing_audit_arg=$7
audit_root_arg=$8

capture_jobs=${SCHEMA16_ONWARD_CAPTURE_JOBS:-8}
capture_convert_jobs=${SCHEMA16_ONWARD_CAPTURE_CONVERT_JOBS:-3}
prepass_jobs=${SCHEMA16_ONWARD_PREPASS_JOBS:-5}
convert_timeout=${SCHEMA16_ONWARD_CONVERT_TIMEOUT_SECONDS:-7200}
sweep_concurrency=${SCHEMA16_ONWARD_SWEEP_CONCURRENCY:-8}
final_outer_lock=${SCHEMA16_FINAL_OUTER_LOCK:-/tmp/robin-parity-runner.lock}
first_seed=5000000
seed_step=1000000
expected_replays=9720

# These are the exact scripts deployed with the authenticated seed-2/3/4
# controller. Updating any of them requires an explicit review and hash bump.
expected_capture_script_sha=0e99a6a935335761eef740faee097708282e7bd4584c0d1275f07beb539442d6
expected_prepass_script_sha=d2b7fd1eb29a921655a3aec49617ca5c48e70cbf990f94593490ba17631a074b
expected_final_script_sha=229146eebcf0e3b09bf2987d2af17917e3addaa813697a905893ce935549c6b9
expected_sweep_script_sha=e7a1c769a18b76a69c3473f68caa38a6b741c002e826bff97c106e7fdf746cfb
expected_helper_script_sha=79c0d5c9d770812be5ae541ca12d017f21dbb2b9c30e480bde56cfb90a8d27a7

fail() {
    printf 'error: %s\n' "$*" >&2
    exit 2
}

sha256_file() {
    local value
    value=$(sha256sum -- "$1") || return 1
    printf '%s\n' "${value%% *}"
}

normalize_bounded_uint() {
    local LC_ALL=C value=$1 limit=$2
    [[ "$value" =~ ^[0-9]+$ ]] || return 1
    while [[ ${#value} -gt 1 && "$value" == 0* ]]; do
        value=${value#0}
    done
    if (( ${#value} > ${#limit} )) \
        || { (( ${#value} == ${#limit} )) && [[ "$value" > "$limit" ]]; }
    then
        return 1
    fi
    printf '%s\n' "$value"
}

write_atomic() {
    local destination=$1 temporary
    temporary=$(mktemp "${destination}.tmp.XXXXXX") || return 1
    if ! cat >"$temporary" || ! mv -f -- "$temporary" "$destination"; then
        rm -f -- "$temporary"
        return 1
    fi
}

read_one_value() {
    local file=$1 key=$2
    local -a values=()
    mapfile -t values < <(sed -n "s/^${key}=//p" "$file")
    (( ${#values[@]} == 1 )) || return 1
    printf '%s\n' "${values[0]}"
}

verify_script_hash() {
    local path=$1 expected=$2
    [[ -x "$path" ]] || fail "required script is not executable: $path"
    [[ "$(sha256_file "$path")" == "$expected" ]] \
        || fail "required script hash mismatch: $path"
}

pin_recorder() {
    local source=$1 destination directory temporary pin_lock_fd
    directory="$workspace/.git/schema16-onward-recorders/$recorder_sha"
    destination="$directory/robin"
    mkdir -p -- "${directory%/*}"
    exec {pin_lock_fd}>"${directory%/*}/.pin.lock"
    flock "$pin_lock_fd" || fail 'cannot lock pinned recorder store'
    if [[ ! -e "$directory" ]]; then
        temporary=$(mktemp -d "${directory}.tmp.XXXXXX") \
            || fail 'cannot stage pinned recorder'
        if ! cp -p -- "$source" "$temporary/robin" \
            || ! chmod 0555 "$temporary/robin" \
            || [[ "$(sha256_file "$temporary/robin")" != "$recorder_sha" ]] \
            || ! mv -T -- "$temporary" "$directory"
        then
            rm -rf -- "$temporary"
            fail 'cannot publish authenticated pinned recorder'
        fi
    fi
    [[ -d "$directory" && ! -L "$directory" && -f "$destination" \
        && ! -L "$destination" && -x "$destination" \
        && "$(find "$directory" -mindepth 1 -maxdepth 1 -printf '%f\n')" == robin \
        && "$(sha256_file "$destination")" == "$recorder_sha" ]] \
        || fail 'pinned recorder directory is malformed or changed'
    exec {pin_lock_fd}>&-
    printf '%s\n' "$destination"
}

pin_capture_bundle() {
    local destination temporary pin_lock_fd shared_runner_fd
    destination="$workspace/.git/schema16-final-runners/$bundle_trust_sha"
    mkdir -p -- "${destination%/*}"
    # The final validator holds this same lock across its existing runner-pin
    # publisher. Sharing that protocol makes its plain directory rename and
    # this controller's no-target-directory rename mutually exclusive.
    mkdir -p -- "$(dirname -- "$final_outer_lock")"
    exec {shared_runner_fd}>"$final_outer_lock"
    flock "$shared_runner_fd" || fail 'cannot lock shared final runner publisher'
    exec {pin_lock_fd}>"${destination%/*}/.pin.lock"
    flock "$pin_lock_fd" || fail 'cannot lock pinned runner store'
    if [[ ! -e "$destination" ]]; then
        temporary=$(mktemp -d "${destination%/*}/.runner-${bundle_trust_sha}.tmp.XXXXXX") \
            || fail 'cannot stage pinned runner bundle'
        if ! cp -a --no-preserve=ownership -- "$bundle/." "$temporary/" \
            || ! verify_capture_bundle "$temporary" "$bundle" \
            || ! mv -T -- "$temporary" "$destination"
        then
            rm -rf -- "$temporary"
            fail 'cannot publish pinned runner bundle'
        fi
    fi
    verify_capture_bundle "$destination" "$bundle"
    exec {pin_lock_fd}>&-
    exec {shared_runner_fd}>&-
    printf '%s\n' "$destination"
}

verify_capture_bundle() {
    local candidate=$1 loader_proof_root=$2 manifest line path
    verify_bundle "$candidate"
    [[ -x "$candidate/original_parity_replay" \
        && -x "$candidate/original_parity_replay.remote" \
        && -x "$candidate/lib/ld-linux-x86-64.so.2" \
        && -f "$candidate/LOADER_LIST.txt" ]] \
        || fail "runner bundle lacks canonical runtime inputs: $candidate"
    if find "$candidate" -type l -print -quit | grep -q .; then
        fail "runner bundle contains a symlink: $candidate"
    fi
    for manifest in "$candidate/SHA256SUMS" "$candidate/LIB_SHA256SUMS"; do
        while IFS= read -r line; do
            [[ "$line" =~ ^[0-9a-fA-F]{64}[[:space:]][\ \*](.+)$ ]] \
                || fail "malformed runner bundle checksum entry: $manifest"
            path=${BASH_REMATCH[1]}
            [[ "$path" != /* && "$path" != ../* && "$path" != */../* \
                && "$path" != *'/..' && "$path" != *$'\n'* ]] \
                || fail "unsafe runner bundle checksum path: $path"
        done <"$manifest"
    done
    diff -u -- \
        <(find "$candidate/lib" -type f -printf 'lib/%P\n' | LC_ALL=C sort) \
        <(sed -n 's/^[0-9a-fA-F]\{64\} [ *]//p' "$candidate/LIB_SHA256SUMS" \
            | LC_ALL=C sort) >/dev/null \
        || fail "runner library manifest does not exactly cover lib tree: $candidate"
    diff -u -- <(printf 'lib\n') \
        <(find "$candidate" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' \
            | LC_ALL=C sort) >/dev/null \
        || fail "runner bundle has an unexpected root directory: $candidate"
    diff -u -- \
        <(printf '%s\n' LIB_SHA256SUMS LOADER_LIST.txt PROVENANCE.txt \
            original_parity_replay original_parity_replay.remote | LC_ALL=C sort) \
        <(sed -n 's/^[0-9a-fA-F]\{64\} [ *]//p' "$candidate/SHA256SUMS" \
            | LC_ALL=C sort) >/dev/null \
        || fail "runner main manifest does not have the canonical file set: $candidate"
    diff -u -- \
        <(printf '%s\n' LIB_SHA256SUMS LOADER_LIST.txt PROVENANCE.txt SHA256SUMS \
            original_parity_replay original_parity_replay.remote | LC_ALL=C sort) \
        <(find "$candidate" -maxdepth 1 -type f -printf '%f\n' | LC_ALL=C sort) \
        >/dev/null || fail "runner bundle root file set is not canonical: $candidate"
    grep -Fq -- "=> $loader_proof_root/lib/ld-linux-x86-64.so.2 " \
        "$candidate/LOADER_LIST.txt" \
        || fail "runner loader proof is not bound to authenticated source: $candidate"
    awk -v prefix="$loader_proof_root/lib/" '
        /=>/ {
            resolved=$0
            sub(/^.*=>[[:space:]]*/, "", resolved)
            sub(/[[:space:]].*$/, "", resolved)
            if (index(resolved, prefix) != 1) exit 1
        }
    ' "$candidate/LOADER_LIST.txt" \
        || fail "runner loader proof resolves outside authenticated lib tree: $candidate"
    grep -Eq '^[0-9a-fA-F]{64} [ *]original_parity_replay$' "$candidate/SHA256SUMS" \
        && grep -Eq '^[0-9a-fA-F]{64} [ *]original_parity_replay\.remote$' "$candidate/SHA256SUMS" \
        && grep -Eq '^[0-9a-fA-F]{64} [ *]LIB_SHA256SUMS$' "$candidate/SHA256SUMS" \
        && grep -Eq '^[0-9a-fA-F]{64} [ *]PROVENANCE\.txt$' "$candidate/SHA256SUMS" \
        && grep -Eq '^[0-9a-fA-F]{64} [ *]LOADER_LIST\.txt$' "$candidate/SHA256SUMS" \
        && grep -Eq '^[0-9a-fA-F]{64} [ *]lib/ld-linux-x86-64\.so\.2$' \
            "$candidate/LIB_SHA256SUMS" \
        || fail "runner manifests omit canonical runtime inputs: $candidate"
}

write_phase() {
    local phase=$1
    {
        printf 'UPDATED_UTC=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
        printf 'PHASE=%s\n' "$phase"
    } | write_atomic "$audit_root/state.env" || fail 'cannot publish controller phase'
    printf '%s phase=%s\n' "$(date -Is)" "$phase"
}

campaign_for_seed() {
    printf '%s/parity-save-replays/60s-random-input/schema16-seed%s-onward\n' \
        "$workspace" "$1"
}

initialize_campaign() {
    local seed=$1 campaign=$2 candidate temporary
    if [[ ! -e "$campaign" ]]; then
        mkdir -p -- "${campaign%/*}"
        temporary=$(mktemp -d "${campaign}.tmp.XXXXXX") \
            || fail "cannot stage seed $seed campaign"
        campaign=$temporary
        candidate="$campaign/campaign.env"
        mkdir -p -- "$campaign/traces" "$campaign/logs"
        {
            printf 'CAMPAIGN_STATE=recording_authenticated_onward_controller\n'
            printf 'PARITY_TRACE_SCHEMA=16\nPARITY_RANDOM_REPLAYS=40\nPARITY_FRAMES=1500\n'
            printf 'PARITY_INPUT_SEED_BASE=%s\nPARITY_INPUT_SEED_FIRST=%s\n' "$seed" "$seed"
            printf 'PARITY_INPUT_SEED_LAST=%s\nPARITY_SEED=1\n' "$((seed + 39))"
            printf 'SHERWOOD_LIMIT=30\nSHERWOOD_SAMPLE_SEED=1\n'
            printf 'SHARD_COUNT=1\nCAPTURE_JOBS=%s\nCAPTURE_CONVERT_JOBS=%s\n' \
                "$capture_jobs" "$capture_convert_jobs"
            printf 'EXPECTED_SELECTED_SAVES=243\n'
            printf 'EXPECTED_LOGICAL_REPLAYS=%s\nEXPECTED_FRAMES_PER_REPLAY=1500\n' "$expected_replays"
            printf 'ROBIN_BINARY=%s\nROBIN_BINARY_SHA256=%s\n' "$recorder" "$recorder_sha"
        } >"$candidate" || { rm -rf -- "$temporary"; fail "cannot stage seed $seed metadata"; }
        if ! mv -T -- "$temporary" "$(campaign_for_seed "$seed")"; then
            rm -rf -- "$temporary"
            fail "cannot publish seed $seed campaign"
        fi
        return 0
    fi
    [[ -d "$campaign" && ! -L "$campaign" && -f "$campaign/campaign.env" ]] \
        || fail "refusing unauthenticated pre-existing seed $seed campaign path"
    candidate="$campaign/campaign.env.candidate"
    [[ -d "$campaign/traces" && -d "$campaign/logs" ]] \
        || fail "seed $seed campaign is missing required directories"
    {
        printf 'CAMPAIGN_STATE=recording_authenticated_onward_controller\n'
        printf 'PARITY_TRACE_SCHEMA=16\nPARITY_RANDOM_REPLAYS=40\nPARITY_FRAMES=1500\n'
        printf 'PARITY_INPUT_SEED_BASE=%s\nPARITY_INPUT_SEED_FIRST=%s\n' "$seed" "$seed"
        printf 'PARITY_INPUT_SEED_LAST=%s\nPARITY_SEED=1\n' "$((seed + 39))"
        printf 'SHERWOOD_LIMIT=30\nSHERWOOD_SAMPLE_SEED=1\n'
        printf 'SHARD_COUNT=1\nCAPTURE_JOBS=%s\nCAPTURE_CONVERT_JOBS=%s\n' \
            "$capture_jobs" "$capture_convert_jobs"
        printf 'EXPECTED_SELECTED_SAVES=243\n'
        printf 'EXPECTED_LOGICAL_REPLAYS=%s\nEXPECTED_FRAMES_PER_REPLAY=1500\n' "$expected_replays"
        printf 'ROBIN_BINARY=%s\nROBIN_BINARY_SHA256=%s\n' "$recorder" "$recorder_sha"
    } >"$candidate" || fail "cannot stage seed $seed campaign metadata"
    if [[ -f "$campaign/campaign.env" ]]; then
        cmp -s -- "$candidate" "$campaign/campaign.env" \
            || fail "seed $seed campaign metadata differs from its immutable prior invocation"
        rm -f -- "$candidate"
    else
        mv -- "$candidate" "$campaign/campaign.env" \
            || fail "cannot publish seed $seed campaign metadata"
    fi
}

verify_existing_controller_provenance() {
    local provenance="$existing_audit/provenance.env"
    [[ -f "$provenance" ]] || fail 'existing seed-2/3/4 audit lacks provenance.env'
    [[ "$(read_one_value "$provenance" WORKSPACE)" == "$workspace" \
        && "$(read_one_value "$provenance" RUNNER_BUNDLE)" == "$bundle" \
        && "$(read_one_value "$provenance" RUNNER_SHA256)" == "$runner_sha" \
        && "$(read_one_value "$provenance" RUNNER_BUNDLE_TRUST_SHA256)" == "$bundle_trust_sha" \
        && "$(read_one_value "$provenance" ORCHESTRATOR_SCRIPT_SHA256)" == "$expected_helper_script_sha" \
        && "$(read_one_value "$provenance" PREPASS_SCRIPT_SHA256)" == "$expected_prepass_script_sha" \
        && "$(read_one_value "$provenance" FINAL_SCRIPT_SHA256)" == "$expected_final_script_sha" \
        && "$(read_one_value "$provenance" SWEEP_SCRIPT_SHA256)" == "$expected_sweep_script_sha" ]] \
        || fail 'existing seed-2/3/4 audit provenance is not the authenticated current campaign'
}

execute_capture_impl() {
    local seed=$1 campaign=$2 rc=0
    local capture_home="$workspace/.schema16-capture-home"
    local capture_tmp="$workspace/tmp/schema16-onward-capture"
    mkdir -p -- "$capture_home" "$capture_tmp"
    write_phase "capture-seed$seed"
    [[ "$(sha256_file "$recorder")" == "$recorder_sha" ]] \
        || fail 'pinned recorder changed before capture'
    verify_capture_bundle "$capture_bundle" "$bundle"
    /usr/bin/env -i PATH=/usr/bin:/bin LC_ALL=C TZ=UTC \
        HOME="$capture_home" TMPDIR="$capture_tmp" \
        PARITY_TRACE_SCHEMA=16 PARITY_RANDOM_REPLAYS=40 PARITY_FRAMES=1500 \
        PARITY_INPUT_SEED_BASE="$seed" PARITY_SEED=1 SHERWOOD_LIMIT=30 \
        SHERWOOD_SAMPLE_SEED=1 SHARD_COUNT=1 SHARD_INDEX=0 \
        CAPTURE_JOBS="$capture_jobs" CONVERT_JOBS="$capture_convert_jobs" COMPRESS=1 \
        HEADFUL=0 SKIP_BUILD=1 WATCHDOG_SECONDS=2700 \
        CAPTURE_MIN_FREE_KIB=31457280 CAPTURE_RESERVE_KIB=9437184 \
        CAPTURE_EMERGENCY_FREE_KIB=32505856 CAPTURE_GATE_POLL_SECONDS=2 \
        CAPTURE_EMERGENCY_POLL_SECONDS=1 CAPTURE_EMERGENCY_KILL_AFTER_SECONDS=2 \
        CAPTURE_PAUSE_FILE="$campaign/.capture.pause" \
        CAPTURE_DRAIN_FILE="$campaign/.capture.drain" CAPTURE_DISK_PATH="$campaign" \
        ROBIN_BINARY="$recorder" \
        ROBIN_LIBRARY_DIR="$workspace/original-code/runtime-i386" \
        ROBINHOOD_DATA_DIR="$workspace/datadirs/fullgame_linux" \
        PARITY_CONVERTER="$capture_bundle/original_parity_replay.remote" \
        "$capture_script" "$workspace/reference-saves" "$campaign" \
            "$workspace/datadirs/fullgame_linux" \
        >>"$audit_root/capture-seed$seed.session.log" 2>&1 || rc=$?
    (( rc == 0 )) || fail "seed $seed capture failed with status $rc; preserved for in-place repair"
    [[ "$(sha256_file "$recorder")" == "$recorder_sha" ]] \
        || fail 'pinned recorder changed during capture'
    verify_capture_bundle "$capture_bundle" "$bundle"
    verify_campaign_inventory "$campaign" "$expected_replays"
}

execute_capture() {
    execute_capture_impl "$@"
}

archive_session_log() {
    local log=$1 label=$2 archive
    [[ -f "$log" ]] || return 0
    mkdir -p -- "$audit_root/session-log-history"
    archive="$audit_root/session-log-history/${label}-$(date -u +%Y%m%dT%H%M%SZ)-$BASHPID.log"
    cp -- "$log" "$archive" || fail "cannot preserve prior session log: $log"
}

execute_prepass_impl() {
    local seed=$1 campaign=$2 audit
    audit="$audit_root/native-seed${seed}-p${prepass_jobs}"
    write_phase "normalize-seed$seed"
    archive_session_log "$audit_root/native-seed${seed}-p${prepass_jobs}.session.log" \
        "native-seed${seed}-p${prepass_jobs}"
    env NATIVE_CONVERT_JOBS="$prepass_jobs" \
        NATIVE_CONVERT_MIN_AVAILABLE_KIB_PER_JOB=10485760 \
        NATIVE_CONVERT_OUTER_LOCK="$final_outer_lock" \
        NATIVE_CONVERT_TIMEOUT_SECONDS="$convert_timeout" \
        "$prepass_script" "$workspace" "$campaign" "$bundle" \
            "$bundle_trust_sha" "$audit" \
        >"$audit_root/native-seed${seed}-p${prepass_jobs}.session.log" 2>&1 \
        || fail "seed $seed normalization failed; preserved for in-place repair"
    [[ -f "$audit/COMPLETE" ]] \
        || fail "seed $seed normalization returned without COMPLETE"
}

execute_prepass() {
    execute_prepass_impl "$@"
}

execute_final() {
    local seed=$1 campaign=$2
    archive_session_log "$audit_root/final-seed$seed.session.log" "final-seed$seed"
    archive_session_log "$audit_root/collect-all-seed$seed.session.log" "collect-all-seed$seed"
    run_final_campaign "$seed" "$campaign"
}

verify_summary_for_campaign() {
    local summary_seed=$1 input_seed=$2 campaign=$3 summary=$4
    local exact nonexact audit
    [[ -f "$summary" ]] || return 2
    [[ "$(read_one_value "$summary" SEED)" == "$summary_seed" ]] \
        || fail "summary seed does not match authenticated campaign: $summary"
    exact=$(read_one_value "$summary" EXACT_PARITY) \
        || fail "malformed final summary: $summary"
    audit=$(read_one_value "$summary" AUDIT) \
        || fail "final summary lacks AUDIT: $summary"
    [[ "$campaign" == "$workspace"/* && -d "$campaign" && ! -L "$campaign" ]] \
        || fail "summary campaign is outside the authenticated workspace: $campaign"
    [[ "$audit" == "$(final_audit_for_campaign "$campaign")" && -d "$audit" ]] \
        || fail "summary names the wrong runner-specific campaign audit: $summary"
    [[ "$(read_one_value "$audit/validation.env" CAMPAIGN)" == "$campaign" \
        && "$(read_one_value "$audit/validation.env" PARITY_INPUT_SEED_BASE)" == "$input_seed" \
        && "$(read_one_value "$audit/validation.env" PARITY_TRACE_SCHEMA)" == 16 \
        && "$(read_one_value "$audit/validation.env" EXPECTED_LOGICAL_REPLAYS)" == "$expected_replays" \
        && "$(read_one_value "$audit/validation.env" RUNNER_SHA256)" == "$runner_sha" \
        && "$(read_one_value "$audit/validation.env" RUNNER_BUNDLE_MANIFEST_SHA256)" \
            == "$(sha256_file "$bundle/SHA256SUMS")" \
        && "$(read_one_value "$audit/validation.env" RUNNER_LIB_MANIFEST_SHA256)" \
            == "$(sha256_file "$bundle/LIB_SHA256SUMS")" ]] \
        || fail "summary proof is not bound to its campaign and authenticated current runner: $summary"
    case "$exact" in
    1)
        [[ "$(read_one_value "$audit/parity-verdict.env" CAMPAIGN)" == "$campaign" \
            && "$(read_one_value "$audit/parity-verdict.env" PARITY_INPUT_SEED_BASE)" == "$input_seed" \
            && "$(read_one_value "$audit/parity-verdict.env" TOTAL)" == "$expected_replays" \
            && "$(read_one_value "$audit/parity-verdict.env" PASSED)" == "$expected_replays" \
            && "$(read_one_value "$audit/parity-verdict.env" FAILED)" == 0 \
            && "$(read_one_value "$audit/parity-verdict.env" EXACT_PARITY)" == 1 \
            && "$(read_one_value "$audit/parity-verdict.env" RUNNER_SHA256)" == "$runner_sha" ]] \
            || fail "exact summary lacks its campaign-bound exact verdict: $summary"
        verify_frozen_trace_identities "$audit" \
            || fail "exact summary has invalid frozen trace identities: $summary"
        return 0
        ;;
    0)
        nonexact=$(read_one_value "$summary" NONEXACT) \
            || fail "nonexact summary lacks NONEXACT: $summary"
        [[ "$nonexact" =~ ^[1-9][0-9]*$ ]] \
            || fail "nonexact summary has no failures: $summary"
        [[ "$(verify_collect_all_set "$audit" "$expected_replays")" == "$nonexact" ]] \
            || fail "collect-all proof differs from its summary: $summary"
        verify_frozen_trace_identities "$audit" \
            || fail "nonexact summary has invalid frozen trace identities: $summary"
        return 1
        ;;
    *) fail "summary has invalid EXACT_PARITY=$exact: $summary" ;;
    esac
}

read_prior_campaign() {
    local seed=$1 campaign
    campaign=$(read_one_value "$existing_audit/provenance.env" "SEED${seed}") \
        || fail "existing provenance lacks seed $seed campaign"
    [[ "$campaign" =~ ^/[-_/A-Za-z0-9.]+$ ]] \
        || fail "seed $seed provenance path is not a canonical absolute path"
    printf '%s\n' "$campaign"
}

verify_prior_summary() {
    local seed=$1 campaign
    campaign=$(read_prior_campaign "$seed") || return $?
    verify_campaign_metadata "$campaign" "$((seed * 1000000))"
    verify_summary_for_campaign "$seed" "$((seed * 1000000))" "$campaign" \
        "$existing_audit/final-seed${seed}.env"
}

verify_prior_proof_gate() {
    local seed rc
    exact_prior_seed=
    prior_nonexact=0
    for seed in 2 3 4; do
        rc=0
        verify_prior_summary "$seed" || rc=$?
        case "$rc" in
        0) exact_prior_seed=$seed; return 0 ;;
        1) prior_nonexact=$((prior_nonexact + 1)) ;;
        *) fail "seed $seed prerequisite proof is missing or invalid" ;;
        esac
    done
    (( prior_nonexact == 3 )) || fail 'internal prior-proof accounting failure'
    return 1
}

verify_onward_summary() {
    local seed=$1 campaign=$2
    verify_campaign_metadata "$campaign" "$seed"
    verify_summary_for_campaign "$seed" "$seed" "$campaign" \
        "$audit_root/final-seed${seed}.env"
}

initialize_or_verify_provenance() {
    local candidate="$audit_root/provenance.candidate.env"
    {
        printf 'WORKSPACE=%s\nRECORDER_SOURCE=%s\nRECORDER=%s\nRECORDER_SHA256=%s\n' \
            "$workspace" "$recorder_source" "$recorder" "$recorder_sha"
        printf 'RUNNER_BUNDLE=%s\nRUNNER_BUNDLE_TRUST_SHA256=%s\nRUNNER_SHA256=%s\n' \
            "$bundle" "$bundle_trust_sha" "$runner_sha"
        printf 'CAPTURE_RUNNER_BUNDLE=%s\n' "$capture_bundle"
        printf 'EXISTING_AUDIT=%s\nFIRST_SEED=%s\nSEED_STEP=%s\n' \
            "$existing_audit" "$first_seed" "$seed_step"
        printf 'CAPTURE_JOBS=%s\nCAPTURE_CONVERT_JOBS=%s\nPREPASS_JOBS=%s\n' \
            "$capture_jobs" "$capture_convert_jobs" "$prepass_jobs"
        printf 'PREPASS_MIN_AVAILABLE_KIB_PER_JOB=10485760\n'
        printf 'CONVERT_TIMEOUT_SECONDS=%s\n' "$convert_timeout"
        printf 'SWEEP_CONCURRENCY=%s\nFINAL_OUTER_LOCK=%s\n' \
            "$sweep_concurrency" "$final_outer_lock"
        printf 'CONTROLLER_SCRIPT_SHA256=%s\n' "$(sha256_file "$controller_script")"
        printf 'CAPTURE_SCRIPT_SHA256=%s\nPREPASS_SCRIPT_SHA256=%s\n' \
            "$expected_capture_script_sha" "$expected_prepass_script_sha"
        printf 'FINAL_SCRIPT_SHA256=%s\nSWEEP_SCRIPT_SHA256=%s\nHELPER_SCRIPT_SHA256=%s\n' \
            "$expected_final_script_sha" "$expected_sweep_script_sha" "$expected_helper_script_sha"
    } >"$candidate" || fail 'cannot stage controller provenance'
    if [[ -f "$audit_root/provenance.env" ]]; then
        cmp -s -- "$candidate" "$audit_root/provenance.env" \
            || fail 'controller invocation differs from immutable prior provenance'
        rm -f -- "$candidate"
    else
        mv -- "$candidate" "$audit_root/provenance.env" \
            || fail 'cannot publish controller provenance'
    fi
}

controller_loop() {
    local seed campaign result_seed result_campaign
    if [[ -f "$audit_root/result.env" ]]; then
        [[ "$(read_one_value "$audit_root/result.env" EXACT_PARITY)" == 1 ]] \
            || fail 'committed onward result is malformed'
        result_seed=$(read_one_value "$audit_root/result.env" SEED) \
            || fail 'committed onward result lacks SEED'
        result_campaign=$(read_one_value "$audit_root/result.env" CAMPAIGN) \
            || fail 'committed onward result lacks CAMPAIGN'
        [[ "$result_seed" =~ ^[0-9]{1,10}$ ]] \
            || fail 'committed onward result has an invalid seed'
        (( result_seed >= first_seed && result_seed <= 4294967295 \
            && (result_seed - first_seed) % seed_step == 0 )) \
            || fail 'committed onward result is outside the seed ladder'
        [[ "$result_campaign" == "$(campaign_for_seed "$result_seed")" ]] \
            || fail 'committed onward result is outside the seed ladder'
        verify_onward_summary "$result_seed" "$result_campaign" \
            || fail 'committed onward exact result no longer authenticates'
        write_phase complete-exact-resumed
        return 0
    fi
    seed=$first_seed
    while :; do
        (( seed <= 4294967256 )) \
            || fail 'schema16 onward ladder exhausted the 32-bit 40-replay seed range'
        campaign=$(campaign_for_seed "$seed")
        if [[ -f "$audit_root/final-seed${seed}.env" ]]; then
            if verify_onward_summary "$seed" "$campaign"; then
                {
                    printf 'COMPLETED_UTC=%s\nEXACT_PARITY=1\nSEED=%s\nCAMPAIGN=%s\n' \
                        "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$seed" "$campaign"
                } | write_atomic "$audit_root/result.env" \
                    || fail 'cannot recover exact onward result'
                write_phase complete-exact-recovered
                return 0
            fi
            seed=$((seed + seed_step))
            continue
        fi
        initialize_campaign "$seed" "$campaign"
        execute_capture "$seed" "$campaign" || return $?
        execute_prepass "$seed" "$campaign" || return $?
        if execute_final "$seed" "$campaign"; then
            {
                printf 'COMPLETED_UTC=%s\nEXACT_PARITY=1\nSEED=%s\nCAMPAIGN=%s\n' \
                    "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$seed" "$campaign"
            } | write_atomic "$audit_root/result.env" \
                || fail 'cannot publish exact onward result'
            write_phase complete-exact
            return 0
        fi
        # execute_final returns one only after the helper has authenticated a
        # semantic failure and completed the full 9,720-trace evidence set.
        seed=$((seed + seed_step))
    done
}

validate_controller_settings() {
    capture_jobs=$(normalize_bounded_uint "$capture_jobs" 10) \
        || fail 'capture jobs must be 1 through 10'
    (( capture_jobs >= 1 )) || fail 'capture jobs must be 1 through 10'
    capture_convert_jobs=$(normalize_bounded_uint "$capture_convert_jobs" 8) \
        || fail 'capture conversion jobs must be 1 through 8'
    (( capture_convert_jobs >= 1 )) || fail 'capture conversion jobs must be 1 through 8'
    prepass_jobs=$(normalize_bounded_uint "$prepass_jobs" 8) \
        || fail 'prepass jobs must be 1 through 8'
    (( prepass_jobs >= 1 )) || fail 'prepass jobs must be 1 through 8'
    convert_timeout=$(normalize_bounded_uint "$convert_timeout" 2147483647) \
        || fail 'conversion timeout must be 3600 through 2147483647 seconds'
    (( convert_timeout >= 3600 )) \
        || fail 'conversion timeout must be 3600 through 2147483647 seconds'
    sweep_concurrency=$(normalize_bounded_uint "$sweep_concurrency" 64) \
        || fail 'sweep concurrency must be 1 through 64'
    (( sweep_concurrency >= 1 )) || fail 'sweep concurrency must be 1 through 64'
}

if [[ "${BASH_SOURCE[0]}" != "$0" ]]; then
    return 0
fi

validate_controller_settings
[[ "$recorder_sha" =~ ^[0-9a-f]{64}$ && "$bundle_trust_sha" =~ ^[0-9a-f]{64}$ \
    && "$runner_sha" =~ ^[0-9a-f]{64}$ ]] || fail 'trust values must be SHA-256 digests'

workspace=$(realpath -e -- "$workspace_arg")
recorder_source=$(realpath -e -- "$recorder_arg")
bundle=$(realpath -e -- "$bundle_arg")
existing_audit=$(realpath -e -- "$existing_audit_arg")
audit_parent=$(dirname -- "$audit_root_arg")
mkdir -p -- "$audit_parent"
audit_root=$(realpath -m -- "$audit_root_arg")
controller_script=$(realpath -e -- "$0")
capture_script="$workspace/original-code/scripts/capture_parity_save_replays.sh"
prepass_script="$workspace/scripts/run_native_conversion_prepass.sh"
final_script="$workspace/scripts/run_schema16_final_validation.sh"
sweep_script="$workspace/scripts/run_parity_release_sweep.sh"
helper_script="$workspace/scripts/run_schema16_existing_corpora_orchestrator.sh"

[[ "$recorder_source" == "$workspace"/* && "$bundle" == "$workspace"/* \
    && "$existing_audit" == "$workspace"/* && "$audit_root" == "$workspace"/* ]] \
    || fail 'recorder, bundle and audit paths must be below the workspace'
[[ "$workspace" =~ ^[-_/A-Za-z0-9.]+$ && "$bundle" =~ ^[-_/A-Za-z0-9.]+$ \
    && "$existing_audit" =~ ^[-_/A-Za-z0-9.]+$ ]] \
    || fail 'existing-controller authenticated paths must not require shell escaping'
[[ "$audit_root" != "$existing_audit" && "$audit_root" != "$existing_audit"/* \
    && "$existing_audit" != "$audit_root"/* ]] || fail 'audit roots must not overlap'
[[ -x "$recorder_source" && "$(sha256_file "$recorder_source")" == "$recorder_sha" ]] \
    || fail 'recorder hash mismatch'
recorder=$(pin_recorder "$recorder_source")
verify_script_hash "$capture_script" "$expected_capture_script_sha"
verify_script_hash "$prepass_script" "$expected_prepass_script_sha"
verify_script_hash "$final_script" "$expected_final_script_sha"
verify_script_hash "$sweep_script" "$expected_sweep_script_sha"
verify_script_hash "$helper_script" "$expected_helper_script_sha"

# Import the already-reviewed bundle, inventory, final-validation and
# collect-all functions. Its normal main is guarded when sourced.
configured_sweep_concurrency=$sweep_concurrency
configured_final_outer_lock=$final_outer_lock
source "$helper_script" "$workspace" "$bundle" "$bundle_trust_sha" "$runner_sha" \
    unused-seed2 unused-seed3 unused-seed4 "$audit_root"
sweep_concurrency=$configured_sweep_concurrency
final_outer_lock=$configured_final_outer_lock
verify_bundle "$bundle"
capture_bundle=$(pin_capture_bundle)
mkdir -p -- "$audit_root"
exec {controller_fd}>"$audit_root/controller.lock"
flock -n "$controller_fd" || fail "another onward controller owns $audit_root"
corpus_root="$workspace/parity-save-replays/60s-random-input"
mkdir -p -- "$corpus_root"
exec {campaign_controller_fd}>"$corpus_root/.schema16-onward-controller.lock"
flock -n "$campaign_controller_fd" \
    || fail 'another authenticated onward controller owns the shared corpus ladder'
initialize_or_verify_provenance
verify_existing_controller_provenance

if verify_prior_proof_gate; then
    write_phase "complete-existing-seed$exact_prior_seed-exact"
    printf '%s existing seed %s corpus is already exact; no onward capture needed\n' \
        "$(date -Is)" "$exact_prior_seed"
    exit 0
fi

# Variables consumed by the imported final/collect-all implementation.
seed2=unused-seed2
seed3=unused-seed3
seed4=unused-seed4
write_phase begin-onward-after-three-complete-nonexact-proofs
controller_loop
