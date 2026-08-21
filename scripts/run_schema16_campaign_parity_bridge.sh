#!/usr/bin/env bash
set -u

# Follow the currently active schema-16 ladder campaign. The local parity
# sweep and remote mirror intentionally share one audit directory namespace:
# run_parity_release_sweep.sh derives identical status keys from paths below
# the local and remote workspace roots.

workspace=${SCHEMA16_BRIDGE_WORKSPACE:-/home/phire/robinhood}
corpus_root=${SCHEMA16_BRIDGE_CORPUS_ROOT:-$workspace/parity-save-replays/60s-random-input}
runner=${SCHEMA16_BRIDGE_RUNNER:?SCHEMA16_BRIDGE_RUNNER must name a pinned parity runner}
audit_dir=${SCHEMA16_BRIDGE_AUDIT_DIR:-$workspace/parity-save-replays/audits/schema16-ladder}
permanent_eof_dir=${SCHEMA16_BRIDGE_PERMANENT_EOF_DIR:-$workspace/parity-save-replays/audits/permanent-eof}
local_concurrency=${SCHEMA16_BRIDGE_LOCAL_CONCURRENCY:-2}
local_slot_dir=${SCHEMA16_BRIDGE_LOCAL_SLOT_DIR:-$workspace/tmp/schema16-ladder-local-slots}
poll_seconds=${SCHEMA16_BRIDGE_POLL_SECONDS:-10}
ladder_session=${SCHEMA16_BRIDGE_LADDER_SESSION:-schema16-corpus-ladder}
ssh_config=${SCHEMA16_BRIDGE_SSH_CONFIG:-$workspace/tmp/ssh_config}
remote_host=${SCHEMA16_BRIDGE_REMOTE_HOST:-robin-worker}
remote_root=${SCHEMA16_BRIDGE_REMOTE_ROOT:-/srv/robinhood}
remote_audit=${SCHEMA16_BRIDGE_REMOTE_AUDIT:-$remote_root/audits/schema16-ladder}
manifest=${SCHEMA16_BRIDGE_MANIFEST:-$workspace/tmp/schema16-ladder-remote.manifest}

if [[ ! -x "$runner" ]]; then
    printf 'error: parity runner is not executable: %s\n' "$runner" >&2
    exit 2
fi
if [[ ! -r "$ssh_config" ]]; then
    printf 'error: SSH config is not readable: %s\n' "$ssh_config" >&2
    exit 2
fi
if [[ ! "$local_concurrency" =~ ^[0-9]+$ ]]; then
    printf 'error: SCHEMA16_BRIDGE_LOCAL_CONCURRENCY must be a non-negative integer\n' >&2
    exit 2
fi
if [[ ! "$poll_seconds" =~ ^[1-9][0-9]*$ ]]; then
    printf 'error: SCHEMA16_BRIDGE_POLL_SECONDS must be positive\n' >&2
    exit 2
fi

mkdir -p -- \
    "$audit_dir/status" \
    "$audit_dir/logs" \
    "$permanent_eof_dir/status" \
    "$local_slot_dir" \
    "${manifest%/*}"
cd -- "$workspace"

promote_exact_eof_statuses() {
    local status destination value

    while IFS= read -r -d '' status; do
        IFS= read -r value <"$status" || continue
        [[ "$value" == 0 ]] || continue
        destination="$permanent_eof_dir/status/${status##*/}"
        [[ -e "$destination" ]] || cp -- "$status" "$destination"
    done < <(find "$audit_dir/status" -type f -name '*.status' -print0)
}

seed_audit_from_permanent_eof() {
    # Exact EOF is monotonic campaign evidence.  A later runner audits only
    # unseen or previously nonzero traces, so permanent zeros intentionally
    # replace any stale nonzero result in the runner-specific namespace.
    rsync -a "$permanent_eof_dir/status/" "$audit_dir/status/"
    rsync -a -e "ssh -F $ssh_config" \
        "$permanent_eof_dir/status/" \
        "$remote_host:$remote_audit/status/"
}

ladder_is_running() {
    tmux has-session -t "$ladder_session" 2>/dev/null
}

active_campaign() {
    local env_file state schema seed best_seed=-1 best_campaign=

    while IFS= read -r -d '' env_file; do
        [[ ! -f "${env_file%/campaign.env}/parity-verdict.env" ]] || continue
        state=$(sed -n 's/^CAMPAIGN_STATE=//p' "$env_file" | head -n 1)
        schema=$(sed -n 's/^PARITY_TRACE_SCHEMA=//p' "$env_file" | head -n 1)
        seed=$(sed -n 's/^PARITY_INPUT_SEED_BASE=//p' "$env_file" | head -n 1)
        [[ "$state" == recording_autonomous_ladder && "$schema" == 16 ]] || continue
        [[ "$seed" =~ ^[0-9]+$ ]] || continue
        if (( seed > best_seed )); then
            best_seed=$seed
            best_campaign=${env_file%/campaign.env}
        fi
    done < <(
        find "$corpus_root" -mindepth 2 -maxdepth 2 -type f \
            -path '*/schema16-seed*/campaign.env' -print0 2>/dev/null
    )

    [[ -n "$best_campaign" ]] || return 1
    printf '%s\n' "$best_campaign"
}

write_complete_trace_manifest() {
    local campaign=$1 destination=$2 trace marker temporary
    temporary=$(mktemp "${destination}.tmp.XXXXXX") || return 1

    while IFS= read -r -d '' trace; do
        marker=${trace%-session-*}.complete
        [[ -f "$marker" || -f "$trace.complete" ]] || continue
        printf '%s\n' "$trace" >>"$temporary"
    done < <(find "$campaign/traces" -type f -name '*.jsonl.zst' -print0 | sort -z)

    mv -f -- "$temporary" "$destination"
}

run_local_watch() {
    local campaign previous_campaign= snapshot shard sweep_pids sweep_pid
    snapshot="$audit_dir/traces.snapshot"

    while ladder_is_running; do
        if ! campaign=$(active_campaign); then
            sleep "$poll_seconds"
            continue
        fi
        if [[ "$campaign" != "$previous_campaign" ]]; then
            printf '%s local sweep following %s\n' "$(date -Is)" "$campaign"
            previous_campaign=$campaign
        fi
        if ! write_complete_trace_manifest "$campaign" "$snapshot"; then
            printf 'warning: could not refresh local snapshot for %s\n' "$campaign" >&2
            sleep "$poll_seconds"
            continue
        fi

        sweep_pids=()
        for ((shard = 0; shard < local_concurrency; shard += 1)); do
            PARITY_SWEEP_GLOBAL_CONCURRENCY="$local_concurrency" \
                PARITY_SWEEP_SLOT_DIR="$local_slot_dir" \
                scripts/run_parity_release_sweep.sh \
                    "$workspace" "$audit_dir" "$runner" "$shard" "$local_concurrency" &
            sweep_pids+=("$!")
        done
        for sweep_pid in "${sweep_pids[@]}"; do
            wait "$sweep_pid" || true
        done
        sleep "$poll_seconds"
    done
}

run_remote_sync() {
    local campaign previous_campaign= relative_manifest remote_snapshot_command
    relative_manifest="$manifest.relative"
    remote_snapshot_command=$(printf \
        'cat > %q && mv -f %q %q' \
        "$remote_audit/traces.snapshot.tmp" \
        "$remote_audit/traces.snapshot.tmp" \
        "$remote_audit/traces.snapshot")

    if ! ssh -F "$ssh_config" "$remote_host" \
        "mkdir -p '$remote_audit/status' '$remote_audit/logs'"; then
        printf 'error: unable to initialize remote audit directory\n' >&2
        return 1
    fi
    promote_exact_eof_statuses
    seed_audit_from_permanent_eof || return 1

    while ladder_is_running; do
        if ! campaign=$(active_campaign); then
            sleep "$poll_seconds"
            continue
        fi
        if [[ "$campaign" != "$previous_campaign" ]]; then
            printf '%s remote mirror following %s\n' "$(date -Is)" "$campaign"
            previous_campaign=$campaign
        fi
        if ! write_complete_trace_manifest "$campaign" "$manifest"; then
            printf 'warning: could not refresh remote manifest for %s\n' "$campaign" >&2
            sleep "$poll_seconds"
            continue
        fi

        sed "s#^$workspace/##" "$manifest" >"$relative_manifest.tmp"
        mv -f -- "$relative_manifest.tmp" "$relative_manifest"
        if [[ -s "$relative_manifest" ]]; then
            if rsync -aR --files-from="$relative_manifest" \
                -e "ssh -F $ssh_config" \
                "$workspace/" "$remote_host:$remote_root/"; then
                sed "s#^#$remote_root/#" "$relative_manifest" \
                    | ssh -F "$ssh_config" "$remote_host" "$remote_snapshot_command" \
                    || printf 'warning: could not publish remote snapshot\n' >&2
            else
                printf 'warning: remote trace mirror failed; retrying\n' >&2
            fi
        fi

        # Remote workers create the log before cache construction and replay
        # finish.  Re-sync existing files so a prefix copied mid-run converges
        # to the completed diagnostic instead of remaining permanently stale.
        if (( local_concurrency == 0 )); then
            # With remote-only validation these files are authoritative and
            # may repair an older audit artifact created by a prior overlap.
            rsync -a --exclude='*.tmp.*' -e "ssh -F $ssh_config" \
                "$remote_host:$remote_audit/status/" "$audit_dir/status/" \
                || printf 'warning: could not pull remote statuses\n' >&2
        else
            rsync -a --ignore-existing --exclude='*.tmp.*' -e "ssh -F $ssh_config" \
                "$remote_host:$remote_audit/status/" "$audit_dir/status/" \
                || printf 'warning: could not pull remote statuses\n' >&2
        fi
        rsync -a --exclude='*.tmp.*' -e "ssh -F $ssh_config" \
            "$remote_host:$remote_audit/logs/" "$audit_dir/logs/" \
            || printf 'warning: could not pull remote logs\n' >&2
        promote_exact_eof_statuses
        seed_audit_from_permanent_eof \
            || printf 'warning: could not seed permanent EOF statuses\n' >&2
        sleep "$poll_seconds"
    done
}

local_watch_pid=
cleanup() {
    if [[ -n "$local_watch_pid" ]]; then
        kill "$local_watch_pid" 2>/dev/null || true
        wait "$local_watch_pid" 2>/dev/null || true
    fi
}
trap cleanup EXIT INT TERM

printf 'schema16 campaign bridge started: audit=%s remote=%s:%s\n' \
    "$audit_dir" "$remote_host" "$remote_audit"
if (( local_concurrency > 0 )); then
    run_local_watch &
    local_watch_pid=$!
else
    printf 'local parity validation disabled; remote audit is authoritative\n'
fi
run_remote_sync
if [[ -n "$local_watch_pid" ]]; then
    wait "$local_watch_pid" 2>/dev/null || true
fi
local_watch_pid=
printf 'schema16 campaign bridge stopped with ladder tmux %s\n' "$ladder_session"
