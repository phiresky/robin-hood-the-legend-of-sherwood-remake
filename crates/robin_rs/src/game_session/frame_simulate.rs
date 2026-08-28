//! Deterministic interactive-frame simulation and modal orchestration.
//!
//! Input preparation remains in `flow`. This phase owns the exact post-input
//! order from command recording through simulation history, stepping, scripted
//! modals, and the handoff to presentation.

use super::flow::{FrameControl, MissionServices};
use super::interactive::{MissionPresentation, MissionResources};
use super::runtime::FrameContractStage;
use super::terminal_debriefing::{TerminalDebriefingContext, drive_tick_exit_modals};
use super::ui_task_state::UiTaskOutcome;
use super::*;
use crate::game::Game;
use crate::host::HostSignal;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct FramePresentationHandoff {
    pub(super) frame: MissionFrame,
    pub(super) rewind_active: bool,
    pub(super) consumed_buffered: bool,
    pub(super) shift_held: bool,
    pub(super) modal_rendered: bool,
    pub(super) history_commit_pending: bool,
}

pub(super) enum FrameSimulationOutcome {
    Control(FrameControl),
    Present(FramePresentationHandoff),
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub(super) struct FrameSimulationFlags {
    pub(super) rewind_active: bool,
    pub(super) paused: bool,
    pub(super) consumed_buffered: bool,
    pub(super) shift_held: bool,
    pub(super) modal_rendered: bool,
    pub(super) step_forward_pressed: bool,
    pub(super) step_back_pressed: bool,
}

/// One admitted interactive frame after input preparation has completed.
///
/// This short-lived owner carries deterministic frame data and control flags
/// through simulation. It deliberately borrows process resources only while
/// [`Self::run`] is active, then hands the completed frame back to presentation.
pub(super) struct InteractiveFrameSimulation {
    frame: MissionFrame,
    flags: FrameSimulationFlags,
}

struct SimulationModalState {
    frame: MissionFrame,
    rewind_active: bool,
    consumed_buffered: bool,
    shift_held: bool,
    modal_rendered_this_frame: bool,
    auto_dismiss_modals: bool,
    tick_exit_code: Option<GameCode>,
    history_commit_pending: bool,
}

/// Host-only visual state which must be refreshed immediately after the
/// deterministic tick but before scripted modal drains.
struct SimulationVisualRefresh<'a> {
    last_shadow_color: &'a mut u16,
    manager: &'a mut robin_engine::engine_manager::EngineManager,
    host: &'a mut Host,
    dev: &'a mut robin_engine::engine::DevState,
    presentation: &'a mut MissionPresentation,
    resources: &'a mut MissionResources,
    window: &'a GameWindow,
}

impl SimulationVisualRefresh<'_> {
    fn run(self) {
        let Self {
            last_shadow_color,
            manager,
            host,
            dev,
            presentation,
            resources,
            window,
        } = self;

        let current_shadow_color = manager.engine.weather().night_color;
        if current_shadow_color != *last_shadow_color {
            tracing::info!(
                "Ambience shadow-key changed {:#06x} → {:#06x}; rebinding sprite caches",
                last_shadow_color,
                current_shadow_color,
            );
            presentation.rebind_shadow_key(resources, host, &window.gpu, current_shadow_color);
            *last_shadow_color = current_shadow_color;
        }

        // Console `LEVEL TEXT D/DB/PT` requests are host-side because the
        // descriptor tables deliberately do not live in deterministic state.
        if dev.debug.all_dialogues {
            dev.debug.all_dialogues = false;
            if let Some(descriptors) = &resources.level_descriptors {
                host.effects
                    .extend_dialogues((0..descriptors.dialogues.len()).map(|index| index as i32));
            } else {
                tracing::warn!("cheat all_dialogues: level descriptors unavailable");
            }
        }
        if dev.debug.all_popup_texts {
            dev.debug.all_popup_texts = false;
            if let Some(descriptors) = &resources.level_descriptors {
                host.effects.extend_popup_texts(
                    (0..descriptors.popup_text.picture_ids.len()).map(|index| index as i32),
                );
            } else {
                tracing::warn!("cheat all_popup_texts: level descriptors unavailable");
            }
        }
        if dev.debug.all_debriefings {
            dev.debug.all_debriefings = false;
            if let Some(descriptors) = &resources.level_descriptors {
                host.effects.extend_debriefings(
                    (0..descriptors.debriefing.lose_count as usize)
                        .map(|index| engine_player_command::DebriefingTextId::Lose { index }),
                );
                host.effects.extend_debriefings(
                    (0..descriptors.debriefing.win_count as usize)
                        .map(|index| engine_player_command::DebriefingTextId::Win { index }),
                );
            } else {
                tracing::warn!("cheat all_debriefings: level descriptors unavailable");
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum ScriptedModalMode {
    Interactive,
    AutoDismiss,
}

/// Drain the ordered dialogue -> popup/report -> debriefing lanes. An
/// interactive frame renders at most one lane; headless map export drains all
/// lanes without presenting them.
#[allow(clippy::too_many_arguments)]
async fn drive_scripted_modal_lanes(
    host: &mut Host,
    game: &Game,
    manager: &robin_engine::engine_manager::EngineManager,
    profiles: &engine_profiles::ProfileManager,
    window: &mut GameWindow,
    audio: &mut super::interactive::MissionAudio,
    resources: &mut super::interactive::MissionResources,
    ui: &mut super::interactive::MissionUi,
    presentation: &mut super::interactive::MissionPresentation,
    _runtime: &mut super::runtime::TimelineRuntime,
    frame: &mut MissionFrame,
    mode: ScriptedModalMode,
    mut rendered: bool,
) -> bool {
    let auto_dismiss = mode == ScriptedModalMode::AutoDismiss;
    let mut modal_ctx = ModalContext {
        window,
        renderer: &mut presentation.renderer,
        cursor_res: &mut resources.cursor,
        cursor_renderer: &mut presentation.sprites.cursor_renderer,
        audio_backend: &mut audio.backend,
        sample_loader: &audio.sample_loader,
        menu_resources: &mut resources.menu,
        modal_dismissals: &mut frame.modal_dismissals,
    };
    if auto_dismiss {
        drain_pending_dialogues(
            host,
            &mut modal_ctx,
            &mut resources.text,
            game,
            &resources.level_descriptors,
            &mut frame.replay_modal_dismissals,
            true,
        )
        .await;
    } else {
        if ui.active_modal.is_none()
            && let Some(batch) = start_active_dialogue_batch(
                host,
                &mut resources.text,
                game,
                &resources.level_descriptors,
                &mut frame.replay_modal_dismissals,
            )
        {
            ui.active_modal = Some(ActiveModal::Dialogue(Box::new(batch)));
        }
        if ui.active_modal.is_some() {
            let outcome = tick_active_modal(
                &mut ui.active_modal,
                host,
                &mut modal_ctx,
                &mut frame.replay_modal_dismissals,
            );
            debug_assert_eq!(outcome, ActiveModalOutcome::None);
            rendered = true;
        }
    }

    if !rendered && auto_dismiss {
        drain_pending_popup_scroll(
            host,
            &mut modal_ctx,
            &mut resources.text,
            &resources.level_descriptors,
            &mut frame.replay_modal_dismissals,
            manager.engine.simulation_tick().number(),
        )
        .await;
        drain_pending_sherwood_stat(
            host,
            &mut modal_ctx,
            &manager.engine,
            profiles,
            &mut frame.replay_modal_dismissals,
        )
        .await;
    } else if !rendered {
        if ui.active_modal.is_none()
            && let Some(batch) = start_active_popup_scroll_batch(
                host,
                &mut modal_ctx,
                &mut resources.text,
                &resources.level_descriptors,
                &mut frame.replay_modal_dismissals,
                manager.engine.simulation_tick().number(),
            )
        {
            ui.active_modal = Some(ActiveModal::PopupScroll(Box::new(batch)));
        }
        if ui.active_modal.is_none()
            && let Some(batch) = start_active_sherwood_report(
                host,
                &mut modal_ctx,
                &manager.engine,
                profiles,
                &mut frame.replay_modal_dismissals,
            )
        {
            ui.active_modal = Some(ActiveModal::PopupScroll(Box::new(batch)));
        }
        if ui.active_modal.is_some() {
            let outcome = tick_active_modal(
                &mut ui.active_modal,
                host,
                &mut modal_ctx,
                &mut frame.replay_modal_dismissals,
            );
            debug_assert_eq!(outcome, ActiveModalOutcome::None);
            rendered = true;
        }
    }

    if !rendered && auto_dismiss {
        drain_pending_debriefings(
            host,
            &mut modal_ctx,
            &mut resources.text,
            &resources.level_descriptors,
            &mut frame.replay_modal_dismissals,
        )
        .await;
    } else if !rendered {
        if ui.active_modal.is_none()
            && let Some(batch) = start_active_debriefing_batch(
                host,
                &mut modal_ctx,
                &mut resources.text,
                &resources.level_descriptors,
                &mut frame.replay_modal_dismissals,
            )
        {
            ui.active_modal = Some(ActiveModal::Debriefing(Box::new(batch)));
        }
        if ui.active_modal.is_some() {
            let outcome = tick_active_modal(
                &mut ui.active_modal,
                host,
                &mut modal_ctx,
                &mut frame.replay_modal_dismissals,
            );
            debug_assert_eq!(outcome, ActiveModalOutcome::None);
            rendered = true;
        }
    }
    rendered
}

/// Drive the first mission-won "leave now" prompt after scripted modal lanes.
#[allow(clippy::too_many_arguments)]
fn drive_leave_mission_prompt(
    host: &mut Host,
    manager: &mut robin_engine::engine_manager::EngineManager,
    assets: &robin_engine::engine::LevelAssets,
    window: &mut GameWindow,
    audio: &mut super::interactive::MissionAudio,
    resources: &mut super::interactive::MissionResources,
    ui: &mut super::interactive::MissionUi,
    presentation: &mut super::interactive::MissionPresentation,
    _runtime: &mut super::runtime::TimelineRuntime,
    frame: &mut MissionFrame,
    mode: ScriptedModalMode,
    rendered: bool,
) -> bool {
    if rendered
        || (!host.effects.has_signal(HostSignal::MissionStatePopup) && ui.active_modal.is_none())
    {
        return rendered;
    }
    if host.effects.take_signal(HostSignal::MissionStatePopup) {
        if mode == ScriptedModalMode::AutoDismiss {
            let cmd = PlayerCommand::QuitMissionRequested;
            dispatch_local_command(
                host,
                &mut manager.engine,
                &mut frame.post_commands,
                assets,
                &cmd,
            );
        } else if let Some(menu_resources) = resources.menu.as_ref() {
            let kind = engine_player_command::ModalKind::MissionState {
                kind: engine_player_command::MissionStateModalKind::LeaveMissionNow,
            };
            let replay_result = pop_matching_dismissal(&mut frame.replay_modal_dismissals, &kind);
            let message = menu_resources.menu_text.get(MT_MSG_LEAVE_MISSION_NOW);
            let message = if message.is_empty() {
                "You may leave the mission now.".to_string()
            } else {
                message
            };
            ui.active_modal = Some(ActiveModal::MissionState {
                kind,
                state: MissionStatePopupState::new(
                    &presentation.renderer,
                    menu_resources,
                    message,
                    true,
                    None,
                ),
                replay_result,
            });
        }
    }

    if ui.active_modal.is_none() {
        return rendered;
    }
    let mut modal_ctx = ModalContext {
        window,
        renderer: &mut presentation.renderer,
        cursor_res: &mut resources.cursor,
        cursor_renderer: &mut presentation.sprites.cursor_renderer,
        audio_backend: &mut audio.backend,
        sample_loader: &audio.sample_loader,
        menu_resources: &mut resources.menu,
        modal_dismissals: &mut frame.modal_dismissals,
    };
    let outcome = tick_active_modal(
        &mut ui.active_modal,
        host,
        &mut modal_ctx,
        &mut frame.replay_modal_dismissals,
    );
    if outcome == ActiveModalOutcome::QuitMissionRequested {
        let cmd = PlayerCommand::QuitMissionRequested;
        dispatch_local_command(
            host,
            &mut manager.engine,
            &mut frame.post_commands,
            assets,
            &cmd,
        );
    }
    true
}

fn drain_deferred_save_load_after_zoom(
    host: &Host,
    game: &mut Game,
    manager: &robin_engine::engine_manager::EngineManager,
    assets: &robin_engine::engine::LevelAssets,
    callbacks: &mut RustCallbacks,
    shift_held: bool,
) {
    if !manager.engine.is_zoom_possible(&host.engine_display) {
        return;
    }
    if std::mem::take(&mut game.quick_save_after_zoom) {
        let mission_id = current_mission_id(manager.engine.campaign(), &assets.profile_manager);
        callbacks.pending = Some(SaveLoadRequest::QuickSave { mission_id });
    }
    if std::mem::take(&mut game.quick_load_after_zoom) {
        callbacks.pending = Some(SaveLoadRequest::QuickLoad {
            use_backup: shift_held,
        });
    }
}

fn reset_input_after_tick_request(host: &mut Host, input: &mut super::interactive::MissionInput) {
    if !host.effects.take_signal(HostSignal::ResetInput) {
        return;
    }
    input.reset_after_engine_request();
    host.input.left_mouse_down = false;
    host.input.right_mouse_down = false;
    host.input.is_dragging = false;
    host.input.multi_selection_active = false;
    host.input.multi_unselection_active = false;
    host.input.draw_multi_selection = false;
}

impl InteractiveFrameSimulation {
    pub(super) fn new(frame: MissionFrame, flags: FrameSimulationFlags) -> Self {
        Self { frame, flags }
    }

    /// Run the deterministic tick, timeline/step bookkeeping, and scripted
    /// modal flow before handing the completed frame to presentation.
    pub(super) async fn run(
        self,
        mission: &mut InteractiveMission,
        services: &mut MissionServices<'_>,
    ) -> Result<FrameSimulationOutcome, String> {
        let state = Self::advance_simulation(self, mission, services);
        Self::drive_modals(mission, services, state).await
    }

    fn advance_simulation(
        this: Self,
        mission: &mut InteractiveMission,
        services: &mut MissionServices<'_>,
    ) -> SimulationModalState {
        let window = &mut *services.window;
        let args = services.args;
        // File-backed screenshot runs have no player to dismiss a dialogue
        // which appears before their requested frame. Use the established
        // headless auto-dismiss path while retaining normal graphical ticks.
        let auto_dismiss_modals = args.mission_start_map_output.is_some();
        let InteractiveMission { runtime, frontend } = mission;
        let MissionRuntime {
            world,
            timeline: runtime,
            control,
        } = runtime;
        let MissionSimulationPhase {
            host,
            game,
            manager,
            assets,
            dev,
        } = world.simulation_phase();
        let MissionControl {
            manual_pause,
            last_shadow_color,
            ..
        } = control;
        let resources = &mut frontend.resources;
        let ui = &mut frontend.ui;
        let presentation = &mut frontend.presentation;
        let input = &mut frontend.input;
        let Self { mut frame, flags } = this;
        let FrameSimulationFlags {
            rewind_active,
            paused,
            consumed_buffered,
            shift_held,
            modal_rendered: modal_rendered_this_frame,
            step_forward_pressed,
            step_back_pressed,
        } = flags;

        // HTTP stepping is automation-first and defaults to dismissing local
        // UI. Resolve conservatively: never accept a save/load/quit/settings
        // mutation, and leave pause-owned tasks on the pause surface.
        if crate::http_server::has_pending_steps()
            && let Some(mut task) = ui.active_ui_task.take()
        {
            let kind = task.kind();
            let outcome = task.auto_dismiss();
            task.cleanup();
            debug_assert!(matches!(
                outcome,
                UiTaskOutcome::ReturnToPause | UiTaskOutcome::QuickLoadCancelled
            ));
            if let Some(menu) = ui.pause_menu.as_mut() {
                menu.reset_after_side_menu();
                menu.seed_mouse_from_window(
                    window,
                    presentation.renderer.screen_width() as i32,
                    presentation.renderer.screen_height() as i32,
                );
            }
            input.reset_after_modal();
            tracing::debug!(?kind, "HTTP step: auto-dismissed cooperative UI task");
        }

        let tick_exit_code = Self::advance_timeline(
            runtime,
            host,
            game,
            manager,
            assets,
            dev,
            &mut frame,
            rewind_active,
            paused,
            consumed_buffered,
        );
        let history_commit_pending = frame.timeline_advances(!paused && !rewind_active);
        Self::drive_manual_steps(
            runtime,
            host,
            game,
            manager,
            assets,
            dev,
            manual_pause,
            &mut ui.active_modal,
            step_forward_pressed,
            step_back_pressed,
        );
        SimulationVisualRefresh {
            last_shadow_color,
            manager,
            host,
            dev,
            presentation,
            resources,
            window,
        }
        .run();

        SimulationModalState {
            frame,
            rewind_active,
            consumed_buffered,
            shift_held,
            modal_rendered_this_frame,
            auto_dismiss_modals,
            tick_exit_code,
            history_commit_pending,
        }
    }

    async fn drive_modals(
        mission: &mut InteractiveMission,
        services: &mut MissionServices<'_>,
        state: SimulationModalState,
    ) -> Result<FrameSimulationOutcome, String> {
        let window = &mut *services.window;
        let callbacks = &mut *services.callbacks;
        let profiles = services.profiles;
        let InteractiveMission { runtime, frontend } = mission;
        let MissionRuntime {
            world,
            timeline: runtime,
            ..
        } = runtime;
        let MissionSimulationPhase {
            host,
            game,
            manager,
            assets,
            dev,
        } = world.simulation_phase();
        let input = &mut frontend.input;
        let audio = &mut frontend.audio;
        let resources = &mut frontend.resources;
        let ui = &mut frontend.ui;
        let hud = &mut frontend.hud;
        let presentation = &mut frontend.presentation;
        let SimulationModalState {
            mut frame,
            rewind_active,
            consumed_buffered,
            shift_held,
            mut modal_rendered_this_frame,
            auto_dismiss_modals,
            tick_exit_code,
            history_commit_pending,
        } = state;

        let modal_mode = if auto_dismiss_modals {
            ScriptedModalMode::AutoDismiss
        } else {
            ScriptedModalMode::Interactive
        };
        let mut ui_task_exit_requested = false;

        // Pause-side UI is a cooperative process state just like scripted
        // modals: it receives one event batch, draws once, and returns control
        // to the mission loop. A script modal has priority and cancels the
        // local task so deterministic dialogue cannot be obscured.
        if ui.active_ui_task.is_some()
            && (ui.active_modal.is_some() || modal_state_pending(host) || auto_dismiss_modals)
        {
            // A cross-mission QuickLoad already owns an exact decoded save;
            // suspend it behind scripted dialogue rather than dropping the
            // request. Pause-side pages are local and are cancelled when an
            // authoritative modal takes over.
            if !ui
                .active_ui_task
                .as_ref()
                .is_some_and(|task| task.is_quick_load())
            {
                if let Some(mut task) = ui.active_ui_task.take() {
                    task.cleanup();
                }
                if ui.close_pause(input, presentation) {
                    callbacks.emit_app_effect(AppEffect::SetSoundMode(SoundMode::Mission));
                    if host.transport.net.is_none() {
                        callbacks.start_play_time();
                    }
                }
            }
        } else if let Some(mut task) = ui.active_ui_task.take() {
            let scene_screenshots =
                crate::http_server::take_pending_scene_screenshots(runtime.frame_number());
            if !scene_screenshots.is_empty() {
                pre_render_engine_setup(manager, host, assets.as_ref(), &mut presentation.renderer);
                update_mouse_and_cursor(
                    manager,
                    host,
                    assets.as_ref(),
                    dev,
                    &mut frame.post_external_actions,
                    &mut presentation.renderer,
                    &mut resources.cursor,
                    &mut presentation.sprites.cursor_renderer,
                    &input.threaded,
                    &presentation.sprites.portrait_cache,
                    shift_held,
                    &mut hud.last_cursor_id,
                );
                let display_snapshot = host.engine_display.clone();
                let mut render_context = presentation.render_context(
                    resources,
                    hud,
                    input,
                    ui,
                    game,
                    RenderViewState {
                        shift_held,
                        rewind_active,
                        display_info_elapsed_secs:
                            <RustCallbacks as crate::game::GameCallbacks>::get_current_playing_time(
                                callbacks,
                                manager.engine.campaign(),
                            ),
                    },
                );
                drain_screenshot_requests(
                    scene_screenshots,
                    &manager.engine,
                    &display_snapshot,
                    host,
                    assets.as_ref(),
                    dev,
                    &mut render_context,
                );
                post_render_engine_cleanup(&mut frame, host);
            }
            let menu_resources =
                required_menu_resources(&resources.menu, "cooperative pause side-screen rendering");
            let cursor = default_modal_cursor(
                &mut presentation.sprites.cursor_renderer,
                &mut resources.cursor,
                &mut presentation.renderer,
            );
            let task_outcome = task.tick(
                window,
                &mut presentation.renderer,
                menu_resources,
                Some(&cursor),
                &mut callbacks.save_manager,
                Some(profiles),
                Some(&mut host.audio.sound),
                audio
                    .backend
                    .as_mut()
                    .map(|backend| backend as &mut dyn crate::sound::AudioBackend),
                Some(&audio.sample_loader),
            );
            modal_rendered_this_frame = true;
            drain_presented_ui_screenshots(runtime.frame_number(), &presentation.renderer);

            if let Some(outcome) = task_outcome {
                task.cleanup();
                match outcome {
                    UiTaskOutcome::ReturnToPause => {
                        if let Some(menu) = ui.pause_menu.as_mut() {
                            menu.reset_after_side_menu();
                            menu.seed_mouse_from_window(
                                window,
                                presentation.renderer.screen_width() as i32,
                                presentation.renderer.screen_height() as i32,
                            );
                        }
                        input.reset_after_modal();
                    }
                    UiTaskOutcome::OptionsAccepted(result) => {
                        if result.changed {
                            host.application_context
                                .with_player_profiles_mut(|manager| {
                                    let profile = manager
                                        .profiles
                                        .iter_mut()
                                        .find(|profile| profile.id == result.profile_id)
                                        .expect(
                                            "Options profile disappeared while side task was open",
                                        );
                                    profile.graphic_config = result.graphic_config.clone();
                                    profile.gameplay_config = result.gameplay_config;
                                    profile.sound_config = result.sound_config;
                                    if let Err(error) = manager.save() {
                                        tracing::error!(
                                            "Options: failed to save profile manager: {error:#}"
                                        );
                                    }
                                })
                                .unwrap_or_else(|error| {
                                    panic!("Options profile update failed: {error}")
                                });
                        }

                        host.frontend.key_config = result.key_config.clone();
                        host.frontend.custom_key_config = result.custom_key_config.clone();
                        host.control_tactical_units = result.gameplay_config.control_tactical_units;
                        if !host.control_tactical_units {
                            dispatch_local_command(
                                host,
                                &mut manager.engine,
                                &mut frame.post_commands,
                                assets.as_ref(),
                                &PlayerCommand::ReleaseTacticalControl,
                            );
                        }
                        if result.sound_config.amount_of_speaking
                            != result.original_amount_of_speaking
                        {
                            dispatch_local_command(
                                host,
                                &mut manager.engine,
                                &mut frame.post_commands,
                                assets.as_ref(),
                                &PlayerCommand::SetAmountOfSpeaking {
                                    amount: result.sound_config.amount_of_speaking,
                                },
                            );
                        }
                        if result.gameplay_config.fix_hard_reaction_times
                            != result.original_gameplay_config.fix_hard_reaction_times
                        {
                            dispatch_local_command(
                                host,
                                &mut manager.engine,
                                &mut frame.post_commands,
                                assets.as_ref(),
                                &PlayerCommand::SetFixHardReactionTimes {
                                    enabled: result.gameplay_config.fix_hard_reaction_times,
                                },
                            );
                        }
                        if result.gameplay_config.enable_unbinding
                            != result.original_gameplay_config.enable_unbinding
                        {
                            dispatch_local_command(
                                host,
                                &mut manager.engine,
                                &mut frame.post_commands,
                                assets.as_ref(),
                                &PlayerCommand::SetUnbindingEnabled {
                                    enabled: result.gameplay_config.enable_unbinding,
                                },
                            );
                        }
                        if result.gameplay_config.reusable_cloaks
                            != result.original_gameplay_config.reusable_cloaks
                        {
                            dispatch_local_command(
                                host,
                                &mut manager.engine,
                                &mut frame.post_commands,
                                assets.as_ref(),
                                &PlayerCommand::SetReusableCloaks {
                                    enabled: result.gameplay_config.reusable_cloaks,
                                },
                            );
                        }

                        presentation
                            .renderer
                            .set_scale_mode(result.graphic_config.scale_mode);
                        presentation
                            .renderer
                            .set_shader_preset(result.graphic_config.shader_preset.clone());
                        if let Some(backend) = audio.backend.as_mut() {
                            host.audio.sound.apply_sound_settings(
                                false,
                                backend,
                                &result.sound_config,
                                None,
                            );
                        } else {
                            host.audio.sound.apply_volumes(&result.sound_config);
                        }

                        if result.resolution_changed {
                            let width = result.graphic_config.resolution_x;
                            let height = result.graphic_config.resolution_y;
                            let width_u16 = width.round() as u16;
                            let height_u16 = height.round() as u16;
                            window.set_logical_size(width_u16 as u32, height_u16 as u32);
                            host.viewport.set_screen_size(width, height);
                            presentation.renderer.resize(width_u16, height_u16);
                            input.resize(width_u16 as u32, height_u16 as u32, &result.key_config);
                            hud.resize(width_u16 as u32, height_u16 as u32);
                            if host.minimap_corner_size.x > 0.0 {
                                dispatch_local_command(
                                    host,
                                    &mut manager.engine,
                                    &mut frame.post_commands,
                                    assets.as_ref(),
                                    &PlayerCommand::MinimapResize {
                                        base: engine_coordinates::ScreenPoint::new(
                                            width - 83.0,
                                            38.0,
                                        ),
                                        corner_size: host.minimap_corner_size,
                                    },
                                );
                            }
                            game.reshow_campaign_map();
                        } else if result.key_config_changed {
                            input
                                .translator
                                .load_bindings_from_keyconfig(&result.key_config);
                        }

                        if result.key_config_changed {
                            host.application_context
                                .with_key_configs_mut(|store| {
                                    let entry = store.entry_or_default(result.profile_id);
                                    entry.active = result.key_config.clone();
                                    entry.custom = result.custom_key_config.clone();
                                    if let Err(error) = store.save() {
                                        tracing::error!(
                                            "Options: failed to save key configs: {error:#}"
                                        );
                                    }
                                })
                                .unwrap_or_else(|error| {
                                    panic!("Options key-config update failed: {error}")
                                });
                            host.minimap_fast_key =
                                input.translator.get_binding(GameKey::DisplayMap);
                        }
                        if let Some(menu) = ui.pause_menu.as_mut() {
                            menu.reset_after_side_menu();
                            menu.seed_mouse_from_window(
                                window,
                                presentation.renderer.screen_width() as i32,
                                presentation.renderer.screen_height() as i32,
                            );
                        }
                        input.reset_after_modal();
                    }
                    UiTaskOutcome::SaveLoadSelected {
                        mode,
                        filename,
                        mission_id,
                    } => {
                        let slot = callbacks
                            .save_manager
                            .find_by_filename(&filename)
                            .unwrap_or_else(|| {
                                panic!("selected save slot '{filename}' disappeared")
                            });
                        callbacks.pending = Some(match mode {
                            SaveLoadMode::Save => SaveLoadRequest::Save {
                                slot: Some(slot),
                                mission_id,
                            },
                            SaveLoadMode::Load => SaveLoadRequest::Load {
                                slot: Some(slot),
                                mission_id,
                                save: None,
                            },
                        });
                        ui.pause_menu = None;
                        presentation.renderer.clear_frozen_scene();
                        input.reset_after_modal();
                        callbacks.emit_app_effect(AppEffect::SetSoundMode(SoundMode::Mission));
                        if host.transport.net.is_none() {
                            callbacks.start_play_time();
                        }
                    }
                    UiTaskOutcome::QuickLoadAccepted {
                        slot,
                        mission_id,
                        save,
                    } => {
                        callbacks.pending = Some(SaveLoadRequest::Load {
                            slot: Some(slot),
                            mission_id,
                            save: Some(*save),
                        });
                        input.reset_after_modal();
                    }
                    UiTaskOutcome::QuickLoadCancelled => {
                        input.reset_after_modal();
                    }
                    UiTaskOutcome::QuitMissionRequested | UiTaskOutcome::ExitRequested => {
                        callbacks.emit_app_effect(AppEffect::SetSoundMode(SoundMode::Mission));
                        ui_task_exit_requested = true;
                    }
                }
            } else {
                ui.active_ui_task = Some(task);
            }
        }

        if auto_dismiss_modals {
            let dismissed = dismiss_pending_modals(host);
            let active_dismissed = usize::from(ui.active_modal.take().is_some());
            if dismissed + active_dismissed > 0 {
                tracing::debug!(
                    dismissed = dismissed + active_dismissed,
                    "mission map render: auto-dismissed pending modal(s)"
                );
            }
            modal_rendered_this_frame = false;
        } else {
            modal_rendered_this_frame = drive_scripted_modal_lanes(
                host,
                game,
                manager,
                profiles,
                window,
                audio,
                resources,
                ui,
                presentation,
                runtime,
                &mut frame,
                ScriptedModalMode::Interactive,
                modal_rendered_this_frame,
            )
            .await;
        }

        drain_pending_console_display(host, &mut ui.console_overlay);

        modal_rendered_this_frame = drive_leave_mission_prompt(
            host,
            manager,
            assets.as_ref(),
            window,
            audio,
            resources,
            ui,
            presentation,
            runtime,
            &mut frame,
            modal_mode,
            modal_rendered_this_frame,
        );

        drain_deferred_save_load_after_zoom(
            host,
            game,
            manager,
            assets.as_ref(),
            callbacks,
            shift_held,
        );
        reset_input_after_tick_request(host, input);

        if ui_task_exit_requested {
            let mut display = std::mem::take(&mut host.engine_display);
            let post_initialized = runtime.cross_post_initialize(|| {
                crate::sim_timeline::run_post_initialize_stage_with_actions(
                    host,
                    &mut display,
                    assets,
                    &mut manager.engine,
                    dev,
                    frame.unapplied_post_external_actions(),
                    &frame.post_commands.commands,
                    frame.run_post_initialize,
                )
            });
            frame.run_post_initialize = post_initialized;
            host.engine_display = display;
            if history_commit_pending {
                runtime.commit_simulation_history(
                    host,
                    manager,
                    &frame,
                    FrameCommitPolicy {
                        store_rewind_commands: !consumed_buffered,
                    },
                );
            }
            runtime.finish_recording(&mut frame);
            runtime.trace(FrameContractStage::Exit);
            return Ok(FrameSimulationOutcome::Control(FrameControl::exit(
                GameCode::Quit,
            )));
        }

        if drive_tick_exit_modals(TerminalDebriefingContext {
            tick_exit_code,
            host,
            game,
            manager,
            assets: assets.as_ref(),
            window,
            callbacks,
            input,
            audio,
            resources,
            ui,
            presentation,
            frame: &mut frame,
        })
        .await
        {
            let mut display = std::mem::take(&mut host.engine_display);
            let post_initialized = runtime.cross_post_initialize(|| {
                crate::sim_timeline::run_post_initialize_stage_with_actions(
                    host,
                    &mut display,
                    assets,
                    &mut manager.engine,
                    dev,
                    frame.unapplied_post_external_actions(),
                    &frame.post_commands.commands,
                    frame.run_post_initialize,
                )
            });
            frame.run_post_initialize = post_initialized;
            host.engine_display = display;
            if history_commit_pending {
                runtime.commit_simulation_history(
                    host,
                    manager,
                    &frame,
                    FrameCommitPolicy {
                        store_rewind_commands: !consumed_buffered,
                    },
                );
            }
            runtime.finish_recording(&mut frame);
            runtime.trace(FrameContractStage::Exit);
            return Ok(FrameSimulationOutcome::Control(FrameControl::exit(
                GameCode::Quit,
            )));
        }

        runtime.trace(FrameContractStage::ModalDrain);
        Ok(FrameSimulationOutcome::Present(FramePresentationHandoff {
            frame,
            rewind_active,
            consumed_buffered,
            shift_held,
            modal_rendered: modal_rendered_this_frame,
            history_commit_pending,
        }))
    }

    /// Record commands, advance the engine, service script RPC, and commit
    /// the resulting deterministic history before any manual stepping.
    fn advance_timeline(
        runtime: &mut super::runtime::TimelineRuntime,
        host: &mut Host,
        game: &mut crate::game::Game,
        manager: &mut robin_engine::engine_manager::EngineManager,
        assets: &std::sync::Arc<robin_engine::engine::LevelAssets>,
        dev: &mut robin_engine::engine::DevState,
        frame: &mut MissionFrame,
        rewind_active: bool,
        paused: bool,
        consumed_buffered: bool,
    ) -> Option<GameCode> {
        // ── Record frame commands + periodic state hash ──
        // The matching `recorder.end_frame()` runs after the modal
        // drain block so `ModalDismiss` entries land in the same
        // frame as the modal that produced them.  Skipped while
        // rewinding (no tick is running) and while consuming buffered
        // commands (they were already written to disk on the original
        // pass). The hash itself was computed at the top of the
        // frame into `frame.recorder_hash` — writing it here
        // keeps the gating in one place.
        runtime.begin_recording(frame, !rewind_active && !consumed_buffered);
        runtime.trace(FrameContractStage::Simulation);

        // ── Engine tick ──
        // The pause menu freezes the simulation by skipping the
        // hourglass while the menu is shown.  Rewind also freezes
        // the tick: the engine state was just replaced with a
        // reconstruction of an earlier frame and must not be
        // advanced this frame.
        let tick_exit_code = runtime.run_simulation(|| {
            if rewind_active {
                return None;
            }
            let mut display = std::mem::take(&mut host.engine_display);
            let mission_transitioning = !game
                .operation
                .is(robin_engine::game_operation::GameCode::LevelInProgress);
            frame.run_hourglass &= game.should_run_hourglass(false, mission_transitioning, paused);
            let simulation_frame = frame.hourglass_input();
            let result = game.run_engine_tick(
                host,
                &mut display,
                assets.as_ref(),
                &mut manager.engine,
                dev,
                simulation_frame,
                false,
                paused,
            );
            host.engine_display = display;
            result
        });

        // ── Drain pending script-RPC requests ──
        // External tools (HTTP /native, /command, /console, /state, …)
        // queue invocations on the server thread; we run them here so
        // any side-effect commands (camera, dialog, sequences, sound,
        // PlayerCommand applies) land on the same frame as the tick
        // that just finished.  No-op when the HTTP server is disabled
        // or the mission isn't loaded yet (each handler returns an
        // `Err` that's relayed back).
        let pending_actions = frame.unapplied_post_external_actions().to_vec();
        if !pending_actions.is_empty() {
            let mut display = std::mem::take(&mut host.engine_display);
            crate::sim_timeline::run_post_external_action_stage(
                host,
                &mut display,
                assets,
                &mut manager.engine,
                dev,
                &pending_actions,
            );
            host.engine_display = display;
            frame.mark_post_external_actions_applied();
        }
        let net = host.transport.net.take();
        let actions = crate::http_server::drain_global(
            manager,
            host,
            assets,
            net.as_ref(),
            &mut frame.post_commands,
        );
        frame.record_applied_post_external_actions(actions);
        host.transport.net = net;

        // ── Rollback check + rewind buffer commit ──
        // Both are post-tick bookkeeping.  Skipped on paused frames
        // (no tick ran) and rewind frames (tick was suppressed).  The
        // rewind buffer also skips commits while consuming its own
        // log — the slot is already populated and would duplicate.
        if frame.timeline_advances(!paused && !rewind_active) {
            let next_frame = runtime.advance_frame().number();
            if let Some(net) = host.transport.net.as_ref()
                && host.transport.local_seat == engine_player_command::PlayerId::HOST
            {
                net.set_initial_snapshot(next_frame, &manager.engine);
            }
        }
        frame.commit_timeline_after(runtime.current_frame());

        runtime.trace(FrameContractStage::HostRpcAndTimelineCommit);
        tick_exit_code
    }

    /// Apply queued and keyboard-driven single-frame timeline movement.
    ///
    /// This stays after the normal history commit: step-forward owns its own
    /// tick/PostInitialize boundary, while step-back replaces the live engine.
    fn drive_manual_steps(
        runtime: &mut super::runtime::TimelineRuntime,
        host: &mut Host,
        game: &mut crate::game::Game,
        manager: &mut robin_engine::engine_manager::EngineManager,
        assets: &std::sync::Arc<robin_engine::engine::LevelAssets>,
        dev: &mut robin_engine::engine::DevState,
        manual_pause: &mut bool,
        active_modal: &mut Option<ActiveModal>,
        step_forward_pressed: bool,
        step_back_pressed: bool,
    ) {
        // ── Pending `/step-forward` / `/step-back` requests ──
        // Run each queued step synchronously with its own tick +
        // bookkeeping (forward) or rewind-buffer seek (back).  These
        // requests intentionally bypass the `paused` gate — their whole
        // purpose is to drive the sim from a paused state — but still
        // refuse if a modal dialog is queued so the user doesn't step
        // past it.
        drain_steps(
            manager,
            host,
            assets.as_ref(),
            dev,
            game,
            runtime,
            manual_pause,
            active_modal,
        );

        // Publish replay-playback status for the script-RPC `state`
        // endpoint so JS timelines can render a playhead.  `None`
        // when we're not replaying — the state response will carry
        // `null` for `replay`, the JS UI's "hide me" signal.
        crate::http_server::set_replay_status(runtime.replay_player.as_ref().map(|p| {
            crate::http_server::ReplayStatus {
                frame: p.current_frame(),
                total: p.total_frames(),
                paused: *manual_pause,
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
        if step_forward_pressed && !modal_state_pending(host) {
            // Stepping into a save-marker / load-back frame must pin or
            // swap state exactly like the normal playback admission path.
            runtime
                .apply_playback_timeline_events(host, game, manager, assets)
                .unwrap_or_else(|error| panic!("replay step boundary failed: {error}"));
            let step_frame = runtime.frame_number();
            let reusing_recorded_frame = step_frame < runtime.rewind_buffer.next_record_frame();
            let buffered_frame = if reusing_recorded_frame {
                runtime.rewind_buffer.frame_for(step_frame).cloned()
            } else {
                None
            };
            if !reusing_recorded_frame || buffered_frame.is_some() {
                let (replay_input, replay_timeline_after) = match runtime
                    .consume_replay_frame_for_step()
                    .unwrap_or_else(|error| panic!("replay step admission failed: {error}"))
                {
                    super::runtime::ReplayStepAdmission::NoActiveReplay => (None, None),
                    super::runtime::ReplayStepAdmission::Recorded(recorded) => {
                        assert_eq!(recorded.timeline_before, step_frame);
                        (
                            Some(recorded.input),
                            Some(super::runtime::TimelineFrame::from_wire(
                                recorded.timeline_after,
                            )),
                        )
                    }
                    super::runtime::ReplayStepAdmission::Finished {
                        ordinal,
                        total_frames,
                    } => {
                        tracing::warn!(
                            timeline_frame = step_frame,
                            replay_ordinal = ordinal,
                            replay_total_frames = total_frames,
                            "step-forward: replay is already finished"
                        );
                        return;
                    }
                };

                runtime
                    .rewind_buffer
                    .begin_frame(step_frame, &manager.engine, assets);

                let simulation_frame = match (buffered_frame, replay_input) {
                    (Some(buffered), _) => buffered,
                    (None, Some(recorded)) => recorded,
                    (None, None) => robin_engine::engine::SimulationFrameInput::default()
                        .with_post_initialize(true),
                };
                let mut display = std::mem::take(&mut host.engine_display);
                game.run_engine_tick(
                    host,
                    &mut display,
                    assets.as_ref(),
                    &mut manager.engine,
                    dev,
                    simulation_frame.clone(),
                    false,
                    false,
                );
                host.engine_display = display;
                if !reusing_recorded_frame {
                    runtime.rewind_buffer.end_frame_input(simulation_frame);
                }
                if let Some(after) = replay_timeline_after {
                    runtime.adopt_frame(after);
                } else {
                    runtime.advance_frame();
                }
                // Stepping bypasses the checker's begin_frame/end_frame
                // pairing, so its ring buffer is now stale relative to the
                // advanced engine.  Clear it — the checker resumes
                // populating on the next normal frame.
                if let Some(ref mut checker) = runtime.rollback_checker {
                    checker.reset();
                }
            } else {
                tracing::warn!(
                    frame = step_frame,
                    oldest_command_frame = runtime.rewind_buffer.oldest_cmd_frame(),
                    "step-forward: frame lies inside recorded history but its commands are missing"
                );
            }
        } else if step_back_pressed && !modal_state_pending(host) {
            if let Some(target) = runtime.current_frame().previous()
                && let Some(oldest) = runtime.rewind_buffer.oldest_reachable_frame()
                && target.number() >= oldest
            {
                runtime.rewind_buffer.begin_session();
                let restored = runtime.restore_retained_frame(manager, assets, target);
                runtime.rewind_buffer.end_session();
                if !restored {
                    tracing::warn!("step-back: rewind_to({}) failed", target.number());
                }
            } else {
                tracing::debug!("step-back: already at oldest retained frame");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ScriptedModalMode;

    #[test]
    fn scripted_modal_mode_roundtrips_for_phase_handoffs() {
        for mode in [
            ScriptedModalMode::Interactive,
            ScriptedModalMode::AutoDismiss,
        ] {
            let encoded = serde_json::to_string(&mode).expect("serialize modal mode");
            let decoded: ScriptedModalMode =
                serde_json::from_str(&encoded).expect("deserialize modal mode");
            assert_eq!(decoded, mode);
        }
    }
}
