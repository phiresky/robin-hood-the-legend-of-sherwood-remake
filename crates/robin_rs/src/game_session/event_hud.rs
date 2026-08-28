//! Window-event collection and direct HUD dispatch for one graphical frame.
//!
//! The helper deliberately stops before general gameplay actions. Its output is
//! an owned event/action batch, so the caller can preserve the original order:
//! HUD collection first, host-only view input second, live simulation dispatch
//! third.

use super::interactive::{
    MissionHud, MissionInput, MissionPresentation, MissionResources, MissionUi,
};
use super::*;
use crate::game::Game;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct InputModifiers {
    pub(super) ctrl: bool,
    pub(super) shift: bool,
    pub(super) alt: bool,
}

/// Owned input data handed from event/HUD collection to the live-action phase.
pub(super) struct CollectedFrameInput {
    pub(super) events: Vec<GameEvent>,
    pub(super) keyboard_actions: Vec<GameAction>,
    pub(super) mouse_actions: Vec<GameAction>,
    pub(super) modifiers: InputModifiers,
    pub(super) minimap_toggle_pressed: bool,
    pub(super) pause_closed_this_frame: bool,
    pub(super) rewind_active: bool,
    pub(super) step_forward_pressed: bool,
    pub(super) step_back_pressed: bool,
}

pub(super) enum EventHudOutcome {
    Ready(CollectedFrameInput),
    Control(HandlerAction),
}

/// Explicit mutable inputs for the event/HUD collection boundary.
pub(super) struct EventHudContext<'a> {
    pub(super) host: &'a mut Host,
    pub(super) manager: &'a mut robin_engine::engine_manager::EngineManager,
    pub(super) game: &'a mut Game,
    pub(super) assets: &'a robin_engine::engine::LevelAssets,
    pub(super) dev: &'a mut robin_engine::engine::DevState,
    pub(super) callbacks: &'a mut RustCallbacks,
    pub(super) window: &'a mut GameWindow,
    pub(super) presentation: &'a mut MissionPresentation,
    pub(super) resources: &'a mut MissionResources,
    pub(super) input: &'a mut MissionInput,
    pub(super) ui: &'a mut MissionUi,
    pub(super) hud: &'a mut MissionHud,
    pub(super) runtime: &'a mut super::runtime::TimelineRuntime,
    pub(super) frame: &'a mut MissionFrame,
    pub(super) manual_pause: &'a mut bool,
    pub(super) step_forward_repeat_at_ms: &'a mut Option<u32>,
    pub(super) step_back_repeat_at_ms: &'a mut Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct StepShortcutInput {
    now_ms: u32,
    forward_held: bool,
    forward_hit: bool,
    back_held: bool,
    back_hit: bool,
    backspace_held: bool,
    unpause_hit: bool,
    gated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct StepShortcutOutput {
    forward: bool,
    back: bool,
    manual_pause: Option<bool>,
}

fn plan_step_shortcuts(
    input: StepShortcutInput,
    forward_repeat_at_ms: &mut Option<u32>,
    back_repeat_at_ms: &mut Option<u32>,
) -> StepShortcutOutput {
    const INITIAL_DELAY_MS: u32 = 160;
    const REPEAT_INTERVAL_MS: u32 = 40;

    let repeat = |held: bool, hit: bool, repeat_at_ms: &mut Option<u32>| -> bool {
        if !held {
            *repeat_at_ms = None;
            return false;
        }
        if hit {
            *repeat_at_ms = Some(input.now_ms.saturating_add(INITIAL_DELAY_MS));
            return true;
        }
        if let Some(next_ms) = *repeat_at_ms
            && input.now_ms >= next_ms
        {
            *repeat_at_ms = Some(input.now_ms.saturating_add(REPEAT_INTERVAL_MS));
            return true;
        }
        false
    };

    let forward = repeat(input.forward_held, input.forward_hit, forward_repeat_at_ms);
    let back = repeat(input.back_held, input.back_hit, back_repeat_at_ms) || input.backspace_held;
    if input.gated {
        return StepShortcutOutput {
            forward: false,
            back: false,
            manual_pause: None,
        };
    }
    let manual_pause = if input.unpause_hit {
        Some(false)
    } else if forward || back {
        Some(true)
    } else {
        None
    };
    StepShortcutOutput {
        forward,
        back,
        manual_pause,
    }
}

fn input_modifiers(keys: &std::collections::BTreeSet<winit::keyboard::KeyCode>) -> InputModifiers {
    use winit::keyboard::KeyCode;
    InputModifiers {
        ctrl: keys.contains(&KeyCode::ControlLeft) || keys.contains(&KeyCode::ControlRight),
        shift: keys.contains(&KeyCode::ShiftLeft) || keys.contains(&KeyCode::ShiftRight),
        alt: keys.contains(&KeyCode::AltLeft) || keys.contains(&KeyCode::AltRight),
    }
}

/// Synchronize the physical swapchain and bounded logical game canvas.
///
/// `GameWindow::poll_events` applies the active profile's aspect policy before
/// returning events, so this stage only has to propagate the resulting logical
/// dimensions through renderer, camera, input, minimap, and HUD ownership.
/// The comparison also handles a resize consumed by a nested modal on the
/// preceding frame.
fn apply_frame_resizes(
    events: &[GameEvent],
    window: &mut GameWindow,
    host: &mut Host,
    game: &mut Game,
    manager: &mut robin_engine::engine_manager::EngineManager,
    assets: &robin_engine::engine::LevelAssets,
    input: &mut MissionInput,
    hud: &mut MissionHud,
    presentation: &mut MissionPresentation,
    frame: &mut MissionFrame,
) {
    if events
        .iter()
        .any(|event| matches!(event, GameEvent::Resized(..)))
    {
        presentation.renderer.sync_window_size(window);
    }

    let (new_w, new_h) = window.logical_size();
    let logical_changed = presentation.renderer.screen_width() != new_w as u16
        || presentation.renderer.screen_height() != new_h as u16
        || game.width != new_w as u16
        || game.height != new_h as u16
        || host.viewport.screen_size.x != new_w as f32
        || host.viewport.screen_size.y != new_h as f32;
    if !logical_changed {
        return;
    }

    presentation.renderer.sync_window_size(window);
    let w = new_w as f32;
    let h = new_h as f32;
    host.viewport.set_screen_size(w, h);
    game.set_resolution(new_w as u16, new_h as u16);
    input.resize(new_w, new_h, &host.key_config);
    if host.minimap_corner_size.x > 0.0 {
        let cmd = PlayerCommand::MinimapResize {
            base: engine_coordinates::ScreenPoint::new(w - 83.0, 38.0),
            corner_size: host.minimap_corner_size,
        };
        dispatch_local_command(host, &mut manager.engine, &mut frame.commands, assets, &cmd);
    }
    hud.resize(new_w, new_h);
}

/// Poll process events and immediately dispatch HUD controls in the historical
/// order. General keyboard/mouse actions are returned for the next phase.
pub(super) fn collect_event_and_hud_input(context: EventHudContext<'_>) -> EventHudOutcome {
    let EventHudContext {
        host,
        manager,
        game,
        assets,
        dev,
        callbacks,
        window,
        presentation,
        resources,
        input,
        ui,
        hud,
        runtime,
        frame,
        manual_pause,
        step_forward_repeat_at_ms,
        step_back_repeat_at_ms,
    } = context;

    // Active graphical modals own the raw event queue. Do not drain it from
    // underneath them or allow global gameplay shortcuts to fire.
    let scripted_modal_input_active = ui.active_modal.is_some() || modal_state_pending(host);
    let modal_input_active = scripted_modal_input_active || ui.active_ui_task.is_some();
    let mut pause_closed_this_frame = false;
    if scripted_modal_input_active && ui.close_pause(input, presentation) {
        if let Some(mut task) = ui.active_ui_task.take() {
            task.cleanup();
        }
        pause_closed_this_frame = true;
        callbacks.emit_app_effect(AppEffect::SetSoundMode(SoundMode::Mission));
    }
    let mut events = if modal_input_active {
        Vec::new()
    } else {
        window.poll_events()
    };
    apply_frame_resizes(
        &events,
        window,
        host,
        game,
        manager,
        assets,
        input,
        hud,
        presentation,
        frame,
    );
    input.threaded.feed_events(&events);

    let rewind_active = handle_hold_to_rewind(manager, assets, &input.threaded, runtime);

    if runtime.replay_player.is_none() && !rewind_active {
        handle_gamepad_events(
            host,
            manager,
            assets,
            &mut input.threaded,
            &mut frame.commands,
            &events,
            &mut window.active_gamepad,
        );
    }
    events.extend(input.threaded.drain_synthetic_events());

    if input.threaded.is_ended() {
        return EventHudOutcome::Control(HandlerAction::Exit(GameCode::Quit));
    }

    match handle_sherwood_hud_buttons(
        game,
        manager,
        host,
        &mut frame.commands,
        assets,
        callbacks,
        window,
        &mut presentation.renderer,
        &resources.menu,
        &mut ui.sherwood_campaign_flow,
        &events,
        &hud.sherwood_layout,
        &mut hud.sherwood_enable,
    ) {
        HandlerAction::Proceed => {}
        control => return EventHudOutcome::Control(control),
    }

    let input_suppressed = runtime.replay_player.is_some() || rewind_active;
    if !input_suppressed {
        let zoom_enable = ZoomButtonEnable::from_engine(&manager.engine, &host.engine_display);
        let zoom_hit = events.iter().find_map(|event| {
            let GameEvent::MouseDown(mx, my, 1, _) = *event else {
                return None;
            };
            hud.zoom_layout
                .hit_test(mx, my, zoom_enable)
                .map(|button| (button, mx, my))
        });
        if let Some((button, mx, my)) = zoom_hit {
            let factor = match button {
                ZoomButton::ZoomUp => 2.0,
                ZoomButton::ZoomDown => 0.5,
            };
            host.viewport.zoom_by(
                factor,
                Some(engine_coordinates::ScreenPoint::new(mx as f32, my as f32)),
            );
        }
    }

    if !game.is_sherwood && !input_suppressed {
        let corner_enable = CornerButtonEnable::from_engine(&manager.engine);
        for event in &events {
            match *event {
                GameEvent::MouseDown(mx, my, 1, _) => {
                    let Some(button) = hud.corner_layout.hit_test(mx, my, corner_enable) else {
                        continue;
                    };
                    dispatch_corner_button_left_click(
                        button,
                        manager,
                        game,
                        host,
                        assets,
                        &mut frame.commands,
                    );
                }
                GameEvent::MouseDown(mx, my, 3, _) => {
                    let Some(button) = hud.corner_layout.hit_test_geometric(mx, my) else {
                        continue;
                    };
                    dispatch_corner_button_right_click(
                        button,
                        manager,
                        host,
                        assets,
                        &mut frame.commands,
                    );
                }
                _ => {}
            }
        }

        let stature = manager.engine.retrieve_stature(None);
        game.stature_focus.maybe_clear(stature);
        let stature_enable =
            StatureEnable::from_stature(stature).with_focus_latch(game.stature_focus);
        for event in &events {
            if let GameEvent::MouseDown(mx, my, 1, _) = *event
                && let Some(button) = hud.stature_layout.hit_test(mx, my, stature_enable)
            {
                let command = button.as_command();
                dispatch_local_command(
                    host,
                    &mut manager.engine,
                    &mut frame.commands,
                    assets,
                    &command,
                );
                match button {
                    StatureButton::Up => game.stature_focus.latch_stand_up(stature),
                    StatureButton::Down => game.stature_focus.latch_crouch_down(stature),
                }
            }
        }
    }

    // These physical-key edges must be sampled before keyboard translation
    // advances the translator's previous-key buffer.
    let keys = &input.threaded.keyboard_state().keys;
    let minimap_toggle_pressed = host
        .minimap_fast_key
        .is_some_and(|fast_key| input.translator.was_key_released(fast_key, keys));
    use winit::keyboard::KeyCode;
    let step_keys_gated =
        ui.console_overlay.is_visible() || ui.pause_menu.is_some() || modal_input_active;
    let step_shortcuts = plan_step_shortcuts(
        StepShortcutInput {
            now_ms: frame.started_at_ms,
            forward_held: keys.contains(&KeyCode::Period),
            forward_hit: input.translator.was_key_pressed(KeyCode::Period, keys),
            back_held: keys.contains(&KeyCode::Comma),
            back_hit: input.translator.was_key_pressed(KeyCode::Comma, keys),
            backspace_held: keys.contains(&KeyCode::Backspace),
            unpause_hit: input.translator.was_key_released(KeyCode::Enter, keys),
            gated: step_keys_gated,
        },
        step_forward_repeat_at_ms,
        step_back_repeat_at_ms,
    );
    if let Some(paused) = step_shortcuts.manual_pause {
        *manual_pause = paused;
    }

    let mut keyboard_actions = input
        .translator
        .translate_keyboard(&input.threaded.keyboard_state().keys, TranslationFlags::ALL);
    if events
        .iter()
        .any(|event| matches!(event, GameEvent::MenuToggleRequested))
        || (ui.pause_menu.is_none()
            && !modal_input_active
            && events
                .iter()
                .any(|event| matches!(event, GameEvent::PauseRequested)))
    {
        keyboard_actions.push(GameAction::DisplayMenu);
    }
    let mouse_actions = if input.threaded.has_position() {
        input.translator.translate_mouse(
            input.threaded.position().x,
            input.threaded.position().y,
            input.threaded.wheel_delta(),
        )
    } else {
        Vec::new()
    };
    let modifiers = input_modifiers(&input.threaded.keyboard_state().keys);
    host.input.is_alt = modifiers.alt;

    handle_console_overlay_events(
        &mut ui.console_overlay,
        &mut manager.engine,
        assets,
        host,
        dev,
        &events,
        &keyboard_actions,
        &mut input.translator,
        frame,
    );

    EventHudOutcome::Ready(CollectedFrameInput {
        events,
        keyboard_actions,
        mouse_actions,
        modifiers,
        minimap_toggle_pressed,
        pause_closed_this_frame,
        rewind_active,
        step_forward_pressed: step_shortcuts.forward,
        step_back_pressed: step_shortcuts.back,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::KeyCode;

    fn step_input(now_ms: u32) -> StepShortcutInput {
        StepShortcutInput {
            now_ms,
            forward_held: false,
            forward_hit: false,
            back_held: false,
            back_hit: false,
            backspace_held: false,
            unpause_hit: false,
            gated: false,
        }
    }

    #[test]
    fn step_shortcuts_preserve_edge_repeat_and_modal_gating() {
        let mut forward_repeat = None;
        let mut back_repeat = None;
        let first = plan_step_shortcuts(
            StepShortcutInput {
                forward_held: true,
                forward_hit: true,
                ..step_input(100)
            },
            &mut forward_repeat,
            &mut back_repeat,
        );
        assert_eq!(
            first,
            StepShortcutOutput {
                forward: true,
                back: false,
                manual_pause: Some(true),
            }
        );
        assert_eq!(forward_repeat, Some(260));

        let before_repeat = plan_step_shortcuts(
            StepShortcutInput {
                forward_held: true,
                ..step_input(259)
            },
            &mut forward_repeat,
            &mut back_repeat,
        );
        assert!(!before_repeat.forward);

        let gated_repeat = plan_step_shortcuts(
            StepShortcutInput {
                forward_held: true,
                gated: true,
                ..step_input(260)
            },
            &mut forward_repeat,
            &mut back_repeat,
        );
        assert_eq!(
            gated_repeat,
            StepShortcutOutput {
                forward: false,
                back: false,
                manual_pause: None,
            }
        );
        assert_eq!(forward_repeat, Some(300));
    }

    #[test]
    fn unpause_edge_wins_when_sampled_with_a_step() {
        let mut forward_repeat = None;
        let mut back_repeat = None;
        let output = plan_step_shortcuts(
            StepShortcutInput {
                forward_held: true,
                forward_hit: true,
                unpause_hit: true,
                ..step_input(400)
            },
            &mut forward_repeat,
            &mut back_repeat,
        );

        assert!(output.forward);
        assert_eq!(output.manual_pause, Some(false));
    }

    #[test]
    fn modifier_collection_merges_left_and_right_keys() {
        let keys = [KeyCode::ControlRight, KeyCode::ShiftLeft, KeyCode::AltRight]
            .into_iter()
            .collect();

        assert_eq!(
            input_modifiers(&keys),
            InputModifiers {
                ctrl: true,
                shift: true,
                alt: true,
            }
        );
    }
}
