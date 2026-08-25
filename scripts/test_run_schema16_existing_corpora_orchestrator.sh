#!/usr/bin/env bash
set -euo pipefail

repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
orchestrator="$repository/scripts/run_schema16_existing_corpora_orchestrator.sh"
mkdir -p -- "$repository/.codex-tmp"
test_root=$(mktemp -d "$repository/.codex-tmp/schema16-existing-orchestrator-test.XXXXXX")
cleanup() {
    rm -rf -- "$test_root"
}
trap cleanup EXIT

workspace="$test_root/work space"
fake_bin="$test_root/fake-bin"
bundle="$workspace/runner bundle"
mkdir -p -- "$workspace/scripts" "$fake_bin" "$bundle"

for script in run_native_conversion_prepass.sh \
    run_schema16_final_validation.sh run_parity_release_sweep.sh; do
    ln -s -- "$repository/scripts/$script" "$workspace/scripts/$script"
done

cat >"$fake_bin/tmux" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
case "${1:-}" in
has-session)
    [[ ${FAKE_TMUX_ACTIVE:-0} == 1 ]]
    ;;
list-panes)
    [[ ${FAKE_TMUX_ACTIVE:-0} == 1 ]]
    cat -- "$FAKE_TMUX_COMMAND_FILE"
    ;;
*)
    printf 'unexpected fake tmux invocation:' >&2
    printf ' %q' "$@" >&2
    printf '\n' >&2
    exit 64
    ;;
esac
EOF
chmod +x -- "$fake_bin/tmux"

cat >"$bundle/original_parity_replay" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
cat >"$bundle/original_parity_replay.remote" <<'EOF'
#!/usr/bin/env bash
exec "$(dirname -- "$0")/original_parity_replay" "$@"
EOF
cat >"$bundle/PROVENANCE.txt" <<'EOF'
NATIVE_CONVERSION_PROTOCOL=2
EOF
printf 'fake shared object\n' >"$bundle/libfake.so"
chmod +x -- "$bundle/original_parity_replay" "$bundle/original_parity_replay.remote"
(
    cd -- "$bundle"
    sha256sum -- original_parity_replay original_parity_replay.remote PROVENANCE.txt >SHA256SUMS
    sha256sum -- libfake.so >LIB_SHA256SUMS
)
runner_sha=$(sha256sum -- "$bundle/original_parity_replay")
runner_sha=${runner_sha%% *}
main_manifest_sha=$(sha256sum -- "$bundle/SHA256SUMS")
main_manifest_sha=${main_manifest_sha%% *}
lib_manifest_sha=$(sha256sum -- "$bundle/LIB_SHA256SUMS")
lib_manifest_sha=${lib_manifest_sha%% *}
bundle_trust_sha=$(printf 'schema16-runner-bundle-v1\nSHA256SUMS=%s\nLIB_SHA256SUMS=%s\n' \
    "$main_manifest_sha" "$lib_manifest_sha" | sha256sum)
bundle_trust_sha=${bundle_trust_sha%% *}

make_campaign() {
    local name=$1 seed=$2 campaign
    campaign="$workspace/$name"
    mkdir -p -- "$campaign/traces"
    {
        printf 'PARITY_TRACE_SCHEMA=16\n'
        printf 'EXPECTED_LOGICAL_REPLAYS=9720\n'
        printf 'PARITY_INPUT_SEED_BASE=%s\n' "$seed"
    } >"$campaign/campaign.env"
    printf '%s\n' "$campaign"
}

seed2=$(make_campaign seed2 2000000)
seed3=$(make_campaign seed3 3000000)
seed4=$(make_campaign seed4 4000000)
seed3_log="$seed3/capture-seed3-shard-0.log"
printf 'old\n' >"$seed3_log"
command_file="$test_root/tmux-command"
recorder_component=7425511e6c9bac698804fe93bb4cf7af02740504b8f46d702f03e4bc2a385ff8
printf "env SHARD_COUNT=1 SHARD_INDEX=0 CORPUS='%s' /immutable/%s/robin\n" \
    "$seed3" "$recorder_component" >"$command_file"

run_preflight() {
    local audit=$1 active=$2
    shift 2
    env PATH="$fake_bin:$PATH" \
        FAKE_TMUX_ACTIVE="$active" \
        FAKE_TMUX_COMMAND_FILE="$command_file" \
        SCHEMA16_ORCH_PREFLIGHT_ONLY=1 \
        SCHEMA16_ORCH_SEED3_SHARD_COUNT=1 \
        "$@" \
        "$orchestrator" "$workspace" "$bundle" "$bundle_trust_sha" "$runner_sha" \
            "$seed2" "$seed3" "$seed4" "$audit"
}

expect_failure() {
    local expected=$1 output rc=0
    shift
    output=$("$@" 2>&1) || rc=$?
    if (( rc == 0 )); then
        printf 'test failure: command unexpectedly succeeded: %q\n' "$*" >&2
        exit 1
    fi
    if ! grep -Fq -- "$expected" <<<"$output"; then
        printf 'test failure: missing expected diagnostic %q in:\n%s\n' "$expected" "$output" >&2
        exit 1
    fi
}

audit="$workspace/audits/preflight"
run_preflight "$audit" 1 >/dev/null
grep -Fxq 'PHASE=preflight-complete' "$audit/state.env"
grep -Fxq 'SEED3_LOG_0_INITIAL_SIZE=4' "$audit/seed3-controller-epoch.env"
initial_prefix_sha=$(printf 'old\n' | sha256sum)
initial_prefix_sha=${initial_prefix_sha%% *}
grep -Fxq "SEED3_LOG_0_INITIAL_PREFIX_SHA256=$initial_prefix_sha" \
    "$audit/seed3-controller-epoch.env"
orchestrator_sha=$(sha256sum -- "$orchestrator")
orchestrator_sha=${orchestrator_sha%% *}
grep -Fxq "ORCHESTRATOR_SCRIPT_SHA256=$orchestrator_sha" "$audit/provenance.env"
expected_command=$(<"$command_file")
expected_digest=$(printf '%s' "$expected_command" | sha256sum)
expected_digest=${expected_digest%% *}
grep -Fxq "SEED3_SESSION_0_COMMAND_SHA256=$expected_digest" \
    "$audit/seed3-controller-epoch.env"

# A stopped, already-authenticated controller may be resumed through preflight:
# the immutable epoch is retained and does not require the old tmux session.
epoch_before=$(sha256sum -- "$audit/seed3-controller-epoch.env")
epoch_before=${epoch_before%% *}
run_preflight "$audit" 0 >/dev/null
epoch_after=$(sha256sum -- "$audit/seed3-controller-epoch.env")
epoch_after=${epoch_after%% *}
[[ "$epoch_after" == "$epoch_before" ]]

# Operational knobs are part of immutable provenance, so an accidental resume
# with a different polling contract must fail before doing corpus work.
expect_failure 'orchestrator invocation differs from immutable prior provenance' \
    run_preflight "$audit" 0 SCHEMA16_ORCH_POLL_SECONDS=17
expect_failure 'orchestrator invocation differs from immutable prior provenance' \
    run_preflight "$audit" 0 SCHEMA16_ORCH_SWEEP_CONCURRENCY=7
expect_failure 'orchestrator invocation differs from immutable prior provenance' \
    run_preflight "$audit" 0 SCHEMA16_ORCH_PREPASS_JOBS=4
expect_failure 'orchestrator invocation differs from immutable prior provenance' \
    run_preflight "$audit" 0 SCHEMA16_ORCH_P5_GATE_ATTEMPTS=2
expect_failure 'orchestrator invocation differs from immutable prior provenance' \
    run_preflight "$audit" 0 SCHEMA16_ORCH_P5_MIN_MEMORY_KIB=52428801
expect_failure 'orchestrator invocation differs from immutable prior provenance' \
    run_preflight "$audit" 0 SCHEMA16_ORCH_P5_MAX_LOAD1=15
expect_failure 'orchestrator invocation differs from immutable prior provenance' \
    run_preflight "$audit" 0 SCHEMA16_ORCH_P5_MAX_MEMORY_PSI_AVG10=0.5
expect_failure 'orchestrator invocation differs from immutable prior provenance' \
    run_preflight "$audit" 0 SCHEMA16_ORCH_P5_MAX_CPU_PSI_AVG60=4
expect_failure 'orchestrator invocation differs from immutable prior provenance' \
    run_preflight "$audit" 0 SCHEMA16_ORCH_P5_SWAP_SAMPLE_SECONDS=61
expect_failure 'orchestrator invocation differs from immutable prior provenance' \
    run_preflight "$audit" 0 SCHEMA16_ORCH_P3_MIN_MEMORY_KIB=25165825
expect_failure 'orchestrator invocation differs from immutable prior provenance' \
    run_preflight "$audit" 0 SCHEMA16_FINAL_OUTER_LOCK="$test_root/different-final.lock"

# A live session whose command changes after authentication must be rejected,
# and the authenticated epoch itself must remain byte-for-byte unchanged.
printf "env SHARD_COUNT=1 SHARD_INDEX=0 CORPUS='%s' EXTRA=changed /immutable/%s/robin\n" \
    "$seed3" "$recorder_component" >"$command_file"
expect_failure 'seed3 session command changed after epoch authentication' \
    run_preflight "$audit" 1
[[ "$(sha256sum -- "$audit/seed3-controller-epoch.env" | cut -d' ' -f1)" == "$epoch_before" ]]

# A fresh epoch cannot bless a missing controller merely because preflight was
# requested. This prevents an unauthenticated capture from entering the audit.
expect_failure 'cannot authenticate absent initial seed3 session' \
    run_preflight "$workspace/audits/absent-controller" 0

# Bundle content is checked, not merely the caller-provided trust digest.
printf 'tampered\n' >>"$bundle/libfake.so"
expect_failure 'runner bundle checksum verification failed' \
    run_preflight "$workspace/audits/tampered-bundle" 1

# Exercise the pure resource-decision state machine with synthetic samplers.
# This avoids sleeping while proving P5 selection, bounded downshift, crash
# recovery between samples and decision publication, and immutable reuse.
(
    source "$orchestrator" unused unused \
        0000000000000000000000000000000000000000000000000000000000000000 \
        0000000000000000000000000000000000000000000000000000000000000000 \
        unused unused unused unused
    audit_root="$workspace/audits/resource-state-machine"
    mkdir -p -- "$audit_root"
    prepass_jobs=5
    p5_gate_attempts=2
    poll_seconds=1
    write_phase() { :; }
    sleep() { :; }
    append_gate_row() {
        printf 'synthetic\tutc\t1\t1\t0\t0\t0\t1\t1\t0\t0\t0\t1\t1\t0\t0\t%s\n' "$2" >>"$1"
    }
    sample_p5_gate() { append_gate_row "$2" rejected; return 1; }
    sample_p3_gate() { append_gate_row "$2" p3-pass; return 0; }
    select_prepass_jobs 3
    [[ "$selected_prepass_jobs" == 3 ]]
    grep -Fxq 'REASON=p5-gates-exhausted-downshift-p3' \
        "$audit_root/resource-gate-seed3.env"
    decision_sha=$(sha256sum -- "$audit_root/resource-gate-seed3.env")
    select_prepass_jobs 3
    [[ "$selected_prepass_jobs" == 3 \
        && "$(sha256sum -- "$audit_root/resource-gate-seed3.env")" == "$decision_sha" ]]

    sample_p5_gate() { append_gate_row "$2" pass; return 0; }
    select_prepass_jobs 2
    [[ "$selected_prepass_jobs" == 5 ]]
    grep -Fxq 'REASON=p5-gates-passed' "$audit_root/resource-gate-seed2.env"
    confirm_prepass_launch 2
    [[ "$selected_prepass_jobs" == 5 ]]
    grep -Fxq 'SELECTED_JOBS=5' "$audit_root/resource-launch-seed2.env"
    sample_p5_gate() { append_gate_row "$2" rejected; return 1; }
    sample_p3_gate() { append_gate_row "$2" p3-pass; return 0; }
    confirm_prepass_launch 2
    [[ "$selected_prepass_jobs" == 3 ]]
    grep -Fxq 'SELECTED_JOBS=3' "$audit_root/resource-resume-downshift-seed2.env"
    # Once a failed fresh resume gate pins the P3 continuation, later helper
    # invocations never resurrect the original stale P5 authorization.
    sample_p5_gate() { return 77; }
    confirm_prepass_launch 2
    [[ "$selected_prepass_jobs" == 3 ]]

    # Recover the exact samples->decision crash window without resampling.
    samples4="$audit_root/resource-gate-seed4.tsv"
    resource_sample_header >"$samples4"
    append_gate_row "$samples4" pass
    select_prepass_jobs 4
    [[ "$selected_prepass_jobs" == 5 ]]
    grep -Fxq "SAMPLES_SHA256=$(sha256sum -- "$samples4" | cut -d' ' -f1)" \
        "$audit_root/resource-gate-seed4.env"

    # Fault boundaries in the append-only launch journal are recovered or
    # rejected deterministically rather than silently skipped.
    orphan_downshift="$audit_root/resource-resume-downshift-seed6.tsv"
    printf '# ADMISSION_SEQUENCE=1\n' >"$orphan_downshift"
    resource_sample_header >>"$orphan_downshift"
    append_gate_row "$orphan_downshift" rejected
    recover_and_verify_admission_journal 6
    [[ "$next_admission_sequence" == 2 ]]
    grep -Fxq 'SELECTED_JOBS=3' "$audit_root/resource-resume-downshift-seed6.env"
    orphan_admission="$audit_root/resource-admission-seed7-1.tsv"
    resource_sample_header >"$orphan_admission"
    append_gate_row "$orphan_admission" pass
    selected_prepass_jobs=5
    recover_and_verify_admission_journal 7
    [[ "$next_admission_sequence" == 2 ]]
    [[ -f "$audit_root/resource-admission-seed7-1.env" ]]
    printf 'tampered\n' >>"$orphan_admission"
    if (recover_and_verify_admission_journal 7) 2>/dev/null; then
        printf 'test failure: accepted a changed prior admission sample\n' >&2
        exit 1
    fi
    printf 'SEED=8\nSELECTED_JOBS=5\nSAMPLES_SHA256=bad\n' \
        >"$audit_root/resource-admission-seed8-1.env"
    if (recover_and_verify_admission_journal 8) 2>/dev/null; then
        printf 'test failure: accepted an admission proof without samples\n' >&2
        exit 1
    fi

    [[ "$(normalize_bounded_uint 00089 999)" == 89 ]]
    float_le 16 16
    ! float_le 16.01 16
    float_lt 0.99 1
    ! float_lt 1 1
)

# Exercise the real sampler against fake proc metrics while using real flock
# descriptors. gate_sleep proves all seven exclusion locks remain held for the
# whole sample; no wall-clock sleep is needed in this fixture.
(
    source "$orchestrator" unused unused \
        0000000000000000000000000000000000000000000000000000000000000000 \
        0000000000000000000000000000000000000000000000000000000000000000 \
        unused unused unused unused
    sampler_root="$workspace/sampler-fixture"
    seed2="$sampler_root/seed2"; seed3="$sampler_root/seed3"; seed4="$sampler_root/seed4"
    mkdir -p -- "$seed2/.capture-reservations" "$seed3/.capture-reservations" \
        "$seed4/.capture-reservations"
    final_outer_lock="$sampler_root/global.lock"
    meminfo_path="$sampler_root/meminfo"
    loadavg_path="$sampler_root/loadavg"
    memory_psi_path="$sampler_root/memory.pressure"
    cpu_psi_path="$sampler_root/cpu.pressure"
    vmstat_path="$sampler_root/vmstat"
    p5_min_memory_kib=52428800
    p5_max_load1=16
    p5_max_memory_psi_avg10=1
    p5_max_cpu_psi_avg60=5
    p5_swap_sample_seconds=60
    printf 'MemAvailable: 52428800 kB\n' >"$meminfo_path"
    printf '16.00 0.00 0.00 1/1 1\n' >"$loadavg_path"
    printf 'some avg10=0.99 avg60=0.00 avg300=0.00 total=0\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=0\n' \
        >"$memory_psi_path"
    printf 'some avg10=0.00 avg60=4.99 avg300=0.00 total=0\n' >"$cpu_psi_path"
    printf 'pswpin 10\n' >"$vmstat_path"
    gate_sleep() {
        local lock fd
        for lock in "$final_outer_lock" \
            "$seed2/.capture-admission.lock" "$seed2/.distributed-collector.lock" \
            "$seed3/.capture-admission.lock" "$seed3/.distributed-collector.lock" \
            "$seed4/.capture-admission.lock" "$seed4/.distributed-collector.lock"; do
            exec {fd}>"$lock"
            if flock -n "$fd"; then
                printf 'test failure: sampler did not retain lock %s\n' "$lock" >&2
                exit 1
            fi
            eval "exec ${fd}>&-"
        done
        if [[ ${increment_swap:-0} == 1 ]]; then
            printf 'pswpin 11\n' >"$vmstat_path"
        fi
        if [[ ${degrade_load:-0} == 1 ]]; then
            printf '16.01 0.00 0.00 1/1 1\n' >"$loadavg_path"
        fi
    }
    sample_log="$sampler_root/pass.tsv"; : >"$sample_log"
    sample_p5_gate boundary "$sample_log"
    grep -Fq $'\tpass' "$sample_log"
    # Exact full-memory PSI zero is required, not merely below one.
    printf 'some avg10=0.99 avg60=0.00 avg300=0.00 total=0\nfull avg10=0.01 avg60=0.00 avg300=0.00 total=0\n' \
        >"$memory_psi_path"
    ! sample_p5_gate full-nonzero "$sampler_root/full-nonzero.tsv"
    printf 'some avg10=0.99 avg60=0.00 avg300=0.00 total=0\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=0\n' \
        >"$memory_psi_path"
    printf 'MemAvailable: 52428799 kB\n' >"$meminfo_path"
    ! sample_p5_gate memory-below "$sampler_root/memory-below.tsv"
    printf 'MemAvailable: 52428800 kB\n' >"$meminfo_path"
    printf '16.01 0.00 0.00 1/1 1\n' >"$loadavg_path"
    ! sample_p5_gate load-above "$sampler_root/load-above.tsv"
    printf '16.00 0.00 0.00 1/1 1\n' >"$loadavg_path"
    printf 'some avg10=1.00 avg60=0.00 avg300=0.00 total=0\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=0\n' \
        >"$memory_psi_path"
    ! sample_p5_gate memory-some-equal "$sampler_root/memory-some-equal.tsv"
    printf 'some avg10=0.99 avg60=0.00 avg300=0.00 total=0\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=0\n' \
        >"$memory_psi_path"
    printf 'some avg10=0.00 avg60=5.00 avg300=0.00 total=0\n' >"$cpu_psi_path"
    ! sample_p5_gate cpu-equal "$sampler_root/cpu-equal.tsv"
    printf 'some avg10=0.00 avg60=4.99 avg300=0.00 total=0\n' >"$cpu_psi_path"
    degrade_load=1
    ! sample_p5_gate final-degradation "$sampler_root/final-degradation.tsv"
    grep -Fq $'\tgate-changed-during-sample' "$sampler_root/final-degradation.tsv"
    degrade_load=0
    printf '16.00 0.00 0.00 1/1 1\n' >"$loadavg_path"
    increment_swap=1
    ! sample_p5_gate swap-delta "$sampler_root/swap-delta.tsv"
    grep -Fq $'\tswap-in-detected' "$sampler_root/swap-delta.tsv"
    increment_swap=0
    printf 'pswpin 10\n' >"$vmstat_path"

    : >"$seed3/.capture-reservations/active.reserve"
    ! all_corpora_quiet
    rm -f -- "$seed3/.capture-reservations/active.reserve"
    bash -c 'while :; do sleep 1; done' run_native_conversion_prepass.sh &
    writer_pid=$!
    ! all_corpora_quiet
    kill "$writer_pid"
    wait "$writer_pid" 2>/dev/null || true

    # A mid-acquisition collision rolls every earlier gate lock back.
    exec {collision_fd}>"$seed3/.distributed-collector.lock"
    flock "$collision_fd"
    ! acquire_gate_locks
    exec {probe_fd}>"$final_outer_lock"
    flock -n "$probe_fd"
    eval "exec ${probe_fd}>&-"
    eval "exec ${collision_fd}>&-"
)

bash -n -- "$orchestrator"
bash -n -- "$0"
printf 'schema16 existing-corpora orchestrator preflight tests passed\n'
