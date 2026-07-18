//! Game session: mission selection loop and the per-mission game loop.

mod bootstrap;
mod debriefing;
mod dispatch;
mod flow;
mod frame_prepare;
mod frame_simulate;
mod headless;
mod input_handlers;
mod interactive;
mod modal_state;
mod mouse_input;
mod multiplayer;
mod render;
mod replay_init;
mod runtime;
mod setup;
mod tick;

use bootstrap::{
    HeadlessBuildOutcome, HeadlessMissionBuilder, InteractiveBuildOutcome,
    InteractiveMissionBuilder,
};
use debriefing::{
    SettledDebriefingOutcome, final_debriefing_outcome_from_replay, final_debriefing_result,
};
use dispatch::apply_local_viewport_scroll;
pub(crate) use dispatch::{dispatch_local_command, dispatch_local_commands};
use frame_simulate::{FrameSimulationFlags, FrameSimulationOutcome, InteractiveFrameSimulation};
use input_handlers::{handle_console_overlay_events, handle_gamepad_events, handle_hold_to_rewind};
use interactive::{InteractiveFrontend, InteractiveMission, RenderViewState};
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
    accept_host_frame_schedule, drain_net_inputs, host_scheduled_frame_deadline_ms,
    setup_multiplayer_session,
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

use crate::Host;
use crate::app_effect::{AppEffect, SoundMode};
use crate::campaign::Campaign;
use crate::corner_hud::{CornerButton, CornerButtonEnable, CornerHudLayout};
use crate::cursor::CursorRenderer;
use crate::game::GameCallbacks;
use crate::game_operation::GameCode;
use crate::gfx_types::GameEvent;
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
    perform_pending_save_load, required_mission_id,
};
use crate::main_menu::custom_missions::CustomMissionLaunch;
use crate::multiplayer::lobby::current_epoch_ms;
use crate::player_command::{PlayerCommand, PlayerInput};
use crate::profiles::MissionLocation;
use crate::renderer::Renderer;
use crate::resource_manager::ResourceManager;
use crate::save_file::special_slots;
use crate::stature_hud::{StatureButton, StatureEnable, StatureHudLayout};
use crate::ui_panel::PortraitHitArea;
use crate::window::GameWindow;
use crate::zoom_hud::{ZoomButton, ZoomButtonEnable};

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
    pub(crate) result: Result<GameCode, String>,
}

impl MissionOutcome {
    pub(crate) fn new(campaign: Campaign, result: Result<GameCode, String>) -> Self {
        Self { campaign, result }
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
        host.lua_session = Some(session);
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

pub(crate) async fn run_mission_headless(
    callbacks: &mut RustCallbacks,
    campaign: Campaign,
    profiles: &engine_profiles::ProfileManager,
    mission_idx: usize,
    location: MissionLocation,
    args: &crate::main_entry::CliArgs,
) -> MissionOutcome {
    let mut mission = match HeadlessMissionBuilder::build(
        callbacks,
        campaign,
        profiles,
        mission_idx,
        location,
        args,
    ) {
        HeadlessBuildOutcome::Ready(mission) => mission,
        HeadlessBuildOutcome::Finished(outcome) => return outcome,
    };
    let outcome = mission.run(args).await;
    mission.finish(outcome)
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
    args: &crate::main_entry::CliArgs,
    initial_load: Option<SaveLoadRequest>,
) -> SessionOutcome {
    let mut callbacks = RustCallbacks::new();
    callbacks.pending = initial_load;

    loop {
        // Determine the next mission to play
        let mission_idx = campaign.determine_next_mission(profiles);

        let location = campaign.missions[mission_idx].profile(profiles).location;

        // Sherwood is a real loaded mission (level geometry, PCs,
        // NPCs, production sectors, script). The campaign map is an
        // overlay toggled via the DisplayCampaignMap widget. Fall
        // through to `run_mission` — Sherwood-specific behavior
        // (campaign-map overlay, Start/Quit-mission widgets,
        // `SerializeForSherwood` on mission confirm) is wired inside
        // the per-frame loop.

        // Capture the pre-mission snapshot for the restart / abandon
        // path.  Taken right before the main mission loop; lives on the
        // campaign itself (serde-skipped) and is consumed by
        // `restore_snapshot` on `LevelRestart` below.
        campaign.snapshot();

        // Run the actual mission
        tracing::info!("Starting mission idx={} at {:?}", mission_idx, location);
        let mission_outcome = run_mission(
            window,
            &mut callbacks,
            campaign,
            profiles,
            mission_idx,
            location,
            args,
        )
        .await;
        campaign = mission_outcome.campaign;
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
                    if let Err(e) =
                        crate::video_player::play_video(window, "Data/Cinematics/Outro.ogg").await
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
                if !campaign.restore_snapshot() {
                    tracing::warn!(
                        "LevelRestart: no pre-mission snapshot to restore — continuing with current campaign state"
                    );
                }
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
                let target_idx = campaign
                    .missions
                    .iter()
                    .position(|m| m.profile(profiles).id == req.target_mission_id);
                match target_idx {
                    Some(idx) => {
                        tracing::info!(
                            "Cross-mission load: switching to mission id={} (idx={}) and applying slot {}",
                            req.target_mission_id,
                            idx,
                            req.slot,
                        );
                        // Use `next_mission_idx` so
                        // `determine_next_mission` honours the override at
                        // the top of the session loop.
                        campaign.next_mission_idx = Some(idx);
                        // Queue the Load again so the first frame of the
                        // new mission applies the save to its fresh engine.
                        callbacks.pending = Some(SaveLoadRequest::Load {
                            slot: Some(req.slot),
                            mission_id: req.target_mission_id,
                        });
                        continue;
                    }
                    None => {
                        tracing::error!(
                            "Cross-mission load: save's mission id={} is not in the current campaign",
                            req.target_mission_id,
                        );
                    }
                }
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
/// Pre-screens a queued `SaveLoadRequest::QuickLoad` before it reaches
/// `perform_pending_save_load`: if the targeted quicksave's mission ID
/// differs from the running mission, ask "Do you really want to load
/// this quicksave?".  On "No" the request is dropped; on "Yes" it is
/// rewritten into `SaveLoadRequest::Load { slot, mission_id: current }`
/// so the existing `Load` arm's `PendingLevelLoad` routing performs the
/// mission switch + re-queue.  When the mission IDs match the request
/// is left untouched and the modal is skipped (load proceeds without
/// prompting).
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
    let target_mission_id = required_mission_id(
        callbacks.save_manager.slot_mission_id(idx),
        "QuickLoad confirmation slot must have a cached mission ID",
    );
    let campaign = engine.campaign();
    let current = current_mission_id(campaign, profiles);
    if target_mission_id == current {
        return;
    }
    let resources = required_menu_resources(menu_resources, "cross-mission QuickLoad confirmation");
    let msg = resources.menu_text.get(MT_MSG_REALLY_LOAD_QUICKSAVE);
    let cursor = Some(default_modal_cursor(cursor_renderer, cursor_res, renderer));
    let confirmed = show_yesno(event_pump, renderer, resources, cursor, &msg).await;
    if confirmed {
        // Route through the regular `Load` arm so its existing
        // `PendingLevelLoad` cross-mission plumbing handles the mission
        // swap + Load re-queue on the fresh engine.  Pass the running
        // mission id so the arm's `header.mission_id != mission_id`
        // check fires.
        callbacks.pending = Some(SaveLoadRequest::Load {
            slot: Some(idx),
            mission_id: current,
        });
    } else {
        callbacks.pending = None;
    }
}

pub(crate) async fn run_mission(
    window: &mut GameWindow,
    callbacks: &mut RustCallbacks,
    campaign: Campaign,
    profiles: &engine_profiles::ProfileManager,
    mission_idx: usize,
    location: MissionLocation,
    args: &crate::main_entry::CliArgs,
) -> MissionOutcome {
    let mut mission = match InteractiveMissionBuilder::build(
        window,
        callbacks,
        campaign,
        profiles,
        mission_idx,
        location,
        args,
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
    use super::{MissionOutcome, required_menu_resources};
    use crate::campaign::{Campaign, CampaignValue};
    use crate::game_operation::GameCode;
    use crate::ingame_menu::IngameMenuResources;

    #[test]
    fn mission_exit_restores_the_exact_campaign_allocation() {
        let mut engine_campaign = Campaign::default();
        engine_campaign.values[CampaignValue::Custom20] = 0x25_25_25;
        let production_sectors = engine_campaign.production_sectors.as_ptr();

        let outcome = MissionOutcome::new(engine_campaign, Ok(GameCode::LevelSucceeded));

        assert_eq!(outcome.campaign.values[CampaignValue::Custom20], 0x25_25_25);
        assert_eq!(
            outcome.campaign.production_sectors.as_ptr(),
            production_sectors
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

            let actual = MissionOutcome::new(engine_campaign, outcome.clone());

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
    #[should_panic(expected = "test confirmation: in-game menu resources are missing")]
    fn confirmation_rejects_missing_menu_resources() {
        let resources: Option<IngameMenuResources> = None;
        required_menu_resources(&resources, "test confirmation");
    }
}
