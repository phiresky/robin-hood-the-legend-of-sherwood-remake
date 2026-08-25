#!/usr/bin/env bash
set -euo pipefail

# Claim and attest independent replay records against one SQLite authority.
# Local mode reaches that authority through SSH and stages only the claimed
# native artifact. Remote mode reads the authoritative artifact in place.

if [[ ${1:-} == --receive-result ]]; then
    (( $# == 3 )) || exit 2
    audit=$2 relative=$3
    [[ "$relative" =~ ^(results/[0-9a-f]{64}|attempts/[0-9a-f]{64}\.[0-9]{8}T[0-9]{6}Z\.[0-9]+)$ ]] \
        || { printf 'unsafe result path\n' >&2; exit 2; }
    mkdir -p -- "$audit" "$audit/${relative%/*}"
    archive=$(mktemp "$audit/.incoming.tar.XXXXXX")
    incoming=$(mktemp -d "$audit/.incoming.result.XXXXXX")
    cleanup_receive() { rm -f -- "$archive"; rm -rf -- "$incoming"; }
    trap cleanup_receive EXIT
    cat >"$archive"
    mapfile -t members < <(tar -tf "$archive" | LC_ALL=C sort)
    expected=(
        "$relative/"
        "$relative/MANIFEST.sha256"
        "$relative/attestation.env"
        "$relative/log"
        "$relative/status"
        "$relative/trace.path"
    )
    mapfile -t expected < <(printf '%s\n' "${expected[@]}" | LC_ALL=C sort)
    [[ "$(printf '%s\n' "${members[@]}")" == "$(printf '%s\n' "${expected[@]}")" ]] \
        || { printf 'result archive has unexpected members\n' >&2; exit 2; }
    tar -xf "$archive" -C "$incoming" --no-same-owner --no-same-permissions
    result="$incoming/$relative"
    [[ -z "$(find "$result" -mindepth 1 -maxdepth 1 ! -type f -print -quit)" ]] \
        || { printf 'result archive has non-file members\n' >&2; exit 2; }
    (cd -- "$result" && sha256sum --strict -c MANIFEST.sha256 >/dev/null)
    destination="$audit/$relative"
    if [[ -e "$destination" ]]; then
        (cd -- "$destination" && sha256sum --strict -c MANIFEST.sha256 >/dev/null)
        cmp -s -- "$result/MANIFEST.sha256" "$destination/MANIFEST.sha256" \
            || { printf 'conflicting result already published\n' >&2; exit 2; }
    else
        mv -- "$result" "$destination"
    fi
    printf '%s\n' "$destination"
    exit 0
fi

if [[ ${1:-} == --publish-stop ]]; then
    (( $# == 5 )) || exit 2
    audit=$2 identity=$3 status=$4 finished=$5
    [[ "$identity" =~ ^[0-9a-f]{64}$ && "$status" != *$'\n'* ]] || exit 2
    mkdir -p -- "$audit"
    exec {stop_fd}>"$audit/STOP.lock"
    flock "$stop_fd"
    if [[ ! -e "$audit/STOP.env" ]]; then
        temporary=$(mktemp "$audit/STOP.env.tmp.XXXXXX")
        {
            printf 'FAILED_UTC=%s\n' "$finished"
            printf 'IDENTITY_SHA256=%s\nSTATUS=%s\n' "$identity" "$status"
        } >"$temporary"
        mv -- "$temporary" "$audit/STOP.env"
    fi
    exit 0
fi

if [[ ${1:-} == --stop-open ]]; then
    (( $# == 2 )) || exit 2
    [[ ! -e "$2/STOP.env" && ! -e "$2/BATCH_FATAL.env" ]]
    exit
fi

if (( $# != 9 )); then
    printf 'usage: %s MODE WORKSPACE BUNDLE TRUST_SHA RUNNER_SHA REMOTE_AUDIT CORPUS WORKER_ID CONTROL_HOST\n' "$0" >&2
    exit 2
fi

mode=$1
workspace_arg=$2
bundle_arg=$3
bundle_trust_sha=${4,,}
runner_sha=${5,,}
remote_audit=$6
corpus=$7
worker_id=$8
control_host=$9

[[ "$mode" == local || "$mode" == remote ]] || { printf 'invalid mode\n' >&2; exit 2; }
[[ "$worker_id" =~ ^[A-Za-z0-9_.:-]+$ ]] || { printf 'invalid worker id\n' >&2; exit 2; }
[[ "$corpus" != /* && "$corpus" != *..* && "$corpus" != *$'\n'* ]] \
    || { printf 'invalid corpus root\n' >&2; exit 2; }
[[ "$bundle_trust_sha" =~ ^[0-9a-f]{64}$ && "$runner_sha" =~ ^[0-9a-f]{64}$ ]] \
    || { printf 'runner hashes must be SHA-256 digests\n' >&2; exit 2; }

lease_seconds=${DISTRIBUTED_REPLAY_LEASE_SECONDS:-120}
heartbeat_seconds=${DISTRIBUTED_REPLAY_HEARTBEAT_SECONDS:-20}
timeout_seconds=${DISTRIBUTED_REPLAY_TIMEOUT_SECONDS:-900}
oneshot=${DISTRIBUTED_REPLAY_ONESHOT:-0}
nice_level=${DISTRIBUTED_REPLAY_NICE_LEVEL:-0}
ionice_class=${DISTRIBUTED_REPLAY_IONICE_CLASS:-2}
ionice_level=${DISTRIBUTED_REPLAY_IONICE_LEVEL:-0}
remote_workspace=${DISTRIBUTED_REPLAY_REMOTE_WORKSPACE:-/srv/robinhood}
remote_db=${DISTRIBUTED_REPLAY_STATE_DB:-$remote_workspace/parity-save-replays/replay-state.sqlite3}
remote_tool=${DISTRIBUTED_REPLAY_STATE_TOOL:-$remote_workspace/scripts/replay_state_db.py}
remote_script=${DISTRIBUTED_REPLAY_REMOTE_SCRIPT:-$remote_workspace/scripts/run_distributed_replay_worker.sh}
ssh_bin=${DISTRIBUTED_REPLAY_SSH:-ssh}
ssh_config=${DISTRIBUTED_REPLAY_SSH_CONFIG:-}
exact_marker='parity trace matched every recorded frame'

[[ "$lease_seconds" =~ ^[0-9]+$ && "$heartbeat_seconds" =~ ^[0-9]+$ \
    && "$timeout_seconds" =~ ^[0-9]+$ ]] || { printf 'invalid timing value\n' >&2; exit 2; }
(( lease_seconds >= heartbeat_seconds * 3 && heartbeat_seconds > 0 && timeout_seconds > 0 )) \
    || { printf 'lease must be at least three heartbeat intervals\n' >&2; exit 2; }
[[ "$oneshot" == 0 || "$oneshot" == 1 ]] || exit 2

workspace=$(realpath -e -- "$workspace_arg")
bundle=$(realpath -e -- "$bundle_arg")
data_dir=$(realpath -e -- "$workspace/datadirs/fullgame_linux")
audit_identity=$(printf 'distributed-replay-worker-audit-v1\nAUDIT=%s\nCORPUS=%s\n' \
    "$remote_audit" "$corpus" | sha256sum)
audit_identity=${audit_identity%% *}
# Recovery must never inspect evidence produced for a different authenticated
# runner or audit. Worker IDs are intentionally reusable across rollouts.
scratch="$workspace/.agent-debug/distributed-replay-worker/$bundle_trust_sha/$audit_identity/$worker_id"
local_audit="$scratch/audit"
[[ "$mode" == local ]] || local_audit=$remote_audit
mkdir -p -- "$scratch/work" "$local_audit/results" "$local_audit/attempts"

sha256_file() { local value; value=$(sha256sum -- "$1"); printf '%s\n' "${value%% *}"; }

remote_exec() {
    if [[ "$mode" == remote ]]; then
        "$@"
        return
    fi
    local command= part argument
    for argument in "$@"; do
        printf -v part '%q' "$argument"
        command+="${command:+ }$part"
    done
    if [[ -n "$ssh_config" ]]; then
        "$ssh_bin" -F "$ssh_config" "$control_host" "$command"
    else
        "$ssh_bin" "$control_host" "$command"
    fi
}

# replay_state_db.py places DATABASE immediately after the subcommand.
db_command() {
    local command=$1; shift
    remote_exec python3 "$remote_tool" "$command" "$remote_db" "$@"
}

stop_open() { remote_exec "$remote_script" --stop-open "$remote_audit"; }

stop_or_transport_status() {
    local status=0
    stop_open || status=$?
    case $status in
        0) return 0 ;;
        1) return 1 ;;
        *) return 3 ;;
    esac
}

publish_infrastructure_stop() {
    local status=$1 identity finished
    identity=$(printf 'distributed-replay-infrastructure-v1\nWORKER=%s\nSTATUS=%s\n' \
        "$worker_id" "$status" | sha256sum)
    identity=${identity%% *}
    finished=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    remote_exec "$remote_script" --publish-stop "$remote_audit" "$identity" \
        "$status" "$finished" >/dev/null 2>&1 || true
}

verify_bundle() {
    [[ -x "$bundle/original_parity_replay" && -x "$bundle/original_parity_replay.remote" \
        && -x "$bundle/lib/ld-linux-x86-64.so.2" && -f "$bundle/LOADER_LIST.txt" \
        && -f "$bundle/SHA256SUMS" && -f "$bundle/LIB_SHA256SUMS" ]] || return 1
    [[ "$(sha256_file "$bundle/original_parity_replay")" == "$runner_sha" ]] || return 1
    (cd -- "$bundle" && sha256sum --strict -c SHA256SUMS >/dev/null \
        && sha256sum --strict -c LIB_SHA256SUMS >/dev/null) || return 1
    [[ -z "$(find "$bundle" -type l -print -quit)" ]] || return 1
    diff -u -- \
        <(find "$bundle/lib" -type f -printf 'lib/%P\n' | LC_ALL=C sort) \
        <(sed -n 's/^[0-9a-fA-F]\{64\} [ *]//p' "$bundle/LIB_SHA256SUMS" | LC_ALL=C sort) \
        >/dev/null || return 1
    diff -u -- \
        <(printf '%s\n' LIB_SHA256SUMS LOADER_LIST.txt PROVENANCE.txt \
            original_parity_replay original_parity_replay.remote | LC_ALL=C sort) \
        <(sed -n 's/^[0-9a-fA-F]\{64\} [ *]//p' "$bundle/SHA256SUMS" | LC_ALL=C sort) \
        >/dev/null || return 1
    diff -u -- \
        <(printf '%s\n' LIB_SHA256SUMS LOADER_LIST.txt PROVENANCE.txt SHA256SUMS \
            original_parity_replay original_parity_replay.remote | LC_ALL=C sort) \
        <(find "$bundle" -maxdepth 1 -type f -printf '%f\n' | LC_ALL=C sort) \
        >/dev/null || return 1
    mapfile -t protocol_values < <(sed -n 's/^NATIVE_CONVERSION_PROTOCOL=//p' \
        "$bundle/PROVENANCE.txt")
    [[ ${#protocol_values[@]} == 1 && ${protocol_values[0]} == 2 ]] || return 1
    local main_sha lib_sha actual
    main_sha=$(sha256_file "$bundle/SHA256SUMS")
    lib_sha=$(sha256_file "$bundle/LIB_SHA256SUMS")
    actual=$(printf 'schema16-runner-bundle-v1\nSHA256SUMS=%s\nLIB_SHA256SUMS=%s\n' \
        "$main_sha" "$lib_sha" | sha256sum)
    [[ "${actual%% *}" == "$bundle_trust_sha" ]] || return 1

    # LOADER_LIST.txt and SHA256SUMS remain byte-for-byte deployment metadata
    # from the authoritative remote bundle. This separate ephemeral check
    # proves those same raw/lib bytes are safely relocatable on this host; it
    # never regenerates or rewrites authenticated metadata.
    local loader_output resolved
    loader_output=$("$bundle/lib/ld-linux-x86-64.so.2" \
        --library-path "$bundle/lib" --list "$bundle/original_parity_replay") || return 1
    while IFS= read -r resolved; do
        [[ -n "$resolved" ]] || continue
        resolved=$(realpath -e -- "$resolved") || return 1
        [[ "$resolved" == "$bundle/lib/"* ]] || return 1
    done < <(printf '%s\n' "$loader_output" | sed -n \
        -e 's/.* => \([^ ]*\) .*/\1/p' \
        -e 's/^[[:space:]]*\(\/[^ ]*ld-linux[^ ]*\) .*/\1/p')
}

if ! verify_bundle; then
    printf 'runner bundle authentication failed\n' >&2
    publish_infrastructure_stop infrastructure-bundle-authentication
    exit 2
fi
bundle_manifest_sha=$(sha256_file "$bundle/SHA256SUMS")
bundle_lib_manifest_sha=$(sha256_file "$bundle/LIB_SHA256SUMS")
runner_wrapper_sha=$(sha256_file "$bundle/original_parity_replay.remote")
data_dir_path_sha=$(printf '%s' "$data_dir" | sha256sum); data_dir_path_sha=${data_dir_path_sha%% *}

fetch_remote_file() {
    local logical=$1 suffix=$2 destination=$3
    remote_exec python3 -c '
import pathlib,sys
root=pathlib.Path(sys.argv[1]).resolve()
logical=sys.argv[2]
if logical.startswith("/") or ".." in pathlib.PurePosixPath(logical).parts:
    raise SystemExit(2)
path=pathlib.Path(str(root / logical)+sys.argv[3]).resolve(strict=True)
path.relative_to(root)
with path.open("rb") as source:
    while chunk := source.read(1024*1024):
        sys.stdout.buffer.write(chunk)
' "$remote_workspace" "$logical" "$suffix" >"$destination"
}

upload_result() {
    local result=$1 relative=${1#"$local_audit"/}
    if [[ "$mode" == remote ]]; then
        printf '%s\n' "$result"
        return
    fi
    tar -C "$local_audit" --no-recursion -cf - "$relative" \
        "$relative/MANIFEST.sha256" "$relative/attestation.env" "$relative/log" \
        "$relative/status" "$relative/trace.path" \
        | remote_exec "$remote_script" --receive-result "$remote_audit" "$relative"
}

import_result() {
    local remote_result=$1
    db_command import-result "$remote_result" --audit-root "$remote_audit" \
        --workspace "$remote_workspace" --host "${HOSTNAME:-$worker_id}"
}

recover_local_results() {
    [[ "$mode" == local ]] || return 0
    local result remote_result status finished identity
    while IFS= read -r -d '' result; do
        remote_result=$(upload_result "$result") || return 1
        import_result "$remote_result" >/dev/null || return 1
        if [[ "$result" == "$local_audit/results/"* ]]; then
            status=$(<"$result/status")
            if [[ "$status" != 0 ]]; then
                identity=${result##*/}
                finished=$(sed -n 's/^FINISHED_UTC=//p' "$result/attestation.env")
                remote_exec "$remote_script" --publish-stop "$remote_audit" \
                    "$identity" "$status" "$finished" || return 1
            fi
        fi
    done < <(find "$local_audit/results" "$local_audit/attempts" -mindepth 1 \
        -maxdepth 1 -type d -print0 | LC_ALL=C sort -z)
}

json_field() {
    python3 -c 'import json,sys; value=json.load(sys.stdin).get(sys.argv[1]); print("" if value is None else value)' "$1"
}

publish_result() {
    local logical=$1 marker_logical=$2 marker_path=$3 native=$4 native_pre=$5 native_post=$6
    local command_status=$7 marker_count=$8 result_status=$9 started=${10} finished=${11}
    local private_log=${12}
    local logical_sha identity relative result temporary log_sha marker_sha
    logical_sha=$(printf '%s' "$logical" | sha256sum); logical_sha=${logical_sha%% *}
    identity=$(printf 'schema16-incremental-eof-v1\nLOGICAL=%s\nNATIVE_SHA256=%s\nRUNNER_BUNDLE_TRUST_SHA256=%s\n' \
        "$logical" "$native_pre" "$bundle_trust_sha" | sha256sum); identity=${identity%% *}
    if [[ "$result_status" == aborted-* ]]; then
        relative="attempts/$identity.$(date -u +%Y%m%dT%H%M%SZ).$$"
    else
        relative="results/$identity"
    fi
    result="$local_audit/$relative"
    mkdir -p -- "$local_audit/.result-locks"
    exec {result_lock_fd}>"$local_audit/.result-locks/$identity.lock"
    flock "$result_lock_fd"
    if [[ -e "$result" ]]; then
        (cd -- "$result" && sha256sum --strict -c MANIFEST.sha256 >/dev/null)
        [[ "$(<"$result/trace.path")" == "$logical" ]] || return 1
        grep -Fqx -- "NATIVE_SHA256_PRE=$native_pre" "$result/attestation.env" || return 1
        grep -Fqx -- "RUNNER_BUNDLE_TRUST_SHA256=$bundle_trust_sha" \
            "$result/attestation.env" || return 1
        printf '%s\n' "$result"
        return
    fi
    temporary=$(mktemp -d "$local_audit/.result.tmp.XXXXXX")
    printf '%s\n' "$logical" >"$temporary/trace.path"
    mv -- "$private_log" "$temporary/log"
    printf '%s\n' "$result_status" >"$temporary/status"
    log_sha=$(sha256_file "$temporary/log")
    marker_sha=$(sha256_file "$marker_path" 2>/dev/null || printf missing)
    {
        printf 'FORMAT=schema16-incremental-eof-v1\n'
        printf 'STARTED_UTC=%s\nFINISHED_UTC=%s\n' "$started" "$finished"
        printf 'LOGICAL_PATH_SHA256=%s\nIDENTITY_SHA256=%s\n' "$logical_sha" "$identity"
        printf 'COMPLETION_MARKER=%s\nCOMPLETION_MARKER_SHA256=%s\n' \
            "$marker_logical" "$marker_sha"
        printf 'NATIVE_SHA256_PRE=%s\nNATIVE_SHA256_POST=%s\n' "$native_pre" "$native_post"
        printf 'RUNNER_RAW_SHA256=%s\nRUNNER_BUNDLE_TRUST_SHA256=%s\n' "$runner_sha" "$bundle_trust_sha"
        printf 'RUNNER_BUNDLE_MANIFEST_SHA256=%s\nRUNNER_LIB_MANIFEST_SHA256=%s\n' \
            "$bundle_manifest_sha" "$bundle_lib_manifest_sha"
        printf 'RUNNER_WRAPPER_SHA256=%s\nDATA_DIR=%s\nDATA_DIR_PATH_SHA256=%s\n' \
            "$runner_wrapper_sha" "$data_dir" "$data_dir_path_sha"
        printf 'COMMAND=original_parity_replay.remote --no-auto-dump LOGICAL_TRACE\n'
        printf 'TIMEOUT_SECONDS=%s\nNICE_LEVEL=%s\nIONICE_CLASS=%s\nIONICE_LEVEL=%s\n' \
            "$timeout_seconds" "$nice_level" "$ionice_class" "$ionice_level"
        printf 'RUNNER_COMMAND_STATUS=%s\nEXACT_EOF_MARKER_COUNT=%s\nLOG_SHA256=%s\n' \
            "$command_status" "$marker_count" "$log_sha"
    } >"$temporary/attestation.env"
    (cd -- "$temporary" && sha256sum attestation.env log status trace.path >MANIFEST.sha256)
    mkdir -p -- "${result%/*}"
    mv -- "$temporary" "$result"
    printf '%s\n' "$result"
}

stop_heartbeat() {
    if [[ -n ${heartbeat_pid:-} ]]; then
        kill "$heartbeat_pid" 2>/dev/null || true
        wait "$heartbeat_pid" 2>/dev/null || true
        heartbeat_pid=
    fi
}
worker_exit() {
    local status=$?
    trap - EXIT
    stop_heartbeat
    if (( status == 3 )); then
        publish_infrastructure_stop infrastructure-transport-or-lease-loss
    fi
    exit "$status"
}
trap worker_exit EXIT
trap 'stop_heartbeat; exit 130' INT TERM HUP

recover_local_results || { printf 'pending evidence recovery failed\n' >&2; exit 3; }

while true; do
    stop_status=0
    stop_or_transport_status || stop_status=$?
    [[ $stop_status == 0 ]] || exit "$stop_status"
    claim_json=$(db_command claim-work replay "$worker_id" --lease-seconds "$lease_seconds" \
        --runner-trust "$bundle_trust_sha" --logical-root "$corpus") \
        || { printf 'claim transport failed\n' >&2; exit 3; }
    [[ "$claim_json" != null ]] || exit 0
    mapfile -t fields < <(printf '%s' "$claim_json" | python3 -c '
import json,sys
v=json.load(sys.stdin)
for key in ("work_id","claim_token","logical_path","completion_marker","source_sha256"):
 print("" if v.get(key) is None else v[key])
')
    work_id=${fields[0]} token=${fields[1]} logical=${fields[2]} marker_logical=${fields[3]} native_expected=${fields[4]}
    [[ "$logical" == "$corpus"/* && -n "$marker_logical" && "$marker_logical" == "$corpus"/* \
        && "$native_expected" =~ ^[0-9a-f]{64}$ ]] || { printf 'unsafe claim payload\n' >&2; exit 2; }

    existing=$(db_command exact-evidence-key "$logical" --runner-trust "$bundle_trust_sha" \
        --native-sha256 "$native_expected") || exit 3
    existing_key=$(printf '%s' "$existing" | json_field evidence_key)
    if [[ -n "$existing_key" ]]; then
        db_command complete-work "$token" exact_eof --evidence-key "$existing_key" >/dev/null
        [[ "$oneshot" == 0 ]] || exit 0
        continue
    fi

    claim_dir="$scratch/work/$work_id"
    rm -rf -- "$claim_dir"
    mkdir -p -- "$claim_dir/root/${logical%/*}"
    lost_flag="$claim_dir/lease-lost"
    peer_flag="$claim_dir/peer-stop"
    heartbeat_pid=
    (
        while sleep "$heartbeat_seconds"; do
            if ! db_command renew-work "$token" --lease-seconds "$lease_seconds" >/dev/null; then
                : >"$lost_flag"; exit
            fi
            stop_status=0
            stop_or_transport_status || stop_status=$?
            if [[ $stop_status == 1 ]]; then : >"$peer_flag"; exit; fi
            if [[ $stop_status != 0 ]]; then : >"$lost_flag"; exit; fi
        done
    ) &
    heartbeat_pid=$!

    if [[ "$mode" == local ]]; then
        native="$claim_dir/root/$logical.parity.bitcode.zst"
        marker="$claim_dir/root/$marker_logical"
        if ! fetch_remote_file "$logical" .parity.bitcode.zst "$native" \
            || ! fetch_remote_file "$marker_logical" '' "$marker"; then
            : >"$lost_flag"
        fi
        logical_run="$claim_dir/root/$logical"
    else
        native="$workspace/$logical.parity.bitcode.zst"
        marker="$workspace/$marker_logical"
        logical_run="$workspace/$logical"
    fi
    native_pre=$(sha256_file "$native" 2>/dev/null || printf missing)
    footer=$(tail -c 36 -- "$native" 2>/dev/null | head -c 16 || true)
    if [[ ! -e "$lost_flag" ]] && { [[ "$native_pre" != "$native_expected" \
        || "$footer" != RHPRTRACEFOOTER! || ! -f "$marker" ]]; }; then
        publish_infrastructure_stop integrity-native-artifact
        exit 1
    fi
    stop_status=0
    stop_or_transport_status || stop_status=$?
    if [[ $stop_status == 1 ]]; then : >"$peer_flag"; fi
    if [[ $stop_status != 0 && $stop_status != 1 ]]; then : >"$lost_flag"; fi
    if [[ -e "$lost_flag" || -e "$peer_flag" ]]; then
        stop_heartbeat
        [[ -e "$peer_flag" ]] && exit 1 || exit 3
    fi

    private_log=$(mktemp "$claim_dir/runner.log.XXXXXX")
    started=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    setsid timeout --foreground --signal=TERM --kill-after=10s "${timeout_seconds}s" \
        nice -n "$nice_level" ionice -c "$ionice_class" -n "$ionice_level" env \
        ROBINHOOD_DATA_DIR="$data_dir" "$bundle/original_parity_replay.remote" \
        --no-auto-dump "$logical_run" >"$private_log" 2>&1 &
    runner_pid=$!
    while kill -0 "$runner_pid" 2>/dev/null; do
        if [[ -e "$lost_flag" || -e "$peer_flag" ]]; then
            kill -TERM -- "-$runner_pid" 2>/dev/null || true
            break
        fi
        [[ -r /proc/$runner_pid/stat \
            && "$(awk '{print $3}' /proc/$runner_pid/stat 2>/dev/null)" == Z ]] && break
        sleep 1
    done
    command_status=0
    wait "$runner_pid" || command_status=$?
    finished=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    stop_heartbeat
    native_post=$(sha256_file "$native" 2>/dev/null || printf missing)
    marker_count=$(grep -Fxc -- "$exact_marker" "$private_log" || true)
    if [[ -e "$lost_flag" ]]; then result_status=aborted-lease-loss
    elif [[ -e "$peer_flag" ]]; then result_status=aborted-peer-failure
    elif [[ "$native_post" != "$native_pre" ]]; then result_status=integrity-native-changed
    elif (( command_status != 0 )); then result_status=$command_status
    elif (( marker_count != 1 )); then result_status=integrity-eof-marker
    else result_status=0
    fi
    result=$(publish_result "$logical" "$marker_logical" "$marker" "$native" "$native_pre" "$native_post" \
        "$command_status" "$marker_count" "$result_status" "$started" "$finished" "$private_log")
    remote_result=$(upload_result "$result") || exit 3
    import_json=$(import_result "$remote_result") || exit 3
    evidence_key=$(printf '%s' "$import_json" | json_field evidence_key)
    [[ "$evidence_key" =~ ^[0-9a-f]{64}$ ]] || exit 2

    if [[ "$result_status" == aborted-* ]]; then
        [[ "$result_status" == aborted-peer-failure ]] && exit 1 || exit 3
    fi
    if [[ "$result_status" == 0 ]]; then outcome=exact_eof
    elif [[ "$command_status" == 124 ]]; then outcome=timeout
    elif grep -Eq 'first parity divergence|divergent frames' "$result/log"; then outcome=mismatch
    elif [[ "$result_status" == integrity-* ]]; then outcome=integrity_error
    else outcome=crash
    fi
    if [[ "$result_status" == integrity-* ]]; then
        identity=${result##*/}
        remote_exec "$remote_script" --publish-stop "$remote_audit" "$identity" \
            "$result_status" "$finished" || exit 3
    fi
    db_command complete-work "$token" "$outcome" --evidence-key "$evidence_key" >/dev/null || exit 3
    [[ "$result_status" != integrity-* ]] || exit 1
    [[ "$oneshot" == 0 ]] || exit 0
done
