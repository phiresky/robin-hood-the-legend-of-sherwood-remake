#!/usr/bin/env bash
set -euo pipefail

repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
controller="$repository/scripts/run_schema16_onward_corpus_controller.sh"
mkdir -p -- "$repository/.codex-tmp"
test_root=$(mktemp -d "$repository/.codex-tmp/schema16-onward-controller-test.XXXXXX")
cleanup() { rm -rf -- "$test_root"; }
trap cleanup EXIT

# Source the controller's state machine and replace only its expensive process
# boundaries. This fake harness exercises durable campaign creation, serial
# mismatch advancement, exact termination, preservation and resume behavior.
unset SCHEMA16_ONWARD_CAPTURE_CONVERT_JOBS SCHEMA16_ONWARD_PREPASS_JOBS
source "$controller" unused unused \
    0000000000000000000000000000000000000000000000000000000000000000 \
    unused 0000000000000000000000000000000000000000000000000000000000000000 \
    0000000000000000000000000000000000000000000000000000000000000000 \
    unused unused
[[ "$capture_convert_jobs" == 3 && "$prepass_jobs" == 5 ]]

valid_settings() {
    capture_jobs=8
    capture_convert_jobs=3
    prepass_jobs=5
    convert_timeout=7200
    sweep_concurrency=8
}

# Call validation in subshells so fail() exits only the fixture transaction.
if (valid_settings; capture_jobs=18446744073709551617; validate_controller_settings) \
    2>"$test_root/capture-jobs-overflow.err"
then
    printf 'test failure: capture-jobs overflow was accepted\n' >&2
    exit 1
fi
grep -Fq 'capture jobs must be 1 through 10' "$test_root/capture-jobs-overflow.err"
if (valid_settings; prepass_jobs=18446744073709551617; validate_controller_settings) \
    2>"$test_root/prepass-jobs-overflow.err"
then
    printf 'test failure: prepass-jobs overflow was accepted\n' >&2
    exit 1
fi
grep -Fq 'prepass jobs must be 1 through 8' "$test_root/prepass-jobs-overflow.err"
if (valid_settings; convert_timeout=18446744073709551617; validate_controller_settings) \
    2>"$test_root/timeout-overflow.err"
then
    printf 'test failure: conversion-timeout overflow was accepted\n' >&2
    exit 1
fi
grep -Fq 'conversion timeout must be 3600 through 2147483647 seconds' \
    "$test_root/timeout-overflow.err"
if (valid_settings; sweep_concurrency=18446744073709551617; validate_controller_settings) \
    2>"$test_root/sweep-overflow.err"
then
    printf 'test failure: sweep-concurrency overflow was accepted\n' >&2
    exit 1
fi
grep -Fq 'sweep concurrency must be 1 through 64' "$test_root/sweep-overflow.err"
valid_settings
capture_jobs=0008
capture_convert_jobs=03
prepass_jobs=05
convert_timeout=07200
sweep_concurrency=08
validate_controller_settings
[[ "$capture_jobs" == 8 && "$capture_convert_jobs" == 3 && "$prepass_jobs" == 5 \
    && "$convert_timeout" == 7200 && "$sweep_concurrency" == 8 ]]

workspace="$test_root/work space"
audit_root="$workspace/audits/onward"
mkdir -p -- "$audit_root"
recorder="$workspace/fake-recorder"
recorder_sha=1111111111111111111111111111111111111111111111111111111111111111
capture_jobs=2
capture_convert_jobs=1
prepass_jobs=1
first_seed=5000000
seed_step=1000000
expected_replays=9720
call_log="$test_root/calls"
: >"$call_log"

# Exercise the real capture transaction boundary with a recording executable,
# proving inherited control variables cannot alter the exact invocation.
(
    workspace="$test_root/invocation-work"
    audit_root="$workspace/audits/onward"
    campaign="$workspace/corpus"
    capture_bundle="$workspace/pinned-bundle"
    bundle="$workspace/source-bundle"
    mkdir -p -- "$audit_root" "$campaign" "$capture_bundle"
    recorder="$workspace/pinned-recorder"
    printf '#!/usr/bin/env bash\nexit 0\n' >"$recorder"
    chmod +x -- "$recorder"
    recorder_sha=$(sha256sum -- "$recorder"); recorder_sha=${recorder_sha%% *}
    printf '#!/usr/bin/env bash\nexit 0\n' >"$capture_bundle/original_parity_replay.remote"
    chmod +x -- "$capture_bundle/original_parity_replay.remote"
    capture_script="$workspace/fake-capture"
    invocation_proof="$test_root/capture-invocation.env"
    printf '%s\n' \
        '#!/usr/bin/env bash' \
        'set -euo pipefail' \
        'for variable in ROBIN_LOADER LD_LIBRARY_PATH FORCE DRY_RUN BASH_ENV ENV CAPTURE_FREE_KIB_FILE CAPTURE_UNKNOWN_FUTURE; do' \
        '    [[ ! -v "$variable" ]] || exit 91' \
        'done' \
        '{' \
        '    printf "ARG1=%s\\nARG2=%s\\nARG3=%s\\n" "$1" "$2" "$3"' \
        '    for variable in PATH LC_ALL TZ HOME TMPDIR PARITY_TRACE_SCHEMA PARITY_RANDOM_REPLAYS PARITY_FRAMES PARITY_INPUT_SEED_BASE PARITY_SEED SHERWOOD_LIMIT SHERWOOD_SAMPLE_SEED SHARD_COUNT SHARD_INDEX CAPTURE_JOBS CONVERT_JOBS COMPRESS HEADFUL SKIP_BUILD WATCHDOG_SECONDS CAPTURE_MIN_FREE_KIB CAPTURE_RESERVE_KIB CAPTURE_EMERGENCY_FREE_KIB CAPTURE_GATE_POLL_SECONDS CAPTURE_EMERGENCY_POLL_SECONDS CAPTURE_EMERGENCY_KILL_AFTER_SECONDS CAPTURE_PAUSE_FILE CAPTURE_DRAIN_FILE CAPTURE_DISK_PATH ROBIN_BINARY ROBIN_LIBRARY_DIR ROBINHOOD_DATA_DIR PARITY_CONVERTER; do' \
        '        printf "%s=%s\\n" "$variable" "${!variable}"' \
        '    done' \
        "} >$(printf %q "$invocation_proof")" \
        >"$capture_script"
    chmod +x -- "$capture_script"
    export ROBIN_LOADER=/poison/loader LD_LIBRARY_PATH=/poison/lib FORCE=1 DRY_RUN=1
    printf ': >%q\n' "$test_root/bash-env-was-sourced" >"$test_root/poison-bash-env"
    export BASH_ENV="$test_root/poison-bash-env" ENV="$test_root/poison-bash-env"
    export CAPTURE_FREE_KIB_FILE=/poison/free CAPTURE_UNKNOWN_FUTURE=poison
    export CAPTURE_JOBS=10 CAPTURE_PAUSE_FILE=/poison/pause CAPTURE_DISK_PATH=/poison/disk
    poison_path="$test_root/poison-path"
    mkdir -p -- "$poison_path"
    printf '#!/bin/sh\n: >%s\nexec /bin/bash "$@"\n' \
        "$(printf %q "$test_root/poison-bash-was-used")" >"$poison_path/bash"
    chmod +x -- "$poison_path/bash"
    export PATH="$poison_path:/usr/bin:/bin"
    capture_jobs=4
    capture_convert_jobs=2
    verify_capture_bundle() { :; }
    verify_campaign_inventory() { :; }
    write_phase() { :; }
    execute_capture_impl 5000000 "$campaign"
    grep -Fxq "ARG1=$workspace/reference-saves" "$invocation_proof"
    grep -Fxq "ARG2=$campaign" "$invocation_proof"
    grep -Fxq "ARG3=$workspace/datadirs/fullgame_linux" "$invocation_proof"
    grep -Fxq 'PATH=/usr/bin:/bin' "$invocation_proof"
    grep -Fxq 'LC_ALL=C' "$invocation_proof"
    grep -Fxq 'TZ=UTC' "$invocation_proof"
    grep -Fxq 'PARITY_TRACE_SCHEMA=16' "$invocation_proof"
    grep -Fxq 'PARITY_INPUT_SEED_BASE=5000000' "$invocation_proof"
    grep -Fxq 'CAPTURE_JOBS=4' "$invocation_proof"
    grep -Fxq 'CONVERT_JOBS=2' "$invocation_proof"
    grep -Fxq 'COMPRESS=1' "$invocation_proof"
    grep -Fxq 'HEADFUL=0' "$invocation_proof"
    grep -Fxq 'SKIP_BUILD=1' "$invocation_proof"
    grep -Fxq 'WATCHDOG_SECONDS=2700' "$invocation_proof"
    grep -Fxq 'SHERWOOD_LIMIT=30' "$invocation_proof"
    grep -Fxq 'SHERWOOD_SAMPLE_SEED=1' "$invocation_proof"
    grep -Fxq 'SHARD_COUNT=1' "$invocation_proof"
    grep -Fxq 'SHARD_INDEX=0' "$invocation_proof"
    grep -Fxq 'CAPTURE_MIN_FREE_KIB=31457280' "$invocation_proof"
    grep -Fxq 'CAPTURE_GATE_POLL_SECONDS=2' "$invocation_proof"
    grep -Fxq "CAPTURE_PAUSE_FILE=$campaign/.capture.pause" "$invocation_proof"
    grep -Fxq "CAPTURE_DISK_PATH=$campaign" "$invocation_proof"
    grep -Fxq "ROBIN_BINARY=$recorder" "$invocation_proof"
    grep -Fxq "ROBIN_LIBRARY_DIR=$workspace/original-code/runtime-i386" "$invocation_proof"
    grep -Fxq "ROBINHOOD_DATA_DIR=$workspace/datadirs/fullgame_linux" "$invocation_proof"
    grep -Fxq "PARITY_CONVERTER=$capture_bundle/original_parity_replay.remote" "$invocation_proof"
    [[ ! -e "$test_root/bash-env-was-sourced" && ! -e "$test_root/poison-bash-was-used" ]]
)

# Capture-time conversion and the post-capture native prepass have independent
# immutable concurrency. The default P5 prepass explicitly asks the audited
# prepass to admit 10 GiB per job (50 GiB total) and fails rather than downshifts.
(
    workspace="$test_root/prepass-work"
    audit_root="$workspace/audits/onward"
    campaign="$workspace/corpus"
    bundle="$workspace/bundle"
    mkdir -p -- "$audit_root" "$campaign" "$bundle"
    prepass_jobs=5
    convert_timeout=7200
    final_outer_lock="$workspace/final.lock"
    prepass_proof="$test_root/prepass-invocation.env"
    prepass_script="$workspace/fake-prepass"
    {
        printf '%s\n' '#!/usr/bin/env bash' 'set -euo pipefail'
        printf 'printf '\''ARG1=%%s\\nARG2=%%s\\nARG3=%%s\\nARG4=%%s\\nARG5=%%s\\nJOBS=%%s\\nMIN_KIB=%%s\\n'\'' "$1" "$2" "$3" "$4" "$5" "$NATIVE_CONVERT_JOBS" "$NATIVE_CONVERT_MIN_AVAILABLE_KIB_PER_JOB" >%q\n' \
            "$prepass_proof"
        printf '%s\n' 'mkdir -p -- "$5"' ': >"$5/COMPLETE"'
    } >"$prepass_script"
    chmod +x -- "$prepass_script"
    bundle_trust_sha=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
    write_phase() { :; }
    execute_prepass_impl 5000000 "$campaign"
    grep -Fxq "ARG1=$workspace" "$prepass_proof"
    grep -Fxq "ARG2=$campaign" "$prepass_proof"
    grep -Fxq "ARG3=$bundle" "$prepass_proof"
    grep -Fxq "ARG5=$audit_root/native-seed5000000-p5" "$prepass_proof"
    grep -Fxq 'JOBS=5' "$prepass_proof"
    grep -Fxq 'MIN_KIB=10485760' "$prepass_proof"
)

# Both concurrency choices and the fixed P5 per-job memory admission are part
# of immutable controller provenance.
(
    workspace="$test_root/provenance-work"
    audit_root="$workspace/audits/onward"
    mkdir -p -- "$audit_root"
    recorder_source="$workspace/source-recorder"
    recorder="$workspace/pinned-recorder"
    recorder_sha=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
    bundle="$workspace/source-bundle"
    capture_bundle="$workspace/pinned-bundle"
    bundle_trust_sha=cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc
    runner_sha=dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd
    existing_audit="$workspace/audits/existing"
    controller_script="$controller"
    capture_jobs=8
    capture_convert_jobs=3
    prepass_jobs=5
    convert_timeout=7200
    sweep_concurrency=8
    final_outer_lock="$workspace/final.lock"
    initialize_or_verify_provenance
    grep -Fxq 'CAPTURE_CONVERT_JOBS=3' "$audit_root/provenance.env"
    grep -Fxq 'PREPASS_JOBS=5' "$audit_root/provenance.env"
    grep -Fxq 'PREPASS_MIN_AVAILABLE_KIB_PER_JOB=10485760' "$audit_root/provenance.env"
    prepass_jobs=3
    if (initialize_or_verify_provenance) 2>"$test_root/prepass-provenance-drift.err"; then
        printf 'test failure: prepass concurrency provenance drift was accepted\n' >&2
        exit 1
    fi
    grep -Fq 'differs from immutable prior provenance' \
        "$test_root/prepass-provenance-drift.err"
)

# The shared pin accepts exactly the final validator's canonical bundle tree,
# rejecting mutable symlinks and unmanifested root content before publication.
bundle_fixture="$test_root/canonical-bundle"
mkdir -p -- "$bundle_fixture/lib"
for executable in original_parity_replay original_parity_replay.remote; do
    printf '#!/usr/bin/env bash\nexit 0\n' >"$bundle_fixture/$executable"
    chmod +x -- "$bundle_fixture/$executable"
done
printf '#!/usr/bin/env bash\nexit 0\n' >"$bundle_fixture/lib/ld-linux-x86-64.so.2"
chmod +x -- "$bundle_fixture/lib/ld-linux-x86-64.so.2"
printf 'NATIVE_CONVERSION_PROTOCOL=2\n' >"$bundle_fixture/PROVENANCE.txt"
printf '/lib/loader => %s/lib/ld-linux-x86-64.so.2 (0x1)\n' "$bundle_fixture" \
    >"$bundle_fixture/LOADER_LIST.txt"
(
    cd -- "$bundle_fixture"
    sha256sum -- lib/ld-linux-x86-64.so.2 >LIB_SHA256SUMS
    sha256sum -- original_parity_replay original_parity_replay.remote \
        LIB_SHA256SUMS PROVENANCE.txt LOADER_LIST.txt >SHA256SUMS
)
verify_bundle() {
    (cd -- "$1" && sha256sum --strict -c SHA256SUMS \
        && sha256sum --strict -c LIB_SHA256SUMS) >/dev/null
}
verify_capture_bundle "$bundle_fixture" "$bundle_fixture"
ln -s -- original_parity_replay "$bundle_fixture/unmanifested-link"
if (verify_capture_bundle "$bundle_fixture" "$bundle_fixture") \
    2>"$test_root/bundle-symlink.err"
then
    printf 'test failure: canonical capture bundle accepted a symlink\n' >&2
    exit 1
fi
grep -Fq 'runner bundle contains a symlink' "$test_root/bundle-symlink.err"
rm -f -- "$bundle_fixture/unmanifested-link"
printf 'extra\n' >"$bundle_fixture/unmanifested-root"
if (verify_capture_bundle "$bundle_fixture" "$bundle_fixture") \
    2>"$test_root/bundle-extra.err"
then
    printf 'test failure: canonical capture bundle accepted an extra root file\n' >&2
    exit 1
fi
grep -Fq 'root file set is not canonical' "$test_root/bundle-extra.err"

write_phase() {
    printf '%s\n' "$1" >>"$test_root/phases"
}
execute_capture() {
    printf 'capture\t%s\t%s\n' "$1" "$2" >>"$call_log"
    : >"$2/capture-preserved.marker"
}
execute_prepass() {
    printf 'normalize\t%s\t%s\n' "$1" "$2" >>"$call_log"
}
execute_final() {
    printf 'validate\t%s\t%s\n' "$1" "$2" >>"$call_log"
    if [[ "$1" == 5000000 ]]; then
        mkdir -p -- "$audit_root/collect-all-seed5000000"
        printf 'NONEXACT=7\n' >"$audit_root/collect-all-seed5000000/summary.env"
        printf 'SEED=5000000\nEXACT_PARITY=0\n' >"$audit_root/final-seed5000000.env"
        return 1
    fi
    printf 'SEED=%s\nEXACT_PARITY=1\n' "$1" >"$audit_root/final-seed${1}.env"
    return 0
}
verify_onward_summary() {
    grep -Fxq "SEED=$1" "$audit_root/final-seed${1}.env"
    grep -Fxq 'EXACT_PARITY=1' "$audit_root/final-seed${1}.env"
}

controller_loop
seed5=$(campaign_for_seed 5000000)
seed6=$(campaign_for_seed 6000000)
[[ -f "$seed5/capture-preserved.marker" && -f "$seed5/campaign.env" ]]
[[ -f "$seed6/capture-preserved.marker" && -f "$seed6/campaign.env" ]]
grep -Fxq 'CAPTURE_CONVERT_JOBS=1' "$seed5/campaign.env"
grep -Fxq 'SEED=6000000' "$audit_root/result.env"

# Retry diagnostics are copied aside before conventional session filenames are
# reused, and a pre-existing unowned corpus path cannot be silently blessed.
printf 'prior failed normalization\n' >"$audit_root/native-seed7000000-p1.session.log"
archive_session_log "$audit_root/native-seed7000000-p1.session.log" native-seed7000000-p1
grep -R -Fxq 'prior failed normalization' "$audit_root/session-log-history"
unauthenticated=$(campaign_for_seed 7000000)
mkdir -p -- "$unauthenticated"
if (initialize_campaign 7000000 "$unauthenticated") 2>"$test_root/unauthenticated.err"; then
    printf 'test failure: unauthenticated pre-existing campaign was blessed\n' >&2
    exit 1
fi
grep -Fq 'refusing unauthenticated pre-existing' "$test_root/unauthenticated.err"
grep -Fxq "CAMPAIGN=$seed6" "$audit_root/result.env"
diff -u -- <(printf 'capture\t5000000\t%s\nnormalize\t5000000\t%s\nvalidate\t5000000\t%s\ncapture\t6000000\t%s\nnormalize\t6000000\t%s\nvalidate\t6000000\t%s\n' \
    "$seed5" "$seed5" "$seed5" "$seed6" "$seed6" "$seed6") "$call_log"

# A process restart authenticates the committed exact result and performs no
# recorder, conversion or validator work again.
: >"$call_log"
controller_loop
[[ ! -s "$call_log" ]]
rm -f -- "$audit_root/result.env"
controller_loop
[[ ! -s "$call_log" ]]
grep -Fxq 'SEED=6000000' "$audit_root/result.env"

# Resuming an initialized campaign accepts byte-identical metadata and refuses
# an operational-knob change rather than silently changing corpus provenance.
initialize_campaign 5000000 "$seed5"
capture_jobs=3
if (initialize_campaign 5000000 "$seed5") 2>"$test_root/drift.err"; then
    printf 'test failure: campaign metadata drift was accepted\n' >&2
    exit 1
fi
grep -Fq 'campaign metadata differs from its immutable prior invocation' "$test_root/drift.err"
capture_jobs=2

# An operational capture failure halts at the same seed. It must neither
# normalize/validate that incomplete corpus nor create the next campaign.
failure_workspace="$test_root/failure work"
workspace="$failure_workspace"
audit_root="$workspace/audits/onward"
mkdir -p -- "$audit_root"
call_log="$test_root/failure-calls"
: >"$call_log"
execute_capture() {
    printf 'capture\t%s\n' "$1" >>"$call_log"
    return 23
}
execute_prepass() { printf 'unexpected-normalize\n' >>"$call_log"; }
execute_final() { printf 'unexpected-validate\n' >>"$call_log"; }
if (controller_loop) 2>"$test_root/failure.err"; then
    printf 'test failure: operational capture failure was ignored\n' >&2
    exit 1
fi
grep -Fxq $'capture\t5000000' "$call_log"
[[ ! -e "$(campaign_for_seed 6000000)" ]]

# The prerequisite gate distinguishes an authenticated semantic status 1 from
# missing/corrupt status 2; only three exact status-1 proofs unlock seed 5.
verify_prior_summary() {
    [[ "$1" != 3 ]] || return 2
    return 1
}
if (verify_prior_proof_gate) 2>"$test_root/prior-missing.err"; then
    printf 'test failure: missing prior summary unlocked onward capture\n' >&2
    exit 1
fi
grep -Fq 'seed 3 prerequisite proof is missing or invalid' "$test_root/prior-missing.err"
verify_prior_summary() { return 1; }
prior_rc=0
verify_prior_proof_gate || prior_rc=$?
[[ "$prior_rc" == 1 && "$prior_nonexact" == 3 ]]

# A pre-created symlink cannot impersonate the immutable recorder pin.
workspace="$test_root/pin-work"
mkdir -p -- "$workspace/.git/schema16-onward-recorders"
printf '#!/bin/sh\nexit 0\n' >"$test_root/pin-source"
chmod +x -- "$test_root/pin-source"
recorder_sha=$(sha256sum -- "$test_root/pin-source"); recorder_sha=${recorder_sha%% *}
pin_dir="$workspace/.git/schema16-onward-recorders/$recorder_sha"
mkdir -p -- "$pin_dir"
ln -s -- "$test_root/pin-source" "$pin_dir/robin"
if (pin_recorder "$test_root/pin-source") 2>"$test_root/pin-symlink.err"; then
    printf 'test failure: recorder pin accepted a symlink\n' >&2
    exit 1
fi
grep -Fq 'pinned recorder directory is malformed or changed' "$test_root/pin-symlink.err"

# Prior provenance is read literally, never evaluated. Only absolute path text
# unchanged by the deployed helper's %q serialization is accepted.
existing_audit="$test_root/prior-path-audit"
mkdir -p -- "$existing_audit"
printf 'SEED2=/home/example\\ path/seed2\n' >"$existing_audit/provenance.env"
if (read_prior_campaign 2) 2>"$test_root/prior-path.err"; then
    printf 'test failure: escaped prior SEED path was accepted\n' >&2
    exit 1
fi
grep -Fq 'provenance path is not a canonical absolute path' "$test_root/prior-path.err"
printf 'SEED2=/home/example-path/seed2\n' >"$existing_audit/provenance.env"
[[ "$(read_prior_campaign 2)" == /home/example-path/seed2 ]]

bash -n -- "$controller"
bash -n -- "$0"
printf 'schema16 onward corpus controller fake-harness tests passed\n'
