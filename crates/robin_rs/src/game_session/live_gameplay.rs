//! Live graphical action dispatch after replay/rewind admission.
//!
//! This module owns only live player actions. Event/HUD collection happens
//! before it, and replay/pre-tick command injection remains after it.

use super::event_hud::InputModifiers;
use super::interactive::{
    MissionAudio, MissionInput, MissionPresentation, MissionResources, MissionUi,
};
use super::*;
use crate::game::Game;
use robin_engine::player_command::FrameCommands;

/// Mission/process borrows required while live actions are admitted.
pub(super) struct LiveGameplayContext<'a> {
    pub(super) host: &'a mut Host,
    pub(super) engine: &'a Engine,
    pub(super) game: &'a mut Game,
    pub(super) assets: &'a robin_engine::engine::LevelAssets,
    pub(super) dev: &'a mut robin_engine::engine::DevState,
    pub(super) callbacks: &'a mut RustCallbacks,
    pub(super) window: &'a mut GameWindow,
    pub(super) presentation: &'a mut MissionPresentation,
    pub(super) resources: &'a mut MissionResources,
    pub(super) audio: &'a mut MissionAudio,
    pub(super) input: &'a mut MissionInput,
    pub(super) ui: &'a mut MissionUi,
    pub(super) commands: &'a mut FrameCommands,
    pub(super) external_actions: &'a mut Vec<engine_api::ExternalAction>,
}

/// Immutable frame inputs plus the one pause-close flag updated by dispatch.
pub(super) struct LiveGameplayInput<'a> {
    pub(super) events: &'a [GameEvent],
    pub(super) keyboard_actions: &'a [GameAction],
    pub(super) mouse_actions: &'a [GameAction],
    pub(super) minimap_toggle_pressed: bool,
    pub(super) modifiers: InputModifiers,
    pub(super) pause_closed_this_frame: &'a mut bool,
}

fn toggle_pause_menu(context: &mut LiveGameplayContext<'_>, pause_closed: &mut bool) {
    let LiveGameplayContext {
        host,
        engine,
        assets,
        callbacks,
        presentation,
        resources,
        input,
        ui,
        ..
    } = context;
    if ui.pause_menu.is_some() {
        debug_assert!(ui.close_pause(host, input, presentation));
        *pause_closed = true;
        callbacks.emit_app_effect(AppEffect::SetSoundMode(SoundMode::Mission));
        return;
    }

    let sherwood_trading_available = engine.is_sherwood(&assets.profile_manager)
        && host.transport.local_seat == engine_player_command::PlayerId::HOST
        && sherwood_trading_access(host, engine, &assets.profile_manager)
            .validate()
            .is_ok();
    if let Some(menu_resources) = resources.menu.as_ref() {
        ui.pause_menu = Some(PauseMenu::new_with_sherwood_trading(
            menu_resources,
            ui.restart_allowed,
            sherwood_trading_available,
        ));
    } else {
        let files = match host.preparation_files() {
            Ok(files) => files.clone(),
            Err(error) => {
                tracing::error!(%error, "cannot reload pause menu without preparation authority");
                return;
            }
        };
        let fallback = IngameMenuResources::new(
            &mut presentation.renderer,
            host.frontend.shipping.as_deref(),
            files,
        );
        let menu_resources =
            required_menu_resources(&fallback, "opening the pause menu after resource reload");
        ui.pause_menu = Some(PauseMenu::new_with_sherwood_trading(
            menu_resources,
            ui.restart_allowed,
            sherwood_trading_available,
        ));
        resources.menu = fallback;
    }
    if ui.pause_menu.is_some() {
        presentation.renderer.freeze_scene_for_modal();
        callbacks.emit_app_effect(AppEffect::SetSoundMode(SoundMode::Menu));
    }
}

fn portrait_selection_command(
    portrait_index: u8,
    ctrl_held: bool,
    quick_group_exists: bool,
) -> PlayerCommand {
    if ctrl_held {
        PlayerCommand::AssignQuickGroup {
            index: portrait_index,
        }
    } else if quick_group_exists {
        PlayerCommand::RecallQuickGroup {
            index: portrait_index,
        }
    } else {
        PlayerCommand::SelectByPortrait {
            portrait_index: u32::from(portrait_index),
            append: false,
        }
    }
}

/// Dispatch one admitted non-menu gameplay action.
fn dispatch_gameplay_action(
    context: &mut LiveGameplayContext<'_>,
    action: &GameAction,
    modifiers: InputModifiers,
) {
    let LiveGameplayContext {
        host,
        engine,
        game,
        assets,
        dev,
        callbacks,
        input,
        commands,
        external_actions,
        ..
    } = context;
    let InputModifiers {
        ctrl: ctrl_held,
        shift: shift_held,
        alt: _,
        plan: planning_held,
    } = modifiers;

    match action {
        GameAction::SlowMotion => host.frontend.slow_motion = !host.frontend.slow_motion,
        GameAction::SwitchMaskedDisplay => {
            host.frontend.input.draw_hidden = !host.frontend.input.draw_hidden
        }
        // Host-only view actions have already run in the preceding phase.
        GameAction::ScrollUp
        | GameAction::ScrollDown
        | GameAction::ScrollLeft
        | GameAction::ScrollRight
        | GameAction::ZoomIn
        | GameAction::ZoomOut => {}
        GameAction::SelectAll => {
            dispatch_local_command(&host.transport, commands, &PlayerCommand::SelectAllPcs);
        }
        GameAction::UnselectAll => {
            dispatch_local_command(&host.transport, commands, &PlayerCommand::UnselectAllPcs);
        }
        GameAction::SelectAction { index } => {
            let selected = engine.hero_selection(host.transport.local_seat);
            if selected.len() == 1 {
                let pc_id = selected[0];
                let command = if planning_held {
                    let Some(action) = engine
                        .get_entity(pc_id)
                        .and_then(|entity| entity.pc_data())
                        .and_then(|pc| assets.profile_manager.get_character(pc.profile_index))
                        .and_then(|profile| profile.actions.get(*index as usize))
                        .copied()
                    else {
                        tracing::warn!(
                            ?pc_id,
                            index,
                            "planned action shortcut has no profile action"
                        );
                        return;
                    };
                    PlayerCommand::SelectPlannedAction { pc_id, action }
                } else {
                    PlayerCommand::SelectAction {
                        pc_id,
                        action_index: *index as u32,
                    }
                };
                dispatch_local_command(&host.transport, commands, &command);
            }
        }
        GameAction::SelectCharacter { portrait_index } => {
            let index = *portrait_index as usize;
            let command = portrait_selection_command(
                *portrait_index,
                ctrl_held,
                index < 9 && !engine.quick_select_group(index).is_empty(),
            );
            dispatch_local_command(&host.transport, commands, &command);
        }
        GameAction::QuickSave => {
            if !engine.is_zoom_possible(&host.frontend.engine_display) {
                game.quick_save_after_zoom = true;
            } else {
                let mission_id = current_mission_id(engine.campaign(), &assets.profile_manager);
                callbacks.queue_operation(SaveLoadRequest::QuickSave { mission_id });
            }
        }
        GameAction::QuickLoad => {
            if !host.transport.authoritative_transition_actions_enabled() {
                game.display_message(
                    "Quick Load is available only to the multiplayer host after synchronization finishes."
                        .to_string(),
                    100,
                );
                return;
            }
            if !engine.is_zoom_possible(&host.frontend.engine_display) {
                game.quick_load_after_zoom = true;
            } else {
                callbacks.queue_operation(SaveLoadRequest::QuickLoad {
                    use_backup: shift_held,
                });
            }
        }
        GameAction::CrouchDown => {
            let pre_command_stature = engine.retrieve_stature(None);
            let command = if planning_held {
                PlayerCommand::QueueQuickAction {
                    action: robin_engine::profiles::Action::NoAction,
                    command: robin_engine::player_command::QueuedQuickActionCommand::CrouchDown,
                }
            } else {
                PlayerCommand::CrouchDown
            };
            dispatch_local_command(&host.transport, commands, &command);
            if !planning_held {
                game.stature_focus.latch_crouch_down(pre_command_stature);
            }
        }
        GameAction::StandUp => {
            let pre_command_stature = engine.retrieve_stature(None);
            let command = if planning_held {
                PlayerCommand::QueueQuickAction {
                    action: robin_engine::profiles::Action::NoAction,
                    command: robin_engine::player_command::QueuedQuickActionCommand::StandUp,
                }
            } else {
                PlayerCommand::StandUp
            };
            dispatch_local_command(&host.transport, commands, &command);
            if !planning_held {
                game.stature_focus.latch_stand_up(pre_command_stature);
            }
        }
        GameAction::ToggleCloak => {
            // Resolve selection into per-actor commands before recording.
            // This keeps multiplayer/replay semantics independent of later
            // selection changes and makes mixed cloaked/upright groups safe.
            for command in engine.cloak_toggle_commands_for_seat(host.transport.local_seat) {
                dispatch_local_command(&host.transport, commands, &command);
            }
        }
        GameAction::KeyControl => {
            dispatch_local_command(&host.transport, commands, &PlayerCommand::KeyControl);
        }
        GameAction::KeyReleaseControl => {
            dispatch_local_command(&host.transport, commands, &PlayerCommand::KeyReleaseControl);
        }
        GameAction::SwitchTask => {
            external_actions.push(robin_engine::engine::ExternalAction::SimpleMessage {
                message: engine_messenger::SimpleMessage::SwitchTask,
            });
        }
        GameAction::Teleport => {
            let mouse_screen = input.threaded.position();
            if let Some(mouse_map) = host.frontend.viewport.screen_to_map(mouse_screen) {
                if !engine.hero_selection(host.transport.local_seat).is_empty() {
                    let accessible = engine.fast_grid().get_sector_screen_accessible(mouse_map);
                    if let Some(sector_idx) = accessible.sector_idx {
                        let command = PlayerCommand::TeleportSelectedToPoint {
                            dest: mouse_map,
                            layer: accessible.layer,
                            sector: u16::try_from(u32::from(sector_idx))
                                .ok()
                                .and_then(engine_position_interface::SectorHandle::new),
                        };
                        dispatch_local_command(&host.transport, commands, &command);
                    }
                } else if dev.debug.free_shadow_polygon {
                    let point = engine.fast_grid().convert_2d_to_3d(
                        mouse_map,
                        engine_sight_obstacle::SIGHTOBSTACLE_MOUSE,
                        engine.sight_obstacles(assets),
                    );
                    dev.cheat_free_shadow_polygon_pos = Some(engine_coordinates::WorldPoint3D {
                        x: point.x,
                        y: point.y,
                        z: point.z + 45.0,
                    });
                }
            }
        }
        GameAction::RecordQa => {
            if !game.is_sherwood {
                dispatch_corner_button_left_click(
                    CornerButton::Clock,
                    engine,
                    game,
                    host,
                    commands,
                );
            }
        }
        GameAction::OpenSherwoodTrading => {
            if let Err(reason) =
                request_sherwood_trading_panel(host, engine, &assets.profile_manager)
            {
                tracing::debug!(?reason, "Sherwood trading shortcut rejected");
            }
        }
        GameAction::PrintScreen => {
            host.frontend.pending_print_screen =
                Some(print_screen_request_from_modifiers(ctrl_held, shift_held));
        }
        _ => tracing::trace!("Game action: {:?}", action),
    }
}

/// Dispatch simulation-affecting keyboard, pause-menu, and mouse input. The
/// caller admits this phase only when replay and rewind are inactive.
pub(super) async fn drive_live_gameplay_input(
    mut context: LiveGameplayContext<'_>,
    input_batch: LiveGameplayInput<'_>,
) -> HandlerAction {
    let LiveGameplayInput {
        events,
        keyboard_actions,
        mouse_actions,
        minimap_toggle_pressed,
        modifiers,
        pause_closed_this_frame,
    } = input_batch;

    let planned_action = context
        .engine
        .planned_action_for_seat(context.host.transport.local_seat);
    if context.assets.attachments.spellforge_runtime.is_some()
        && !context.ui.console_overlay.is_visible()
        && context.ui.pause_menu.is_none()
    {
        for event in events {
            if let crate::gfx_types::GameEvent::KeyDown { keycode, .. } = event
                && let Some(virtual_key) = spellforge_virtual_key(*keycode)
            {
                dispatch_local_command(
                    &context.host.transport,
                    context.commands,
                    &PlayerCommand::ScriptKeyPressed { virtual_key },
                );
            }
        }
    }
    if should_cancel_planned_action(modifiers.plan, planned_action) {
        dispatch_local_command(
            &context.host.transport,
            context.commands,
            &PlayerCommand::CancelPlannedAction,
        );
    }
    if minimap_toggle_pressed
        && !context.ui.console_overlay.is_visible()
        && context.ui.pause_menu.is_none()
    {
        dispatch_local_command(
            &context.host.transport,
            context.commands,
            &PlayerCommand::MinimapToggle,
        );
    }

    for action in keyboard_actions.iter().chain(mouse_actions) {
        if context.ui.console_overlay.is_visible() {
            continue;
        }
        match action {
            GameAction::DisplayConsole => {}
            GameAction::DisplayInfo => {
                context.host.frontend.info_displayed = !context.host.frontend.info_displayed;
                tracing::debug!(
                    "DisplayInfo toggled: {}",
                    context.host.frontend.info_displayed
                );
            }
            GameAction::DisplayMenu => {
                toggle_pause_menu(&mut context, pause_closed_this_frame);
            }
            _ if context.ui.pause_menu.is_some() || *pause_closed_this_frame => {}
            _ => dispatch_gameplay_action(&mut context, action, modifiers),
        }
    }

    match handle_pause_menu_events(
        &mut context.ui.pause_menu,
        &mut context.ui.active_ui_task,
        pause_closed_this_frame,
        context.host,
        context.engine,
        context.assets,
        context.callbacks,
        context.window,
        &mut context.presentation.renderer,
        &context.resources.menu,
        &mut context.audio.backend,
        &context.audio.sample_loader,
        &mut context.input.threaded,
        &mut context.input.translator,
        events,
    ) {
        HandlerAction::Continue => return HandlerAction::Continue,
        HandlerAction::Exit(code) => {
            execute_app_effects(
                &mut context.callbacks.app_effects,
                &mut context.host.audio.sound,
                &mut context.input.threaded,
                context
                    .audio
                    .backend
                    .as_mut()
                    .map(|backend| backend as &mut dyn crate::sound::AudioBackend),
            );
            return HandlerAction::Exit(code);
        }
        HandlerAction::Proceed => {}
    }

    handle_mouse_input(
        context.engine,
        context.host,
        context.assets,
        context.presentation.renderer.screen_width(),
        context.presentation.renderer.screen_height(),
        &context.presentation.sprites.portrait_cache,
        context.commands,
        events,
        context.ui.pause_menu.as_ref(),
        *pause_closed_this_frame,
        modifiers.shift,
        modifiers.plan,
        modifiers.ctrl,
    );
    HandlerAction::Proceed
}

/// Stable Win32 virtual-key mapping used by upstream Spellforge missions.
/// `Char` values are normalized to uppercase, matching `WM_KEYDOWN`'s VK
/// identity rather than locale-dependent text input.
fn spellforge_virtual_key(key: crate::gfx_types::Keycode) -> Option<i32> {
    use crate::gfx_types::Keycode;
    Some(match key {
        Keycode::Escape => 0x1b,
        Keycode::Return | Keycode::KpEnter => 0x0d,
        Keycode::Tab => 0x09,
        Keycode::Space => 0x20,
        Keycode::Backspace => 0x08,
        Keycode::Delete => 0x2e,
        Keycode::Up => 0x26,
        Keycode::Down => 0x28,
        Keycode::Left => 0x25,
        Keycode::Right => 0x27,
        Keycode::Home => 0x24,
        Keycode::End => 0x23,
        Keycode::PageUp => 0x21,
        Keycode::PageDown => 0x22,
        Keycode::F1 => 0x70,
        Keycode::F2 => 0x71,
        Keycode::F3 => 0x72,
        Keycode::F4 => 0x73,
        Keycode::F5 => 0x74,
        Keycode::F6 => 0x75,
        Keycode::F7 => 0x76,
        Keycode::F8 => 0x77,
        Keycode::F9 => 0x78,
        Keycode::F10 => 0x79,
        Keycode::F11 => 0x7a,
        Keycode::F12 => 0x7b,
        Keycode::LShift | Keycode::RShift => 0x10,
        Keycode::LCtrl | Keycode::RCtrl => 0x11,
        Keycode::LAlt | Keycode::RAlt => 0x12,
        Keycode::Insert => 0x2d,
        Keycode::Char(character) if character.is_ascii_alphanumeric() => {
            i32::from(character.to_ascii_uppercase())
        }
        Keycode::Char(_) | Keycode::Unknown => return None,
    })
}

fn should_cancel_planned_action(
    planning_is_active: bool,
    planned_action: robin_engine::profiles::Action,
) -> bool {
    !planning_is_active && planned_action != robin_engine::profiles::Action::NoAction
}

#[cfg(test)]
mod tests {
    use super::{portrait_selection_command, should_cancel_planned_action, spellforge_virtual_key};
    use crate::gfx_types::Keycode;
    use robin_engine::player_command::PlayerCommand;
    use robin_engine::profiles::Action;

    #[test]
    fn portrait_dispatch_prioritizes_assignment_then_recall_then_portrait() {
        assert!(matches!(
            portrait_selection_command(3, true, true),
            PlayerCommand::AssignQuickGroup { index: 3 }
        ));
        assert!(matches!(
            portrait_selection_command(3, false, true),
            PlayerCommand::RecallQuickGroup { index: 3 }
        ));
        assert!(matches!(
            portrait_selection_command(3, false, false),
            PlayerCommand::SelectByPortrait {
                portrait_index: 3,
                append: false
            }
        ));
    }

    #[test]
    fn shift_release_cancels_only_an_armed_planned_action() {
        assert!(should_cancel_planned_action(false, Action::Bow));
        assert!(!should_cancel_planned_action(true, Action::Bow));
        assert!(!should_cancel_planned_action(false, Action::NoAction));
    }

    #[test]
    fn spellforge_keys_use_stable_win32_virtual_key_values() {
        assert_eq!(spellforge_virtual_key(Keycode::Char(b'a')), Some(0x41));
        assert_eq!(spellforge_virtual_key(Keycode::F12), Some(0x7b));
        assert_eq!(spellforge_virtual_key(Keycode::Left), Some(0x25));
        assert_eq!(spellforge_virtual_key(Keycode::Unknown), None);
    }
}
