# Argument bundling candidates

Policy: the workspace keeps `clippy::too_many_arguments = "allow"` in the root
`Cargo.toml`. Bundling arguments into context structs is done case by case, when a
struct makes a family clearer. The lint does not drive it, and per-function
`#[allow]` annotations are not added. Bundles already applied: `EntitySeekRequest`
(seek wait), `LiveArchiveAssets`, `AutosaveRequest`, `StepWorld`
(`run_forward_ticks`), `MenuRect` (menu text-box and surface helpers), and
`AcceptanceManifests`.

Families found in the September 2026 signature survey, not bundled yet
(call-site counts are approximate):

| Family | Defining file(s) | Call sites | Why bundle |
|---|---|---|---|
| Gate A* route-search query (`find_path_gates*`, `find_path_into_door*`, `find_path_to_door`, `compute_avenger_wait_position`) | `robin_engine/src/gate.rs` | ~40 | Shared doors/source/auth/callback inputs across engine, natives and tests |
| Sprite profile loaders (`load_frame_info`, `load_alternate_profile`, `apply_sprite_info`) | `robin_engine/src/sprite.rs` | ~17 | Seven shared profile-load params, mostly from `engine/level_loading` |
| `ScriptVmCall` (`call_script_vm_inner*`, `call_scb_script_vm_inner`, `call_spellforge_vm_inner`, `drive_started_script_vm`) | `robin_engine/src/engine/script.rs` | ~10 | Same key/fn_name/params/frame/active set; parity-critical hot path |
| `AiContextInputs` (`init_one_ai`, `build_ai_context_from_entity`) | `robin_engine/src/engine/ai/{initialization,mod}.rs` | ~4 | Identical shared-Arc AI context inputs |
| Friendly-AI env (`think`, `think_unexpected_event`, `alert_soldier`, ...) | `robin_engine/src/ai_friendly.rs` | ~20 (mostly tests) | sim/ctx/tick/grid/doors analogue of the enemy `ThinkEnv` |
| Think/stimulus dispatch inputs | `robin_engine/src/engine/ai/cross_npc_actions.rs`, `engine/script.rs` | ~5 | stimulus/ctx/tick_data/policy travel together; a `ThinkEnv`-like bundle |
| Detection per-NPC / per-type pass inputs | `robin_engine/src/engine/ai/detection.rs` | 2 | Hoisted once per tick; needs the caller loop restructured first |
| `GroupMoveOverrides` | `robin_engine/src/engine/movement/formation.rs` | ~6 | Recorded route overrides repeated across the group-move family |
| `SpecialWalkOrder` | `robin_engine/src/engine/{door_pass,special_motion}.rs` | ~3 | Same order fields in `install_initial_walk` / `install_special_walk_order` |
| `LevelInitRequest` | `robin_engine/src/engine/{level_loading,script}.rs` | 2 | `initialize_from_mission` / `_campaign` share assets/staging/progress |
| Command payload structs (sword strike, DropAle / `DropAleRoute`, recorded quick-action route) | `robin_engine/src/engine/commands/{combat,object_use,interaction_route}.rs` | ~6 | Take the command variant struct instead of destructured fields |
| Small snapshots: `PostDamageInput`, `TakenObject`, `CorpseProbe` | `engine/melee/damage.rs`, `engine/archery.rs`, `engine/corpse_intersection.rs` | 1-3 each | Caller-hoisted snapshot fields |
| `NewAbuseReport` + `AbuseReportLimits` | `robin_highscores/src/db.rs` | ~5 | Report fields plus three rate limits |
| `DumpFrameInputs` | `robin_parity/src/original_parity_replay/reporting.rs` | 2 | Disjoint original/rust trace slices for one dump |
| Render inputs (`render_frame_with_hud`, `begin_wide_map_rgba`, `render_hud_text`) | `robin_rs/src/game_session/render.rs`, `robin_rs/src/hud_text.rs` | ~6 | engine/display/host/assets/dev/ctx repeated across the family |
| wgpu pass target (`GpuUpscale::render_multipass`, `ShaderPresetRenderer::render`) | `robin_rs/src/gpu_upscale.rs`, `robin_rs/src/shader_preset.rs` | ~4 | encoder/source/target_view/size/dst_rect/frame_count/preset |
| Headless mission restart loop (`run_mission_headless*`, `HeadlessMissionBuilder::build`) | `robin_rs/src/game_session/{mod,bootstrap}.rs` | ~4 | campaign/args/seed/sim_config per attempt; needs `MissionOutcome` ownership reworked |
| Save writers vs private `SaveCapture` | `robin_rs/src/savegame/writes.rs` | ~20 | Writers take `&mut Host` where `SaveCapture` takes `&Host` |
| `render_darken_inside` + `_gpu_spans` | `robin_rs/src/shadow_polygon.rs` | 2 | All ten params shared |

Families under the two-function threshold (tactical move orders, drunken
deviation) stay flat unless they grow.
