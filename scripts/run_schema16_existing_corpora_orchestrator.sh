#!/usr/bin/env bash
set -euo pipefail

# Normalize and validate an already-created ordered set of schema-16 corpora.
# This supervisor is intentionally serial: capture drains first, conversion
# prepasses share one authenticated protocol-2 bundle, and final validation
# obtains the global parity-runner lock. A semantic mismatch is collected over
# the rest of that corpus before the next corpus is attempted; an integrity or
# setup failure stops the supervisor immediately.

if (( $# != 8 )); then
    printf 'usage: %s WORKSPACE RUNNER_BUNDLE BUNDLE_TRUST_SHA RUNNER_SHA SEED2_CORPUS SEED3_CORPUS SEED4_CORPUS AUDIT_ROOT\n' "$0" >&2
    exit 2
fi

workspace_arg=$1
bundle_arg=$2
bundle_trust_sha=${3,,}
runner_sha=${4,,}
seed2_arg=$5
seed3_arg=$6
seed4_arg=$7
audit_root_arg=$8

poll_seconds=${SCHEMA16_ORCH_POLL_SECONDS:-300}
prepass_jobs=${SCHEMA16_ORCH_PREPASS_JOBS:-5}
prepass_timeout=${SCHEMA16_ORCH_PREPASS_TIMEOUT_SECONDS:-7200}
preflight_only=${SCHEMA16_ORCH_PREFLIGHT_ONLY:-0}
final_outer_lock=${SCHEMA16_FINAL_OUTER_LOCK:-/tmp/robin-parity-runner.lock}
sweep_concurrency=${SCHEMA16_ORCH_SWEEP_CONCURRENCY:-8}
p5_gate_attempts=${SCHEMA16_ORCH_P5_GATE_ATTEMPTS:-30}
p5_min_memory_kib=${SCHEMA16_ORCH_P5_MIN_MEMORY_KIB:-52428800}
p5_max_load1=${SCHEMA16_ORCH_P5_MAX_LOAD1:-16}
p5_max_memory_psi_avg10=${SCHEMA16_ORCH_P5_MAX_MEMORY_PSI_AVG10:-1}
p5_max_cpu_psi_avg60=${SCHEMA16_ORCH_P5_MAX_CPU_PSI_AVG60:-5}
p5_swap_sample_seconds=${SCHEMA16_ORCH_P5_SWAP_SAMPLE_SECONDS:-60}
p3_min_memory_kib=${SCHEMA16_ORCH_P3_MIN_MEMORY_KIB:-25165824}
seed3_shard_count=${SCHEMA16_ORCH_SEED3_SHARD_COUNT:-4}
seed3_session_prefix=${SCHEMA16_ORCH_SEED3_SESSION_PREFIX:-schema16-capture-seed3-sha7425511-20260824T085459Z-shard-}
seed3_log_prefix=${SCHEMA16_ORCH_SEED3_LOG_PREFIX:-capture-seed3-shard-}
seed4_prepass_session=${SCHEMA16_ORCH_SEED4_PREPASS_SESSION:-seed4-native-84d75548-20260824T100702Z}
seed4_prepass_audit=${SCHEMA16_ORCH_SEED4_PREPASS_AUDIT:-$workspace_arg/audits/seed4-native-conversion-84d75548-20260824T100702Z}
seed4_prepass_status=${SCHEMA16_ORCH_SEED4_PREPASS_STATUS:-$seed4_prepass_audit.session.status}

# These hashes bind the exact audited scripts deployed for this campaign.
expected_prepass_script_sha=d2b7fd1eb29a921655a3aec49617ca5c48e70cbf990f94593490ba17631a074b
expected_final_script_sha=229146eebcf0e3b09bf2987d2af17917e3addaa813697a905893ce935549c6b9
expected_sweep_script_sha=e7a1c769a18b76a69c3473f68caa38a6b741c002e826bff97c106e7fdf746cfb

# Fixed production metric sources. The source-only regression harness replaces
# these shell variables after loading the functions; executed controllers do
# not accept environment overrides for host evidence.
meminfo_path=/proc/meminfo
loadavg_path=/proc/loadavg
memory_psi_path=/proc/pressure/memory
cpu_psi_path=/proc/pressure/cpu
vmstat_path=/proc/vmstat

fail() {
    printf 'error: %s\n' "$*" >&2
    exit 2
}

sha256_file() {
    local result
    result=$(sha256sum -- "$1") || return 1
    printf '%s\n' "${result%% *}"
}

normalize_bounded_uint() {
    local value=$1 limit=$2
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

write_phase() {
    local phase=$1
    {
        printf 'UPDATED_UTC=%q\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
        printf 'PHASE=%q\n' "$phase"
    } | write_atomic "$audit_root/state.env" || fail 'cannot publish orchestrator phase'
    printf '%s phase=%s\n' "$(date -Is)" "$phase"
}

read_one_uint() {
    local file=$1 key=$2
    local -a values=()
    mapfile -t values < <(sed -n "s/^${key}=//p" "$file")
    (( ${#values[@]} == 1 )) && [[ "${values[0]}" =~ ^[0-9]+$ ]] \
        || fail "$file must contain exactly one unsigned $key"
    printf '%s\n' "${values[0]}"
}

read_one_value() {
    local file=$1 key=$2
    local -a values=()
    mapfile -t values < <(sed -n "s/^${key}=//p" "$file")
    (( ${#values[@]} == 1 )) || fail "$file must contain exactly one $key"
    printf '%s\n' "${values[0]}"
}

read_status_value() {
    local file=$1
    local -a values=()
    mapfile -t values <"$file" || return 1
    (( ${#values[@]} == 1 )) || return 1
    printf '%s\n' "${values[0]}"
}

verify_script() {
    local path=$1 expected=$2
    [[ -x "$path" ]] || fail "required script is not executable: $path"
    [[ "$(sha256_file "$path")" == "$expected" ]] \
        || fail "required script hash mismatch: $path"
}

runner_bundle_digest() {
    local bundle=$1 main_sha lib_sha result
    main_sha=$(sha256_file "$bundle/SHA256SUMS") || return 1
    lib_sha=$(sha256_file "$bundle/LIB_SHA256SUMS") || return 1
    result=$(printf 'schema16-runner-bundle-v1\nSHA256SUMS=%s\nLIB_SHA256SUMS=%s\n' \
        "$main_sha" "$lib_sha" | sha256sum) || return 1
    printf '%s\n' "${result%% *}"
}

verify_bundle() {
    local bundle=$1
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
    mapfile -t protocol < <(sed -n 's/^NATIVE_CONVERSION_PROTOCOL=//p' "$bundle/PROVENANCE.txt")
    [[ ${#protocol[@]} == 1 && "${protocol[0]}" == 2 ]] \
        || fail 'runner bundle does not authenticate native conversion protocol 2'
    (cd -- "$bundle" && sha256sum --strict -c SHA256SUMS \
        && sha256sum --strict -c LIB_SHA256SUMS) >/dev/null \
        || fail 'runner bundle checksum verification failed'
}

verify_campaign_metadata() {
    local campaign=$1 expected_seed=$2 schema expected
    [[ -d "$campaign/traces" && -f "$campaign/campaign.env" ]] \
        || fail "campaign is incomplete: $campaign"
    schema=$(read_one_uint "$campaign/campaign.env" PARITY_TRACE_SCHEMA)
    expected=$(read_one_uint "$campaign/campaign.env" EXPECTED_LOGICAL_REPLAYS)
    [[ "$schema" == 16 && "$expected" == 9720 ]] \
        || fail "campaign metadata is outside the expected schema16/9720 universe: $campaign"
    [[ "$(read_one_uint "$campaign/campaign.env" PARITY_INPUT_SEED_BASE)" == "$expected_seed" ]] \
        || fail "campaign has the wrong input seed base: $campaign"
}

paths_overlap() {
    local first=$1 second=$2
    [[ "$first" == "$second" || "$first" == "$second"/* || "$second" == "$first"/* ]]
}

initialize_or_verify_provenance() {
    local candidate="$audit_root/provenance.candidate.env"
    {
        printf 'WORKSPACE=%q\nRUNNER_BUNDLE=%q\n' "$workspace" "$bundle"
        printf 'RUNNER_SHA256=%s\nRUNNER_BUNDLE_TRUST_SHA256=%s\n' "$runner_sha" "$bundle_trust_sha"
        printf 'SEED2=%q\nSEED3=%q\nSEED4=%q\n' "$seed2" "$seed3" "$seed4"
        printf 'POLL_SECONDS=%s\nPREPASS_JOBS=%s\nPREPASS_TIMEOUT_SECONDS=%s\n' \
            "$poll_seconds" "$prepass_jobs" "$prepass_timeout"
        printf 'P5_GATE_ATTEMPTS=%s\nP5_MIN_MEMORY_KIB=%s\nP5_MAX_LOAD1=%s\n' \
            "$p5_gate_attempts" "$p5_min_memory_kib" "$p5_max_load1"
        printf 'P5_MAX_MEMORY_PSI_AVG10=%s\nP5_MAX_CPU_PSI_AVG60=%s\n' \
            "$p5_max_memory_psi_avg10" "$p5_max_cpu_psi_avg60"
        printf 'P5_REQUIRE_MEMORY_FULL_AVG10_ZERO=1\n'
        printf 'P5_SWAP_SAMPLE_SECONDS=%s\nP3_MIN_MEMORY_KIB=%s\n' \
            "$p5_swap_sample_seconds" "$p3_min_memory_kib"
        printf 'SWEEP_CONCURRENCY=%s\n' "$sweep_concurrency"
        printf 'ORCHESTRATOR_SCRIPT_SHA256=%s\nFINAL_OUTER_LOCK=%q\n' \
            "$orchestrator_script_sha" "$final_outer_lock"
        printf 'SEED3_SHARD_COUNT=%s\nSEED3_SESSION_PREFIX=%q\nSEED3_LOG_PREFIX=%q\n' \
            "$seed3_shard_count" "$seed3_session_prefix" "$seed3_log_prefix"
        printf 'SEED4_PREPASS_SESSION=%q\nSEED4_PREPASS_AUDIT=%q\nSEED4_PREPASS_STATUS=%q\n' \
            "$seed4_prepass_session" "$seed4_prepass_audit" "$seed4_prepass_status"
        printf 'PREPASS_SCRIPT_SHA256=%s\nFINAL_SCRIPT_SHA256=%s\nSWEEP_SCRIPT_SHA256=%s\n' \
            "$expected_prepass_script_sha" "$expected_final_script_sha" "$expected_sweep_script_sha"
    } >"$candidate" || fail 'cannot stage orchestrator provenance'
    if [[ -f "$audit_root/provenance.env" ]]; then
        cmp -s -- "$candidate" "$audit_root/provenance.env" \
            || fail 'orchestrator invocation differs from immutable prior provenance'
        rm -f -- "$candidate"
    else
        mv -- "$candidate" "$audit_root/provenance.env" \
            || fail 'cannot publish orchestrator provenance'
    fi
}

initialize_or_verify_seed3_epoch() {
    local epoch="$audit_root/seed3-controller-epoch.env" temporary shard session command digest size prefix_sha
    if [[ ! -f "$epoch" ]]; then
        temporary=$(mktemp "$epoch.tmp.XXXXXX") || fail 'cannot stage seed3 controller epoch'
        printf 'OBSERVED_UTC=%q\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >"$temporary"
        for ((shard=0; shard<seed3_shard_count; shard+=1)); do
            session="$seed3_session_prefix$shard"
            tmux has-session -t "$session" 2>/dev/null \
                || { rm -f -- "$temporary"; fail "cannot authenticate absent initial seed3 session: $session"; }
            mapfile -t pane_commands < <(tmux list-panes -t "$session" -F '#{pane_start_command}')
            (( ${#pane_commands[@]} == 1 )) \
                || { rm -f -- "$temporary"; fail "seed3 session does not have exactly one pane: $session"; }
            command=${pane_commands[0]}
            [[ "$command" == *"$seed3"* \
                && "$command" == *"SHARD_COUNT=$seed3_shard_count"* \
                && "$command" == *"SHARD_INDEX=$shard"* \
                && "$command" == *'7425511e6c9bac698804fe93bb4cf7af02740504b8f46d702f03e4bc2a385ff8/robin'* ]] \
                || { rm -f -- "$temporary"; fail "seed3 session command does not match authenticated shard: $session"; }
            digest=$(printf '%s' "$command" | sha256sum); digest=${digest%% *}
            size=$(stat -c %s -- "$seed3/${seed3_log_prefix}${shard}.log") \
                || { rm -f -- "$temporary"; fail "cannot stat seed3 shard $shard log"; }
            prefix_sha=$(head -c "$size" -- "$seed3/${seed3_log_prefix}${shard}.log" | sha256sum) \
                || { rm -f -- "$temporary"; fail "cannot hash seed3 shard $shard log prefix"; }
            prefix_sha=${prefix_sha%% *}
            printf 'SEED3_SESSION_%s=%q\nSEED3_SESSION_%s_COMMAND=%q\nSEED3_SESSION_%s_COMMAND_SHA256=%s\nSEED3_LOG_%s_INITIAL_SIZE=%s\nSEED3_LOG_%s_INITIAL_PREFIX_SHA256=%s\n' \
                "$shard" "$session" "$shard" "$command" "$shard" "$digest" \
                "$shard" "$size" "$shard" "$prefix_sha" >>"$temporary"
        done
        mv -- "$temporary" "$epoch" || fail 'cannot publish seed3 controller epoch'
    fi
    for ((shard=0; shard<seed3_shard_count; shard+=1)); do
        mapfile -t size_values < <(sed -n "s/^SEED3_LOG_${shard}_INITIAL_SIZE=//p" "$epoch")
        mapfile -t digest_values < <(sed -n "s/^SEED3_SESSION_${shard}_COMMAND_SHA256=//p" "$epoch")
        mapfile -t prefix_values < <(sed -n "s/^SEED3_LOG_${shard}_INITIAL_PREFIX_SHA256=//p" "$epoch")
        [[ ${#size_values[@]} == 1 && "${size_values[0]}" =~ ^[0-9]+$ \
            && ${#digest_values[@]} == 1 && "${digest_values[0]}" =~ ^[0-9a-f]{64}$ \
            && ${#prefix_values[@]} == 1 && "${prefix_values[0]}" =~ ^[0-9a-f]{64}$ ]] \
            || fail "seed3 controller epoch is malformed for shard $shard"
        session="$seed3_session_prefix$shard"
        if tmux has-session -t "$session" 2>/dev/null; then
            mapfile -t pane_commands < <(tmux list-panes -t "$session" -F '#{pane_start_command}')
            (( ${#pane_commands[@]} == 1 )) || fail "seed3 resumed session has multiple panes: $session"
            digest=$(printf '%s' "${pane_commands[0]}" | sha256sum); digest=${digest%% *}
            [[ "$digest" == "${digest_values[0]}" ]] \
                || fail "seed3 session command changed after epoch authentication: $session"
        fi
    done
}

verify_campaign_inventory() {
    local campaign=$1 expected=$2 work logical path base owner marker duplicate
    work=$(mktemp -d "${TMPDIR:-/tmp}/schema16-orchestrator-inventory.XXXXXX") \
        || fail 'cannot create inventory work directory'
    logical="$work/logical"; : >"$logical"; : >"$work/owners"; : >"$work/markers"
    while IFS= read -r -d '' path; do
        [[ "$path" != *$'\n'* ]] || { rm -rf -- "$work"; fail "newline in trace path: $path"; }
        printf '%s\n' "${path%.parity.bitcode.zst}" >>"$logical"
    done < <(find "$campaign/traces" -type f \
        \( -name '*.jsonl.zst' -o -name '*.jsonl.zst.parity.bitcode.zst' \) -print0)
    LC_ALL=C sort -u "$logical" -o "$logical"
    [[ "$(wc -l <"$logical")" == "$expected" ]] \
        || { rm -rf -- "$work"; fail "logical trace count is not $expected: $campaign"; }
    while IFS= read -r path; do
        base=${path##*/}; base=${base%.jsonl.zst}; owner=${base%%-session-*}
        marker="${path%/*}/$owner.complete"
        [[ -f "$marker" ]] || { rm -rf -- "$work"; fail "trace lacks completion marker: $path"; }
        printf '%s\n' "$marker" >>"$work/owners"
    done <"$logical"
    duplicate=$(LC_ALL=C sort "$work/owners" | uniq -d | head -n 1)
    [[ -z "$duplicate" ]] \
        || { rm -rf -- "$work"; fail "completion marker owns multiple traces: $duplicate"; }
    LC_ALL=C sort -u "$work/owners" -o "$work/owners"
    find "$campaign/traces" -type f -name '*.complete' -print \
        | LC_ALL=C sort -u >"$work/markers"
    cmp -s -- "$work/owners" "$work/markers" \
        || { rm -rf -- "$work"; fail "marker/trace ownership is not bijective: $campaign"; }
    rm -rf -- "$work"
}

target_processes() {
    local campaign=$1 process pid command
    for process in /proc/[0-9]*/cmdline; do
        pid=${process#/proc/}; pid=${pid%/cmdline}
        [[ "$pid" != "$$" ]] || continue
        command=$(tr '\0' ' ' <"$process" 2>/dev/null) || continue
        [[ "$command" == *"$campaign"* ]] || continue
        case "$command" in
            *' -PARITYTRACE '*|*original_parity_replay*' --convert '*|*capture_parity_save_replays.sh*|*run_schema16_distributed_capture.sh*|*rsync*)
                printf '%s\t%s\n' "$pid" "$command"
                ;;
        esac
    done
}

seed3_sessions_running() {
    local shard
    for ((shard=0; shard<seed3_shard_count; shard+=1)); do
        tmux has-session -t "$seed3_session_prefix$shard" 2>/dev/null && return 0
    done
    return 1
}

wait_for_seed3_sessions() {
    local complete reservations
    while seed3_sessions_running; do
        complete=$(find "$seed3/traces" -type f -name '*.complete' | wc -l)
        reservations=$(find "$seed3/.capture-reservations" -type f -name '*.reserve' 2>/dev/null | wc -l)
        printf '%s seed3 capture=%s/9720 reservations=%s; waiting for all shard sessions to exit naturally\n' \
            "$(date -Is)" "$complete" "$reservations"
        sleep "$poll_seconds"
    done
}

drain_and_verify_seed3() {
    local shard final_line active initial_size current_size epoch_log expected_prefix actual_prefix
    exec {admission_fd}>"$seed3/.capture-admission.lock"
    flock "$admission_fd"
    seed3_sessions_running && fail 'a seed3 shard restarted after the natural-exit wait'
    drain_tmp=$(mktemp "$seed3/.capture.drain.tmp.XXXXXX") || fail 'cannot stage seed3 drain marker'
    : >"$drain_tmp"
    mv -f -- "$drain_tmp" "$seed3/.capture.drain"
    exec {collector_fd}>"$seed3/.distributed-collector.lock"
    flock "$collector_fd"
    if find "$seed3/.capture-reservations" -type f -name '*.reserve' -print -quit 2>/dev/null | grep -q .; then
        fail 'seed3 still has a capture reservation after all shard sessions exited'
    fi
    active=$(target_processes "$seed3")
    [[ -z "$active" ]] || fail "seed3 still has an active writer:\n$active"
    verify_campaign_inventory "$seed3" 9720
    for ((shard=0; shard<seed3_shard_count; shard+=1)); do
        initial_size=$(sed -n "s/^SEED3_LOG_${shard}_INITIAL_SIZE=//p" "$audit_root/seed3-controller-epoch.env")
        [[ "$initial_size" =~ ^[0-9]+$ ]] || fail "missing initial byte boundary for seed3 shard $shard"
        current_size=$(stat -c %s -- "$seed3/${seed3_log_prefix}${shard}.log") \
            || fail "cannot stat seed3 shard $shard log"
        (( current_size >= initial_size )) \
            || fail "seed3 shard $shard log was truncated after epoch authentication"
        expected_prefix=$(read_one_value "$audit_root/seed3-controller-epoch.env" \
            "SEED3_LOG_${shard}_INITIAL_PREFIX_SHA256")
        actual_prefix=$(head -c "$initial_size" -- "$seed3/${seed3_log_prefix}${shard}.log" | sha256sum) \
            || fail "cannot rehash seed3 shard $shard authenticated log prefix"
        actual_prefix=${actual_prefix%% *}
        [[ "$actual_prefix" == "$expected_prefix" ]] \
            || fail "seed3 shard $shard authenticated log prefix changed"
        (( current_size > initial_size )) \
            || fail "seed3 shard $shard log has no output after the authenticated controller epoch"
        epoch_log=$(mktemp "${TMPDIR:-/tmp}/schema16-seed3-shard-$shard.XXXXXX") \
            || fail 'cannot stage shard epoch proof'
        tail -c "+$((initial_size + 1))" -- "$seed3/${seed3_log_prefix}${shard}.log" >"$epoch_log" \
            || { rm -f -- "$epoch_log"; fail "cannot read seed3 shard $shard epoch"; }
        mapfile -t final_lines < <(grep -E "^shard ${shard}/${seed3_shard_count} done: [0-9]+ captured, [0-9]+ failed, [0-9]+ skipped$" \
            "$epoch_log" || true)
        rm -f -- "$epoch_log"
        (( ${#final_lines[@]} > 0 )) || fail "seed3 shard $shard has no terminal summary"
        final_line=${final_lines[${#final_lines[@]}-1]}
        [[ "$final_line" =~ ^shard\ ${shard}/${seed3_shard_count}\ done:\ [0-9]+\ captured,\ 0\ failed,\ [0-9]+\ skipped$ ]] \
            || fail "seed3 shard $shard terminal summary reports failures: $final_line"
    done
    {
        printf 'DRAINED_UTC=%q\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
        printf 'LOGICAL_COUNT=9720\nMARKER_COUNT=9720\nSHARD_COUNT=%s\n' "$seed3_shard_count"
        printf 'CAPTURE_FAILURES=0\n'
    } | write_atomic "$audit_root/seed3-drain.env" || fail 'cannot publish seed3 drain proof'
    exec {collector_fd}>&-
    exec {admission_fd}>&-
}

descendants_non_zombie() {
    local root=$1 current child state
    local -a queue=("$root")
    while (( ${#queue[@]} != 0 )); do
        current=${queue[0]}; queue=("${queue[@]:1}")
        [[ -r "/proc/$current/task/$current/children" ]] || continue
        for child in $(<"/proc/$current/task/$current/children"); do
            queue+=("$child")
            [[ -r "/proc/$child/stat" ]] || continue
            state=$(awk '{print $3}' "/proc/$child/stat" 2>/dev/null) || continue
            [[ "$state" == Z ]] || printf '%s\n' "$child"
        done
    done
}

retire_existing_seed4_prepass_at_boundary() {
    local pane_pid main_pid child command state active deadline status_zero_count
    local -a pane_children=() matches=()
    if ! tmux has-session -t "$seed4_prepass_session" 2>/dev/null; then
        printf '%s seed4 P1 session already exited; its audit remains preserved\n' "$(date -Is)"
        return 0
    fi
    mapfile -t pane_pids < <(tmux list-panes -t "$seed4_prepass_session" -F '#{pane_pid}')
    (( ${#pane_pids[@]} == 1 )) || fail 'seed4 P1 session does not have exactly one pane'
    pane_pid=${pane_pids[0]}
    [[ -r "/proc/$pane_pid/task/$pane_pid/children" ]] \
        || fail 'cannot resolve seed4 P1 pane children'
    read -r -a pane_children <"/proc/$pane_pid/task/$pane_pid/children"
    for child in "${pane_children[@]}"; do
        command=$(tr '\0' ' ' <"/proc/$child/cmdline" 2>/dev/null) || continue
        if [[ "$command" == *'/scripts/run_native_conversion_prepass.sh '* \
            && "$command" == *"$seed4"* && "$command" == *"$seed4_prepass_audit"* ]]
        then
            matches+=("$child")
        fi
    done
    (( ${#matches[@]} == 1 )) || fail 'cannot resolve exactly one seed4 P1 supervisor process'
    main_pid=${matches[0]}
    command=$(tr '\0' ' ' <"/proc/$main_pid/cmdline") \
        || fail 'cannot capture seed4 P1 supervisor command'
    kill -STOP "$main_pid" || fail 'cannot freeze seed4 P1 supervisor at its wait boundary'
    deadline=$((SECONDS + 60))
    while :; do
        state=$(awk '{print $3}' "/proc/$main_pid/stat" 2>/dev/null) \
            || fail 'seed4 P1 supervisor disappeared before boundary retirement'
        [[ "$state" == T || "$state" == t ]] && break
        (( SECONDS < deadline )) || fail 'seed4 P1 supervisor did not enter stopped state'
        sleep 1
    done
    # The stopped supervisor cannot admit another run_one transaction. Its
    # already-admitted worker/converter remains runnable and publishes its
    # atomic status before becoming a zombie; only then terminate the parent.
    deadline=$((SECONDS + prepass_timeout + 120))
    while :; do
        active=$(descendants_non_zombie "$main_pid")
        [[ -z "$active" ]] && break
        (( SECONDS < deadline )) || {
            kill -CONT "$main_pid" 2>/dev/null || true
            fail "seed4 P1 transaction did not finish before retirement deadline:\n$active"
        }
        sleep 10
    done
    if find "$seed4/traces" -type f -name '*.parity-conversion-source' -print -quit | grep -q .; then
        kill -CONT "$main_pid" 2>/dev/null || true
        fail 'seed4 P1 reached retirement boundary with a quarantined source'
    fi
    status_zero_count=$(find "$seed4_prepass_audit/status" -maxdepth 1 -type f -name '*.status' \
        -exec awk -F '\t' '$1 == "0" {count += 1} END {print count + 0}' {} + \
        | awk '{sum += $1} END {print sum + 0}')
    {
        printf 'RETIRED_UTC=%q\nSESSION=%q\nPANE_PID=%s\nSUPERVISOR_PID=%s\n' \
            "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$seed4_prepass_session" "$pane_pid" "$main_pid"
        printf 'SUPERVISOR_COMMAND_SHA256=%s\nSUCCESSFUL_TRANSACTIONS=%s\n' \
            "$(printf '%s' "$command" | sha256sum | cut -d' ' -f1)" "$status_zero_count"
        printf 'RETIREMENT_BOUNDARY=no_non_zombie_descendants_and_no_quarantine\n'
        printf 'PRESERVED_AUDIT=%q\n' "$seed4_prepass_audit"
    } | write_atomic "$audit_root/seed4-p1-retirement.env" \
        || { kill -CONT "$main_pid" 2>/dev/null || true; fail 'cannot publish seed4 P1 retirement proof'; }
    kill -TERM "$main_pid" || fail 'cannot queue termination for stopped seed4 P1 supervisor'
    kill -CONT "$main_pid" || fail 'cannot release stopped seed4 P1 supervisor for termination'
    deadline=$((SECONDS + 60))
    while tmux has-session -t "$seed4_prepass_session" 2>/dev/null; do
        (( SECONDS < deadline )) || fail 'seed4 P1 tmux session did not exit after boundary retirement'
        sleep 1
    done
}

read_metric() {
    local file=$1 line_key=$2 field_key=$3
    awk -v line_key="$line_key" -v field_key="$field_key" '
        $1 == line_key {
            for (field_index = 2; field_index <= NF; field_index += 1) {
                split($field_index, pair, "=")
                if (pair[1] == field_key && pair[2] ~ /^[0-9]+([.][0-9]+)?$/) {
                    print pair[2]
                    exit
                }
            }
        }
    ' "$file"
}

float_lt() {
    LC_ALL=C awk -v value="$1" -v limit="$2" \
        'BEGIN { exit !(value + 0 < limit + 0) }'
}

float_le() {
    LC_ALL=C awk -v value="$1" -v limit="$2" \
        'BEGIN { exit !(value + 0 <= limit + 0) }'
}

process_is_global_writer() {
    local cmdline=$1 argument base saw_converter=0 saw_convert_flag=0
    local -a arguments=()
    mapfile -d '' -t arguments <"$cmdline" 2>/dev/null || return 1
    (( ${#arguments[@]} != 0 )) || return 1
    for argument in "${arguments[@]}"; do
        base=${argument##*/}
        case "$base" in
        capture_parity_save_replays.sh|run_schema16_distributed_capture.sh|run_native_conversion_prepass.sh|rsync)
            return 0
            ;;
        original_parity_replay|original_parity_replay.remote) saw_converter=1 ;;
        esac
        [[ "$argument" == -PARITYTRACE ]] && return 0
        [[ "$argument" == --convert ]] && saw_convert_flag=1
    done
    (( saw_converter == 1 && saw_convert_flag == 1 ))
}

all_corpora_quiet() {
    local campaign process pid
    for campaign in "$seed2" "$seed3" "$seed4"; do
        if find "$campaign/.capture-reservations" -type f -name '*.reserve' \
            -print -quit 2>/dev/null | grep -q .
        then
            return 1
        fi
    done
    # Conversion admission is intentionally global. Refuse P5/P3 launch while
    # any capture, conversion, transfer, or competing prepass can still write,
    # including work for a corpus outside this three-seed campaign.
    for process in /proc/[0-9]*/cmdline; do
        pid=${process#/proc/}; pid=${pid%/cmdline}
        [[ "$pid" != "$$" ]] || continue
        process_is_global_writer "$process" && return 1
    done
    return 0
}

release_gate_locks() {
    local fd
    for fd in "${gate_lock_fds[@]:-}"; do
        [[ -n "$fd" ]] && eval "exec ${fd}>&-"
    done
    gate_lock_fds=()
}

acquire_gate_locks() {
    local campaign fd lock_path
    gate_lock_fds=()
    exec {fd}>"$final_outer_lock" || return 1
    gate_lock_fds+=("$fd")
    flock -n "$fd" || { release_gate_locks; return 1; }
    for campaign in "$seed2" "$seed3" "$seed4"; do
        for lock_path in "$campaign/.capture-admission.lock" \
            "$campaign/.distributed-collector.lock"; do
            exec {fd}>"$lock_path" || { release_gate_locks; return 1; }
            gate_lock_fds+=("$fd")
            flock -n "$fd" || { release_gate_locks; return 1; }
        done
    done
}

read_pswpin() {
    local value
    value=$(awk '$1 == "pswpin" && $2 ~ /^[0-9]+$/ {print $2; exit}' "$vmstat_path")
    [[ "$value" =~ ^[0-9]+$ ]] || return 1
    printf '%s\n' "$value"
}

gate_sleep() {
    sleep "$1"
}

resource_sample_header() {
    printf 'ATTEMPT\tUTC\tINITIAL_MEM_AVAILABLE_KIB\tINITIAL_LOAD1\tINITIAL_MEMORY_SOME_AVG10\tINITIAL_MEMORY_FULL_AVG10\tINITIAL_CPU_SOME_AVG60\tFINAL_MEM_AVAILABLE_KIB\tFINAL_LOAD1\tFINAL_MEMORY_SOME_AVG10\tFINAL_MEMORY_FULL_AVG10\tFINAL_CPU_SOME_AVG60\tQUIET\tINSTANTANEOUS_PASS\tPSWPIN_BEFORE\tPSWPIN_AFTER\tRESULT\n'
}

sample_p5_gate() {
    local attempt=$1 log=$2 now memory_i load_i memory_some_i memory_full_i cpu_some_i
    local memory_f=NA load_f=NA memory_some_f=NA memory_full_f=NA cpu_some_f=NA
    local swap_before=NA swap_after=NA quiet=0 instantaneous=0 passed=0 reason
    now=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    memory_i=$(awk '$1 == "MemAvailable:" && $2 ~ /^[0-9]+$/ {print $2; exit}' "$meminfo_path")
    load_i=$(awk '{print $1}' "$loadavg_path")
    memory_some_i=$(read_metric "$memory_psi_path" some avg10)
    memory_full_i=$(read_metric "$memory_psi_path" full avg10)
    cpu_some_i=$(read_metric "$cpu_psi_path" some avg60)
    if acquire_gate_locks && all_corpora_quiet; then
        quiet=1
    else
        release_gate_locks
    fi
    reason=instantaneous-gate
    if [[ "$memory_i" =~ ^[0-9]+$ && "$load_i" =~ ^[0-9]+([.][0-9]+)?$ \
        && "$memory_some_i" =~ ^[0-9]+([.][0-9]+)?$ \
        && "$memory_full_i" =~ ^[0-9]+([.][0-9]+)?$ \
        && "$cpu_some_i" =~ ^[0-9]+([.][0-9]+)?$ ]] \
        && (( memory_i >= p5_min_memory_kib && quiet == 1 )) \
        && float_le "$load_i" "$p5_max_load1" \
        && float_lt "$memory_some_i" "$p5_max_memory_psi_avg10" \
        && float_le "$memory_full_i" 0 \
        && float_lt "$cpu_some_i" "$p5_max_cpu_psi_avg60"
    then
        instantaneous=1
        swap_before=$(read_pswpin) || fail 'cannot read pswpin before P5 resource sample'
        gate_sleep "$p5_swap_sample_seconds"
        swap_after=$(read_pswpin) || fail 'cannot read pswpin after P5 resource sample'
        reason=swap-in-detected
        memory_f=$(awk '$1 == "MemAvailable:" && $2 ~ /^[0-9]+$/ {print $2; exit}' "$meminfo_path")
        load_f=$(awk '{print $1}' "$loadavg_path")
        memory_some_f=$(read_metric "$memory_psi_path" some avg10)
        memory_full_f=$(read_metric "$memory_psi_path" full avg10)
        cpu_some_f=$(read_metric "$cpu_psi_path" some avg60)
        quiet=0
        all_corpora_quiet && quiet=1
        if (( swap_after == swap_before && quiet == 1 )) \
            && [[ "$memory_f" =~ ^[0-9]+$ && "$load_f" =~ ^[0-9]+([.][0-9]+)?$ \
                && "$memory_some_f" =~ ^[0-9]+([.][0-9]+)?$ \
                && "$memory_full_f" =~ ^[0-9]+([.][0-9]+)?$ \
                && "$cpu_some_f" =~ ^[0-9]+([.][0-9]+)?$ ]] \
            && (( memory_f >= p5_min_memory_kib )) \
            && float_le "$load_f" "$p5_max_load1" \
            && float_lt "$memory_some_f" "$p5_max_memory_psi_avg10" \
            && float_le "$memory_full_f" 0 \
            && float_lt "$cpu_some_f" "$p5_max_cpu_psi_avg60"
        then
            passed=1
            reason=pass
        elif (( swap_after == swap_before )); then
            reason=gate-changed-during-sample
        fi
    else
        gate_sleep "$p5_swap_sample_seconds"
    fi
    release_gate_locks
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
        "$attempt" "$now" "${memory_i:-NA}" "${load_i:-NA}" \
        "${memory_some_i:-NA}" "${memory_full_i:-NA}" "${cpu_some_i:-NA}" \
        "$memory_f" "$load_f" "$memory_some_f" "$memory_full_f" "$cpu_some_f" \
        "$quiet" "$instantaneous" "$swap_before" "$swap_after" "$reason" >>"$log" \
        || fail 'cannot append P5 resource sample'
    (( passed == 1 ))
}

sample_p3_gate() {
    local attempt=$1 log=$2 memory now quiet=0 passed=0 reason=p3-memory-or-work
    now=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    memory=$(awk '$1 == "MemAvailable:" && $2 ~ /^[0-9]+$/ {print $2; exit}' "$meminfo_path")
    [[ "$memory" =~ ^[0-9]+$ ]] || fail 'cannot read MemAvailable for P3 fallback gate'
    if acquire_gate_locks && all_corpora_quiet; then
        quiet=1
    else
        release_gate_locks
    fi
    if (( memory >= p3_min_memory_kib && quiet == 1 )); then
        passed=1
        reason=p3-pass
    fi
    printf '%s\t%s\t%s\tNA\tNA\tNA\tNA\tNA\tNA\tNA\tNA\tNA\t%s\t%s\tNA\tNA\t%s\n' \
        "p3-$attempt" "$now" "$memory" "$quiet" "$passed" "$reason" >>"$log" \
        || fail 'cannot append P3 resource sample'
    release_gate_locks
    (( passed == 1 ))
}

publish_resource_decision() {
    local seed=$1 selected=$2 reason=$3 samples=$4 decision=$5 decision_tmp
    decision_tmp=$(mktemp "$decision.tmp.XXXXXX") || fail 'cannot stage resource decision'
    {
        printf 'DECIDED_UTC=%q\nSEED=%s\nREQUESTED_JOBS=%s\nSELECTED_JOBS=%s\n' \
            "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$seed" "$prepass_jobs" "$selected"
        printf 'REASON=%s\nSAMPLES_SHA256=%s\n' "$reason" "$(sha256_file "$samples")"
    } >"$decision_tmp" || fail "cannot stage seed$seed resource decision"
    [[ ! -e "$decision" ]] || fail "seed$seed resource decision appeared concurrently"
    mv -- "$decision_tmp" "$decision" || fail "cannot publish seed$seed resource decision"
}

reuse_resource_decision() {
    local seed=$1 samples=$2 decision=$3 selected samples_sha actual_sha reason
    [[ -f "$samples" && -f "$decision" ]] \
        || fail "seed$seed resource-gate evidence is only partially published"
    [[ "$(sed -n '1p' "$samples")" == "$(resource_sample_header)" ]] \
        || fail "seed$seed resource-gate samples have the wrong header"
    [[ "$(read_one_uint "$decision" SEED)" == "$seed" \
        && "$(read_one_uint "$decision" REQUESTED_JOBS)" == "$prepass_jobs" ]] \
        || fail "seed$seed resource decision does not match immutable provenance"
    selected=$(read_one_uint "$decision" SELECTED_JOBS)
    [[ "$selected" =~ ^[1-5]$ && "$selected" -le "$prepass_jobs" ]] \
        || fail "seed$seed resource decision has an invalid selected job count"
    samples_sha=$(read_one_value "$decision" SAMPLES_SHA256)
    actual_sha=$(sha256_file "$samples") || fail "cannot hash seed$seed resource samples"
    [[ "$samples_sha" =~ ^[0-9a-f]{64}$ && "$actual_sha" == "$samples_sha" ]] \
        || fail "seed$seed resource samples changed after decision publication"
    reason=$(read_one_value "$decision" REASON)
    if (( prepass_jobs > 3 && selected == prepass_jobs )); then
        [[ "$reason" == p5-gates-passed ]] \
            || fail "seed$seed P5 resource decision has the wrong reason"
    elif (( prepass_jobs > 3 && selected == 3 )); then
        [[ "$reason" == p5-gates-exhausted-downshift-p3 ]] \
            || fail "seed$seed P3 downshift decision has the wrong reason"
    else
        [[ "$selected" == "$prepass_jobs" && "$reason" == requested-with-baseline-gate ]] \
            || fail "seed$seed baseline resource decision is inconsistent"
    fi
    selected_prepass_jobs=$selected
}

select_prepass_jobs() {
    local seed attempt decision samples samples_tmp selected reason last_result
    seed=$1
    decision="$audit_root/resource-gate-seed${seed}.env"
    samples="$audit_root/resource-gate-seed${seed}.tsv"
    if [[ -e "$decision" ]]; then
        reuse_resource_decision "$seed" "$samples" "$decision"
        return 0
    fi
    if [[ -e "$samples" ]]; then
        # Samples are moved into place only after a successful P5 or P3 gate.
        # Recover the narrow crash window before decision publication from the
        # authenticated final result rather than rerunning or overwriting it.
        [[ "$(sed -n '1p' "$samples")" == "$(resource_sample_header)" ]] \
            || fail "seed$seed orphan resource-gate samples have the wrong header"
        last_result=$(awk -F '\t' 'NR > 1 {value=$17} END {print value}' "$samples")
        case "$last_result" in
        pass)
            (( prepass_jobs > 3 )) \
                || fail "seed$seed resource samples contain an impossible P5 decision"
            selected=$prepass_jobs
            reason=p5-gates-passed
            ;;
        p3-pass)
            (( prepass_jobs > 3 )) && selected=3 || selected=$prepass_jobs
            (( prepass_jobs > 3 )) \
                && reason=p5-gates-exhausted-downshift-p3 \
                || reason=requested-with-baseline-gate
            ;;
        *) fail "seed$seed finalized resource samples have no successful terminal gate" ;;
        esac
        publish_resource_decision "$seed" "$selected" "$reason" "$samples" "$decision"
        reuse_resource_decision "$seed" "$samples" "$decision"
        return 0
    fi
    samples_tmp=$(mktemp "$samples.tmp.XXXXXX") || fail 'cannot stage resource samples'
    resource_sample_header >"$samples_tmp"
    selected=$prepass_jobs
    reason=requested-with-baseline-gate
    if (( prepass_jobs > 3 )); then
        selected=3
        reason=p5-gates-exhausted-downshift-p3
        for ((attempt=1; attempt<=p5_gate_attempts; attempt+=1)); do
            write_phase "resource-gate-seed${seed}-p${prepass_jobs}-attempt${attempt}"
            if sample_p5_gate "$attempt" "$samples_tmp"; then
                selected=$prepass_jobs
                reason=p5-gates-passed
                break
            fi
        done
    fi
    if (( selected <= 3 )); then
        attempt=1
        until sample_p3_gate "$attempt" "$samples_tmp"; do
            write_phase "resource-gate-seed${seed}-p${selected}-wait${attempt}"
            sleep "$poll_seconds"
            attempt=$((attempt + 1))
        done
    fi
    [[ ! -e "$samples" ]] || fail "seed$seed resource samples appeared concurrently"
    mv -- "$samples_tmp" "$samples" || fail "cannot publish seed$seed resource samples"
    publish_resource_decision "$seed" "$selected" "$reason" "$samples" "$decision"
    selected_prepass_jobs=$selected
}

read_launch_choice() {
    local file=$1 seed=$2 selected
    [[ -f "$file" ]] || return 1
    [[ "$(read_one_uint "$file" SEED)" == "$seed" ]] \
        || fail "seed$seed launch choice has the wrong seed"
    selected=$(read_one_uint "$file" SELECTED_JOBS)
    [[ "$selected" =~ ^[1-5]$ ]] || fail "seed$seed launch choice has invalid jobs"
    printf '%s\n' "$selected"
}

publish_launch_choice() {
    local file=$1 seed=$2 selected=$3 reason=$4 temporary
    temporary=$(mktemp "$file.tmp.XXXXXX") || fail 'cannot stage launch choice'
    {
        printf 'DECIDED_UTC=%q\nSEED=%s\nSELECTED_JOBS=%s\nREASON=%s\n' \
            "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$seed" "$selected" "$reason"
    } >"$temporary" || fail "cannot stage seed$seed launch choice"
    [[ ! -e "$file" ]] || fail "seed$seed launch choice appeared concurrently"
    mv -- "$temporary" "$file" || fail "cannot publish seed$seed launch choice"
}

publish_admission_proof() {
    local file=$1 seed=$2 selected=$3 reason=$4 samples=$5 temporary
    temporary=$(mktemp "$file.tmp.XXXXXX") || fail 'cannot stage admission proof'
    {
        printf 'ADMITTED_UTC=%q\nSEED=%s\nSELECTED_JOBS=%s\nREASON=%s\n' \
            "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$seed" "$selected" "$reason"
        printf 'SAMPLES_SHA256=%s\n' "$(sha256_file "$samples")"
    } >"$temporary" || fail "cannot stage seed$seed admission proof"
    [[ ! -e "$file" ]] || fail "seed$seed admission proof appeared concurrently"
    mv -- "$temporary" "$file" || fail "cannot publish seed$seed admission proof"
}

publish_resume_downshift() {
    local seed=$1 sequence=$2 samples=$3 downshift=$4 temporary
    temporary=$(mktemp "$downshift.tmp.XXXXXX") || fail 'cannot stage resume downshift'
    {
        printf 'DECIDED_UTC=%q\nSEED=%s\nSELECTED_JOBS=3\n' \
            "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$seed"
        printf 'REASON=resume-p5-gate-failed\nADMISSION_SEQUENCE=%s\nSAMPLES_SHA256=%s\n' \
            "$sequence" "$(sha256_file "$samples")"
    } >"$temporary" || fail "cannot stage seed$seed resume downshift"
    [[ ! -e "$downshift" ]] || fail "seed$seed resume downshift appeared concurrently"
    mv -- "$temporary" "$downshift" || fail "cannot publish seed$seed resume downshift"
}

recover_and_verify_admission_journal() {
    local seed=$1 downshift downshift_samples downshift_sequence=0 maximum=0 path base sequence
    local samples proof samples_sha selected result candidate reason
    downshift="$audit_root/resource-resume-downshift-seed${seed}.env"
    downshift_samples="$audit_root/resource-resume-downshift-seed${seed}.tsv"
    if [[ -e "$downshift_samples" && ! -e "$downshift" ]]; then
        [[ "$(sed -n '2p' "$downshift_samples")" == "$(resource_sample_header)" ]] \
            || fail "seed$seed orphan downshift sample has the wrong header"
        result=$(awk -F '\t' 'NR > 1 {value=$17} END {print value}' "$downshift_samples")
        [[ -n "$result" && "$result" != pass && "$result" != p3-pass ]] \
            || fail "seed$seed orphan downshift sample does not prove P5 rejection"
        sequence=$(sed -n 's/^# ADMISSION_SEQUENCE=//p' "$downshift_samples")
        [[ "$sequence" =~ ^[1-9][0-9]*$ ]] \
            || fail "seed$seed orphan downshift sample lacks its sequence"
        publish_resume_downshift "$seed" "$sequence" "$downshift_samples" "$downshift"
    fi
    if [[ -e "$downshift" || -e "$downshift_samples" ]]; then
        [[ -f "$downshift" && -f "$downshift_samples" ]] \
            || fail "seed$seed resume downshift is only partially published"
        [[ "$(read_one_uint "$downshift" SEED)" == "$seed" \
            && "$(read_one_uint "$downshift" SELECTED_JOBS)" == 3 \
            && "$(read_one_value "$downshift" REASON)" == resume-p5-gate-failed ]] \
            || fail "seed$seed resume downshift is malformed"
        downshift_sequence=$(read_one_uint "$downshift" ADMISSION_SEQUENCE)
        samples_sha=$(read_one_value "$downshift" SAMPLES_SHA256)
        [[ "$samples_sha" == "$(sha256_file "$downshift_samples")" ]] \
            || fail "seed$seed resume-downshift samples changed"
        maximum=$downshift_sequence
    fi
    for path in "$audit_root"/resource-admission-seed"$seed"-*.{tsv,env}; do
        [[ -e "$path" ]] || continue
        base=${path##*/}; base=${base#resource-admission-seed${seed}-}; base=${base%.*}
        [[ "$base" =~ ^[1-9][0-9]*$ ]] \
            || fail "seed$seed admission journal has a malformed sequence path"
        (( base > maximum )) && maximum=$base
    done
    for ((sequence=1; sequence<=maximum; sequence+=1)); do
        samples="$audit_root/resource-admission-seed${seed}-${sequence}.tsv"
        proof="$audit_root/resource-admission-seed${seed}-${sequence}.env"
        if [[ ! -e "$samples" && ! -e "$proof" ]]; then
            (( sequence == downshift_sequence )) \
                || fail "seed$seed admission journal has a sequence gap at $sequence"
            continue
        fi
        [[ -f "$samples" ]] || fail "seed$seed admission proof $sequence lacks samples"
        [[ "$(sed -n '1p' "$samples")" == "$(resource_sample_header)" ]] \
            || fail "seed$seed admission sample $sequence has the wrong header"
        result=$(awk -F '\t' 'NR > 1 {value=$17} END {print value}' "$samples")
        if [[ -e "$proof" ]]; then
            [[ "$(read_one_uint "$proof" SEED)" == "$seed" ]] \
                || fail "seed$seed admission proof $sequence has the wrong seed"
            selected=$(read_one_uint "$proof" SELECTED_JOBS)
            case "$result" in
            pass) (( selected > 3 && selected <= prepass_jobs )) \
                || fail "seed$seed admission proof $sequence misclassifies a P5 pass" ;;
            p3-pass) (( selected <= 3 )) \
                || fail "seed$seed admission proof $sequence misclassifies a P3 pass" ;;
            *) fail "seed$seed admission $sequence has no successful terminal sample" ;;
            esac
        else
            case "$result" in
            pass)
                candidate=$selected_prepass_jobs
                (( candidate > 3 )) \
                    || fail "seed$seed orphan admission $sequence has impossible P5 pass"
                reason=fresh-p5-gate-passed
                ;;
            p3-pass)
                if [[ -e "$downshift" ]]; then candidate=3; else candidate=$selected_prepass_jobs; fi
                (( candidate <= 3 )) \
                    || fail "seed$seed orphan admission $sequence has impossible P3 pass"
                reason=fresh-baseline-gate-passed
                ;;
            *) fail "seed$seed admission $sequence has no successful terminal sample" ;;
            esac
            publish_admission_proof "$proof" "$seed" "$candidate" "$reason" "$samples"
        fi
        samples_sha=$(read_one_value "$proof" SAMPLES_SHA256)
        [[ "$samples_sha" == "$(sha256_file "$samples")" ]] \
            || fail "seed$seed admission samples $sequence changed"
    done
    next_admission_sequence=$((maximum + 1))
}

confirm_prepass_launch() {
    local seed=$1 launch downshift candidate sequence samples temporary passed=0 reason
    launch="$audit_root/resource-launch-seed${seed}.env"
    downshift="$audit_root/resource-resume-downshift-seed${seed}.env"
    recover_and_verify_admission_journal "$seed"
    if [[ -e "$downshift" ]]; then
        candidate=$(read_launch_choice "$downshift" "$seed")
        [[ "$candidate" == 3 ]] || fail "seed$seed resume downshift is not P3"
    elif [[ -e "$launch" ]]; then
        candidate=$(read_launch_choice "$launch" "$seed")
    else
        candidate=$selected_prepass_jobs
    fi
    sequence=$next_admission_sequence
    samples="$audit_root/resource-admission-seed${seed}-${sequence}.tsv"
    temporary=$(mktemp "$samples.tmp.XXXXXX") || fail 'cannot stage launch admission sample'
    resource_sample_header >"$temporary"
    if (( candidate > 3 )); then
        write_phase "admit-seed${seed}-p${candidate}"
        if sample_p5_gate launch "$temporary"; then
            passed=1
            reason=fresh-p5-gate-passed
        else
            candidate=3
            reason=fresh-p5-gate-failed-downshift-p3
            downshift_samples="$audit_root/resource-resume-downshift-seed${seed}.tsv"
            [[ ! -e "$downshift_samples" ]] \
                || fail "seed$seed resume-downshift samples appeared concurrently"
            {
                printf '# ADMISSION_SEQUENCE=%s\n' "$sequence"
                cat -- "$temporary"
            } | write_atomic "$downshift_samples" \
                || fail "cannot publish seed$seed resume-downshift samples"
            publish_resume_downshift "$seed" "$sequence" "$downshift_samples" "$downshift"
        fi
    fi
    if (( candidate <= 3 )); then
        until sample_p3_gate launch "$temporary"; do
            write_phase "admit-seed${seed}-p${candidate}-wait"
            sleep "$poll_seconds"
        done
        passed=1
        [[ -n ${reason:-} ]] || reason=fresh-baseline-gate-passed
    fi
    (( passed == 1 )) || fail "seed$seed launch admission did not reach a safe gate"
    mv -- "$temporary" "$samples" || fail "cannot publish seed$seed launch admission sample"
    publish_admission_proof "$audit_root/resource-admission-seed${seed}-${sequence}.env" \
        "$seed" "$candidate" "$reason" "$samples"
    if [[ ! -e "$launch" ]]; then
        publish_launch_choice "$launch" "$seed" "$candidate" "$reason"
    fi
    selected_prepass_jobs=$candidate
}

run_prepass() {
    local seed=$1 campaign=$2 audit
    select_prepass_jobs "$seed"
    confirm_prepass_launch "$seed"
    audit="$audit_root/native-seed${seed}-p${selected_prepass_jobs}"
    write_phase "normalize-seed$seed"
    env NATIVE_CONVERT_JOBS="$selected_prepass_jobs" \
        NATIVE_CONVERT_OUTER_LOCK="$final_outer_lock" \
        NATIVE_CONVERT_TIMEOUT_SECONDS="$prepass_timeout" \
        "$workspace/scripts/run_native_conversion_prepass.sh" \
            "$workspace" "$campaign" "$bundle" "$bundle_trust_sha" "$audit" \
        >"$audit_root/native-seed${seed}-p${selected_prepass_jobs}.session.log" 2>&1
    [[ -f "$audit/COMPLETE" ]] || fail "seed$seed prepass returned without COMPLETE"
}

final_audit_for_campaign() {
    local campaign=$1 relative digest label
    relative=${campaign#"$workspace"/}
    digest=$(printf '%s' "$relative" | sha256sum); digest=${digest%% *}
    label=$(printf '%s' "${campaign##*/}" | tr -c 'A-Za-z0-9._-' '_')
    label=${label:0:48}
    printf '%s/parity-save-replays/audits/schema16-final-%s-path-%s-runner-%s\n' \
        "$workspace" "$label" "$digest" "$bundle_trust_sha"
}

verify_collect_all_set() {
    local audit=$1 expected=$2 trace relative key status log count=0 nonexact=0 value
    [[ "$(find "$audit/status" -maxdepth 1 -type f | wc -l)" == "$expected" \
        && "$(find "$audit/logs" -maxdepth 1 -type f | wc -l)" == "$expected" ]] \
        || fail "collect-all status/log set does not contain exactly $expected files"
    while IFS= read -r trace; do
        relative=${trace#"$workspace"/}; key=${relative//\//__}
        status="$audit/status/$key.status"; log="$audit/logs/$key.log"
        [[ -f "$status" && -f "$log" ]] || fail "collect-all evidence is incomplete: $trace"
        value=$(read_status_value "$status") \
            || fail "collect-all status must contain exactly one line: $status"
        evidence_status_is_valid "$value" "$log" \
            || fail "collect-all status is operational/corrupt rather than semantic evidence: $status ($value)"
        count=$((count + 1))
        [[ "$value" == 0 ]] || nonexact=$((nonexact + 1))
    done <"$audit/traces.snapshot"
    [[ "$count" == "$expected" ]] || fail "collect-all snapshot count is not $expected"
    printf '%s\n' "$nonexact"
}

verify_frozen_trace_identities() {
    local audit=$1 manifest="$1/traces.sha256" snapshot="$1/traces.snapshot"
    local line digest logical count=0 work expected_snapshot_sha expected_identities_sha
    expected_snapshot_sha=$(read_one_value "$audit/validation.env" SNAPSHOT_SHA256)
    expected_identities_sha=$(read_one_value "$audit/validation.env" TRACE_IDENTITIES_SHA256)
    [[ "$(sha256_file "$snapshot")" == "$expected_snapshot_sha" \
        && "$(sha256_file "$manifest")" == "$expected_identities_sha" ]] || return 1
    work=$(mktemp "${TMPDIR:-/tmp}/schema16-frozen-identities.XXXXXX") || return 1
    : >"$work"
    while IFS= read -r line; do
        [[ "$line" =~ ^([0-9a-f]{64})[[:space:]][[:space:]](/.*)$ ]] \
            || { rm -f -- "$work"; return 1; }
        digest=${BASH_REMATCH[1]}; logical=${BASH_REMATCH[2]}
        [[ ! -e "$logical" && -f "$logical.parity.bitcode.zst" \
            && "$(sha256_file "$logical.parity.bitcode.zst")" == "$digest" ]] \
            || { rm -f -- "$work"; return 1; }
        printf '%s\n' "$logical" >>"$work" || { rm -f -- "$work"; return 1; }
        count=$((count + 1))
    done <"$manifest"
    LC_ALL=C sort -o "$work" "$work" || { rm -f -- "$work"; return 1; }
    if (( count != 9720 )) || ! cmp -s -- "$work" "$snapshot"; then
        rm -f -- "$work"
        return 1
    fi
    rm -f -- "$work"
}

evidence_status_is_valid() {
    local value=$1 log=$2 marker_count
    case "$value" in
    0)
        marker_count=$(grep -Fxc -- 'parity trace matched every recorded frame' "$log" || true)
        [[ "$marker_count" == 1 ]]
        ;;
    1)
        grep -Eq -- 'first parity divergence after frame|logical parity scan: [1-9][0-9]* divergent frames' "$log"
        ;;
    101)
        # A schema-16 semantic mismatch can deliberately trip a parity/RNG
        # invariant. Do not classify arbitrary panics, malformed traces,
        # loader failures, signals or timeouts as parity evidence.
        grep -Eqi -- 'Rust consumed RNG draws|RNG[^[:cntrl:]]*(replay|stream)[^[:cntrl:]]*exhaust|parity[^[:cntrl:]]*diverg' "$log"
        ;;
    *) return 1 ;;
    esac
}

initial_failure_is_semantic() {
    local audit=$1 status log value found=0 classification failed_status trace relative key anchored_status
    classification=$(read_one_value "$audit/parity-last-failure.env" CLASSIFICATION)
    failed_status=$(read_one_value "$audit/parity-last-failure.env" STATUS)
    trace=$(read_one_value "$audit/parity-last-failure.env" TRACE)
    if [[ "$classification" == nonzero-or-malformed-status ]]; then
        [[ "$trace" == /* && "$trace" != *'\\'* && "$trace" == "$workspace"/* ]] \
            || return 1
        relative=${trace#"$workspace"/}; key=${relative//\//__}
        status="$audit/status/$key.status"; log="$audit/logs/$key.log"
        [[ -f "$status" && -f "$log" ]] || return 1
        anchored_status=$(read_status_value "$status") || return 1
        [[ "$anchored_status" == "$failed_status" && "$anchored_status" != 0 ]] || return 1
        evidence_status_is_valid "$anchored_status" "$log" || return 1
    elif [[ "$classification" == missing-status \
        && -f "$audit/.parallel-fail-fast-stop" ]]
    then
        # Parallel shards leave a sparse proof set after the authenticated
        # triggering worker publishes the shared stop. Snapshot-order
        # classification may encounter a deliberately unstarted trace before
        # that nonzero status. Authorize collection only from the actual proof
        # set below: at least one semantic nonzero, and no published
        # operational, integrity, malformed, or half-published result.
        :
    else
        return 1
    fi
    while IFS= read -r -d '' status; do
        log="$audit/logs/${status##*/}"; log=${log%.status}.log
        [[ -f "$log" ]] || return 1
        value=$(read_status_value "$status") || return 1
        evidence_status_is_valid "$value" "$log" || return 1
        [[ "$value" == 0 ]] || found=1
    done < <(find "$audit/status" -maxdepth 1 -type f -name '*.status' -print0)
    (( found == 1 ))
}

run_collect_all_parallel() (
    set -euo pipefail
    local audit=$1 shard pid status=0
    local -a workers=()
    cleanup_collect_workers() {
        local worker attempt alive
        trap - EXIT INT TERM
        for worker in "${workers[@]}"; do
            kill -TERM -- "-$worker" 2>/dev/null || kill "$worker" 2>/dev/null || true
        done
        for attempt in {1..50}; do
            alive=0
            for worker in "${workers[@]}"; do
                kill -0 -- "-$worker" 2>/dev/null && alive=1
            done
            (( alive == 0 )) && break
            sleep 0.1
        done
        for worker in "${workers[@]}"; do
            kill -KILL -- "-$worker" 2>/dev/null || true
            wait "$worker" 2>/dev/null || true
        done
    }
    trap cleanup_collect_workers EXIT
    trap 'exit 130' INT TERM
    for ((shard=0; shard<sweep_concurrency; shard+=1)); do
        (
            cd -- "$workspace"
            exec setsid env PARITY_SWEEP_FAIL_FAST=0 \
                PARITY_SWEEP_GLOBAL_CONCURRENCY="$sweep_concurrency" \
                PARITY_SWEEP_SLOT_DIR="$workspace/.git/parity-runner-slots" \
                "$workspace/scripts/run_parity_release_sweep.sh" \
                    "$workspace" "$audit" \
                    "$workspace/.git/schema16-final-runners/$bundle_trust_sha/original_parity_replay.remote" \
                    "$shard" "$sweep_concurrency"
        ) &
        workers+=("$!")
    done
    for pid in "${workers[@]}"; do
        wait "$pid" || status=1
    done
    (( status == 0 ))
    trap - EXIT INT TERM
)

run_final_campaign() {
    local seed=$1 campaign=$2 audit rc nonexact summary_tmp
    audit=$(final_audit_for_campaign "$campaign")
    write_phase "validate-seed$seed"
    rc=0
    env SCHEMA16_FINAL_RUNNER_MODE=bundle \
        SCHEMA16_FINAL_RUNNER_BUNDLE_SHA256="$bundle_trust_sha" \
        SCHEMA16_FINAL_SWEEP_CONCURRENCY="$sweep_concurrency" \
        "$workspace/scripts/run_schema16_final_validation.sh" \
            "$workspace" "$bundle" "$runner_sha" "$campaign" \
        >"$audit_root/final-seed$seed.session.log" 2>&1 || rc=$?
    if (( rc == 0 )); then
        [[ -f "$audit/parity-verdict.env" ]] || fail "seed$seed validation returned without a verdict"
        grep -Fxq 'EXACT_PARITY=1' "$audit/parity-verdict.env" \
            || fail "seed$seed validation returned success without exact parity"
        printf 'SEED=%s\nEXACT_PARITY=1\nNONEXACT=0\nSWEEP_CONCURRENCY=%s\nAUDIT=%s\n' \
            "$seed" "$sweep_concurrency" "$audit" \
            | write_atomic "$audit_root/final-seed$seed.env" \
            || fail "cannot publish exact seed$seed summary"
        return 0
    fi
    (( rc == 1 )) || fail "seed$seed final validator stopped on setup/integrity status $rc"
    [[ -f "$audit/parity-last-failure.env" && -f "$audit/traces.snapshot" \
        && -f "$audit/validation.env" ]] \
        || fail "seed$seed status 1 lacks an initialized failure audit"
    initial_failure_is_semantic "$audit" \
        || fail "seed$seed first failure is operational/integrity evidence; refusing collect-all"
    write_phase "collect-all-seed$seed"
    collect_outer_lock=$final_outer_lock
    exec {collect_outer_fd}>"$collect_outer_lock" \
        || fail "cannot open collect-all outer lock: $collect_outer_lock"
    flock "$collect_outer_fd" || fail 'cannot acquire collect-all outer lock'
    run_collect_all_parallel "$audit" \
        >"$audit_root/collect-all-seed$seed.session.log" 2>&1 \
        || fail "seed$seed collect-all sweep failed operationally"
    nonexact=$(verify_collect_all_set "$audit" 9720) \
        || fail "seed$seed collect-all proof validation failed"
    verify_frozen_trace_identities "$audit" \
        || fail "seed$seed trace bytes changed during collect-all"
    verify_bundle "$workspace/.git/schema16-final-runners/$bundle_trust_sha"
    exec {collect_outer_fd}>&-
    (( nonexact > 0 )) || fail "seed$seed initial nonexact disappeared in collect-all evidence"
    summary_tmp=$(mktemp "$audit_root/final-seed$seed.env.tmp.XXXXXX") || exit 2
    {
        printf 'SEED=%s\nEXACT_PARITY=0\nNONEXACT=%s\nSWEEP_CONCURRENCY=%s\nAUDIT=%s\n' \
            "$seed" "$nonexact" "$sweep_concurrency" "$audit"
        printf 'STATUS_COUNTS_BEGIN\n'
        find "$audit/status" -maxdepth 1 -type f -exec cat -- {} + \
            | LC_ALL=C sort | uniq -c
        printf 'STATUS_COUNTS_END\n'
    } >"$summary_tmp" || fail "cannot stage seed$seed collect-all summary"
    mv -f -- "$summary_tmp" "$audit_root/final-seed$seed.env" \
        || fail "cannot publish seed$seed collect-all summary"
    return 1
}

# The regression harness sources the pure resource-decision functions with
# synthetic samplers. Normal execution always continues into validation/main.
if [[ "${BASH_SOURCE[0]}" != "$0" ]]; then
    return 0
fi

[[ "$poll_seconds" =~ ^[1-9][0-9]*$ ]] || fail 'poll interval must be positive'
[[ "$prepass_jobs" =~ ^[1-5]$ ]] || fail 'prepass jobs must be 1 through 5'
[[ "$prepass_timeout" =~ ^[0-9]+$ && "$prepass_timeout" -ge 3600 ]] \
    || fail 'prepass timeout must be at least 3600 seconds'
p5_gate_attempts=$(normalize_bounded_uint "$p5_gate_attempts" 999) \
    || fail 'P5 gate attempts must be 1 through 999'
(( p5_gate_attempts >= 1 )) || fail 'P5 gate attempts must be 1 through 999'
p5_min_memory_kib=$(normalize_bounded_uint "$p5_min_memory_kib" 9999999999) \
    || fail 'P5 memory threshold must be an unsigned value of at most 9999999999 KiB'
p3_min_memory_kib=$(normalize_bounded_uint "$p3_min_memory_kib" 9999999999) \
    || fail 'P3 memory threshold must be an unsigned value of at most 9999999999 KiB'
p5_swap_sample_seconds=$(normalize_bounded_uint "$p5_swap_sample_seconds" 99999) \
    || fail 'P5 swap sample must be between 60 and 99999 seconds'
(( p5_swap_sample_seconds >= 60 )) \
    || fail 'P5 swap sample must be between 60 and 99999 seconds'
[[ "$p5_max_load1" =~ ^(0|[1-9][0-9]{0,2})([.][0-9]{1,6})?$ \
    && "$p5_max_memory_psi_avg10" =~ ^(0|[1-9][0-9]{0,2})([.][0-9]{1,6})?$ \
    && "$p5_max_cpu_psi_avg60" =~ ^(0|[1-9][0-9]{0,2})([.][0-9]{1,6})?$ ]] \
    || fail 'resource-gate load and PSI thresholds must be bounded nonnegative decimals'
float_le "$p5_max_load1" 128 \
    || fail 'P5 load threshold must not exceed 128'
float_le "$p5_max_memory_psi_avg10" 100 \
    || fail 'P5 memory PSI threshold must not exceed 100'
float_le "$p5_max_cpu_psi_avg60" 100 \
    || fail 'P5 CPU PSI threshold must not exceed 100'
float_lt 0 "$p5_max_memory_psi_avg10" \
    || fail 'P5 memory PSI threshold must be positive'
float_lt 0 "$p5_max_cpu_psi_avg60" \
    || fail 'P5 CPU PSI threshold must be positive'
[[ "$preflight_only" == 0 || "$preflight_only" == 1 ]] || fail 'preflight flag must be 0 or 1'
[[ "$seed3_shard_count" =~ ^[1-9][0-9]*$ ]] || fail 'seed3 shard count must be positive'
[[ "$sweep_concurrency" =~ ^[1-9][0-9]*$ && "$sweep_concurrency" -le 64 ]] \
    || fail 'sweep concurrency must be 1 through 64'
[[ "$bundle_trust_sha" =~ ^[0-9a-f]{64}$ && "$runner_sha" =~ ^[0-9a-f]{64}$ ]] \
    || fail 'runner trust values must be lowercase SHA-256 digests'

workspace=$(realpath -e -- "$workspace_arg")
orchestrator_script=$(realpath -e -- "$0")
orchestrator_script_sha=$(sha256_file "$orchestrator_script") \
    || fail 'cannot hash orchestrator script'
bundle=$(realpath -e -- "$bundle_arg")
seed2=$(realpath -e -- "$seed2_arg")
seed3=$(realpath -e -- "$seed3_arg")
seed4=$(realpath -e -- "$seed4_arg")
audit_parent=$(dirname -- "$audit_root_arg")
mkdir -p -- "$audit_parent"
audit_root=$(realpath -m -- "$audit_root_arg")
[[ "$seed2" == "$workspace"/* && "$seed3" == "$workspace"/* \
    && "$seed4" == "$workspace"/* && "$audit_root" == "$workspace"/* ]] \
    || fail 'all campaigns and the audit root must be below the workspace'
paths_overlap "$audit_root" "$seed2" && fail 'audit root must not overlap seed2 corpus'
paths_overlap "$audit_root" "$seed3" && fail 'audit root must not overlap seed3 corpus'
paths_overlap "$audit_root" "$seed4" && fail 'audit root must not overlap seed4 corpus'
paths_overlap "$seed2" "$seed3" && fail 'seed2 and seed3 corpora must not overlap'
paths_overlap "$seed2" "$seed4" && fail 'seed2 and seed4 corpora must not overlap'
paths_overlap "$seed3" "$seed4" && fail 'seed3 and seed4 corpora must not overlap'
mkdir -p -- "$audit_root"

exec {orchestrator_fd}>"$audit_root/orchestrator.lock"
flock -n "$orchestrator_fd" || fail "another orchestrator owns $audit_root"
verify_script "$workspace/scripts/run_native_conversion_prepass.sh" "$expected_prepass_script_sha"
verify_script "$workspace/scripts/run_schema16_final_validation.sh" "$expected_final_script_sha"
verify_script "$workspace/scripts/run_parity_release_sweep.sh" "$expected_sweep_script_sha"
verify_bundle "$bundle"
verify_campaign_metadata "$seed2" 2000000
verify_campaign_metadata "$seed3" 3000000
verify_campaign_metadata "$seed4" 4000000
initialize_or_verify_provenance
initialize_or_verify_seed3_epoch

if (( preflight_only == 1 )); then
    write_phase preflight-complete
    exit 0
fi

write_phase wait-seed3-natural-exit
wait_for_seed3_sessions
write_phase drain-and-verify-seed3
drain_and_verify_seed3
write_phase retire-existing-seed4-prepass-at-transaction-boundary
retire_existing_seed4_prepass_at_boundary

run_prepass 3 "$seed3"
run_prepass 4 "$seed4"
run_prepass 2 "$seed2"

semantic_failures=0
run_final_campaign 2 "$seed2" || semantic_failures=$((semantic_failures + 1))
run_final_campaign 3 "$seed3" || semantic_failures=$((semantic_failures + 1))
run_final_campaign 4 "$seed4" || semantic_failures=$((semantic_failures + 1))

if (( semantic_failures != 0 )); then
    write_phase "complete-with-$semantic_failures-nonexact-corpora"
    exit 1
fi
write_phase complete-all-exact
printf '%s all three existing schema16 corpora achieved exact parity\n' "$(date -Is)"
