//! Game session: mission selection loop and the per-mission game loop.

mod bootstrap;
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
mod live_gameplay;
mod modal_state;
mod mouse_input;
mod multiplayer;
mod render;
mod replay_init;
mod runtime;
mod setup;
pub(crate) use setup::initial_sim_config;
pub use setup::{load_fixed_vip_name_map, load_peasant_name_pool};
mod terminal_debriefing;
mod tick;

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
    setup::setup_mission_audio(host, None, engine, assets, profiles, location, sound_dir);
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
use interactive::{InteractiveFrontend, InteractiveMission, RenderViewState};
pub(crate) use modal_state::ModalContext;
use modal_state::{
    ActiveModal, ActiveModalOutcome, drain_pending_console_display, drain_pending_debriefings,
    drain_pending_dialogues, drain_pending_popup_scroll, drain_pending_sherwood_stat,
    pop_matching_dismissal, start_active_debriefing_batch, start_active_dialogue_batch,
    start_active_popup_scroll_batch, start_active_sherwood_report, tick_active_modal,
};
use mouse_input::{
    dispatch_corner_button_left_click, dispatch_corner_button_right_click, handle_mouse_input,
    handle_pause_menu_events, handle_sherwood_campaign_map_overlay, handle_sherwood_hud_buttons,
};
use multiplayer::{
    drain_mission_network, host_scheduled_frame_deadline_ms, setup_multiplayer_session,
};
pub use render::RenderContext;
use render::{
    capture_save_thumbnail, capture_screenshot_to_path, drain_print_screen_request,
    drain_screenshots, drain_wide_print_screen, print_screen_request_from_modifiers, render_frame,
    update_mouse_and_cursor,
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
    FrameCommitPolicy, FrameOutcome, FramePacing, MissionControl, MissionFrame, MissionRuntime,
    MissionWorld,
};
use tick::{
    dismiss_pending_modals, drain_steps, modal_state_pending, post_render_engine_cleanup,
    pre_render_engine_setup,
};

use crate::app_effect::{AppEffect, SoundMode};
use crate::corner_hud::{CornerButton, CornerButtonEnable, CornerHudLayout};
use crate::cursor::CursorRenderer;
use crate::game::GameCallbacks;
use crate::gfx_types::GameEvent;
use crate::host::ApplicationContext;
use crate::host::Host;
use crate::host::PrintScreenRequest;
use crate::ingame_menu::resources::{MT_MSG_LEAVE_MISSION_NOW, MT_MSG_REALLY_LOAD_QUICKSAVE};
use crate::ingame_menu::widget_bridge::default_modal_cursor;
use crate::ingame_menu::{
    DebriefingOutcome, IngameMenuResources, MissionStatePopupState, PauseMenu, SaveLoadMode,
    SaveLoadOutcome, show_yesno,
};
use crate::input_translator::GameKey;
use crate::input_translator::{GameAction, TranslationFlags};
use crate::lua_session::LuaSession;
use crate::main_entry::{
    RustCallbacks, SaveBannerKind, SaveLoadRequest, current_mission_id, execute_app_effects,
    perform_pending_save_load, validated_save_reload_target,
};
use crate::main_menu::custom_missions::CustomMissionLaunch;
use crate::multiplayer::matchmaking::current_epoch_ms;
use crate::renderer::Renderer;
use crate::save_file::special_slots;
use crate::stature_hud::{StatureButton, StatureEnable, StatureHudLayout};
use crate::ui_panel::PortraitHitArea;
use crate::window::GameWindow;
use crate::zoom_hud::{ZoomButton, ZoomButtonEnable};
use robin_assets::resource_manager::ResourceManager;
use robin_engine::campaign::Campaign;
use robin_engine::game_operation::GameCode;
use robin_engine::player_command::{PlayerCommand, PlayerInput};
use robin_engine::profiles::MissionLocation;

pub(crate) fn prepare_replay_mission(
    profiles: &engine_profiles::ProfileManager,
    args: &crate::main_entry::CliArgs,
    data: robin_engine::replay::ReplayData,
    paused: bool,
) -> Result<
    (
        Campaign,
        usize,
        MissionLocation,
        crate::main_entry::CliArgs,
        u64,
        engine_api::SimConfig,
    ),
    String,
> {
    crate::replay_format::validate_replay_data(&data)
        .map_err(|error| format!("invalid replay: {error}"))?;
    let campaign: Campaign = bitcode::deserialize(&data.header.campaign)
        .map_err(|error| format!("failed to restore replay campaign: {error}"))?;
    let mission_id = data.header.mission_id.clone();
    let mission_idx = campaign
        .missions
        .iter()
        .position(|mission| mission.profile(profiles).mission_filename == mission_id)
        .ok_or_else(|| format!("replay mission `{mission_id}` is absent from its campaign"))?;
    if campaign.current_mission_idx != Some(mission_idx) {
        return Err(format!(
            "replay campaign current mission {:?} does not match header mission `{mission_id}` at index {mission_idx}",
            campaign.current_mission_idx,
        ));
    }
    let location = campaign.missions[mission_idx].profile(profiles).location;
    let rng_seed = data.header.rng_seed;
    let sim_config = data.header.sim_config;
    let mut replay_args = args.clone();
    replay_args.replay_data = Some(data);
    replay_args.replay = None;
    replay_args.start_paused |= paused;
    Ok((
        campaign,
        mission_idx,
        location,
        replay_args,
        rng_seed,
        sim_config,
    ))
}

fn choose_pending_replay(
    newly_queued: Option<crate::http_server::PendingReplay>,
    restart_fallback: &mut Option<crate::http_server::PendingReplay>,
) -> Option<crate::http_server::PendingReplay> {
    if newly_queued.is_some() {
        // A newly queued replay supersedes the whole prior replay lifecycle,
        // including its restart copy. Do not leave the old recording armed
        // for a later loop iteration after the new replay exits.
        *restart_fallback = None;
        newly_queued
    } else {
        restart_fallback.take()
    }
}

fn center_on_reselected_portrait_pc(
    host: &mut Host,
    engine: &Engine,
    local_seat: engine_player_command::PlayerId,
    pc_id: engine_element::EntityId,
    append: bool,
    area: PortraitHitArea,
) -> bool {
    if append
        || !matches!(
            area,
            PortraitHitArea::TopScroll | PortraitHitArea::BottomScroll | PortraitHitArea::Visage
        )
        || !engine.seat_selection(local_seat).contains(&pc_id)
    {
        return false;
    }

    let Some(entity) = engine.get_entity(pc_id) else {
        tracing::warn!("Portrait reselect: selected PC {:?} is missing", pc_id);
        return false;
    };

    // Selecting an already-selected portrait is rewritten into a
    // `MSG_CENTER_ON` before the normal `MSG_SELECT_CHARACTER_WITH_ECHO`
    // flow continues.
    host.viewport
        .center_on_point(entity.position_iface().map_position());
    true
}

fn allied_portrait_center(
    engine: &Engine,
    members: &[engine_element::EntityId],
) -> Option<robin_engine::coordinates::MapPoint> {
    let mut count = 0_u32;
    let mut sum_x = 0.0_f32;
    let mut sum_y = 0.0_f32;
    for member in members {
        let Some(entity) = engine.get_entity(*member) else {
            tracing::warn!(?member, "Allied portrait center: group member is missing");
            continue;
        };
        let point = entity.position_iface().map_position();
        count += 1;
        sum_x += point.x;
        sum_y += point.y;
    }
    (count > 0).then(|| {
        let reciprocal = 1.0 / count as f32;
        robin_engine::coordinates::MapPoint::new(sum_x * reciprocal, sum_y * reciprocal)
    })
}

fn center_on_reselected_allied_portrait(
    host: &mut Host,
    engine: &Engine,
    local_seat: engine_player_command::PlayerId,
    members: &[engine_element::EntityId],
    append: bool,
    area: PortraitHitArea,
) -> bool {
    if append
        || !matches!(
            area,
            PortraitHitArea::TopScroll | PortraitHitArea::BottomScroll | PortraitHitArea::Visage
        )
        || engine.allied_selection(local_seat) != members
    {
        return false;
    }

    let Some(center) = allied_portrait_center(engine, members) else {
        tracing::warn!("Allied portrait reselect: group has no live members");
        return false;
    };
    host.viewport.center_on_point(center);
    true
}

/// Outcome of a game session (series of missions).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionResult {
    /// Player chose to return to the main menu.
    QuitToMenu,
}

/// Consuming result of one mission. The campaign is returned on every
/// controlled exit, including setup and runtime errors.
pub(crate) struct MissionOutcome {
    pub(crate) campaign: Campaign,
    pub(crate) rng_seed: u64,
    pub(crate) sim_config: engine_api::SimConfig,
    pub(crate) result: Result<GameCode, String>,
}

impl MissionOutcome {
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
        }
    }

    pub(crate) fn from_engine(
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
/// original control flow (outer-loop `continue`, outer function
/// `return`, or fall-through).
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

/// Construct the optional custom-mission Lua state before level loading.
/// A Spellforge-tagged launch treats construction as required; only Vanilla
/// custom missions may legitimately produce no session.
pub(super) fn install_pending_lua_session(
    host: &mut Host,
    args: &crate::main_entry::CliArgs,
) -> Result<(), crate::lua_session::SpellforgeSessionError> {
    let Some(pending) = args.pending_lua_mission.as_ref() else {
        return Ok(());
    };
    let launch = CustomMissionLaunch {
        slug: pending.slug.clone(),
        mod_title: pending.slug.clone(),
        version_zip: pending.version_zip.clone(),
        rhm_basename: pending.rhm_basename.clone(),
        // These fields exist for the picker and mission loader; Lua startup
        // only needs the archive, basename, and compatibility tag.
        map_filename: String::new(),
        requires_spellforge: pending.requires_spellforge,
    };
    if let Some(session) = LuaSession::start_for_launch(&launch, &pending.mods_root)? {
        tracing::info!(
            "LuaSession installed for mission '{}'",
            session.mission_basename()
        );
        host.scripting.lua_session = Some(session);
    }
    Ok(())
}

/// Borrow menu resources required by a confirmation or pause-menu action.
///
/// Original: `original-code/RHMenuIngame.cpp:297-310` constructs the Really
/// Quit Yes/No menu and changes the game operation only for `YES`; resource
/// absence cannot be interpreted as confirmation.
pub(super) fn required_menu_resources<'a>(
    resources: &'a Option<IngameMenuResources>,
    context: &str,
) -> &'a IngameMenuResources {
    resources
        .as_ref()
        .unwrap_or_else(|| panic!("{context}: in-game menu resources are missing"))
}

pub(super) fn selected_pc_profile_indices(
    engine: &engine_api::Engine,
    seat: engine_player_command::PlayerId,
) -> Vec<engine_profiles::CharacterProfileIdx> {
    engine
        .seat_selection(seat)
        .iter()
        .filter_map(|&id| match engine.get_entity(id)? {
            engine_element::Entity::Pc(pc) => Some(pc.pc.profile_index),
            _ => None,
        })
        .collect()
}

/// Ensure a mission that bypassed campaign selection still has an exact
/// save/restart boundary before its Engine exists. Existing session/replay
/// checkpoints are authoritative and are never overwritten.
pub(crate) fn establish_mission_restart_boundary(
    mut campaign: Campaign,
    rng_seed: u64,
    sim_config: engine_api::SimConfig,
) -> Campaign {
    if !campaign.has_restart_simulation_checkpoint() {
        campaign.snapshot_preselected_with_simulation(rng_seed, sim_config);
    }
    campaign
}

/// Restore construction-time simulation controls for a mission restart while
/// retaining the profile setting the player deterministically changed during
/// the just-finished attempt. Replay restarts must instead return to their
/// exact header config and let the recorded command reapply the edit.
fn simulation_config_for_level_restart(
    mut checkpoint: engine_api::SimConfig,
    outcome: engine_api::SimConfig,
    replay_restart: bool,
) -> engine_api::SimConfig {
    if !replay_restart {
        checkpoint.amount_of_speaking = outcome.amount_of_speaking;
    }
    checkpoint
}

pub(crate) async fn run_mission_headless(
    callbacks: &mut RustCallbacks,
    mut campaign: Campaign,
    profiles: &engine_profiles::ProfileManager,
    mission_idx: usize,
    location: MissionLocation,
    args: &crate::main_entry::CliArgs,
    mut rng_seed: u64,
    mut sim_config: engine_api::SimConfig,
) -> MissionOutcome {
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
        let outcome_sim_config = outcome.sim_config;
        campaign = outcome.campaign;
        if let Some((replay_campaign, replay_seed, replay_config)) = &replay_restart {
            campaign = replay_campaign.clone();
            rng_seed = *replay_seed;
            sim_config =
                simulation_config_for_level_restart(*replay_config, outcome_sim_config, true);
        } else {
            if !campaign.restore_snapshot() || !campaign.pre_mission_was_preselected {
                return MissionOutcome::new(
                    campaign,
                    rng_seed,
                    sim_config,
                    Err(
                        "direct LevelRestart is missing its preselected mission checkpoint"
                            .to_string(),
                    ),
                );
            }
            let checkpoint = campaign.restart_simulation_checkpoint();
            rng_seed = checkpoint.0;
            sim_config =
                simulation_config_for_level_restart(checkpoint.1, outcome_sim_config, false);
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
    mut campaign: Campaign,
    profiles: &engine_profiles::ProfileManager,
    application_context: &ApplicationContext,
    args: &crate::main_entry::CliArgs,
    initial_load: Option<SaveLoadRequest>,
) -> SessionOutcome {
    let mut callbacks = RustCallbacks::new(application_context.clone());
    callbacks.pending = initial_load;
    if args.replay_data.is_some() || args.replay.is_some() {
        return SessionOutcome {
            campaign,
            result: Err("replay missions must bypass campaign selection and launch directly from their header".to_string()),
        };
    }
    let mut authoritative_rng_seed = 0;
    let mut authoritative_sim_config = setup::initial_sim_config(args);
    let mut preselected_mission = None;
    if let Some(SaveLoadRequest::Load {
        slot,
        mission_id,
        save,
    }) = callbacks.pending.take()
    {
        let (slot, save) = match crate::main_entry::preflight_or_use_decoded_load(
            &callbacks.save_manager,
            slot,
            save,
        ) {
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
        let target_idx = match crate::main_entry::validate_save_mission(&save, profiles) {
            Ok(idx) => idx,
            Err(error) => {
                return SessionOutcome {
                    campaign,
                    result: Err(format!("save preflight failed: {error}")),
                };
            }
        };
        (authoritative_rng_seed, authoritative_sim_config) = save.engine.mission_start_simulation();
        campaign = save.engine.campaign().clone();
        preselected_mission = Some(target_idx);
        callbacks.pending = Some(SaveLoadRequest::Load {
            slot: Some(slot),
            mission_id,
            save: Some(save),
        });
    }
    let mut replay_restart: Option<crate::http_server::PendingReplay> = None;
    loop {
        let pending_replay = choose_pending_replay(
            crate::http_server::take_pending_replay(),
            &mut replay_restart,
        );
        let mut mission_args_storage = None;
        let mut replay_for_restart = None;
        let (mission_idx, location, restart_rng_seed, restart_sim_config) = if let Some(pending) =
            pending_replay
        {
            let paused = pending.paused;
            let replay_copy = pending.data.clone();
            let prepared = match prepare_replay_mission(profiles, args, pending.data, paused) {
                Ok(prepared) => prepared,
                Err(error) => {
                    return SessionOutcome {
                        campaign,
                        result: Err(error),
                    };
                }
            };
            campaign = prepared.0;
            authoritative_rng_seed = prepared.4;
            authoritative_sim_config = prepared.5;
            mission_args_storage = Some(prepared.3);
            replay_for_restart = Some(crate::http_server::PendingReplay {
                data: replay_copy,
                paused,
            });
            (
                prepared.1,
                prepared.2,
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
        let mission_args = mission_args_storage.as_ref().unwrap_or(args);

        // Sherwood is a real loaded mission (level geometry, PCs,
        // NPCs, production sectors, script). The campaign map is an
        // overlay toggled via the DisplayCampaignMap widget. Fall
        // through to `run_mission` — Sherwood-specific behavior
        // (campaign-map overlay, Start/Quit-mission widgets,
        // `SerializeForSherwood` on mission confirm) is wired inside
        // the per-frame loop.

        // Run the actual mission
        tracing::info!("Starting mission idx={} at {:?}", mission_idx, location);
        let mission_outcome = run_mission_with_seed(
            window,
            &mut callbacks,
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
                tracing::info!("Restarting mission idx={}", mission_idx);
                continue;
            }
            GameCode::LevelLoad => {
                // Cross-mission load: `perform_pending_save_load` left the
                // slot + target mission in `pending_level_load` and forced
                // the Game state machine into LevelLoad so `run_mission`
                // exited. Switch the campaign to the target mission and
                // re-queue the Load on the fresh engine.
                let Some(req) = callbacks.pending_level_load.take() else {
                    tracing::warn!("LevelLoad exit without a pending load — returning to map");
                    continue;
                };
                let idx = match crate::main_entry::validate_save_mission(&req.save, profiles) {
                    Ok(idx) => idx,
                    Err(error) => {
                        return SessionOutcome {
                            campaign,
                            result: Err(format!("cross-mission save became invalid: {error}")),
                        };
                    }
                };
                tracing::info!(
                    "Cross-mission load: switching to mission id={} (idx={}) and applying slot {}",
                    req.target_mission_id,
                    idx,
                    req.slot,
                );
                let save = req.save;
                (authoritative_rng_seed, authoritative_sim_config) =
                    save.engine.mission_start_simulation();
                campaign = save.engine.campaign().clone();
                preselected_mission = Some(idx);
                // Queue the Load again so the first frame of the
                // new mission applies the save to its fresh engine.
                callbacks.pending = Some(SaveLoadRequest::Load {
                    slot: Some(req.slot),
                    mission_id: req.target_mission_id,
                    save: Some(save),
                });
                continue;
            }
            _ => {}
        }
    }
}

/// Run a single mission game loop.
///
/// Creates a Game + Engine, runs frames until the mission ends.
/// Returns the exit GameCode.
/// Cross-mission quick-load confirmation modal.
///
/// Decode and strictly validate the exact queued QuickLoad payload before
/// deciding whether a cross-mission confirmation is required. The decoded
/// bytes are carried into `Load`, so neither a stale `saves.json` entry nor a
/// file replacement after the modal can change what is eventually applied.
#[allow(clippy::too_many_arguments)]
async fn confirm_quickload_cross_mission(
    callbacks: &mut RustCallbacks,
    engine: &Engine,
    profiles: &engine_profiles::ProfileManager,
    _host: &Host,
    event_pump: &mut GameWindow,
    renderer: &mut Renderer,
    cursor_res: &mut ResourceManager,
    cursor_renderer: &mut CursorRenderer,
    menu_resources: &Option<IngameMenuResources>,
) {
    let use_backup = match callbacks.pending {
        Some(SaveLoadRequest::QuickLoad { use_backup }) => use_backup,
        _ => return,
    };
    let slot_name = if use_backup {
        special_slots::EX_QUICK
    } else {
        special_slots::QUICK
    };
    let Some(idx) = callbacks.save_manager.find_by_filename(slot_name) else {
        return;
    };
    if !callbacks.save_manager.slot_file_exists(idx) {
        return;
    }
    let save = match callbacks.save_manager.preflight_exact_slot(idx) {
        Ok(save) => save,
        Err(error) => {
            tracing::error!("QuickLoad confirmation preflight failed for {slot_name}: {error:#}");
            callbacks.pending = None;
            return;
        }
    };
    if let Err(error) = callbacks.save_manager.validate_slot_identity(idx, &save) {
        tracing::error!("QuickLoad confirmation rejected stale {slot_name} slot: {error:#}");
        callbacks.pending = None;
        return;
    }
    let current = current_mission_id(engine.campaign(), profiles);
    let target_mission_id = match validated_save_reload_target(&save, profiles, current) {
        Ok(target) => target,
        Err(error) => {
            tracing::error!("QuickLoad confirmation rejected {slot_name}: {error}");
            callbacks.pending = None;
            return;
        }
    };
    if target_mission_id.is_none() {
        callbacks.pending = Some(SaveLoadRequest::Load {
            slot: Some(idx),
            mission_id: current,
            save: Some(save),
        });
        return;
    }
    let resources = required_menu_resources(menu_resources, "cross-mission QuickLoad confirmation");
    let msg = resources.menu_text.get(MT_MSG_REALLY_LOAD_QUICKSAVE);
    let cursor = Some(default_modal_cursor(cursor_renderer, cursor_res, renderer));
    let confirmed = show_yesno(event_pump, renderer, resources, cursor, &msg).await;
    if confirmed {
        // Route the already-validated payload through the regular `Load`
        // arm so its cross-mission plumbing switches immutable level assets.
        callbacks.pending = Some(SaveLoadRequest::Load {
            slot: Some(idx),
            mission_id: current,
            save: Some(save),
        });
    } else {
        callbacks.pending = None;
    }
}

pub(crate) async fn run_mission(
    window: &mut GameWindow,
    callbacks: &mut RustCallbacks,
    mut campaign: Campaign,
    profiles: &engine_profiles::ProfileManager,
    mission_idx: usize,
    location: MissionLocation,
    args: &crate::main_entry::CliArgs,
    mut rng_seed: u64,
    mut sim_config: engine_api::SimConfig,
) -> MissionOutcome {
    let replay_restart = args
        .replay_data
        .as_ref()
        .map(|_| (campaign.clone(), rng_seed, sim_config));
    loop {
        let outcome = run_mission_with_seed(
            window,
            callbacks,
            campaign,
            profiles,
            mission_idx,
            location,
            args,
            rng_seed,
            sim_config,
            MultiplayerSetupFailurePolicy::Fatal,
        )
        .await;
        if !matches!(&outcome.result, Ok(GameCode::LevelRestart)) {
            return outcome;
        }
        let outcome_sim_config = outcome.sim_config;
        campaign = outcome.campaign;
        if let Some((replay_campaign, replay_seed, replay_config)) = &replay_restart {
            campaign = replay_campaign.clone();
            rng_seed = *replay_seed;
            sim_config =
                simulation_config_for_level_restart(*replay_config, outcome_sim_config, true);
        } else {
            if !campaign.restore_snapshot() || !campaign.pre_mission_was_preselected {
                return MissionOutcome::new(
                    campaign,
                    rng_seed,
                    sim_config,
                    Err(
                        "direct LevelRestart is missing its preselected mission checkpoint"
                            .to_string(),
                    ),
                );
            }
            let checkpoint = campaign.restart_simulation_checkpoint();
            rng_seed = checkpoint.0;
            sim_config =
                simulation_config_for_level_restart(checkpoint.1, outcome_sim_config, false);
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_mission_with_seed(
    window: &mut GameWindow,
    callbacks: &mut RustCallbacks,
    campaign: Campaign,
    profiles: &engine_profiles::ProfileManager,
    mission_idx: usize,
    location: MissionLocation,
    args: &crate::main_entry::CliArgs,
    rng_seed: u64,
    sim_config: engine_api::SimConfig,
    multiplayer_setup_failure_policy: MultiplayerSetupFailurePolicy,
) -> MissionOutcome {
    let campaign = establish_mission_restart_boundary(campaign, rng_seed, sim_config);
    let mut mission = match InteractiveMissionBuilder::build(
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
        InteractiveBuildOutcome::Ready(mission) => mission,
        InteractiveBuildOutcome::Finished(outcome) => return outcome,
    };
    let outcome = mission.run(window, callbacks, profiles, args).await;
    mission.finish(outcome)
}

#[cfg(test)]
mod required_state_tests {
    use super::{
        MissionOutcome, allied_portrait_center, choose_pending_replay,
        establish_mission_restart_boundary, prepare_replay_mission, required_menu_resources,
        simulation_config_for_level_restart,
    };
    use crate::ingame_menu::IngameMenuResources;
    use robin_engine::campaign::{Campaign, CampaignValue};
    use robin_engine::game_operation::GameCode;
    use robin_engine::mission::Mission;
    use robin_engine::profiles::{MissionProfile, ProfileManager};
    use robin_engine::replay::{ReplayFile, ReplayHeader};
    use std::collections::BTreeMap;

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
            let mut element = ElementData {
                kind: ElementKind::ActorSoldier,
                active: true,
                ..Default::default()
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
        let mut sim_config = robin_engine::engine::SimConfig::default();
        sim_config.highlander2 = true;
        let data = ReplayFile {
            header: ReplayHeader {
                mission_id: "MissionA".into(),
                rng_seed: 0x2020,
                sim_config,
                version: robin_engine::replay::REPLAY_SCHEMA_VERSION,
                total_frames: 0,
                campaign: bitcode::serialize(&campaign).unwrap(),
            },
            frames: BTreeMap::new(),
            hashes: BTreeMap::new(),
            save_markers: BTreeMap::new(),
            load_backs: BTreeMap::new(),
        }
        .into();
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
        let mut campaign = Campaign::default();
        campaign.current_mission_idx = Some(3);
        campaign.next_mission_idx = None;
        campaign.values[CampaignValue::Custom20] = 17;
        let mut config = robin_engine::engine::SimConfig::default();
        config.amount_of_speaking = 8;

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
        let mut checkpoint = robin_engine::engine::SimConfig::default();
        checkpoint.amount_of_speaking = 3;
        checkpoint.highlander2 = true;
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
        let mut display = robin_engine::engine::HostDisplayState::default();
        let mut input = robin_engine::engine::InputState::default();
        engine
            .advance_frame(
                &mut display,
                &mut input,
                &assets,
                &mut robin_engine::engine::DevState::default(),
                robin_engine::engine::SimulationFrameInput::new(vec![
                    robin_engine::player_command::PlayerCommand::SetAmountOfSpeaking { amount: 9 }
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
    fn session_restart_preserves_commanded_amount_of_speaking_only() {
        let (checkpoint, outcome) = commanded_level_restart_fixture();
        assert!(matches!(outcome.result, Ok(GameCode::LevelRestart)));

        let restarted = simulation_config_for_level_restart(checkpoint, outcome.sim_config, false);

        assert_eq!(restarted.amount_of_speaking, 9);
        assert!(restarted.highlander2, "other construction config resets");
    }

    #[test]
    fn direct_restart_preserves_commanded_amount_of_speaking_only() {
        let (checkpoint, outcome) = commanded_level_restart_fixture();
        assert!(matches!(outcome.result, Ok(GameCode::LevelRestart)));

        let restarted = simulation_config_for_level_restart(checkpoint, outcome.sim_config, false);

        assert_eq!(restarted.amount_of_speaking, 9);
        assert!(restarted.highlander2, "direct launch uses its checkpoint");
    }

    #[test]
    fn replay_restart_keeps_exact_frame_zero_config_for_command_replay() {
        let (checkpoint, outcome) = commanded_level_restart_fixture();
        assert!(matches!(outcome.result, Ok(GameCode::LevelRestart)));

        let restarted = simulation_config_for_level_restart(checkpoint, outcome.sim_config, true);

        assert_eq!(restarted, checkpoint);
        assert_eq!(restarted.amount_of_speaking, 3);
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
    fn replay_preparation_restores_all_frame_zero_metadata() {
        let (profiles, data) = replay_fixture(Some(0));
        let args = crate::main_entry::CliArgs::default();

        let (campaign, mission_idx, location, prepared_args, seed, config) =
            prepare_replay_mission(&profiles, &args, data, true).unwrap();

        assert_eq!(campaign.current_mission_idx, Some(0));
        assert_eq!(mission_idx, 0);
        assert_eq!(
            location,
            robin_engine::profiles::MissionLocation::Nottingham
        );
        assert_eq!(seed, 0x2020);
        assert!(config.highlander2);
        assert!(prepared_args.start_paused);
        assert!(prepared_args.replay.is_none());
        assert_eq!(prepared_args.replay_data.unwrap().header.sim_config, config);
    }

    #[test]
    fn replay_preparation_rejects_current_mission_mismatch() {
        let (profiles, data) = replay_fixture(None);
        let error = prepare_replay_mission(
            &profiles,
            &crate::main_entry::CliArgs::default(),
            data,
            false,
        )
        .unwrap_err();
        assert!(error.contains("current mission None"));
    }

    #[test]
    fn newly_queued_replay_wins_over_restart_fallback() {
        let (_, queued_data) = replay_fixture(Some(0));
        let (_, mut restart_data) = replay_fixture(Some(0));
        restart_data.header.rng_seed = 0x3030;
        let mut restart = Some(crate::http_server::PendingReplay {
            data: restart_data,
            paused: false,
        });

        let selected = choose_pending_replay(
            Some(crate::http_server::PendingReplay {
                data: queued_data,
                paused: true,
            }),
            &mut restart,
        )
        .unwrap();

        assert_eq!(selected.data.header.rng_seed, 0x2020);
        assert!(selected.paused);
        assert!(restart.is_none(), "new replay must discard the old restart");
    }

    #[test]
    fn completed_new_replay_cannot_resurrect_the_old_restart() {
        for terminal_code in [GameCode::LevelSucceeded, GameCode::Quit] {
            let (_, queued_data) = replay_fixture(Some(0));
            let (_, mut restart_data) = replay_fixture(Some(0));
            restart_data.header.rng_seed = 0x3030;
            let mut restart = Some(crate::http_server::PendingReplay {
                data: restart_data,
                paused: false,
            });

            let selected = choose_pending_replay(
                Some(crate::http_server::PendingReplay {
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
