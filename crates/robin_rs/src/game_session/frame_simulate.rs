//! Deterministic interactive-frame simulation and modal orchestration.
//!
//! Input preparation remains in `flow`. This phase owns the exact post-input
//! order from command recording through simulation history, stepping, scripted
//! modals, and the handoff to presentation.

use super::flow::{FrameControl, MissionServices};
use super::interactive::{MissionPresentation, MissionResources};
use super::runtime::FrameContractStage;
use super::*;
use crate::game::Game;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct FramePresentationHandoff {
    pub(super) frame: MissionFrame,
    pub(super) rewind_active: bool,
    pub(super) consumed_buffered: bool,
    pub(super) shift_held: bool,
    pub(super) modal_rendered: bool,
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
                host.pending_dialogues
                    .extend((0..descriptors.dialogues.len()).map(|index| index as i32));
            } else {
                tracing::warn!("cheat all_dialogues: level descriptors unavailable");
            }
        }
        if dev.debug.all_popup_texts {
            dev.debug.all_popup_texts = false;
            if let Some(descriptors) = &resources.level_descriptors {
                host.pending_popup_texts.extend(
                    (0..descriptors.popup_text.picture_ids.len()).map(|index| index as i32),
                );
            } else {
                tracing::warn!("cheat all_popup_texts: level descriptors unavailable");
            }
        }
        if dev.debug.all_debriefings {
            dev.debug.all_debriefings = false;
            if let Some(descriptors) = &resources.level_descriptors {
                host.pending_debriefings.extend(
                    (0..descriptors.debriefing.lose_count as usize)
                        .map(|index| engine_player_command::DebriefingTextId::Lose { index }),
                );
                host.pending_debriefings.extend(
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum TerminalDebriefingAction {
    Continue,
    LoadRestart,
    Load { slot: usize, mission_id: u32 },
    EmergencyExit,
}

fn terminal_debriefing_action(
    outcome: &SettledDebriefingOutcome,
    mission_id: u32,
) -> TerminalDebriefingAction {
    match outcome {
        SettledDebriefingOutcome::Ok => TerminalDebriefingAction::Continue,
        SettledDebriefingOutcome::Restart => TerminalDebriefingAction::LoadRestart,
        SettledDebriefingOutcome::Load { slot } => TerminalDebriefingAction::Load {
            slot: *slot,
            mission_id,
        },
        SettledDebriefingOutcome::EmergencyEnd => TerminalDebriefingAction::EmergencyExit,
    }
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
    runtime: &mut super::runtime::TimelineRuntime,
    frame: &mut MissionFrame,
    mode: ScriptedModalMode,
    mut rendered: bool,
) -> bool {
    let auto_dismiss = mode == ScriptedModalMode::AutoDismiss;
    if auto_dismiss {
        drain_pending_dialogues(
            host,
            window,
            &mut presentation.renderer,
            &mut resources.cursor,
            &mut presentation.sprites.cursor_renderer,
            &mut audio.backend,
            &mut resources.text,
            game,
            &resources.level_descriptors,
            &mut resources.menu,
            &mut runtime.replay_recorder,
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
                window,
                &mut presentation.renderer,
                &mut resources.cursor,
                &mut presentation.sprites.cursor_renderer,
                &mut audio.backend,
                &audio.sample_loader,
                &mut resources.menu,
                &mut runtime.replay_recorder,
            );
            debug_assert_eq!(outcome, ActiveModalOutcome::None);
            rendered = true;
        }
    }

    if !rendered && auto_dismiss {
        drain_pending_popup_scroll(
            host,
            window,
            &mut presentation.renderer,
            &mut resources.cursor,
            &mut presentation.sprites.cursor_renderer,
            &mut audio.backend,
            &audio.sample_loader,
            &mut resources.text,
            &resources.level_descriptors,
            &mut resources.menu,
            &mut runtime.replay_recorder,
            &mut frame.replay_modal_dismissals,
            manager.engine.frame_counter(),
        )
        .await;
        drain_pending_sherwood_stat(
            host,
            window,
            &mut presentation.renderer,
            &mut resources.cursor,
            &mut presentation.sprites.cursor_renderer,
            &manager.engine,
            profiles,
            &mut audio.backend,
            &audio.sample_loader,
            &mut resources.menu,
            &mut runtime.replay_recorder,
            &mut frame.replay_modal_dismissals,
        )
        .await;
    } else if !rendered {
        if ui.active_modal.is_none()
            && let Some(batch) = start_active_popup_scroll_batch(
                host,
                &mut presentation.renderer,
                &mut resources.text,
                &resources.level_descriptors,
                &mut resources.menu,
                &mut frame.replay_modal_dismissals,
                manager.engine.frame_counter(),
            )
        {
            ui.active_modal = Some(ActiveModal::PopupScroll(Box::new(batch)));
        }
        if ui.active_modal.is_none()
            && let Some(batch) = start_active_sherwood_report(
                host,
                &manager.engine,
                profiles,
                &mut resources.menu,
                &mut frame.replay_modal_dismissals,
            )
        {
            ui.active_modal = Some(ActiveModal::PopupScroll(Box::new(batch)));
        }
        if ui.active_modal.is_some() {
            let outcome = tick_active_modal(
                &mut ui.active_modal,
                host,
                window,
                &mut presentation.renderer,
                &mut resources.cursor,
                &mut presentation.sprites.cursor_renderer,
                &mut audio.backend,
                &audio.sample_loader,
                &mut resources.menu,
                &mut runtime.replay_recorder,
            );
            debug_assert_eq!(outcome, ActiveModalOutcome::None);
            rendered = true;
        }
    }

    if !rendered && auto_dismiss {
        drain_pending_debriefings(
            host,
            window,
            &mut presentation.renderer,
            &mut resources.cursor,
            &mut presentation.sprites.cursor_renderer,
            &mut resources.text,
            &resources.level_descriptors,
            &resources.menu,
            &mut runtime.replay_recorder,
            &mut frame.replay_modal_dismissals,
        )
        .await;
    } else if !rendered {
        if ui.active_modal.is_none()
            && let Some(batch) = start_active_debriefing_batch(
                host,
                &mut resources.text,
                &resources.level_descriptors,
                &resources.menu,
                &mut frame.replay_modal_dismissals,
            )
        {
            ui.active_modal = Some(ActiveModal::Debriefing(Box::new(batch)));
        }
        if ui.active_modal.is_some() {
            let outcome = tick_active_modal(
                &mut ui.active_modal,
                host,
                window,
                &mut presentation.renderer,
                &mut resources.cursor,
                &mut presentation.sprites.cursor_renderer,
                &mut audio.backend,
                &audio.sample_loader,
                &mut resources.menu,
                &mut runtime.replay_recorder,
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
    runtime: &mut super::runtime::TimelineRuntime,
    frame: &mut MissionFrame,
    mode: ScriptedModalMode,
    rendered: bool,
) -> bool {
    if rendered || (!host.pending_mission_state_popup && ui.active_modal.is_none()) {
        return rendered;
    }
    if host.pending_mission_state_popup {
        host.pending_mission_state_popup = false;
        if mode == ScriptedModalMode::AutoDismiss {
            let cmd = PlayerCommand::QuitMissionRequested;
            dispatch_local_command(host, &mut manager.engine, &mut frame.commands, assets, &cmd);
            frame.commands.push(cmd);
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
    let outcome = tick_active_modal(
        &mut ui.active_modal,
        host,
        window,
        &mut presentation.renderer,
        &mut resources.cursor,
        &mut presentation.sprites.cursor_renderer,
        &mut audio.backend,
        &audio.sample_loader,
        &mut resources.menu,
        &mut runtime.replay_recorder,
    );
    if outcome == ActiveModalOutcome::QuitMissionRequested {
        let cmd = PlayerCommand::QuitMissionRequested;
        dispatch_local_command(host, &mut manager.engine, &mut frame.commands, assets, &cmd);
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
    if !std::mem::take(&mut host.pending_reset_input) {
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

/// Resolve an engine tick exit through the original mission-state/debriefing
/// flow. Returns true only for an emergency window close.
#[allow(clippy::too_many_arguments)]
async fn drive_tick_exit_modals(
    tick_exit_code: Option<GameCode>,
    host: &mut Host,
    game: &mut Game,
    manager: &mut robin_engine::engine_manager::EngineManager,
    assets: &robin_engine::engine::LevelAssets,
    window: &mut GameWindow,
    callbacks: &mut RustCallbacks,
    input: &mut super::interactive::MissionInput,
    audio: &mut super::interactive::MissionAudio,
    resources: &mut super::interactive::MissionResources,
    ui: &mut super::interactive::MissionUi,
    presentation: &mut super::interactive::MissionPresentation,
    runtime: &mut super::runtime::TimelineRuntime,
    frame: &mut MissionFrame,
) -> bool {
    if let Some(exit_code) = tick_exit_code {
        tracing::info!("Engine tick returned: {:?}", exit_code);

        // Apply quit-mission updates (stat sync, coma reset,
        // score bonuses, warcrime recruitment, blazon
        // consumption) before showing the debriefing so it
        // displays correct stats. The command mutates the campaign in
        // place inside the engine's required mission domain.
        dispatch_local_command(
            host,
            &mut manager.engine,
            &mut frame.commands,
            assets,
            &PlayerCommand::ApplyQuitMissionUpdates {
                exit_code,
                difficulty: game.global_options.sim_config().difficulty,
            },
        );

        // Show the mission state popup + debriefing synchronously
        // now, while the presentation.renderer and menu resources are still
        // alive.  `show_debriefing` blocks the loop until the
        // player dismisses it.
        if let (Some((popup_title, _popup_body)), Some(menu_resources)) = (
            crate::ingame_menu::mission_state_text(exit_code),
            &resources.menu,
        ) {
            let won = exit_code == GameCode::LevelSucceeded;
            let mission_state_kind = engine_player_command::ModalKind::MissionState {
                kind: engine_player_command::MissionStateModalKind::EndState { won },
            };
            let mission_state_result = match pop_matching_dismissal(
                &mut frame.replay_modal_dismissals,
                &mission_state_kind,
            ) {
                Some(
                    result @ (engine_player_command::DialogResult::Completed
                    | engine_player_command::DialogResult::Aborted),
                ) => result,
                Some(result) => {
                    tracing::warn!(
                        ?result,
                        "final mission-state replay result is only yes/no; treating as aborted"
                    );
                    engine_player_command::DialogResult::Aborted
                }
                None => {
                    let cursor = Some(default_modal_cursor(
                        &mut presentation.sprites.cursor_renderer,
                        &mut resources.cursor,
                        &mut presentation.renderer,
                    ));
                    let confirmed = crate::ingame_menu::show_mission_state_popup(
                        &mut *window,
                        &mut presentation.renderer,
                        menu_resources,
                        cursor,
                        popup_title,
                        won,
                        None,
                    )
                    .await;
                    if confirmed {
                        engine_player_command::DialogResult::Completed
                    } else {
                        engine_player_command::DialogResult::Aborted
                    }
                }
            };
            if let Some(recorder) = runtime.replay_recorder.as_mut() {
                recorder.push(engine_player_command::PlayerCommand::ModalDismiss {
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
            let debriefing_kind = engine_player_command::ModalKind::FinalDebriefing {
                text_id: engine_player_command::DebriefingTextId::from_outcome(
                    won,
                    debriefing_index,
                ),
            };
            let debriefing_body = if let Some(descriptors) = resources.level_descriptors.as_ref() {
                let table_id = if won {
                    descriptors.debriefing.win_text_table_id
                } else {
                    descriptors.debriefing.lose_text_table_id
                };
                match resources.text.get_string(table_id, debriefing_index) {
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
            let mission_length =
                <RustCallbacks as crate::game::GameCallbacks>::get_current_playing_time(
                    callbacks,
                    manager.engine.campaign(),
                );
            // When restart is allowed, the debriefing accepts a
            // QuickLoad keypress to short-circuit into a load.
            // Pull the configured `QuickLoad1` key out of
            // the input translator so the modal can fire on that
            // key.
            let quick_load_key = input.translator.get_binding(GameKey::QuickLoad1);
            // Restart only fires when a restart snapshot exists.
            // When missing, the body window closes and the stat
            // panel still shows.  Probe the save manager up
            // front so the modal can short-circuit a no-snapshot
            // Restart click to "skip body, show stat".
            let restart_snapshot_exists =
                ui.restart_allowed && callbacks.save_manager.has_restart_save();
            let campaign = manager.engine.campaign();
            let mission_id = current_mission_id(campaign, &assets.profile_manager);

            // Re-entry loop for the Load button: clicking Load
            // chains into the save-load picker; if the picker is
            // cancelled, the current debriefing page stays
            // visible and the player can continue interacting
            // with it.  We model that by re-entering
            // `show_debriefing` with the page text the player
            // was viewing when they clicked Load.
            let post_load_outcome = if let Some(result) =
                pop_matching_dismissal(&mut frame.replay_modal_dismissals, &debriefing_kind)
            {
                final_debriefing_outcome_from_replay(result)
            } else {
                let mut current_body = debriefing_body.clone();
                let mut start_at_stat = false;
                loop {
                    let cursor = Some(default_modal_cursor(
                        &mut presentation.sprites.cursor_renderer,
                        &mut resources.cursor,
                        &mut presentation.renderer,
                    ));
                    let outcome = crate::ingame_menu::show_debriefing(
                        &mut *window,
                        &mut presentation.renderer,
                        menu_resources,
                        cursor,
                        &current_body,
                        Some(manager.engine.mission_stat()),
                        mission_length,
                        won,
                        ui.restart_allowed,
                        quick_load_key,
                        restart_snapshot_exists,
                        start_at_stat,
                    )
                    .await;
                    match outcome {
                        DebriefingOutcome::LoadAttempt {
                            body_remaining,
                            was_on_stat,
                        } => {
                            // Run the save-load picker.  If a slot is
                            // selected we'll re-enter the loop with a
                            // synthetic outcome below; otherwise we
                            // re-show the same page (body or stat).
                            let cursor = Some(default_modal_cursor(
                                &mut presentation.sprites.cursor_renderer,
                                &mut resources.cursor,
                                &mut presentation.renderer,
                            ));
                            let picker_outcome = crate::ingame_menu::show_save_load(
                                &mut *window,
                                &mut presentation.renderer,
                                menu_resources,
                                cursor,
                                &mut callbacks.save_manager,
                                mission_id,
                                Some(&assets.profile_manager),
                                SaveLoadMode::Load,
                                Some(&mut host.sound),
                                audio
                                    .backend
                                    .as_mut()
                                    .map(|b| b as &mut dyn crate::sound::AudioBackend),
                                Some(&audio.sample_loader),
                            )
                            .await;
                            match picker_outcome {
                                SaveLoadOutcome::Slot(slot) => {
                                    // Synthesise a Load-resolved outcome and
                                    // exit the re-entry loop.  Stored in
                                    // `post_load_outcome` so the match
                                    // below processes it uniformly.
                                    break SettledDebriefingOutcome::Load { slot };
                                }
                                SaveLoadOutcome::Cancel => {
                                    // Picker cancelled — re-enter the
                                    // debriefing on the same page.
                                    current_body = body_remaining;
                                    start_at_stat = was_on_stat;
                                    continue;
                                }
                            }
                        }
                        DebriefingOutcome::Ok { .. } => {
                            break SettledDebriefingOutcome::Ok;
                        }
                        DebriefingOutcome::Restart => {
                            break SettledDebriefingOutcome::Restart;
                        }
                        DebriefingOutcome::EmergencyEnd => {
                            break SettledDebriefingOutcome::EmergencyEnd;
                        }
                    }
                }
            };
            if let Some(recorder) = runtime.replay_recorder.as_mut() {
                recorder.push(engine_player_command::PlayerCommand::ModalDismiss {
                    kind: debriefing_kind,
                    result: final_debriefing_result(&post_load_outcome),
                });
            }

            // Wire the Load/Restart outcomes back into the game
            // state machine.  Both funnel through the engine's
            // save-game slot machinery rather than re-running
            // the mission cold.
            match terminal_debriefing_action(&post_load_outcome, mission_id) {
                TerminalDebriefingAction::Continue => {
                    // Normal dismissal — let the exit_code flow
                    // through the Game state machine on the next
                    // frame's `process_operation`.
                }
                TerminalDebriefingAction::LoadRestart => {
                    // We've already verified the restart snapshot
                    // exists via `restart_snapshot_exists`; queue
                    // `SaveLoadRequest::LoadRestart` and reset
                    // `game.operation` so the next frame's
                    // `perform_pending_save_load` applies it in
                    // place.
                    callbacks.pending = Some(SaveLoadRequest::LoadRestart);
                    game.operation.set(GameCode::LevelInProgress);
                }
                TerminalDebriefingAction::Load { slot, mission_id } => {
                    // The Load button chains into the save-load
                    // picker (run inline above) and queues a
                    // level load.
                    callbacks.pending = Some(SaveLoadRequest::Load {
                        slot: Some(slot),
                        mission_id,
                    });
                    game.operation.set(GameCode::LevelInProgress);
                }
                TerminalDebriefingAction::EmergencyExit => {
                    // External force-close (window close / Alt-
                    // F4) propagates as `GameCode::Quit` so
                    // `handle_quit` writes the continue-save and
                    // the outer session returns to the main
                    // menu.
                    return true;
                }
            }
        }
    }

    false
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
        let MissionWorld {
            host,
            game,
            manager,
            assets,
            dev,
        } = world;
        let MissionControl {
            manual_pause,
            last_shadow_color,
            ..
        } = control;
        let resources = &mut frontend.resources;
        let ui = &mut frontend.ui;
        let presentation = &mut frontend.presentation;
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
        let MissionWorld {
            host,
            game,
            manager,
            assets,
            ..
        } = world;
        let input = &mut frontend.input;
        let audio = &mut frontend.audio;
        let resources = &mut frontend.resources;
        let ui = &mut frontend.ui;
        let presentation = &mut frontend.presentation;
        let SimulationModalState {
            mut frame,
            rewind_active,
            consumed_buffered,
            shift_held,
            mut modal_rendered_this_frame,
            auto_dismiss_modals,
            tick_exit_code,
        } = state;

        let modal_mode = if auto_dismiss_modals {
            ScriptedModalMode::AutoDismiss
        } else {
            ScriptedModalMode::Interactive
        };
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
            modal_mode,
            modal_rendered_this_frame,
        )
        .await;

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

        if drive_tick_exit_modals(
            tick_exit_code,
            host,
            game,
            manager,
            assets.as_ref(),
            window,
            callbacks,
            input,
            audio,
            resources,
            ui,
            presentation,
            runtime,
            &mut frame,
        )
        .await
        {
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
        runtime.record_commands(frame, !rewind_active && !consumed_buffered);
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
            let result = game.run_engine_tick(
                host,
                &mut display,
                assets.as_ref(),
                &mut manager.engine,
                dev,
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
        let net = host.net.take();
        crate::http_server::drain_global(manager, host, &assets, net.as_ref());
        host.net = net;

        // ── Rollback check + rewind buffer commit ──
        // Both are post-tick bookkeeping.  Skipped on paused frames
        // (no tick ran) and rewind frames (tick was suppressed).  The
        // rewind buffer also skips commits while consuming its own
        // log — the slot is already populated and would duplicate.
        if !paused && !rewind_active {
            runtime.commit_simulation_history(
                host,
                manager,
                &frame,
                FrameCommitPolicy {
                    store_rewind_commands: !consumed_buffered,
                },
            );
            manager.sim_frame += 1;
            if let Some(net) = host.net.as_ref()
                && host.local_seat == engine_player_command::PlayerId::HOST
            {
                net.set_initial_snapshot(manager.sim_frame, &manager.engine);
            }
        }

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
            &mut runtime.rewind_buffer,
            &mut runtime.rollback_checker,
            &mut runtime.replay_player,
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
        if step_forward_pressed && !modal_state_pending(&host) {
            let step_frame = manager.sim_frame;
            let buffered_cmds = if step_frame < runtime.rewind_buffer.next_record_frame() {
                runtime
                    .rewind_buffer
                    .commands_for(step_frame)
                    .map(<[PlayerInput]>::to_vec)
            } else {
                Some(Vec::new())
            };
            if let Some(buffered_cmds) = buffered_cmds {
                let reusing_recorded_frame = step_frame < runtime.rewind_buffer.next_record_frame();
                runtime
                    .rewind_buffer
                    .begin_frame(step_frame, &manager.engine, &assets);

                let mut step_frame_cmds: Vec<PlayerInput> = Vec::new();
                if let Some(ref mut player) = runtime.replay_player
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
                } else if reusing_recorded_frame {
                    step_frame_cmds = buffered_cmds;
                }
                manager.engine.apply_commands(
                    &mut host.engine_display,
                    &mut host.input,
                    &assets,
                    &step_frame_cmds,
                );

                let mut display = std::mem::take(&mut host.engine_display);
                game.run_engine_tick(
                    host,
                    &mut display,
                    assets.as_ref(),
                    &mut manager.engine,
                    dev,
                    false,
                    false,
                );
                crate::sim_timeline::run_post_initialize_stage(
                    host,
                    &mut display,
                    &assets,
                    &mut manager.engine,
                    dev,
                );
                host.engine_display = display;
                if !reusing_recorded_frame {
                    runtime.rewind_buffer.end_frame(step_frame_cmds);
                }
                manager.sim_frame += 1;
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
        } else if step_back_pressed && !modal_state_pending(&host) {
            if let Some(target) = manager.sim_frame.checked_sub(1)
                && let Some(oldest) = runtime.rewind_buffer.oldest_reachable_frame()
                && target >= oldest
            {
                runtime.rewind_buffer.begin_session();
                let rewound = runtime.rewind_buffer.rewind_to(&assets, target);
                runtime.rewind_buffer.end_session();
                if let Some(new_engine) = rewound {
                    manager.engine = new_engine;
                    manager.sim_frame = target;
                    // Keep the replay cursor in sync with the rewound
                    // sim frame so resuming playback re-applies the
                    // right commands.
                    if let Some(ref mut player) = runtime.replay_player {
                        player.seek(target);
                    }
                    // The rollback checker's ring now references a
                    // timeline the live engine is no longer on; clear
                    // it so the next normal frame starts a fresh
                    // window.
                    if let Some(ref mut checker) = runtime.rollback_checker {
                        checker.reset();
                    }
                } else {
                    tracing::warn!("step-back: rewind_to({target}) failed");
                }
            } else {
                tracing::debug!("step-back: already at oldest retained frame");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ScriptedModalMode, TerminalDebriefingAction, terminal_debriefing_action};
    use crate::game_session::debriefing::SettledDebriefingOutcome;

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

    #[test]
    fn terminal_debriefing_maps_to_explicit_control_actions() {
        let mission_id = 42;
        let cases = [
            (
                SettledDebriefingOutcome::Ok,
                TerminalDebriefingAction::Continue,
            ),
            (
                SettledDebriefingOutcome::Restart,
                TerminalDebriefingAction::LoadRestart,
            ),
            (
                SettledDebriefingOutcome::Load { slot: 7 },
                TerminalDebriefingAction::Load {
                    slot: 7,
                    mission_id,
                },
            ),
            (
                SettledDebriefingOutcome::EmergencyEnd,
                TerminalDebriefingAction::EmergencyExit,
            ),
        ];

        for (outcome, expected) in cases {
            assert_eq!(terminal_debriefing_action(&outcome, mission_id), expected);
        }
    }
}
