#!/usr/bin/env bash
set -euo pipefail

# Restart-safe scale-out for an existing schema-16 ladder campaign. Capture
# locks are host-local, so hosts own disjoint save shards. Remote publications
# are first copied into an incoming directory, checked, and atomically moved.

workspace=${SCHEMA16_DISTRIBUTED_WORKSPACE:-/home/phire/robinhood}
ssh_config=${SCHEMA16_DISTRIBUTED_SSH_CONFIG:-$workspace/tmp/ssh_config}
remote_host=${SCHEMA16_DISTRIBUTED_REMOTE_HOST:-robin-worker}
remote_root=${SCHEMA16_DISTRIBUTED_REMOTE_ROOT:-/srv/robinhood}
shard_count=${SCHEMA16_DISTRIBUTED_SHARDS:-4}
local_jobs=${SCHEMA16_DISTRIBUTED_LOCAL_JOBS:-10}
remote_jobs=${SCHEMA16_DISTRIBUTED_REMOTE_JOBS:-7}
ladder_session=${SCHEMA16_DISTRIBUTED_LADDER_SESSION:-schema16-corpus-ladder}
local_session=${SCHEMA16_DISTRIBUTED_LOCAL_SESSION:-schema16-capture-local}
remote_session=${SCHEMA16_DISTRIBUTED_REMOTE_SESSION:-schema16-capture-remote}
collector_session=${SCHEMA16_DISTRIBUTED_COLLECTOR_SESSION:-schema16-capture-collector}
remote_warmup_session=${SCHEMA16_DISTRIBUTED_REMOTE_WARMUP_SESSION:-parity-warmup}
poll_seconds=${SCHEMA16_DISTRIBUTED_POLL_SECONDS:-30}
audit_dir=${SCHEMA16_DISTRIBUTED_AUDIT_DIR:-}
recorder_sha=${SCHEMA16_DISTRIBUTED_RECORDER_SHA:-02485baafdcfb285039a763f1c3bc4d9f2534d97bf26b3724fbb5c24fca0ccba}
recorder_rel="binaries/parity-recorders/$recorder_sha/robin"

usage() {
    printf 'usage: %s prepare|migrate|collect|watch|status [CAMPAIGN]\n' "$0"
    printf '       %s capture-shard CAMPAIGN SHARD SHARDS JOBS\n' "$0"
}

require_uint() {
    local name=$1 value=$2
    if [[ ! "$value" =~ ^[1-9][0-9]*$ ]]; then
        printf 'error: %s must be a positive integer\n' "$name" >&2
        exit 2
    fi
}

campaign_value() {
    local campaign=$1 key=$2
    sed -n "s/^${key}=//p" "$campaign/campaign.env" | head -n 1
}

find_campaign() {
    local requested=${1:-} env_file best= best_seed=-1 seed
    if [[ -n "$requested" ]]; then
        [[ "$requested" == /* ]] || requested="$workspace/$requested"
        printf '%s\n' "${requested%/}"
        return
    fi
    while IFS= read -r -d '' env_file; do
        seed=$(campaign_value "${env_file%/campaign.env}" PARITY_INPUT_SEED_BASE)
        [[ "$seed" =~ ^[0-9]+$ ]] || continue
        if (( seed > best_seed )); then
            best_seed=$seed
            best=${env_file%/campaign.env}
        fi
    done < <(find "$workspace/parity-save-replays/60s-random-input" \
        -mindepth 2 -maxdepth 2 -type f -path '*/schema16-seed*/campaign.env' -print0)
    [[ -n "$best" ]] || { printf 'error: no schema16 seed campaign found\n' >&2; exit 2; }
    printf '%s\n' "$best"
}

validate_campaign() {
    local campaign=$1 schema binary actual_sha
    [[ -f "$campaign/campaign.env" ]] || { printf 'error: missing %s/campaign.env\n' "$campaign" >&2; exit 2; }
    schema=$(campaign_value "$campaign" PARITY_TRACE_SCHEMA)
    [[ "$schema" == 16 ]] || { printf 'error: campaign schema is %s, not 16\n' "$schema" >&2; exit 2; }
    binary="$workspace/$recorder_rel"
    [[ -x "$binary" ]] || { printf 'error: missing recorder %s\n' "$binary" >&2; exit 2; }
    actual_sha=$(sha256sum -- "$binary"); actual_sha=${actual_sha%% *}
    [[ "$actual_sha" == "$recorder_sha" ]] || { printf 'error: recorder SHA mismatch\n' >&2; exit 2; }
}

ssh_worker() {
    ssh -F "$ssh_config" "$remote_host" "$@"
}

prepare_remote() {
    local campaign=$1 remote_campaign relative
    relative=${campaign#"$workspace"/}
    remote_campaign="$remote_root/$relative"
    ssh_worker "mkdir -p '$remote_root/original-code/scripts' '$remote_root/original-code/runtime-i386' '$remote_root/reference-saves' '$remote_root/${recorder_rel%/*}' '$remote_root/scripts' '$remote_campaign'"
    rsync -a -e "ssh -F $ssh_config" \
        "$workspace/original-code/runtime-i386/" \
        "$remote_host:$remote_root/original-code/runtime-i386/"
    rsync -a -e "ssh -F $ssh_config" \
        "$workspace/reference-saves/" "$remote_host:$remote_root/reference-saves/"
    rsync -a -e "ssh -F $ssh_config" \
        "$workspace/original-code/scripts/capture_parity_save_replays.sh" \
        "$remote_host:$remote_root/original-code/scripts/"
    rsync -a -e "ssh -F $ssh_config" \
        "$workspace/scripts/run_schema16_distributed_capture.sh" \
        "$remote_host:$remote_root/scripts/"
    rsync -a -e "ssh -F $ssh_config" "$workspace/$recorder_rel" \
        "$remote_host:$remote_root/$recorder_rel"
    ssh_worker "test \"\$(sha256sum '$remote_root/$recorder_rel' | cut -d' ' -f1)\" = '$recorder_sha'; '$remote_root/original-code/runtime-i386/ld-linux.so.2' --library-path '$remote_root/original-code/runtime-i386:$remote_root/original-code/runtime-i386/pulseaudio' --list '$remote_root/$recorder_rel' >/dev/null"
    printf 'remote capture runtime ready: %s:%s\n' "$remote_host" "$remote_root"
}

capture_shard() {
    local campaign=$1 shard=$2 shards=$3 jobs=$4 root binary seed replays loader library_dir
    root=${SCHEMA16_DISTRIBUTED_WORKSPACE:-$workspace}
    [[ "$campaign" == /* ]] || campaign="$root/$campaign"
    seed=$(campaign_value "$campaign" PARITY_INPUT_SEED_BASE)
    replays=$(campaign_value "$campaign" PARITY_RANDOM_REPLAYS)
    binary="$root/$recorder_rel"
    loader=${SCHEMA16_DISTRIBUTED_LOADER:-}
    if [[ -n "$loader" ]]; then
        library_dir=${SCHEMA16_DISTRIBUTED_LIBRARY_DIR:-$root/original-code/runtime-i386}
    else
        # The local host has the matching contemporary i386 glibc.  Mixing
        # its loader with the checked-in legacy libc produces GLIBC_PRIVATE
        # symbol failures; the bundled runtime is only paired with its
        # explicit loader on workers that need it.
        library_dir=${SCHEMA16_DISTRIBUTED_LIBRARY_DIR:-/lib/i386-linux-gnu}
    fi
    env \
        PARITY_TRACE_SCHEMA=16 PARITY_RANDOM_REPLAYS="$replays" PARITY_FRAMES=1500 \
        PARITY_INPUT_SEED_BASE="$seed" PARITY_SEED=1 \
        SHERWOOD_LIMIT=30 SHERWOOD_SAMPLE_SEED=1 \
        SHARD_COUNT="$shards" SHARD_INDEX="$shard" CAPTURE_JOBS="$jobs" \
        COMPRESS=1 ZSTD_THREADS=1 ZSTD_LEVEL=16 HEADFUL=0 SKIP_BUILD=1 \
        WATCHDOG_SECONDS=2700 ROBIN_BINARY="$binary" \
        ROBIN_LOADER="$loader" ROBIN_LIBRARY_DIR="$library_dir" \
        ROBINHOOD_DATA_DIR="$root/datadirs/fullgame_linux" \
        "$root/original-code/scripts/capture_parity_save_replays.sh" \
        "$root/reference-saves" "$campaign" "$root/datadirs/fullgame_linux"
    : >"$campaign/.distributed-shard-$shard.complete"
}

sync_completed_to_remote() {
    local campaign=$1 relative remote_campaign
    relative=${campaign#"$workspace"/}; remote_campaign="$remote_root/$relative"
    rsync -a --exclude='.capture*' --exclude='incomplete-traces/' \
        -e "ssh -F $ssh_config" "$campaign/" "$remote_host:$remote_campaign/"
    ssh_worker "touch '$remote_campaign/.distributed-remote-start'"
}

start_local_shard() {
    local campaign=$1 local_command
    [[ ! -e "$campaign/.distributed-shard-0.complete" ]] || return 0
    tmux has-session -t "$local_session" 2>/dev/null && return 0
    local_command=$(printf 'cd %q && exec env SCHEMA16_DISTRIBUTED_WORKSPACE=%q bash scripts/run_schema16_distributed_capture.sh capture-shard %q 0 %q %q >> %q 2>&1' \
        "$workspace" "$workspace" "$campaign" "$shard_count" "$local_jobs" "$campaign/capture-distributed-local.log")
    tmux new-session -d -s "$local_session" "$local_command"
}

start_remote_shard() {
    local campaign=$1 shard=$2 relative remote_campaign remote_command session
    relative=${campaign#"$workspace"/}; remote_campaign="$remote_root/$relative"
    session="$remote_session-$shard"
    if ssh_worker "test -e '$remote_campaign/.distributed-shard-$shard.complete' || tmux has-session -t '$session' 2>/dev/null"; then
        return 0
    fi
    remote_command=$(printf 'cd %q && exec env SCHEMA16_DISTRIBUTED_WORKSPACE=%q SCHEMA16_DISTRIBUTED_LOADER=%q bash scripts/run_schema16_distributed_capture.sh capture-shard %q %q %q %q >> %q 2>&1' \
        "$remote_root" "$remote_root" "$remote_root/original-code/runtime-i386/ld-linux.so.2" "$remote_campaign" "$shard" "$shard_count" "$remote_jobs" "$remote_campaign/capture-distributed-remote-$shard.log")
    ssh_worker "tmux new-session -d -s '$session' $(printf %q "$remote_command")"
}

start_shards() {
    local campaign=$1 shard
    rm -f -- "$campaign/.capture.drain"
    start_local_shard "$campaign"
    for ((shard=1; shard<shard_count; shard+=1)); do
        start_remote_shard "$campaign" "$shard"
    done
}

restart_external_ladder() {
    local command
    if [[ -z "$audit_dir" ]]; then
        audit_dir=$(find "$workspace/parity-save-replays/audits" -mindepth 1 -maxdepth 1 \
            -type d -name 'autonomous-watch-*' -printf '%T@ %p\n' 2>/dev/null \
            | sort -n | tail -n 1 | cut -d' ' -f2-)
    fi
    [[ -n "$audit_dir" ]] || { printf 'error: set SCHEMA16_DISTRIBUTED_AUDIT_DIR\n' >&2; exit 2; }
    command=$(printf 'cd %q && exec env SCHEMA16_LADDER_RECORDER=%q SCHEMA16_LADDER_RECORDER_SHA=%q SCHEMA16_LADDER_AUDIT_DIR=%q SCHEMA16_LADDER_FIRST_SEED_BASE=%q SCHEMA16_LADDER_CAPTURE_EXTERNALLY=1 SCHEMA16_LADDER_POLL_SECONDS=%q bash scripts/run_schema16_corpus_ladder.sh >> %q 2>&1' \
        "$workspace" "$workspace/$recorder_rel" "$recorder_sha" \
        "$audit_dir" \
        "$(campaign_value "$1" PARITY_INPUT_SEED_BASE)" "$poll_seconds" \
        "$workspace/parity-save-replays/60s-random-input/schema16-corpus-ladder-distributed.log")
    tmux new-session -d -s "$ladder_session" "$command"
}

migrate() {
    local campaign=$1 reservations
    prepare_remote "$campaign"
    : >"$campaign/.capture.drain"
    printf 'draining current local recorder at replay boundaries...\n'
    while :; do
        reservations=$(find "$campaign/.capture-reservations" -type f -name '*.reserve' 2>/dev/null | wc -l)
        (( reservations == 0 )) && break
        sleep 2
    done
    tmux kill-session -t "$ladder_session" 2>/dev/null || true
    sync_completed_to_remote "$campaign"
    # Warmups are expendable spare-capacity work. Keep the live fresh-corpus
    # validator, but free the remaining remote cores for capture.
    ssh_worker "tmux kill-session -t '$remote_warmup_session' 2>/dev/null || true"
    start_shards "$campaign"
    restart_external_ladder "$campaign"
    start_collector "$campaign"
    printf 'distributed capture started: local shard 0/%s; remote shards 1..%s; zstd=16\n' "$shard_count" "$((shard_count - 1))"
}

start_collector() {
    local campaign=$1 command
    tmux has-session -t "$collector_session" 2>/dev/null && return 0
    command=$(printf 'cd %q && exec env SCHEMA16_DISTRIBUTED_WORKSPACE=%q SCHEMA16_DISTRIBUTED_AUDIT_DIR=%q SCHEMA16_DISTRIBUTED_SHARDS=%q SCHEMA16_DISTRIBUTED_LOCAL_JOBS=%q SCHEMA16_DISTRIBUTED_REMOTE_JOBS=%q bash scripts/run_schema16_distributed_capture.sh watch %q >> %q 2>&1' \
        "$workspace" "$workspace" "$audit_dir" "$shard_count" "$local_jobs" \
        "$remote_jobs" "$campaign" "$campaign/capture-distributed-collector.log")
    tmux new-session -d -s "$collector_session" "$command"
}

collect() {
    local campaign=$1 relative remote_campaign incoming available pending imported imported_tmp
    local lock_file lock_fd rel file destination
    relative=${campaign#"$workspace"/}; remote_campaign="$remote_root/$relative"
    incoming="$campaign/.distributed-incoming"
    imported="$campaign/.distributed-imported-files"
    lock_file="$campaign/.distributed-collector.lock"
    mkdir -p -- "$incoming" "$workspace/tmp"
    exec {lock_fd}>"$lock_file"
    flock "$lock_fd"
    available=$(mktemp "$workspace/tmp/schema16-remote-available.XXXXXX")
    pending=$(mktemp "$workspace/tmp/schema16-remote-pending.XXXXXX")
    imported_tmp=$(mktemp "$workspace/tmp/schema16-remote-imported.XXXXXX")
    touch "$imported"

    # Inventory the whole remote-origin suffix in one SSH round trip.  The
    # start marker was created after the pre-migration local corpus sync, so
    # only files authored by remote shards compare newer.  A persistent local
    # manifest turns later scans into a cheap set difference instead of
    # retransferring and revalidating every earlier result.
    ssh_worker "cd '$remote_root' && { \
        find '${relative}/traces' -type f \
            \( -name '*.jsonl.zst' -o -name '*.complete' \) \
            -newer '${relative}/.distributed-remote-start' -printf '%p\\n'; \
        find '${relative}/logs' -type f -name '*.log' \
            -newer '${relative}/.distributed-remote-start' -printf '%p\\n'; \
    } | sort -u" >"$available"
    sort -u "$imported" >"$imported_tmp"
    mv -f -- "$imported_tmp" "$imported"
    comm -23 "$available" "$imported" >"$pending"
    if [[ ! -s "$pending" ]]; then
        rm -f -- "$available" "$pending"
        exec {lock_fd}>&-
        return 0
    fi

    rsync -aR --files-from="$pending" -e "ssh -F $ssh_config" \
        "$remote_host:$remote_root/" "$incoming/"
    while IFS= read -r rel; do
        file="$incoming/$rel"; destination="$workspace/$rel"
        if [[ "$file" == *.jsonl.zst ]]; then zstd -t -q --long=31 -- "$file"; fi
        mkdir -p -- "${destination%/*}"
        if [[ -e "$destination" ]]; then
            cmp -s -- "$file" "$destination" || { printf 'error: collision differs: %s\n' "$destination" >&2; exit 1; }
            rm -f -- "$file"
        else
            mv -- "$file" "$destination"
        fi
        printf '%s\n' "$rel" >>"$imported"
    done <"$pending"
    sort -u "$imported" >"$imported_tmp"
    mv -f -- "$imported_tmp" "$imported"
    rm -f -- "$available" "$pending"
    exec {lock_fd}>&-
}

status() {
    local campaign=$1 expected complete remote_complete remote_up=0 shard
    expected=$(campaign_value "$campaign" EXPECTED_LOGICAL_REPLAYS)
    complete=$(find "$campaign/traces" -type f -name '*.complete' | wc -l)
    remote_complete=$(ssh_worker "find '$remote_root/${campaign#"$workspace"/}/traces' -type f -name '*.complete' 2>/dev/null | wc -l")
    for ((shard=1; shard<shard_count; shard+=1)); do
        if ssh_worker "tmux has-session -t '$remote_session-$shard' 2>/dev/null"; then
            remote_up=$((remote_up + 1))
        fi
    done
    printf 'local=%s/%s remote=%s local_session=%s remote_sessions=%s/%s\n' \
        "$complete" "$expected" "$remote_complete" \
        "$(tmux has-session -t "$local_session" 2>/dev/null && printf up || printf down)" \
        "$remote_up" "$((shard_count - 1))"
}

watch_capture() {
    local campaign=$1 expected complete shard
    expected=$(campaign_value "$campaign" EXPECTED_LOGICAL_REPLAYS)
    while true; do
        collect "$campaign"
        complete=$(find "$campaign/traces" -type f -name '*.complete' | wc -l)
        status "$campaign"
        (( complete < expected )) || return 0
        start_local_shard "$campaign"
        for ((shard=1; shard<shard_count; shard+=1)); do
            start_remote_shard "$campaign" "$shard"
        done
        sleep "$poll_seconds"
    done
}

require_uint SCHEMA16_DISTRIBUTED_SHARDS "$shard_count"
require_uint SCHEMA16_DISTRIBUTED_LOCAL_JOBS "$local_jobs"
require_uint SCHEMA16_DISTRIBUTED_REMOTE_JOBS "$remote_jobs"
(( shard_count >= 2 )) || { printf 'error: at least two shards are required\n' >&2; exit 2; }
(( local_jobs <= 10 && remote_jobs <= 10 )) || { printf 'error: recorder limits jobs to 10 per shard\n' >&2; exit 2; }

action=${1:-}; shift || true
if [[ "$action" == capture-shard ]]; then
    [[ $# == 4 ]] || { usage >&2; exit 2; }
    capture_shard "$@"
    exit
fi
[[ -r "$ssh_config" ]] || { printf 'error: missing SSH config %s\n' "$ssh_config" >&2; exit 2; }
campaign=$(find_campaign "${1:-}")
validate_campaign "$campaign"
case "$action" in
    prepare) prepare_remote "$campaign" ;;
    migrate) migrate "$campaign" ;;
    collect) collect "$campaign" ;;
    watch) watch_capture "$campaign" ;;
    status) status "$campaign" ;;
    *) usage >&2; exit 2 ;;
esac
