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

#[derive(Debug, serde::Serialize, serde::Deserialize)]
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
    pub(super) execution: FrameExecutionMode,
    pub(super) shift_held: bool,
    pub(super) modal_rendered: bool,
}

/// Admitted timeline work, not independent switches. Buffered history is only
/// consumed by a running forward frame; a rewind has already replaced state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum FrameExecutionMode {
    Live,
    Paused,
    Buffered,
    Rewind,
}

impl FrameExecutionMode {
    pub(super) fn admitted(rewind: bool, paused: bool, buffered: bool) -> Self {
        assert!(
            !buffered || (!rewind && !paused),
            "buffered input requires a running forward frame"
        );
        match (rewind, paused, buffered) {
            (true, _, _) => Self::Rewind,
            (_, true, _) => Self::Paused,
            (_, _, true) => Self::Buffered,
            _ => Self::Live,
        }
    }

    fn records_commands(self) -> bool {
        matches!(self, Self::Live | Self::Paused)
    }

    fn advances_live_timeline(self) -> bool {
        matches!(self, Self::Live | Self::Buffered)
    }
}

/// One keyboard action, selected before any HTTP step changes the playhead.
/// Simultaneous bindings retain the existing forward-before-back precedence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum KeyboardStep {
    None,
    Forward,
    Back,
}

impl KeyboardStep {
    pub(super) fn from_pressed(forward: bool, back: bool) -> Self {
        match (forward, back) {
            (true, _) => Self::Forward,
            (_, true) => Self::Back,
            _ => Self::None,
        }
    }

    fn admitted(self, multiplayer: bool, host_modal: bool, mission_modal: bool) -> Self {
        if multiplayer || host_modal || mission_modal {
            Self::None
        } else {
            self
        }
    }
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
        UiTaskKind::CampaignManager
        | UiTaskKind::Options
        | UiTaskKind::SaveLoad
        | UiTaskKind::QuitConfirmation => UiTaskModalAdmission::Cancel,
    }
}

/// Host-only visual state which must be refreshed immediately after the
/// deterministic tick but before scripted modal drains.
struct SimulationVisualRefresh<'a> {
    last_shadow_color: &'a mut u16,
    last_visual_ambiance: &'a mut robin_engine::engine::Ambiance,
    engine: &'a robin_engine::engine::Engine,
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
            engine,
            host,
            dev,
            presentation,
            resources,
            window,
        } = self;

        let dynamic_visuals = host
            .application_context()
            .with_active_profile(|profile| profile.graphic_config.dynamic_ambience_visuals)
            .unwrap_or_else(|error| panic!("visual refresh requires an active profile: {error}"));
        let current_visual_ambiance = if dynamic_visuals {
            engine.weather().ambiance
        } else {
            engine.initial_mission_ambiance()
        };
        let current_shadow_color = if dynamic_visuals {
            engine.weather().night_color
        } else {
            engine.initial_mission_night_color()
        };
        let ambiance_changed = current_visual_ambiance != *last_visual_ambiance;
        if ambiance_changed {
            presentation.apply_ambience_maps(engine, host, current_visual_ambiance);
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
                current_shadow_color,
                current_visual_ambiance,
                engine.sim_config().bypass_fog_sprites_crash,
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
    engine: &robin_engine::engine::Engine,
    profiles: &engine_profiles::ProfileManager,
    window: &mut GameWindow,
    audio: &mut super::interactive::MissionAudio,
    resources: &mut super::interactive::MissionResources,
    ui: &mut super::interactive::MissionUi,
    presentation: &mut super::interactive::MissionPresentation,
    frame: &mut MissionFrame,
    mode: ScriptedModalMode,
    mut rendered: bool,
) -> bool {
    let auto_dismiss = mode == ScriptedModalMode::AutoDismiss;
    if !rendered
        && ui.active_modal.is_none()
        && host.effects.has_signal(HostSignal::SherwoodTrading)
    {
        let access = sherwood_trading_access(host, engine, profiles);
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
                let sectors = engine.live_tradable_production_sectors(profiles);
                let ransom = crate::ingame_menu::trading::ransom_from_engine(engine);
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
                engine.simulation_tick().number(),
            )
            .await;
            drain_pending_sherwood_stat(
                host,
                &mut modal_ctx,
                engine,
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
    // ModalContext borrows the frame's acknowledgement journal throughout this
    // loop. Collect its command output separately, then admit the complete FIFO
    // batch after that disjoint presentation borrow ends.
    let mut modal_commands = engine_player_command::FrameCommands::new();
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
                    engine.simulation_tick().number(),
                )
                .map(|batch| ActiveModal::PopupScroll(Box::new(batch))),
                ScriptedModalLane::SherwoodReport => {
                    start_active_sherwood_report(host, &mut modal_ctx, engine, profiles)
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
                engine,
                profiles,
            );
            dispatch_active_modal_outcome(outcome, host, &mut modal_commands);
            rendered = true;
            processed = true;
        }
        if processed && ui.active_modal.is_none() && !frame.replay_modal_dismissals.is_empty() {
            rendered = false;
            continue;
        }
        break;
    }
    frame
        .stage_post_commands()
        .commands
        .extend(modal_commands.commands);
    rendered
}

fn dispatch_active_modal_outcome(
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
            // A leave-mission prompt created after these lanes remains active
            // into the next frame, where the shared modal driver ticks it.
            // Preserve its Yes result just as on the prompt's first frame.
            dispatch_local_command(
                &host.transport,
                post_commands,
                &PlayerCommand::QuitMissionRequested,
            );
        }
    }
}

/// Drive the first mission-won "leave now" prompt after scripted modal lanes.
#[allow(clippy::too_many_arguments)]
fn drive_leave_mission_prompt(
    host: &mut Host,
    engine: &robin_engine::engine::Engine,
    assets: &robin_engine::engine::LevelAssets,
    window: &mut GameWindow,
    audio: &mut super::interactive::MissionAudio,
    resources: &mut super::interactive::MissionResources,
    ui: &mut super::interactive::MissionUi,
    presentation: &mut super::interactive::MissionPresentation,
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
            dispatch_local_command(&host.transport, &mut frame.stage_post_commands(), &cmd);
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
                dismissal: crate::ingame_menu::modal_net::ModalDismissalGate::default(),
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
        engine,
        &assets.profile_manager,
    );
    dispatch_active_modal_outcome(outcome, host, &mut frame.stage_post_commands());
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
    if !manager.engine.is_zoom_possible() {
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
            http,
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
            execution,
            shift_held,
            modal_rendered: modal_rendered_this_frame,
        } = flags;

        let tick_exit_code = Self::advance_timeline(
            http,
            runtime,
            host,
            game,
            &mut manager.engine,
            assets,
            dev,
            &mut frame,
            execution,
        );
        let history_commit_pending = frame.timeline_advances(execution.advances_live_timeline());
        SimulationVisualRefresh {
            last_shadow_color,
            last_visual_ambiance,
            engine: &manager.engine,
            host,
            dev,
            presentation,
            resources,
            window,
        }
        .run();

        SimulationModalState {
            frame,
            rewind_active: execution == FrameExecutionMode::Rewind,
            consumed_buffered: execution == FrameExecutionMode::Buffered,
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
            http,
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
            let scene_screenshots = http.take_pending_scene_screenshots(runtime.frame_number());
            if !scene_screenshots.is_empty() {
                pre_render_engine_setup(host);
                update_mouse_and_cursor(
                    &manager.engine,
                    host,
                    assets.as_ref(),
                    dev,
                    &mut frame.stage_post_external_actions(),
                    &mut presentation.renderer,
                    &mut resources.cursor,
                    &mut presentation.sprites.cursor_renderer,
                    &input.threaded,
                    &presentation.sprites.portrait_cache,
                    shift_held,
                    &mut hud.last_cursor_id,
                );
                let display_snapshot = host.frontend.presentation.engine_display.clone();
                presentation.prepare_zoom(&manager.engine, &host.presentation(), hud, input);
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
                    http,
                    scene_screenshots,
                    &manager.engine.presentation_view(),
                    &display_snapshot,
                    &mut host.presentation(),
                    assets.as_ref(),
                    dev,
                    &mut render_context,
                );
                post_render_engine_cleanup(
                    &mut frame,
                    host.transport.local_seat(),
                    runtime.playback().is_some(),
                );
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
                drain_presented_ui_screenshots(
                    http,
                    runtime.frame_number(),
                    &presentation.renderer,
                );
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
                                .update_and_retain_player_profiles(|manager| {
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
                                })
                                .unwrap_or_else(|error| {
                                    panic!("Options profile update failed: {error}")
                                })
                                .log_persistence_error("Options: failed to save profile manager");
                        }

                        let effects = crate::host::FrontendPreferences::new(
                            result.key_config.clone(),
                            result.custom_key_config.clone(),
                            result.profile_gameplay_config,
                            &result.graphic_config,
                        )
                        .apply(&mut host.frontend);
                        // Preserve live side-effect order: cancel planning, update
                        // window/renderer presentation, release tactical control,
                        // then enqueue authorized simulation-setting commands.
                        if effects.cancel_planned_action {
                            dispatch_local_command(
                                &host.transport,
                                &mut frame.stage_post_commands(),
                                &PlayerCommand::CancelPlannedAction,
                            );
                        }
                        window.set_native_refresh_presentation(effects.native_refresh_presentation);
                        presentation.renderer.configure_native_refresh_presentation(
                            effects.native_refresh_presentation,
                            window.surface_config.width,
                            window.surface_config.height,
                        );
                        if effects.release_tactical_control {
                            dispatch_local_command(
                                &host.transport,
                                &mut frame.stage_post_commands(),
                                &PlayerCommand::ReleaseTacticalControl,
                            );
                        }
                        if result.sound_config.amount_of_speaking
                            != result.original_amount_of_speaking
                        {
                            dispatch_local_command(
                                &host.transport,
                                &mut frame.stage_post_commands(),
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
                                &mut frame.stage_post_commands(),
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
                                &mut frame.stage_post_commands(),
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
                                &mut frame.stage_post_commands(),
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
                                &mut frame.stage_post_commands(),
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
                                &mut frame.stage_post_commands(),
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
                                &mut frame.stage_post_commands(),
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
                                &mut frame.stage_post_commands(),
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
                                &mut frame.stage_post_commands(),
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
                                &mut frame.stage_post_commands(),
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
                                &mut frame.stage_post_commands(),
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
                                &mut frame.stage_post_commands(),
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
                                &mut frame.stage_post_commands(),
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
                                &mut frame.stage_post_commands(),
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
                            input.resize(logical_width, logical_height);
                            hud.resize(logical_width, logical_height);
                            if host.frontend.resources.mission_surfaces.corner_size().x > 0.0 {
                                dispatch_local_command(
                                    &host.transport,
                                    &mut frame.stage_post_commands(),
                                    &PlayerCommand::MinimapResize {
                                        base: engine_coordinates::ScreenPoint::new(
                                            width - 83.0,
                                            38.0,
                                        ),
                                        corner_size: host
                                            .frontend
                                            .resources
                                            .mission_surfaces
                                            .corner_size(),
                                    },
                                );
                            }
                            game.reshow_campaign_map();
                        }
                        if result.key_config_changed {
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
                &manager.engine,
                profiles,
                window,
                audio,
                resources,
                ui,
                presentation,
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
                &manager.engine,
                assets.as_ref(),
                window,
                audio,
                resources,
                ui,
                presentation,
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
            let application_context = host.application_context().clone();
            let requested = frame.begin_post_initialize();
            let post_initialized = runtime.cross_post_initialize(|| {
                crate::sim_timeline::run_post_initialize_stage_with_actions(
                    &mut host.frontend,
                    &mut host.audio,
                    &mut host.effects,
                    &application_context,
                    host.transport.local_seat(),
                    assets,
                    &mut manager.engine,
                    dev,
                    frame.unapplied_post_external_actions(),
                    frame.post_commands(),
                    requested,
                )
            });
            frame.complete_post_initialize(post_initialized);

            if history_commit_pending {
                runtime.commit_simulation_history(
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
            playing_back: runtime.playback().is_some(),
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
            let application_context = host.application_context().clone();
            let requested = frame.begin_post_initialize();
            let post_initialized = runtime.cross_post_initialize(|| {
                crate::sim_timeline::run_post_initialize_stage_with_actions(
                    &mut host.frontend,
                    &mut host.audio,
                    &mut host.effects,
                    &application_context,
                    host.transport.local_seat(),
                    assets,
                    &mut manager.engine,
                    dev,
                    frame.unapplied_post_external_actions(),
                    frame.post_commands(),
                    requested,
                )
            });
            frame.complete_post_initialize(post_initialized);

            if history_commit_pending {
                runtime.commit_simulation_history(
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
        http: &mut crate::http_server::SessionIngress,
        runtime: &mut super::runtime::TimelineRuntime,
        host: &mut Host,
        game: &mut crate::game::Game,
        engine: &mut robin_engine::engine::Engine,
        assets: &robin_engine::engine::LevelAssets,
        dev: &mut robin_engine::engine::DevState,
        frame: &mut MissionFrame,
        execution: FrameExecutionMode,
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
        runtime.begin_recording(frame, execution.records_commands());
        runtime.trace(FrameContractStage::Simulation);

        // ── Engine tick ──
        // The pause menu freezes the simulation by skipping the
        // hourglass while the menu is shown.  Rewind also freezes
        // the tick: the engine state was just replaced with a
        // reconstruction of an earlier frame and must not be
        // advanced this frame.
        let replay_idle = runtime.playback().is_some() && !frame.has_recorded_input();
        let tick_exit_code = runtime.run_simulation(|| {
            if replay_idle {
                frame.admit_simulation();
                return None;
            }
            if execution == FrameExecutionMode::Rewind {
                return None;
            }
            let paused = execution == FrameExecutionMode::Paused;
            let application_context = host.application_context().clone();
            let mission_transitioning = !game
                .operation
                .is(robin_engine::game_operation::GameCode::LevelInProgress);
            frame.restrict_hourglass(game.should_run_hourglass(
                false,
                mission_transitioning,
                paused,
            ));
            let simulation_frame = frame.hourglass_input();
            frame.admit_simulation();
            let result = game.run_engine_tick(
                &mut host.frontend,
                &mut host.audio,
                &mut host.effects,
                &application_context,
                host.transport.local_seat(),
                assets,
                engine,
                dev,
                simulation_frame,
                false,
                paused,
            );

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
        let application_context = host.application_context().clone();
        super::runtime::drain_post_tick_rpc(
            http,
            runtime,
            &mut host.frontend,
            &mut host.audio,
            &mut host.effects,
            &application_context,
            &host.transport,
            engine,
            assets,
            dev,
            frame,
        );

        // ── Rollback check + rewind buffer commit ──
        // Both are post-tick bookkeeping.  Skipped on paused frames
        // (no tick ran) and rewind frames (tick was suppressed).  The
        // rewind buffer also skips commits while consuming its own
        // log — the slot is already populated and would duplicate.
        if frame.timeline_advances(execution.advances_live_timeline()) {
            let next_frame = runtime.advance_frame().number();
            if let Some(net) = host.transport.net()
                && host.transport.local_seat() == engine_player_command::PlayerId::HOST
            {
                net.set_initial_snapshot(next_frame, engine);
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
        http: &mut crate::http_server::SessionIngress,
        runtime: &mut super::runtime::TimelineRuntime,
        save_manager: &crate::savegame::SaveGameManager,
        mutation: super::runtime::MissionMutation<'_>,
        manual_pause: &mut bool,
        ui: &mut super::interactive::MissionUi,
        window: &crate::window::GameWindow,
        presentation: &mut super::interactive::MissionPresentation,
        input: &mut super::interactive::MissionInput,
        terminal_exit_pending: bool,
        keyboard_step: KeyboardStep,
    ) {
        // ── Pending `/step-forward` / `/step-back` requests ──
        let super::runtime::MissionMutation {
            host,
            game,
            manager,
            assets,
            dev,
        } = mutation;
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
            http.take_pending_steps(),
            super::runtime::MissionMutation {
                manager,
                host,
                assets,
                dev,
                game,
            },
            runtime,
            manual_pause,
            &mut ui.active_modal,
            ui.terminal_debriefing.as_mut(),
            Some(save_manager),
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
        http.set_replay_status(
            runtime
                .playback()
                .map(|p| crate::http_server::ReplayStatus {
                    frame: p.current_frame(),
                    total: p.total_frames(),
                    paused: *manual_pause,
                }),
        );

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
        let keyboard_step = keyboard_step.admitted(
            host.transport.net().is_some(),
            modal_state_pending(host),
            mission_ui_modal_pending,
        );
        if keyboard_step == KeyboardStep::Forward {
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
        } else if keyboard_step == KeyboardStep::Back {
            if let Some(target) = runtime.current_frame().previous()
                && let Some(oldest) = runtime.retained_history().oldest_reachable_frame()
                && target.number() >= oldest
            {
                runtime.begin_rewind_session();
                let restored = runtime.restore_retained_frame(manager, assets, target);
                runtime.end_rewind_session();
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
    fn execution_modes_preserve_admission_and_recording_policy() {
        use super::super::runtime::{MissionFrame, TimelineFrame, TimelineTransition};
        use super::FrameExecutionMode::{self, *};
        for (rewind, paused, buffered, expected, records, advances) in [
            (false, false, false, Live, true, true),
            (false, true, false, Paused, true, false),
            (false, false, true, Buffered, false, true),
            (true, false, false, Rewind, false, false),
            (true, true, false, Rewind, false, false),
        ] {
            let mode = FrameExecutionMode::admitted(rewind, paused, buffered);
            assert_eq!(mode, expected);
            assert_eq!(mode.records_commands(), records);
            assert_eq!(mode.advances_live_timeline(), advances);
            let mut frame = MissionFrame::new(0);
            assert_eq!(
                frame.timeline_advances(mode.advances_live_timeline()),
                advances
            );
            // A recorded transition remains authoritative independently of
            // the host's clock mode (including command-only replay records).
            for after in [TimelineFrame::ZERO, TimelineFrame::ZERO.next()] {
                frame.replay_timeline_transition = Some(TimelineTransition {
                    before: TimelineFrame::ZERO,
                    after,
                });
                assert_eq!(
                    frame.timeline_advances(mode.advances_live_timeline()),
                    after != TimelineFrame::ZERO,
                );
            }
        }
    }

    #[test]
    #[should_panic(expected = "buffered input requires a running forward frame")]
    fn buffered_execution_rejects_rewind() {
        super::FrameExecutionMode::admitted(true, false, true);
    }

    #[test]
    #[should_panic(expected = "buffered input requires a running forward frame")]
    fn buffered_execution_rejects_pause() {
        super::FrameExecutionMode::admitted(false, true, true);
    }

    #[test]
    #[should_panic(expected = "buffered input requires a running forward frame")]
    fn buffered_execution_rejects_paused_rewind() {
        super::FrameExecutionMode::admitted(true, true, true);
    }

    #[test]
    fn paused_commands_and_rewind_post_actions_keep_their_frame_boundary() {
        use super::{FrameExecutionMode, InteractiveFrameSimulation};
        use crate::game_session::runtime::{FrameContract, MissionFrame, TimelineRuntime};
        use robin_engine::engine::{DevState, Engine, ExternalAction, LevelAssets};
        use robin_engine::player_command::{PlayerCommand, PlayerId, PlayerInput};

        for mode in [FrameExecutionMode::Paused, FrameExecutionMode::Rewind] {
            let mut assets = LevelAssets::new();
            let mut engine = Engine::new_for_test_with_level_size(
                1024.0,
                768.0,
                Default::default(),
                &mut assets,
                4096.0,
                4096.0,
            )
            .unwrap();
            let assets = std::sync::Arc::new(assets);
            let mut host = crate::host::Host::default();
            let mut game = crate::game::Game::default();
            let mut dev = DevState::default();
            let mut timeline = TimelineRuntime::new(
                super::super::replay_init::ReplayAndRollback {
                    recording_control:
                        std::sync::Arc::<crate::replay_service::ReplayService>::default()
                            .recording(),
                    recorder: None,
                    player: None,
                    rollback_checker: None,
                    rewind_buffer: crate::rewind::RewindBuffer::new(),
                    start_paused: false,
                },
                FrameContract::Graphical,
                false,
                true,
            );
            let mut frame = MissionFrame::new(0);
            timeline.open_frame(&mut frame, &engine);
            frame.stage_commands().push(PlayerInput::new(
                PlayerId::HOST,
                PlayerCommand::SetLockAlt(true),
            ));
            let campaign = robin_engine::campaign::Campaign {
                ares: 3,
                ..Default::default()
            };
            frame
                .stage_post_external_actions()
                .push(ExternalAction::ReplaceCampaign { campaign });
            let before_tick = engine.simulation_tick();
            let mut http = crate::http_server::SessionIngress::detached_for_test();
            InteractiveFrameSimulation::advance_timeline(
                &mut http,
                &mut timeline,
                &mut host,
                &mut game,
                &mut engine,
                &assets,
                &mut dev,
                &mut frame,
                mode,
            );
            assert_eq!(engine.simulation_tick(), before_tick);
            assert_eq!(engine.is_lock_alt(), mode == FrameExecutionMode::Paused);
            assert_eq!(
                engine.campaign().ares,
                3,
                "post-tick RPC stage still executes during rewind"
            );
            assert!(frame.unapplied_post_external_actions().is_empty());
            assert_eq!(timeline.frame_number(), 0);
        }
    }

    #[test]
    fn keyboard_step_has_one_direction_and_cannot_bypass_modal_or_network_gates() {
        use super::KeyboardStep::{self, *};
        assert_eq!(KeyboardStep::from_pressed(false, false), None);
        assert_eq!(KeyboardStep::from_pressed(true, true), Forward);
        assert_eq!(KeyboardStep::from_pressed(false, true), Back);
        for step in [None, Forward, Back] {
            assert_eq!(step.admitted(false, false, false), step);
            for gate in [
                (true, false, false),
                (false, true, false),
                (false, false, true),
            ] {
                assert_eq!(step.admitted(gate.0, gate.1, gate.2), None);
            }
        }
    }

    #[test]
    fn shared_modal_driver_preserves_leave_mission_confirmation() {
        use super::{ActiveModalOutcome, dispatch_active_modal_outcome};
        use robin_engine::player_command::{FrameCommands, PlayerCommand};

        let mut host = crate::host::Host::scratch(640.0, 480.0);
        let mut commands = FrameCommands::new();
        // Waiting or answering No must not end the mission.
        dispatch_active_modal_outcome(ActiveModalOutcome::None, &mut host, &mut commands);
        assert!(commands.commands.is_empty());

        // A prompt carried over from the preceding frame is ticked by the
        // shared driver. Its Yes outcome must reach the recorded transaction.
        dispatch_active_modal_outcome(
            ActiveModalOutcome::QuitMissionRequested,
            &mut host,
            &mut commands,
        );
        assert_eq!(commands.commands.len(), 1);
        assert!(matches!(
            commands.commands[0].command,
            PlayerCommand::QuitMissionRequested
        ));
        let mut assets = robin_engine::engine::LevelAssets::new();
        let mut engine = robin_engine::engine::Engine::new_for_test(
            640.0,
            480.0,
            robin_engine::campaign::Campaign::default(),
            &mut assets,
        )
        .expect("engine");
        engine.test_set_mission_flags(false, false, true);
        engine
            .advance_frame(
                &assets,
                robin_engine::engine::SimulationFrameInput {
                    post_commands: commands.commands.into_iter().map(Into::into).collect(),
                    run_hourglass: false,
                    ..Default::default()
                },
            )
            .expect("admit the confirmation while the modal pauses simulation");
        let next = engine
            .advance_frame(&assets, Default::default())
            .expect("advance after closing the victory prompt");
        assert_eq!(
            next.game_code(),
            robin_engine::game_operation::GameCode::LevelSucceeded
        );
    }

    #[test]
    fn shared_modal_confirmation_waits_for_multiplayer_command_echo() {
        use super::{ActiveModalOutcome, dispatch_active_modal_outcome};
        use crate::multiplayer::{NetChannels, NetOutbound};
        use robin_engine::player_command::{FrameCommands, PlayerCommand};

        let mut host = crate::host::Host::scratch(640.0, 480.0);
        let (channels, _incoming, outgoing, _, _) = NetChannels::new();
        host.transport =
            crate::host::HostTransport::test_session(channels, host.transport.local_seat());
        let mut commands = FrameCommands::new();
        dispatch_active_modal_outcome(
            ActiveModalOutcome::QuitMissionRequested,
            &mut host,
            &mut commands,
        );
        assert!(commands.commands.is_empty());
        assert!(matches!(
            outgoing.try_recv().expect("confirmation sent to server"),
            NetOutbound::Input {
                command: PlayerCommand::QuitMissionRequested,
                ..
            }
        ));
    }

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
                UiTaskKind::CampaignManager,
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
                UiTaskKind::CampaignManager,
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
