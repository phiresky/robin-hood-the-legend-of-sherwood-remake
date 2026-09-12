//! Game session: mission selection loop and the per-mission game loop.

mod bootstrap;
#[cfg(all(feature = "projection-export", not(target_arch = "wasm32")))]
pub(crate) use bootstrap::export_official_mission_headless;
mod debriefing;
mod dispatch;
mod event_hud;
mod flow;
mod frame_perf;
mod frame_prepare;
mod frame_simulate;
mod headless;
mod input_handlers;
mod interactive;
pub(crate) mod leaderboard_runtime;
mod live_gameplay;
mod modal_state;
mod mouse_input;
mod multiplayer;
mod render;
pub(crate) mod replay_init;
mod replay_launch;
use replay_launch::choose_pending_replay;
#[cfg(test)]
use replay_launch::prepare_replay_mission;
pub(crate) use replay_launch::{PreparedReplayLaunch, prepare_replay_launch};
mod retirement;
mod runtime;
mod session_policy;
mod setup;
pub(crate) use setup::PhaseTimer;
mod sherwood_flow;
pub(crate) use setup::initial_sim_config;
pub use setup::{load_fixed_vip_name_map, load_peasant_name_pool};
mod terminal_debriefing;
mod tick;
mod ui_task_state;

/// Initialize the host-side mission sound caches and deterministic duration
/// tables for developer tools that construct an [`Engine`] directly.
///
/// Normal game sessions perform this during their loading pipeline. Headless
/// parity tools still need the same metadata because NPC speech completion is
/// simulation state even when no audio backend is present.
pub fn setup_mission_audio_for_tool(
    host: &mut crate::Host,
    engine: &robin_engine::engine::Engine,
    assets: &mut robin_engine::engine::LevelAssets,
    profiles: &robin_engine::profiles::ProfileManager,
    sound_dir: &str,
) {
    let mission_idx = engine
        .campaign()
        .current_mission_idx
        .expect("mission-audio setup requires a current campaign mission");
    let location = engine.campaign().missions[mission_idx]
        .profile(profiles)
        .location;
    setup::setup_mission_audio(host, None, engine, assets, profiles, location, sound_dir)
        .expect("tool mission audio preparation failed");
}

use bootstrap::{
    HeadlessBuildOutcome, HeadlessMissionBuilder, InteractiveBuildOutcome,
    InteractiveMissionBuilder, MultiplayerSetupFailurePolicy,
};
use debriefing::{
    SettledDebriefingOutcome, final_debriefing_outcome_from_replay, final_debriefing_result,
};
use dispatch::apply_local_viewport_scroll;
pub(crate) use dispatch::{dispatch_local_command, dispatch_local_commands};
use frame_simulate::{FrameSimulationFlags, FrameSimulationOutcome, InteractiveFrameSimulation};
use input_handlers::{handle_console_overlay_events, handle_gamepad_events, handle_hold_to_rewind};
use interactive::{
    CameraPresentationPose, InteractiveFrontend, InteractiveMission, RenderViewState,
};
pub(crate) use modal_state::ModalContext;
use modal_state::{
    ActiveModal, ActiveModalOutcome, drain_pending_console_display, drain_pending_debriefings,
    drain_pending_dialogues, drain_pending_popup_scroll, drain_pending_sherwood_stat,
    pop_matching_dismissal, start_active_debriefing_batch, start_active_dialogue_batch,
    start_active_popup_scroll_batch, start_active_sherwood_report, tick_active_modal,
};
#[cfg(test)]
use mouse_input::allied_portrait_center;
use mouse_input::{
    dispatch_corner_button_left_click, dispatch_corner_button_right_click, handle_mouse_input,
    handle_pause_menu_events, handle_sherwood_campaign_map_overlay, handle_sherwood_hud_buttons,
    request_sherwood_trading_panel, sherwood_trading_access,
};
use multiplayer::{drain_mission_network, setup_multiplayer_session};
pub use render::RenderContext;
use render::{
    capture_screenshot_to_path, drain_presented_ui_screenshots, drain_print_screen_request,
    drain_screenshot_requests, drain_screenshots, drain_wide_print_screen,
    print_screen_request_from_modifiers, render_frame, update_mouse_and_cursor,
};
use robin_engine::coordinates as engine_coordinates;
use robin_engine::element as engine_element;
use robin_engine::engine as engine_api;
use robin_engine::engine::{Engine, ScrollDirection};
use robin_engine::messenger as engine_messenger;
use robin_engine::player_command as engine_player_command;
use robin_engine::position_interface as engine_position_interface;
use robin_engine::profiles as engine_profiles;
use robin_engine::sight_obstacle as engine_sight_obstacle;
use runtime::{
    FrameCommitPolicy, FrameOutcome, FramePacing, MissionAudioPhase, MissionControl, MissionFrame,
    MissionIngress, MissionInputPhase, MissionMutation, MissionPreTickPhase,
    MissionPresentationPhase, MissionRuntime, MissionWorld,
};
use tick::{
    dismiss_pending_modals, drain_steps, modal_state_pending, post_render_engine_cleanup,
    pre_render_engine_setup, sync_render_camera,
};

use crate::app_effect::{AppEffect, SoundMode};
use crate::corner_hud::{CornerButton, CornerButtonEnable, CornerHudLayout};
use crate::game::GameCallbacks;
use crate::gfx_types::GameEvent;
use crate::host::ApplicationContext;
use crate::host::Host;
use crate::host::PrintScreenRequest;
use crate::ingame_menu::resources::{MT_MSG_LEAVE_MISSION_NOW, MT_MSG_REALLY_LOAD_QUICKSAVE};
use crate::ingame_menu::{
    DebriefingOutcome, IngameMenuResources, MissionStatePopupState, PauseMenu, SaveLoadMode,
    SaveLoadOutcome,
};
use crate::input_translator::GameKey;
use crate::input_translator::{GameAction, TranslationFlags};
use crate::lua_session::LuaSession;
use crate::main_entry::{
    RustCallbacks, SaveBannerKind, SaveLoadRequest, current_mission_id, execute_app_effects,
    perform_pending_save_load, validated_save_reload_target,
};
use crate::multiplayer::matchmaking::current_epoch_ms;
use crate::renderer::Renderer;
use crate::save_file::special_slots;
use crate::stature_hud::{StatureButton, StatureEnable, StatureHudLayout};
use crate::window::GameWindow;
use crate::zoom_hud::{ZoomButton, ZoomButtonEnable};
use robin_engine::campaign::Campaign;
use robin_engine::game_operation::GameCode;
use robin_engine::player_command::PlayerCommand;
use robin_engine::profiles::MissionLocation;
use std::sync::Arc;

/// Outcome of a game session (series of missions).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionResult {
    /// Player chose to return to the main menu.
    QuitToMenu,
    /// Save admission recovery propagated an explicit application close.
    ExitRequested,
}

/// Consuming result of one mission. The campaign is returned on every
/// controlled exit, including setup and runtime errors.
pub(crate) struct MissionOutcome {
    pub(crate) campaign: Campaign,
    pub(crate) rng_seed: u64,
    pub(crate) sim_config: engine_api::SimConfig,
    pub(crate) result: Result<GameCode, String>,
    pub(crate) transition: Option<crate::main_entry::PendingLevelLoad>,
}

impl MissionOutcome {
    pub(crate) fn with_transition(
        mut self,
        transition: Option<crate::main_entry::PendingLevelLoad>,
    ) -> Self {
        if self.result.is_err() {
            // A later exit/screenshot error supersedes the transition. Dropping
            // its payload here cannot contaminate the next mission's callbacks.
            self.transition = None;
        } else {
            assert!(
                transition.is_none() || matches!(self.result, Ok(GameCode::LevelLoad)),
                "a cross-mission payload must accompany its LevelLoad exit"
            );
            self.transition = transition;
        }
        self
    }

    pub(crate) fn new(
        campaign: Campaign,
        rng_seed: u64,
        sim_config: engine_api::SimConfig,
        result: Result<GameCode, String>,
    ) -> Self {
        Self {
            campaign,
            rng_seed,
            sim_config,
            result,
            transition: None,
        }
    }
}

/// Consuming result of the outer mission-selection loop.
pub(crate) struct SessionOutcome {
    pub(crate) campaign: Campaign,
    pub(crate) result: Result<SessionResult, String>,
}

/// Control-flow signal returned by `run_mission` helpers that were
/// extracted from inside the outer `loop { ... }` body but retain
/// their control flow: continue the outer loop, return, or fall through.
#[derive(Debug)]
pub(super) enum HandlerAction {
    /// Caller should `continue;` the outer loop (skip remaining
    /// per-frame work and start the next iteration).
    Continue,
    /// Caller should proceed through the rest of the frame normally.
    Proceed,
    /// Caller should `return Ok(code)` from `run_mission`.
    Exit(GameCode),
}

pub(crate) async fn run_mission_headless(
    callbacks: &mut RustCallbacks,
    campaign: Campaign,
    profiles: &engine_profiles::ProfileManager,
    mission_idx: usize,
    location: MissionLocation,
    args: &crate::main_entry::MissionLaunch,
    rng_seed: u64,
    sim_config: engine_api::SimConfig,
) -> MissionOutcome {
    retirement::run(callbacks, async move |callbacks| {
        run_mission_headless_body(
            callbacks,
            campaign,
            profiles,
            mission_idx,
            location,
            args,
            rng_seed,
            sim_config,
        )
        .await
    })
    .await
}

async fn run_mission_headless_body(
    callbacks: &mut RustCallbacks,
    mut campaign: Campaign,
    profiles: &engine_profiles::ProfileManager,
    mission_idx: usize,
    location: MissionLocation,
    args: &crate::main_entry::MissionLaunch,
    mut rng_seed: u64,
    mut sim_config: engine_api::SimConfig,
) -> MissionOutcome {
    // Direct headless restart must carry launch policy without mutating the
    // caller's original arguments.
    let mut session_args = args.clone();
    let args = &mut session_args;
    if let Some(error) = unprepared_replay_launch_error(args) {
        return MissionOutcome::new(campaign, rng_seed, sim_config, Err(error));
    }
    let mission_name = campaign.missions[mission_idx]
        .profile(profiles)
        .mission_filename
        .clone();
    let has_decoded_saved_world = pending_decoded_saved_world(callbacks);
    let archive_restored = args
        .resolved_mission_assets
        .as_ref()
        .is_some_and(|resolved| resolved.is_archive());
    if !archive_restored
        && let Err(error) = ensure_shipping_mission(
            args,
            &mission_name,
            &campaign,
            profiles,
            has_decoded_saved_world,
            |_| {},
        )
        .await
    {
        return MissionOutcome::new(campaign, rng_seed, sim_config, Err(error));
    }
    let replay_restart = args
        .replay_data
        .as_ref()
        .map(|_| (campaign.clone(), rng_seed, sim_config));
    loop {
        campaign = establish_mission_restart_boundary(campaign, rng_seed, sim_config);
        let outcome = match HeadlessMissionBuilder::build(
            callbacks,
            campaign,
            profiles,
            mission_idx,
            location,
            args,
            rng_seed,
            sim_config,
        )
        .await
        {
            HeadlessBuildOutcome::Ready(mut mission) => {
                let outcome = mission.run(args).await;
                mission.finish(outcome)
            }
            HeadlessBuildOutcome::Finished(outcome) => outcome,
        };
        if !matches!(&outcome.result, Ok(GameCode::LevelRestart)) {
            return outcome;
        }
        args.mission_restart = true;
        let outcome_sim_config = outcome.sim_config;
        campaign = outcome.campaign;
        match prepare_direct_restart(
            &mut campaign,
            args,
            replay_restart.as_ref(),
            outcome_sim_config,
        ) {
            Ok((seed, config)) => {
                rng_seed = seed;
                sim_config = config;
            }
            Err(error) => {
                return MissionOutcome::new(campaign, rng_seed, sim_config, Err(error));
            }
        }
    }
}

/// Run the outer mission loop.
///
/// `initial_load` lets the caller pre-seed a load request — used by the
/// main menu's "Load Game" entry to kick straight into a saved mission
/// (see `main_menu::save_load`).
pub(crate) async fn run_session(
    window: &mut GameWindow,
    campaign: Campaign,
    profiles: &mut engine_profiles::ProfileManager,
    application_context: &ApplicationContext,
    args: &crate::main_entry::MissionLaunch,
    initial_load: Option<(crate::savegame::SlotName, u32)>,
) -> SessionOutcome {
    let session_args = args.clone();
    let mut callbacks =
        match RustCallbacks::new_for_window(application_context.clone(), window).await {
            Ok(Some(callbacks)) => callbacks,
            Ok(None) => {
                return SessionOutcome {
                    campaign,
                    result: Ok(if window.close_requested {
                        SessionResult::ExitRequested
                    } else {
                        SessionResult::QuitToMenu
                    }),
                };
            }
            Err(error) => {
                return SessionOutcome {
                    campaign,
                    result: Err(error),
                };
            }
        };
    retirement::run(&mut callbacks, async move |callbacks| {
        run_session_body(
            callbacks,
            window,
            campaign,
            profiles,
            application_context,
            args,
            initial_load,
            session_args,
        )
        .await
    })
    .await
}

async fn run_session_body(
    callbacks: &mut RustCallbacks,
    window: &mut GameWindow,
    mut campaign: Campaign,
    profiles: &mut engine_profiles::ProfileManager,
    application_context: &ApplicationContext,
    args: &crate::main_entry::MissionLaunch,
    initial_load: Option<(crate::savegame::SlotName, u32)>,
    mut session_args: crate::main_entry::MissionLaunch,
) -> SessionOutcome {
    if let Some((name, mission_id)) = initial_load {
        let Some(slot) = callbacks.save_manager.find_by_filename(name.as_str()) else {
            return SessionOutcome {
                campaign,
                result: Err(format!("selected save {} no longer exists", name.as_str())),
            };
        };
        let slot = match callbacks.save_manager.slot_handle(slot) {
            Ok(slot) => slot,
            Err(error) => {
                return SessionOutcome {
                    campaign,
                    result: Err(format!("selected save is unavailable: {error:#}")),
                };
            }
        };
        callbacks.queue_operation(SaveLoadRequest::Load {
            slot: Some(slot),
            mission_id,
        });
    }
    if args.replay_data.is_some() || args.replay.is_some() {
        return SessionOutcome {
            campaign,
            result: Err("replay missions must bypass campaign selection and launch directly from their header".to_string()),
        };
    }
    let mut authoritative_rng_seed = 0;
    let mut authoritative_sim_config = setup::initial_sim_config(args);
    let mut preselected_mission = None;
    if let Some(SaveLoadRequest::Load { slot, mission_id }) = callbacks.take_initial_request() {
        let load = match crate::main_entry::PreparedLoad::preflight(&callbacks.save_manager, slot) {
            Ok(Some(result)) => result,
            Ok(None) => {
                return SessionOutcome {
                    campaign,
                    result: Err("requested save slot has no loadable payload".to_string()),
                };
            }
            Err(error) => {
                return SessionOutcome {
                    campaign,
                    result: Err(format!("save preflight failed: {error:#}")),
                };
            }
        };
        let save = load.save();
        if mission_id != save.header.mission_id {
            return SessionOutcome {
                campaign,
                result: Err(format!(
                    "save preflight failed: selected mission id {mission_id} differs from decoded header {}",
                    save.header.mission_id
                )),
            };
        }
        let (target_idx, _location, resolved) =
            match prepare_cold_save_mission(application_context, profiles, save).await {
                Ok(prepared) => prepared,
                Err(error) => {
                    return SessionOutcome {
                        campaign,
                        result: Err(format!("save preflight failed: {error}")),
                    };
                }
            };
        clear_ambient_custom_launch(&mut session_args);
        session_args.resolved_mission_assets = Some(resolved);
        (authoritative_rng_seed, authoritative_sim_config) = save.engine.mission_start_simulation();
        campaign = save.engine.campaign().clone();
        preselected_mission = Some(target_idx);
        callbacks.queue_operation(SaveLoadRequest::ApplyLoad(load));
    }
    let mut replay_restart: Option<crate::replay_service::PendingReplay> = None;
    loop {
        let pending_replay = choose_pending_replay(
            args.global_options.replay_launches().take_pending(),
            &mut replay_restart,
        );
        let mut mission_args_storage = None;
        let mut replay_for_restart = None;
        let (mission_idx, location, restart_rng_seed, restart_sim_config) = if let Some(pending) =
            pending_replay
        {
            let paused = pending.paused;
            let replay_copy = pending.data.clone();
            // The persisted replay descriptor is the next mission's sole
            // asset authority. Release a live/save overlay before resolving
            // and mounting it so two archives can never compete in SbFile.
            session_args.resolved_mission_assets = None;
            clear_ambient_custom_launch(&mut session_args);
            let prepared = match prepare_replay_launch(
                application_context,
                profiles,
                &session_args,
                pending.data,
                paused,
            )
            .await
            {
                Ok(prepared) => prepared,
                Err(error) => {
                    return SessionOutcome {
                        campaign,
                        result: Err(error),
                    };
                }
            };
            campaign = prepared.campaign;
            authoritative_rng_seed = prepared.rng_seed;
            authoritative_sim_config = prepared.sim_config;
            mission_args_storage = Some(prepared.launch);
            replay_for_restart = Some(crate::replay_service::PendingReplay {
                data: replay_copy,
                paused,
            });
            (
                prepared.mission_idx,
                prepared.location,
                authoritative_rng_seed,
                authoritative_sim_config,
            )
        } else if let Some(mission_idx) = preselected_mission.take() {
            let location = campaign.missions[mission_idx].profile(profiles).location;
            let (restart_rng_seed, restart_sim_config) = campaign.restart_simulation_checkpoint();
            (mission_idx, location, restart_rng_seed, restart_sim_config)
        } else {
            let restart_rng_seed = authoritative_rng_seed;
            let restart_sim_config = authoritative_sim_config;
            campaign.snapshot_with_simulation(restart_rng_seed, restart_sim_config);
            // Mission selection runs on a temporary bare Engine owner,
            // then hands the complete next RNG/config state to the loaded
            // mission Engine.
            let selected = Engine::select_next_mission(
                campaign,
                profiles,
                authoritative_rng_seed,
                authoritative_sim_config,
            );
            campaign = selected.0;
            authoritative_rng_seed = selected.2;
            authoritative_sim_config = selected.3;
            let mission_idx = selected.1;
            let location = campaign.missions[mission_idx].profile(profiles).location;
            (mission_idx, location, restart_rng_seed, restart_sim_config)
        };
        let mission_args = mission_args_storage.as_ref().unwrap_or(&session_args);

        // Sherwood is a real loaded mission (level geometry, PCs,
        // NPCs, production sectors, script). The campaign map is an
        // overlay toggled via the DisplayCampaignMap widget. Fall
        // through to `run_mission` — Sherwood-specific behavior
        // (campaign-map overlay, Start/Quit-mission widgets,
        // campaign-state serialization on mission confirm) is wired inside
        // the per-frame loop.

        // Run the actual mission
        tracing::info!("Starting mission idx={} at {:?}", mission_idx, location);
        let mission_outcome = run_mission_with_seed(
            window,
            callbacks,
            campaign,
            profiles,
            mission_idx,
            location,
            mission_args,
            authoritative_rng_seed,
            authoritative_sim_config,
            MultiplayerSetupFailurePolicy::ReturnToMenu,
        )
        .await;
        // This evidence belongs to the just-consumed reconstruction, not the
        // next campaign mission. Only LevelRestart below installs it again.
        session_args.mission_restart = false;
        campaign = mission_outcome.campaign;
        authoritative_rng_seed = mission_outcome.rng_seed;
        authoritative_sim_config = mission_outcome.sim_config;
        let game_result = match mission_outcome.result {
            Ok(result) => result,
            Err(error) => {
                return SessionOutcome {
                    campaign,
                    result: Err(error),
                };
            }
        };

        match game_result {
            GameCode::Quit => {
                return SessionOutcome {
                    campaign,
                    result: Ok(SessionResult::QuitToMenu),
                };
            }
            GameCode::LevelSucceeded | GameCode::LevelInterrupted if campaign.get_ares() >= 9 => {
                if campaign.get_ares() == 9 {
                    // Campaign just completed — play the outro cinematic
                    // and bump ARES to 10.
                    tracing::info!("Campaign complete — playing outro cinematic");
                    if let Err(e) = crate::video_player::play_video(
                        application_context,
                        window,
                        "Data/Cinematics/Outro.ogg",
                    )
                    .await
                    {
                        tracing::warn!("Outro video error: {e}");
                    }
                    campaign.set_ares(10);
                }
                tracing::info!("Returning to main menu (ARES={})", campaign.get_ares());
                return SessionOutcome {
                    campaign,
                    result: Ok(SessionResult::QuitToMenu),
                };
            }
            GameCode::LevelSucceeded | GameCode::LevelInterrupted => {
                // Continue to next mission selection
            }
            GameCode::LevelFailed => {
                // Back to Sherwood for next mission
            }
            GameCode::LevelRestart => {
                // Re-run the same mission (player chose Restart from pause menu).
                // Roll campaign state back from the in-memory snapshot
                // captured above so accumulated mid-mission changes
                // (collected relics, ransom spends, kills, …) don't leak
                // into the retry.
                if replay_for_restart.is_none() {
                    if !campaign.restore_snapshot() {
                        panic!("LevelRestart requires the pre-selection campaign snapshot");
                    }
                    if campaign.pre_mission_was_preselected {
                        preselected_mission = Some(mission_idx);
                    }
                }
                authoritative_rng_seed = restart_rng_seed;
                authoritative_sim_config = simulation_config_for_level_restart(
                    restart_sim_config,
                    authoritative_sim_config,
                    replay_for_restart.is_some(),
                );
                replay_restart = replay_for_restart;
                session_args.mp_continue_session = session_args.server;
                session_args.mission_restart = true;
                tracing::info!("Restarting mission idx={}", mission_idx);
                continue;
            }
            GameCode::LevelLoad => {
                // Cross-mission load: `perform_pending_save_load` left the
                // slot + target mission in the consumed mission outcome and forced
                // the Game state machine into LevelLoad so `run_mission`
                // exited. Switch the campaign to the target mission and
                // re-queue the Load on the fresh engine.
                let Some(req) = mission_outcome.transition else {
                    tracing::warn!("LevelLoad exit without a pending load — returning to map");
                    continue;
                };
                if let Err(error) = req.validate_slot(&callbacks.save_manager) {
                    return SessionOutcome {
                        campaign,
                        result: Err(format!(
                            "cross-mission save selection became invalid: {error:#}"
                        )),
                    };
                }
                // The old session has been consumed, but `session_args` still
                // owns its archive/cache lifetime. Drop that owner before
                // mounting the next exact descriptor so two same-path custom
                // missions can never overlap.
                session_args.resolved_mission_assets = None;
                clear_ambient_custom_launch(&mut session_args);
                let (idx, _location, resolved) = match prepare_cold_save_mission(
                    application_context,
                    profiles,
                    req.save(),
                )
                .await
                {
                    Ok(prepared) => prepared,
                    Err(error) => {
                        return SessionOutcome {
                            campaign,
                            result: Err(format!("cross-mission save became invalid: {error}")),
                        };
                    }
                };
                session_args.resolved_mission_assets = Some(resolved);
                tracing::info!(
                    "Cross-mission load: switching to mission id={} (idx={}) and applying slot {:?}",
                    req.mission_id(),
                    idx,
                    req,
                );
                let save = req.save();
                (authoritative_rng_seed, authoritative_sim_config) =
                    save.engine.mission_start_simulation();
                campaign = save.engine.campaign().clone();
                preselected_mission = Some(idx);
                // Queue the Load again so the first frame of the
                // new mission applies the save to its fresh engine.
                callbacks.queue_operation(SaveLoadRequest::ApplyLoad(req.into_load()));
                session_args.mp_continue_session = session_args.server;
                continue;
            }
            _ => {}
        }
        // Only LevelRestart re-enters the same exact mission. Every other
        // completed mission releases its process-local overlay/cache owner
        // before campaign selection can inspect another profile.
        session_args.resolved_mission_assets = None;
        session_args.mp_continue_session = session_args.server;
    }
}

pub(crate) async fn run_mission(
    window: &mut GameWindow,
    callbacks: &mut RustCallbacks,
    campaign: Campaign,
    profiles: &mut engine_profiles::ProfileManager,
    mission_idx: usize,
    location: MissionLocation,
    args: crate::main_entry::MissionLaunch,
    rng_seed: u64,
    sim_config: engine_api::SimConfig,
) -> MissionOutcome {
    retirement::run(callbacks, async move |callbacks| {
        run_mission_body(
            window,
            callbacks,
            campaign,
            profiles,
            mission_idx,
            location,
            args,
            rng_seed,
            sim_config,
        )
        .await
    })
    .await
}

async fn run_mission_body(
    window: &mut GameWindow,
    callbacks: &mut RustCallbacks,
    mut campaign: Campaign,
    profiles: &mut engine_profiles::ProfileManager,
    mut mission_idx: usize,
    mut location: MissionLocation,
    mut args: crate::main_entry::MissionLaunch,
    mut rng_seed: u64,
    mut sim_config: engine_api::SimConfig,
) -> MissionOutcome {
    if let Some(error) = unprepared_replay_launch_error(&args) {
        return MissionOutcome::new(campaign, rng_seed, sim_config, Err(error));
    }
    let mut replay_restart = args
        .replay_data
        .as_ref()
        .map(|_| (campaign.clone(), rng_seed, sim_config));
    let mut pending_replay = args.global_options.replay_launches().take_pending();
    loop {
        match prepare_pending_direct_replay(
            &mut pending_replay,
            &callbacks.application_context(),
            profiles,
            &mut args,
        )
        .await
        {
            Ok(Some(prepared)) => {
                (campaign, mission_idx, location, rng_seed, sim_config) = prepared;
                replay_restart = Some((campaign.clone(), rng_seed, sim_config));
            }
            Ok(None) => {}
            Err(error) => {
                return MissionOutcome::new(campaign, rng_seed, sim_config, Err(error));
            }
        }
        let outcome = run_mission_with_seed(
            window,
            callbacks,
            campaign,
            profiles,
            mission_idx,
            location,
            &args,
            rng_seed,
            sim_config,
            MultiplayerSetupFailurePolicy::Fatal,
        )
        .await;
        if !matches!(&outcome.result, Ok(GameCode::LevelRestart)) {
            return outcome;
        }
        args.mission_restart = true;
        let outcome_sim_config = outcome.sim_config;
        campaign = outcome.campaign;
        pending_replay = args.global_options.replay_launches().take_pending();
        if pending_replay.is_some() {
            // A newly admitted replay owns the next cold construction. Do not
            // restore the previous mission checkpoint or reuse its selection.
            continue;
        }
        match prepare_direct_restart(
            &mut campaign,
            &mut args,
            replay_restart.as_ref(),
            outcome_sim_config,
        ) {
            Ok((seed, config)) => {
                rng_seed = seed;
                sim_config = config;
            }
            Err(error) => {
                return MissionOutcome::new(campaign, rng_seed, sim_config, Err(error));
            }
        }
    }
}

async fn run_mission_with_seed(
    window: &mut GameWindow,
    callbacks: &mut RustCallbacks,
    campaign: Campaign,
    profiles: &engine_profiles::ProfileManager,
    mission_idx: usize,
    location: MissionLocation,
    args: &crate::main_entry::MissionLaunch,
    rng_seed: u64,
    sim_config: engine_api::SimConfig,
    multiplayer_setup_failure_policy: MultiplayerSetupFailurePolicy,
) -> MissionOutcome {
    let outcome = match InteractiveMissionBuilder::build(
        window,
        callbacks,
        campaign,
        profiles,
        mission_idx,
        location,
        args,
        rng_seed,
        sim_config,
        multiplayer_setup_failure_policy,
    )
    .await
    {
        InteractiveBuildOutcome::Ready(mut mission) => {
            let result = mission.run(window, callbacks, profiles, args).await;
            mission.finish(result)
        }
        InteractiveBuildOutcome::Finished(outcome) => outcome,
    };
    #[cfg(target_arch = "wasm32")]
    let outcome = {
        let mut outcome = outcome;
        if let Err(error) = crate::replay_archive::retire_browser_mission().await {
            outcome.result = Err(format!("retire browser replay: {error:#}"));
            outcome.transition = None;
        }
        outcome
    };
    outcome
}

#[cfg(test)]
mod required_state_tests {
    #[test]
    fn shared_direct_restart_policy_preserves_live_and_replay_checkpoints() {
        let initial = robin_engine::engine::SimConfig::default();
        let changed = robin_engine::engine::SimConfig {
            enable_unbinding: !initial.enable_unbinding,
            ..initial
        };
        for headless in [false, true] {
            let mut args = crate::main_entry::MissionLaunch::from(crate::main_entry::CliArgs {
                server: true,
                headless,
                ..Default::default()
            });
            let mut campaign = Campaign::default();
            assert!(
                super::prepare_direct_restart(&mut campaign, &mut args, None, changed).is_err()
            );
            assert!(!args.mp_continue_session);
            campaign.snapshot_preselected_with_simulation(11, initial);
            let (seed, config) =
                super::prepare_direct_restart(&mut campaign, &mut args, None, changed).unwrap();
            assert_eq!(seed, 11);
            assert_eq!(config.enable_unbinding, changed.enable_unbinding);
            assert!(args.mp_continue_session);

            args.mp_continue_session = false;
            let replay = (Campaign::default(), 23, initial);
            let (seed, config) =
                super::prepare_direct_restart(&mut campaign, &mut args, Some(&replay), changed)
                    .unwrap();
            assert_eq!(seed, 23);
            assert_eq!(config.enable_unbinding, initial.enable_unbinding);
            assert!(!args.mp_continue_session);
            assert_eq!(
                serde_json::to_value(&campaign).unwrap(),
                serde_json::to_value(&replay.0).unwrap()
            );
        }
    }

    #[test]
    fn direct_restart_adapter_restores_checkpoint_before_admitting_continuation() {
        let mut args = crate::main_entry::MissionLaunch {
            config: crate::main_entry::CliArgs {
                server: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut campaign = Campaign::default();
        assert!(!super::restore_direct_restart_boundary(
            &mut campaign,
            &mut args
        ));
        assert!(!args.mp_continue_session);
        campaign.snapshot_with_simulation(7, robin_engine::engine::SimConfig::default());
        assert!(!super::restore_direct_restart_boundary(
            &mut campaign,
            &mut args
        ));
        assert!(!args.mp_continue_session);
        campaign
            .snapshot_preselected_with_simulation(11, robin_engine::engine::SimConfig::default());
        assert!(super::restore_direct_restart_boundary(
            &mut campaign,
            &mut args
        ));
        assert!(args.mp_continue_session);
        assert_eq!(campaign.restart_simulation_checkpoint().0, 11);
    }

    #[test]
    fn direct_restart_continuation_matches_campaign_only_for_restored_live_hosts() {
        for server in [false, true] {
            for restored in [false, true] {
                for replay in [false, true] {
                    for headless in [false, true] {
                        let mut args = crate::main_entry::MissionLaunch {
                            config: crate::main_entry::CliArgs {
                                server,
                                headless,
                                replay: replay.then(|| "recorded.rhrec".into()),
                                ..Default::default()
                            },
                            ..Default::default()
                        };
                        super::carry_direct_restart_multiplayer_continuation(&mut args, restored);
                        assert_eq!(args.mp_continue_session, server && restored && !replay);
                    }
                }
            }
        }
        // Ineligible transitions do not revoke already-established policy.
        let mut args = crate::main_entry::MissionLaunch {
            config: crate::main_entry::CliArgs {
                server: true,
                ..Default::default()
            },
            mp_continue_session: true,
            ..Default::default()
        };
        super::carry_direct_restart_multiplayer_continuation(&mut args, false);
        assert!(args.mp_continue_session);
    }

    use super::{
        MissionOutcome, allied_portrait_center, choose_pending_replay,
        establish_mission_restart_boundary, prepare_pending_direct_replay, prepare_replay_launch,
        prepare_replay_mission, required_menu_resources, simulation_config_for_level_restart,
        validate_cold_save_spellforge_preference,
    };
    use crate::ingame_menu::IngameMenuResources;
    use robin_engine::campaign::{Campaign, CampaignValue};
    use robin_engine::game_operation::GameCode;
    use robin_engine::mission::Mission;
    use robin_engine::profiles::{MissionProfile, ProfileManager};
    use robin_engine::replay::{ReplayFile, ReplayHeader};
    use std::collections::BTreeMap;

    #[test]
    fn cold_spellforge_admission_requires_profile_services_only_for_spellforge() {
        let context = crate::host::ApplicationContext::default();
        assert!(
            validate_cold_save_spellforge_preference(&context, "OrdinaryMission", false).is_ok()
        );
        let expected = context.active_profile_snapshot().unwrap_err();
        assert_eq!(
            validate_cold_save_spellforge_preference(&context, "NestedMission", true).unwrap_err(),
            format!("read Spellforge gameplay preference: {expected}")
        );
    }

    #[test]
    fn disabled_cold_spellforge_save_fails_without_mutating_preference() {
        use crate::host::ApplicationContext;
        use crate::key_config_store::KeyConfigStore;
        use robin_engine::engine::GlobalOptions;
        use robin_engine::player_profile::{DifficultyLevel, PlayerProfileManager};

        let directory = tempfile::tempdir().unwrap();
        let save_directory = directory.path().to_string_lossy().into_owned();
        let mut players = PlayerProfileManager::new(save_directory.clone());
        let active = players.create_profile("Cold save".to_owned(), DifficultyLevel::Medium);
        players.set_active(active);
        players.profiles[active]
            .gameplay_config
            .enable_spellforge_missions = false;
        let context = ApplicationContext::complete(
            crate::player_profile_store::PlayerProfileStore::for_directory(&save_directory),
            GlobalOptions::default(),
            players,
            KeyConfigStore::new(save_directory),
            None,
        )
        .unwrap();

        let error =
            validate_cold_save_spellforge_preference(&context, "NestedMission", true).unwrap_err();
        assert!(error.contains("disabled in Gameplay settings"));
        assert!(
            !context
                .active_profile_snapshot()
                .unwrap()
                .gameplay_config
                .enable_spellforge_missions,
            "cold admission must never coerce the persisted gameplay toggle"
        );
        context
            .with_player_profiles_mut(|profiles| {
                profiles
                    .get_active_mut()
                    .unwrap()
                    .gameplay_config
                    .enable_spellforge_missions = true;
            })
            .unwrap();
        assert!(validate_cold_save_spellforge_preference(&context, "NestedMission", true).is_ok());
    }

    #[test]
    fn allied_portrait_center_uses_the_whole_group() {
        use robin_engine::coordinates::MapPoint;
        use robin_engine::element::{
            ActorData, ActorSoldier, ElementData, Entity, HumanData, NpcData, SoldierData,
        };
        use robin_engine::element_kinds::ElementKind;

        let mut assets = robin_engine::engine::LevelAssets::new();
        let mut engine = robin_engine::engine::Engine::new_for_test(
            800.0,
            600.0,
            Campaign::default(),
            &mut assets,
        )
        .expect("test engine");
        let mut add_member = |point: MapPoint| {
            let mut element = {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::ActorSoldier;
                initial_element.active = true;
                initial_element
            };
            element.set_position_map(point);
            engine.test_add_entity(Entity::Soldier(ActorSoldier {
                element,
                actor: ActorData::default(),
                human: HumanData::default(),
                npc: NpcData::default(),
                soldier: SoldierData::default(),
            }))
        };
        let left = add_member(MapPoint::new(100.0, 200.0));
        let right = add_member(MapPoint::new(300.0, 400.0));

        assert_eq!(
            allied_portrait_center(&engine, &[left, right]),
            Some(MapPoint::new(200.0, 300.0))
        );
    }

    fn replay_fixture(
        current_mission_idx: Option<usize>,
    ) -> (ProfileManager, robin_engine::replay::ReplayData) {
        let mut profiles = ProfileManager::new();
        profiles.missions.push(MissionProfile {
            id: 17,
            mission_filename: "MissionA".into(),
            proto_level_filename: "ProtoA".into(),
            location: robin_engine::profiles::MissionLocation::Nottingham,
            ..Default::default()
        });
        let mut campaign = Campaign::default();
        campaign.missions.push(Mission {
            profile_idx: Some(0),
            ..Default::default()
        });
        campaign.current_mission_idx = current_mission_idx;
        campaign.snapshot_with_simulation(0x1010, robin_engine::engine::SimConfig::default());
        campaign.current_mission_idx = current_mission_idx;
        let sim_config = robin_engine::engine::SimConfig {
            highlander2: true,
            ..Default::default()
        };
        let data = ReplayFile {
            header: ReplayHeader {
                mission_id: "MissionA".into(),
                mission_assets: robin_engine::mission_assets::MissionAssetDescriptor::built_in(
                    "MissionA", "ProtoA", "ProtoA",
                )
                .unwrap(),
                rng_seed: 0x2020,
                sim_config,
                spellforge_package: None,
                version: robin_engine::replay::REPLAY_SCHEMA_VERSION,
                total_frames: 0,
                rankability: robin_engine::replay_rankability::ReplayRankability::rankable(),
                campaign: bitcode::encode(&campaign),
            },
            frames: BTreeMap::new(),
            hashes: BTreeMap::new(),
            save_markers: BTreeMap::new(),
            load_backs: BTreeMap::new(),
        }
        .try_into()
        .expect("valid replay fixture");
        (profiles, data)
    }

    #[test]
    fn mission_exit_returns_the_exact_campaign_allocation() {
        let mut engine_campaign = Campaign::default();
        engine_campaign.values[CampaignValue::Custom20] = 0x25_25_25;
        let production_sectors = engine_campaign.production_sectors.as_ptr();

        let outcome = MissionOutcome::new(
            engine_campaign,
            17,
            robin_engine::engine::SimConfig::default(),
            Ok(GameCode::LevelSucceeded),
        );

        assert_eq!(outcome.campaign.values[CampaignValue::Custom20], 0x25_25_25);
        assert_eq!(
            outcome.campaign.production_sectors.as_ptr(),
            production_sectors
        );
    }

    #[test]
    fn direct_launch_restart_restores_the_exact_preselected_boundary() {
        let mut campaign = Campaign {
            current_mission_idx: Some(3),
            next_mission_idx: None,
            ..Default::default()
        };
        campaign.values[CampaignValue::Custom20] = 17;
        let config = robin_engine::engine::SimConfig {
            amount_of_speaking: 8,
            ..Default::default()
        };

        let mut launched = establish_mission_restart_boundary(campaign, 0x5151, config);
        launched.values[CampaignValue::Custom20] = 99;
        assert!(launched.restore_snapshot());

        assert!(launched.pre_mission_was_preselected);
        assert_eq!(launched.current_mission_idx, Some(3));
        assert_eq!(launched.next_mission_idx, None);
        assert_eq!(launched.values[CampaignValue::Custom20], 17);
        assert_eq!(launched.restart_simulation_checkpoint(), (0x5151, config));
    }

    fn commanded_level_restart_fixture() -> (robin_engine::engine::SimConfig, MissionOutcome) {
        let checkpoint = robin_engine::engine::SimConfig {
            amount_of_speaking: 3,
            highlander2: true,
            // This synthetic level deliberately has no mission program.
            script_enabled: false,
            ..Default::default()
        };
        let mut assets = robin_engine::engine::LevelAssets::new();
        let mut engine = robin_engine::engine::Engine::new_for_test_with_simulation(
            1024.0,
            768.0,
            Campaign::default(),
            &mut assets,
            0x5151,
            checkpoint,
        )
        .unwrap();
        engine
            .advance_frame(
                &assets,
                robin_engine::engine::SimulationFrameInput::new(vec![
                    robin_engine::player_command::PlayerCommand::SetAmountOfSpeaking { amount: 9 }
                        .into(),
                    robin_engine::player_command::PlayerCommand::SetUnbindingEnabled {
                        enabled: false,
                    }
                    .into(),
                    robin_engine::player_command::PlayerCommand::SetItemGameplayConfig {
                        config: robin_engine::gameplay_config::ItemGameplayConfig::classic(),
                    }
                    .into(),
                    robin_engine::player_command::PlayerCommand::SetNoiseDistractionFeedback {
                        enabled: false,
                    }
                    .into(),
                ])
                .with_hourglass(false),
            )
            .expect("restart-boundary command admission");
        let outcome = MissionOutcome::new(
            engine.campaign().clone(),
            engine.rng_seed(),
            engine.sim_config(),
            Ok(GameCode::LevelRestart),
        );
        (checkpoint, outcome)
    }

    #[test]
    fn session_restart_preserves_commanded_profile_options() {
        let (checkpoint, outcome) = commanded_level_restart_fixture();
        assert!(matches!(outcome.result, Ok(GameCode::LevelRestart)));

        let restarted = simulation_config_for_level_restart(checkpoint, outcome.sim_config, false);

        assert_eq!(restarted.amount_of_speaking, 9);
        assert!(!restarted.enable_unbinding);
        assert_eq!(
            restarted.item_gameplay,
            robin_engine::gameplay_config::ItemGameplayConfig::classic()
        );
        assert!(!restarted.noise_distraction_feedback);
        assert!(restarted.highlander2, "other construction config resets");
    }

    #[test]
    fn direct_restart_preserves_commanded_profile_options() {
        let (checkpoint, outcome) = commanded_level_restart_fixture();
        assert!(matches!(outcome.result, Ok(GameCode::LevelRestart)));

        let restarted = simulation_config_for_level_restart(checkpoint, outcome.sim_config, false);

        assert_eq!(restarted.amount_of_speaking, 9);
        assert!(!restarted.enable_unbinding);
        assert_eq!(
            restarted.item_gameplay,
            robin_engine::gameplay_config::ItemGameplayConfig::classic()
        );
        assert!(!restarted.noise_distraction_feedback);
        assert!(restarted.highlander2, "direct launch uses its checkpoint");
    }

    #[test]
    fn replay_restart_keeps_exact_frame_zero_config_for_command_replay() {
        let (checkpoint, outcome) = commanded_level_restart_fixture();
        assert!(matches!(outcome.result, Ok(GameCode::LevelRestart)));

        let restarted = simulation_config_for_level_restart(checkpoint, outcome.sim_config, true);

        assert_eq!(restarted, checkpoint);
        assert_eq!(restarted.amount_of_speaking, 3);
        assert!(restarted.enable_unbinding);
        assert_eq!(restarted.item_gameplay, checkpoint.item_gameplay);
        assert_eq!(
            restarted.noise_distraction_feedback,
            checkpoint.noise_distraction_feedback
        );
    }

    #[test]
    fn mission_outcome_returns_campaign_for_success_and_error() {
        let exit_outcomes = [
            ("normal mission exit", Ok(GameCode::LevelSucceeded)),
            ("mission-start map export", Ok(GameCode::Quit)),
            ("window close", Ok(GameCode::Quit)),
            ("modal emergency exit", Ok(GameCode::Quit)),
            ("cross-mission load", Ok(GameCode::LevelLoad)),
            ("pause-menu restart", Ok(GameCode::LevelRestart)),
            ("Sherwood mission launch", Ok(GameCode::LevelInterrupted)),
            ("campaign-map quit", Ok(GameCode::Quit)),
            ("headless mission exit", Ok(GameCode::LevelFailed)),
            ("headless replay completion", Ok(GameCode::Quit)),
            (
                "Spellforge startup error",
                Err("startup failed".to_string()),
            ),
            ("map export error", Err("capture failed".to_string())),
            ("mission frame error", Err("frame failed".to_string())),
        ];

        for (index, (path, outcome)) in exit_outcomes.into_iter().enumerate() {
            let mut engine_campaign = Campaign::default();
            let marker = index as i32 + 1;
            engine_campaign.values[CampaignValue::Custom20] = marker;
            let production_sectors = engine_campaign.production_sectors.as_ptr();

            let actual = MissionOutcome::new(
                engine_campaign,
                index as u64,
                robin_engine::engine::SimConfig::default(),
                outcome.clone(),
            );

            assert_eq!(actual.result, outcome, "{path}");
            assert_eq!(
                actual.campaign.values[CampaignValue::Custom20],
                marker,
                "{path}"
            );
            assert_eq!(
                actual.campaign.production_sectors.as_ptr(),
                production_sectors,
                "{path}"
            );
        }
    }

    #[test]
    fn pending_direct_replay_absent_preserves_ordinary_launch() {
        let mut profiles = ProfileManager::new();
        let mut args = crate::main_entry::MissionLaunch {
            config: crate::main_entry::CliArgs {
                start_paused: true,
                custom_mission: Some("ordinary-custom-mission".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let mut pending = None;
        assert!(
            pollster::block_on(prepare_pending_direct_replay(
                &mut pending,
                &crate::host::ApplicationContext::default(),
                &mut profiles,
                &mut args,
            ))
            .unwrap()
            .is_none()
        );
        assert!(args.start_paused);
        assert_eq!(
            args.custom_mission.as_deref(),
            Some(std::path::Path::new("ordinary-custom-mission"))
        );
        assert!(args.replay_data.is_none());
        assert!(profiles.missions.is_empty());
    }

    #[test]
    fn pending_direct_replay_replaces_selection_and_releases_previous_lease_once() {
        let context = crate::host::ApplicationContext::default();
        let (mut profiles, mut data) = replay_fixture(Some(0));
        let mut args = pollster::block_on(prepare_replay_launch(
            &context,
            &mut profiles,
            &crate::main_entry::MissionLaunch::default(),
            data.clone(),
            true,
        ))
        .unwrap()
        .launch;
        let old_lease = std::sync::Arc::downgrade(args.resolved_mission_assets.as_ref().unwrap());
        profiles.missions.push(MissionProfile {
            id: 18,
            mission_filename: "MissionB".into(),
            proto_level_filename: "ProtoB".into(),
            location: robin_engine::profiles::MissionLocation::Sherwood,
            ..Default::default()
        });
        let mut campaign: Campaign = bitcode::decode(&data.header().campaign).unwrap();
        campaign.missions.push(Mission {
            profile_idx: Some(1),
            ..Default::default()
        });
        campaign.current_mission_idx = Some(1);
        campaign.values[CampaignValue::Custom20] = 123;
        data.try_edit_header(|header| {
            header.mission_id = "MissionB".into();
            header.mission_assets = robin_engine::mission_assets::MissionAssetDescriptor::built_in(
                "MissionB", "ProtoB", "ProtoB",
            )
            .unwrap();
            header.campaign = bitcode::encode(&campaign);
            header.rng_seed = 0x3030;
        })
        .unwrap();
        let mut pending = Some(crate::replay_service::PendingReplay {
            data,
            paused: false,
        });
        let (campaign, index, location, seed, config) = pollster::block_on(
            prepare_pending_direct_replay(&mut pending, &context, &mut profiles, &mut args),
        )
        .unwrap()
        .unwrap();
        assert!(pending.is_none());
        assert!(old_lease.upgrade().is_none());
        assert_eq!(index, 1);
        assert_eq!(campaign.current_mission_idx, Some(1));
        assert_eq!(campaign.values[CampaignValue::Custom20], 123);
        assert_eq!(location, robin_engine::profiles::MissionLocation::Sherwood);
        assert_eq!(seed, 0x3030);
        assert!(config.highlander2);
        assert!(!args.start_paused);
        assert_eq!(
            args.replay_data.as_ref().unwrap().header().mission_id,
            "MissionB"
        );
        let lease = std::sync::Arc::clone(args.resolved_mission_assets.as_ref().unwrap());
        assert!(
            pollster::block_on(prepare_pending_direct_replay(
                &mut pending,
                &context,
                &mut profiles,
                &mut args,
            ))
            .unwrap()
            .is_none()
        );
        assert!(std::sync::Arc::ptr_eq(
            &lease,
            args.resolved_mission_assets.as_ref().unwrap()
        ));
    }

    #[test]
    fn pending_direct_replay_same_mission_restores_recorded_metadata() {
        let (mut profiles, data) = replay_fixture(Some(0));
        let mut args = crate::main_entry::MissionLaunch::default();
        let mut pending = Some(crate::replay_service::PendingReplay { data, paused: true });
        let (campaign, index, _, seed, config) = pollster::block_on(prepare_pending_direct_replay(
            &mut pending,
            &crate::host::ApplicationContext::default(),
            &mut profiles,
            &mut args,
        ))
        .unwrap()
        .unwrap();
        assert_eq!(campaign.current_mission_idx, Some(0));
        assert_eq!(index, 0);
        assert_eq!(seed, 0x2020);
        assert!(config.highlander2);
        assert!(args.start_paused);
        assert!(args.resolved_mission_assets.is_some());
        assert!(pending.is_none());
    }

    #[test]
    fn pending_direct_replay_rejection_is_consumed_without_fallback() {
        let (mut profiles, data) = replay_fixture(None);
        let mut args = crate::main_entry::MissionLaunch::default();
        let mut pending = Some(crate::replay_service::PendingReplay {
            data,
            paused: false,
        });
        let error = pollster::block_on(prepare_pending_direct_replay(
            &mut pending,
            &crate::host::ApplicationContext::default(),
            &mut profiles,
            &mut args,
        ))
        .unwrap_err();
        assert!(error.contains("pending direct-mission replay launch failed"));
        assert!(error.contains("has no current mission"));
        assert!(pending.is_none());
        assert!(args.resolved_mission_assets.is_none());
    }

    #[test]
    fn raw_replay_inputs_fail_at_the_real_decode_and_mission_admission_boundaries() {
        let directory = tempfile::tempdir().unwrap();
        for (name, contents, expected) in [
            ("missing", None, "open"),
            ("malformed", Some("not a replay header\n"), "bad header"),
            (
                "unsupported",
                Some(
                    "{\"mission_id\":\"MissionA\",\"rng_seed\":42,\"version\":999,\"total_frames\":0,\"campaign\":null}\n",
                ),
                "unsupported replay schema version 999",
            ),
        ] {
            let path = directory.path().join(format!("{name}.rhrec.jsonl"));
            if let Some(contents) = contents {
                std::fs::write(&path, contents).unwrap();
            }
            let spec = path.to_str().unwrap();
            let error = crate::replay_format::load_replay_spec(spec).unwrap_err();
            assert!(error.to_string().contains(expected), "{name}: {error}");
            let args = crate::main_entry::MissionLaunch {
                config: crate::main_entry::CliArgs {
                    replay: Some(spec.to_owned()),
                    ..Default::default()
                },
                ..Default::default()
            };
            assert!(
                super::unprepared_replay_launch_error(&args)
                    .unwrap()
                    .contains("before canonical decode")
            );
        }
    }

    #[test]
    fn replay_preparation_restores_all_frame_zero_metadata() {
        let (mut profiles, data) = replay_fixture(Some(0));
        let args = crate::main_entry::MissionLaunch {
            mission_restart: true,
            ..Default::default()
        };

        let super::PreparedReplayLaunch {
            campaign,
            mission_idx,
            location,
            launch: prepared_args,
            rng_seed: seed,
            sim_config: config,
        } = prepare_replay_mission(&mut profiles, &args, data, true).unwrap();

        assert_eq!(campaign.current_mission_idx, Some(0));
        assert_eq!(mission_idx, 0);
        assert_eq!(
            location,
            robin_engine::profiles::MissionLocation::Nottingham
        );
        assert_eq!(seed, 0x2020);
        assert!(config.highlander2);
        assert!(prepared_args.start_paused);
        assert!(
            !prepared_args.mission_restart,
            "an admitted replay supersedes live restart evidence"
        );
        assert!(prepared_args.replay.is_none());
        assert_eq!(
            prepared_args.replay_data.unwrap().header().sim_config,
            config
        );
    }

    #[test]
    fn replay_preparation_rejects_current_mission_mismatch() {
        let (mut profiles, data) = replay_fixture(None);
        let error = prepare_replay_mission(
            &mut profiles,
            &crate::main_entry::MissionLaunch::default(),
            data,
            false,
        )
        .unwrap_err();
        assert!(error.contains("has no current mission"));
    }

    #[test]
    fn replay_preparation_retains_map_identity_for_loaded_level_validation() {
        let (mut profiles, mut data) = replay_fixture(Some(0));
        data.try_edit_header(|header| header.mission_assets.map_filename = "DifferentMap".into())
            .unwrap();

        let super::PreparedReplayLaunch {
            launch: prepared_args,
            ..
        } = prepare_replay_mission(
            &mut profiles,
            &crate::main_entry::MissionLaunch::default(),
            data,
            false,
        )
        .expect("profile proto and loaded RHM map are distinct identities");
        let prepared_replay = prepared_args.replay_data.expect("prepared replay data");
        let descriptor = &prepared_replay.header().mission_assets;

        assert_eq!(descriptor.proto_level_filename, "ProtoA");
        assert_eq!(descriptor.map_filename, "DifferentMap");
    }

    #[test]
    fn replay_preparation_restores_forced_custom_mission_profile() {
        let (mut profiles, mut data) = replay_fixture(Some(0));
        let mut campaign: Campaign =
            bitcode::decode(&data.header().campaign).expect("decode replay campaign fixture");
        campaign.missions[0].profile_idx = Some(1);
        data.try_edit_header(|header| {
            header.mission_id = "CustomArena".into();
            header.mission_assets = robin_engine::mission_assets::MissionAssetDescriptor::archive(
                "CustomArena",
                "CustomProto",
                "CustomProto",
                robin_engine::mission_assets::ArchiveMissionAssets {
                    mission_archive: robin_engine::mission_assets::ArchiveIdentity {
                        sha256: [7; 32],
                        bytes: 1,
                    },
                    selected_rhm_entry: "nested/CustomArena.rhm".into(),
                    shared_archive: None,
                    installed: Some(robin_engine::mission_assets::InstalledArchiveLocator {
                        root: robin_engine::mission_assets::InstalledModsRoot::ConfiguredMods,
                        mission_relative_path: "custom/arena.zip".into(),
                        shared_relative_path: None,
                    }),
                    distributed_cache: None,
                },
            )
            .unwrap();
            header.campaign = bitcode::encode(&campaign);
        })
        .unwrap();

        let super::PreparedReplayLaunch { mission_idx, .. } = prepare_replay_mission(
            &mut profiles,
            &crate::main_entry::MissionLaunch::default(),
            data,
            false,
        )
        .expect("forced custom mission replay should restore its synthetic profile");

        assert_eq!(mission_idx, 0);
        assert_eq!(profiles.missions.len(), 2);
        assert_eq!(profiles.missions[1].mission_filename, "CustomArena");
        assert_eq!(profiles.missions[1].proto_level_filename, "CustomProto");
    }

    #[test]
    fn disabled_spellforge_replay_fails_before_asset_resolution_without_changing_preference() {
        use crate::key_config_store::KeyConfigStore;
        use robin_engine::mission_assets::{
            ArchiveIdentity, ArchiveMissionAssets, InstalledArchiveLocator, InstalledModsRoot,
            MissionAssetDescriptor,
        };
        use robin_engine::player_profile::{DifficultyLevel, PlayerProfileManager};
        use robin_engine::spellforge::{
            SPELLFORGE_CONTRACT_VERSION, SpellforgePackage, SpellforgeScriptMode,
        };

        let root = tempfile::tempdir().unwrap();
        let root_text = root.path().to_string_lossy().into_owned();
        let mut player_profiles = PlayerProfileManager::new(root_text.clone());
        let profile_idx =
            player_profiles.create_profile("Replay policy".into(), DifficultyLevel::Medium);
        player_profiles.set_active(profile_idx);
        player_profiles
            .get_active_mut()
            .unwrap()
            .gameplay_config
            .enable_spellforge_missions = false;
        let application_context = crate::host::ApplicationContext::complete(
            crate::player_profile_store::PlayerProfileStore::for_directory(&root_text),
            robin_engine::engine::GlobalOptions::default(),
            player_profiles,
            KeyConfigStore::new(root_text),
            None,
        )
        .unwrap();

        let (mut profiles, mut data) = replay_fixture(Some(0));
        let mission_assets = MissionAssetDescriptor::archive(
            "MissionA",
            "ProtoA",
            "ProtoA",
            ArchiveMissionAssets {
                mission_archive: ArchiveIdentity {
                    sha256: [9; 32],
                    bytes: 1,
                },
                selected_rhm_entry: "German/DATA/Levels/MissionA.rhm".into(),
                shared_archive: None,
                installed: Some(InstalledArchiveLocator {
                    root: InstalledModsRoot::ConfiguredMods,
                    mission_relative_path: "missing/nested-multilingual.zip".into(),
                    shared_relative_path: None,
                }),
                distributed_cache: None,
            },
        )
        .unwrap();
        let mut package = SpellforgePackage {
            contract_version: SPELLFORGE_CONTRACT_VERSION,
            vm_abi: robin_spellforge::spellforge_vm_abi().to_owned(),
            script_mode: SpellforgeScriptMode::Replace,
            entrypoint: "missiona.lua".into(),
            files: [(
                "missiona.lua".into(),
                b"function Initialize() return 0 end".to_vec(),
            )]
            .into_iter()
            .collect(),
            sha256: [0; 32],
        };
        package.sha256 = robin_spellforge::compute_package_sha256(&package);
        data.try_edit_header(|header| {
            header.mission_assets = mission_assets;
            header.spellforge_package = Some(package);
        })
        .unwrap();

        let error = pollster::block_on(prepare_replay_launch(
            &application_context,
            &mut profiles,
            &crate::main_entry::MissionLaunch::default(),
            data,
            false,
        ))
        .unwrap_err();

        assert!(error.contains("disabled in Gameplay settings"), "{error}");
        assert!(
            !application_context
                .active_profile_snapshot()
                .unwrap()
                .gameplay_config
                .enable_spellforge_missions,
            "replay startup must never coerce the saved Gameplay preference"
        );
    }

    #[test]
    fn current_replay_rejects_original_parity_capture_before_resolution() {
        let (mut profiles, data) = replay_fixture(Some(0));
        let args = crate::main_entry::MissionLaunch {
            mission_start_legacy_save: Some(vec![0; 4]),
            ..Default::default()
        };

        let error = pollster::block_on(prepare_replay_launch(
            &crate::host::ApplicationContext::default(),
            &mut profiles,
            &args,
            data,
            false,
        ))
        .unwrap_err();

        assert!(error.contains("Original parity"), "{error}");
    }

    #[test]
    fn newly_queued_replay_wins_over_restart_fallback() {
        let (_, queued_data) = replay_fixture(Some(0));
        let (_, mut restart_data) = replay_fixture(Some(0));
        restart_data
            .try_edit_header(|header| header.rng_seed = 0x3030)
            .unwrap();
        let mut restart = Some(crate::replay_service::PendingReplay {
            data: restart_data,
            paused: false,
        });

        let selected = choose_pending_replay(
            Some(crate::replay_service::PendingReplay {
                data: queued_data,
                paused: true,
            }),
            &mut restart,
        )
        .unwrap();

        assert_eq!(selected.data.header().rng_seed, 0x2020);
        assert!(selected.paused);
        assert!(restart.is_none(), "new replay must discard the old restart");
    }

    #[test]
    fn completed_new_replay_cannot_resurrect_the_old_restart() {
        for terminal_code in [GameCode::LevelSucceeded, GameCode::Quit] {
            let (_, queued_data) = replay_fixture(Some(0));
            let (_, mut restart_data) = replay_fixture(Some(0));
            restart_data
                .try_edit_header(|header| header.rng_seed = 0x3030)
                .unwrap();
            let mut restart = Some(crate::replay_service::PendingReplay {
                data: restart_data,
                paused: false,
            });

            let selected = choose_pending_replay(
                Some(crate::replay_service::PendingReplay {
                    data: queued_data,
                    paused: true,
                }),
                &mut restart,
            );
            assert!(selected.is_some());

            // The selected replay has now completed or exited. Entering the
            // selection loop again must not reveal the superseded replay.
            assert!(
                choose_pending_replay(None, &mut restart).is_none(),
                "terminal outcome {terminal_code:?} resurrected the old replay"
            );
        }
    }

    #[test]
    #[should_panic(expected = "test confirmation: in-game menu resources are missing")]
    fn confirmation_rejects_missing_menu_resources() {
        let resources: Option<IngameMenuResources> = None;
        required_menu_resources(&resources, "test confirmation");
    }
}
mod mission_launch;
use mission_launch::*;
pub(crate) use mission_launch::{
    establish_mission_restart_boundary, install_and_validate_saved_profile,
    install_cold_save_lua_session, install_pending_lua_session, pending_cold_save_lua_launch,
    required_menu_resources, selected_pc_profile_indices,
};
