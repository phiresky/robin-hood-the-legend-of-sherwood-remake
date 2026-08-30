//! Frame-driven mission-end popup, debriefing renderer, and load picker.
//!
//! The complete sequence is retained across outer graphical frames so network,
//! replay, and HTTP services continue draining while mission-end UI is open.

use super::interactive::{
    MissionAudio, MissionInput, MissionPresentation, MissionResources, MissionUi,
};
use super::*;
use crate::game::Game;
use crate::ingame_menu::widget_bridge::default_modal_cursor;

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

fn mission_completion_clock() -> (Option<i64>, Option<u64>) {
    let Ok(duration) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) else {
        return (None, None);
    };
    (
        i64::try_from(duration.as_secs()).ok(),
        u64::try_from(duration.as_nanos()).ok(),
    )
}

fn debriefing_text_table_id(won: bool, win_table_id: i32, lose_table_id: i32) -> i32 {
    if won { win_table_id } else { lose_table_id }
}

struct TerminalDebriefingPage {
    kind: engine_player_command::ModalKind,
    body: String,
    mission_length: u32,
    quick_load_key: Option<winit::keyboard::KeyCode>,
    restart_allowed: bool,
    restart_snapshot_exists: bool,
    mission_id: u32,
    won: bool,
    mission_stat: robin_engine::mission_stat::MissionStat,
}

enum TerminalDebriefingPhase {
    MissionState(crate::ingame_menu::MissionStatePopupState),
    AwaitingMissionAuthority,
    Debriefing(crate::ingame_menu::DebriefingModalState),
    LoadPicker {
        picker: crate::ingame_menu::LoadPickerModalState,
        body: String,
        was_on_stat: bool,
    },
    AwaitingFinalAuthority,
}

pub(super) struct TerminalDebriefingState {
    exit_code: GameCode,
    popup_kind: engine_player_command::ModalKind,
    page: TerminalDebriefingPage,
    phase: TerminalDebriefingPhase,
    http_result: Option<(
        engine_player_command::ModalKind,
        engine_player_command::DialogResult,
    )>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TerminalDebriefingProgress {
    Inactive,
    Pending,
    Complete,
    EmergencyExit,
}

/// Explicit owners borrowed by the blocking terminal graphical flow.
pub(super) struct TerminalDebriefingContext<'a> {
    pub(super) tick_exit_code: Option<GameCode>,
    pub(super) host: &'a mut Host,
    pub(super) game: &'a mut Game,
    pub(super) manager: &'a mut robin_engine::engine_manager::EngineManager,
    pub(super) assets: &'a robin_engine::engine::LevelAssets,
    pub(super) window: &'a mut GameWindow,
    pub(super) callbacks: &'a mut RustCallbacks,
    pub(super) input: &'a mut MissionInput,
    pub(super) audio: &'a mut MissionAudio,
    pub(super) resources: &'a mut MissionResources,
    pub(super) ui: &'a mut MissionUi,
    pub(super) presentation: &'a mut MissionPresentation,
    pub(super) frame: &'a mut MissionFrame,
}

fn terminal_debriefing_page(
    context: &mut TerminalDebriefingContext<'_>,
    won: bool,
    terminal_mission_id: u32,
) -> TerminalDebriefingPage {
    let index = context.manager.engine.mission().victory_defeat_id as usize;
    let kind = engine_player_command::ModalKind::FinalDebriefing {
        text_id: engine_player_command::DebriefingTextId::from_outcome(won, index),
    };
    let mut body = if let Some(descriptors) = context.resources.level_descriptors.as_ref() {
        let table_id = debriefing_text_table_id(
            won,
            descriptors.debriefing.win_text_table_id,
            descriptors.debriefing.lose_text_table_id,
        );
        match context.resources.text.get_string(table_id, index) {
            Ok(text) => text.to_string(),
            Err(error) => {
                tracing::warn!(
                    "Debriefing text lookup failed (table={table_id}, index={index}): {error}"
                );
                "Invalid debriefing ID...".to_string()
            }
        }
    } else {
        tracing::warn!("Debriefing text lookup: level descriptors unavailable");
        "No dynamic resources for this level...".to_string()
    };
    if won
        && context.host.gameplay_config.show_achievement_debrief
        && let Some(results) = context.manager.engine.mission_achievement_results()
    {
        body.push_str("\n\n");
        body.push_str(&crate::achievement_hud::format_attempt_summary(*results));
    }
    let mission_length = <RustCallbacks as crate::game::GameCallbacks>::get_current_playing_time(
        context.callbacks,
        context.manager.engine.campaign(),
    );
    let quick_load_key = context.input.translator.get_binding(GameKey::QuickLoad1);
    let restart_snapshot_exists =
        context.ui.restart_allowed && context.callbacks.save_manager.has_restart_save();
    let restart_allowed = context.ui.restart_allowed;
    TerminalDebriefingPage {
        kind,
        body,
        mission_length,
        quick_load_key,
        restart_allowed,
        restart_snapshot_exists,
        mission_id: terminal_mission_id,
        won,
        mission_stat: context.manager.engine.mission_stat().clone(),
    }
}

impl TerminalDebriefingState {
    fn new(
        context: &mut TerminalDebriefingContext<'_>,
        exit_code: GameCode,
        popup_title: String,
        page: TerminalDebriefingPage,
    ) -> Self {
        let popup_kind = engine_player_command::ModalKind::MissionState {
            kind: engine_player_command::MissionStateModalKind::EndState { won: page.won },
        };
        let resources = context
            .resources
            .menu
            .as_ref()
            .expect("terminal modal resources were checked before state construction");
        let phase =
            TerminalDebriefingPhase::MissionState(crate::ingame_menu::MissionStatePopupState::new(
                &context.presentation.renderer,
                resources,
                popup_title.clone(),
                page.won,
                None,
            ));
        Self {
            exit_code,
            popup_kind,
            page,
            phase,
            http_result: None,
        }
    }

    pub(super) fn current_kind(&self) -> engine_player_command::ModalKind {
        match self.phase {
            TerminalDebriefingPhase::MissionState(_)
            | TerminalDebriefingPhase::AwaitingMissionAuthority => self.popup_kind.clone(),
            TerminalDebriefingPhase::Debriefing(_)
            | TerminalDebriefingPhase::LoadPicker { .. }
            | TerminalDebriefingPhase::AwaitingFinalAuthority => self.page.kind.clone(),
        }
    }

    pub(super) fn queue_http_result(
        &mut self,
        kind: engine_player_command::ModalKind,
        result: engine_player_command::DialogResult,
    ) -> Result<(), String> {
        let current = self.current_kind();
        if current != kind {
            return Err(format!(
                "terminal modal changed from {} to {} before dismissal was applied",
                serde_json::to_string(&kind).expect("ModalKind serializes"),
                serde_json::to_string(&current).expect("ModalKind serializes")
            ));
        }
        self.http_result = Some((kind, result));
        Ok(())
    }

    fn begin_debriefing(&self, resources: &IngameMenuResources) -> TerminalDebriefingPhase {
        TerminalDebriefingPhase::Debriefing(crate::ingame_menu::DebriefingModalState::new(
            resources,
            self.page.body.clone(),
            Some(&self.page.mission_stat),
            self.page.mission_length,
            self.page.won,
            self.page.restart_allowed,
            self.page.quick_load_key,
            self.page.restart_snapshot_exists,
            false,
        ))
    }

    fn modal_net<'a>(
        context: &'a TerminalDebriefingContext<'_>,
        kind: engine_player_command::ModalKind,
    ) -> Option<crate::ingame_menu::ModalNet<'a>> {
        context.host.transport.net.as_ref().map(|net| {
            crate::ingame_menu::ModalNet::new(
                net,
                kind,
                context.host.transport.local_seat == engine_player_command::PlayerId::HOST,
            )
        })
    }

    fn record_popup_decision(
        &mut self,
        context: &mut TerminalDebriefingContext<'_>,
        result: engine_player_command::DialogResult,
    ) {
        context
            .frame
            .modal_dismissals
            .push(engine_player_command::PlayerCommand::ModalDismiss {
                kind: self.popup_kind.clone(),
                result,
            });
        let resources = context
            .resources
            .menu
            .as_ref()
            .expect("terminal modal resources disappeared during mission-state transition");
        self.phase = self.begin_debriefing(resources);
    }

    fn finish_final_decision(
        &mut self,
        context: &mut TerminalDebriefingContext<'_>,
        result: engine_player_command::DialogResult,
    ) -> TerminalDebriefingProgress {
        context
            .frame
            .modal_dismissals
            .push(engine_player_command::PlayerCommand::ModalDismiss {
                kind: self.page.kind.clone(),
                result,
            });
        let outcome = final_debriefing_outcome_from_replay(result);
        context.game.operation.set(self.exit_code);
        if apply_terminal_debriefing_action(context, &outcome, self.page.mission_id) {
            TerminalDebriefingProgress::EmergencyExit
        } else {
            TerminalDebriefingProgress::Complete
        }
    }

    fn poll_authoritative_decision(
        context: &TerminalDebriefingContext<'_>,
        kind: engine_player_command::ModalKind,
    ) -> Option<engine_player_command::DialogResult> {
        Self::modal_net(context, kind).and_then(|modal| modal.poll_remote_dismissal())
    }

    fn publish_or_accept_local(
        context: &TerminalDebriefingContext<'_>,
        kind: engine_player_command::ModalKind,
        result: engine_player_command::DialogResult,
    ) -> bool {
        let Some(modal) = Self::modal_net(context, kind) else {
            return true;
        };
        modal.publish(result);
        modal.is_authority()
    }

    fn tick(&mut self, context: &mut TerminalDebriefingContext<'_>) -> TerminalDebriefingProgress {
        if let Some((kind, result)) = self.http_result.take() {
            if kind == self.popup_kind {
                if Self::publish_or_accept_local(context, kind, result) {
                    self.record_popup_decision(context, result);
                } else {
                    self.phase = TerminalDebriefingPhase::AwaitingMissionAuthority;
                }
                return TerminalDebriefingProgress::Pending;
            }
            if kind == self.page.kind {
                if let TerminalDebriefingPhase::LoadPicker { picker, .. } = &mut self.phase {
                    picker.close(&mut context.presentation.renderer);
                }
                if Self::publish_or_accept_local(context, kind, result) {
                    return self.finish_final_decision(context, result);
                }
                self.phase = TerminalDebriefingPhase::AwaitingFinalAuthority;
                return TerminalDebriefingProgress::Pending;
            }
            panic!("validated terminal HTTP modal kind changed before tick")
        }
        if let Some(result) =
            pop_matching_dismissal(&mut context.frame.replay_modal_dismissals, &self.popup_kind)
        {
            self.record_popup_decision(context, result);
            return TerminalDebriefingProgress::Pending;
        }

        match &mut self.phase {
            TerminalDebriefingPhase::MissionState(state) => {
                if let Some(result) =
                    Self::poll_authoritative_decision(context, self.popup_kind.clone())
                {
                    self.record_popup_decision(context, result);
                    return TerminalDebriefingProgress::Pending;
                }
                let resources = context
                    .resources
                    .menu
                    .as_ref()
                    .expect("terminal mission-state resources disappeared");
                let cursor = default_modal_cursor(
                    &mut context.presentation.sprites.cursor_renderer,
                    &mut context.resources.cursor,
                    &mut context.presentation.renderer,
                );
                let Some(confirmed) = state.tick(
                    context.window,
                    &mut context.presentation.renderer,
                    resources,
                    Some(cursor),
                ) else {
                    return TerminalDebriefingProgress::Pending;
                };
                let result = if confirmed {
                    engine_player_command::DialogResult::Completed
                } else {
                    engine_player_command::DialogResult::Aborted
                };
                if Self::publish_or_accept_local(context, self.popup_kind.clone(), result) {
                    self.record_popup_decision(context, result);
                } else {
                    self.phase = TerminalDebriefingPhase::AwaitingMissionAuthority;
                }
                TerminalDebriefingProgress::Pending
            }
            TerminalDebriefingPhase::AwaitingMissionAuthority => {
                if let Some(result) =
                    Self::poll_authoritative_decision(context, self.popup_kind.clone())
                {
                    self.record_popup_decision(context, result);
                }
                TerminalDebriefingProgress::Pending
            }
            TerminalDebriefingPhase::Debriefing(state) => {
                if let Some(result) = pop_matching_dismissal(
                    &mut context.frame.replay_modal_dismissals,
                    &self.page.kind,
                ) {
                    return self.finish_final_decision(context, result);
                }
                if let Some(result) =
                    Self::poll_authoritative_decision(context, self.page.kind.clone())
                {
                    return self.finish_final_decision(context, result);
                }
                let resources = context
                    .resources
                    .menu
                    .as_ref()
                    .expect("terminal debriefing resources disappeared");
                let cursor = default_modal_cursor(
                    &mut context.presentation.sprites.cursor_renderer,
                    &mut context.resources.cursor,
                    &mut context.presentation.renderer,
                );
                let Some(outcome) = state.tick(
                    context.window,
                    &mut context.presentation.renderer,
                    resources,
                    Some(cursor),
                ) else {
                    return TerminalDebriefingProgress::Pending;
                };
                if let DebriefingOutcome::LoadAttempt {
                    body_remaining,
                    was_on_stat,
                } = outcome
                {
                    let detailed_metadata = context
                        .host
                        .application_context
                        .active_profile_snapshot()
                        .unwrap_or_else(|error| {
                            panic!(
                                "terminal debriefing load picker requires an active profile: {error}"
                            )
                        })
                        .gameplay_config
                        .detailed_save_metadata;
                    self.phase = TerminalDebriefingPhase::LoadPicker {
                        picker: crate::ingame_menu::LoadPickerModalState::new(
                            context.window,
                            &context.presentation.renderer,
                            &mut context.callbacks.save_manager,
                            detailed_metadata,
                            context.host.transport.net.is_some(),
                        ),
                        body: body_remaining,
                        was_on_stat,
                    };
                    return TerminalDebriefingProgress::Pending;
                }
                let settled = match outcome {
                    DebriefingOutcome::Ok { .. } => SettledDebriefingOutcome::Ok,
                    DebriefingOutcome::Restart => SettledDebriefingOutcome::Restart,
                    DebriefingOutcome::EmergencyEnd => SettledDebriefingOutcome::EmergencyEnd,
                    DebriefingOutcome::LoadAttempt { .. } => unreachable!(),
                };
                let result = final_debriefing_result(&settled);
                if Self::publish_or_accept_local(context, self.page.kind.clone(), result) {
                    self.finish_final_decision(context, result)
                } else {
                    self.phase = TerminalDebriefingPhase::AwaitingFinalAuthority;
                    TerminalDebriefingProgress::Pending
                }
            }
            TerminalDebriefingPhase::LoadPicker {
                picker,
                body,
                was_on_stat,
            } => {
                let resources = context
                    .resources
                    .menu
                    .as_ref()
                    .expect("terminal load-picker resources disappeared");
                let cursor = default_modal_cursor(
                    &mut context.presentation.sprites.cursor_renderer,
                    &mut context.resources.cursor,
                    &mut context.presentation.renderer,
                );
                let outcome = picker.tick(
                    context.window,
                    &mut context.presentation.renderer,
                    resources,
                    Some(cursor),
                    &mut context.callbacks.save_manager,
                    Some(&mut context.host.audio.sound),
                    context
                        .audio
                        .backend
                        .as_mut()
                        .map(|backend| backend as &mut dyn crate::sound::AudioBackend),
                    Some(&context.audio.sample_loader),
                );
                let Some(outcome) = outcome else {
                    return TerminalDebriefingProgress::Pending;
                };
                picker.close(&mut context.presentation.renderer);
                match outcome {
                    SaveLoadOutcome::Cancel => {
                        self.phase = TerminalDebriefingPhase::Debriefing(
                            crate::ingame_menu::DebriefingModalState::new(
                                resources,
                                body.clone(),
                                Some(&self.page.mission_stat),
                                self.page.mission_length,
                                self.page.won,
                                self.page.restart_allowed,
                                self.page.quick_load_key,
                                self.page.restart_snapshot_exists,
                                *was_on_stat,
                            ),
                        );
                        TerminalDebriefingProgress::Pending
                    }
                    SaveLoadOutcome::Slot(slot) => {
                        let result =
                            engine_player_command::DialogResult::Load { slot: slot as u32 };
                        if Self::publish_or_accept_local(context, self.page.kind.clone(), result) {
                            self.finish_final_decision(context, result)
                        } else {
                            self.phase = TerminalDebriefingPhase::AwaitingFinalAuthority;
                            TerminalDebriefingProgress::Pending
                        }
                    }
                }
            }
            TerminalDebriefingPhase::AwaitingFinalAuthority => {
                if let Some(result) =
                    Self::poll_authoritative_decision(context, self.page.kind.clone())
                {
                    self.finish_final_decision(context, result)
                } else {
                    TerminalDebriefingProgress::Pending
                }
            }
        }
    }
}

fn apply_terminal_debriefing_action(
    context: &mut TerminalDebriefingContext<'_>,
    outcome: &SettledDebriefingOutcome,
    mission_id: u32,
) -> bool {
    match terminal_debriefing_action(outcome, mission_id) {
        TerminalDebriefingAction::Continue => false,
        TerminalDebriefingAction::LoadRestart => {
            context.callbacks.pending = Some(SaveLoadRequest::LoadRestart);
            context.game.operation.set(GameCode::LevelInProgress);
            false
        }
        TerminalDebriefingAction::Load { slot, mission_id } => {
            context.callbacks.pending = Some(SaveLoadRequest::Load {
                slot: Some(slot),
                mission_id,
                save: None,
            });
            context.game.operation.set(GameCode::LevelInProgress);
            false
        }
        TerminalDebriefingAction::EmergencyExit => true,
    }
}

/// Advance the terminal mission-state/debrief/load sequence by one outer frame.
pub(super) fn drive_tick_exit_modals(
    mut context: TerminalDebriefingContext<'_>,
) -> TerminalDebriefingProgress {
    if let Some(mut state) = context.ui.terminal_debriefing.take() {
        let progress = state.tick(&mut context);
        if progress == TerminalDebriefingProgress::Pending {
            context.ui.terminal_debriefing = Some(state);
        }
        return progress;
    }

    let Some(exit_code) = context.tick_exit_code else {
        return TerminalDebriefingProgress::Inactive;
    };
    tracing::info!("Engine tick returned: {:?}", exit_code);
    // A history replay restores campaign progression while applying terminal
    // updates. Freeze the loaded mission identity first so the debriefing and
    // load/restart actions continue to refer to the mission just played.
    let terminal_mission_id = current_mission_id(
        context.manager.engine.campaign(),
        &context.assets.profile_manager,
    );
    let (completed_at_unix_seconds, campaign_run_nonce) = mission_completion_clock();

    // Campaign/stat updates precede both terminal graphical surfaces, matching
    // RHgame.cpp's mission-end operation handling.
    let difficulty = context.manager.engine.sim_config().difficulty;
    dispatch_local_command(
        context.host,
        &mut context.manager.engine,
        &mut context.frame.post_commands,
        context.assets,
        &PlayerCommand::ApplyQuitMissionUpdates {
            exit_code,
            difficulty,
            completed_at_unix_seconds,
            campaign_run_nonce,
        },
    );
    if exit_code == GameCode::LevelSucceeded {
        let run_context = robin_engine::achievement::AchievementRunContext {
            kind: context.host.achievement_run_kind,
            multiplayer: context.host.transport.net.is_some(),
            replay_playback: context.host.achievement_replay_playback,
            headless: context.host.achievement_headless,
            // The engine ORs its authoritative cheat flag into this value.
            cheat_used: false,
        };
        let update = context
            .manager
            .engine
            .promote_mission_achievement_results(
                robin_engine::achievement::AchievementUnlockPolicy::default(),
                run_context,
                &context.assets.profile_manager,
            )
            .unwrap_or_else(|error| panic!("achievement history promotion failed: {error}"));
        if let Some(update) = update
            && !update.blockers.is_empty()
        {
            tracing::info!(
                ?run_context,
                "achievement results calculated but unlock/history persistence was blocked"
            );
        }
    }

    // ApplyQuitMissionUpdates first appends the immutable raw attempt and the
    // success path above attaches its exactly-once eligibility attestation.
    // Only then promote that canonical campaign history into the profile, so
    // failed/interrupted attempts and policy-blocked calculations remain
    // losslessly auditable without becoming awarded badges.
    let campaign = context.manager.engine.campaign().clone();
    context
        .host
        .application_context
        .with_player_profiles_mut(|profiles| {
            let profile = profiles.get_active_mut().unwrap_or_else(|| {
                panic!("campaign-history promotion has no active player profile")
            });
            profile
                .promote_campaign_history(&campaign, &context.assets.profile_manager)
                .unwrap_or_else(|error| panic!("campaign-history promotion failed: {error}"));
            if let Err(error) = profiles.save() {
                #[cfg(not(target_arch = "wasm32"))]
                panic!("failed to persist campaign history: {error}");
                #[cfg(target_arch = "wasm32")]
                tracing::warn!(
                    "Failed to persist campaign history in browser storage; keeping it in memory for this session: {error}"
                );
            }
        })
        .unwrap_or_else(|error| panic!("campaign profile synchronization failed: {error}"));

    let Some((popup_title, _)) = crate::ingame_menu::mission_state_text(exit_code) else {
        return TerminalDebriefingProgress::Complete;
    };
    if context.resources.menu.is_none() {
        tracing::warn!("terminal debriefing resources unavailable — skipping modal sequence");
        return TerminalDebriefingProgress::Complete;
    }

    let won = exit_code == GameCode::LevelSucceeded;
    let page = terminal_debriefing_page(&mut context, won, terminal_mission_id);
    let mut state =
        TerminalDebriefingState::new(&mut context, exit_code, popup_title.to_string(), page);
    // The operation pass runs before modal presentation on subsequent frames.
    // Hold it in-progress until the typed terminal outcome is settled.
    context.game.operation.set(GameCode::LevelInProgress);
    let progress = state.tick(&mut context);
    if progress == TerminalDebriefingProgress::Pending {
        context.ui.terminal_debriefing = Some(state);
    }
    progress
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn terminal_text_table_follows_win_loss_outcome() {
        assert_eq!(debriefing_text_table_id(true, 100, 200), 100);
        assert_eq!(debriefing_text_table_id(false, 100, 200), 200);
    }
}
