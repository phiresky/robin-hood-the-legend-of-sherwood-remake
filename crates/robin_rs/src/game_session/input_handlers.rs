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
#[allow(clippy::too_many_arguments)]
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
    let gamepad_frame = device.process(now_ms, &manager.engine, threaded_input);
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
                        host.transport.local_seat,
                    );
                    PlayerCommand::StartRecordingMacro { pc: None, slot }
                }
            }
            QaEvent::LaunchAllMacros => PlayerCommand::StartMacro { pc: None, slot: 0 },
            QaEvent::LaunchMacroForSelected => {
                let Some(&pc) = manager
                    .engine
                    .hero_selection(host.transport.local_seat)
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
#[allow(clippy::too_many_arguments)]
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
        timeline.rewind_buffer.begin_session();
    } else {
        timeline.rewind_buffer.end_session();
    }
    let mut rewind_active = false;
    if rewind_held
        && timeline.current_frame().previous().is_some()
        && timeline
            .rewind_buffer
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

/// Handle the in-game console overlay's per-frame event dispatch:
/// feed events through the console, drain auto-close / CAMPAIGN load
/// requests, reset text input state on visibility transitions.
///
/// When visible, the console captures keyboard events so they
/// don't leak into the game (typing "FREEZE" mustn't trigger
/// selection / movement actions).  Mouse events still pass
/// through so the player can pan/click while the console is
/// up — the game keeps running underneath.
#[allow(clippy::too_many_arguments)]
pub(super) fn handle_console_overlay_events(
    console_overlay: &mut ConsoleOverlay,
    engine: &mut Engine,
    assets: &engine_api::LevelAssets,
    host: &mut Host,
    dev: &mut engine_api::DevState,
    events: &[GameEvent],
    kb_actions: &[GameAction],
    input_translator: &mut InputTranslator,
    frame: &mut super::runtime::MissionFrame,
) {
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
                    .pending_console_output
                    .push("Campaign values loaded !".to_string());
            }
            Err(err) => {
                tracing::error!("CAMPAIGN load failed for {}: {err:#}", path.display());
                // Echo the open-failure message into the console.
                host.frontend
                    .pending_console_output
                    .push("Kaputt !".to_string());
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
    if display_console_pressed && !auto_closed {
        // The toggle key reached us via the action stream — flip.
        // The action fires on key-release (`key_released`), so when
        // the console is already visible the KeyDown is swallowed by
        // `handle_events` (so the toggle key doesn't insert as text)
        // while the corresponding release-edge still flips the
        // overlay closed.
        console_should_be_visible = console_overlay.toggle();
    }
    if console_should_be_visible != console_visible_now || auto_closed {
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
            frame
                .external_actions
                .push(robin_engine::engine::ExternalAction::SimpleMessage {
                    message: engine_messenger::SimpleMessage::HideConsole,
                });
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
        let mut assets = engine_api::LevelAssets::default();
        let engine = Engine::new_for_test(640.0, 480.0, Default::default(), &mut assets).unwrap();
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
