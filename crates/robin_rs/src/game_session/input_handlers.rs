//! Gamepad / hold-to-rewind / console-overlay per-frame input handlers.

use super::{apply_local_viewport_scroll, dispatch_local_command};
use crate::Host;
use crate::gfx_types::GameEvent;
use crate::player_command::{FrameCommands, PlayerCommand};
use robin_engine::engine::Engine;

/// Translate the per-frame SDL3 controller events into joystick
/// state, then dispatch. The gamepad's in-flight state persists
/// across frames — events are sparse (only changed axes/buttons
/// report) — so we only mutate the fields each event touches.
#[allow(clippy::too_many_arguments)]
pub(super) fn handle_gamepad_events(
    host: &mut Host,
    manager: &mut robin_engine::engine_manager::EngineManager,
    assets: &robin_engine::engine::LevelAssets,
    threaded_input: &mut crate::input::ThreadedInput,
    frame_cmds: &mut FrameCommands,
    events: &[GameEvent],
    active_gamepad: &mut Option<u32>,
) {
    // ── Gamepad event folding ──
    //
    // Translate the per-frame SDL3 controller events into joystick
    // state, then dispatch. The gamepad's in-flight state persists
    // across frames — events are sparse (only changed axes/buttons
    // report) — so we only mutate the fields each event touches.
    //
    // SDL3 delivers the D-pad as four separate buttons.  Latch the
    // current D-pad button states after each event and fold them
    // into `povs[0]`.
    let mut dpad = [false; 4]; // [up, right, down, left]
    for event in events {
        match event {
            // gilrs has already opened the device — just promote it
            // to active if no other gamepad holds the slot.
            GameEvent::GamepadAdded { which } if active_gamepad.is_none() => {
                *active_gamepad = Some(*which);
            }
            GameEvent::GamepadRemoved { which } => {
                if active_gamepad.map(|id| id == *which).unwrap_or(true) {
                    *active_gamepad = None;
                }
                // Reset the state so any held buttons release cleanly.
                host.gamepad = crate::gamepad::GamePadState::default();
                dpad = [false; 4];
            }
            GameEvent::GamepadAxis { axis, value, .. } => {
                host.gamepad.apply_axis_event(*axis, *value);
            }
            GameEvent::GamepadButton {
                button, pressed, ..
            } => {
                if crate::gamepad::is_dpad_button(*button) {
                    let idx = (*button - 11) as usize; // 11=Up, 12=Down, 13=Left, 14=Right
                    // Reorder to [up, right, down, left] for apply_dpad_state.
                    let slot = match idx {
                        0 => 0, // Up
                        1 => 2, // Down
                        2 => 3, // Left
                        3 => 1, // Right
                        _ => continue,
                    };
                    dpad[slot] = *pressed;
                    host.gamepad
                        .apply_dpad_state(dpad[0], dpad[1], dpad[2], dpad[3]);
                } else if let Some(idx) = crate::gamepad::sdl_button_to_gamepad_index(*button) {
                    host.gamepad.apply_button_event(idx, *pressed);
                }
            }
            _ => {}
        }
    }

    // ── Gamepad dispatch ──
    let now_ms = crate::window::process_uptime_ms();
    let gamepad_frame = host
        .gamepad
        .process_gamepad_input(now_ms, &manager.engine, threaded_input);
    for cmd in &gamepad_frame.viewport {
        match cmd {
            crate::gamepad::ViewportCommand::Scroll(dir) => apply_local_viewport_scroll(host, *dir),
            crate::gamepad::ViewportCommand::ZoomIn => {
                let mp = threaded_input.position();
                host.viewport.zoom_by(
                    2.0,
                    Some(robin_engine::coordinates::ScreenPoint::new(mp.x, mp.y)),
                );
            }
            crate::gamepad::ViewportCommand::ZoomOut => {
                let mp = threaded_input.position();
                host.viewport.zoom_by(
                    0.5,
                    Some(robin_engine::coordinates::ScreenPoint::new(mp.x, mp.y)),
                );
            }
        }
    }
    for cmd in &gamepad_frame.cmds {
        dispatch_local_command(host, &mut manager.engine, frame_cmds, assets, cmd);
    }
    if let Some(qa_event) = gamepad_frame.qa {
        let cmd = match qa_event {
            crate::gamepad::QaEvent::ToggleRecording => {
                if manager.engine.is_recording_macro() {
                    PlayerCommand::StopRecordingMacro
                } else {
                    let slot = super::mouse_input::choose_recording_place(
                        &manager.engine,
                        host.local_seat,
                    );
                    PlayerCommand::StartRecordingMacro { pc: None, slot }
                }
            }
            crate::gamepad::QaEvent::LaunchAllMacros => {
                PlayerCommand::StartMacro { pc: None, slot: 0 }
            }
            crate::gamepad::QaEvent::LaunchMacroForSelected => {
                let Some(&pc) = manager.engine.seat_selection(host.local_seat).first() else {
                    return;
                };
                PlayerCommand::StartMacro {
                    pc: Some(pc),
                    slot: 0,
                }
            }
        };
        dispatch_local_command(host, &mut manager.engine, frame_cmds, assets, &cmd);
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
    manager: &mut robin_engine::engine_manager::EngineManager,
    assets: &robin_engine::engine::LevelAssets,
    threaded_input: &crate::input::ThreadedInput,
    rewind_buffer: &mut crate::rewind::RewindBuffer,
    rollback_checker: &mut Option<crate::rollback_checker::RollbackChecker>,
    replay_player: &mut Option<crate::replay::ReplayPlayer>,
) -> bool {
    let engine = &mut manager.engine;
    let sim_frame = &mut manager.sim_frame;
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
        rewind_buffer.begin_session();
    } else {
        rewind_buffer.end_session();
    }
    let mut rewind_active = false;
    if rewind_held
        && *sim_frame > 0
        && rewind_buffer
            .oldest_reachable_frame()
            .is_some_and(|f| f < *sim_frame)
    {
        let target = *sim_frame - 1;
        if let Some(new_engine) = rewind_buffer.rewind_to(assets, target) {
            *engine = new_engine;
            *sim_frame = target;
            // Keep the replay cursor in lockstep with the rewound
            // sim frame so resuming playback re-applies the right
            // recorded commands.
            if let Some(player) = replay_player.as_mut() {
                player.seek(target);
            }
            rewind_active = true;
            // The rollback checker's ring history is now "ahead"
            // of the live engine and would false-positive on the
            // first normal frame after rewind releases.  Clear it
            // so it starts fresh.
            if let Some(checker) = rollback_checker {
                checker.reset();
            }
            tracing::trace!("Rewind → frame {target}");
        }
    }
    rewind_active
}

/// Handle the in-game console overlay's per-frame event dispatch:
/// feed events through the console, drain auto-close / CAMPAIGN load
/// requests, toggle SDL text input on visibility transitions.
///
/// When visible, the console captures keyboard events so they
/// don't leak into the game (typing "FREEZE" mustn't trigger
/// selection / movement actions).  Mouse events still pass
/// through so the player can pan/click while the console is
/// up — the game keeps running underneath.
#[allow(clippy::too_many_arguments)]
pub(super) fn handle_console_overlay_events(
    console_overlay: &mut crate::console_overlay::ConsoleOverlay,
    engine: &mut Engine,
    assets: &robin_engine::engine::LevelAssets,
    host: &mut Host,
    dev: &mut robin_engine::engine::DevState,
    events: &[GameEvent],
    kb_actions: &[crate::input_translator::GameAction],
    input_translator: &mut crate::input_translator::InputTranslator,
) {
    // ── In-game console overlay event handling ──
    // When visible, the console captures keyboard events so they
    // don't leak into the game (typing "FREEZE" mustn't trigger
    // selection / movement actions).  Mouse events still pass
    // through so the player can pan/click while the console is
    // up — the game keeps running underneath.
    console_overlay.handle_events(events, engine, assets, host, dev);
    // Auto-close after WIN / WINCAMPAIGN / LOSE — same as the cheat
    // `mbCommandTerminateConsole`.  Drains the pending flag so
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
        match crate::save_file::GameSaveFile::read_from(&path) {
            Ok(loaded) => match loaded.engine.campaign().cloned() {
                Some(campaign) => {
                    engine.install_campaign(campaign);
                    tracing::info!("Loaded campaign values from {}", path.display());
                    // Echo the success message into the console.
                    host.pending_console_output
                        .push("Campaign values loaded !".to_string());
                }
                None => {
                    tracing::error!(
                        "CAMPAIGN load: save file {} has no active campaign",
                        path.display(),
                    );
                    // The save header was readable but carried no
                    // campaign.  Fall through to the same error
                    // wording as the open-failure case — nothing
                    // useful was applied.
                    host.pending_console_output.push("Kaputt !".to_string());
                }
            },
            Err(err) => {
                tracing::error!("CAMPAIGN load failed for {}: {err:#}", path.display());
                // Echo the open-failure message into the console.
                host.pending_console_output.push("Kaputt !".to_string());
            }
        }
    }
    // Detect any visibility change (open via action below, close
    // via Esc / `~` / auto-close) so we toggle SDL text input.
    let console_visible_now = console_overlay.is_visible();
    let display_console_pressed = kb_actions
        .iter()
        .any(|a| matches!(a, crate::input_translator::GameAction::DisplayConsole));
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
            engine.send_simple_message(robin_engine::messenger::SimpleMessage::HideConsole);
        }
    }
}
