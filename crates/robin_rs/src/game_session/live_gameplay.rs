//! Live graphical action dispatch after replay/rewind admission.
//!
//! This module owns only live player actions. Event/HUD collection happens
//! before it, and replay/pre-tick command injection remains after it.

use super::event_hud::InputModifiers;
use super::interactive::{
    MissionAudio, MissionHud, MissionInput, MissionPresentation, MissionResources, MissionUi,
};
use super::*;
use crate::game::Game;

/// Mission/process borrows required while live actions are admitted.
pub(super) struct LiveGameplayContext<'a> {
    pub(super) host: &'a mut Host,
    pub(super) manager: &'a mut robin_engine::engine_manager::EngineManager,
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
    pub(super) hud: &'a mut MissionHud,
    pub(super) frame: &'a mut MissionFrame,
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
        callbacks,
        presentation,
        resources,
        input,
        ui,
        ..
    } = context;
    if ui.pause_menu.is_some() {
        debug_assert!(ui.close_pause(input, presentation));
        *pause_closed = true;
        callbacks.emit_app_effect(AppEffect::SetSoundMode(SoundMode::Mission));
        callbacks.start_play_time();
        return;
    }

    callbacks.suspend_play_time();
    if let Some(menu_resources) = resources.menu.as_ref() {
        ui.pause_menu = Some(PauseMenu::new(menu_resources, ui.restart_allowed));
    } else {
        let fallback =
            IngameMenuResources::new(&mut presentation.renderer, host.shipping.as_deref());
        let menu_resources =
            required_menu_resources(&fallback, "opening the pause menu after resource reload");
        ui.pause_menu = Some(PauseMenu::new(menu_resources, ui.restart_allowed));
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
        manager,
        game,
        assets,
        dev,
        callbacks,
        input,
        frame,
        ..
    } = context;
    let InputModifiers {
        ctrl: ctrl_held,
        shift: shift_held,
        alt: _,
    } = modifiers;

    match action {
        GameAction::SlowMotion => host.slow_motion = !host.slow_motion,
        GameAction::SwitchMaskedDisplay => host.input.draw_hidden = !host.input.draw_hidden,
        // Host-only view actions have already run in the preceding phase.
        GameAction::ScrollUp
        | GameAction::ScrollDown
        | GameAction::ScrollLeft
        | GameAction::ScrollRight
        | GameAction::ZoomIn
        | GameAction::ZoomOut => {}
        GameAction::SelectAll => {
            dispatch_local_command(
                host,
                &mut manager.engine,
                &mut frame.commands,
                assets,
                &PlayerCommand::SelectAllPcs,
            );
        }
        GameAction::UnselectAll => {
            dispatch_local_command(
                host,
                &mut manager.engine,
                &mut frame.commands,
                assets,
                &PlayerCommand::UnselectAllPcs,
            );
        }
        GameAction::SelectAction { index } => {
            let selected = manager.engine.hero_selection(host.transport.local_seat);
            if selected.len() == 1 {
                let command = PlayerCommand::SelectAction {
                    pc_id: selected[0],
                    action_index: *index as u32,
                };
                dispatch_local_command(
                    host,
                    &mut manager.engine,
                    &mut frame.commands,
                    assets,
                    &command,
                );
            }
        }
        GameAction::SelectCharacter { portrait_index } => {
            let index = *portrait_index as usize;
            let command = portrait_selection_command(
                *portrait_index,
                ctrl_held,
                index < 9 && !manager.engine.quick_select_group(index).is_empty(),
            );
            dispatch_local_command(
                host,
                &mut manager.engine,
                &mut frame.commands,
                assets,
                &command,
            );
        }
        GameAction::QuickSave => {
            if !manager.engine.is_zoom_possible(&host.engine_display) {
                game.quick_save_after_zoom = true;
            } else {
                let mission_id =
                    current_mission_id(manager.engine.campaign(), &assets.profile_manager);
                callbacks.pending = Some(SaveLoadRequest::QuickSave { mission_id });
            }
        }
        GameAction::QuickLoad => {
            if !manager.engine.is_zoom_possible(&host.engine_display) {
                game.quick_load_after_zoom = true;
            } else {
                callbacks.pending = Some(SaveLoadRequest::QuickLoad {
                    use_backup: shift_held,
                });
            }
        }
        GameAction::CrouchDown => {
            let pre_command_stature = manager.engine.retrieve_stature(None);
            dispatch_local_command(
                host,
                &mut manager.engine,
                &mut frame.commands,
                assets,
                &PlayerCommand::CrouchDown,
            );
            game.stature_focus.latch_crouch_down(pre_command_stature);
        }
        GameAction::StandUp => {
            let pre_command_stature = manager.engine.retrieve_stature(None);
            dispatch_local_command(
                host,
                &mut manager.engine,
                &mut frame.commands,
                assets,
                &PlayerCommand::StandUp,
            );
            game.stature_focus.latch_stand_up(pre_command_stature);
        }
        GameAction::ToggleCloak => {
            // Resolve selection into per-actor commands before recording.
            // This keeps multiplayer/replay semantics independent of later
            // selection changes and makes mixed cloaked/upright groups safe.
            for command in manager
                .engine
                .cloak_toggle_commands_for_seat(host.transport.local_seat)
            {
                dispatch_local_command(
                    host,
                    &mut manager.engine,
                    &mut frame.commands,
                    assets,
                    &command,
                );
            }
        }
        GameAction::KeyControl => {
            dispatch_local_command(
                host,
                &mut manager.engine,
                &mut frame.commands,
                assets,
                &PlayerCommand::KeyControl,
            );
        }
        GameAction::KeyReleaseControl => {
            dispatch_local_command(
                host,
                &mut manager.engine,
                &mut frame.commands,
                assets,
                &PlayerCommand::KeyReleaseControl,
            );
        }
        GameAction::SwitchTask => {
            frame
                .external_actions
                .push(robin_engine::engine::ExternalAction::SimpleMessage {
                    message: engine_messenger::SimpleMessage::SwitchTask,
                });
        }
        GameAction::Teleport => {
            let mouse_screen = input.threaded.position();
            if let Some(mouse_map) = host.viewport.screen_to_map(mouse_screen) {
                if !manager
                    .engine
                    .hero_selection(host.transport.local_seat)
                    .is_empty()
                {
                    let accessible = manager
                        .engine
                        .fast_grid()
                        .get_sector_screen_accessible(mouse_map);
                    if let Some(sector_idx) = accessible.sector_idx {
                        let command = PlayerCommand::TeleportSelectedToPoint {
                            dest: mouse_map,
                            layer: accessible.layer,
                            sector: u16::try_from(u32::from(sector_idx))
                                .ok()
                                .and_then(engine_position_interface::SectorHandle::new),
                        };
                        dispatch_local_command(
                            host,
                            &mut manager.engine,
                            &mut frame.commands,
                            assets,
                            &command,
                        );
                    }
                } else if dev.debug.free_shadow_polygon {
                    let point = manager.engine.fast_grid().convert_2d_to_3d(
                        mouse_map,
                        engine_sight_obstacle::SIGHTOBSTACLE_MOUSE,
                        manager.engine.sight_obstacles(assets),
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
                    manager,
                    game,
                    host,
                    assets,
                    &mut frame.commands,
                );
            }
        }
        GameAction::PrintScreen => {
            host.pending_print_screen =
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
        .manager
        .engine
        .planned_action_for_seat(context.host.transport.local_seat);
    if should_cancel_planned_action(modifiers.shift, planned_action) {
        dispatch_local_command(
            context.host,
            &mut context.manager.engine,
            &mut context.frame.commands,
            context.assets,
            &PlayerCommand::CancelPlannedAction,
        );
    }
    if minimap_toggle_pressed
        && !context.ui.console_overlay.is_visible()
        && context.ui.pause_menu.is_none()
    {
        dispatch_local_command(
            context.host,
            &mut context.manager.engine,
            &mut context.frame.commands,
            context.assets,
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
                context.host.info_displayed = !context.host.info_displayed;
                tracing::debug!("DisplayInfo toggled: {}", context.host.info_displayed);
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
        pause_closed_this_frame,
        context.host,
        context.manager,
        context.game,
        context.assets,
        context.callbacks,
        context.window,
        &mut context.presentation.renderer,
        &mut context.presentation.sprites.cursor_renderer,
        context.resources,
        &mut context.audio.backend,
        &context.audio.sample_loader,
        &mut context.input.threaded,
        &mut context.input.translator,
        &mut context.hud.sherwood_layout,
        &mut context.hud.zoom_layout,
        &context.hud.zoom_sprites,
        &mut context.frame.commands,
        events,
    )
    .await
    {
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
        context.manager,
        context.host,
        context.assets,
        context.presentation.renderer.screen_width(),
        context.presentation.renderer.screen_height(),
        &context.presentation.sprites.portrait_cache,
        &mut context.frame.commands,
        events,
        context.ui.pause_menu.as_ref(),
        *pause_closed_this_frame,
        modifiers.shift,
        modifiers.ctrl,
    );
    HandlerAction::Proceed
}

fn should_cancel_planned_action(
    shift_is_held: bool,
    planned_action: robin_engine::profiles::Action,
) -> bool {
    !shift_is_held && planned_action != robin_engine::profiles::Action::NoAction
}

#[cfg(test)]
mod tests {
    use super::{portrait_selection_command, should_cancel_planned_action};
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
}
