//! Game session: mission selection loop and the per-mission game loop.

mod dispatch;
mod input_handlers;
mod modal_state;
mod mouse_input;
mod multiplayer;
mod render;
mod replay_init;
mod setup;
mod tick;

use dispatch::apply_local_viewport_scroll;
pub(crate) use dispatch::{dispatch_local_command, dispatch_local_commands};
use input_handlers::{handle_console_overlay_events, handle_gamepad_events, handle_hold_to_rewind};
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
    MultiplayerRollbackTelemetry, accept_host_frame_schedule, drain_net_inputs,
    host_scheduled_frame_deadline_ms, setup_multiplayer_session,
};
pub use render::RenderContext;
use render::{
    capture_save_thumbnail, drain_print_screen_request, drain_screenshots, drain_wide_print_screen,
    print_screen_request_from_modifiers, render_frame, update_mouse_and_cursor,
};
use replay_init::{ReplayAndRollback, init_replay_and_rollback};
use setup::{
    MissionSprites, extract_ground_mark_sprite_data, extract_minimap_widget_setup,
    extract_titbit_row_frame_counts, init_audio_backend, load_level_and_sprite_bank,
    load_mission_sprites, pre_decode_maps_and_resources, setup_input_and_camera,
    setup_mission_audio,
};
use tick::{
    dismiss_pending_modals, drain_steps, modal_state_pending, post_render_engine_cleanup,
    pre_render_engine_setup, tick_audio,
};

use crate::Host;
use crate::campaign::Campaign;
use crate::game::{Game, GameCallbacks};
use crate::game_operation::GameCode;
use crate::geo2d;
use crate::gfx_types::GameEvent;
use crate::host::PrintScreenRequest;
use crate::ingame_menu::widget_bridge::default_modal_cursor;
use crate::ingame_menu::{IngameMenuResources, PauseMenu};
use crate::main_entry::{
    RustCallbacks, current_mission_id, detect_demo_mode, flush_pending_callbacks,
    perform_pending_save_load, resolve_loading_pak,
};
use crate::player_command::{FrameCommands, PlayerCommand, PlayerInput};
use crate::profiles::MissionLocation;
use crate::sdl_audio::{self};
use crate::window::GameWindow;
use robin_engine::engine::Engine;
use robin_engine::graphic_config::TextureScaleMode;
use std::sync::Arc;

/// Read the active player profile's texture scale mode, falling back to
/// the default (`Linear`) if no profile is loaded yet.
fn active_profile_scale_mode() -> TextureScaleMode {
    let guard = crate::player_profile::PlayerProfileManager::global();
    guard
        .as_ref()
        .and_then(|m| m.get_active())
        .map(|p| p.graphic_config.scale_mode)
        .unwrap_or_default()
}

fn active_profile_shader_preset() -> String {
    let guard = crate::player_profile::PlayerProfileManager::global();
    guard
        .as_ref()
        .and_then(|m| m.get_active())
        .map(|p| p.graphic_config.shader_preset.clone())
        .unwrap_or_default()
}

fn center_on_reselected_portrait_pc(
    host: &mut Host,
    engine: &Engine,
    local_seat: robin_engine::player_command::PlayerId,
    pc_id: robin_engine::element::EntityId,
    append: bool,
    area: crate::ui_panel::PortraitHitArea,
) -> bool {
    if append
        || !matches!(
            area,
            crate::ui_panel::PortraitHitArea::TopScroll
                | crate::ui_panel::PortraitHitArea::BottomScroll
                | crate::ui_panel::PortraitHitArea::Visage
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
        .center_on_point(entity.position_iface().map_position().to_geo());
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

pub(super) fn selected_pc_profile_indices(
    engine: &robin_engine::engine::Engine,
    seat: robin_engine::player_command::PlayerId,
) -> Vec<robin_engine::profiles::CharacterProfileIdx> {
    engine
        .seat_selection(seat)
        .iter()
        .filter_map(|&id| match engine.get_entity(id)? {
            robin_engine::element::Entity::Pc(pc) => Some(pc.pc.profile_index),
            _ => None,
        })
        .collect()
}

pub(crate) async fn run_mission_headless(
    callbacks: &mut RustCallbacks,
    campaign_ref: &mut Campaign,
    profiles: &robin_engine::profiles::ProfileManager,
    mission_idx: usize,
    location: MissionLocation,
    args: &crate::main_entry::CliArgs,
) -> Result<GameCode, String> {
    let mut host = Host::new(1024.0, 768.0);
    if let Err(e) = setup_multiplayer_session(&mut host, args) {
        tracing::error!("{e}; aborting headless mission");
        return Ok(GameCode::Quit);
    }

    let mut game = Game::new(location);
    game.global_options = args.global_options.clone();

    let mut text_res = crate::resource_manager::ResourceManager::new();
    if let Err(e) =
        text_res.attach_or_from_shipping("Data/Text/Level.res", host.shipping.as_deref())
    {
        tracing::warn!("Failed to load text resource file: {e}");
    }

    let mut cursor_res = crate::resource_manager::ResourceManager::new();
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

    let (mut engine, mut assets, mut dev, _pre_decoded_bg, _pre_decoded_mm, engine_rng_seed) =
        load_level_and_sprite_bank(
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

    setup_mission_audio(
        &mut host,
        None,
        &engine,
        &mut assets,
        profiles,
        location,
        &game.global_options.sound_directory,
    );
    let (_unused_bg_slot, _unused_mm_slot, _level_descriptors, _hud_fonts) =
        pre_decode_maps_and_resources(None, &mut None, &mut engine, profiles, &host, &game);

    engine.campaign_reset_mission_length();
    <RustCallbacks as crate::game::GameCallbacks>::start_play_time(callbacks);

    let mission_id_for_recorder = engine
        .campaign()
        .and_then(|c| {
            c.missions
                .get(mission_idx)
                .map(|m| m.profile(profiles).mission_filename.clone())
        })
        .unwrap_or_else(|| format!("mission_{mission_idx}"));
    let assets = Arc::new(assets);
    let ReplayAndRollback {
        recorder: mut replay_recorder,
        player: mut replay_player,
        mut rollback_checker,
        mut rewind_buffer,
        start_paused,
    } = init_replay_and_rollback(
        &mut engine,
        Arc::clone(&assets),
        args,
        mission_idx,
        &mission_id_for_recorder,
        engine_rng_seed,
        host.net.is_some(),
    );
    let mut manager = robin_engine::engine_manager::EngineManager::new(engine, host.local_seat);
    let mut manual_pause = start_paused;

    loop {
        let recorder_hash_this_frame = replay_recorder.as_ref().and_then(|r| {
            let f = r.frame_number();
            (f % 25 == 0).then(|| crate::replay::state_hash(&manager.engine))
        });
        rewind_buffer.begin_frame(manager.sim_frame, &manager.engine, &assets);
        if let Some(checker) = rollback_checker.as_mut() {
            checker.begin_frame(manager.sim_frame, &manager.engine);
        }

        let mut frame_cmds = Vec::new();
        let mut frame_modal_dismissals = Vec::new();
        let mut replay_modal_dismissals: std::collections::VecDeque<PlayerCommand> =
            std::collections::VecDeque::new();
        if let Some(player) = replay_player.as_mut()
            && !player.is_finished()
        {
            for cmd in player.next_frame() {
                if matches!(cmd.command, PlayerCommand::ModalDismiss { .. }) {
                    replay_modal_dismissals.push_back(cmd.command.clone());
                    continue;
                }
                frame_cmds.push(cmd.clone());
            }
            manager.engine.apply_commands(
                &mut host.engine_display,
                &mut host.input,
                &assets,
                &frame_cmds,
            );
        }

        let _ = dismiss_pending_modals(&mut host);
        let tick_exit_code = if manual_pause {
            None
        } else {
            let mut display = std::mem::take(&mut host.engine_display);
            let result = game.run_engine_tick(
                &mut host,
                &mut display,
                assets.as_ref(),
                &mut manager.engine,
                &mut dev,
                false,
                false,
            );
            host.engine_display = display;
            result
        };

        let net = host.net.take();
        crate::http_server::drain_global(&mut manager, &mut host, &assets, net.as_ref());
        host.net = net;
        if host.pending_mission_state_popup {
            host.pending_mission_state_popup = false;
            let kind = robin_engine::player_command::ModalKind::MissionState {
                kind: robin_engine::player_command::MissionStateModalKind::LeaveMissionNow,
            };
            let result = pop_matching_dismissal(&mut replay_modal_dismissals, &kind)
                .unwrap_or(robin_engine::player_command::DialogResult::Completed);
            frame_modal_dismissals.push(PlayerCommand::ModalDismiss { kind, result });
            if result == robin_engine::player_command::DialogResult::Completed {
                let cmd = PlayerCommand::QuitMissionRequested;
                if let Some(net) = host.net.as_ref() {
                    net.send_input(cmd.clone());
                } else {
                    manager.engine.apply_local_commands(
                        &mut host.engine_display,
                        &mut host.input,
                        &assets,
                        std::slice::from_ref(&cmd),
                    );
                }
                frame_cmds.push(PlayerInput::new(host.local_seat, cmd));
            }
        }
        let mut headless_active_modal: Option<ActiveModal> = None;
        drain_steps(
            &mut manager,
            &mut host,
            &assets,
            &mut dev,
            &mut game,
            &mut rewind_buffer,
            &mut rollback_checker,
            &mut replay_player,
            &mut manual_pause,
            &mut headless_active_modal,
        );
        let dismissed = dismiss_pending_modals(&mut host);
        if dismissed > 0 {
            tracing::debug!(dismissed, "headless: auto-dismissed pending modal(s)");
        }
        // Headless stepping has no UI to interact with. Recorded
        // dismissals can be left over when the headless auto-dismiss
        // path has already closed the modal; keep that visible at
        // debug level without making every clean headless replay look
        // like a simulation warning.
        if !replay_modal_dismissals.is_empty() {
            tracing::debug!(
                "Replay headless: {} recorded ModalDismiss command(s) unused this frame",
                replay_modal_dismissals.len()
            );
        }

        if !manual_pause {
            if let Some(checker) = rollback_checker.as_mut() {
                checker.end_frame(&mut host, frame_cmds.clone(), &manager.engine);
            }
            rewind_buffer.end_frame(frame_cmds.clone());
            if let Some(recorder) = replay_recorder.as_mut() {
                if let Some(hash) = recorder_hash_this_frame {
                    recorder.write_hash(recorder.frame_number(), hash);
                }
                for cmd in &frame_cmds {
                    recorder.push(cmd.clone());
                }
                for cmd in &frame_modal_dismissals {
                    recorder.push(cmd.clone());
                }
                recorder.end_frame();
            }
            manager.sim_frame += 1;
        }

        if let Some(code) = tick_exit_code {
            *campaign_ref = manager.engine.take_campaign().unwrap_or_default();
            return Ok(code);
        }
        if replay_player.as_ref().is_some_and(|p| p.is_finished()) {
            tracing::info!("headless replay finished");
            *campaign_ref = manager.engine.take_campaign().unwrap_or_default();
            return Ok(GameCode::Quit);
        }

        if manual_pause {
            crate::window::sleep_ms(10).await;
        } else {
            crate::window::yield_to_runtime().await;
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
    profiles: &robin_engine::profiles::ProfileManager,
    args: &crate::main_entry::CliArgs,
    initial_load: Option<crate::main_entry::SaveLoadRequest>,
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
                        callbacks.pending = Some(crate::main_entry::SaveLoadRequest::Load {
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
) -> robin_engine::player_command::DialogResult {
    match outcome {
        SettledDebriefingOutcome::Ok => robin_engine::player_command::DialogResult::Completed,
        SettledDebriefingOutcome::Restart => robin_engine::player_command::DialogResult::Restart,
        SettledDebriefingOutcome::Load { slot } => {
            robin_engine::player_command::DialogResult::Load { slot: *slot as u32 }
        }
        SettledDebriefingOutcome::EmergencyEnd => {
            robin_engine::player_command::DialogResult::Aborted
        }
    }
}

fn final_debriefing_outcome_from_replay(
    result: robin_engine::player_command::DialogResult,
) -> SettledDebriefingOutcome {
    match result {
        robin_engine::player_command::DialogResult::Completed => SettledDebriefingOutcome::Ok,
        robin_engine::player_command::DialogResult::Aborted => {
            SettledDebriefingOutcome::EmergencyEnd
        }
        robin_engine::player_command::DialogResult::Restart => SettledDebriefingOutcome::Restart,
        robin_engine::player_command::DialogResult::Load { slot } => {
            SettledDebriefingOutcome::Load {
                slot: slot as usize,
            }
        }
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
    profiles: &robin_engine::profiles::ProfileManager,
    _host: &Host,
    event_pump: &mut GameWindow,
    renderer: &mut crate::renderer::Renderer,
    cursor_res: &mut crate::resource_manager::ResourceManager,
    cursor_renderer: &mut crate::cursor::CursorRenderer,
    menu_resources: &Option<IngameMenuResources>,
) {
    let use_backup = match callbacks.pending {
        Some(crate::main_entry::SaveLoadRequest::QuickLoad { use_backup }) => use_backup,
        _ => return,
    };
    let slot_name = if use_backup {
        crate::save_file::special_slots::EX_QUICK
    } else {
        crate::save_file::special_slots::QUICK
    };
    let Some(idx) = callbacks.save_manager.find_by_filename(slot_name) else {
        return;
    };
    if !callbacks.save_manager.slot_file_exists(idx) {
        return;
    }
    let Some(target_mission_id) = callbacks.save_manager.slot_mission_id(idx) else {
        tracing::warn!(
            "QuickLoad cross-mission confirm: slot {idx} has no cached mission id — \
             falling through without prompt"
        );
        return;
    };
    let current = engine
        .campaign()
        .map(|c| current_mission_id(c, profiles))
        .unwrap_or(0);
    if current == 0 || target_mission_id == current {
        return;
    }
    let Some(resources) = menu_resources.as_ref() else {
        tracing::warn!(
            "QuickLoad cross-mission confirm: menu resources unavailable — falling through \
             without prompt"
        );
        return;
    };
    let msg = resources
        .menu_text
        .get(crate::ingame_menu::resources::MT_MSG_REALLY_LOAD_QUICKSAVE);
    let cursor = Some(default_modal_cursor(cursor_renderer, cursor_res, renderer));
    let confirmed =
        crate::ingame_menu::show_yesno(event_pump, renderer, resources, cursor, &msg).await;
    if confirmed {
        // Route through the regular `Load` arm so its existing
        // `PendingLevelLoad` cross-mission plumbing handles the mission
        // swap + Load re-queue on the fresh engine.  Pass the running
        // mission id so the arm's `header.mission_id != mission_id`
        // check fires.
        callbacks.pending = Some(crate::main_entry::SaveLoadRequest::Load {
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
    profiles: &robin_engine::profiles::ProfileManager,
    mission_idx: usize,
    location: MissionLocation,
    args: &crate::main_entry::CliArgs,
) -> Result<GameCode, String> {
    // ── Loading screen ──
    // Show a sand-dissolve loading screen while initializing the mission.
    // Uses its own Renderer at the .pak image resolution; SDL logical size
    // scales it to fill the window. Dropped before the game Renderer is created.
    //
    // The TextureCreator must outlive the loading-screen Renderer; we hold it
    // in a local that survives until the loading screen is dropped further down.

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
                Some(MissionLocation::Leicester) => {
                    crate::loading_screen::LoadingDatadirKind::DemoI
                }
                Some(MissionLocation::Lincoln) => crate::loading_screen::LoadingDatadirKind::DemoII,
                _ => crate::loading_screen::LoadingDatadirKind::FullGame,
            };
            crate::loading_screen::LoadingScreenRenderer::new(
                window,
                &path,
                datadir_kind,
                LOADING_MAX_LEVEL,
                scale_mode,
            )
        })
    };
    if let Some(ref mut ls) = loading_screen {
        ls.set_status("Initializing audio...", 0.02);
        ls.refresh(); // show initial state (0%)
        ls.drain_events(&mut *window);
    }

    let mut host = Host::new(window.width as f32, window.height as f32);
    // Build a Lua session if the main-menu picker stashed a
    // pending Spellforge launch on the CLI args. Failure here is
    // non-fatal — the engine's `.scb` script path still runs, the
    // player just loses the custom Lua hooks. We surface the
    // reason loudly so it shows up in the loading-screen log.
    if let Some(pending) = args.pending_lua_mission.as_ref() {
        let launch = crate::main_menu::custom_missions::CustomMissionLaunch {
            slug: pending.slug.clone(),
            mod_title: pending.slug.clone(),
            version_zip: pending.version_zip.clone(),
            rhm_basename: pending.rhm_basename.clone(),
            // Both these fields exist for the picker UI and the
            // mission loader; the Lua session only reads the
            // version_zip + basename, but the constructor wants
            // the whole struct.
            map_filename: String::new(),
            requires_spellforge: true,
        };
        match crate::lua_session::LuaSession::start(&launch, &pending.mods_root) {
            Ok(Some(session)) => {
                tracing::info!(
                    "LuaSession installed for mission '{}'",
                    session.mission_basename()
                );
                host.lua_session = Some(session);
            }
            Ok(None) => {
                tracing::debug!(
                    "LuaSession::start returned None for '{}' (Vanilla)",
                    pending.slug
                );
            }
            Err(e) => {
                tracing::warn!(
                    "LuaSession::start failed for '{}': {e} — continuing without Lua",
                    pending.slug
                );
            }
        }
    }
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
    let mut text_res = crate::resource_manager::ResourceManager::new();
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
    let mut cursor_res = crate::resource_manager::ResourceManager::new();
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
    let (mut engine, mut assets, mut dev, pre_decoded_bg, pre_decoded_mm, engine_rng_seed) =
        load_level_and_sprite_bank(
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

    // ── Spellforge Lua: post-level-load events ──
    //
    // The engine just finished its `.scb` Initialize / PostInitialize
    // path inside `load_level_and_sprite_bank`. If a custom mission
    // shipped a `.lua` companion, fire the matching Lua events now —
    // the Lua side defines its own globals (entity name tables, AI
    // patrol assignments, etc.) on top of whatever the `.scb` did.
    //
    // Vanilla missions (and any Spellforge launch where the Lua
    // session failed to build) take this branch as a no-op since
    // `host.lua_session` is `None`.
    if let Some(lua) = host.lua_session.as_ref()
        && let Some(game_host) = engine.mission_script_game_host_mut()
    {
        tracing::info!(
            "Lua: firing Initialize for mission '{}' (seed={engine_rng_seed})",
            lua.mission_basename()
        );
        let _ = lua.run_event(game_host, "Initialize", &[engine_rng_seed as i32]);
        let _ = lua.run_event(game_host, "PostInitialize", &[]);
    }

    // ── Mission-specific sound setup (banks + mission music) ──
    // The audio backend and menu music were initialized before the loading
    // screen. Now load mission-specific assets and switch to mission music.
    if let Some(ref mut ls) = loading_screen {
        ls.set_status("Loading mission audio...", 0.75);
    }
    setup_mission_audio(
        &mut host,
        audio_backend.as_mut(),
        &engine,
        &mut assets,
        profiles,
        location,
        &game.global_options.sound_directory,
    );
    let assets = Arc::new(assets);
    // ── Post-audio progress + pre-decode ──
    // Runs the slow CPU work *before* closing the loading screen.
    // See `pre_decode_maps_and_resources` for the full breakdown.
    let (_unused_bg_slot, _unused_mm_slot, level_descriptors, hud_fonts) =
        pre_decode_maps_and_resources(
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
    let mut renderer = crate::renderer::Renderer::new(window, render_w, render_h, scale_mode);
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
        mut selection_mark_renderer,
        mouse_trail_renderer,
        mut titbit_renderer,
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

    let sample_loader = sdl_audio::create_sample_loader(std::path::PathBuf::from(
        &game.global_options.sound_directory,
    ));
    let mut sound_rng = fastrand::Rng::new();

    // ── Input, camera center, mouse grab ──
    // Builds ThreadedInput + InputTranslator, loads key bindings from
    // the player profile, pushes the DisplayMap accelerator into the
    // engine minimap, centers the camera on the first PC, and grabs
    // the mouse for edge-scrolling.  See `setup_input_and_camera`.
    let (mut threaded_input, mut input_translator) = setup_input_and_camera(
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
    let mut menu_resources = IngameMenuResources::new(&mut renderer, host.shipping.as_deref());
    if menu_resources.is_none() {
        tracing::warn!("In-game menu resources unavailable — pause menu will use fallback rects");
    }

    // Restart is disabled when the current mission is Sherwood,
    // since there is no in-mission save to restart from.
    let restart_allowed = location != MissionLocation::Sherwood;

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
        if last_status != robin_engine::mission::MissionStatus::Lost {
            tracing::warn!(
                ?last_status,
                "Lost-Leicester gate: ARES=0 but last pseudo-mission status != Lost"
            );
        }

        // Resolve the per-mission loose text from the pseudo-mission's
        // .red descriptor.
        let pseudo_red = {
            let filename = robin_assets::res_descr::red_filename(last_id);
            host.shipping
                .as_deref()
                .and_then(|dd| dd.red_files.get(&filename).cloned())
                .or_else(|| {
                    let path = format!("Data/Text/{filename}");
                    robin_assets::res_descr::load(&path)
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
            let text = per_mission_text.unwrap_or_else(|| {
                resources
                    .menu_text
                    .get(crate::ingame_menu::resources::MT_MSG_STRATEGICAL_MISSION_LOST)
            });
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
        *campaign_ref = engine.take_campaign().unwrap_or_default();
        return Ok(GameCode::Quit);
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
        let mission_id = engine
            .campaign()
            .map(|c| current_mission_id(c, &assets.profile_manager))
            .unwrap_or(0);
        callbacks.save_manager.write_restart_save_background(
            &mut host,
            &game,
            &engine,
            mission_id,
            Some(&assets.profile_manager),
            None,
        );
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

    // State for the Sherwood campaign-map overlay.  Built lazily the
    // first time the overlay is raised so the CampaignMapState reflects
    // the live campaign at that instant.
    let mut sherwood_campaign_map = crate::menu::CampaignMapState::new();

    // Sherwood HUD button state.  Tracks the widget enable mask for
    // DisplayCampaignMap / GoToExit / StartMission / QuitMission.
    // Starts in the pre-commit state (only DisplayCampaignMap live)
    // and flips to post-commit once the player picks a mission via
    // the overlay.
    let mut sherwood_enable = crate::sherwood_hud::SherwoodButtonEnable::pre_commit();
    // Button sprites from DEFAULT.RES.  Loaded once per mission;
    // missing sprites just don't render.
    let sherwood_sprites =
        crate::sherwood_hud::SherwoodButtonSprites::load(&mut cursor_res, &mut renderer);
    let mut sherwood_layout = crate::sherwood_hud::SherwoodHudLayout::for_resolution(
        window.width,
        window.height,
        &sherwood_sprites,
    );

    // Zoom HUD buttons (ZoomUp / ZoomDown).  Layout tracks window
    // size just like the Sherwood HUD; enable state is re-derived
    // from engine queries each frame.
    let zoom_sprites = crate::zoom_hud::ZoomButtonSprites::load(&mut cursor_res, &mut renderer);
    let mut zoom_layout =
        crate::zoom_hud::ZoomHudLayout::for_resolution(window.width, window.height, &zoom_sprites);
    let mut zoom_tooltip = crate::zoom_hud::ZoomTooltipTracker::new();

    // Top-of-panel HUD buttons (Clock / Sight / QuickStart).
    // Non-Sherwood missions only.  Sprites load once per mission;
    // the layout is re-derived every frame at the top of the game
    // loop from the renderer's current screen size (cheap rect
    // arithmetic) so nested menus that change resolution don't need
    // to plumb a layout ref.
    let corner_sprites =
        crate::corner_hud::CornerButtonSprites::load(&mut cursor_res, &mut renderer);
    // Initial layout placeholder; re-assigned at the top of every frame
    // before use, so any value here is overwritten before it's read.
    let mut corner_layout;
    let mut corner_tooltip = crate::corner_hud::CornerTooltipTracker::new();
    let stature_sprites = crate::stature_hud::StatureSprites::load(&mut cursor_res, &mut renderer);
    let mut stature_layout;

    // Pause menu state
    let mut pause_menu: Option<PauseMenu> = None;
    let mut active_modal: Option<ActiveModal> = None;
    // In-game cheat / debug console overlay (toggled via `~`).  Lives
    // for the whole mission so command history persists across opens.
    let mut console_overlay = crate::console_overlay::ConsoleOverlay::new();
    // Hover-idle tracker for the Sherwood requirements-bar tooltip.
    let mut requirements_tooltip = crate::ui_panel::RequirementsTooltipTracker::new();
    // Same pipeline for the blazon-bar slots.
    let mut blazon_tooltip = crate::ui_panel::BlazonTooltipTracker::new();
    // Stature arrow (up / down) and Sherwood-HUD tooltip timers.
    let mut stature_tooltip = crate::stature_hud::StatureTooltipTracker::new();
    let mut sherwood_tooltip = crate::sherwood_hud::SherwoodTooltipTracker::new();
    // PC portrait action-button tooltip timer — each of the three
    // per-PC action buttons gets a localized tooltip after 75 idle
    // ticks.
    let mut pc_action_tooltip = crate::ui_panel::PcActionTooltipTracker::new();
    let mut last_cursor_id: i32 = crate::resource_ids::RHMOUSE_DEFAULT;

    // ── Replay recording / playback + rollback + rewind ──
    // Record-by-default; `--record <path>` overrides the destination,
    // `--replay <path>` disables recording in favour of playback.  See
    // `init_replay_and_rollback` for the full breakdown.
    let ReplayAndRollback {
        recorder: mut replay_recorder,
        player: mut replay_player,
        mut rollback_checker,
        mut rewind_buffer,
        start_paused,
    } = {
        // `campaign_ref` was emptied by `std::mem::take` during level
        // init (see the top of this function) — the real campaign
        // now lives inside the engine.  Pull the `mission_filename`
        // out here so the replay recorder can stamp it into the
        // header; mutable-borrowing `engine` for
        // `init_replay_and_rollback` below would conflict with an
        // immutable `engine.campaign()` borrow.
        let mission_id_for_recorder = engine
            .campaign()
            .and_then(|c| {
                c.missions
                    .get(mission_idx)
                    .map(|m| m.profile(profiles).mission_filename.clone())
            })
            .unwrap_or_else(|| format!("mission_{mission_idx}"));
        init_replay_and_rollback(
            &mut engine,
            Arc::clone(&assets),
            args,
            mission_idx,
            &mission_id_for_recorder,
            engine_rng_seed,
            host.net.is_some(),
        )
    };

    // Bundle the per-frame engine + rollback state into one
    // `EngineManager`.  After this point, `engine` no longer exists
    // as a separate binding — use `manager.engine`, `manager.sim_frame`,
    // and `manager.pending_inputs` (or the methods on `manager`) for
    // the rest of `run_mission`.
    let mut manager = robin_engine::engine_manager::EngineManager::new(engine, host.local_seat);
    // Multiplayer: peer state hashes received from the host (only the
    // server broadcasts).  Each entry is `(frame → host_hash)`; the
    // client compares its locally-computed hash at the same sampling
    // point.  Drained as frames are reached.
    let mut peer_hashes: std::collections::BTreeMap<u32, u64> = std::collections::BTreeMap::new();
    let mut recent_timeline_history = crate::sim_timeline::RecentTimelineHistory::new(
        crate::sim_timeline::RECENT_TIMELINE_HISTORY_FRAMES,
    );

    // Manual pause toggle, distinct from the pause menu.  Set on mission
    // entry by `--start-paused` or by a `load-replay` RPC call that
    // requested `paused: true`; persists across `/step-forward` calls
    // (step requests bypass the pause gate to run their own ticks
    // synchronously, but they don't clear this flag — the sim stays
    // paused between step calls).  Kept separate from `pause_menu` so
    // the HUD / cursor / input stay fully interactive while the sim is
    // frozen.  Also toggled by the in-game step-debug keys: `.` and `,`
    // enable it (and step one frame forward/back), Enter clears it.
    let mut mp_start_gate = None;
    let mut mp_waiting_for_initial_snapshot =
        host.net.is_some() && host.local_seat != robin_engine::player_command::PlayerId::HOST;
    let mut mp_waiting_for_begin_sim = host.net.is_some();
    let mut mp_host_frame_schedule: Option<(u32, u32)> = None;
    let mut last_mp_rollback: Option<MultiplayerRollbackTelemetry> = None;
    let mut last_mp_clock_ahead_log_ms = 0;
    let mut last_mp_sleep_correction_log_ms = 0;
    let mut last_mp_state_hash_frame: Option<u32> = None;
    let mut manual_pause =
        start_paused || mp_waiting_for_initial_snapshot || mp_waiting_for_begin_sim;
    let mut replay_finished_logged = false;
    let mut step_forward_repeat_at_ms: Option<u32> = None;
    let mut step_back_repeat_at_ms: Option<u32> = None;

    // Track the ambience-derived shadow key so host-side sprite caches
    // (selection marks + titbits) can be rebound when
    // `weather.ambiance` changes mid-mission. The shadow key is
    // baked into the frame dictionaries at load time and never
    // re-run; this poll closes that gap by reloading the
    // shadow-dependent sprite caches whenever the engine state's
    // `night_color` deviates from the last rebind. Cheap (two u16
    // comparisons per frame) and dormant until something actually
    // mutates the ambience.
    let mut last_shadow_color = manager.engine.weather().night_color;

    loop {
        let frame_start = crate::window::process_uptime_ms();
        let mut frame_cmds = FrameCommands::new();
        let mut modal_rendered_this_frame = false;
        if let Some(start_at) = mp_start_gate {
            if crate::multiplayer::lobby::current_epoch_ms() >= start_at {
                mp_start_gate = None;
                if !start_paused {
                    manual_pause = false;
                }
                tracing::info!("multiplayer: synchronized lobby start gate opened");
            } else {
                manual_pause = true;
            }
        }

        // ── Multiplayer: drain incoming wire events ───────────────
        // - Future inputs queue in `pending_inputs[target_frame]`.
        // - Late inputs (target < sim_frame) splice into the rewind
        //   buffer and trigger a rollback to reconstruct the engine
        //   state with the late input woven in.  `drain_net_inputs`
        //   replaces the live rollback state when that fires.
        // - Inputs scheduled for `sim_frame` come back in the return
        //   value; we apply them and append to `frame_cmds`.
        // - Authoritative state hashes from the host land in
        //   `peer_hashes`, drained below alongside the per-25-frame
        //   sampling tick.
        // Publishes the current sim_frame to the server's broadcast
        // pump so peer-input target frames are stamped against a
        // fresh cursor.
        if let Some(net) = host.net.as_ref() {
            net.publish_frame(manager.sim_frame);
        }
        let net_drain = drain_net_inputs(
            &mut host,
            &mut manager,
            assets.as_ref(),
            &mut rewind_buffer,
            &mut peer_hashes,
            &mut recent_timeline_history,
        );
        if net_drain.rewrote_sim_state
            && let Some(ref mut checker) = rollback_checker
        {
            checker.reset();
        }
        if let Some(rollback) = net_drain.rollback.clone() {
            last_mp_rollback = Some(rollback);
        }
        if let Some((_frame, start_epoch_ms)) = net_drain.begin_sim {
            mp_waiting_for_begin_sim = false;
            mp_start_gate = Some(start_epoch_ms);
            manual_pause = true;
        }
        if mp_waiting_for_initial_snapshot && net_drain.received_initial_snapshot {
            mp_waiting_for_initial_snapshot = false;
            tracing::info!(
                "multiplayer: initial snapshot received; client ready for start barrier"
            );
        }
        if mp_waiting_for_initial_snapshot || mp_waiting_for_begin_sim {
            manual_pause = true;
        }
        if host.net.is_some()
            && host.local_seat != robin_engine::player_command::PlayerId::HOST
            && let Some((clock_frame, ms_until_next_frame)) = net_drain.latest_host_clock_sample
        {
            accept_host_frame_schedule(
                &mut mp_host_frame_schedule,
                clock_frame,
                ms_until_next_frame,
                manager.sim_frame,
            );
        }
        let mut mp_clock_pause = false;
        if host.net.is_some()
            && host.local_seat != robin_engine::player_command::PlayerId::HOST
            && !mp_waiting_for_initial_snapshot
            && !mp_waiting_for_begin_sim
            && mp_start_gate.is_none()
        {
            if let Some(deadline_ms) =
                host_scheduled_frame_deadline_ms(mp_host_frame_schedule, manager.sim_frame)
            {
                let now_ms = crate::window::process_uptime_ms();
                let until_frame_ms = deadline_ms - i64::from(now_ms);
                if until_frame_ms > 0 {
                    mp_clock_pause = true;
                    if now_ms.saturating_sub(last_mp_clock_ahead_log_ms) >= 1000 {
                        last_mp_clock_ahead_log_ms = now_ms;
                        tracing::info!(
                            scheduled_frame = mp_host_frame_schedule.map(|(frame, _)| frame),
                            local_frame = manager.sim_frame,
                            until_frame_ms,
                            "multiplayer: local frame is ahead of host schedule; holding sim"
                        );
                    }
                }
            } else {
                mp_clock_pause = true;
            }
        }
        let net_inputs = net_drain.inputs;
        if host.net.is_some() {
            recent_timeline_history.remember(crate::sim_timeline::SimSnapshot::new(
                manager.sim_frame,
                &manager.engine,
            ));
        }
        if !net_inputs.is_empty() {
            manager.engine.apply_commands(
                &mut host.engine_display,
                &mut host.input,
                &assets,
                &net_inputs,
            );
            for inp in net_inputs {
                frame_cmds.commands.push(inp);
            }
        }

        let mut pending_mp_state_hash: Option<(u32, u64)> = None;

        // Re-derive the corner HUD layout every frame so resolution
        // changes triggered from nested menus (options modal, Sherwood
        // flow, etc.) take effect without needing every call site to
        // plumb a mutable layout ref.  Cheap — just a few rect
        // arithmetic operations.
        corner_layout = crate::corner_hud::CornerHudLayout::for_resolution(
            renderer.screen_width() as u32,
            renderer.screen_height() as u32,
            &corner_sprites,
        );
        stature_layout = crate::stature_hud::StatureHudLayout::for_resolution(
            renderer.screen_width() as u32,
            renderer.screen_height() as u32,
            &stature_sprites,
        );

        // Refresh the host-cached back-to-front entity draw order from
        // the current engine state.  Consumed by this frame's input
        // handlers (hit-test via `find_focusable_entity`), render loop,
        // and titbit Z flush.  Headless replay has no hit-test/render
        // consumer, so skip the sort there.
        if !args.headless {
            host.draw_order = manager.engine.compute_display_order();
        }

        // Take the pre-commands snapshot for the rollback checker at
        // the top of the sim frame, before any input events are
        // processed and applied inline to the engine.
        if let Some(ref mut checker) = rollback_checker {
            checker.begin_frame(manager.sim_frame, &manager.engine);
        }
        // Same snapshot point for the rewind buffer — only commits
        // when this frame aligns to `SNAPSHOT_INTERVAL` — plus the
        // per-frame command log end_frame records further down.
        // Populated during replay too so the step-back debug key can
        // rewind through a recording.
        rewind_buffer.begin_frame(manager.sim_frame, &manager.engine, &assets);
        // Replay state-hash: sample the engine at the "start of
        // frame N — after N-1's tick, before N's commands" point,
        // before any event-loop handler (Resized → inline
        // `MinimapResize`, live input → local viewport edits, …)
        // mutates the engine.  The recorder write and the player
        // check MUST sample here in lockstep; if the recording
        // captures post-input state while the replay checks
        // pre-input state, every hash-carrying frame spuriously
        // desyncs.  The actual write is deferred to the recorder
        // block further down so the existing
        // `!rewind_active && !consumed_buffered` gating stays in
        // one place.
        let recorder_hash_this_frame: Option<u64> = replay_recorder.as_ref().and_then(|r| {
            let f = r.frame_number();
            (f % 25 == 0).then(|| crate::replay::state_hash(&manager.engine))
        });
        if let Some(ref player) = replay_player
            && !player.is_finished()
        {
            let frame_idx = player.current_frame();
            let is_terminal_frame = frame_idx + 1 >= player.total_frames();
            if !is_terminal_frame && let Some(expected) = player.hash_for_frame(frame_idx) {
                let actual = crate::replay::state_hash(&manager.engine);
                if actual != expected {
                    tracing::error!(
                        "Replay desync at frame {frame_idx}: expected {expected:016x}, got {actual:016x}"
                    );
                } else {
                    tracing::debug!("Replay hash OK @ frame {frame_idx}: {actual:016x}");
                }
            }
        }

        match handle_sherwood_campaign_map_overlay(
            &mut game,
            &mut manager,
            &mut host,
            &mut frame_cmds,
            &assets,
            campaign_ref,
            &mut *window,
            &mut renderer,
            &mut cursor_res,
            &mut cursor_renderer,
            &mut text_res,
            &mut sherwood_campaign_map,
            &mut menu_resources,
            &mut sherwood_enable,
        )
        .await?
        {
            HandlerAction::Continue => continue,
            HandlerAction::Exit(code) => return Ok(code),
            HandlerAction::Proceed => {}
        }

        // Tracks whether the pause menu closed at any point this
        // frame.  Used to flush queued input so actions that would
        // have run while the menu was up don't leak into the resumed
        // game.
        let mut pause_closed_this_frame = false;

        // ── Input ──
        // Flattened gameplay modals still own their own one-frame
        // `poll_events()` call.  If the main loop drains the window
        // first, modal widgets only receive occasional synthetic state,
        // while global gameplay shortcuts (notably Escape/DisplayMenu)
        // can fire underneath the modal.  Leave the event queue intact
        // whenever a modal is active or queued so `tick_active_modal`
        // gets first chance at the raw input.
        let modal_input_active = active_modal.is_some() || modal_state_pending(&host);
        if modal_input_active && pause_menu.is_some() {
            pause_menu = None;
            pause_closed_this_frame = true;
            renderer.clear_frozen_scene();
            threaded_input.reset_input_state();
            input_translator.reset_state();
            callbacks.set_sound_mode(crate::game::SoundMode::Mission);
            threaded_input.queue_mouse_motion_resync();
        }
        // Disjoint-borrow event poll: `event_pump`/`width`/`height` are
        // separate fields from `canvas`, which the renderer owns mutably.
        let mut events = if modal_input_active {
            Vec::new()
        } else {
            window.poll_events()
        };
        threaded_input.feed_sdl_events(&events);

        let rewind_active = handle_hold_to_rewind(
            &mut manager,
            assets.as_ref(),
            &threaded_input,
            &mut rewind_buffer,
            &mut rollback_checker,
            &mut replay_player,
        );

        // Field-disjoint access to keep `renderer` (holding &mut *window)
        // alive through the event loop.  Skip gamepad command dispatch
        // during replay/rewind — see input_suppressed comment below.
        if replay_player.is_none() && !rewind_active {
            handle_gamepad_events(
                &mut host,
                &mut manager,
                &assets,
                &mut threaded_input,
                &mut frame_cmds,
                &events,
                &mut window.active_gamepad,
            );
        }
        events.extend(threaded_input.drain_synthetic_events());

        // ── Handle window resize ──
        // Window-size changes don't change the game's logical render
        // resolution any more — `present()` letterboxes the fixed-size
        // offscreen RT into whatever shape the WM hands the swapchain.
        // The `host.viewport.set_screen_size` + `renderer.resize` below
        // are kept for the graphics-options menu's resolution change
        // path, which fakes a Resized event with the user-picked
        // logical size; under the new arch we should separate those,
        // but for now: only fire the full
        // logical-resize cascade if the new size matches one of the
        // menu's supported resolutions. Pure WM resizes drop through
        // and only the swapchain reconfigures.
        for event in &events {
            if let GameEvent::Resized(new_w, new_h) = *event {
                renderer.configure_surface_size(new_w, new_h);
                let is_logical_resize =
                    matches!((new_w, new_h), (640, 480) | (800, 600) | (1024, 768));
                if !is_logical_resize {
                    continue;
                }
                let w = new_w as f32;
                let h = new_h as f32;
                window.set_logical_size(new_w, new_h);
                host.viewport.set_screen_size(w, h);
                renderer.resize(new_w as u16, new_h as u16);
                threaded_input.set_clipping(crate::geo2d::BBox2D::from_coords(0.0, 0.0, w, h));
                input_translator = crate::input_translator::InputTranslator::new(w, h);
                // Reflect the active key profile into the freshly-
                // built translator.  Without this the resized
                // translator would fall back to the hardcoded
                // keyset1 defaults.
                input_translator.load_bindings_from_keyconfig(&host.key_config);
                // Re-install HUD-adjacent dead zones at the new
                // resolution.
                input_translator.install_hud_dead_zones();
                // Reposition minimap.
                if host.minimap_corner_size.x > 0.0 {
                    let cmd = PlayerCommand::MinimapResize {
                        base: geo2d::pt(w - 83.0, 38.0),
                        corner_size: host.minimap_corner_size,
                    };
                    dispatch_local_command(
                        &mut host,
                        &mut manager.engine,
                        &mut frame_cmds,
                        &assets,
                        &cmd,
                    );
                }
                // Reposition the Sherwood HUD buttons alongside the
                // other resolution-dependent layouts.
                sherwood_layout = crate::sherwood_hud::SherwoodHudLayout::for_resolution(
                    new_w,
                    new_h,
                    &sherwood_sprites,
                );
                zoom_layout =
                    crate::zoom_hud::ZoomHudLayout::for_resolution(new_w, new_h, &zoom_sprites);
                corner_layout = crate::corner_hud::CornerHudLayout::for_resolution(
                    new_w,
                    new_h,
                    &corner_sprites,
                );
                stature_layout = crate::stature_hud::StatureHudLayout::for_resolution(
                    new_w,
                    new_h,
                    &stature_sprites,
                );
            }
        }

        if threaded_input.is_ended() {
            *campaign_ref = manager.engine.take_campaign().unwrap_or_default();
            return Ok(GameCode::Quit);
        }

        match handle_sherwood_hud_buttons(
            &mut game,
            &mut manager,
            &mut host,
            &mut frame_cmds,
            &assets,
            callbacks,
            campaign_ref,
            &mut *window,
            &mut renderer,
            &mut cursor_res,
            &mut cursor_renderer,
            &menu_resources,
            &events,
            &sherwood_layout,
            &mut sherwood_enable,
            args.headless,
        )
        .await
        {
            HandlerAction::Continue => continue,
            HandlerAction::Exit(code) => return Ok(code),
            HandlerAction::Proceed => {}
        }

        // Suppress all mouse-driven HUD widget clicks while a replay
        // is playing back (recorded commands re-enter at the tick
        // boundary) or a rewind hold is active (live clicks
        // shouldn't perturb a reconstructed past state).  Without
        // this the user could click HUD buttons mid-replay and steer
        // the run.
        let input_suppressed = replay_player.is_some() || rewind_active;

        // Zoom HUD buttons (ZoomUp / ZoomDown).  Maps to
        // `EngineStateRequest::ZoomingUp/Down` — same path the
        // mouse wheel + keyboard bindings use.
        if !input_suppressed {
            let zoom_enable = crate::zoom_hud::ZoomButtonEnable::from_engine(
                &manager.engine,
                &host.engine_display,
            );
            let mut zoom_btn_hit = None;
            for event in &events {
                if let GameEvent::MouseDown(mx, my, 1 /* left */, _) = *event
                    && let Some(btn) = zoom_layout.hit_test(mx, my, zoom_enable)
                {
                    zoom_btn_hit = Some((btn, mx, my));
                    break;
                }
            }
            if let Some((btn, mx, my)) = zoom_btn_hit {
                use crate::zoom_hud::ZoomButton;
                let factor = match btn {
                    ZoomButton::ZoomUp => 2.0,
                    ZoomButton::ZoomDown => 0.5,
                };
                host.viewport
                    .zoom_by(factor, Some(robin_engine::geo2d::pt(mx as f32, my as f32)));
            }
        }

        // Corner HUD buttons (Clock / Sight / QuickStart).  Only
        // active on non-Sherwood missions.
        //
        // Left-click dispatches the activation message (record /
        // lock-alt / launch-all).  Right-click unlocks / deletes
        // macros.
        if !game.is_sherwood && !input_suppressed {
            let corner_enable = crate::corner_hud::CornerButtonEnable::from_engine(&manager.engine);
            for event in &events {
                match *event {
                    GameEvent::MouseDown(mx, my, 1 /* left */, _) => {
                        let Some(btn) = corner_layout.hit_test(mx, my, corner_enable) else {
                            continue;
                        };
                        dispatch_corner_button_left_click(
                            btn,
                            &mut manager,
                            &mut game,
                            &mut host,
                            &assets,
                            &mut frame_cmds,
                        );
                    }
                    GameEvent::MouseDown(mx, my, 3 /* right */, _) => {
                        let Some(btn) = corner_layout.hit_test_geometric(mx, my) else {
                            continue;
                        };
                        dispatch_corner_button_right_click(
                            btn,
                            &mut manager,
                            &mut host,
                            &assets,
                            &mut frame_cmds,
                        );
                    }
                    _ => {}
                }
            }

            // Stature up/down-arrow click dispatch.  Emits the same
            // PlayerCommand the keyboard path uses.  Clicking either
            // arrow also primes the focus-latch so the arrow stays
            // visually pressed while the sim runs the posture
            // transition.  Auto-clears when the aggregate stature
            // shifts.
            let stature = manager.engine.retrieve_stature(None);
            game.stature_focus.maybe_clear(stature);
            let stature_enable = crate::stature_hud::StatureEnable::from_stature(stature)
                .with_focus_latch(game.stature_focus);
            for event in &events {
                if let GameEvent::MouseDown(mx, my, 1 /* left */, _) = *event
                    && let Some(btn) = stature_layout.hit_test(mx, my, stature_enable)
                {
                    let cmd = btn.as_command();
                    dispatch_local_command(
                        &mut host,
                        &mut manager.engine,
                        &mut frame_cmds,
                        &assets,
                        &cmd,
                    );
                    match btn {
                        crate::stature_hud::StatureButton::Up => {
                            game.stature_focus.latch_stand_up(stature);
                        }
                        crate::stature_hud::StatureButton::Down => {
                            game.stature_focus.latch_crouch_down(stature);
                        }
                    }
                }
            }
        }

        // Edge-check the minimap accelerator scancode BEFORE
        // `translate_keyboard` advances the translator's prev-key
        // buffer.  The widget holds the accelerator itself and
        // toggles on release.
        let minimap_toggle_pressed = {
            let fast_key = host.minimap_fast_key;
            fast_key != 0
                && input_translator
                    .was_scancode_released(fast_key, &threaded_input.keyboard_state().keys)
        };

        // Step-debug keys: `.` (forward), `,` / Backspace (back), Enter
        // (unpause).  `.` and `,` step immediately on the press edge,
        // then repeat after a short hold delay so a normal key tap
        // advances exactly one frame but holding still scrubs. Backspace
        // keeps its held-state rewind scrub behavior.  Enter uses the
        // release edge so a held Enter doesn't spam-resume.  All checks
        // read raw scancodes rather than the bindable `GameAction` keyset.
        const STEP_REPEAT_INITIAL_DELAY_MS: u32 = 160;
        const STEP_REPEAT_INTERVAL_MS: u32 = 40;
        const SDL_SCANCODE_RETURN: u16 = 40;
        const SDL_SCANCODE_BACKSPACE: u16 = 42;
        const SDL_SCANCODE_COMMA: u16 = 54;
        const SDL_SCANCODE_PERIOD: u16 = 55;
        let keys = &threaded_input.keyboard_state().keys;
        let is_down = |sc: u16| keys.get(sc as usize).copied().unwrap_or(0) != 0;
        let step_forward_held = is_down(SDL_SCANCODE_PERIOD);
        let step_back_comma_held = is_down(SDL_SCANCODE_COMMA);
        let step_backspace_held = is_down(SDL_SCANCODE_BACKSPACE);
        let step_forward_hit = input_translator.was_scancode_pressed(SDL_SCANCODE_PERIOD, keys);
        let step_back_comma_hit = input_translator.was_scancode_pressed(SDL_SCANCODE_COMMA, keys);
        let repeat_step_key = |held: bool, hit: bool, repeat_at_ms: &mut Option<u32>| -> bool {
            if !held {
                *repeat_at_ms = None;
                return false;
            }
            if hit {
                *repeat_at_ms = Some(frame_start.saturating_add(STEP_REPEAT_INITIAL_DELAY_MS));
                return true;
            }
            if let Some(next_ms) = *repeat_at_ms
                && frame_start >= next_ms
            {
                *repeat_at_ms = Some(frame_start.saturating_add(STEP_REPEAT_INTERVAL_MS));
                return true;
            }
            false
        };
        let step_forward_pressed = repeat_step_key(
            step_forward_held,
            step_forward_hit,
            &mut step_forward_repeat_at_ms,
        );
        let step_back_pressed = repeat_step_key(
            step_back_comma_held,
            step_back_comma_hit,
            &mut step_back_repeat_at_ms,
        ) || step_backspace_held;
        let step_unpause_pressed =
            input_translator.was_scancode_released(SDL_SCANCODE_RETURN, keys);
        // Suppress these shortcuts when any modal input sink has focus
        // so `.` / `,` / Enter typed into the console, pause menu, or
        // text input don't accidentally freeze/step the sim.
        let step_keys_gated =
            console_overlay.is_visible() || pause_menu.is_some() || modal_input_active;
        if !step_keys_gated {
            if step_forward_pressed || step_back_pressed {
                manual_pause = true;
            }
            if step_unpause_pressed {
                manual_pause = false;
            }
        }
        let step_forward_pressed = step_forward_pressed && !step_keys_gated;
        let step_back_pressed = step_back_pressed && !step_keys_gated;

        // Translate to game actions
        let mut kb_actions = input_translator.translate_keyboard(
            &threaded_input.keyboard_state().keys,
            crate::input_translator::TranslationFlags::ALL,
        );
        if events
            .iter()
            .any(|event| matches!(event, GameEvent::MenuToggleRequested))
            || (pause_menu.is_none()
                && !modal_input_active
                && events
                    .iter()
                    .any(|event| matches!(event, GameEvent::PauseRequested)))
        {
            kb_actions.push(crate::input_translator::GameAction::DisplayMenu);
        }
        let mouse_actions = if threaded_input.has_position() {
            input_translator.translate_mouse(
                threaded_input.position().x,
                threaded_input.position().y,
                threaded_input.wheel_delta(),
            )
        } else {
            Vec::new()
        };

        // Helper: check if Ctrl is held via keyboard state
        let ctrl_held = {
            let ks = &threaded_input.keyboard_state().keys;
            const SDL_SCANCODE_LCTRL: usize = 224;
            const SDL_SCANCODE_RCTRL: usize = 228;
            (ks.len() > SDL_SCANCODE_LCTRL && ks[SDL_SCANCODE_LCTRL] != 0)
                || (ks.len() > SDL_SCANCODE_RCTRL && ks[SDL_SCANCODE_RCTRL] != 0)
        };
        let shift_held = {
            let ks = &threaded_input.keyboard_state().keys;
            const SDL_SCANCODE_LSHIFT: usize = 225;
            const SDL_SCANCODE_RSHIFT: usize = 229;
            (ks.len() > SDL_SCANCODE_LSHIFT && ks[SDL_SCANCODE_LSHIFT] != 0)
                || (ks.len() > SDL_SCANCODE_RSHIFT && ks[SDL_SCANCODE_RSHIFT] != 0)
        };
        let alt_held = {
            let ks = &threaded_input.keyboard_state().keys;
            const SDL_SCANCODE_LALT: usize = 226;
            const SDL_SCANCODE_RALT: usize = 230;
            (ks.len() > SDL_SCANCODE_LALT && ks[SDL_SCANCODE_LALT] != 0)
                || (ks.len() > SDL_SCANCODE_RALT && ks[SDL_SCANCODE_RALT] != 0)
        };
        // Persist the alt state on `InputState` so subsystems that
        // don't otherwise see the SDL modifier mask can read it.
        host.input.is_alt = alt_held;

        handle_console_overlay_events(
            &mut console_overlay,
            &mut manager.engine,
            &assets,
            &mut host,
            &mut dev,
            &events,
            &kb_actions,
            &mut input_translator,
        );

        // ── View-only input (scroll / zoom): always allowed ──
        // These mutate host-side viewport state only — never the sim —
        // so they're safe during replay playback and rewind, when the
        // user wants to pan/zoom around the paused world.  Suppressed
        // only when the console or the pause menu has focus.
        {
            use crate::input_translator::GameAction;
            use robin_engine::engine::ScrollDirection;
            let view_suppressed =
                console_overlay.is_visible() || pause_menu.is_some() || pause_closed_this_frame;
            if !view_suppressed {
                for action in kb_actions.iter().chain(mouse_actions.iter()) {
                    let scroll_suppressed_by_minimap =
                        matches!(
                            action,
                            GameAction::ScrollUp
                                | GameAction::ScrollDown
                                | GameAction::ScrollLeft
                                | GameAction::ScrollRight
                        ) && host.engine_display.minimap().drag_start();
                    if scroll_suppressed_by_minimap {
                        continue;
                    }
                    match action {
                        GameAction::ScrollUp => {
                            apply_local_viewport_scroll(&mut host, ScrollDirection::Up);
                        }
                        GameAction::ScrollDown => {
                            apply_local_viewport_scroll(&mut host, ScrollDirection::Down);
                        }
                        GameAction::ScrollLeft => {
                            apply_local_viewport_scroll(&mut host, ScrollDirection::Left);
                        }
                        GameAction::ScrollRight => {
                            apply_local_viewport_scroll(&mut host, ScrollDirection::Right);
                        }
                        GameAction::ZoomIn => {
                            let mp = threaded_input.position();
                            host.viewport
                                .zoom_by(2.0, Some(robin_engine::geo2d::pt(mp.x, mp.y)));
                        }
                        GameAction::ZoomOut => {
                            let mp = threaded_input.position();
                            host.viewport
                                .zoom_by(0.5, Some(robin_engine::geo2d::pt(mp.x, mp.y)));
                        }
                        _ => {}
                    }
                }
            }
        }

        // ── Mouse middle-drag viewport pan: always allowed ──
        // Same reasoning as the keyboard scroll/zoom block above —
        // ViewportPan is pure host-side viewport state.  Apply it here
        // before `handle_mouse_input` (which is gated by replay state)
        // can swallow it.
        if pause_menu.is_none() && !pause_closed_this_frame && !manager.engine.user_locked() {
            for event in &events {
                if let GameEvent::ViewportPan { xrel, yrel } = *event {
                    host.viewport
                        .scroll_by(geo2d::pt(-(xrel as f32), -(yrel as f32)));
                    host.input.cancel_multi_selection();
                }
            }
        }

        // ── Skip all sim-affecting input during replay / rewind ──
        // Recorded commands are injected at the tick boundary instead
        // (replay), or suppressed entirely (rewind — live input
        // shouldn't perturb a state reconstructed from the past).
        if replay_player.is_none() && !rewind_active {
            // Minimap accelerator key.
            // Suppressed while the console or pause menu has focus so the
            // toggle can't fire underneath modal UI.
            if minimap_toggle_pressed && !console_overlay.is_visible() && pause_menu.is_none() {
                let cmd = PlayerCommand::MinimapToggle;
                dispatch_local_command(
                    &mut host,
                    &mut manager.engine,
                    &mut frame_cmds,
                    &assets,
                    &cmd,
                );
            }

            for action in kb_actions.iter().chain(mouse_actions.iter()) {
                use crate::input_translator::GameAction;
                // Console captures every other action while it has focus.
                if console_overlay.is_visible() {
                    continue;
                }
                match action {
                    GameAction::DisplayConsole => {
                        // Already handled above — swallow so we don't
                        // hit the catch-all below.
                    }
                    GameAction::DisplayInfo => {
                        // Toggle the host flag — the per-frame debug
                        // overlay renderer polls `host.info_displayed`
                        // to decide whether to draw FPS / mission
                        // clock / music-mode bars.
                        host.info_displayed = !host.info_displayed;
                        tracing::debug!("DisplayInfo toggled: {}", host.info_displayed);
                    }
                    GameAction::DisplayMenu => {
                        if pause_menu.is_some() {
                            pause_menu = None;
                            pause_closed_this_frame = true;
                            renderer.clear_frozen_scene();
                            threaded_input.reset_input_state();
                            input_translator.reset_state();
                            callbacks.set_sound_mode(crate::game::SoundMode::Mission);
                            // Forward a MSG_MOUSE_MOVED at the current
                            // cursor position so HUD widgets /
                            // portraits / buttons under the cursor
                            // rebuild their hover highlight on the
                            // first frame after the menu closes.
                            threaded_input.queue_mouse_motion_resync();
                            // Resume play-time recording after the
                            // modal closes.
                            callbacks.start_play_time();
                        } else {
                            // Suspend play-time recording before
                            // opening the modal so `MissionLength`
                            // doesn't count wall-clock spent in the
                            // pause menu.
                            callbacks.suspend_play_time();
                            if let Some(ref resources) = menu_resources {
                                pause_menu = Some(PauseMenu::new(resources, restart_allowed));
                            } else {
                                // Fallback — still open the menu with synthetic sizes.
                                let fallback = IngameMenuResources::new(
                                    &mut renderer,
                                    host.shipping.as_deref(),
                                );
                                if let Some(res) = fallback.as_ref() {
                                    pause_menu = Some(PauseMenu::new(res, restart_allowed));
                                }
                                menu_resources = fallback;
                            }
                            if pause_menu.is_some() {
                                // Freeze the current screen so the
                                // pause-menu backdrop composites over
                                // a still frame instead of the live
                                // engine output.  Idempotent; the
                                // symmetric close-branch above calls
                                // `clear_frozen_scene`.
                                renderer.freeze_scene_for_modal();
                                callbacks.set_sound_mode(crate::game::SoundMode::Menu);
                            }
                        }
                    }
                    _ if pause_menu.is_some() || pause_closed_this_frame => {
                        // Skip all other game actions while paused
                        // and for the remainder of the frame if pause
                        // was toggled off this frame, so actions
                        // queued during pause don't fire the instant
                        // the game resumes.
                    }
                    _ => {
                        match action {
                            GameAction::SlowMotion => {
                                // Toggle the slow-motion pacing flag.
                                // The frame-pacing block multiplies
                                // the 40 ms frame target by 10 when
                                // set.  Pure host-side, not sim state.
                                host.slow_motion = !host.slow_motion;
                            }
                            GameAction::SwitchMaskedDisplay => {
                                // Toggle the "draw hidden" debug view.
                                // This is per-seat presentation state;
                                // script-visible outline display
                                // changes still come from sim-side
                                // `SetOutlineDisplay` commands.
                                host.input.draw_hidden = !host.input.draw_hidden;
                            }
                            // Scroll{Up,Down,Left,Right} and Zoom{In,Out}
                            // are handled by the always-on view-only
                            // input pass at the top of the frame so
                            // they work during replay/rewind.
                            GameAction::ScrollUp
                            | GameAction::ScrollDown
                            | GameAction::ScrollLeft
                            | GameAction::ScrollRight
                            | GameAction::ZoomIn
                            | GameAction::ZoomOut => {}
                            GameAction::SelectAll => {
                                let cmd = PlayerCommand::SelectAllPcs;
                                dispatch_local_command(
                                    &mut host,
                                    &mut manager.engine,
                                    &mut frame_cmds,
                                    &assets,
                                    &cmd,
                                );
                            }
                            GameAction::UnselectAll => {
                                let cmd = PlayerCommand::UnselectAllPcs;
                                dispatch_local_command(
                                    &mut host,
                                    &mut manager.engine,
                                    &mut frame_cmds,
                                    &assets,
                                    &cmd,
                                );
                            }
                            GameAction::SelectAction { index } => {
                                let selected = manager.engine.seat_selection(host.local_seat);
                                if selected.len() == 1 {
                                    let pc_id = selected[0];
                                    let cmd = PlayerCommand::SelectAction {
                                        pc_id,
                                        action_index: *index as u32,
                                    };
                                    dispatch_local_command(
                                        &mut host,
                                        &mut manager.engine,
                                        &mut frame_cmds,
                                        &assets,
                                        &cmd,
                                    );
                                }
                            }
                            GameAction::SelectCharacter { portrait_index } => {
                                let idx = *portrait_index as usize;
                                let cmd = if ctrl_held {
                                    PlayerCommand::AssignQuickGroup { index: idx as u8 }
                                } else {
                                    let has_group = idx < 9
                                        && !manager.engine.quick_select_group(idx).is_empty();
                                    if has_group {
                                        PlayerCommand::RecallQuickGroup { index: idx as u8 }
                                    } else {
                                        PlayerCommand::SelectByPortrait {
                                            portrait_index: *portrait_index as u32,
                                            append: false,
                                        }
                                    }
                                };
                                dispatch_local_command(
                                    &mut host,
                                    &mut manager.engine,
                                    &mut frame_cmds,
                                    &assets,
                                    &cmd,
                                );
                            }
                            GameAction::QuickSave => {
                                // F9 (default binding).  The quick-
                                // save request rotates the previous
                                // QuickSave to ExQuickSave before
                                // writing — distinct from the generic
                                // `LevelSave` state-machine path.
                                //
                                // Defer the save until any active zoom
                                // finishes so the mid-zoom background
                                // isn't captured.
                                if !manager.engine.is_zoom_possible(&host.engine_display) {
                                    game.quick_save_after_zoom = true;
                                } else {
                                    let mission_id = manager
                                        .engine
                                        .campaign()
                                        .map(|c| current_mission_id(c, &assets.profile_manager))
                                        .unwrap_or(0);
                                    callbacks.pending =
                                        Some(crate::main_entry::SaveLoadRequest::QuickSave {
                                            mission_id,
                                        });
                                }
                            }
                            GameAction::QuickLoad => {
                                // F12 (default binding).  Loads the
                                // quick-save slot into the current
                                // engine, with a zoom-defer gate and a
                                // Shift+F12 → backup (ExQuickSave)
                                // shortcut.  The cross-mission
                                // confirmation modal is handled by
                                // `confirm_quickload_cross_mission`
                                // running before the per-frame
                                // `perform_pending_save_load` flush —
                                // it either drops the queued request
                                // (No) or rewrites it to
                                // `SaveLoadRequest::Load` so the
                                // cross-mission `PendingLevelLoad`
                                // routing performs the mission swap
                                // (Yes).
                                if !manager.engine.is_zoom_possible(&host.engine_display) {
                                    game.quick_load_after_zoom = true;
                                } else {
                                    callbacks.pending =
                                        Some(crate::main_entry::SaveLoadRequest::QuickLoad {
                                            use_backup: shift_held,
                                        });
                                }
                            }
                            GameAction::CrouchDown => {
                                // Prime the crouch-down focus latch
                                // before issuing the command so the
                                // down-arrow "pressed" overlay
                                // appears for the full transition.
                                // Snapshot the pre-command stature so
                                // the latch clears the first frame
                                // posture shifts.
                                let pre = manager.engine.retrieve_stature(None);
                                let cmd = PlayerCommand::CrouchDown;
                                dispatch_local_command(
                                    &mut host,
                                    &mut manager.engine,
                                    &mut frame_cmds,
                                    &assets,
                                    &cmd,
                                );
                                game.stature_focus.latch_crouch_down(pre);
                            }
                            GameAction::StandUp => {
                                // Companion of CrouchDown above —
                                // primes the stand-up focus latch so
                                // the up-arrow holds pressed while
                                // the sim runs the stand-up animation.
                                let pre = manager.engine.retrieve_stature(None);
                                let cmd = PlayerCommand::StandUp;
                                dispatch_local_command(
                                    &mut host,
                                    &mut manager.engine,
                                    &mut frame_cmds,
                                    &assets,
                                    &cmd,
                                );
                                game.stature_focus.latch_stand_up(pre);
                            }
                            GameAction::KeyControl => {
                                // Save the current action on every
                                // selected PC.  Used by the
                                // "move during action" modifier so
                                // ctrl-release can restore the action.
                                let cmd = PlayerCommand::KeyControl;
                                dispatch_local_command(
                                    &mut host,
                                    &mut manager.engine,
                                    &mut frame_cmds,
                                    &assets,
                                    &cmd,
                                );
                            }
                            GameAction::KeyReleaseControl => {
                                // Restore each selected PC's saved
                                // action on ctrl-up.  The handler
                                // honours the macOS carve-out via
                                // `cfg(target_os = "macos")`.
                                let cmd = PlayerCommand::KeyReleaseControl;
                                dispatch_local_command(
                                    &mut host,
                                    &mut manager.engine,
                                    &mut frame_cmds,
                                    &assets,
                                    &cmd,
                                );
                            }
                            GameAction::SwitchTask => {
                                // Emit a reset-input so held-key edges
                                // caught during an Alt+Tab / Ctrl+Esc
                                // task switch don't re-fire in-game
                                // when focus returns.  Route through
                                // the engine messenger so the drain
                                // handler applies the reset
                                // symmetrically with the hide-console
                                // path.
                                manager.engine.send_simple_message(
                                    robin_engine::messenger::SimpleMessage::SwitchTask,
                                );
                            }
                            GameAction::Teleport => {
                                // F7 cheat — teleport every selected
                                // PC to the current mouse map point.
                                let mouse_screen = threaded_input.position();
                                if let Some(mouse_map) = host.viewport.screen_to_map(mouse_screen) {
                                    if !manager.engine.seat_selection(host.local_seat).is_empty() {
                                        // Resolve destination sector/layer
                                        // via `get_sector_screen_accessible`
                                        // and bail when it returns None.
                                        // Doors / motion obstacles / empty
                                        // cells are rejected up front rather
                                        // than going through as the topmost
                                        // hit.
                                        let accessible = manager
                                            .engine
                                            .fast_grid()
                                            .get_sector_screen_accessible(mouse_map);
                                        if let Some(sector_idx) = accessible.sector_idx {
                                            let cmd = PlayerCommand::TeleportSelectedToPoint {
                                                dest: mouse_map,
                                                layer: accessible.layer,
                                                sector: u16::try_from(u32::from(sector_idx))
                                                    .ok()
                                                    .and_then(
                                                    robin_engine::position_interface::SectorHandle::new,
                                                ),
                                            };
                                            dispatch_local_command(
                                                &mut host,
                                                &mut manager.engine,
                                                &mut frame_cmds,
                                                &assets,
                                                &cmd,
                                            );
                                        }
                                    } else if dev.debug.free_shadow_polygon {
                                        // With no PCs selected and the
                                        // shadow-polygon dev cheat on,
                                        // reposition the free-floating
                                        // shadow-polygon viewer at the
                                        // mouse map point, 45 units
                                        // above the impact surface.
                                        // Non-sim dev state, handled
                                        // host-side outside the replay
                                        // pipeline.
                                        let p3d = manager.engine.fast_grid().convert_2d_to_3d(
                                            mouse_map,
                                            robin_engine::sight_obstacle::SIGHTOBSTACLE_MOUSE,
                                            manager.engine.sight_obstacles(&assets),
                                        );
                                        dev.cheat_free_shadow_polygon_pos =
                                            Some(robin_engine::element::Point3D {
                                                x: p3d.x,
                                                y: p3d.y,
                                                z: p3d.z + 45.0,
                                            });
                                    }
                                }
                            }
                            GameAction::RecordQa => {
                                // F5 (default binding) — replay the
                                // corner-clock left-click behaviour:
                                // start / cycle the macro slot for
                                // the currently-selected PC(s).
                                if !game.is_sherwood {
                                    dispatch_corner_button_left_click(
                                        crate::corner_hud::CornerButton::Clock,
                                        &mut manager,
                                        &mut game,
                                        &mut host,
                                        &assets,
                                        &mut frame_cmds,
                                    );
                                }
                            }
                            GameAction::PrintScreen => {
                                // Defer to the post-render drain so we
                                // capture the fully-composited frame
                                // rather than an incomplete in-progress draw
                                // queue. Ctrl matches the historical wide
                                // snapshot branch; Shift applies the 3x3
                                // median filter branch.
                                host.pending_print_screen = Some(
                                    print_screen_request_from_modifiers(ctrl_held, shift_held),
                                );
                            }
                            _ => {
                                tracing::trace!("Game action: {:?}", action);
                            }
                        }
                    }
                }
            }

            match handle_pause_menu_events(
                &mut pause_menu,
                &mut pause_closed_this_frame,
                &mut host,
                &mut manager,
                &mut game,
                &assets,
                callbacks,
                campaign_ref,
                &mut *window,
                &mut renderer,
                &mut cursor_res,
                &mut cursor_renderer,
                &menu_resources,
                &mut audio_backend,
                &*sample_loader,
                &mut threaded_input,
                &mut input_translator,
                &mut sherwood_layout,
                &mut zoom_layout,
                &zoom_sprites,
                &mut frame_cmds,
                &events,
            )
            .await
            {
                HandlerAction::Continue => continue,
                HandlerAction::Exit(code) => return Ok(code),
                HandlerAction::Proceed => {}
            }

            handle_mouse_input(
                &mut manager,
                &mut host,
                &assets,
                &renderer,
                &portrait_cache,
                &mut frame_cmds,
                &events,
                pause_menu.as_ref(),
                pause_closed_this_frame,
                shift_held,
                ctrl_held,
            );
        } // if replay_player.is_none()

        // ── Cross-mission QuickLoad confirmation modal ──
        // Quick-load prompts the
        // player with `MSG_REALLY_LOAD_QUICKSAVE` whenever the quicksave
        // header's mission ID differs from the running mission.  Run
        // the modal here, before the thumbnail capture and state-machine
        // drain — the helper either drops the pending request (No) or
        // rewrites it into a `Load` so the existing cross-mission
        // routing performs the mission swap (Yes).
        confirm_quickload_cross_mission(
            callbacks,
            &manager.engine,
            profiles,
            &host,
            &mut *window,
            &mut renderer,
            &mut cursor_res,
            &mut cursor_renderer,
            &menu_resources,
        )
        .await;

        // ── Process game operations (save/load/quit/win/lose) ──
        //
        // The Game state machine queues save/load intents on the
        // callbacks; `perform_pending_save_load` then flushes them to
        // disk with live engine access.
        let exit_code = manager
            .engine
            .campaign()
            .and_then(|c| game.process_operation(c, profiles, callbacks));
        let pending_thumbnail = if callbacks
            .pending
            .as_ref()
            .is_some_and(|request| request.writes_save_payload())
            && !host.skip_render
            && !args.headless
            && !modal_rendered_this_frame
        {
            pre_render_engine_setup(&mut manager, &mut host, assets.as_ref(), &mut renderer);
            update_mouse_and_cursor(
                &mut manager,
                &mut host,
                &assets,
                &dev,
                &mut renderer,
                &mut cursor_res,
                &mut cursor_renderer,
                &threaded_input,
                &portrait_cache,
                shift_held,
                &mut last_cursor_id,
            );
            let display_snapshot = host.engine_display.clone();
            let mut render_ctx = RenderContext {
                renderer: &mut renderer,
                cursor_renderer: &mut cursor_renderer,
                selection_mark_renderer: &mut selection_mark_renderer,
                titbit_renderer: &mut titbit_renderer,
                console_overlay: &mut console_overlay,
                zoom_tooltip: &mut zoom_tooltip,
                corner_tooltip: &mut corner_tooltip,
                requirements_tooltip: &mut requirements_tooltip,
                blazon_tooltip: &mut blazon_tooltip,
                stature_tooltip: &mut stature_tooltip,
                sherwood_tooltip: &mut sherwood_tooltip,
                pc_action_tooltip: &mut pc_action_tooltip,
                mouse_trail_renderer: mouse_trail_renderer.as_ref(),
                portrait_cache: &portrait_cache,
                menu_resources: menu_resources.as_ref(),
                hud_fonts: hud_fonts.as_ref(),
                short_briefing_strings: &short_briefing_strings,
                sherwood_layout: &sherwood_layout,
                sherwood_sprites: &sherwood_sprites,
                zoom_layout: &zoom_layout,
                zoom_sprites: &zoom_sprites,
                corner_layout: &corner_layout,
                corner_sprites: &corner_sprites,
                stature_layout: &stature_layout,
                stature_sprites: &stature_sprites,
                threaded_input: &threaded_input,
                game: &game,
                pause_menu: pause_menu.as_ref(),
                sherwood_enable,
                shift_held,
                rewind_active,
                display_info_elapsed_secs:
                    <RustCallbacks as crate::game::GameCallbacks>::get_current_playing_time(
                        callbacks,
                        campaign_ref,
                    ),
            };
            capture_save_thumbnail(
                &manager.engine,
                &display_snapshot,
                &mut host,
                &assets,
                &dev,
                &mut render_ctx,
            )
        } else {
            None
        };
        if let Some(exit_code) = exit_code {
            tracing::info!("Game exited with: {:?}", exit_code);
            // Flush any pending save before returning (e.g. the
            // quit-time continue save).
            let save_load_processed = perform_pending_save_load(
                &mut host,
                &mut game,
                callbacks,
                &mut manager.engine,
                assets.as_ref(),
                profiles,
                pending_thumbnail.clone(),
            );
            if save_load_processed && let Some(ref mut checker) = rollback_checker {
                checker.reset();
            }
            if let Some(sync) = callbacks.post_load_sync.take() {
                game.apply_post_load_sync(sync.is_continue);
                game.post_load_resolution_resync();
            }
            *campaign_ref = manager.engine.take_campaign().unwrap_or_default();
            return Ok(exit_code);
        }
        let save_load_processed = perform_pending_save_load(
            &mut host,
            &mut game,
            callbacks,
            &mut manager.engine,
            assets.as_ref(),
            profiles,
            pending_thumbnail,
        );
        if save_load_processed && let Some(ref mut checker) = rollback_checker {
            checker.reset();
        }

        // ── Cross-mission load: bubble up ──
        // `perform_pending_save_load` stashes a `PendingLevelLoad` when the
        // chosen slot targets a different mission than the one running. Force
        // the Game state machine into LevelLoad so `process_operation` exits
        // on the next iteration; the outer session loop will switch missions
        // and re-queue the Load on the fresh engine.
        if callbacks.pending_level_load.is_some() {
            game.operation.set(GameCode::LevelLoad);
            *campaign_ref = manager.engine.take_campaign().unwrap_or_default();
            return Ok(GameCode::LevelLoad);
        }

        // ── Post-load slot-type sync ──
        // Sync the continue-save flag and re-arm the campaign-map
        // overlay if the loaded save had it open.  `post_load_sync`
        // is armed by `perform_pending_save_load` after any Load
        // variant succeeds, threading the slot type back out of the
        // save-I/O layer.
        if let Some(sync) = callbacks.post_load_sync.take() {
            game.apply_post_load_sync(sync.is_continue);
            game.post_load_resolution_resync();
        }

        // ── Reset input state after load ──
        // Clear the translator's scancode ring so half-pressed keys
        // at save time don't emit stale edge-detection events on the
        // next frame.  Host-side `InputState` is already wiped by
        // `Host::post_load_reset` during `apply_to`; this clears the
        // mirror that lives on the input translator itself.
        if std::mem::take(&mut callbacks.pending_reset_input) {
            input_translator.reset_state();
        }

        // ── Save/load banner ──
        // `perform_pending_save_load` queues GAME_SAVED/GAME_LOADED
        // on every successful non-Restart/non-Sherwood save or load.
        // Threaded onto `game.message_text` / `message_delay` through
        // `Game::display_message`.
        if let Some(kind) = callbacks.pending_save_banner.take() {
            let text = match kind {
                crate::main_entry::SaveBannerKind::Saved => "Game saved.",
                crate::main_entry::SaveBannerKind::Loaded => "Game loaded.",
            };
            // 100 ticks — `display_message` is a fire-and-forget
            // delay that the renderer polls (the hook lives in
            // `render_frame` and calls
            // `hud_text::render_transient_message`).  IDs
            // `MT_MSG_GAME_SAVED` / `MT_MSG_GAME_LOADED` should be
            // wired for localisation later.
            game.display_message(text.to_string(), 100);
        }

        // ── Replay: inject recorded commands + desync check ──
        // `ModalDismiss` commands are split out of the recorded stream
        // here and handed to the modal drain step further down, so the
        // interactive dialog / popup event loops are skipped during
        // playback. All other commands are sim-affecting and applied
        // immediately.
        // Freeze every sim-advancing step (replay playback, engine
        // tick, rewind-buffer commit, sim-frame increment) whenever the
        // user has asked to pause.  Under `--replay`, this means the
        // player's cursor on the recorded command stream stops too —
        // otherwise `--start-paused --replay` would still race through
        // the replay even though the tick was suppressed.
        let modal_pause = active_modal.as_ref().is_some_and(|modal| !modal.is_empty());

        // Drain once more at the last deterministic pre-tick boundary.
        // Packets can arrive after the top-of-loop drain while this
        // frame handles UI, local input, and modal work.  Applying due
        // inputs here keeps them on the same `sim_frame` without
        // mutating sim state at arbitrary points in the frame.
        if host.net.is_some() && !rewind_active {
            if let Some(net) = host.net.as_ref() {
                net.publish_frame(manager.sim_frame);
            }
            let pre_tick_net_drain = drain_net_inputs(
                &mut host,
                &mut manager,
                assets.as_ref(),
                &mut rewind_buffer,
                &mut peer_hashes,
                &mut recent_timeline_history,
            );
            if pre_tick_net_drain.rewrote_sim_state
                && let Some(ref mut checker) = rollback_checker
            {
                checker.reset();
            }
            if let Some(rollback) = pre_tick_net_drain.rollback.clone() {
                last_mp_rollback = Some(rollback);
            }
            if let Some((_frame, start_epoch_ms)) = pre_tick_net_drain.begin_sim {
                mp_waiting_for_begin_sim = false;
                mp_start_gate = Some(start_epoch_ms);
                manual_pause = true;
            }
            if mp_waiting_for_initial_snapshot && pre_tick_net_drain.received_initial_snapshot {
                mp_waiting_for_initial_snapshot = false;
                tracing::info!(
                    "multiplayer: initial snapshot received; client ready for start barrier"
                );
            }
            if mp_waiting_for_initial_snapshot || mp_waiting_for_begin_sim {
                manual_pause = true;
            }
            if host.net.is_some()
                && host.local_seat != robin_engine::player_command::PlayerId::HOST
                && let Some((clock_frame, ms_until_next_frame)) =
                    pre_tick_net_drain.latest_host_clock_sample
            {
                accept_host_frame_schedule(
                    &mut mp_host_frame_schedule,
                    clock_frame,
                    ms_until_next_frame,
                    manager.sim_frame,
                );
            }
            if host.net.is_some()
                && host.local_seat != robin_engine::player_command::PlayerId::HOST
                && !mp_waiting_for_initial_snapshot
                && !mp_waiting_for_begin_sim
                && mp_start_gate.is_none()
            {
                if let Some(deadline_ms) =
                    host_scheduled_frame_deadline_ms(mp_host_frame_schedule, manager.sim_frame)
                {
                    let now_ms = crate::window::process_uptime_ms();
                    let until_frame_ms = deadline_ms - i64::from(now_ms);
                    if until_frame_ms > 0 {
                        mp_clock_pause = true;
                        if now_ms.saturating_sub(last_mp_clock_ahead_log_ms) >= 1000 {
                            last_mp_clock_ahead_log_ms = now_ms;
                            tracing::info!(
                                scheduled_frame = mp_host_frame_schedule.map(|(frame, _)| frame),
                                local_frame = manager.sim_frame,
                                until_frame_ms,
                                "multiplayer: local frame is ahead of host schedule; holding sim"
                            );
                        }
                    }
                } else {
                    mp_clock_pause = true;
                }
            }
            if pre_tick_net_drain.rewrote_sim_state && host.net.is_some() {
                recent_timeline_history.remember(crate::sim_timeline::SimSnapshot::new(
                    manager.sim_frame,
                    &manager.engine,
                ));
            }
            if !pre_tick_net_drain.inputs.is_empty() {
                manager.engine.apply_commands(
                    &mut host.engine_display,
                    &mut host.input,
                    &assets,
                    &pre_tick_net_drain.inputs,
                );
                frame_cmds.commands.extend(pre_tick_net_drain.inputs);
            }
        }

        // ── Multiplayer: state hash broadcast / verify ──
        // Sample after the final deterministic pre-tick network drain.
        // Inputs can arrive between the top-of-loop drain and this
        // boundary; hashing earlier can compare two machines that will
        // tick the same commands but sampled before/after a current-frame
        // input that just arrived.
        if host.net.is_some()
            && manager
                .sim_frame
                .is_multiple_of(crate::multiplayer::STATE_HASH_INTERVAL)
        {
            if host.local_seat == robin_engine::player_command::PlayerId::HOST
                && last_mp_state_hash_frame != Some(manager.sim_frame)
            {
                last_mp_state_hash_frame = Some(manager.sim_frame);
                let mp_hash_start = web_time::Instant::now();
                let live_hash_start = web_time::Instant::now();
                let local_hash = crate::replay::state_hash(&manager.engine);
                let live_hash_us = live_hash_start.elapsed().as_micros();
                pending_mp_state_hash = Some((manager.sim_frame, local_hash));

                let total_us = mp_hash_start.elapsed().as_micros();
                tracing::debug!(
                    frame = manager.sim_frame,
                    total_us,
                    live_hash_us,
                    "multiplayer hash frame timing"
                );
            } else if let Some(&host_hash) = peer_hashes.get(&manager.sim_frame) {
                let local_hash = crate::replay::state_hash(&manager.engine);
                if local_hash != host_hash {
                    let last_rollback_path = last_mp_rollback.as_ref().map_or("none", |r| r.path);
                    let last_rollback_earliest =
                        last_mp_rollback.as_ref().map_or(0, |r| r.earliest_frame);
                    let last_rollback_target =
                        last_mp_rollback.as_ref().map_or(0, |r| r.target_frame);
                    let last_rollback_replayed =
                        last_mp_rollback.as_ref().map_or(0, |r| r.replayed_frames);
                    let last_rollback_total_us =
                        last_mp_rollback.as_ref().map_or(0, |r| r.total_us);
                    tracing::error!(
                        frame = manager.sim_frame,
                        local = format!("{local_hash:016x}"),
                        host = format!("{host_hash:016x}"),
                        host_schedule_frame = mp_host_frame_schedule.map(|(frame, _)| frame),
                        pending_input_frames = manager.pending_inputs.len(),
                        last_rollback_path,
                        last_rollback_earliest,
                        last_rollback_target,
                        last_rollback_replayed,
                        last_rollback_total_us,
                        "multiplayer DESYNC: local engine hash differs from host's"
                    );
                } else {
                    tracing::debug!(frame = manager.sim_frame, "multiplayer hash OK");
                }
            }
            // Stale entries: drop everything strictly older than
            // sim_frame so the map doesn't grow unbounded if the
            // host sends ahead of our verification.
            peer_hashes.retain(|&f, _| f > manager.sim_frame);
        }

        let mut paused = pause_menu.is_some() || manual_pause || mp_clock_pause || modal_pause;

        let mut replay_modal_dismissals: std::collections::VecDeque<
            robin_engine::player_command::PlayerCommand,
        > = std::collections::VecDeque::new();
        if let Some(ref mut player) = replay_player
            && !paused
        {
            if player.is_finished() {
                if !replay_finished_logged {
                    tracing::info!("Replay finished after {} frames", player.current_frame());
                    replay_finished_logged = true;
                }
                manual_pause = true;
                paused = true;
            } else {
                replay_finished_logged = false;
                // Hash check for this frame was already done at the top of
                // the loop (see the record/check block after begin_frame),
                // so the check and the recorder write share the same
                // engine-state sampling point and can't drift.
                let replay_cmds = player.next_frame();
                let mut sim_cmds: Vec<PlayerInput> = Vec::with_capacity(replay_cmds.len());
                for cmd in replay_cmds {
                    match cmd.command {
                        robin_engine::player_command::PlayerCommand::ModalDismiss { .. } => {
                            replay_modal_dismissals.push_back(cmd.command.clone());
                        }
                        _ => sim_cmds.push(cmd.clone()),
                    }
                }
                manager.engine.apply_commands(
                    &mut host.engine_display,
                    &mut host.input,
                    &assets,
                    &sim_cmds,
                );
                // Discard any live input commands during replay, then stash
                // the commands we actually applied so the rewind buffer's
                // per-frame command log captures them — otherwise a later
                // step-back during replay has nothing to walk forward from
                // its snapshots.  Recording is still a no-op (the recorder
                // gate below short-circuits when `replay_recorder` is None,
                // which it always is in replay mode).
                frame_cmds = FrameCommands::new();
                frame_cmds.commands = sim_cmds;
            }
        }

        // ── Post-rewind auto-replay ──
        // When the player releases the rewind key, `sim_frame` ends up
        // inside the buffer's recorded range and the original
        // `[sim_frame .. next_record_frame)` commands are still
        // buffered.  Keep replaying that future forward one frame at a
        // time until a live input fires — at which point the player
        // has chosen to diverge, so truncate the now-orphaned future
        // out of the buffer and record fresh commands from here on.
        //
        // Paused frames don't tick, so they'd consume the same
        // buffered slot repeatedly; skipped here for the same reason
        // the tick below is.  Replay playback (`--replay`) has its
        // own command stream and stays unaffected (guarded below by
        // the separate `replay_player.is_none()` check).
        let mut consumed_buffered = false;
        if !rewind_active
            && !paused
            && replay_player.is_none()
            && manager.sim_frame < rewind_buffer.next_record_frame()
        {
            if frame_cmds.commands.is_empty() {
                if let Some(recorded) = rewind_buffer.commands_for(manager.sim_frame) {
                    let recorded: Vec<PlayerInput> = recorded.to_vec();
                    manager.engine.apply_commands(
                        &mut host.engine_display,
                        &mut host.input,
                        &assets,
                        &recorded,
                    );
                    frame_cmds.commands = recorded;
                    consumed_buffered = true;
                    tracing::trace!("Auto-replay → frame {}", manager.sim_frame);
                }
            } else {
                tracing::trace!(
                    "Auto-replay interrupted by live input; truncating buffer at {}",
                    manager.sim_frame
                );
                rewind_buffer.truncate_future(manager.sim_frame);
            }
        }

        // ── Locker follow hover ──
        // `SelectFollowElement` mutates sim-visible seat/camera state.
        // Keep it in the recorded pre-tick command stream; doing this
        // from the render cursor pass applies it after rollback/rewind
        // have committed the frame and leaves no command to replay.
        if replay_player.is_none()
            && !rewind_active
            && !paused
            && manager.engine.locker_active()
            && let Some(mouse_map) = host.viewport.screen_to_map(threaded_input.position())
            && let Some(id) = manager.engine.find_focusable_npc(
                &assets,
                mouse_map,
                robin_engine::element::Focus::View,
            )
        {
            let cmd = PlayerCommand::SelectFollowElement {
                entity_id: Some(id),
            };
            dispatch_local_command(
                &mut host,
                &mut manager.engine,
                &mut frame_cmds,
                &assets,
                &cmd,
            );
        }

        // ── Per-frame aim orientation ──
        // This is sim state (direction/current animation row and
        // bow raise/lower command launch), so it must be recorded in
        // the same frame command log as clicks and keys.  Do not run
        // it from `host_mouse::update_mouse`: render happens after
        // rollback has committed the frame, and live-only mutation
        // there desynchronizes replay/rollback.
        if replay_player.is_none()
            && !rewind_active
            && !paused
            && let Some(mouse_map) = host.viewport.screen_to_map(threaded_input.position())
        {
            let bow_armed = manager.engine.selected_action_for_seat(host.local_seat)
                == robin_engine::profiles::Action::Bow;
            if host.time_no_mouse_move != 0 || bow_armed {
                let cmd = PlayerCommand::PerformOrientation { mouse_map };
                dispatch_local_command(
                    &mut host,
                    &mut manager.engine,
                    &mut frame_cmds,
                    &assets,
                    &cmd,
                );
            }
        }

        // ── Record frame commands + periodic state hash ──
        // The matching `recorder.end_frame()` runs after the modal
        // drain block so `ModalDismiss` entries land in the same
        // frame as the modal that produced them.  Skipped while
        // rewinding (no tick is running) and while consuming buffered
        // commands (they were already written to disk on the original
        // pass). The hash itself was computed at the top of the
        // frame into `recorder_hash_this_frame` — writing it here
        // keeps the gating in one place.
        if let Some(ref mut recorder) = replay_recorder
            && !rewind_active
            && !consumed_buffered
        {
            if let Some(hash) = recorder_hash_this_frame {
                recorder.write_hash(recorder.frame_number(), hash);
            }
            for cmd in &frame_cmds.commands {
                recorder.push(cmd.clone());
            }
        }

        // ── Engine tick ──
        // The pause menu freezes the simulation by skipping the
        // hourglass while the menu is shown.  Rewind also freezes
        // the tick: the engine state was just replaced with a
        // reconstruction of an earlier frame and must not be
        // advanced this frame.
        let tick_exit_code = if rewind_active {
            None
        } else {
            let mut display = std::mem::take(&mut host.engine_display);
            let result = game.run_engine_tick(
                &mut host,
                &mut display,
                assets.as_ref(),
                &mut manager.engine,
                &mut dev,
                false,
                paused,
            );
            host.engine_display = display;
            result
        };

        // ── Drain pending script-RPC requests ──
        // External tools (HTTP /native, /command, /console, /state, …)
        // queue invocations on the server thread; we run them here so
        // any side-effect commands (camera, dialog, sequences, sound,
        // PlayerCommand applies) land on the same frame as the tick
        // that just finished.  No-op when the HTTP server is disabled
        // or the mission isn't loaded yet (each handler returns an
        // `Err` that's relayed back).
        let net = host.net.take();
        crate::http_server::drain_global(&mut manager, &mut host, &assets, net.as_ref());
        host.net = net;

        // ── Rollback check + rewind buffer commit ──
        // Both are post-tick bookkeeping.  Skipped on paused frames
        // (no tick ran) and rewind frames (tick was suppressed).  The
        // rewind buffer also skips commits while consuming its own
        // log — the slot is already populated and would duplicate.
        if !paused && !rewind_active {
            if let Some(ref mut checker) = rollback_checker {
                checker.end_frame(&mut host, frame_cmds.commands.clone(), &manager.engine);
            }
            if !consumed_buffered {
                rewind_buffer.end_frame(frame_cmds.commands.clone());
            }
            manager.sim_frame += 1;
            if let Some(net) = host.net.as_ref()
                && host.local_seat == robin_engine::player_command::PlayerId::HOST
            {
                net.set_initial_snapshot(manager.sim_frame, &manager.engine);
            }
        }

        // ── Pending `/step-forward` / `/step-back` requests ──
        // Run each queued step synchronously with its own tick +
        // bookkeeping (forward) or rewind-buffer seek (back).  These
        // requests intentionally bypass the `paused` gate — their whole
        // purpose is to drive the sim from a paused state — but still
        // refuse if a modal dialog is queued so the user doesn't step
        // past it.
        drain_steps(
            &mut manager,
            &mut host,
            assets.as_ref(),
            &mut dev,
            &mut game,
            &mut rewind_buffer,
            &mut rollback_checker,
            &mut replay_player,
            &mut manual_pause,
            &mut active_modal,
        );

        // Publish replay-playback status for the script-RPC `state`
        // endpoint so JS timelines can render a playhead.  `None`
        // when we're not replaying — the state response will carry
        // `null` for `replay`, the JS UI's "hide me" signal.
        crate::http_server::set_replay_status(replay_player.as_ref().map(|p| {
            crate::http_server::ReplayStatus {
                frame: p.current_frame(),
                total: p.total_frames(),
                paused: manual_pause,
            }
        }));

        // ── Keyboard-driven single-frame step (`.` / `,`) ──
        // Same bookkeeping as the HTTP `/step-forward` / `/step-back`
        // requests handled in `drain_steps`, but driven by the local
        // keybindings and without a network reply.  Refused while a
        // modal is pending for the same reason (stepping past a queued
        // dialog would skip it).
        //
        // During replay, the main per-frame replay advance is skipped
        // (gated on `!paused`) so the step handlers drive the replay
        // cursor themselves: forward pulls the next recorded commands
        // and applies them before the tick; back seeks the cursor to
        // the rewound frame so playback resumes from there.
        if step_forward_pressed && !modal_state_pending(&host) {
            let mut step_frame_cmds: Vec<PlayerInput> = Vec::new();
            if let Some(ref mut player) = replay_player
                && !player.is_finished()
            {
                let replay_cmds = player.next_frame();
                for cmd in replay_cmds {
                    if matches!(cmd.command, PlayerCommand::ModalDismiss { .. }) {
                        tracing::debug!(
                            "step-forward: dropping recorded ModalDismiss at frame {}",
                            manager.sim_frame
                        );
                        continue;
                    }
                    step_frame_cmds.push(cmd.clone());
                }
                manager.engine.apply_commands(
                    &mut host.engine_display,
                    &mut host.input,
                    &assets,
                    &step_frame_cmds,
                );
            }
            let mut display = std::mem::take(&mut host.engine_display);
            game.run_engine_tick(
                &mut host,
                &mut display,
                assets.as_ref(),
                &mut manager.engine,
                &mut dev,
                false,
                false,
            );
            host.engine_display = display;
            rewind_buffer.end_frame(step_frame_cmds);
            manager.sim_frame += 1;
            // Stepping bypasses the checker's begin_frame/end_frame
            // pairing, so its ring buffer is now stale relative to the
            // advanced engine.  Clear it — the checker resumes
            // populating on the next normal frame.
            if let Some(ref mut checker) = rollback_checker {
                checker.reset();
            }
        } else if step_back_pressed && !modal_state_pending(&host) {
            if let Some(target) = manager.sim_frame.checked_sub(1)
                && let Some(oldest) = rewind_buffer.oldest_reachable_frame()
                && target >= oldest
            {
                rewind_buffer.begin_session();
                let rewound = rewind_buffer.rewind_to(&assets, target);
                rewind_buffer.end_session();
                if let Some(new_engine) = rewound {
                    manager.engine = new_engine;
                    manager.sim_frame = target;
                    // Keep the replay cursor in sync with the rewound
                    // sim frame so resuming playback re-applies the
                    // right commands.
                    if let Some(ref mut player) = replay_player {
                        player.seek(target);
                    }
                    // The rollback checker's ring now references a
                    // timeline the live engine is no longer on; clear
                    // it so the next normal frame starts a fresh
                    // window.
                    if let Some(ref mut checker) = rollback_checker {
                        checker.reset();
                    }
                } else {
                    tracing::warn!("step-back: rewind_to({target}) failed");
                }
            } else {
                tracing::debug!("step-back: already at oldest retained frame");
            }
        }

        // ── Rebind shadow key on ambience change ──
        // The shadow key is baked into frame dictionaries at load
        // time and never re-run, which would leave loaded sprites
        // with a stale shadow key if the ambience ever changed
        // (day→fog, day→night). Poll the engine state's
        // `night_color` and re-run the shadow-dependent host
        // renderers on change. No current code path mutates
        // `weather.ambiance` post-load, so this is dormant until a
        // future weather/scripting feature wires a trigger.
        let current_shadow_color = manager.engine.weather().night_color;
        if current_shadow_color != last_shadow_color {
            tracing::info!(
                "Ambience shadow-key changed {:#06x} → {:#06x}; rebinding sprite caches",
                last_shadow_color,
                current_shadow_color,
            );
            host.frame_holder_mut().apply_arno_law(current_shadow_color);
            selection_mark_renderer.load(&mut cursor_res, &renderer, current_shadow_color);
            titbit_renderer.load(
                &mut cursor_res,
                &window.gpu,
                current_shadow_color,
                renderer.scale_mode(),
            );
            // Frame counts don't change on a shadow-key rebind — same
            // resource rows reloaded with a different shadow colour —
            // so the engine's `titbit_row_frame_counts` stays valid.
            last_shadow_color = current_shadow_color;
        }

        // ── Expand DisplayAll cheats ──
        // Console `LEVEL TEXT D/DB/PT` sets `dev.debug.all_*` bools.
        // The engine tick can't expand them because level descriptors
        // live host-side; we do the expansion here using the same
        // typed IDs the drain code below already understands.
        if dev.debug.all_dialogues {
            dev.debug.all_dialogues = false;
            if let Some(descriptors) = &level_descriptors {
                let count = descriptors.dialogues.len();
                host.pending_dialogues.extend((0..count).map(|i| i as i32));
            } else {
                tracing::warn!("cheat all_dialogues: level descriptors unavailable");
            }
        }
        if dev.debug.all_popup_texts {
            dev.debug.all_popup_texts = false;
            if let Some(descriptors) = &level_descriptors {
                let count = descriptors.popup_text.picture_ids.len();
                host.pending_popup_texts
                    .extend((0..count).map(|i| i as i32));
            } else {
                tracing::warn!("cheat all_popup_texts: level descriptors unavailable");
            }
        }
        if dev.debug.all_debriefings {
            dev.debug.all_debriefings = false;
            if let Some(descriptors) = &level_descriptors {
                let lose = descriptors.debriefing.lose_count as usize;
                let win = descriptors.debriefing.win_count as usize;
                host.pending_debriefings.extend(
                    (0..lose).map(
                        |index| robin_engine::player_command::DebriefingTextId::Lose { index },
                    ),
                );
                host.pending_debriefings
                    .extend((0..win).map(|index| {
                        robin_engine::player_command::DebriefingTextId::Win { index }
                    }));
            } else {
                tracing::warn!("cheat all_debriefings: level descriptors unavailable");
            }
        }

        if args.headless {
            drain_pending_dialogues(
                &mut host,
                &mut *window,
                &mut renderer,
                &mut cursor_res,
                &mut cursor_renderer,
                &mut audio_backend,
                &mut text_res,
                &game,
                &level_descriptors,
                &mut menu_resources,
                &mut replay_recorder,
                &mut replay_modal_dismissals,
                true,
            )
            .await;
        } else {
            if active_modal.is_none()
                && let Some(batch) = start_active_dialogue_batch(
                    &mut host,
                    &mut text_res,
                    &game,
                    &level_descriptors,
                    &mut replay_modal_dismissals,
                )
            {
                active_modal = Some(ActiveModal::Dialogue(Box::new(batch)));
            }
            if active_modal.is_some() {
                let outcome = tick_active_modal(
                    &mut active_modal,
                    &mut host,
                    &mut *window,
                    &mut renderer,
                    &mut cursor_res,
                    &mut cursor_renderer,
                    &mut audio_backend,
                    &sample_loader,
                    &mut menu_resources,
                    &mut replay_recorder,
                );
                debug_assert_eq!(outcome, ActiveModalOutcome::None);
                modal_rendered_this_frame = true;
            }
        }

        if !modal_rendered_this_frame && args.headless {
            drain_pending_popup_scroll(
                &mut host,
                &mut *window,
                &mut renderer,
                &mut cursor_res,
                &mut cursor_renderer,
                &mut audio_backend,
                &sample_loader,
                &mut text_res,
                &level_descriptors,
                &mut menu_resources,
                &mut replay_recorder,
                &mut replay_modal_dismissals,
                manager.engine.frame_counter(),
            )
            .await;
            drain_pending_sherwood_stat(
                &mut host,
                &mut *window,
                &mut renderer,
                &mut cursor_res,
                &mut cursor_renderer,
                &manager.engine,
                profiles,
                &mut audio_backend,
                &sample_loader,
                &mut menu_resources,
                &mut replay_recorder,
                &mut replay_modal_dismissals,
            )
            .await;
        } else if !modal_rendered_this_frame {
            if active_modal.is_none()
                && let Some(batch) = start_active_popup_scroll_batch(
                    &mut host,
                    &mut renderer,
                    &mut text_res,
                    &level_descriptors,
                    &mut menu_resources,
                    &mut replay_modal_dismissals,
                    manager.engine.frame_counter(),
                )
            {
                active_modal = Some(ActiveModal::PopupScroll(Box::new(batch)));
            }
            if active_modal.is_none()
                && let Some(batch) = start_active_sherwood_report(
                    &mut host,
                    &manager.engine,
                    profiles,
                    &mut menu_resources,
                    &mut replay_modal_dismissals,
                )
            {
                active_modal = Some(ActiveModal::PopupScroll(Box::new(batch)));
            }
            if active_modal.is_some() {
                let outcome = tick_active_modal(
                    &mut active_modal,
                    &mut host,
                    &mut *window,
                    &mut renderer,
                    &mut cursor_res,
                    &mut cursor_renderer,
                    &mut audio_backend,
                    &sample_loader,
                    &mut menu_resources,
                    &mut replay_recorder,
                );
                debug_assert_eq!(outcome, ActiveModalOutcome::None);
                modal_rendered_this_frame = true;
            }
        }

        if !modal_rendered_this_frame && args.headless {
            drain_pending_debriefings(
                &mut host,
                &mut *window,
                &mut renderer,
                &mut cursor_res,
                &mut cursor_renderer,
                &mut text_res,
                &level_descriptors,
                &menu_resources,
                &mut replay_recorder,
                &mut replay_modal_dismissals,
            )
            .await;
        } else if !modal_rendered_this_frame {
            if active_modal.is_none()
                && let Some(batch) = start_active_debriefing_batch(
                    &mut host,
                    &mut text_res,
                    &level_descriptors,
                    &menu_resources,
                    &mut replay_modal_dismissals,
                )
            {
                active_modal = Some(ActiveModal::Debriefing(Box::new(batch)));
            }
            if active_modal.is_some() {
                let outcome = tick_active_modal(
                    &mut active_modal,
                    &mut host,
                    &mut *window,
                    &mut renderer,
                    &mut cursor_res,
                    &mut cursor_renderer,
                    &mut audio_backend,
                    &sample_loader,
                    &mut menu_resources,
                    &mut replay_recorder,
                );
                debug_assert_eq!(outcome, ActiveModalOutcome::None);
                modal_rendered_this_frame = true;
            }
        }

        drain_pending_console_display(&mut host, &mut console_overlay);

        // First-time mission-won "leave mission now" banner
        // Blocks the main loop briefly to
        // show the popup; if the player confirms we kick the normal
        // quit-mission flow by queuing `SimpleMessage::QuitMission`
        // (the same path the quit-mission widget would have driven
        // before it was disabled by `Game::perform_hourglass_*`).
        if !modal_rendered_this_frame
            && (host.pending_mission_state_popup || active_modal.is_some())
        {
            if host.pending_mission_state_popup {
                host.pending_mission_state_popup = false;
                if args.headless {
                    let cmd = PlayerCommand::QuitMissionRequested;
                    dispatch_local_command(
                        &mut host,
                        &mut manager.engine,
                        &mut frame_cmds,
                        &assets,
                        &cmd,
                    );
                    frame_cmds.push(cmd);
                } else if let Some(resources) = menu_resources.as_ref() {
                    let kind = robin_engine::player_command::ModalKind::MissionState {
                        kind: robin_engine::player_command::MissionStateModalKind::LeaveMissionNow,
                    };
                    let replay_result = pop_matching_dismissal(&mut replay_modal_dismissals, &kind);
                    let message = resources
                        .menu_text
                        .get(crate::ingame_menu::resources::MT_MSG_LEAVE_MISSION_NOW);
                    let message_str = if message.is_empty() {
                        "You may leave the mission now.".to_string()
                    } else {
                        message
                    };
                    active_modal = Some(ActiveModal::MissionState {
                        kind,
                        state: crate::ingame_menu::MissionStatePopupState::new(
                            &renderer,
                            resources,
                            message_str,
                            true,
                            None,
                        ),
                        replay_result,
                    });
                }
            }

            if active_modal.is_some() {
                let outcome = tick_active_modal(
                    &mut active_modal,
                    &mut host,
                    &mut *window,
                    &mut renderer,
                    &mut cursor_res,
                    &mut cursor_renderer,
                    &mut audio_backend,
                    &sample_loader,
                    &mut menu_resources,
                    &mut replay_recorder,
                );
                modal_rendered_this_frame = true;
                if outcome == ActiveModalOutcome::QuitMissionRequested {
                    // Route through the command pipeline so replay /
                    // rollback reproduce the quit deterministically.
                    // The command sets `quit_won` when the mission
                    // is already marked won (our first-time-mission-
                    // won path) so the next tick returns
                    // `LevelSucceeded`.
                    let cmd = PlayerCommand::QuitMissionRequested;
                    dispatch_local_command(
                        &mut host,
                        &mut manager.engine,
                        &mut frame_cmds,
                        &assets,
                        &cmd,
                    );
                }
            }
        }

        // Drain zoom-deferred QuickSave / QuickLoad: pressing F9/F12
        // during an in-flight zoom is held until the transition
        // settles so we don't snapshot or overwrite a mid-zoom
        // engine.  Once `is_zoom_possible()` reports clear, enqueue
        // the same request the live key would have produced.
        if manager.engine.is_zoom_possible(&host.engine_display) {
            if game.quick_save_after_zoom {
                game.quick_save_after_zoom = false;
                let mission_id = manager
                    .engine
                    .campaign()
                    .map(|c| current_mission_id(c, &assets.profile_manager))
                    .unwrap_or(0);
                callbacks.pending =
                    Some(crate::main_entry::SaveLoadRequest::QuickSave { mission_id });
            }
            if game.quick_load_after_zoom {
                game.quick_load_after_zoom = false;
                // Shift state at drain time differs from press time;
                // re-read it on the deferred fire to keep the
                // shift-modifier semantics intact.
                callbacks.pending = Some(crate::main_entry::SaveLoadRequest::QuickLoad {
                    use_backup: shift_held,
                });
            }
        }

        // Drain `host.pending_reset_input` — set when the engine
        // consumed `SimpleMessage::ResetInput` during this tick,
        // which fires after a modal dialogue / popup / Sherwood
        // report closes.  Zeroes mouse/keyboard state so held-key
        // edges from the modal don't re-fire as gameplay actions.
        // Re-syncs the host cursor latches and clears the
        // InputTranslator's edge-detection buffer too so the next
        // `translate_keyboard` pass sees fresh edges.
        if host.pending_reset_input {
            host.pending_reset_input = false;
            threaded_input.reset_input_state();
            input_translator.reset_state();
            host.input.left_mouse_down = false;
            host.input.right_mouse_down = false;
            host.input.is_dragging = false;
            host.input.multi_selection_active = false;
            host.input.multi_unselection_active = false;
            host.input.draw_multi_selection = false;
        }

        if let Some(exit_code) = tick_exit_code {
            tracing::info!("Engine tick returned: {:?}", exit_code);

            // Apply quit-mission updates (stat sync, coma reset,
            // score bonuses, warcrime recruitment, blazon
            // consumption) before showing the debriefing so it
            // displays correct stats.  The engine internally
            // takes/restores its owned campaign.
            dispatch_local_command(
                &mut host,
                &mut manager.engine,
                &mut frame_cmds,
                &assets,
                &PlayerCommand::ApplyQuitMissionUpdates { exit_code },
            );

            // Show the mission state popup + debriefing synchronously
            // now, while the renderer and menu resources are still
            // alive.  `show_debriefing` blocks the loop until the
            // player dismisses it.
            if let (Some((popup_title, _popup_body)), Some(resources)) = (
                crate::ingame_menu::mission_state_text(exit_code),
                &menu_resources,
            ) {
                let won = exit_code == GameCode::LevelSucceeded;
                let mission_state_kind = robin_engine::player_command::ModalKind::MissionState {
                    kind: robin_engine::player_command::MissionStateModalKind::EndState { won },
                };
                let mission_state_result = match pop_matching_dismissal(
                    &mut replay_modal_dismissals,
                    &mission_state_kind,
                ) {
                    Some(
                        result @ (robin_engine::player_command::DialogResult::Completed
                        | robin_engine::player_command::DialogResult::Aborted),
                    ) => result,
                    Some(result) => {
                        tracing::warn!(
                            ?result,
                            "final mission-state replay result is only yes/no; treating as aborted"
                        );
                        robin_engine::player_command::DialogResult::Aborted
                    }
                    None => {
                        let cursor = Some(default_modal_cursor(
                            &mut cursor_renderer,
                            &mut cursor_res,
                            &mut renderer,
                        ));
                        let confirmed = crate::ingame_menu::show_mission_state_popup(
                            &mut *window,
                            &mut renderer,
                            resources,
                            cursor,
                            popup_title,
                            won,
                            None,
                        )
                        .await;
                        if confirmed {
                            robin_engine::player_command::DialogResult::Completed
                        } else {
                            robin_engine::player_command::DialogResult::Aborted
                        }
                    }
                };
                if let Some(recorder) = replay_recorder.as_mut() {
                    recorder.push(robin_engine::player_command::PlayerCommand::ModalDismiss {
                        kind: mission_state_kind,
                        result: mission_state_result,
                    });
                }
                // Resolve the per-mission debriefing prose from the
                // level's text resource table: pick win or lose
                // table_id depending on `won`, then look up the
                // string at `victory_defeat_id` (set by the
                // script-side `ChooseVictoryDefeatText`).  On any
                // failure, fall back to a placeholder so the body is
                // never empty.
                let debriefing_index = manager.engine.mission().victory_defeat_id as usize;
                let debriefing_kind = robin_engine::player_command::ModalKind::FinalDebriefing {
                    text_id: robin_engine::player_command::DebriefingTextId::from_outcome(
                        won,
                        debriefing_index,
                    ),
                };
                let debriefing_body = if let Some(descriptors) = level_descriptors.as_ref() {
                    let table_id = if won {
                        descriptors.debriefing.win_text_table_id
                    } else {
                        descriptors.debriefing.lose_text_table_id
                    };
                    match text_res.get_string(table_id, debriefing_index) {
                        Ok(s) => s.to_string(),
                        Err(e) => {
                            tracing::warn!(
                                "Debriefing text lookup failed (table={table_id}, \
                                 index={debriefing_index}): {e}"
                            );
                            "Invalid debriefing ID...".to_string()
                        }
                    }
                } else {
                    tracing::warn!("Debriefing text lookup: level descriptors unavailable");
                    "No dynamic resources for this level...".to_string()
                };
                // Feed the mission-stat panel through the mission-clock
                // abstraction. The current implementation returns the
                // deterministic campaign counter, which advances from
                // completed sim seconds.
                let mission_length = manager
                    .engine
                    .campaign()
                    .map(|c| {
                        <RustCallbacks as crate::game::GameCallbacks>::get_current_playing_time(
                            callbacks, c,
                        )
                    })
                    .unwrap_or(0);
                // When restart is allowed, the debriefing accepts a
                // QuickLoad keypress to short-circuit into a load.
                // Pull the configured `QuickLoad1` scancode out of
                // the input translator so the modal can fire on that
                // key.
                let quick_load_scancode = Some(
                    input_translator.get_binding(crate::input_translator::GameKey::QuickLoad1),
                );
                // Restart only fires when a restart snapshot exists.
                // When missing, the body window closes and the stat
                // panel still shows.  Probe the save manager up
                // front so the modal can short-circuit a no-snapshot
                // Restart click to "skip body, show stat".
                let restart_snapshot_exists =
                    restart_allowed && callbacks.save_manager.has_restart_save();
                let mission_id = manager
                    .engine
                    .campaign()
                    .map(|c| current_mission_id(c, &assets.profile_manager))
                    .unwrap_or(0);

                // Re-entry loop for the Load button: clicking Load
                // chains into the save-load picker; if the picker is
                // cancelled, the current debriefing page stays
                // visible and the player can continue interacting
                // with it.  We model that by re-entering
                // `show_debriefing` with the page text the player
                // was viewing when they clicked Load.
                let post_load_outcome = if let Some(result) =
                    pop_matching_dismissal(&mut replay_modal_dismissals, &debriefing_kind)
                {
                    final_debriefing_outcome_from_replay(result)
                } else {
                    let mut current_body = debriefing_body.clone();
                    let mut start_at_stat = false;
                    loop {
                        let cursor = Some(default_modal_cursor(
                            &mut cursor_renderer,
                            &mut cursor_res,
                            &mut renderer,
                        ));
                        let outcome = crate::ingame_menu::show_debriefing(
                            &mut *window,
                            &mut renderer,
                            resources,
                            cursor,
                            &current_body,
                            Some(manager.engine.mission_stat()),
                            mission_length,
                            won,
                            restart_allowed,
                            quick_load_scancode,
                            restart_snapshot_exists,
                            start_at_stat,
                        )
                        .await;
                        match outcome {
                            crate::ingame_menu::DebriefingOutcome::LoadAttempt {
                                body_remaining,
                                was_on_stat,
                            } => {
                                // Run the save-load picker.  If a slot is
                                // selected we'll re-enter the loop with a
                                // synthetic outcome below; otherwise we
                                // re-show the same page (body or stat).
                                let cursor = Some(default_modal_cursor(
                                    &mut cursor_renderer,
                                    &mut cursor_res,
                                    &mut renderer,
                                ));
                                let picker_outcome = crate::ingame_menu::show_save_load(
                                    &mut *window,
                                    &mut renderer,
                                    resources,
                                    cursor,
                                    &mut callbacks.save_manager,
                                    mission_id,
                                    Some(&assets.profile_manager),
                                    crate::ingame_menu::SaveLoadMode::Load,
                                    Some(&mut host.sound),
                                    audio_backend
                                        .as_mut()
                                        .map(|b| b as &mut dyn crate::sound::AudioBackend),
                                    Some(&*sample_loader),
                                )
                                .await;
                                match picker_outcome {
                                    crate::ingame_menu::SaveLoadOutcome::Slot(slot) => {
                                        // Synthesise a Load-resolved outcome and
                                        // exit the re-entry loop.  Stored in
                                        // `post_load_outcome` so the match
                                        // below processes it uniformly.
                                        break SettledDebriefingOutcome::Load { slot };
                                    }
                                    crate::ingame_menu::SaveLoadOutcome::Cancel => {
                                        // Picker cancelled — re-enter the
                                        // debriefing on the same page.
                                        current_body = body_remaining;
                                        start_at_stat = was_on_stat;
                                        continue;
                                    }
                                }
                            }
                            crate::ingame_menu::DebriefingOutcome::Ok { .. } => {
                                break SettledDebriefingOutcome::Ok;
                            }
                            crate::ingame_menu::DebriefingOutcome::Restart => {
                                break SettledDebriefingOutcome::Restart;
                            }
                            crate::ingame_menu::DebriefingOutcome::EmergencyEnd => {
                                break SettledDebriefingOutcome::EmergencyEnd;
                            }
                        }
                    }
                };
                if let Some(recorder) = replay_recorder.as_mut() {
                    recorder.push(robin_engine::player_command::PlayerCommand::ModalDismiss {
                        kind: debriefing_kind,
                        result: final_debriefing_result(&post_load_outcome),
                    });
                }

                // Wire the Load/Restart outcomes back into the game
                // state machine.  Both funnel through the engine's
                // save-game slot machinery rather than re-running
                // the mission cold.
                match post_load_outcome {
                    SettledDebriefingOutcome::Ok => {
                        // Normal dismissal — let the exit_code flow
                        // through the Game state machine on the next
                        // frame's `process_operation`.
                    }
                    SettledDebriefingOutcome::Restart => {
                        // We've already verified the restart snapshot
                        // exists via `restart_snapshot_exists`; queue
                        // `SaveLoadRequest::LoadRestart` and reset
                        // `game.operation` so the next frame's
                        // `perform_pending_save_load` applies it in
                        // place.
                        callbacks.pending = Some(crate::main_entry::SaveLoadRequest::LoadRestart);
                        game.operation.set(GameCode::LevelInProgress);
                    }
                    SettledDebriefingOutcome::Load { slot } => {
                        // The Load button chains into the save-load
                        // picker (run inline above) and queues a
                        // level load.
                        callbacks.pending = Some(crate::main_entry::SaveLoadRequest::Load {
                            slot: Some(slot),
                            mission_id,
                        });
                        game.operation.set(GameCode::LevelInProgress);
                    }
                    SettledDebriefingOutcome::EmergencyEnd => {
                        // External force-close (window close / Alt-
                        // F4) propagates as `GameCode::Quit` so
                        // `handle_quit` writes the continue-save and
                        // the outer session returns to the main
                        // menu.
                        if let Some(ref mut recorder) = replay_recorder
                            && !rewind_active
                            && !consumed_buffered
                        {
                            recorder.end_frame();
                        }
                        *campaign_ref = manager.engine.take_campaign().unwrap_or_default();
                        return Ok(GameCode::Quit);
                    }
                }
            }
        }

        // ── Commit the recorder frame ──
        // Deferred from the record block above so every modal drain,
        // including final mission-state/debriefing popups, can append
        // `ModalDismiss` entries to the same frame as the engine tick
        // that queued them.
        if let Some(ref mut recorder) = replay_recorder
            && !rewind_active
            && !consumed_buffered
        {
            recorder.end_frame();
        }
        // Warn if any recorded dismissals went unused — this should not
        // happen for a clean replay; if it does, the replay commands
        // have drifted out of sync with the engine's modal output.
        if !replay_modal_dismissals.is_empty() {
            tracing::warn!(
                "Replay: {} recorded ModalDismiss command(s) unused this frame",
                replay_modal_dismissals.len()
            );
        }

        // Flush any sound-mode / jingle / mouse intents queued by the
        // state machine (`game.process_operation`), the pause-menu input
        // handler, or script-triggered menus. Must run before the sound
        // hourglass so a fresh `set_mode(Mission)` immediately tees up
        // `load_music = true` before the tick.
        flush_pending_callbacks(
            &mut host,
            callbacks,
            &mut manager,
            &mut threaded_input,
            audio_backend
                .as_mut()
                .map(|b| b as &mut dyn crate::sound::AudioBackend),
        );

        // ── Sound tick ──
        // Combat/alert music transitions + sim-emitted sound drains.
        // See `tick_audio` for the breakdown.
        if let Some(backend) = audio_backend.as_mut() {
            tick_audio(
                &mut manager,
                &mut host,
                backend,
                &*sample_loader,
                &mut sound_rng,
            );
        }

        // ── Render dispatch ──
        // The display-state machine (display_op transitions, scrolling
        // deceleration, zoom interpolation, minimap transition) now runs
        // inside `perform_hourglass` so rollback replay re-runs the
        // same mutations. `last_skip_render` carries the
        // fast-forward "skip this frame" decision back to the host.
        // `--headless` forces the render block off for the entire
        // mission — same gate as the in-game fast-forward
        // `host.skip_render` toggle, just sticky.
        let draw_result = if host.skip_render || args.headless || modal_rendered_this_frame {
            1
        } else {
            0
        };

        if draw_result == 0 {
            pre_render_engine_setup(&mut manager, &mut host, assets.as_ref(), &mut renderer);
            update_mouse_and_cursor(
                &mut manager,
                &mut host,
                &assets,
                &dev,
                &mut renderer,
                &mut cursor_res,
                &mut cursor_renderer,
                &threaded_input,
                &portrait_cache,
                shift_held,
                &mut last_cursor_id,
            );

            let mut render_ctx = RenderContext {
                renderer: &mut renderer,
                cursor_renderer: &mut cursor_renderer,
                selection_mark_renderer: &mut selection_mark_renderer,
                titbit_renderer: &mut titbit_renderer,
                console_overlay: &mut console_overlay,
                zoom_tooltip: &mut zoom_tooltip,
                corner_tooltip: &mut corner_tooltip,
                requirements_tooltip: &mut requirements_tooltip,
                blazon_tooltip: &mut blazon_tooltip,
                stature_tooltip: &mut stature_tooltip,
                sherwood_tooltip: &mut sherwood_tooltip,
                pc_action_tooltip: &mut pc_action_tooltip,
                mouse_trail_renderer: mouse_trail_renderer.as_ref(),
                portrait_cache: &portrait_cache,
                menu_resources: menu_resources.as_ref(),
                hud_fonts: hud_fonts.as_ref(),
                short_briefing_strings: &short_briefing_strings,
                sherwood_layout: &sherwood_layout,
                sherwood_sprites: &sherwood_sprites,
                zoom_layout: &zoom_layout,
                zoom_sprites: &zoom_sprites,
                corner_layout: &corner_layout,
                corner_sprites: &corner_sprites,
                stature_layout: &stature_layout,
                stature_sprites: &stature_sprites,
                threaded_input: &threaded_input,
                game: &game,
                pause_menu: pause_menu.as_ref(),
                sherwood_enable,
                shift_held,
                rewind_active,
                display_info_elapsed_secs:
                    <RustCallbacks as crate::game::GameCallbacks>::get_current_playing_time(
                        callbacks,
                        campaign_ref,
                    ),
            };

            // Pending `/screenshot` requests: each renders a dedicated
            // throwaway frame with its own overridden dev flags into
            // the offscreen target, reads the pixels back, and clears
            // the target for the next render.  Runs BEFORE the live
            // frame so `present()` still blits the real frame last.
            let display_snapshot = host.engine_display.clone();
            drain_screenshots(
                &manager.engine,
                &display_snapshot,
                &mut host,
                &assets,
                &dev,
                &mut render_ctx,
            );

            if host.pending_print_screen == Some(PrintScreenRequest::WideSnapshot) {
                host.pending_print_screen = None;
                let display_snapshot = host.engine_display.clone();
                if !drain_wide_print_screen(
                    &manager.engine,
                    &display_snapshot,
                    &mut host,
                    &assets,
                    &dev,
                    &mut render_ctx,
                ) {
                    host.pending_print_screen = Some(PrintScreenRequest::Plain);
                }
            }

            let display_snapshot = host.engine_display.clone();
            render_frame(
                &manager.engine,
                &display_snapshot,
                &mut host,
                &assets,
                &dev,
                &mut render_ctx,
            );

            // PrintScreen keybind — capture the composited frame
            // after `render_frame` completes but before `present()`
            // resets the target.  Writes to
            // `<save-root>/screen%03u.png`, picking the first free
            // slot in `000..1000`.
            if let Some(request) = host.pending_print_screen.take() {
                drain_print_screen_request(render_ctx.renderer, request);
            }

            render_ctx.renderer.present();
            post_render_engine_cleanup(&mut manager, &mut host, &assets);
        } // end if draw_result == 0 (skip render in fast-forward)

        // Transient-message countdown: the render pass drew the
        // message for this frame if `message_delay` was non-zero;
        // tick down now so next frame sees one less frame remaining,
        // and drop the text when the counter reaches zero.  Runs
        // outside the render block so `ctx.game: &game` is out of
        // scope and we can mutably re-borrow `game`.
        if game.message_delay > 0 {
            game.message_delay -= 1;
            if game.message_delay == 0 {
                game.message_text.clear();
            }
        }

        // ── Frame timing (25 fps) ──
        // `--fast-forward` CLI flag skips the pacing sleep entirely so
        // the loop runs at full host speed (tests / profiling).  The
        // in-game fast-forward engine flag uses a 1 ms floor instead so
        // other host timers don't starve.
        let elapsed = crate::window::process_uptime_ms() - frame_start;
        let target = if args.fast_forward || args.headless {
            0
        } else if manager.engine.is_fast_forward() {
            1
        } else if host.slow_motion {
            // While SlowMotion is on (and neither console nor engine
            // fast-forward are active), each frame waits 40 * 10 ms.
            robin_engine::engine::FRAME_TIME_MS * 10
        } else {
            robin_engine::engine::FRAME_TIME_MS
        };
        let mut remaining_sleep_ms = target.saturating_sub(elapsed);
        if host.net.is_some()
            && host.local_seat != robin_engine::player_command::PlayerId::HOST
            && !args.fast_forward
            && !args.headless
            && let Some(desired_deadline_ms) =
                host_scheduled_frame_deadline_ms(mp_host_frame_schedule, manager.sim_frame)
        {
            let now_ms = crate::window::process_uptime_ms();
            let adjusted_sleep_ms = (desired_deadline_ms - i64::from(now_ms)).max(0) as u32;
            let correction_ms = i64::from(adjusted_sleep_ms) - i64::from(remaining_sleep_ms);
            if correction_ms != 0 && now_ms.saturating_sub(last_mp_sleep_correction_log_ms) >= 1000
            {
                last_mp_sleep_correction_log_ms = now_ms;
                tracing::info!(
                    scheduled_frame = mp_host_frame_schedule.map(|(frame, _)| frame),
                    local_frame = manager.sim_frame,
                    normal_sleep_ms = remaining_sleep_ms,
                    adjusted_sleep_ms,
                    correction_ms,
                    "multiplayer: adjusted frame sleep to host frame schedule"
                );
            }
            remaining_sleep_ms = adjusted_sleep_ms;
        }
        if let Some((hash_frame, hash)) = pending_mp_state_hash
            && let Some(net) = host.net.as_ref()
            && host.local_seat == robin_engine::player_command::PlayerId::HOST
        {
            net.publish_frame(manager.sim_frame);
            tracing::info!(
                hash_frame,
                clock_frame = manager.sim_frame,
                elapsed_ms = elapsed,
                target_ms = target,
                remaining_sleep_ms,
                "multiplayer: host sending state hash timing sample"
            );
            net.send_state_hash(hash_frame, hash, manager.sim_frame, remaining_sleep_ms);
        }
        if remaining_sleep_ms > 0 {
            crate::window::sleep_ms(remaining_sleep_ms as u64).await;
        }
    }
}
