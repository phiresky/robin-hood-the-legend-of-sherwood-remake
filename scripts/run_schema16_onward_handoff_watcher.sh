#!/usr/bin/env bash
set -euo pipefail

if (( $# != 10 )); then
    printf 'usage: %s WORKSPACE EXISTING_SESSION EXISTING_AUDIT ONWARD_AUDIT HANDOFF_AUDIT RECORDER RECORDER_SHA RUNNER_BUNDLE BUNDLE_TRUST_SHA RUNNER_SHA\n' "$0" >&2
    exit 2
fi

workspace=$1
existing_session=$2
existing_audit=$3
onward_audit=$4
handoff_audit=$5
recorder=$6
recorder_sha=${7,,}
bundle=$8
bundle_trust_sha=${9,,}
runner_sha=${10,,}
poll_seconds=${SCHEMA16_HANDOFF_POLL_SECONDS:-300}

expected_controller_sha=1b7d16f416a19fdcfce68feecce524deb0e0c97ef66331d852cffd292736efa9
expected_helper_sha=79c0d5c9d770812be5ae541ca12d017f21dbb2b9c30e480bde56cfb90a8d27a7
expected_capture_sha=0e99a6a935335761eef740faee097708282e7bd4584c0d1275f07beb539442d6
expected_prepass_sha=d2b7fd1eb29a921655a3aec49617ca5c48e70cbf990f94593490ba17631a074b
expected_final_sha=229146eebcf0e3b09bf2987d2af17917e3addaa813697a905893ce935549c6b9
expected_sweep_sha=e7a1c769a18b76a69c3473f68caa38a6b741c002e826bff97c106e7fdf746cfb

fail() { printf 'error: %s\n' "$*" >&2; exit 2; }
sha256_file() { local value; value=$(sha256sum -- "$1") || return 1; printf '%s\n' "${value%% *}"; }
write_atomic() {
    local destination=$1 temporary
    temporary=$(mktemp "${destination}.tmp.XXXXXX") || return 1
    if ! cat >"$temporary" || ! mv -f -- "$temporary" "$destination"; then
        rm -f -- "$temporary"
        return 1
    fi
}
read_one() {
    local file=$1 key=$2
    local -a values=()
    mapfile -t values < <(sed -n "s/^${key}=//p" "$file")
    (( ${#values[@]} == 1 )) || return 1
    printf '%s\n' "${values[0]}"
}
verify_hash() {
    local path=$1 expected=$2
    [[ -f "$path" && "$(sha256_file "$path")" == "$expected" ]] \
        || fail "deployed hash mismatch: $path"
}
write_state() {
    {
        printf 'UPDATED_UTC=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
        printf 'PHASE=%s\n' "$1"
    } | write_atomic "$handoff_audit/state.env" || fail 'cannot publish handoff state'
}
verify_deployment() {
    verify_hash "$workspace/scripts/run_schema16_onward_corpus_controller.sh" "$expected_controller_sha"
    verify_hash "$workspace/scripts/run_schema16_existing_corpora_orchestrator.sh" "$expected_helper_sha"
    verify_hash "$workspace/original-code/scripts/capture_parity_save_replays.sh" "$expected_capture_sha"
    verify_hash "$workspace/scripts/run_native_conversion_prepass.sh" "$expected_prepass_sha"
    verify_hash "$workspace/scripts/run_schema16_final_validation.sh" "$expected_final_sha"
    verify_hash "$workspace/scripts/run_parity_release_sweep.sh" "$expected_sweep_sha"
    verify_hash "$recorder" "$recorder_sha"
    verify_hash "$bundle/original_parity_replay" "$runner_sha"
    [[ -f "$bundle/SHA256SUMS" && -f "$bundle/LIB_SHA256SUMS" ]]
    main_manifest_sha=$(sha256_file "$bundle/SHA256SUMS")
    lib_manifest_sha=$(sha256_file "$bundle/LIB_SHA256SUMS")
    actual_trust=$(printf 'schema16-runner-bundle-v1\nSHA256SUMS=%s\nLIB_SHA256SUMS=%s\n' \
        "$main_manifest_sha" "$lib_manifest_sha" | sha256sum)
    actual_trust=${actual_trust%% *}
    [[ "$actual_trust" == "$bundle_trust_sha" ]] || fail 'runner bundle trust mismatch'
    (cd -- "$bundle" && sha256sum --strict -c SHA256SUMS \
        && sha256sum --strict -c LIB_SHA256SUMS) >/dev/null \
        || fail 'runner bundle content checksum failure'
}

[[ "$poll_seconds" =~ ^[1-9][0-9]{0,3}$ ]] || fail 'poll seconds must be 1 through 9999'
for path in "$workspace" "$existing_audit" "$onward_audit" "$handoff_audit" "$recorder" "$bundle"; do
    [[ "$path" =~ ^/[-_/A-Za-z0-9.]+$ ]] || fail "noncanonical handoff path: $path"
done
[[ -d "$workspace" && -d "$existing_audit" && -x "$recorder" && -d "$bundle" ]] \
    || fail 'handoff inputs are incomplete'
[[ "$existing_audit" == "$workspace"/* && "$onward_audit" == "$workspace"/* \
    && "$handoff_audit" == "$workspace"/* && "$recorder" == "$workspace"/* \
    && "$bundle" == "$workspace"/* ]] || fail 'handoff inputs must remain below workspace'
[[ "$recorder_sha" =~ ^[0-9a-f]{64}$ && "$bundle_trust_sha" =~ ^[0-9a-f]{64}$ \
    && "$runner_sha" =~ ^[0-9a-f]{64}$ ]] || fail 'handoff trust values must be SHA-256'

mkdir -p -- "$handoff_audit"
exec {handoff_fd}>"$handoff_audit/handoff.lock"
flock -n "$handoff_fd" || fail 'another process owns this handoff audit'
verify_deployment

tmux has-session -t "$existing_session" 2>/dev/null \
    || fail "required initial existing-corpora session is absent: $existing_session"
mapfile -t pane_commands < <(tmux list-panes -t "$existing_session" -F '#{pane_start_command}')
(( ${#pane_commands[@]} == 1 )) || fail 'existing-corpora session must have exactly one pane'
pane_command=${pane_commands[0]}
[[ "$pane_command" == *"scripts/run_schema16_existing_corpora_orchestrator.sh"* \
    && "$pane_command" == *"$existing_audit"* && "$pane_command" == *"$bundle"* \
    && "$pane_command" == *"$bundle_trust_sha"* && "$pane_command" == *"$runner_sha"* \
    && "$pane_command" == *'SCHEMA16_ORCH_PREPASS_JOBS=5'* \
    && "$pane_command" == *'SCHEMA16_ORCH_SWEEP_CONCURRENCY=8'* ]] \
    || fail 'existing-corpora pane command does not match authenticated P5 campaign'
pane_command_sha=$(printf '%s' "$pane_command" | sha256sum); pane_command_sha=${pane_command_sha%% *}

evidence_candidate="$handoff_audit/evidence.candidate.env"
{
    printf 'WORKSPACE=%s\n' "$workspace"
    printf 'EXISTING_SESSION=%s\nEXISTING_SESSION_COMMAND_SHA256=%s\n' "$existing_session" "$pane_command_sha"
    printf 'EXISTING_AUDIT=%s\nONWARD_AUDIT=%s\nHANDOFF_AUDIT=%s\n' \
        "$existing_audit" "$onward_audit" "$handoff_audit"
    printf 'HANDOFF_SCRIPT_SHA256=%s\n' "$(sha256_file "$0")"
    printf 'ONWARD_CONTROLLER_SHA256=%s\nEXISTING_HELPER_SHA256=%s\n' \
        "$expected_controller_sha" "$expected_helper_sha"
    printf 'CAPTURE_SCRIPT_SHA256=%s\nPREPASS_SCRIPT_SHA256=%s\nFINAL_SCRIPT_SHA256=%s\nSWEEP_SCRIPT_SHA256=%s\n' \
        "$expected_capture_sha" "$expected_prepass_sha" "$expected_final_sha" "$expected_sweep_sha"
    printf 'RECORDER=%s\nRECORDER_SHA256=%s\nRUNNER_BUNDLE=%s\n' "$recorder" "$recorder_sha" "$bundle"
    printf 'RUNNER_BUNDLE_TRUST_SHA256=%s\nRUNNER_SHA256=%s\n' "$bundle_trust_sha" "$runner_sha"
    printf 'POLL_SECONDS=%s\nCAPTURE_JOBS=8\nCAPTURE_CONVERT_JOBS=3\nPREPASS_JOBS=5\nSWEEP_CONCURRENCY=8\n' "$poll_seconds"
} >"$evidence_candidate" || fail 'cannot stage handoff evidence'
if [[ -f "$handoff_audit/evidence.env" ]]; then
    cmp -s -- "$evidence_candidate" "$handoff_audit/evidence.env" \
        || fail 'handoff evidence differs from immutable prior launch'
    rm -f -- "$evidence_candidate"
else
    mv -- "$evidence_candidate" "$handoff_audit/evidence.env" \
        || fail 'cannot publish handoff evidence'
fi

write_state waiting-for-existing-session-exit
while tmux has-session -t "$existing_session" 2>/dev/null; do
    sleep "$poll_seconds"
done

write_state validating-existing-terminal-proof
phase=$(read_one "$existing_audit/state.env" PHASE) \
    || fail 'existing audit has no unique terminal PHASE'
case "$phase" in
complete-all-exact) expected_nonexact=0 ;;
complete-with-[1-3]-nonexact-corpora)
    expected_nonexact=${phase#complete-with-}; expected_nonexact=${expected_nonexact%-nonexact-corpora}
    ;;
*) fail "existing audit stopped in unsupported phase: $phase" ;;
esac

mapfile -t summaries < <(find "$existing_audit" -maxdepth 1 -type f \
    -name 'final-seed*.env' -printf '%f\n' | LC_ALL=C sort)
[[ "${summaries[*]}" == 'final-seed2.env final-seed3.env final-seed4.env' ]] \
    || fail 'existing audit does not contain exactly the three required final summaries'
actual_nonexact=0
for seed in 2 3 4; do
    summary="$existing_audit/final-seed${seed}.env"
    [[ "$(read_one "$summary" SEED)" == "$seed" ]] || fail "seed$seed summary has wrong SEED"
    exact=$(read_one "$summary" EXACT_PARITY) || fail "seed$seed summary lacks EXACT_PARITY"
    [[ "$exact" == 0 || "$exact" == 1 ]] || fail "seed$seed summary has invalid EXACT_PARITY"
    [[ -n "$(read_one "$summary" AUDIT)" ]] || fail "seed$seed summary lacks AUDIT"
    (( exact == 0 )) && actual_nonexact=$((actual_nonexact + 1))
done
(( actual_nonexact == expected_nonexact )) \
    || fail "terminal phase reports $expected_nonexact nonexact corpora but summaries report $actual_nonexact"

verify_deployment
write_state exec-onward-controller
exec env -i PATH=/usr/bin:/bin LC_ALL=C TZ=UTC HOME=/root \
    SCHEMA16_ONWARD_CAPTURE_JOBS=8 \
    SCHEMA16_ONWARD_CAPTURE_CONVERT_JOBS=3 \
    SCHEMA16_ONWARD_PREPASS_JOBS=5 \
    SCHEMA16_ONWARD_SWEEP_CONCURRENCY=8 \
    "$workspace/scripts/run_schema16_onward_corpus_controller.sh" \
        "$workspace" "$recorder" "$recorder_sha" "$bundle" "$bundle_trust_sha" \
        "$runner_sha" "$existing_audit" "$onward_audit"
