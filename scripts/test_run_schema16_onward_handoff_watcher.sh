#!/usr/bin/env bash
set -euo pipefail

repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
watcher="$repository/scripts/run_schema16_onward_handoff_watcher.sh"
mkdir -p -- "$repository/.codex-tmp"
test_root=$(mktemp -d "$repository/.codex-tmp/schema16-handoff-test.XXXXXX")
cleanup() { rm -rf -- "$test_root"; }
trap cleanup EXIT

workspace="$test_root/workspace"
fake_bin="$test_root/fake-bin"
mkdir -p -- "$workspace/scripts" "$workspace/original-code/scripts" "$fake_bin"
ln -s -- "$repository/scripts/run_schema16_onward_corpus_controller.sh" \
    "$workspace/scripts/run_schema16_onward_corpus_controller.sh"
ln -s -- "$repository/scripts/run_schema16_existing_corpora_orchestrator.sh" \
    "$workspace/scripts/run_schema16_existing_corpora_orchestrator.sh"
ln -s -- "$repository/original-code/scripts/capture_parity_save_replays.sh" \
    "$workspace/original-code/scripts/capture_parity_save_replays.sh"
for script in run_native_conversion_prepass.sh run_schema16_final_validation.sh \
    run_parity_release_sweep.sh; do
    ln -s -- "$repository/scripts/$script" "$workspace/scripts/$script"
done

recorder="$workspace/recorder"
printf '#!/usr/bin/env bash\nexit 0\n' >"$recorder"
chmod +x -- "$recorder"
recorder_sha=$(sha256sum -- "$recorder"); recorder_sha=${recorder_sha%% *}
bundle="$workspace/runner-bundle"
mkdir -p -- "$bundle"
printf '#!/usr/bin/env bash\nexit 0\n' >"$bundle/original_parity_replay"
chmod +x -- "$bundle/original_parity_replay"
printf 'NATIVE_CONVERSION_PROTOCOL=2\n' >"$bundle/PROVENANCE.txt"
printf 'library\n' >"$bundle/libfake.so"
(
    cd -- "$bundle"
    sha256sum -- original_parity_replay PROVENANCE.txt >SHA256SUMS
    sha256sum -- libfake.so >LIB_SHA256SUMS
)
runner_sha=$(sha256sum -- "$bundle/original_parity_replay"); runner_sha=${runner_sha%% *}
main_sha=$(sha256sum -- "$bundle/SHA256SUMS"); main_sha=${main_sha%% *}
lib_sha=$(sha256sum -- "$bundle/LIB_SHA256SUMS"); lib_sha=${lib_sha%% *}
bundle_trust=$(printf 'schema16-runner-bundle-v1\nSHA256SUMS=%s\nLIB_SHA256SUMS=%s\n' \
    "$main_sha" "$lib_sha" | sha256sum); bundle_trust=${bundle_trust%% *}

printf '%s\n' \
    '#!/usr/bin/env bash' \
    'set -euo pipefail' \
    'case "${1:-}" in' \
    'has-session)' \
    '    count=0; [[ ! -f "$FAKE_TMUX_COUNT" ]] || read -r count <"$FAKE_TMUX_COUNT"' \
    '    if (( count < FAKE_TMUX_ACTIVE_CALLS )); then printf "%s\n" "$((count + 1))" >"$FAKE_TMUX_COUNT"; exit 0; fi' \
    '    exit 1' \
    '    ;;' \
    'list-panes) printf "%s\n" "$FAKE_TMUX_PANE_COMMAND" ;;' \
    '*) exit 64 ;;' \
    'esac' >"$fake_bin/tmux"
chmod +x -- "$fake_bin/tmux"
printf '%s\n' \
    '#!/usr/bin/env bash' \
    'set -euo pipefail' \
    'printf "sleep=%s\\n" "$1" >>"$FAKE_SLEEP_LOG"' >"$fake_bin/sleep"
chmod +x -- "$fake_bin/sleep"
printf '%s\n' \
    '#!/usr/bin/env bash' \
    'set -euo pipefail' \
    'printf "%q\\n" "$@" >"$FAKE_ENV_PROOF"' \
    'exit "${FAKE_ENV_STATUS:-0}"' >"$fake_bin/env"
chmod +x -- "$fake_bin/env"

existing_session=schema16-existing-all-p5-fixture

make_summaries() {
    local audit=$1 pattern=$2 seed exact
    mkdir -p -- "$audit"
    for seed in 2 3 4; do
        case "$pattern" in
        all-exact) exact=1 ;;
        one-nonexact) if [[ "$seed" == 2 ]]; then exact=0; else exact=1; fi ;;
        two-nonexact) if [[ "$seed" == 4 ]]; then exact=1; else exact=0; fi ;;
        all-nonexact) exact=0 ;;
        *) return 64 ;;
        esac
        printf 'SEED=%s\nEXACT_PARITY=%s\nAUDIT=%s/proof-seed%s\n' \
            "$seed" "$exact" "$audit" "$seed" >"$audit/final-seed${seed}.env"
    done
}

run_case() {
    local name=$1 phase=$2 pattern=$3 expected_status=$4 pane_mode=${5:-good}
    local case_root="$test_root/$name" existing="$workspace/fixtures/$name-existing"
    local onward="$workspace/fixtures/$name-onward" handoff="$workspace/fixtures/$name-handoff"
    local count_file="$case_root/tmux-count" sleep_log="$case_root/sleep.log"
    local env_proof="$case_root/env-proof" session_log="$case_root/session.log"
    local status_tmp="$case_root/session.status.tmp" status_file="$case_root/session.status"
    local pane_command rc=0
    mkdir -p -- "$case_root"
    make_summaries "$existing" "$pattern"
    printf 'PHASE=%s\n' "$phase" >"$existing/state.env"
    pane_command="env SCHEMA16_ORCH_PREPASS_JOBS=5 SCHEMA16_ORCH_SWEEP_CONCURRENCY=8 $workspace/scripts/run_schema16_existing_corpora_orchestrator.sh $workspace $bundle $bundle_trust $runner_sha seed2 seed3 seed4 $existing"
    [[ "$pane_mode" == good ]] || pane_command='wrong-controller --wrong-audit'
    set +e
    PATH="$fake_bin:/usr/bin:/bin" \
        FAKE_TMUX_COUNT="$count_file" FAKE_TMUX_ACTIVE_CALLS=2 \
        FAKE_TMUX_PANE_COMMAND="$pane_command" FAKE_SLEEP_LOG="$sleep_log" \
        FAKE_ENV_PROOF="$env_proof" \
        "$watcher" "$workspace" "$existing_session" "$existing" "$onward" \
            "$handoff" "$recorder" "$recorder_sha" "$bundle" "$bundle_trust" \
            "$runner_sha" >>"$session_log" 2>&1
    rc=$?
    set -e
    printf '%s\n' "$rc" >"$status_tmp"
    mv -f -- "$status_tmp" "$status_file"
    mapfile -t published_status <"$status_file"
    [[ ! -e "$status_tmp" && ${#published_status[@]} == 1 \
        && "${published_status[0]}" == "$expected_status" ]]
    if [[ "$expected_status" == 0 ]]; then
        local expected_env="$case_root/expected-env-proof"
        [[ -f "$env_proof" ]]
        grep -Fxq 'sleep=300' "$sleep_log"
        grep -Fxq 'PHASE=exec-onward-controller' "$handoff/state.env"
        printf '%q\n' \
            -i PATH=/usr/bin:/bin LC_ALL=C TZ=UTC HOME=/root \
            SCHEMA16_ONWARD_CAPTURE_JOBS=8 \
            SCHEMA16_ONWARD_CAPTURE_CONVERT_JOBS=3 \
            SCHEMA16_ONWARD_PREPASS_JOBS=5 \
            SCHEMA16_ONWARD_SWEEP_CONCURRENCY=8 \
            "$workspace/scripts/run_schema16_onward_corpus_controller.sh" \
            "$workspace" "$recorder" "$recorder_sha" "$bundle" "$bundle_trust" \
            "$runner_sha" "$existing" "$onward" >"$expected_env"
        cmp -s -- "$expected_env" "$env_proof"
    else
        [[ ! -e "$env_proof" ]]
    fi
    printf '%s\n' "$case_root"
}

run_case exact complete-all-exact all-exact 0 >/dev/null
run_case one-nonexact complete-with-1-nonexact-corpora one-nonexact 0 >/dev/null
run_case nonexact complete-with-2-nonexact-corpora two-nonexact 0 >/dev/null
run_case three-nonexact complete-with-3-nonexact-corpora all-nonexact 0 >/dev/null
run_case operational normalize-seed4 all-nonexact 2 >/dev/null
run_case wrong-pane complete-all-exact all-exact 2 wrong >/dev/null

run_prepared_failure() {
    local name=$1 existing=$2 case_root handoff onward rc=0
    case_root="$test_root/$name-prepared"
    handoff="$workspace/fixtures/$name-prepared-handoff"
    onward="$workspace/fixtures/$name-prepared-onward"
    local pane="env SCHEMA16_ORCH_PREPASS_JOBS=5 SCHEMA16_ORCH_SWEEP_CONCURRENCY=8 $workspace/scripts/run_schema16_existing_corpora_orchestrator.sh $workspace $bundle $bundle_trust $runner_sha seed2 seed3 seed4 $existing"
    mkdir -p -- "$case_root"
    set +e
    PATH="$fake_bin:/usr/bin:/bin" \
        FAKE_TMUX_COUNT="$case_root/tmux-count" FAKE_TMUX_ACTIVE_CALLS=1 \
        FAKE_TMUX_PANE_COMMAND="$pane" FAKE_SLEEP_LOG="$case_root/sleep" \
        FAKE_ENV_PROOF="$case_root/env-proof" \
        "$watcher" "$workspace" "$existing_session" "$existing" "$onward" \
            "$handoff" "$recorder" "$recorder_sha" "$bundle" "$bundle_trust" \
            "$runner_sha" >"$case_root/session.log" 2>&1
    rc=$?
    set -e
    printf '%s\n' "$rc" >"$case_root/session.status.tmp"
    mv -f -- "$case_root/session.status.tmp" "$case_root/session.status"
    mapfile -t published_status <"$case_root/session.status"
    [[ ! -e "$case_root/session.status.tmp" && ${#published_status[@]} == 1 \
        && "${published_status[0]}" == 2 && ! -e "$case_root/env-proof" ]]
}

make_summaries "$workspace/fixtures/missing-existing" all-exact
printf 'PHASE=complete-all-exact\n' >"$workspace/fixtures/missing-existing/state.env"
rm -f -- "$workspace/fixtures/missing-existing/final-seed4.env"
run_prepared_failure missing "$workspace/fixtures/missing-existing"

make_summaries "$workspace/fixtures/malformed-existing" all-exact
printf 'PHASE=complete-all-exact\n' >"$workspace/fixtures/malformed-existing/state.env"
printf 'SEED=3\nEXACT_PARITY=broken\nAUDIT=proof\n' \
    >"$workspace/fixtures/malformed-existing/final-seed3.env"
run_prepared_failure malformed "$workspace/fixtures/malformed-existing"

make_summaries "$workspace/fixtures/extra-existing" all-exact
printf 'PHASE=complete-all-exact\n' >"$workspace/fixtures/extra-existing/state.env"
printf 'SEED=5\nEXACT_PARITY=1\nAUDIT=proof\n' \
    >"$workspace/fixtures/extra-existing/final-seed5.env"
run_prepared_failure extra "$workspace/fixtures/extra-existing"

# A deployed dependency drift fails before pane inspection or onward exec.
drift_workspace="$test_root/drift-workspace"
cp -a -- "$workspace" "$drift_workspace"
rm -f -- "$drift_workspace/scripts/run_schema16_onward_corpus_controller.sh"
printf '#!/usr/bin/env bash\nexit 0\n' >"$drift_workspace/scripts/run_schema16_onward_corpus_controller.sh"
chmod +x -- "$drift_workspace/scripts/run_schema16_onward_corpus_controller.sh"
make_summaries "$drift_workspace/existing" all-exact
printf 'PHASE=complete-all-exact\n' >"$drift_workspace/existing/state.env"
set +e
PATH="$fake_bin:/usr/bin:/bin" FAKE_TMUX_COUNT="$test_root/drift-count" \
    FAKE_TMUX_ACTIVE_CALLS=1 FAKE_TMUX_PANE_COMMAND=unused \
    FAKE_SLEEP_LOG="$test_root/drift-sleep" FAKE_ENV_PROOF="$test_root/drift-env" \
    "$watcher" "$drift_workspace" "$existing_session" "$drift_workspace/existing" \
    "$drift_workspace/onward" "$drift_workspace/handoff" "$drift_workspace/recorder" \
    "$recorder_sha" "$drift_workspace/runner-bundle" "$bundle_trust" "$runner_sha" \
    >/dev/null 2>&1
drift_rc=$?
set -e
[[ "$drift_rc" == 2 && ! -e "$test_root/drift-env" ]]

bash -n -- "$watcher" "$0"
printf 'schema16 onward handoff watcher fake-harness tests passed\n'
