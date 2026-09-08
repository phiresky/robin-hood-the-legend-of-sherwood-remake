//! Deterministic interactive-frame simulation and modal orchestration.
//!
//! Input preparation remains in `flow`. This phase owns the exact post-input
//! order from command recording through simulation history, stepping, scripted
//! modals, and the handoff to presentation.

use super::debriefing::{LostSherwoodGateProgress, drive_lost_sherwood_gate};
use super::flow::{FrameControl, MissionServices};
use super::interactive::{MissionPresentation, MissionResources};
use super::runtime::FrameContractStage;
use super::terminal_debriefing::{
    TerminalDebriefingContext, TerminalDebriefingProgress, drive_tick_exit_modals,
};
use super::tick::run_forward_ticks;
use super::ui_task_state::{ActiveUiTask, UiTaskKind, UiTaskOutcome};
use super::*;
use crate::game::Game;
use crate::host::HostSignal;
use crate::ingame_menu::widget_bridge::default_modal_cursor;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum UiTaskModalAdmission {
    Run,
    Suspend,
    Cancel,
}

/// Terminal presentation owns its leaderboard child task just as it owns the
/// debriefing pages. Scripted batches are retained but deferred by that owner;
/// they cannot also suspend the child that the terminal flow is waiting on.
fn ui_task_modal_admission(
    task: UiTaskKind,
    terminal_flow_active: bool,
    scripted_modal_preempts: bool,
) -> UiTaskModalAdmission {
    if !scripted_modal_preempts
        || (terminal_flow_active && task == UiTaskKind::MissionEndLeaderboard)
    {
        return UiTaskModalAdmission::Run;
    }
    match task {
        UiTaskKind::QuickLoadConfirmation | UiTaskKind::MissionEndLeaderboard => {
            UiTaskModalAdmission::Suspend
        }
        UiTaskKind::Options | UiTaskKind::SaveLoad | UiTaskKind::QuitConfirmation => {
            UiTaskModalAdmission::Cancel
        }
    }
}

/// Host-only visual state which must be refreshed immediately after the
/// deterministic tick but before scripted modal drains.
struct SimulationVisualRefresh<'a> {
    last_shadow_color: &'a mut u16,
    last_visual_ambiance: &'a mut robin_engine::engine::Ambiance,
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
            last_visual_ambiance,
            manager,
            host,
            dev,
            presentation,
            resources,
            window,
        } = self;

        let dynamic_visuals = host
            .application_context()
            .active_profile_snapshot()
            .map(|profile| profile.graphic_config.dynamic_ambience_visuals)
            .unwrap_or(true);
        let current_visual_ambiance = if dynamic_visuals {
            manager.engine.weather().ambiance
        } else {
            manager.engine.initial_mission_ambiance()
        };
        let current_shadow_color = if dynamic_visuals {
            manager.engine.weather().night_color
        } else {
            manager.engine.initial_mission_night_color()
        };
        let ambiance_changed = current_visual_ambiance != *last_visual_ambiance;
        if ambiance_changed {
            presentation.apply_ambience_maps(&manager.engine, host, current_visual_ambiance);
            *last_visual_ambiance = current_visual_ambiance;
        }
        if current_shadow_color != *last_shadow_color || ambiance_changed {
            tracing::info!(
                "Ambience shadow-key changed {:#06x} → {:#06x}; rebinding sprite caches",
                last_shadow_color,
                current_shadow_color,
            );
            presentation.rebind_shadow_key(
                resources,
                host,
                &window.gpu,
                current_shadow_color,
                current_visual_ambiance,
                manager.engine.sim_config().bypass_fog_sprites_crash,
            );
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
    manager: &mut robin_engine::engine_manager::EngineManager,
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
    if !rendered
        && ui.active_modal.is_none()
        && host.effects.has_signal(HostSignal::SherwoodTrading)
    {
        let access = sherwood_trading_access(host, &manager.engine, profiles);
        match host.effects.take_sherwood_trading(access) {
            Ok(false) => {}
            Err(reason) => {
                tracing::warn!(?reason, "queued Sherwood trading-panel request rejected");
            }
            Ok(true) if auto_dismiss => {
                tracing::debug!("headless mode ignored a local Sherwood trading-panel request");
            }
            Ok(true) => {
                let Some(menu_resources) = resources.menu.as_ref() else {
                    tracing::warn!("Sherwood trading: menu resources unavailable — skipped");
                    return rendered;
                };
                let sectors = manager.engine.live_tradable_production_sectors(profiles);
                let ransom = crate::ingame_menu::trading::ransom_from_engine(&manager.engine);
                ui.active_modal = Some(ActiveModal::Trading(Box::new(
                    crate::ingame_menu::TradingModalState::new(
                        window,
                        &presentation.renderer,
                        menu_resources,
                        &sectors,
                        ransom,
                    ),
                )));
            }
        }
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
        if !rendered {
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
            drain_pending_debriefings(
                host,
                &mut modal_ctx,
                &mut resources.text,
                &resources.level_descriptors,
                &mut frame.replay_modal_dismissals,
            )
            .await;
        }
        return rendered;
    }

    use super::session_policy::{ScriptedModalLane, take_next_scripted_batch};
    use engine_player_command::ModalKind;
    loop {
        let mut processed = false;
        while !rendered && ui.active_modal.is_none() {
            let Some((lane, items)) = take_next_scripted_batch(&mut host.effects, false) else {
                break;
            };
            ui.active_modal = match lane {
                ScriptedModalLane::Dialogue => start_active_dialogue_batch(
                    items
                        .into_iter()
                        .map(|kind| match kind {
                            ModalKind::Dialog { dialog_id } => dialog_id,
                            _ => unreachable!("dialogue lane identity"),
                        })
                        .collect(),
                    &mut resources.text,
                    game,
                    &resources.level_descriptors,
                )
                .map(|batch| ActiveModal::Dialogue(Box::new(batch))),
                ScriptedModalLane::Popup => start_active_popup_scroll_batch(
                    items
                        .into_iter()
                        .map(|kind| match kind {
                            ModalKind::PopupText { text_id } => text_id,
                            _ => unreachable!("popup lane identity"),
                        })
                        .collect(),
                    &mut modal_ctx,
                    &mut resources.text,
                    &resources.level_descriptors,
                    manager.engine.simulation_tick().number(),
                )
                .map(|batch| ActiveModal::PopupScroll(Box::new(batch))),
                ScriptedModalLane::SherwoodReport => {
                    start_active_sherwood_report(host, &mut modal_ctx, &manager.engine, profiles)
                        .map(|batch| ActiveModal::PopupScroll(Box::new(batch)))
                }
                ScriptedModalLane::Debriefing => start_active_debriefing_batch(
                    items
                        .into_iter()
                        .map(|kind| match kind {
                            ModalKind::Debriefing { text_id } => text_id,
                            _ => unreachable!("debriefing lane identity"),
                        })
                        .collect(),
                    &mut modal_ctx,
                    &mut resources.text,
                    &resources.level_descriptors,
                )
                .map(|batch| ActiveModal::Debriefing(Box::new(batch))),
                ScriptedModalLane::LeaveMission => {
                    unreachable!("leave prompt follows scripted presentation")
                }
            };
        }
        if !rendered && ui.active_modal.is_some() {
            let outcome = tick_active_modal(
                &mut ui.active_modal,
                host,
                &mut modal_ctx,
                &mut frame.replay_modal_dismissals,
                &manager.engine,
                profiles,
            );
            dispatch_trading_modal_outcome(outcome, host, &mut frame.post_commands);
            rendered = true;
            processed = true;
        }
        if processed && ui.active_modal.is_none() && !frame.replay_modal_dismissals.is_empty() {
            rendered = false;
            continue;
        }
        break;
    }
    rendered
}

fn dispatch_trading_modal_outcome(
    outcome: ActiveModalOutcome,
    host: &mut Host,
    post_commands: &mut engine_player_command::FrameCommands,
) {
    match outcome {
        ActiveModalOutcome::None => {}
        ActiveModalOutcome::SellSherwoodItem {
            request_id,
            prod_type,
            quantity,
        } => dispatch_local_command(
            &host.transport,
            post_commands,
            &PlayerCommand::CampaignSellProductionItem {
                request_id,
                prod_type,
                quantity,
            },
        ),
        ActiveModalOutcome::QuitMissionRequested => {
            debug_assert!(false, "mission-state modal reached scripted modal lanes")
        }
    }
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
            dispatch_local_command(&host.transport, &mut frame.post_commands, &cmd);
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
                awaiting_authority: false,
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
        &manager.engine,
        &assets.profile_manager,
    );
    if outcome == ActiveModalOutcome::QuitMissionRequested {
        let cmd = PlayerCommand::QuitMissionRequested;
        dispatch_local_command(&host.transport, &mut frame.post_commands, &cmd);
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
    if !manager
        .engine
        .is_zoom_possible(&host.frontend.engine_display)
    {
        return;
    }
    if std::mem::take(&mut game.quick_save_after_zoom) {
        let mission_id = current_mission_id(manager.engine.campaign(), &assets.profile_manager);
        callbacks.queue_operation(SaveLoadRequest::QuickSave { mission_id });
    }
    if std::mem::take(&mut game.quick_load_after_zoom) {
        if host.transport.authoritative_transition_actions_enabled() {
            callbacks.queue_operation(SaveLoadRequest::QuickLoad {
                use_backup: shift_held,
            });
        } else {
            game.display_message(
                "Quick Load is available only to the multiplayer host after synchronization finishes."
                    .to_string(),
                100,
            );
        }
    }
}

fn reset_input_after_tick_request(host: &mut Host, input: &mut super::interactive::MissionInput) {
    if !host.effects.take_signal(HostSignal::ResetInput) {
        return;
    }
    input.reset_after_engine_request(host);
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
        let InteractiveMission {
            runtime, frontend, ..
        } = mission;
        let MissionRuntime {
            world,
            timeline: runtime,
            control,
            leaderboard: _,
        } = runtime;
        let MissionMutation {
            host,
            game,
            manager,
            assets,
            dev,
        } = world.mutation();
        let MissionControl {
            last_shadow_color,
            last_visual_ambiance,
            ..
        } = control;
        let resources = &mut frontend.resources;
        let presentation = &mut frontend.presentation;
        let Self { mut frame, flags } = this;
        let FrameSimulationFlags {
            rewind_active,
            paused,
            consumed_buffered,
            shift_held,
            modal_rendered: modal_rendered_this_frame,
        } = flags;

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
        SimulationVisualRefresh {
            last_shadow_color,
            last_visual_ambiance,
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
        callbacks.poll_leaderboard_submissions();
        let profiles = services.profiles;
        let InteractiveMission {
            runtime, frontend, ..
        } = mission;
        let MissionRuntime {
            world,
            timeline: runtime,
            leaderboard,
            ..
        } = runtime;
        let MissionMutation {
            host,
            game,
            manager,
            assets,
            dev,
        } = world.mutation();
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
        let task_admission = ui.active_ui_task.as_ref().map(|task| {
            ui_task_modal_admission(
                task.kind(),
                ui.terminal_flow_active(),
                ui.active_modal.is_some() || modal_state_pending(host) || auto_dismiss_modals,
            )
        });
        if matches!(
            task_admission,
            Some(UiTaskModalAdmission::Suspend | UiTaskModalAdmission::Cancel)
        ) {
            // A cross-mission QuickLoad already owns an exact decoded save;
            // suspend it behind scripted dialogue rather than dropping the
            // request. Pause-side pages are local and are cancelled when an
            // authoritative modal takes over.
            if task_admission == Some(UiTaskModalAdmission::Cancel) {
                if let Some(mut task) = ui.active_ui_task.take() {
                    task.cleanup();
                }
                if ui.close_pause(host, input, presentation) {
                    callbacks.emit_app_effect(AppEffect::SetSoundMode(SoundMode::Mission));
                }
            }
        } else if let Some(mut task) = ui.active_ui_task.take() {
            let task_owned_presentation = task.owns_presentation();
            let scene_screenshots =
                crate::http_server::take_pending_scene_screenshots(runtime.frame_number());
            if !scene_screenshots.is_empty() {
                pre_render_engine_setup(host);
                update_mouse_and_cursor(
                    &manager.engine,
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
                let display_snapshot = host.frontend.engine_display.clone();
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
                    &mut host.presentation(),
                    assets.as_ref(),
                    dev,
                    &mut render_context,
                );
                post_render_engine_cleanup(&mut frame, host.transport.local_seat);
            }
            let menu_resources =
                required_menu_resources(&resources.menu, "cooperative pause side-screen rendering");
            let cursor = default_modal_cursor(
                &mut presentation.sprites.cursor_renderer,
                &mut resources.cursor,
                &mut presentation.renderer,
            );
            let application_context = host.application_context().clone();
            let task_outcome = task.tick(
                &application_context,
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
            if task_owned_presentation {
                modal_rendered_this_frame = true;
                drain_presented_ui_screenshots(runtime.frame_number(), &presentation.renderer);
            }

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
                        input.reset_after_modal(host);
                    }
                    UiTaskOutcome::OptionsAccepted(result) => {
                        if result.changed {
                            host.application_context()
                                .with_player_profiles_mut(|manager| {
                                    let profile = manager
                                        .profiles
                                        .iter_mut()
                                        .find(|profile| profile.id == result.profile_id)
                                        .expect(
                                            "Options profile disappeared while side task was open",
                                        );
                                    profile.graphic_config = result.graphic_config.clone();
                                    profile.gameplay_config = result.profile_gameplay_config;
                                    profile.multiplayer_config = result.multiplayer_config;
                                    profile.sound_config = result.profile_sound_config;
                                    if let Err(error) =
                                        host.application_context().persist_player_profiles(manager)
                                    {
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
                        host.frontend.control_tactical_units =
                            result.profile_gameplay_config.control_tactical_units;
                        host.frontend
                            .planning
                            .update_preference(result.profile_gameplay_config.plan_quick_actions);
                        if !host.frontend.planning.enabled() {
                            dispatch_local_command(
                                &host.transport,
                                &mut frame.post_commands,
                                &PlayerCommand::CancelPlannedAction,
                            );
                        }
                        host.frontend.touch_camera_gestures =
                            result.profile_gameplay_config.touch_camera_gestures;
                        host.frontend.gameplay_config = result.profile_gameplay_config;
                        host.frontend.native_refresh_presentation =
                            result.graphic_config.native_refresh_presentation;
                        host.frontend.quick_action_cursor_pulse =
                            result.graphic_config.quick_action_cursor_pulse;
                        host.frontend.diplomacy_visuals = result.graphic_config.diplomacy_visuals;
                        window.set_native_refresh_presentation(
                            result.graphic_config.native_refresh_presentation,
                        );
                        presentation.renderer.configure_native_refresh_presentation(
                            result.graphic_config.native_refresh_presentation,
                            window.surface_config.width,
                            window.surface_config.height,
                        );
                        if !host.frontend.control_tactical_units {
                            dispatch_local_command(
                                &host.transport,
                                &mut frame.post_commands,
                                &PlayerCommand::ReleaseTacticalControl,
                            );
                        }
                        if result.sound_config.amount_of_speaking
                            != result.original_amount_of_speaking
                        {
                            dispatch_local_command(
                                &host.transport,
                                &mut frame.post_commands,
                                &PlayerCommand::SetAmountOfSpeaking {
                                    amount: result.sound_config.amount_of_speaking,
                                },
                            );
                        }
                        if result.gameplay_config.fix_hard_reaction_times
                            != result.original_gameplay_config.fix_hard_reaction_times
                        {
                            dispatch_local_command(
                                &host.transport,
                                &mut frame.post_commands,
                                &PlayerCommand::SetFixHardReactionTimes {
                                    enabled: result.gameplay_config.fix_hard_reaction_times,
                                },
                            );
                        }
                        if result.gameplay_config.enable_unbinding
                            != result.original_gameplay_config.enable_unbinding
                        {
                            dispatch_local_command(
                                &host.transport,
                                &mut frame.post_commands,
                                &PlayerCommand::SetUnbindingEnabled {
                                    enabled: result.gameplay_config.enable_unbinding,
                                },
                            );
                        }
                        if result.gameplay_config.clean_hands_npc_kills_invalidate
                            != result
                                .original_gameplay_config
                                .clean_hands_npc_kills_invalidate
                        {
                            dispatch_local_command(
                                &host.transport,
                                &mut frame.post_commands,
                                &PlayerCommand::SetCleanHandsNpcKillsInvalidate {
                                    enabled: result
                                        .gameplay_config
                                        .clean_hands_npc_kills_invalidate,
                                },
                            );
                        }
                        if result.gameplay_config.reusable_cloaks
                            != result.original_gameplay_config.reusable_cloaks
                        {
                            dispatch_local_command(
                                &host.transport,
                                &mut frame.post_commands,
                                &PlayerCommand::SetReusableCloaks {
                                    enabled: result.gameplay_config.reusable_cloaks,
                                },
                            );
                        }
                        if result.gameplay_config.item_gameplay
                            != result.original_gameplay_config.item_gameplay
                        {
                            dispatch_local_command(
                                &host.transport,
                                &mut frame.post_commands,
                                &PlayerCommand::SetItemGameplayConfig {
                                    config: result.gameplay_config.item_gameplay,
                                },
                            );
                        }
                        if result.gameplay_config.noise_distraction_feedback
                            != result.original_gameplay_config.noise_distraction_feedback
                        {
                            dispatch_local_command(
                                &host.transport,
                                &mut frame.post_commands,
                                &PlayerCommand::SetNoiseDistractionFeedback {
                                    enabled: result.gameplay_config.noise_distraction_feedback,
                                },
                            );
                        }
                        if result.gameplay_config.sherwood_trading
                            != result.original_gameplay_config.sherwood_trading
                        {
                            dispatch_local_command(
                                &host.transport,
                                &mut frame.post_commands,
                                &PlayerCommand::SetSherwoodTrading {
                                    enabled: result.gameplay_config.sherwood_trading,
                                },
                            );
                        }
                        if result.gameplay_config.enable_timed_missions
                            != result.original_gameplay_config.enable_timed_missions
                        {
                            dispatch_local_command(
                                &host.transport,
                                &mut frame.post_commands,
                                &PlayerCommand::SetTimedMissionsEnabled {
                                    enabled: result.gameplay_config.enable_timed_missions,
                                },
                            );
                        }
                        if result.gameplay_config.enable_dynamic_ambience
                            != result.original_gameplay_config.enable_dynamic_ambience
                        {
                            dispatch_local_command(
                                &host.transport,
                                &mut frame.post_commands,
                                &PlayerCommand::SetDynamicAmbienceEnabled {
                                    enabled: result.gameplay_config.enable_dynamic_ambience,
                                },
                            );
                        }
                        if result.gameplay_config.diplomacy
                            != result.original_gameplay_config.diplomacy
                        {
                            dispatch_local_command(
                                &host.transport,
                                &mut frame.post_commands,
                                &PlayerCommand::SetDiplomacyEnabled {
                                    enabled: result.gameplay_config.diplomacy,
                                },
                            );
                        }
                        if result.gameplay_config.npc_faction_wars
                            != result.original_gameplay_config.npc_faction_wars
                        {
                            dispatch_local_command(
                                &host.transport,
                                &mut frame.post_commands,
                                &PlayerCommand::SetNpcFactionWars {
                                    enabled: result.gameplay_config.npc_faction_wars,
                                },
                            );
                        }
                        if result.gameplay_config.more_combat_gestures
                            != result.original_gameplay_config.more_combat_gestures
                            || result.gameplay_config.gesture_quality_damage
                                != result.original_gameplay_config.gesture_quality_damage
                        {
                            dispatch_local_command(
                                &host.transport,
                                &mut frame.post_commands,
                                &PlayerCommand::SetCombatGestureRules {
                                    more_combat_gestures: result
                                        .gameplay_config
                                        .more_combat_gestures,
                                    gesture_quality_damage: result
                                        .gameplay_config
                                        .gesture_quality_damage,
                                },
                            );
                        }
                        if result.gameplay_config.fog_of_war
                            != result.original_gameplay_config.fog_of_war
                        {
                            dispatch_local_command(
                                &host.transport,
                                &mut frame.post_commands,
                                &PlayerCommand::SetFogOfWar {
                                    enabled: result.gameplay_config.fog_of_war,
                                },
                            );
                        }

                        presentation
                            .renderer
                            .apply_upscale_config(&result.graphic_config);
                        if let Some(backend) = audio.backend.as_mut() {
                            host.audio.sound.apply_sound_settings(
                                false,
                                backend,
                                &result.profile_sound_config,
                                None,
                            );
                        } else {
                            host.audio.sound.apply_volumes(&result.profile_sound_config);
                        }

                        if result.resolution_changed {
                            window.set_logical_resolution_policy(&result.graphic_config);
                            presentation.renderer.sync_window_size(window);
                            let (logical_width, logical_height) = window.logical_size();
                            let width = logical_width as f32;
                            let height = logical_height as f32;
                            let width_u16 = logical_width as u16;
                            let height_u16 = logical_height as u16;
                            host.frontend.viewport.set_screen_size(width, height);
                            game.set_resolution(width_u16, height_u16);
                            input.resize(logical_width, logical_height, &result.key_config);
                            hud.resize(logical_width, logical_height);
                            if host.frontend.mission_surfaces.corner_size().x > 0.0 {
                                dispatch_local_command(
                                    &host.transport,
                                    &mut frame.post_commands,
                                    &PlayerCommand::MinimapResize {
                                        base: engine_coordinates::ScreenPoint::new(
                                            width - 83.0,
                                            38.0,
                                        ),
                                        corner_size: host.frontend.mission_surfaces.corner_size(),
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
                            host.application_context()
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
                            host.frontend.minimap_fast_key =
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
                        input.reset_after_modal(host);
                    }
                    UiTaskOutcome::SaveLoadSelected {
                        mode,
                        filename,
                        mission_id,
                    } => {
                        let slot = callbacks
                            .save_manager
                            .find_by_filename(&filename)
                            .ok_or_else(|| {
                                anyhow::anyhow!("selected save slot '{filename}' disappeared")
                            })
                            .and_then(|index| callbacks.save_manager.slot_handle(index));
                        match slot {
                            Ok(slot) => callbacks.queue_operation(match mode {
                                SaveLoadMode::Save => SaveLoadRequest::Save {
                                    slot: Some(slot),
                                    mission_id,
                                },
                                SaveLoadMode::Load => SaveLoadRequest::Load {
                                    slot: Some(slot),
                                    mission_id,
                                },
                            }),
                            Err(error) => {
                                tracing::error!("Save/load selection rejected: {error:#}")
                            }
                        }
                        ui.pause_menu = None;
                        presentation.renderer.clear_frozen_scene();
                        input.reset_after_modal(host);
                        callbacks.emit_app_effect(AppEffect::SetSoundMode(SoundMode::Mission));
                    }
                    UiTaskOutcome::QuickLoadAccepted { load } => {
                        callbacks.queue_operation(SaveLoadRequest::ApplyLoad(load));
                        input.reset_after_modal(host);
                    }
                    UiTaskOutcome::QuickLoadCancelled => {
                        input.reset_after_modal(host);
                    }
                    UiTaskOutcome::QuitMissionRequested | UiTaskOutcome::ExitRequested => {
                        callbacks.emit_app_effect(AppEffect::SetSoundMode(SoundMode::Mission));
                        ui_task_exit_requested = true;
                    }
                    UiTaskOutcome::MissionEndLeaderboardFinished(controller) => {
                        if let Some(controller) = controller {
                            callbacks.detach_leaderboard_submission(controller);
                        }
                    }
                }
            } else {
                ui.active_ui_task = Some(task);
            }
        }

        let lost_sherwood_progress = drive_lost_sherwood_gate(
            &mut ui.lost_sherwood_gate,
            window,
            host,
            &manager.engine,
            game.is_sherwood,
            resources,
            presentation,
        );
        let lost_sherwood_modal_active = matches!(
            lost_sherwood_progress,
            LostSherwoodGateProgress::Pending | LostSherwoodGateProgress::Exit
        );
        let terminal_modal_active = ui.terminal_flow_active();
        if terminal_modal_active || lost_sherwood_modal_active {
            // The terminal sequence owns presentation until its typed outcome
            // settles. Network/HTTP/replay still drain through the surrounding
            // outer frame; only competing scripted surfaces are deferred.
            modal_rendered_this_frame = true;
        } else if auto_dismiss_modals {
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

        if !terminal_modal_active && !lost_sherwood_modal_active {
            drain_pending_console_display(host, &mut ui.console_overlay);
        }

        if !terminal_modal_active && !lost_sherwood_modal_active {
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
        }

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
            let mut display = std::mem::take(&mut host.frontend.engine_display);
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
            host.frontend.engine_display = display;
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

        let terminal_progress = drive_tick_exit_modals(TerminalDebriefingContext {
            tick_exit_code,
            playing_back: runtime.replay_player.is_some(),
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
            leaderboard,
            presentation,
            frame: &mut frame,
        });
        if terminal_progress == TerminalDebriefingProgress::Pending {
            modal_rendered_this_frame = true;
        }
        if terminal_progress == TerminalDebriefingProgress::EmergencyExit
            || lost_sherwood_progress == LostSherwoodGateProgress::Exit
        {
            let mut display = std::mem::take(&mut host.frontend.engine_display);
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
            host.frontend.engine_display = display;
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
            let mut display = std::mem::take(&mut host.frontend.engine_display);
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
            host.frontend.engine_display = display;
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
            let mut display = std::mem::take(&mut host.frontend.engine_display);
            crate::sim_timeline::run_post_external_action_stage(
                host,
                &mut display,
                assets,
                &mut manager.engine,
                dev,
                &pending_actions,
            );
            host.frontend.engine_display = display;
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
        runtime.record_input_taints(crate::http_server::take_pending_replay_taints());
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
    pub(super) fn drive_manual_steps(
        runtime: &mut super::runtime::TimelineRuntime,
        host: &mut Host,
        game: &mut crate::game::Game,
        manager: &mut robin_engine::engine_manager::EngineManager,
        assets: &std::sync::Arc<robin_engine::engine::LevelAssets>,
        dev: &mut robin_engine::engine::DevState,
        manual_pause: &mut bool,
        ui: &mut super::interactive::MissionUi,
        window: &crate::window::GameWindow,
        presentation: &mut super::interactive::MissionPresentation,
        input: &mut super::interactive::MissionInput,
        terminal_exit_pending: bool,
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
        let campaign_ui_blocked = ui.sherwood_campaign_flow.is_some()
            || ui
                .lost_sherwood_gate
                .blocks_mission(game.is_sherwood, &manager.engine);
        let mission_ui_block_reason = manual_step_ui_block_reason(
            terminal_exit_pending,
            ui.terminal_debriefing.is_some(),
            campaign_ui_blocked,
        );
        let active_ui_task = &mut ui.active_ui_task;
        let pause_menu = &mut ui.pause_menu;
        let mut dismissed_ui_task = false;
        drain_steps(
            manager,
            host,
            assets.as_ref(),
            dev,
            game,
            runtime,
            manual_pause,
            &mut ui.active_modal,
            ui.terminal_debriefing.as_mut(),
            mission_ui_block_reason,
            None,
            |policy| {
                let Some(kind) = active_ui_task.as_ref().map(ActiveUiTask::kind) else {
                    return Ok(());
                };
                kind.require_http_auto_dismiss(policy)?;

                let mut task = active_ui_task
                    .take()
                    .expect("validated cooperative UI task disappeared before dismissal");
                let outcome = task.auto_dismiss();
                task.cleanup();
                debug_assert!(matches!(
                    outcome,
                    UiTaskOutcome::ReturnToPause | UiTaskOutcome::QuickLoadCancelled
                ));
                if let Some(menu) = pause_menu.as_mut() {
                    menu.reset_after_side_menu();
                    menu.seed_mouse_from_window(
                        window,
                        presentation.renderer.screen_width() as i32,
                        presentation.renderer.screen_height() as i32,
                    );
                }
                dismissed_ui_task = true;
                tracing::debug!(?kind, "HTTP step: auto-dismissed cooperative UI task");
                Ok(())
            },
        );
        if dismissed_ui_task {
            input.reset_after_modal(host);
        }

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
        let mission_ui_modal_pending = terminal_exit_pending
            || campaign_ui_blocked
            || ui.terminal_flow_active()
            || ui
                .active_modal
                .as_ref()
                .is_some_and(|modal| !modal.is_empty());
        let keyboard_stepping_allowed = host.transport.net.is_none();
        if step_forward_pressed
            && keyboard_stepping_allowed
            && !modal_state_pending(&host)
            && !mission_ui_modal_pending
        {
            // Reuse the HTTP tick transaction, but do not auto-dismiss a
            // modal reached by this single interactive tick.
            let mut policy = crate::http_server::StepModalPolicy {
                auto_dismiss: false,
                ..Default::default()
            };
            if let Err(error) =
                run_forward_ticks(manager, host, assets, dev, game, runtime, 1, &mut policy)
            {
                tracing::warn!(%error, "keyboard step-forward stopped");
            }
        } else if step_back_pressed
            && keyboard_stepping_allowed
            && !modal_state_pending(&host)
            && !mission_ui_modal_pending
        {
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

// The campaign-update handoff blocks all stepping, but a constructed terminal
// modal must reach drain_steps' typed dismissal handler. That handler queues
// its result and refuses to run simulation until the outer frame applies it.
fn manual_step_ui_block_reason(
    terminal_exit_pending: bool,
    terminal_modal_active: bool,
    campaign_ui_blocked: bool,
) -> Option<&'static str> {
    if terminal_modal_active {
        None
    } else if terminal_exit_pending {
        Some("terminal mission transition")
    } else if campaign_ui_blocked {
        Some("campaign UI")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{ScriptedModalMode, UiTaskKind, UiTaskModalAdmission, ui_task_modal_admission};

    #[test]
    fn terminal_http_dismissals_become_reachable_after_campaign_handoff() {
        use super::manual_step_ui_block_reason;

        // terminal_flow_active remains true across both phases. It cannot by
        // itself decide whether typed Restart/Load outcomes may be submitted.
        assert_eq!(
            manual_step_ui_block_reason(true, false, false),
            Some("terminal mission transition")
        );
        assert_eq!(manual_step_ui_block_reason(true, true, false), None);
        assert_eq!(manual_step_ui_block_reason(false, false, false), None);
        assert_eq!(
            manual_step_ui_block_reason(false, false, true),
            Some("campaign UI")
        );
    }

    #[test]
    fn terminal_child_runs_without_discarding_deferred_scripted_modals() {
        let mut effects = crate::host::HostEffectBatches::default();
        effects.extend_dialogues([11]);
        effects.extend_popup_texts([12]);
        let deferred = effects.pending_modal_kinds();
        // The active batch and queued requests are presentation owned by the
        // terminal flow now. Admission must not drain or fabricate dismissals.
        assert_eq!(
            ui_task_modal_admission(UiTaskKind::MissionEndLeaderboard, true, true),
            UiTaskModalAdmission::Run
        );
        assert_eq!(effects.pending_modal_kinds(), deferred);
        // Without terminal ownership the same child still yields to scripts.
        assert_eq!(
            ui_task_modal_admission(UiTaskKind::MissionEndLeaderboard, false, true),
            UiTaskModalAdmission::Suspend
        );
    }

    #[test]
    fn terminal_ownership_does_not_promote_unrelated_pause_tasks() {
        for terminal in [false, true] {
            for task in [
                UiTaskKind::Options,
                UiTaskKind::SaveLoad,
                UiTaskKind::QuitConfirmation,
            ] {
                assert_eq!(
                    ui_task_modal_admission(task, terminal, true),
                    UiTaskModalAdmission::Cancel
                );
            }
            assert_eq!(
                ui_task_modal_admission(UiTaskKind::QuickLoadConfirmation, terminal, true),
                UiTaskModalAdmission::Suspend
            );
        }
    }

    #[test]
    fn every_task_runs_when_scripted_modals_do_not_preempt() {
        for terminal in [false, true] {
            for task in [
                UiTaskKind::Options,
                UiTaskKind::SaveLoad,
                UiTaskKind::QuitConfirmation,
                UiTaskKind::QuickLoadConfirmation,
                UiTaskKind::MissionEndLeaderboard,
            ] {
                assert_eq!(
                    ui_task_modal_admission(task, terminal, false),
                    UiTaskModalAdmission::Run
                );
            }
        }
    }

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
