//! Game session: mission selection loop and the per-mission game loop.

mod bootstrap;
mod dispatch;
mod flow;
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

use bootstrap::{MissionBootstrap, MissionSpec};
use dispatch::apply_local_viewport_scroll;
pub(crate) use dispatch::{dispatch_local_command, dispatch_local_commands};
use headless::HeadlessPolicy;
use input_handlers::{handle_console_overlay_events, handle_gamepad_events, handle_hold_to_rewind};
use interactive::{
    InteractiveFrontend, InteractiveMission, MissionAudio, MissionHud, MissionInput,
    MissionPresentation, MissionResources, MissionUi, RenderViewState,
};
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
use robin_assets::res_descr as assets_res_descr;
use robin_engine::coordinates as engine_coordinates;
use robin_engine::element as engine_element;
use robin_engine::engine as engine_api;
use robin_engine::engine::{Engine, ScrollDirection};
use robin_engine::graphic_config::TextureScaleMode;
use robin_engine::messenger as engine_messenger;
use robin_engine::mission as engine_mission;
use robin_engine::player_command as engine_player_command;
use robin_engine::position_interface as engine_position_interface;
use robin_engine::profiles as engine_profiles;
use robin_engine::sight_obstacle as engine_sight_obstacle;
use runtime::{
    FrameCommitPolicy, FrameOutcome, FramePacing, MissionControl, MissionFrame, MissionRuntime,
    MissionWorld,
};
use setup::{
    LoadedInteractiveResources, MissionSprites, extract_ground_mark_sprite_data,
    extract_minimap_widget_setup, extract_titbit_row_frame_counts, init_audio_backend,
    load_level_and_sprite_bank, load_mission_sprites, pre_decode_maps_and_resources,
    setup_input_and_camera,
};
use tick::{
    dismiss_pending_modals, drain_steps, modal_state_pending, post_render_engine_cleanup,
    pre_render_engine_setup,
};

use crate::Host;
use crate::app_effect::{AppEffect, SoundMode};
use crate::audio_backend;
use crate::campaign::Campaign;
use crate::corner_hud::{
    CornerButton, CornerButtonEnable, CornerButtonSprites, CornerHudLayout, CornerTooltipTracker,
};
use crate::cursor::CursorRenderer;
use crate::game::{Game, GameCallbacks};
use crate::game_operation::GameCode;
use crate::gfx_types::GameEvent;
use crate::host::PrintScreenRequest;
use crate::ingame_menu::resources::{
    MT_MSG_LEAVE_MISSION_NOW, MT_MSG_REALLY_LOAD_QUICKSAVE, MT_MSG_STRATEGICAL_MISSION_LOST,
};
use crate::ingame_menu::widget_bridge::default_modal_cursor;
use crate::ingame_menu::{
    DebriefingOutcome, IngameMenuResources, MissionStatePopupState, PauseMenu, SaveLoadMode,
    SaveLoadOutcome, show_yesno,
};
use crate::input_translator::GameKey;
use crate::input_translator::{GameAction, TranslationFlags};
use crate::loading_screen::{LoadingDatadirKind, LoadingScreenRenderer};
use crate::lua_session::LuaSession;
use crate::main_entry::{
    RustCallbacks, SaveBannerKind, SaveLoadRequest, current_mission_id, detect_demo_mode,
    execute_app_effects, perform_pending_save_load, required_mission_id, resolve_loading_pak,
};
use crate::main_menu::custom_missions::CustomMissionLaunch;
use crate::multiplayer::lobby::current_epoch_ms;
use crate::player_command::{PlayerCommand, PlayerInput};
use crate::player_profile::PlayerProfileManager;
use crate::profiles::MissionLocation;
use crate::renderer::Renderer;
use crate::resource_manager::ResourceManager;
use crate::save_file::special_slots;
use crate::sherwood_hud::{
    SherwoodButtonEnable, SherwoodButtonSprites, SherwoodHudLayout, SherwoodTooltipTracker,
};
use crate::stature_hud::{
    StatureButton, StatureEnable, StatureHudLayout, StatureSprites, StatureTooltipTracker,
};
use crate::ui_panel::{
    BlazonTooltipTracker, PcActionTooltipTracker, PortraitHitArea, RequirementsTooltipTracker,
};
use crate::window::GameWindow;
use crate::zoom_hud::{
    ZoomButton, ZoomButtonEnable, ZoomButtonSprites, ZoomHudLayout, ZoomTooltipTracker,
};

/// Read the active player profile's texture scale mode, falling back to
/// the default (`Linear`) if no profile is loaded yet.
fn active_profile_scale_mode() -> TextureScaleMode {
    let guard = PlayerProfileManager::global();
    guard
        .as_ref()
        .and_then(|m| m.get_active())
        .map(|p| p.graphic_config.scale_mode)
        .unwrap_or_default()
}

fn active_profile_shader_preset() -> String {
    let guard = PlayerProfileManager::global();
    guard
        .as_ref()
        .and_then(|m| m.get_active())
        .map(|p| p.graphic_config.shader_preset.clone())
        .unwrap_or_default()
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

/// Outcome of a game session (series of missions).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionResult {
    /// Player chose to return to the main menu.
    QuitToMenu,
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

/// Move the campaign back out of an engine at a mission-loop boundary.
///
/// Original: `original-code/RHCampaign.cpp:38-55` installs the concrete
/// campaign as a singleton, while `original-code/launcher.cpp:956-958` owns
/// that campaign across mission runs. There is no empty/default campaign to
/// substitute if the engine loses ownership unexpectedly.
fn restore_required_campaign(
    campaign_ref: &mut Campaign,
    campaign: Option<Campaign>,
    context: &str,
) {
    let campaign = campaign.unwrap_or_else(|| panic!("{context}: engine campaign is missing"));
    restore_campaign_value(campaign_ref, campaign);
}

fn restore_campaign_value(campaign_ref: &mut Campaign, campaign: Campaign) {
    *campaign_ref = campaign;
}

/// Session-side owner of the campaign while an active mission leases its value
/// to the Engine.
///
/// Loading is the only ownership-transfer boundary: once
/// `load_level_and_sprite_bank` succeeds, every controlled mission outcome is
/// routed through [`Self::finish`]. Consuming the lease makes restoration a
/// once-only operation while still leaving the Engine as the live campaign
/// owner for saves, deterministic ticks, and Sherwood transitions.
#[must_use = "an active mission campaign lease must be finalized"]
struct MissionCampaignLease<'a> {
    session_campaign: &'a mut Campaign,
}

impl<'a> MissionCampaignLease<'a> {
    fn new(session_campaign: &'a mut Campaign) -> Self {
        Self { session_campaign }
    }

    fn session_campaign(&mut self) -> &mut Campaign {
        self.session_campaign
    }

    /// Restore the campaign after all mission-local exit work has run, then
    /// propagate the already-decided mission result unchanged.
    fn finish(
        self,
        engine: &mut engine_api::Engine,
        outcome: Result<GameCode, String>,
        context: &str,
    ) -> Result<GameCode, String> {
        self.finish_campaign(engine.take_campaign(), outcome, context)
    }

    fn finish_campaign<T>(self, campaign: Option<Campaign>, outcome: T, context: &str) -> T {
        restore_required_campaign(self.session_campaign, campaign, context);
        outcome
    }
}

/// Construct the optional custom-mission Lua state before level loading.
/// A Spellforge-tagged launch treats construction as required; only Vanilla
/// custom missions may legitimately produce no session.
fn install_pending_lua_session(
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
    campaign_ref: &mut Campaign,
    profiles: &engine_profiles::ProfileManager,
    mission_idx: usize,
    location: MissionLocation,
    args: &crate::main_entry::CliArgs,
) -> Result<GameCode, String> {
    crate::lua_session::validate_launch_mode(
        args,
        crate::http_server::peek_pending_replay_mission_id().is_some(),
    )
    .map_err(|error| error.to_string())?;
    let mut host = Host::new(1024.0, 768.0);
    install_pending_lua_session(&mut host, args).map_err(|error| error.to_string())?;
    if let Err(e) = setup_multiplayer_session(&mut host, args) {
        tracing::error!("{e}; aborting headless mission");
        return Ok(GameCode::Quit);
    }

    let mut game = Game::new(location);
    game.global_options = args.global_options.clone();

    let mut text_res = ResourceManager::new();
    if let Err(e) =
        text_res.attach_or_from_shipping("Data/Text/Level.res", host.shipping.as_deref())
    {
        tracing::warn!("Failed to load text resource file: {e}");
    }

    let mut cursor_res = ResourceManager::new();
    if let Err(e) =
        cursor_res.attach_or_from_shipping("Data/Interface/DEFAULT.RES", host.shipping.as_deref())
    {
        tracing::warn!("Failed to load cursor resource file: {e}");
    }
    let ground_mark_sprite = extract_ground_mark_sprite_data(&mut cursor_res);
    if let Some(data) = ground_mark_sprite.as_ref() {
        host.install_trajectory_ground_mark_sprite(data);
    }
    let titbit_row_frame_counts = extract_titbit_row_frame_counts(&mut cursor_res);
    let minimap_widget = extract_minimap_widget_setup(&mut cursor_res);

    let loaded = load_level_and_sprite_bank(
        None,
        &mut None,
        &mut host,
        &mut game,
        campaign_ref,
        profiles,
        &mut text_res,
        args,
        1024.0,
        768.0,
        ground_mark_sprite,
        titbit_row_frame_counts,
        minimap_widget,
    )?;

    let mut bootstrap = MissionBootstrap::new(
        MissionSpec::headless(mission_idx, location),
        host,
        game,
        loaded,
    );
    let campaign_lease = MissionCampaignLease::new(campaign_ref);
    if let Err(error) = bootstrap.start_required_spellforge() {
        return campaign_lease.finish(
            &mut bootstrap.loaded.engine,
            Err(error.to_string()),
            "headless Spellforge startup failure",
        );
    }
    bootstrap.prepare_audio(None, profiles);
    // True headless has no HUD, renderer, dialogue widgets, or debriefing
    // frontend. The engine-required background dimensions were decoded before
    // construction; loading level descriptors and HUD fonts here was purely
    // graphical work.
    bootstrap.loaded.engine.campaign_reset_mission_length();
    <RustCallbacks as crate::game::GameCallbacks>::start_play_time(callbacks);
    let mut mission = bootstrap.finish_headless(args, profiles, HeadlessPolicy::replay_runner());

    loop {
        let frame_result = mission.run_frame(args);
        match frame_result.outcome {
            FrameOutcome::Exit(code) => {
                let context = frame_result
                    .exit
                    .expect("runtime exit must have a campaign finalization context")
                    .campaign_restore_context();
                return campaign_lease.finish(
                    &mut mission.runtime.world.manager.engine,
                    Ok(code),
                    context,
                );
            }
            FrameOutcome::Continue { sleep_ms } if frame_result.paused => {
                crate::window::sleep_ms(u64::from(sleep_ms.max(10))).await;
            }
            FrameOutcome::Continue { sleep_ms: 0 } => crate::window::yield_to_runtime().await,
            FrameOutcome::Continue { sleep_ms } => {
                crate::window::sleep_ms(u64::from(sleep_ms)).await;
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
    campaign: &mut Campaign,
    profiles: &engine_profiles::ProfileManager,
    args: &crate::main_entry::CliArgs,
    initial_load: Option<SaveLoadRequest>,
) -> Result<SessionResult, String> {
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
        let game_result = run_mission(
            window,
            &mut callbacks,
            campaign,
            profiles,
            mission_idx,
            location,
            args,
        )
        .await?;

        match game_result {
            GameCode::Quit => return Ok(SessionResult::QuitToMenu),
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
                return Ok(SessionResult::QuitToMenu);
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
/// Resolved outcome of the post-mission debriefing flow after the
/// caller has driven any Load picker re-entry loop.  Differs from
/// [`crate::ingame_menu::DebriefingOutcome`] only in that
/// `LoadAttempt` (the "user clicked Load, picker not yet run") is
/// resolved into either `Load { slot }` (slot picked) or absorbed
/// back into the loop (cancelled).
enum SettledDebriefingOutcome {
    Ok,
    Restart,
    Load { slot: usize },
    EmergencyEnd,
}

fn final_debriefing_result(
    outcome: &SettledDebriefingOutcome,
) -> engine_player_command::DialogResult {
    match outcome {
        SettledDebriefingOutcome::Ok => engine_player_command::DialogResult::Completed,
        SettledDebriefingOutcome::Restart => engine_player_command::DialogResult::Restart,
        SettledDebriefingOutcome::Load { slot } => {
            engine_player_command::DialogResult::Load { slot: *slot as u32 }
        }
        SettledDebriefingOutcome::EmergencyEnd => engine_player_command::DialogResult::Aborted,
    }
}

fn final_debriefing_outcome_from_replay(
    result: engine_player_command::DialogResult,
) -> SettledDebriefingOutcome {
    match result {
        engine_player_command::DialogResult::Completed => SettledDebriefingOutcome::Ok,
        engine_player_command::DialogResult::Aborted => SettledDebriefingOutcome::EmergencyEnd,
        engine_player_command::DialogResult::Restart => SettledDebriefingOutcome::Restart,
        engine_player_command::DialogResult::Load { slot } => SettledDebriefingOutcome::Load {
            slot: slot as usize,
        },
    }
}

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
    let campaign = engine
        .campaign()
        .expect("QuickLoad confirmation requires the engine campaign");
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
    campaign_ref: &mut Campaign,
    profiles: &engine_profiles::ProfileManager,
    mission_idx: usize,
    location: MissionLocation,
    args: &crate::main_entry::CliArgs,
) -> Result<GameCode, String> {
    crate::lua_session::validate_launch_mode(
        args,
        crate::http_server::peek_pending_replay_mission_id().is_some(),
    )
    .map_err(|error| error.to_string())?;
    // ── Loading screen ──
    // Show a sand-dissolve loading screen while initializing the mission.
    // Uses its own Renderer at the .pak image resolution; presentation scales
    // it to fill the window. Dropped before the game Renderer is created.

    // Drain any pending WM resize events BEFORE creating the loading screen
    // renderer.  Without this, a window manager that snapped our requested
    // 1024x768 to a different size leaves a Resized event in the queue that
    // the loading-screen loop never polls, so its Renderer is built against
    // stale canvas dimensions.  `poll_events_split` also snaps to a supported
    // 4:3 resolution, matching what the main game loop does on resize.
    let _ = window.poll_events();

    // Progress bar units — one unit roughly per phase boundary in the
    // loading pipeline.  Splitting the work finely keeps the bar moving
    // smoothly; missing a unit just means the bar stops short of the end,
    // which is fine.
    const LOADING_MAX_LEVEL: f32 = 22.0;
    // Look up the mission's proto-level filename so the loading screen can
    // probe the per-ambience pak (`Data/Levels/<ambience:%02u>/<proto>.pak`)
    // before falling back to the default. The exact ambience comes from the
    // `.rhm` header which we haven't opened yet — pass `None` and let the
    // resolver probe Day/Fog/Night in turn (only one ever exists per mission).
    let proto_level_filename: Option<String> = campaign_ref
        .missions
        .get(mission_idx)
        .map(|m| m.profile(profiles).proto_level_filename.clone());
    let loading_pak = resolve_loading_pak(proto_level_filename.as_deref(), None);
    let scale_mode = active_profile_scale_mode();
    let shader_preset = active_profile_shader_preset();
    // `--headless`: no screen, no reason to composite the loading .pak.
    // Skip the renderer build so the sand-dissolve animation and its
    // per-phase `set_status` paints are inert.
    let mut loading_screen = if args.headless {
        None
    } else {
        loading_pak.and_then(|path| {
            let datadir_kind = match detect_demo_mode().map(|(_, _, _, location)| location) {
                Some(MissionLocation::Leicester) => LoadingDatadirKind::DemoI,
                Some(MissionLocation::Lincoln) => LoadingDatadirKind::DemoII,
                _ => LoadingDatadirKind::FullGame,
            };
            LoadingScreenRenderer::new(window, &path, datadir_kind, LOADING_MAX_LEVEL, scale_mode)
        })
    };
    if let Some(ref mut ls) = loading_screen {
        ls.set_status("Initializing audio...", 0.02);
        ls.refresh(); // show initial state (0%)
        ls.drain_events(&mut *window);
    }

    let mut host = Host::new(window.width as f32, window.height as f32);
    install_pending_lua_session(&mut host, args).map_err(|error| error.to_string())?;
    if let Err(e) = setup_multiplayer_session(&mut host, args) {
        tracing::error!("{e}; returning to main menu");
        if let Some(ref mut ls) = loading_screen {
            ls.set_status("Multiplayer connection failed", 1.0);
            ls.refresh();
            crate::window::sleep_ms(1200).await;
        }
        return Ok(GameCode::Quit);
    }
    let mut game = Game::new(location);
    // Push CLI-derived global options into the Game so any runtime
    // read (dialogue text directory, mission overrides, sound toggles)
    // sees the parsed values.
    game.global_options = args.global_options.clone();

    // ── Early audio backend + menu music ──
    // Create the audio backend before the CPU-only loading block so
    // menu music can play during the loading screen.  See
    // `init_audio_backend` for the full setup.
    // `--headless`: no audio device, no menu music, no mission audio.
    // The frame loop's `audio_backend.is_some()` guards already handle
    // a `None` backend everywhere downstream.
    let mut audio_backend = if args.headless {
        None
    } else {
        init_audio_backend(&mut host, &game)
    };

    if let Some(ref mut ls) = loading_screen {
        ls.set_status("Loading text resources...", 0.05);
    }

    // Attach the Level.res text resource before the CPU loading block
    // so the peasant name pool can be read and handed to `Engine::new`.
    // `load_level_and_sprite_bank` only reads the peasant names out of
    // it; the remaining consumers (portrait cache, short briefings,
    // dialogue tables) pick it up via `pre_decode_maps_and_resources`
    // below.
    let mut text_res = ResourceManager::new();
    if let Err(e) =
        text_res.attach_or_from_shipping("Data/Text/Level.res", host.shipping.as_deref())
    {
        tracing::warn!("Failed to load text resource file: {e}");
    }

    if let Some(ref mut ls) = loading_screen {
        ls.set_status("Loading interface resources...", 0.12);
    }

    // Attach DEFAULT.RES too — we pre-compute the ground-mark sprite
    // metadata and titbit row-frame counts off its picture rows, and
    // `Engine::new` absorbs both so the sim has them on the first
    // tick (ground-mark `add_mark` / titbit animation both read them).
    let mut cursor_res = ResourceManager::new();
    if let Err(e) =
        cursor_res.attach_or_from_shipping("Data/Interface/DEFAULT.RES", host.shipping.as_deref())
    {
        tracing::warn!("Failed to load cursor resource file: {e}");
    }
    let ground_mark_sprite = extract_ground_mark_sprite_data(&mut cursor_res);
    if let Some(data) = ground_mark_sprite.as_ref() {
        host.install_trajectory_ground_mark_sprite(data);
    }
    let titbit_row_frame_counts = extract_titbit_row_frame_counts(&mut cursor_res);
    let minimap_widget = extract_minimap_widget_setup(&mut cursor_res);

    // ── CPU-only loading block ──
    // Constructs the Engine / LevelAssets / DevState, loads the sprite
    // bank, installs the campaign, parses mission scripts, runs
    // `game.initialize_for_mission` (level geometry, entities, scripts),
    // then applies CLI flags, kicks the mission script's StartUp, and —
    // for Sherwood — spawns production bonuses.
    // See `load_level_and_sprite_bank` for the full sequence.
    let screen_w = window.width as f32;
    let screen_h = window.height as f32;
    let loaded = load_level_and_sprite_bank(
        Some(&mut *window),
        &mut loading_screen,
        &mut host,
        &mut game,
        campaign_ref,
        profiles,
        &mut text_res,
        args,
        screen_w,
        screen_h,
        ground_mark_sprite,
        titbit_row_frame_counts,
        minimap_widget,
    )?;

    let mut bootstrap = MissionBootstrap::new(
        MissionSpec::interactive(mission_idx, location, screen_w, screen_h),
        host,
        game,
        loaded,
    );
    let mut campaign_lease = MissionCampaignLease::new(campaign_ref);

    // ── Spellforge Lua: post-level-load events ──
    //
    // The engine just finished its `.scb` Initialize path inside
    // `load_level_and_sprite_bank`; `.scb` PostInitialize remains armed
    // for the first post-refresh host boundary. If a custom mission
    // shipped a `.lua` companion, fire its loading events now —
    // the Lua side defines its own globals (entity name tables, AI
    // patrol assignments, etc.) on top of whatever the `.scb` did.
    //
    // Vanilla missions take this as a no-op. A required Spellforge session
    // has already been constructed, so a missing host or failed event aborts
    // mission startup instead of continuing with partially initialized state.
    if let Err(error) = bootstrap.start_required_spellforge() {
        return campaign_lease.finish(
            &mut bootstrap.loaded.engine,
            Err(error.to_string()),
            "interactive Spellforge startup failure",
        );
    }

    // ── Mission-specific sound setup (banks + mission music) ──
    // The audio backend and menu music were initialized before the loading
    // screen. Now load mission-specific assets and switch to mission music.
    if let Some(ref mut ls) = loading_screen {
        ls.set_status("Loading mission audio...", 0.75);
    }
    bootstrap.prepare_audio(audio_backend.as_mut(), profiles);
    let mut host = &mut bootstrap.host;
    let game = &mut bootstrap.game;
    let mut engine = &mut bootstrap.loaded.engine;
    let assets = &mut bootstrap.loaded.assets;
    let pre_decoded_bg = bootstrap.loaded.pre_decoded_background.take();
    let pre_decoded_mm = bootstrap.loaded.pre_decoded_minimap.take();
    // ── Post-audio progress + pre-decode ──
    // Runs the slow CPU work *before* closing the loading screen.
    // See `pre_decode_maps_and_resources` for the full breakdown.
    let LoadedInteractiveResources {
        level_descriptors,
        hud_fonts,
    } = pre_decode_maps_and_resources(
        Some(&mut *window),
        &mut loading_screen,
        &mut engine,
        profiles,
        &host,
        &game,
    );

    // Pre-resolve every short-briefing string from the level's text table
    // so the pause-menu render closures can do an immutable lookup.
    //
    // The briefings widget takes a `&dyn Fn(u32) -> Option<String>`
    // for label lookup, but `ResourceManager::get_string` needs `&mut
    // self` (lazy decode + cache).  Materialise the table once here
    // and let the closure do a `HashMap` lookup.  The string index in
    // the resource file is the briefing's id.
    let short_briefing_strings: std::collections::HashMap<u32, String> = level_descriptors
        .as_ref()
        .map(|desc| {
            let table_id = desc.short_briefing.text_table_id;
            match text_res.get_string_count(table_id) {
                Ok(count) => (0..count)
                    .filter_map(|i| {
                        text_res
                            .get_string(table_id, i)
                            .ok()
                            .map(|s| (i as u32, s.to_string()))
                    })
                    .collect(),
                Err(e) => {
                    tracing::warn!(
                        "Short-briefing text table {table_id} unavailable in Level.res: {e}"
                    );
                    std::collections::HashMap::new()
                }
            }
        })
        .unwrap_or_default();

    // Close loading screen — the sand dissolve has fully revealed the
    // final image by now. Must drop before creating the game Renderer so
    // its `&mut Canvas` borrow is released for the game renderer to take.
    if let Some(ls) = loading_screen.take() {
        ls.close();
    }
    drop(loading_screen);

    // ── Create game renderer ──
    let render_w = window.width as u16;
    let render_h = window.height as u16;
    window.set_logical_size(render_w as u32, render_h as u32);
    let mut renderer = Renderer::new(window, render_w, render_h, scale_mode);
    renderer.set_shader_preset(shader_preset);

    // ── Apply pre-decoded background + minimap ──
    // Engine::new already consumed the pre-decoded bg dims for
    // `set_level_size`; now upload pixels to the renderer and install
    // the minimap hit mask on the engine.  Mask composition runs here
    // (inside `apply_background_map`) because `fast_grid.level.masks`
    // is populated only after `Engine::new` has drained the pending
    // motion data.
    if let Some(decoded) = pre_decoded_bg {
        crate::level_loading_host::apply_background_map(&engine, &mut host, &mut renderer, decoded);
    }
    if let Some(mm) = pre_decoded_mm
        .map(|decoded| crate::level_loading_host::apply_minimap(&mut host, &mut renderer, decoded))
    {
        host.engine_display.setup_minimap_map(
            mm.hit_mask,
            mm.map_size,
            mm.saved_position,
            render_w as f32,
            render_h as f32,
        );
    }

    // ── Load cursor, sprites, portraits and peasant names ──
    // Single grouped phase that loads every DEFAULT.RES-backed renderer
    // (cursor, minimap corner/dots, ground focus, selection mark, mouse
    // trail, titbit, portraits) plus the peasant name pool.  See
    // `load_mission_sprites` for the per-resource breakdown.
    let MissionSprites {
        mut cursor_renderer,
        selection_mark_renderer,
        mouse_trail_renderer,
        titbit_renderer,
        portrait_cache,
    } = load_mission_sprites(
        &mut engine,
        &mut host,
        &assets,
        &mut renderer,
        &mut cursor_res,
        &mut text_res,
    );

    // ── HUD fonts ──
    // `hud_fonts` (entity names, HP, action labels) was pre-loaded above,
    // with the loading screen still visible.

    let sample_loader = audio_backend::create_sample_loader(std::path::PathBuf::from(
        &game.global_options.sound_directory,
    ));
    let sound_rng = fastrand::Rng::new();

    // ── Input, camera center, mouse grab ──
    // Builds ThreadedInput + InputTranslator, loads key bindings from
    // the player profile, pushes the DisplayMap accelerator into the
    // engine minimap, centers the camera on the first PC, and grabs
    // the mouse for edge-scrolling.  See `setup_input_and_camera`.
    let (threaded_input, input_translator) = setup_input_and_camera(
        &mut engine,
        &mut host,
        &assets,
        args,
        window.width,
        window.height,
        mission_idx,
    );
    window.grab_mouse(true);

    // ── In-game menu resources ──
    // Loads DEFAULT.RES, menu button sprites and TTF fonts once per mission
    // so the pause menu (and any mid-mission dialogue / debriefing popups)
    // can render without reloading.
    let menu_resources = IngameMenuResources::new(&mut renderer, host.shipping.as_deref());
    if menu_resources.is_none() {
        tracing::error!(
            "In-game menu resources unavailable — pause actions require a successful reload"
        );
    }

    // Restart is disabled when the current mission is Sherwood,
    // since there is no in-mission save to restart from.
    let restart_allowed = location != MissionLocation::Sherwood;
    let mission_ui = MissionUi::new(restart_allowed);

    // ── Lost Leicester gate ──
    // Quit immediately when ARES == 0 on Sherwood entry: the
    // campaign has ended in defeat (last pseudo-mission was LOST and
    // dropped ARES to zero). Before returning, pop a single-page
    // debriefing whose body is the pseudo-mission's lose text
    // (falling back to the generic strategical-mission-lost text
    // when the per-mission entry is missing).
    //
    // The live-game path also runs a pseudo-mission debriefing at
    // ~line 4910 after the campaign-map overlay raises; this pre-loop
    // gate is the defense-in-depth arm for save files loaded with ARES
    // already zero (stale continue-save after a lost campaign).
    if game.is_sherwood
        && engine
            .campaign()
            .map(|c| c.get_ares() == 0)
            .unwrap_or(false)
    {
        // Expect `last_pseudo_mission_status == Lost` alongside
        // `ARES == 0`.  Warn if the invariant fails rather than
        // panic — a save file could plausibly have a reset
        // pseudo-mission status but still zero ARES.
        let (last_id, last_status) = {
            let campaign = engine
                .campaign()
                .expect("campaign present for Sherwood gate");
            (
                campaign.last_pseudo_mission_id,
                campaign.last_pseudo_mission_status,
            )
        };
        if last_status != engine_mission::MissionStatus::Lost {
            tracing::warn!(
                ?last_status,
                "Lost-Leicester gate: ARES=0 but last pseudo-mission status != Lost"
            );
        }

        // Resolve the per-mission loose text from the pseudo-mission's
        // .red descriptor.
        let pseudo_red = {
            let filename = assets_res_descr::red_filename(last_id);
            host.shipping
                .as_deref()
                .and_then(|dd| dd.red_files.get(&filename).cloned())
                .or_else(|| {
                    let path = format!("Data/Text/{filename}");
                    assets_res_descr::load(&path)
                        .map_err(|e| {
                            tracing::warn!(
                                "Lost-Leicester: failed to load pseudo-mission .red {path}: {e}"
                            );
                            e
                        })
                        .ok()
                })
        };
        let per_mission_text = pseudo_red.as_ref().and_then(|desc| {
            let table_id = desc.debriefing.lose_text_table_id;
            if !text_res.has_text_resource(table_id) {
                return None;
            }
            match text_res.get_string(table_id, 0) {
                Ok(s) => Some(s.to_string()),
                Err(e) => {
                    tracing::warn!(
                        "Lost-Leicester: lose_text_table_id {table_id} sub 0 not found: {e}"
                    );
                    None
                }
            }
        });

        if let Some(resources) = menu_resources.as_ref() {
            let text = per_mission_text
                .unwrap_or_else(|| resources.menu_text.get(MT_MSG_STRATEGICAL_MISSION_LOST));
            // Single-button Lost panel — no restart, no load, no
            // stat follow-up.
            let cursor = Some(default_modal_cursor(
                &mut cursor_renderer,
                &mut cursor_res,
                &mut renderer,
            ));
            let _ = crate::ingame_menu::show_debriefing(
                &mut *window,
                &mut renderer,
                resources,
                cursor,
                &text,
                None,
                0,
                false,
                false,
                None,
                false,
                false,
            )
            .await;
        } else {
            tracing::warn!(
                "Lost-Leicester: menu resources unavailable — skipping debriefing popup"
            );
        }

        tracing::info!("Sherwood entry with ARES=0 (lost campaign) — returning to main menu");
        return campaign_lease.finish(
            &mut engine,
            Ok(GameCode::Quit),
            "lost-campaign Sherwood exit",
        );
    }

    // Reset the campaign's `MissionLength` accumulator and start the
    // play-time clock so the debriefing clock measures only the
    // current mission segment.
    engine.campaign_reset_mission_length();
    <RustCallbacks as crate::game::GameCallbacks>::start_play_time(callbacks);

    // ── Restart-point snapshot ──
    //
    // Right after level init completes for any non-Sherwood mission,
    // capture the pristine post-init engine state so a
    // player-triggered restart can snap back without rerunning the
    // expensive level loader.  Skipped in Sherwood.
    //
    // The capture (clone) happens on the main thread; the expensive JSON
    // serialization + disk write is spawned on a background thread so the
    // game loop can start immediately (~9s saved in debug builds).
    if !game.is_sherwood {
        if args.mission_start_map_output.is_none() {
            let campaign = engine
                .campaign()
                .expect("restart snapshot requires the engine campaign");
            let mission_id = current_mission_id(campaign, &assets.profile_manager);
            callbacks.save_manager.write_restart_save_background(
                &mut host,
                &game,
                &engine,
                mission_id,
                Some(&assets.profile_manager),
                None,
            );
        }
    } else {
        // Sherwood opens with the campaign-map overlay already raised
        // so the player can pick the next mission to deploy.  The
        // overlay is a blocking modal driven from the top of the
        // frame loop below via `game.persistent.campaign_map_active`.
        //
        // Skip the auto-raise when the dev `--sherwood` CLI flag was
        // used: we entered Sherwood via a debug shortcut with a
        // freshly-reset campaign that has no enabled map locations yet,
        // so the overlay would close itself immediately with
        // `No missions on campaign map` and exit the mission.
        if !args.sherwood {
            // The displayed flag flips inside the overlay handler
            // when the modal actually opens.
            game.show_campaign_map();
        }
    }

    // Sherwood HUD button state.  Tracks the widget enable mask for
    // DisplayCampaignMap / GoToExit / StartMission / QuitMission.
    // Starts in the pre-commit state (only DisplayCampaignMap live)
    // and flips to post-commit once the player picks a mission via
    // the overlay.
    let sherwood_enable = SherwoodButtonEnable::pre_commit();
    // Button sprites from DEFAULT.RES.  Loaded once per mission;
    // missing sprites just don't render.
    let sherwood_sprites = SherwoodButtonSprites::load(&mut cursor_res, &mut renderer);
    let sherwood_layout =
        SherwoodHudLayout::for_resolution(window.width, window.height, &sherwood_sprites);

    // Zoom HUD buttons (ZoomUp / ZoomDown).  Layout tracks window
    // size just like the Sherwood HUD; enable state is re-derived
    // from engine queries each frame.
    let zoom_sprites = ZoomButtonSprites::load(&mut cursor_res, &mut renderer);
    let zoom_layout = ZoomHudLayout::for_resolution(window.width, window.height, &zoom_sprites);
    let zoom_tooltip = ZoomTooltipTracker::new();

    // Top-of-panel HUD buttons (Clock / Sight / QuickStart).
    // Non-Sherwood missions only.  Sprites load once per mission;
    // the layout is re-derived every frame at the top of the game
    // loop from the renderer's current screen size (cheap rect
    // arithmetic) so nested menus that change resolution don't need
    // to plumb a layout ref.
    let corner_sprites = CornerButtonSprites::load(&mut cursor_res, &mut renderer);
    let corner_tooltip = CornerTooltipTracker::new();
    let stature_sprites = StatureSprites::load(&mut cursor_res, &mut renderer);

    // Hover-idle tracker for the Sherwood requirements-bar tooltip.
    let requirements_tooltip = RequirementsTooltipTracker::new();
    // Same pipeline for the blazon-bar slots.
    let blazon_tooltip = BlazonTooltipTracker::new();
    // Stature arrow (up / down) and Sherwood-HUD tooltip timers.
    let stature_tooltip = StatureTooltipTracker::new();
    let sherwood_tooltip = SherwoodTooltipTracker::new();
    // PC portrait action-button tooltip timer — each of the three
    // per-PC action buttons gets a localized tooltip after 75 idle
    // ticks.
    let pc_action_tooltip = PcActionTooltipTracker::new();
    let last_cursor_id: i32 = crate::resource_ids::RHMOUSE_DEFAULT;

    // Replay/timeline construction remains after SCB Initialize,
    // Spellforge startup, mission audio, and frontend resource loading. The
    // bootstrap consumes all common process state only after the complete
    // interactive frontend below has been assembled.
    let corner_layout = CornerHudLayout::for_resolution(
        renderer.screen_width() as u32,
        renderer.screen_height() as u32,
        &corner_sprites,
    );
    let stature_layout = StatureHudLayout::for_resolution(
        renderer.screen_width() as u32,
        renderer.screen_height() as u32,
        &stature_sprites,
    );
    let frontend = InteractiveFrontend {
        input: MissionInput::new(threaded_input, input_translator),
        audio: MissionAudio::new(audio_backend, sample_loader, sound_rng),
        resources: MissionResources {
            text: text_res,
            cursor: cursor_res,
            level_descriptors,
            hud_fonts,
            short_briefing_strings,
            menu: menu_resources,
        },
        ui: mission_ui,
        hud: MissionHud {
            sherwood_enable,
            sherwood_sprites,
            sherwood_layout,
            zoom_sprites,
            zoom_layout,
            zoom_tooltip,
            corner_sprites,
            corner_layout,
            corner_tooltip,
            stature_sprites,
            stature_layout,
            requirements_tooltip,
            blazon_tooltip,
            stature_tooltip,
            sherwood_tooltip,
            pc_action_tooltip,
            last_cursor_id,
        },
        presentation: MissionPresentation {
            renderer,
            sprites: MissionSprites {
                cursor_renderer,
                selection_mark_renderer,
                mouse_trail_renderer,
                titbit_renderer,
                portrait_cache,
            },
        },
    };

    let mut mission = bootstrap.finish_interactive(frontend, args, profiles);

    let outcome = {
        let mut services = flow::MissionServices {
            window,
            callbacks,
            campaign: campaign_lease.session_campaign(),
            profiles,
            args,
        };
        mission.run(&mut services).await
    };
    // `run` completes the selected exit phase (save/load flush, transition
    // effects, recorder/modal bookkeeping) while the Engine still owns the
    // campaign, matching the original GameLoop-before-session boundary.
    campaign_lease.finish(
        &mut mission.runtime.world.manager.engine,
        outcome,
        "interactive mission finalization",
    )
}

#[cfg(test)]
mod required_state_tests {
    use super::{MissionCampaignLease, required_menu_resources, restore_campaign_value};
    use crate::campaign::{Campaign, CampaignValue};
    use crate::game_operation::GameCode;
    use crate::ingame_menu::IngameMenuResources;

    #[test]
    fn mission_exit_restores_the_exact_campaign_allocation() {
        let mut outer_campaign = Campaign::default();
        let mut engine_campaign = Campaign::default();
        engine_campaign.values[CampaignValue::Custom20] = 0x25_25_25;
        let production_sectors = engine_campaign.production_sectors.as_ptr();

        restore_campaign_value(&mut outer_campaign, engine_campaign);

        assert_eq!(outer_campaign.values[CampaignValue::Custom20], 0x25_25_25);
        assert_eq!(
            outer_campaign.production_sectors.as_ptr(),
            production_sectors
        );
    }

    #[test]
    fn campaign_lease_finalizes_every_controlled_exit_outcome() {
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
            let mut session_campaign = Campaign::default();
            let mut engine_campaign = Campaign::default();
            let marker = index as i32 + 1;
            engine_campaign.values[CampaignValue::Custom20] = marker;
            let production_sectors = engine_campaign.production_sectors.as_ptr();

            let actual = MissionCampaignLease::new(&mut session_campaign).finish_campaign(
                Some(engine_campaign),
                outcome.clone(),
                path,
            );

            assert_eq!(actual, outcome, "{path}");
            assert_eq!(
                session_campaign.values[CampaignValue::Custom20],
                marker,
                "{path}"
            );
            assert_eq!(
                session_campaign.production_sectors.as_ptr(),
                production_sectors,
                "{path}"
            );
        }
    }

    #[test]
    #[should_panic(expected = "test campaign lease: engine campaign is missing")]
    fn campaign_lease_rejects_missing_engine_campaign() {
        let mut campaign = Campaign::default();
        MissionCampaignLease::new(&mut campaign).finish_campaign(None, (), "test campaign lease");
    }

    #[test]
    #[should_panic(expected = "test confirmation: in-game menu resources are missing")]
    fn confirmation_rejects_missing_menu_resources() {
        let resources: Option<IngameMenuResources> = None;
        required_menu_resources(&resources, "test confirmation");
    }
}
