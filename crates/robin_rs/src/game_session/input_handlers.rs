//! Gamepad / hold-to-rewind / console-overlay per-frame input handlers.

use super::{apply_local_viewport_scroll, dispatch_local_command};
use crate::console_overlay::ConsoleOverlay;
use crate::gamepad::{GamepadDeviceInput, QaEvent, ViewportCommand};
use crate::gfx_types::GameEvent;
use crate::host::Host;
use crate::input::ThreadedInput;
use crate::input_translator::{GameAction, InputTranslator};
use crate::save_file::GameSaveFile;
use robin_engine::coordinates as engine_coordinates;
use robin_engine::engine as engine_api;
use robin_engine::engine::Engine;
use robin_engine::engine_manager as engine_manager_api;
use robin_engine::messenger as engine_messenger;
use robin_engine::player_command::{FrameCommands, PlayerCommand};

/// Translate persistent physical input only while gameplay owns the input
/// surface. Suspension retains held hardware state but emits no world effects.
pub(super) fn handle_gamepad_events(
    host: &mut Host,
    manager: &engine_manager_api::EngineManager,
    threaded_input: &mut ThreadedInput,
    frame_cmds: &mut FrameCommands,
    device: &mut GamepadDeviceInput,
    gameplay_allowed: bool,
) {
    if !device.admit_gameplay(gameplay_allowed) {
        return;
    }
    let now_ms = crate::window::process_uptime_ms();
    let gamepad_frame = device.process(
        now_ms,
        &manager.engine,
        host.transport.local_seat(),
        threaded_input,
    );
    for cmd in &gamepad_frame.viewport {
        match cmd {
            ViewportCommand::Scroll(dir) => apply_local_viewport_scroll(host, *dir),
            ViewportCommand::ZoomIn => {
                let mp = threaded_input.position();
                host.frontend
                    .viewport
                    .zoom_by(2.0, Some(engine_coordinates::ScreenPoint::new(mp.x, mp.y)));
            }
            ViewportCommand::ZoomOut => {
                let mp = threaded_input.position();
                host.frontend
                    .viewport
                    .zoom_by(0.5, Some(engine_coordinates::ScreenPoint::new(mp.x, mp.y)));
            }
        }
    }
    for cmd in &gamepad_frame.cmds {
        dispatch_local_command(&host.transport, frame_cmds, cmd);
    }
    if let Some(qa_event) = gamepad_frame.qa {
        let cmd = match qa_event {
            QaEvent::ToggleRecording => {
                if manager.engine.is_recording_macro() {
                    PlayerCommand::StopRecordingMacro
                } else {
                    let slot = super::mouse_input::choose_recording_place(
                        &manager.engine,
                        host.transport.local_seat(),
                    );
                    PlayerCommand::StartRecordingMacro { pc: None, slot }
                }
            }
            QaEvent::LaunchAllMacros => PlayerCommand::StartMacro { pc: None, slot: 0 },
            QaEvent::LaunchMacroForSelected => {
                let Some(&pc) = manager
                    .engine
                    .hero_selection(host.transport.local_seat())
                    .first()
                else {
                    return;
                };
                PlayerCommand::StartMacro {
                    pc: Some(pc),
                    slot: 0,
                }
            }
        };
        dispatch_local_command(&host.transport, frame_cmds, &cmd);
    }
}

/// Process the hold-to-rewind debug feature for this frame.
///
/// Holding BACKSPACE swaps the live sim state with a
/// reconstruction of `sim_frame - 1` from the rewind buffer,
/// decrements `sim_frame`, and skips the frame's input
/// processing + tick.  The renderer still runs, so visually
/// the game plays in reverse at full 25 fps.  Disabled during
/// replay playback (which owns the command stream) and when
/// the buffer hasn't accumulated any history yet.
///
/// Returns `true` when a rewind step fired this frame.
pub(super) fn handle_hold_to_rewind(
    manager: &mut engine_manager_api::EngineManager,
    assets: &engine_api::LevelAssets,
    threaded_input: &ThreadedInput,
    timeline: &mut super::runtime::TimelineRuntime,
) -> bool {
    // ── Hold-to-rewind debug feature ──
    // Holding BACKSPACE swaps the live sim state with a
    // reconstruction of `sim_frame - 1` from the rewind buffer,
    // decrements `sim_frame`, and skips the frame's input
    // processing + tick.  The renderer still runs, so visually
    // the game plays in reverse at full 25 fps.  Disabled during
    // replay playback (which owns the command stream) and when
    // the buffer hasn't accumulated any history yet.
    //
    let rewind_held = threaded_input
        .keyboard_state()
        .is_pressed(winit::keyboard::KeyCode::Backspace);
    // Edge detection: open/close the rewind-session cache so
    // consecutive rewind steps reuse earlier replay work instead
    // of re-ticking from a snapshot each frame.
    if rewind_held {
        timeline.history_mut().begin_rewind_session();
    } else {
        timeline.history_mut().end_rewind_session();
    }
    let mut rewind_active = false;
    if rewind_held
        && timeline.current_frame().previous().is_some()
        && timeline
            .history()
            .buffer()
            .oldest_reachable_frame()
            .is_some_and(|f| f < timeline.frame_number())
    {
        let target = timeline
            .current_frame()
            .previous()
            .expect("frame zero was excluded above");
        if timeline.restore_retained_frame(manager, assets, target) {
            rewind_active = true;
            tracing::trace!("Rewind → frame {}", target.number());
        }
    }
    rewind_active
}

/// This frame's polled events and translated keyboard actions, as seen by
/// the console overlay.
pub(super) struct ConsoleOverlayInput<'a> {
    pub(super) events: &'a [GameEvent],
    pub(super) kb_actions: &'a [GameAction],
}

/// Handle the in-game console overlay's per-frame event dispatch:
/// feed events through the console, drain auto-close / CAMPAIGN load
/// requests, reset text input state on visibility transitions.
///
/// `ConsoleOverlayInput` carries this frame's polled events and translated
/// keyboard actions.
///
/// When visible, the console captures keyboard events so they
/// don't leak into the game (typing "FREEZE" mustn't trigger
/// selection / movement actions).  Mouse events still pass
/// through so the player can pan/click while the console is
/// up — the game keeps running underneath.
pub(super) fn handle_console_overlay_events(
    console_overlay: &mut ConsoleOverlay,
    engine: &mut Engine,
    assets: &engine_api::LevelAssets,
    host: &mut Host,
    dev: &mut engine_api::DevState,
    input: ConsoleOverlayInput<'_>,
    input_translator: &mut InputTranslator,
    frame: &mut super::runtime::MissionFrame,
) {
    let ConsoleOverlayInput { events, kb_actions } = input;
    let was_visible = console_overlay.is_visible();
    // ── In-game console overlay event handling ──
    // When visible, the console captures keyboard events so they
    // don't leak into the game (typing "FREEZE" mustn't trigger
    // selection / movement actions).  Mouse events still pass
    // through so the player can pan/click while the console is
    // up — the game keeps running underneath.
    let mut admitted_actions = Vec::new();
    console_overlay.handle_events(events, engine, assets, host, dev, &mut admitted_actions);
    for action in admitted_actions {
        frame.record_applied_external_action(action);
    }
    // Auto-close after WIN / WINCAMPAIGN / LOSE — same as the cheat
    // the console-termination command flag. Drains the pending flag so
    // we only act once.
    let auto_closed = console_overlay.take_pending_close();

    // Deity easter egg: once the console signals `DeityInvoked`, apply
    // the host-owned input-translator rebind.
    if console_overlay.take_pending_deity_invoked() {
        input_translator.deity_call();
    }

    // `CAMPAIGN <path>` console command hands the host a save-file
    // path to load.  The engine can't touch the filesystem, so it
    // stashes the request on the overlay; we drain it here.
    //
    // The cheat reads only the campaign progress out of the save
    // file — engine/actor state is untouched.  Parse the full save
    // (cheap — JSON), extract the campaign, and assign it onto the
    // engine.
    if let Some(path) = console_overlay.take_pending_load_campaign() {
        match GameSaveFile::read_from(&path) {
            Ok(loaded) => {
                let action = robin_engine::engine::ExternalAction::ReplaceCampaign {
                    campaign: loaded.engine.campaign().clone(),
                };
                engine
                    .advance_frame(
                        assets,
                        robin_engine::engine::SimulationFrameInput::no_hourglass()
                            .with_external_actions(vec![action.clone()]),
                    )
                    .unwrap_or_else(|error| panic!("console campaign admission failed: {error}"));
                frame.record_applied_external_action(action);
                tracing::info!("Loaded campaign values from {}", path.display());
                host.frontend
                    .diagnostics_mut()
                    .queue_console_output("Campaign values loaded !".to_string());
            }
            Err(err) => {
                tracing::error!("CAMPAIGN load failed for {}: {err:#}", path.display());
                // Echo the open-failure message into the console.
                host.frontend
                    .diagnostics_mut()
                    .queue_console_output("Kaputt !".to_string());
            }
        }
    }
    // Detect any visibility change (open via action below, close
    // via Esc / `~` / auto-close) so we reset text input state.
    let console_visible_now = console_overlay.is_visible();
    let display_console_pressed = kb_actions
        .iter()
        .any(|a| matches!(a, GameAction::DisplayConsole));
    let mut console_should_be_visible = console_visible_now;
    let open_chat = !was_visible
        && events.iter().any(|event| {
            matches!(
                event,
                GameEvent::KeyDown {
                    keycode: crate::gfx_types::Keycode::Return | crate::gfx_types::Keycode::KpEnter,
                    ..
                }
            )
        });
    if (display_console_pressed || open_chat) && !auto_closed {
        // The toggle key reached us via the action stream — flip.
        // The action fires on key-release (`key_released`), so when
        // the console is already visible the KeyDown is swallowed by
        // `handle_events` (so the toggle key doesn't insert as text)
        // while the corresponding release-edge still flips the
        // overlay closed.
        console_should_be_visible = console_overlay.toggle();
    }
    if console_should_be_visible != was_visible || auto_closed {
        if console_should_be_visible {
            crate::window::start_text_input();
        } else {
            crate::window::stop_text_input();
            // Hiding the console emits a hide-console message whose
            // post-process resets input state, so held-key edges
            // typed into the overlay don't bleed into the game.
            // Route through the engine messenger so the drain handler
            // applies the reset symmetrically for any future
            // open→close path.
            frame.stage_external_actions().push(
                robin_engine::engine::ExternalAction::SimpleMessage {
                    message: engine_messenger::SimpleMessage::HideConsole,
                },
            );
        }
    }
}

#[cfg(test)]
mod gamepad_admission_tests {
    use super::*;

    #[test]
    fn modal_controller_input_cannot_move_viewport_or_admit_commands() {
        let mut host = Host::scratch(640.0, 480.0);
        host.frontend.viewport.view_position = engine_coordinates::MapPoint::new(400.0, 300.0);
        let position = host.frontend.viewport.view_position;
        let zoom = host.frontend.viewport.zoom_factor;
        let (engine, _assets) = robin_engine::test_support::fresh_engine_sized(640.0, 480.0);
        let manager = engine_manager_api::EngineManager::new(engine);
        let mut input = ThreadedInput::new();
        let mut commands = FrameCommands::new();
        let mut device = GamepadDeviceInput::default();
        for button in [11, 14, 0, 1, 4] {
            device.fold(&GameEvent::GamepadButton {
                which: 1,
                button,
                pressed: true,
            });
        }
        for _ in 0..3 {
            handle_gamepad_events(
                &mut host,
                &manager,
                &mut input,
                &mut commands,
                &mut device,
                false,
            );
        }
        assert_eq!(host.frontend.viewport.view_position, position);
        assert_eq!(host.frontend.viewport.zoom_factor, zoom);
        assert!(commands.commands.is_empty());
        assert!(input.drain_synthetic_events().is_empty());
    }
}

pub(super) fn handle_local_gamepads(
    host: &mut Host,
    engine: &Engine,
    _input: &mut ThreadedInput,
    assets: &engine_api::LevelAssets,
    commands: &mut FrameCommands,
    players: &mut crate::gamepad::LocalPlayers,
    allowed: bool,
) {
    use robin_engine::player_command::{PlayerId, PlayerInput};
    host.frontend.local_player_count = players.count();
    host.frontend.local_keyboard_player = players.keyboard;
    for (index, (_, device)) in players.devices.iter_mut().enumerate() {
        let seat = PlayerId((index + usize::from(players.keyboard)) as u8);
        if !device.is_connected() {
            if host.frontend.local_disconnected.insert(seat.0) {
                host.frontend
                    .diagnostics_mut()
                    .queue_console_output(format!(
                        "Player {} controller disconnected; press A to reconnect",
                        seat.0 + 1
                    ));
                host.frontend.local_cursors.remove(&seat.0);
            }
            if engine.seat(seat).is_some_and(|state| state.connected) {
                commands
                    .commands
                    .push(PlayerInput::host(PlayerCommand::DisconnectSeat {
                        player_id: seat,
                    }));
            }
            continue;
        }
        if host.frontend.local_disconnected.remove(&seat.0) {
            host.frontend
                .diagnostics_mut()
                .queue_console_output(format!("Player {} controller reconnected", seat.0 + 1));
        }
        if !device.admit_gameplay(allowed && !engine.user_locked()) {
            continue;
        }
        if engine.seat(seat).is_none_or(|state| !state.connected) && seat != PlayerId::HOST {
            host.frontend
                .diagnostics_mut()
                .queue_console_output(format!("Player {} connected", seat.0 + 1));
            commands
                .commands
                .push(PlayerInput::host(PlayerCommand::ConnectSeat {
                    player_id: seat,
                    nickname: format!("Player {}", seat.0 + 1),
                }));
        }
        let view = host
            .frontend
            .split_screen
            .views
            .iter()
            .find(|view| view.members.contains(&seat.0))
            .cloned();
        let viewport = view
            .as_ref()
            .map(|view| view.viewport(&host.frontend.viewport))
            .unwrap_or_else(|| host.frontend.viewport.clone());
        let center = view
            .as_ref()
            .map(|view| view.site)
            .unwrap_or([viewport.screen_size.x * 0.5, viewport.screen_size.y * 0.5]);
        let cursor = host
            .frontend
            .local_cursors
            .get(&seat.0)
            .copied()
            .unwrap_or(center);
        let mut input = ThreadedInput::new();
        input.reach_position(engine_coordinates::ScreenPoint::new(cursor[0], cursor[1]));
        input.drain_synthetic_events();
        let output = device.process(crate::window::process_uptime_ms(), engine, seat, &mut input);
        let point = input.position();
        let point = view
            .as_ref()
            .map(|view| view.clamp_cursor([point.x, point.y]))
            .unwrap_or([
                point.x.clamp(0., viewport.screen_size.x - 1.),
                point.y.clamp(0., viewport.screen_size.y - 1.),
            ]);
        host.frontend.local_cursors.insert(seat.0, point);
        if let Some(mouse_map) =
            viewport.screen_to_map(engine_coordinates::ScreenPoint::new(point[0], point[1]))
        {
            commands.commands.push(PlayerInput::new(
                seat,
                PlayerCommand::PerformOrientation { mouse_map },
            ));
        }
        if input
            .drain_synthetic_events()
            .iter()
            .any(|event| matches!(event, GameEvent::MouseUp(_, _, 1)))
        {
            if let Some(map) =
                viewport.screen_to_map(engine_coordinates::ScreenPoint::new(point[0], point[1]))
            {
                let primary_input = std::mem::take(&mut host.frontend.input);
                crate::host_mouse::publish_mouse_spatial_hit_for_seat(
                    engine,
                    host,
                    map,
                    Default::default(),
                    seat,
                );
                let clicks = crate::game_input::resolve_left_click_for_seat(
                    host,
                    engine,
                    assets,
                    map,
                    Default::default(),
                    seat,
                );
                host.frontend.input = primary_input;
                commands.commands.extend(
                    clicks
                        .into_iter()
                        .map(|command| PlayerInput::new(seat, command)),
                );
            }
        }
        commands.commands.extend(
            output
                .cmds
                .into_iter()
                .map(|command| PlayerInput::new(seat, command)),
        );
        if let Some(event) = output.qa {
            let leader = engine.hero_selection(seat).first().copied();
            let command = match event {
                QaEvent::ToggleRecording if engine.is_recording_macro() => {
                    PlayerCommand::StopRecordingMacro
                }
                QaEvent::ToggleRecording => PlayerCommand::StartRecordingMacro {
                    pc: leader,
                    slot: 0,
                },
                QaEvent::LaunchMacroForSelected => PlayerCommand::StartMacro {
                    pc: leader,
                    slot: 0,
                },
                QaEvent::LaunchAllMacros => PlayerCommand::StartMacro { pc: None, slot: 0 },
            };
            commands.commands.push(PlayerInput::new(seat, command));
        }
    }
}
