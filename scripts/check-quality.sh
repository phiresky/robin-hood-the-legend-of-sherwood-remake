#!/usr/bin/env bash
# Named local/CI suites. Keep package selections explicit: bare Cargo commands
# intentionally select only the workspace's two small default members.
set -euo pipefail
repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd -- "$repository"

if (( $# != 1 )); then
    printf 'usage: bash scripts/check-quality.sh SUITE\n' >&2
    printf 'suites: format core scripting-llvm engine assets protocols services parity client client-release tools wasm browser-audio native-lifecycle tooling web editor editor-browser gpu gpu-gl host fixtures-demo fixtures-fullgame\n' >&2
    exit 2
fi

case "$1" in
    format)
        cargo fmt --all -- --check
        # The shared fixture helper is included by test macros rather than mod.
        git ls-files -z -- 'test-support/original_data.rs' \
            | xargs -0 -r rustfmt --edition 2024 --check --config skip_children=true
        ;;
    core)
        cargo test --locked -p robin_util -p robin_state_hash_derive -p robin_spellforge -p robin_lua
        ;;
    scripting-llvm)
        # LLVM verifies unwind/destructor semantics, not just panic detection.
        # Cranelift cannot continue through the poison-recovery catch_unwind.
        # Select only fixture-free cases, not every ignored corpus test.
        cargo test --locked -p robin_spellforge --lib poisoned_runtime_rebuilds_from_committed_history \
            --config 'profile.test.package.robin_spellforge.codegen-backend="llvm"' -- --ignored
        cargo test --locked -p robin_lua --test natives_smoke panic_unwind_detaches_native_session \
            --config 'profile.test.package.robin_lua.codegen-backend="llvm"'
        ;;
    engine) cargo test --locked -p robin_engine ;;
    assets)
        cargo test --locked -p robin_content
        cargo test --locked -p robin_content --features simulation-codecs
        cargo test --locked -p robin_assets -p robin_data_io
        cargo test --locked -p robin_assets --no-default-features
        python3 scripts/check_asset_boundary.py
        ;;
    protocols)
        cargo test --locked -p robin_run_protocol -p robin_replay_format -p robin_official_content -p robin_ranked_verification -p robin_identity_signer
        ;;
    services)
        cargo test --locked -p robin_highscores -p robin_manifest_tool -p robin_replay_verifier
        ;;
    parity) cargo test --locked -p robin_parity ;;
    client)
        cargo test --locked -p robin_rs
        cargo build --locked -p robin_rs --bin robin
        ;;
    client-release)
        cargo test --locked -p robin_rs --lib --no-default-features --features release
        cargo build --locked -p robin_rs --bin robin --no-default-features --features release
        cargo check --locked -p robin_rs --example audio_decode_bench --no-default-features --features release
        ;;
    tools)
        cargo test --locked -p robin_rs --features tools --bin convert_datadir --bin dump_level
        cargo test --locked -p robin_rs --no-default-features --features projection-export --example export_simulation_content
        cargo check --locked -p robin_rs --features tools,projection-export --bins --examples
        ;;
    wasm)
        cargo check --locked --profile wasm-dev --target wasm32-unknown-unknown -p robin_identity_signer --bin leaderboard_identity_bridge --features identity-signer-bridge
        cargo check --locked --profile wasm-dev --target wasm32-unknown-unknown -p robin_replay_admission_wasm
        cargo check --locked --profile wasm-dev --target wasm32-unknown-unknown -p robin_rs --bin robin --no-default-features
        ;;
    browser-audio|native-lifecycle)
        # This delegated gate is the entire suite. Do not retain a suspended
        # shell reader while a long compilation/runtime command is executing.
        exec python3 scripts/validation/lifecycle_gate.py "$1"
        ;;
    gpu)
        # This test also invokes renderer::verify_offscreen_gpu_contract.
        WGPU_BACKEND=vulkan cargo test --locked -p robin_rs --lib gpu_upscale::tests::headless_downlevel_device_executes_every_multipass_profile -- --ignored --exact
        ;;
    gpu-gl)
        WGPU_BACKEND=gl xvfb-run -a cargo test --locked -p robin_rs --lib gpu_upscale::tests::headless_downlevel_device_executes_every_multipass_profile -- --ignored --exact
        ;;
    host)
        cargo test --locked -p robin_rs --lib --no-default-features --features hardware-info hardware::tests::native_memory_query_reports_real_total -- --ignored --exact
        ;;
    tooling)
        # Curated fixture-free tests: never discover arbitrary scripts that may
        # capture real game sessions or operate a deployed service.
        python3 scripts/test_quality_suites.py
        python3 scripts/release/test_author_leaderboard_release.py
        test -f scripts/release/test_publish_native_release.py
        python3 -m unittest discover -s scripts/release -p test_publish_native_release.py
        test -f scripts/test_portable_linker.py
        python3 -m unittest discover -s scripts -p test_portable_linker.py
        test -f scripts/validation/save_load_live_test.py
        python3 -m unittest discover -s scripts/validation -p save_load_live_test.py
        python3 -m unittest discover -s scripts/validation -p lifecycle_gate_test.py
        bash scripts/test_parity_orchestration.sh
        ;;
    web) pnpm --dir wasm-www verify:web ;;
    editor) pnpm --dir level-editor verify ;;
    editor-browser)
        : "${CHROME:?Set CHROME to the Chrome/Chromium executable}"
        "$CHROME" --version
        pnpm --dir level-editor build:browser
        if curl --silent --max-time 1 http://127.0.0.1:5181/tests/lifecycle.html >/dev/null; then
            printf 'port 5181 already serves HTTP; stop that preview before running the browser gate\n' >&2
            exit 1
        fi
        preview_log=$(mktemp "${TMPDIR:-/tmp}/robin-editor-preview.XXXXXX")
        preview_pid=
        cleanup_preview() {
            if [[ -n "$preview_pid" ]]; then
                kill -- "-$preview_pid" 2>/dev/null || true
                for _attempt in {1..10}; do
                    kill -0 "$preview_pid" 2>/dev/null || break
                    sleep 0.1
                done
                kill -KILL -- "-$preview_pid" 2>/dev/null || true
                wait "$preview_pid" 2>/dev/null || true
            fi
            cat -- "$preview_log"
            rm -f -- "$preview_log"
        }
        trap cleanup_preview EXIT
        trap 'exit 130' INT
        trap 'exit 143' TERM
        setsid pnpm --dir level-editor serve:browser >"$preview_log" 2>&1 &
        preview_pid=$!
        preview_ready=0
        for _attempt in {1..30}; do
            if curl --fail --silent --max-time 1 http://127.0.0.1:5181/tests/lifecycle.html >/dev/null; then
                preview_ready=1
                break
            fi
            if ! kill -0 "$preview_pid" 2>/dev/null; then
                printf 'editor preview exited before becoming ready\n' >&2
                exit 1
            fi
            sleep 1
        done
        if [[ "$preview_ready" != 1 ]]; then
            printf 'editor preview did not become ready\n' >&2
            exit 1
        fi
        timeout --signal=TERM --kill-after=10s 120s pnpm --dir level-editor test:browser
        ;;
    fixtures-demo)
        : "${ROBINHOOD_DATA_DIR:?Set ROBINHOOD_DATA_DIR to the Leicester demo root containing Data/}"
        cargo test --locked -p robin_engine --lib profiles::tests::load_demo_profile_json -- --ignored --exact
        cargo test --locked -p robin_engine --lib profiles::tests::demo_profile_serde_round_trip -- --ignored --exact
        cargo test --locked -p robin_rs --lib font::tests::test_parse_real_tfn -- --ignored --exact
        cargo test --locked -p robin_assets --lib frame_holder::tests::test_initialize_sprite_bank_from_game_data -- --ignored --exact
        cargo test --locked -p robin_assets --lib frame_holder::tests::test_sprite_bank_packed_data_present -- --ignored --exact
        cargo test --locked -p robin_assets --lib frame_holder::tests::test_validate_all_sprite_bank_streams -- --ignored --exact
        cargo test --locked -p robin_assets demo_script -- --ignored
        ;;
    fixtures-fullgame)
        : "${ROBINHOOD_DATA_DIR:?Set ROBINHOOD_DATA_DIR to the full-game root containing Data/}"
        cargo test --locked -p robin_engine --lib profiles::tests::load_fullgame_profile_json -- --ignored --exact
        cargo test --locked -p robin_assets fullgame_scripts -- --ignored
        ;;
    *) printf 'unknown quality suite: %s\n' "$1" >&2; exit 2 ;;
esac
