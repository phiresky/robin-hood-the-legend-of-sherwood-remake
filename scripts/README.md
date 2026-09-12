# Developer scripts

Run scripts from the repository root unless their usage says otherwise.
Tests named `test_<driver>.sh` or `test_<module>.py` exercise the corresponding
driver/module with isolated fixtures; they do not imply permission to run a
live corpus or deployment. See `docs/TESTING.md` for package and fixture gates.

## Build and quality gates

- `check-quality.sh`: named CI/local quality suites and explicit fixture gates.
- `check_asset_boundary.py`: engine/assets ownership boundary checks.
- `build-native.sh`: build the native client and matching replay admission helper.
- `build-wasm-threads.sh`: supported threaded browser build wrapper; not obsolete.
- `build_identity_signer.sh`: isolated browser identity-signing module.
- `build_web_shipping_datadir.sh`: browser shipping-data generation.
- `clang-wild-linker.sh`: native linker driver with explicit override support.
- `install_pinned_wasm_bindgen.sh`, `install_pinned_wasm_tools.sh`: install pinned browser tool versions.
- `assert-replay-admission-wasm-memory.sh`: verify the isolated admission memory ceiling.
- `replay-admission-wasm.cargo-config.toml`, `wasm-threads.cargo-config.toml`: distinct WASM build policies.
- `test_quality_suites.py`, `test_portable_linker.py`: suite routing and linker portability regressions.
- `release/`: native release authoring/publishing and its tests; see its own README.

## Replay evidence and campaigns

These scripts may launch long-running processes or write evidence. Read their
usage and select explicit inputs/output roots before running them. The entire
`reference-saves/` tree is a capture input, not just the named Rust fixture.
The separate ignored `binaries/` artifact checkout supplies pinned recorders;
it is not a source submodule. Never silently rebuild a recorder whose hash is
part of campaign provenance.

- `parity_campaign.py`, `parity-campaigns/*.json`: campaign manifests and campaign execution.
- `parity_result.py`: typed runner result parsing/classification.
- `replay_schema.py`: shared recording schema checks.
- `replay_evidence.py`: bounded, verified evidence inputs shared by importers.
- `replay_state_db.py`: replay inventory/evidence database commands.
- `audit_replay_payload.py`: inspect replay payload coverage.
- `run_parity_fixture_gate.py`: explicit parity fixture acceptance gate.
- `capture_parity_subset.sh`: manually selected Original save capture.
- `run_parity_release_sweep.sh`: release-runner trace sweep.
- `run_incremental_eof_checks.sh`: incremental exact-EOF validation with runner identity.
- `run_native_conversion_prepass.sh`: convert corpus recordings to native artifacts.
- `run_native_reblock_snapshot.sh`: reblock snapshot artifacts with evidence.
- `run_corpus_work_supervised.sh`: bounded supervision of corpus work.
- `run_distributed_replay_worker.sh`: execute a worker's assigned replay cases.
- `run_replay_refill_controller.sh`: refill available replay worker capacity.
- `run_schema16_corpus_ladder.sh`: restart-aware sequential capture ladder.
- `run_schema16_distributed_capture.sh`: disjoint local/remote save-shard capture and collection.
- `run_schema16_existing_corpora_orchestrator.sh`: coordinate already captured corpora.
- `run_schema16_final_validation.sh`: final validation and evidence publication.
- `run_schema16_onward_corpus_controller.sh`: advance capture/validation campaigns.
- `run_schema16_onward_handoff_watcher.sh`: watch and hand off completed capture work.
- `test_parity_orchestration.sh`: aggregate orchestration regression suite.
- `test_parity_result.py`, `test_replay_state_db.py`, `test_run_*.py`, `test_run_*.sh`: isolated module/driver regressions.

The schema16 ladder/distributed scripts remain live callers of each other and
of shared evidence tooling. Their historical recorder hashes and seed defaults
are provenance, not permission to capture arbitrary new inputs. Their workspace
defaults resolve from the script location; override `SCHEMA16_*_WORKSPACE` and
audit directories explicitly for a different checkout/campaign.

The completed motion-state worktree driver and fixed 98-case schema15 replacement
validator were removed. Use the current manifest/evidence-based drivers above;
old verdict text alone is not current acceptance evidence.

## Asset authoring and standalone diagnostics

These are explicit developer tools, not automatically launched production
code. Keep standalone tools discoverable instead of treating no importers as
proof of dead code. Most require external game data or Python imaging packages.

- `build_robin_hood_engineer_sprites.py`: export atlases to an explicitly chosen external Factorio mod graphics directory.
- `convert_native_fonts_to_woff2.py`: convert original fonts; the tracked [font specimen](../web-font-specimen/index.html) is a manual visual comparison page.
- `generate_ferris_overlay.py`: generate the local Ferris overlay assets.
- `generate_missing_knights.py`: reconstruct missing mounted-knight colour families.
- `import_fabri18_sprites.py`: import an external sprite collection.
- `optimize_mod_pngs.py`: optimize authored mod PNGs.
- `profile_patch_tools.py`, `test_profile_patch_tools.py`: profile patch authoring and regressions.
- `validate_sprite_mods.py`: sprite-mod consistency checks.
- `sprite_compress_atlas.sh`, `sprite_compress_streams.sh`: sprite compression experiments.
- `render_all_mission_maps.sh`: batch map rendering with external game data.
- `extract_dump_entity.py`: inspect one entity from a diagnostic dump.
- `find_unreferenced_rust_items.py`: advisory source scan; references through macros/features need manual confirmation.
- `benchmark_fog_history.py`: fog-history benchmark driver.

## Browser measurement and experiments

- `wasm_decode_bench.mjs`, `wasm_decode_bench_chrome.mjs`: decoder measurements in Node/browser.
- `wasm_mission_install_chrome.mjs`: mission-install browser measurements.
- `wasm_production_startup_chrome.mjs`: production startup probe; see the adjacent Markdown usage.
- `startup_throttle.mjs`, `startup_throttle.test.mjs`: shared startup throttling and its tests.
- `capture_wasm_http_brotli.mjs`: HTTP/Brotli capture probe; see the adjacent Markdown usage.
- `wasm-precompressed-worker.mjs`: explicitly experimental precompressed-worker probe, not a deployment entrypoint.

## Live client validation

`validation/` contains explicit environment-dependent probes, not unit tests:

- `briefing_x11.py`, `namespace_x11.py`, `client_x11.py`: X11 UI probes/helpers.
- `browser-game.mjs`: browser game harness.
- `capture_live.py`, `client_frames.py`, `frame_steps_live.py`: capture and frame-step probes.
- `client_modal_flow.py`, `client_soak.py`: modal workflows and sustained runs.
- `input_worker.py`: input injection helper.
- `lifecycle_gate.py`, `window_close.py`: startup/shutdown lifecycle checks.
- `multiplayer_live.py`: multiplayer scenarios.
- `parity_sweep.py`: bounded client replay sweep.
- `replay_history_fixture.py`: replay-history fixture generation.
- `runtime_evidence.py`: runtime evidence capture/verification.
- `save_load_live.py`: save/load scenarios.
- `*_test.py`: isolated harness regressions for the corresponding modules.

## Agent launcher

`../claude-worktree` creates `.worktrees/<name>` on branch `<name>` from `main`
and launches the chosen agent in tmux. Invoke it from the main checkout, pass
the prompt on stdin, and set `AGENT=codex` when appropriate. Cache/data paths
respect `CARGO_HOME` and the XDG directories. Agent sessions and merge helpers
are local tooling; no repository-wide stash operation is safe across worktrees.
