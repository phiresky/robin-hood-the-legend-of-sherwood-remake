//! Frame-driven mission-end popup, debriefing renderer, and load picker.
//!
//! The complete sequence is retained across outer graphical frames so network,
//! replay, and HTTP services continue draining while mission-end UI is open.

use super::interactive::{
    MissionAudio, MissionInput, MissionPresentation, MissionResources, MissionUi,
};
use super::*;
use crate::game::Game;
use crate::ingame_menu::modal_net::ModalDismissalGate;
use crate::ingame_menu::widget_bridge::default_modal_cursor;

/// The wire index describes a dialog answer, never filesystem authority. Only
/// a locally accepted selection can bind it to this store's live generation.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct TerminalLoadSelection {
    wire_slot: u32,
    handle: crate::savegame::SlotHandle,
}

impl TerminalLoadSelection {
    fn capture(manager: &crate::savegame::SaveGameManager, slot: u32) -> Result<Self, String> {
        Ok(Self {
            wire_slot: slot,
            handle: manager
                .slot_handle(slot as usize)
                .map_err(|error| format!("terminal load selection rejected: {error:#}"))?,
        })
    }

    fn resolve(
        self,
        manager: &crate::savegame::SaveGameManager,
        wire_slot: usize,
    ) -> Result<crate::savegame::SlotHandle, String> {
        if self.wire_slot as usize != wire_slot {
            return Err(
                "terminal load decision does not match the retained local selection".into(),
            );
        }
        manager
            .resolve_handle(&self.handle)
            .map_err(|error| format!("terminal load selection expired: {error:#}"))?;
        Ok(self.handle)
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
enum TerminalLoadTarget {
    Local(crate::savegame::SlotHandle),
    AuthoritativeSnapshot,
    ReplayContinuation,
}

fn terminal_load_target(
    manager: &crate::savegame::SaveGameManager,
    selection: Option<TerminalLoadSelection>,
    wire_slot: usize,
    playing_back: bool,
    is_client: bool,
) -> Result<TerminalLoadTarget, String> {
    if playing_back {
        return Ok(TerminalLoadTarget::ReplayContinuation);
    }
    if is_client {
        return Ok(TerminalLoadTarget::AuthoritativeSnapshot);
    }
    selection
        .ok_or_else(|| "terminal load decision has no local selection authority".to_owned())?
        .resolve(manager, wire_slot)
        .map(TerminalLoadTarget::Local)
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

fn mission_completion_clock() -> (Option<i64>, Option<u64>) {
    // This runs on ordinary browser frames too; std's wasm SystemTime panics.
    let Ok(duration) = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH) else {
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

fn leaderboard_outcome(
    exit_code: GameCode,
) -> Option<crate::leaderboard_mission_end::MissionEndOutcome> {
    match exit_code {
        GameCode::LevelSucceeded => Some(crate::leaderboard_mission_end::MissionEndOutcome::Won),
        GameCode::LevelFailed => Some(crate::leaderboard_mission_end::MissionEndOutcome::Lost),
        GameCode::LevelInterrupted => {
            Some(crate::leaderboard_mission_end::MissionEndOutcome::Interrupted)
        }
        GameCode::LevelInProgress
        | GameCode::LevelNext
        | GameCode::LevelRestart
        | GameCode::Quit
        | GameCode::LevelLoad
        | GameCode::LevelSave => None,
    }
}

/// Recorded terminal commands are authoritative, including their clock and
/// nonce. Only live play may produce another command at this host boundary.
fn stage_terminal_campaign_update(
    playing_back: bool,
    transport: &crate::host::HostTransport,
    frame: &mut MissionFrame,
    exit_code: GameCode,
    difficulty: robin_engine::player_profile::DifficultyLevel,
    current_attempt_sequence: u64,
) -> u64 {
    if playing_back {
        // A multiplayer echo can be a pre-command already applied before this
        // tick. Its debrief is ready now; a post-command (or later echo) still
        // needs the normal attempt-sequence advancement gate.
        let applied = frame
            .commands()
            .iter()
            .filter(|input| {
                matches!(
                    &input.command,
                    PlayerCommand::ApplyQuitMissionUpdates { .. }
                )
            })
            .count();
        assert!(
            applied <= 1,
            "one mission frame cannot contain multiple terminal updates"
        );
        return if applied == 1 {
            current_attempt_sequence
                .checked_sub(1)
                .expect("applied terminal command must advance the campaign attempt sequence")
        } else {
            current_attempt_sequence
        };
    }
    let (completed_at_unix_seconds, campaign_run_nonce) = mission_completion_clock();
    dispatch_local_command(
        transport,
        &mut frame.stage_post_commands(),
        &PlayerCommand::ApplyQuitMissionUpdates {
            exit_code,
            difficulty,
            completed_at_unix_seconds,
            campaign_run_nonce,
        },
    );
    current_attempt_sequence
}

fn terminal_campaign_update_applied(current_sequence: u64, previous_sequence: u64) -> bool {
    current_sequence > previous_sequence
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
    /// Includes retrying a failed publication as well as awaiting a successfully
    /// proposed client decision; the gate retains that distinction and outcome.
    AwaitingMissionAuthority,
    Debriefing(crate::ingame_menu::DebriefingModalState),
    LoadPicker {
        picker: crate::ingame_menu::LoadPickerModalState,
        body: String,
        was_on_stat: bool,
    },
    AwaitingFinalAuthority,
    LeaderboardPendingStart {
        outcome: SettledDebriefingOutcome,
    },
    AwaitingLeaderboard {
        outcome: SettledDebriefingOutcome,
    },
    /// Keep the old terminal engine paused even if the host's prepare packet
    /// arrives after this peer finishes its leaderboard. Network ingress stays
    /// active and the input phase consumes the eventual committed transition.
    AwaitingSnapshot,
}

pub(super) struct TerminalDebriefingState {
    decisions: super::session_policy::TerminalDecisionOrder,
    popup_dismissal: ModalDismissalGate,
    final_dismissal: ModalDismissalGate,
    exit_code: GameCode,
    popup_kind: engine_player_command::ModalKind,
    page: TerminalDebriefingPage,
    phase: TerminalDebriefingPhase,
    leaderboard_preparation: Option<super::leaderboard_runtime::MissionEndPreparation>,
    local_load: Option<TerminalLoadSelection>,
    http_result: Option<(
        engine_player_command::ModalKind,
        engine_player_command::DialogResult,
    )>,
}

/// Terminal command staged by the host but not yet observed in the engine.
/// Multiplayer may echo this command several frames later, so presentation
/// and profile promotion wait for the campaign attempt sequence to advance.
pub(super) struct PendingTerminalDebriefing {
    exit_code: GameCode,
    terminal_mission_id: u32,
    previous_attempt_sequence: u64,
    popup_title: Option<String>,
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
    pub(super) playing_back: bool,
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
    pub(super) leaderboard: &'a mut Option<super::leaderboard_runtime::MissionLeaderboardRuntime>,
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
        && context
            .host
            .frontend
            .preferences()
            .gameplay_config()
            .show_achievement_debrief
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
    fn await_remote_snapshot(
        &mut self,
        outcome: &SettledDebriefingOutcome,
        playing_back: bool,
        local_seat: engine_player_command::PlayerId,
    ) -> bool {
        if playing_back
            || local_seat == engine_player_command::PlayerId::HOST
            || !matches!(outcome, SettledDebriefingOutcome::Load { .. })
        {
            return false;
        }
        // A locally proposed index cannot authorize this peer's file, even
        // when the host eventually chooses the same numeric answer.
        self.local_load = None;
        self.phase = TerminalDebriefingPhase::AwaitingSnapshot;
        true
    }

    fn new(
        context: &mut TerminalDebriefingContext<'_>,
        exit_code: GameCode,
        popup_title: String,
        page: TerminalDebriefingPage,
        leaderboard_preparation: super::leaderboard_runtime::MissionEndPreparation,
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
            popup_dismissal: ModalDismissalGate::default(),
            final_dismissal: ModalDismissalGate::default(),
            decisions: super::session_policy::TerminalDecisionOrder::new(
                page.won,
                match page.kind {
                    engine_player_command::ModalKind::FinalDebriefing { text_id } => text_id,
                    _ => unreachable!("terminal page identity"),
                },
            ),
            exit_code,
            popup_kind,
            page,
            phase,
            leaderboard_preparation: Some(leaderboard_preparation),
            local_load: None,
            http_result: None,
        }
    }

    pub(super) fn current_kind(&self) -> Option<engine_player_command::ModalKind> {
        self.decisions.current_kind()
    }

    pub(super) fn queue_http_result(
        &mut self,
        kind: engine_player_command::ModalKind,
        result: engine_player_command::DialogResult,
        save_manager: Option<&crate::savegame::SaveGameManager>,
    ) -> Result<(), String> {
        let current = self.current_kind().ok_or_else(|| {
            "terminal narrative is complete; leaderboard presentation is not an authoritative modal"
                .to_owned()
        })?;
        if current != kind {
            return Err(format!(
                "terminal modal changed from {} to {} before dismissal was applied",
                serde_json::to_string(&kind).expect("ModalKind serializes"),
                serde_json::to_string(&current).expect("ModalKind serializes")
            ));
        }
        if self.http_result.is_some()
            || self.popup_dismissal.is_pending()
            || self.final_dismissal.is_pending()
        {
            return Err(
                "terminal modal already retains a local decision pending publication or authority"
                    .to_owned(),
            );
        }
        if let engine_player_command::DialogResult::Load { slot } = result {
            let manager = save_manager.ok_or_else(|| {
                "terminal load dismissal requires the local save catalog".to_owned()
            })?;
            self.local_load = Some(TerminalLoadSelection::capture(manager, slot)?);
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
        context.host.transport.net().map(|net| {
            crate::ingame_menu::ModalNet::new(
                net,
                kind,
                context.host.transport.local_seat() == engine_player_command::PlayerId::HOST,
            )
        })
    }

    fn record_popup_decision(
        &mut self,
        context: &mut TerminalDebriefingContext<'_>,
        result: engine_player_command::DialogResult,
    ) {
        self.decisions
            .accept(&self.popup_kind, result)
            .unwrap_or_else(|error| panic!("terminal popup admission: {error}"));
        self.popup_dismissal.retire();
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
        self.decisions
            .accept(&self.page.kind, result)
            .unwrap_or_else(|error| panic!("terminal final admission: {error}"));
        self.final_dismissal.retire();
        context
            .frame
            .modal_dismissals
            .push(engine_player_command::PlayerCommand::ModalDismiss {
                kind: self.page.kind.clone(),
                result,
            });
        let outcome = final_debriefing_outcome_from_replay(result);
        if matches!(&outcome, SettledDebriefingOutcome::EmergencyEnd) {
            context.game.operation.set(self.exit_code);
            return if apply_terminal_debriefing_action(
                context,
                &outcome,
                self.page.mission_id,
                self.local_load.take(),
            ) {
                TerminalDebriefingProgress::EmergencyExit
            } else {
                TerminalDebriefingProgress::Complete
            };
        }
        self.phase = TerminalDebriefingPhase::LeaderboardPendingStart { outcome };
        TerminalDebriefingProgress::Pending
    }

    fn poll_authoritative_decision(
        context: &TerminalDebriefingContext<'_>,
        kind: engine_player_command::ModalKind,
        dismissal: &mut ModalDismissalGate,
    ) -> Option<engine_player_command::DialogResult> {
        dismissal.poll(Self::modal_net(context, kind).as_ref())
    }

    fn publish_or_accept_local(
        context: &TerminalDebriefingContext<'_>,
        kind: engine_player_command::ModalKind,
        result: engine_player_command::DialogResult,
        dismissal: &mut ModalDismissalGate,
    ) -> Option<engine_player_command::DialogResult> {
        dismissal.request(result, Self::modal_net(context, kind).as_ref())
    }

    fn tick(&mut self, context: &mut TerminalDebriefingContext<'_>) -> TerminalDebriefingProgress {
        if let Some((kind, result)) = self.http_result.take() {
            if kind == self.popup_kind {
                if let Some(result) =
                    Self::publish_or_accept_local(context, kind, result, &mut self.popup_dismissal)
                {
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
                if let Some(result) =
                    Self::publish_or_accept_local(context, kind, result, &mut self.final_dismissal)
                {
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
                if let Some(result) = Self::poll_authoritative_decision(
                    context,
                    self.popup_kind.clone(),
                    &mut self.popup_dismissal,
                ) {
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
                if let Some(result) = Self::publish_or_accept_local(
                    context,
                    self.popup_kind.clone(),
                    result,
                    &mut self.popup_dismissal,
                ) {
                    self.record_popup_decision(context, result);
                } else {
                    self.phase = TerminalDebriefingPhase::AwaitingMissionAuthority;
                }
                TerminalDebriefingProgress::Pending
            }
            TerminalDebriefingPhase::AwaitingMissionAuthority => {
                if let Some(result) = Self::poll_authoritative_decision(
                    context,
                    self.popup_kind.clone(),
                    &mut self.popup_dismissal,
                ) {
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
                if let Some(result) = Self::poll_authoritative_decision(
                    context,
                    self.page.kind.clone(),
                    &mut self.final_dismissal,
                ) {
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
                        .application_context()
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
                            context.host.transport.net().is_some(),
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
                if let Some(result) = Self::publish_or_accept_local(
                    context,
                    self.page.kind.clone(),
                    result,
                    &mut self.final_dismissal,
                ) {
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
                        let slot =
                            u32::try_from(slot).expect("save catalog exceeds wire slot range");
                        self.local_load = match TerminalLoadSelection::capture(
                            &context.callbacks.save_manager,
                            slot,
                        ) {
                            Ok(selection) => Some(selection),
                            Err(error) => {
                                tracing::error!(%error, "debriefing selection was removed before acceptance");
                                self.phase = self.begin_debriefing(resources);
                                return TerminalDebriefingProgress::Pending;
                            }
                        };
                        let result = engine_player_command::DialogResult::Load { slot };
                        if let Some(result) = Self::publish_or_accept_local(
                            context,
                            self.page.kind.clone(),
                            result,
                            &mut self.final_dismissal,
                        ) {
                            self.finish_final_decision(context, result)
                        } else {
                            self.phase = TerminalDebriefingPhase::AwaitingFinalAuthority;
                            TerminalDebriefingProgress::Pending
                        }
                    }
                }
            }
            TerminalDebriefingPhase::AwaitingFinalAuthority => {
                if let Some(result) = Self::poll_authoritative_decision(
                    context,
                    self.page.kind.clone(),
                    &mut self.final_dismissal,
                ) {
                    self.finish_final_decision(context, result)
                } else {
                    TerminalDebriefingProgress::Pending
                }
            }
            TerminalDebriefingPhase::LeaderboardPendingStart { .. } => {
                if context.ui.active_ui_task.is_some() {
                    return TerminalDebriefingProgress::Pending;
                }
                tracing::debug!(
                    active_scripted_modal = context.ui.active_modal.is_some(),
                    pending_scripted_modals = ?context.host.effects.pending_modal_kinds(),
                    pending_mission_state = context.host.effects.has_signal(crate::host::HostSignal::MissionStatePopup),
                    "terminal flow handing presentation to mission-end leaderboard"
                );
                let preparation = self.leaderboard_preparation.take().unwrap_or_else(|| {
                    panic!("terminal leaderboard preparation disappeared before presentation")
                });
                context.ui.active_ui_task =
                    Some(super::ui_task_state::ActiveUiTask::MissionEndLeaderboard(
                        super::leaderboard_runtime::MissionEndLeaderboardTaskState::new(
                            preparation,
                            context.callbacks.application_context(),
                        ),
                    ));
                let TerminalDebriefingPhase::LeaderboardPendingStart { outcome } =
                    std::mem::replace(
                        &mut self.phase,
                        TerminalDebriefingPhase::AwaitingFinalAuthority,
                    )
                else {
                    unreachable!()
                };
                self.phase = TerminalDebriefingPhase::AwaitingLeaderboard { outcome };
                TerminalDebriefingProgress::Pending
            }
            TerminalDebriefingPhase::AwaitingLeaderboard { .. } => {
                if context
                    .ui
                    .active_ui_task
                    .as_ref()
                    .is_some_and(|task| task.is_mission_end_leaderboard())
                {
                    return TerminalDebriefingProgress::Pending;
                }
                let TerminalDebriefingPhase::AwaitingLeaderboard { outcome } = std::mem::replace(
                    &mut self.phase,
                    TerminalDebriefingPhase::AwaitingFinalAuthority,
                ) else {
                    unreachable!()
                };
                if self.await_remote_snapshot(
                    &outcome,
                    context.playing_back,
                    context.host.transport.local_seat(),
                ) {
                    context.game.operation.set(GameCode::LevelInProgress);
                    return TerminalDebriefingProgress::Pending;
                }
                context.game.operation.set(self.exit_code);
                if apply_terminal_debriefing_action(
                    context,
                    &outcome,
                    self.page.mission_id,
                    self.local_load.take(),
                ) {
                    TerminalDebriefingProgress::EmergencyExit
                } else {
                    TerminalDebriefingProgress::Complete
                }
            }
            TerminalDebriefingPhase::AwaitingSnapshot => {
                // No timeout or local fallback: normal ingress owns snapshot
                // commits and fatal disconnects. Replay never enters this hold;
                // its recorded restore may already have run before this phase.
                TerminalDebriefingProgress::Pending
            }
        }
    }
}

fn apply_terminal_debriefing_action(
    context: &mut TerminalDebriefingContext<'_>,
    outcome: &SettledDebriefingOutcome,
    mission_id: u32,
    local_load: Option<TerminalLoadSelection>,
) -> bool {
    match terminal_debriefing_action(outcome, mission_id) {
        TerminalDebriefingAction::Continue => false,
        TerminalDebriefingAction::LoadRestart => {
            context
                .callbacks
                .queue_operation(SaveLoadRequest::LoadRestart);
            context.game.operation.set(GameCode::LevelInProgress);
            false
        }
        TerminalDebriefingAction::Load { slot, mission_id } => {
            // Losing a transport must not turn a former client into local file
            // authority while its host-authored decision is settling.
            let is_client =
                context.host.transport.local_seat() != engine_player_command::PlayerId::HOST;
            let slot = match terminal_load_target(
                &context.callbacks.save_manager,
                local_load,
                slot,
                context.playing_back,
                is_client,
            ) {
                Ok(TerminalLoadTarget::Local(slot)) => slot,
                Ok(TerminalLoadTarget::AuthoritativeSnapshot) => {
                    unreachable!("client load must retain its terminal snapshot-wait phase")
                }
                Err(error) => {
                    tracing::error!("Debriefing load rejected stale slot: {error:#}");
                    return false;
                }
                Ok(TerminalLoadTarget::ReplayContinuation) => {
                    tracing::debug!(
                        slot,
                        "terminal replay load is owned by recorded restore boundaries"
                    );
                    context.game.operation.set(GameCode::LevelInProgress);
                    return false;
                }
            };
            context.callbacks.queue_operation(SaveLoadRequest::Load {
                slot: Some(slot),
                mission_id,
            });
            context.game.operation.set(GameCode::LevelInProgress);
            false
        }
        TerminalDebriefingAction::EmergencyExit => true,
    }
}

fn settle_terminal_debriefing(
    context: &mut TerminalDebriefingContext<'_>,
    pending: PendingTerminalDebriefing,
) -> TerminalDebriefingProgress {
    let PendingTerminalDebriefing {
        exit_code,
        terminal_mission_id,
        popup_title,
        ..
    } = pending;
    if exit_code == GameCode::LevelSucceeded {
        let run_context = context
            .host
            .session_achievement_eligibility()
            .unwrap_or_else(|error| panic!("achievement history promotion failed: {error}"))
            .promotion_context(context.host.transport.net().is_some());
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

    // The deterministic terminal command has now appended the raw attempt.
    // Promote that exact post-command campaign, never the pre-terminal clone.
    let campaign = context.manager.engine.campaign().clone();
    crate::main_entry::RustCallbacks::promote_terminal_profile(
        context.host.application_context(),
        &campaign,
        &context.assets.profile_manager,
    );

    // This cooperative flow bypasses the synchronous Game terminal handlers
    // that supplied the win/loss jingle in the original main loop.
    if matches!(exit_code, GameCode::LevelSucceeded | GameCode::LevelFailed)
        && let Some(backend) = context.audio.backend.as_mut()
    {
        let jingle = if exit_code == GameCode::LevelSucceeded {
            crate::sound::Jingle::MissionWon
        } else {
            crate::sound::Jingle::MissionLost
        };
        context.host.audio.sound.play_jingle(jingle, backend);
    }

    let Some(popup_title) = popup_title else {
        return TerminalDebriefingProgress::Complete;
    };
    if context.resources.menu.is_none() {
        tracing::warn!("terminal debriefing resources unavailable — skipping modal sequence");
        return TerminalDebriefingProgress::Complete;
    }

    let won = exit_code == GameCode::LevelSucceeded;
    let outcome = leaderboard_outcome(exit_code).unwrap_or_else(|| {
        panic!("terminal debriefing received non-attempt operation {exit_code:?}")
    });
    let leaderboard_preparation = context
        .leaderboard
        .as_mut()
        .unwrap_or_else(|| panic!("interactive mission has no leaderboard runtime"))
        .capture_terminal(outcome)
        .unwrap_or_else(|error| panic!("mission-end leaderboard capture failed: {error}"));
    let page = terminal_debriefing_page(context, won, terminal_mission_id);
    let state = TerminalDebriefingState::new(
        context,
        exit_code,
        popup_title,
        page,
        leaderboard_preparation,
    );
    context.ui.terminal_debriefing = Some(state);
    TerminalDebriefingProgress::Pending
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

    if let Some(pending) = context.ui.pending_terminal_debriefing.take() {
        if !terminal_campaign_update_applied(
            context.manager.engine.campaign().mission_attempt_sequence,
            pending.previous_attempt_sequence,
        ) {
            context.ui.pending_terminal_debriefing = Some(pending);
            return TerminalDebriefingProgress::Pending;
        }
        return settle_terminal_debriefing(&mut context, pending);
    }

    let Some(exit_code) = context.tick_exit_code else {
        return TerminalDebriefingProgress::Inactive;
    };
    super::session_policy::TerminalAdapter::Interactive
        .admit_campaign_transition()
        .expect("interactive terminal adapter owns campaign transition services");
    tracing::info!("Engine tick returned: {:?}", exit_code);
    // A history replay restores campaign progression while applying terminal
    // updates. Freeze the loaded mission identity first so the debriefing and
    // load/restart actions continue to refer to the mission just played.
    let terminal_mission_id = current_mission_id(
        context.manager.engine.campaign(),
        &context.assets.profile_manager,
    );
    // Campaign/stat updates precede both terminal graphical surfaces. Playback
    // consumes the recorded update instead of generating a second local one.
    let previous_attempt_sequence = stage_terminal_campaign_update(
        context.playing_back,
        &context.host.transport,
        context.frame,
        exit_code,
        context.manager.engine.sim_config().difficulty,
        context.manager.engine.campaign().mission_attempt_sequence,
    );
    let popup_title =
        crate::ingame_menu::mission_state_text(exit_code).map(|(title, _)| title.to_owned());
    context.ui.pending_terminal_debriefing = Some(PendingTerminalDebriefing {
        exit_code,
        terminal_mission_id,
        previous_attempt_sequence,
        popup_title,
    });
    // Hold the operation in progress while the deterministic terminal command
    // is applied and the frame-driven debrief/leaderboard sequence settles.
    context.game.operation.set(GameCode::LevelInProgress);
    TerminalDebriefingProgress::Pending
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terminal_state() -> TerminalDebriefingState {
        use robin_engine::player_command::{DebriefingTextId, ModalKind};
        let text_id = DebriefingTextId::from_outcome(true, 0);
        TerminalDebriefingState {
            decisions: super::super::session_policy::TerminalDecisionOrder::new(true, text_id),
            popup_dismissal: ModalDismissalGate::default(),
            final_dismissal: ModalDismissalGate::default(),
            exit_code: GameCode::LevelSucceeded,
            popup_kind: ModalKind::MissionState {
                kind: engine_player_command::MissionStateModalKind::EndState { won: true },
            },
            page: TerminalDebriefingPage {
                kind: ModalKind::FinalDebriefing { text_id },
                body: String::new(),
                mission_length: 0,
                quick_load_key: None,
                restart_allowed: false,
                restart_snapshot_exists: false,
                mission_id: 1,
                won: true,
                mission_stat: Default::default(),
            },
            phase: TerminalDebriefingPhase::AwaitingMissionAuthority,
            leaderboard_preparation: None,
            local_load: None,
            http_result: None,
        }
    }

    fn selected_catalog(root: &std::path::Path) -> crate::savegame::SaveGameManager {
        use crate::savegame::{SaveGame, SaveGameManager, SlotState};
        let mut manager = SaveGameManager::new(root.to_string_lossy().into_owned());
        let mut selected = SaveGame::new("Selected".into(), "Selected".into(), 1);
        selected.timestamp = "20".into();
        manager.insert_test_slot(selected, SlotState::Draft);
        manager
    }

    fn reorder_catalog(manager: &mut crate::savegame::SaveGameManager) {
        use crate::savegame::{SaveGame, SlotState};
        let mut autosave = SaveGame::new("Autosave_1_0000".into(), "Earlier".into(), 1);
        autosave.timestamp = "10".into();
        manager.insert_test_slot(autosave, SlotState::Draft);
        manager.sort_by_time();
        assert_eq!(manager.slot_name(0).unwrap().as_str(), "Autosave_1_0000");
    }

    #[test]
    fn terminal_http_rejects_unbound_selection_without_retaining_a_decision() {
        use robin_engine::player_command::DialogResult;
        let root = tempfile::tempdir().unwrap();
        let manager = selected_catalog(root.path());
        let mut state = terminal_state();
        state
            .decisions
            .accept(&state.popup_kind, DialogResult::Completed)
            .unwrap();
        for (slot, catalog) in [(0, None), (99, Some(&manager))] {
            assert!(
                state
                    .queue_http_result(
                        state.page.kind.clone(),
                        DialogResult::Load { slot },
                        catalog
                    )
                    .is_err()
            );
            assert!(state.http_result.is_none());
            assert!(state.local_load.is_none());
            assert!(!state.final_dismissal.is_pending());
        }
    }

    #[test]
    fn terminal_http_selection_is_pinned_before_authority_and_leaderboard_waits() {
        use robin_engine::player_command::DialogResult;
        let root = tempfile::tempdir().unwrap();
        let mut manager = selected_catalog(root.path());
        let mut state = terminal_state();
        state
            .decisions
            .accept(&state.popup_kind, DialogResult::Completed)
            .unwrap();
        state
            .queue_http_result(
                state.page.kind.clone(),
                DialogResult::Load { slot: 0 },
                Some(&manager),
            )
            .unwrap();
        reorder_catalog(&mut manager);
        // Both waits preserve the same local capability, independent of the
        // numeric result echoed by authority and retained for the replay.
        state.phase = TerminalDebriefingPhase::AwaitingFinalAuthority;
        let (_, decision) = state.http_result.take().unwrap();
        state.phase = TerminalDebriefingPhase::AwaitingLeaderboard {
            outcome: final_debriefing_outcome_from_replay(decision),
        };
        let target =
            terminal_load_target(&manager, state.local_load.take(), 0, false, false).unwrap();
        let TerminalLoadTarget::Local(handle) = target else {
            panic!("local selection lost")
        };
        assert_eq!(handle.name().as_str(), "Selected");
        assert_eq!(manager.resolve_handle(&handle).unwrap(), 1);
    }

    #[test]
    fn terminal_picker_selection_survives_reordering_but_rejects_retirement() {
        let root = tempfile::tempdir().unwrap();
        let mut manager = selected_catalog(root.path());
        let selected = TerminalLoadSelection::capture(&manager, 0).unwrap();
        reorder_catalog(&mut manager);
        manager.remove(1).unwrap();
        assert!(selected.resolve(&manager, 0).is_err());

        // Reusing the exact basename must not revive the selected generation.
        manager.insert_test_slot(
            crate::savegame::SaveGame::new("Selected".into(), "Replacement".into(), 1),
            crate::savegame::SlotState::Draft,
        );
        let selected = TerminalLoadSelection::capture(&manager, 1).unwrap();
        manager.remove(1).unwrap();
        manager.insert_test_slot(
            crate::savegame::SaveGame::new("Selected".into(), "Replacement again".into(), 1),
            crate::savegame::SlotState::Draft,
        );
        assert!(selected.resolve(&manager, 1).is_err());
    }

    #[test]
    fn terminal_remote_and_replay_indices_never_authorize_local_files() {
        let root = tempfile::tempdir().unwrap();
        let manager = selected_catalog(root.path());
        let other = selected_catalog(root.path());
        assert!(
            TerminalLoadSelection::capture(&manager, 0)
                .unwrap()
                .resolve(&other, 0)
                .is_err()
        );
        assert!(
            TerminalLoadSelection::capture(&manager, 0)
                .unwrap()
                .resolve(&manager, 1)
                .is_err()
        );
        assert!(terminal_load_target(&manager, None, 0, false, false).is_err());
        for (playback, client) in [(true, false), (false, true)] {
            // Even a client that proposed this number locally must await the
            // host payload rather than selecting its own same-numbered save.
            let local = TerminalLoadSelection::capture(&manager, 0).unwrap();
            let target = terminal_load_target(&other, Some(local), 0, playback, client).unwrap();
            assert!(match target {
                TerminalLoadTarget::ReplayContinuation => playback,
                TerminalLoadTarget::AuthoritativeSnapshot => client && !playback,
                TerminalLoadTarget::Local(_) => false,
            });
        }
    }

    #[test]
    fn client_terminal_load_keeps_simulation_paused_before_snapshot_prepare_arrives() {
        use robin_engine::player_command::PlayerId;
        let mut state = terminal_state();
        let outcome = SettledDebriefingOutcome::Load { slot: 7 };
        state.phase = TerminalDebriefingPhase::AwaitingLeaderboard {
            outcome: SettledDebriefingOutcome::Load { slot: 7 },
        };
        assert!(state.await_remote_snapshot(&outcome, false, PlayerId(1)));
        assert!(matches!(
            state.phase,
            TerminalDebriefingPhase::AwaitingSnapshot
        ));
        let mut ui = MissionUi::new(false);
        ui.terminal_debriefing = Some(state);
        // There is deliberately no transport prepare/reconnecting flag yet.
        // Retaining the terminal owner itself supplies pre-tick's modal pause.
        assert!(ui.terminal_flow_active());
        assert!(!Game::default().should_run_hourglass(false, false, ui.terminal_flow_active()));
        assert!(
            ui.terminal_debriefing
                .as_ref()
                .unwrap()
                .http_result
                .is_none()
        );
    }

    #[test]
    fn recorded_terminal_load_does_not_wait_for_a_live_snapshot() {
        use robin_engine::player_command::PlayerId;
        let outcome = SettledDebriefingOutcome::Load { slot: 7 };
        for seat in [PlayerId::HOST, PlayerId(1)] {
            let mut state = terminal_state();
            assert!(!state.await_remote_snapshot(&outcome, true, seat));
            assert!(!matches!(
                state.phase,
                TerminalDebriefingPhase::AwaitingSnapshot
            ));
        }
        let mut state = terminal_state();
        assert!(!state.await_remote_snapshot(&outcome, false, PlayerId::HOST));
    }

    #[test]
    fn terminal_http_cannot_replace_a_retained_failed_decision() {
        use robin_engine::multiplayer::{MultiplayerSessionId, NetChannels};
        use robin_engine::player_command::DialogResult;
        let mut state = terminal_state();
        let popup = state.popup_kind.clone();
        state
            .queue_http_result(popup.clone(), DialogResult::Aborted, None)
            .unwrap();
        assert!(
            state
                .queue_http_result(popup.clone(), DialogResult::Completed, None)
                .is_err()
        );
        assert_eq!(
            state.http_result.take(),
            Some((popup.clone(), DialogResult::Aborted))
        );

        for final_page in [false, true] {
            let (net, _incoming, outgoing, _, _) = NetChannels::new();
            net.install_session_id(MultiplayerSessionId([9; 32]))
                .unwrap();
            drop(outgoing);
            let kind = if final_page {
                state.page.kind.clone()
            } else {
                popup.clone()
            };
            let modal = crate::ingame_menu::ModalNet::new(&net, kind.clone(), true);
            let gate = if final_page {
                &mut state.final_dismissal
            } else {
                &mut state.popup_dismissal
            };
            let result = if final_page {
                DialogResult::Load { slot: 7 }
            } else {
                DialogResult::Aborted
            };
            assert_eq!(gate.request(result, Some(&modal)), None);
            assert_eq!(gate.poll(Some(&modal)), None);
            assert!(
                state
                    .queue_http_result(kind.clone(), DialogResult::Completed, None)
                    .is_err()
            );
            assert!(state.http_result.is_none());
            assert_eq!(
                state.current_kind(),
                Some(kind.clone()),
                "failed publication cannot advance terminal decision order"
            );
            let (replacement, _incoming, outgoing, _, _) = NetChannels::new();
            replacement
                .install_session_id(MultiplayerSessionId([9; 32]))
                .unwrap();
            let replacement = crate::ingame_menu::ModalNet::new(&replacement, kind.clone(), true);
            let gate = if final_page {
                &mut state.final_dismissal
            } else {
                &mut state.popup_dismissal
            };
            assert_eq!(gate.poll(Some(&replacement)), Some(result));
            assert!(matches!(
                outgoing.try_recv().unwrap(),
                robin_engine::multiplayer::NetOutbound::ModalDecision {
                    result: observed,
                    ..
                } if observed == result
            ));
            state.decisions.accept(&kind, result).unwrap();
        }
        assert_eq!(state.current_kind(), None);
    }

    #[test]
    fn playback_terminal_keeps_recorded_update_and_debrief_progression() {
        let host = Host::scratch(800.0, 600.0);
        let command = PlayerCommand::ApplyQuitMissionUpdates {
            exit_code: GameCode::LevelFailed,
            difficulty: Default::default(),
            completed_at_unix_seconds: Some(123),
            campaign_run_nonce: Some(456),
        };
        for pre_command in [false, true] {
            let mut frame = MissionFrame::new(0);
            if pre_command {
                frame.stage_commands().push(command.clone());
            } else {
                frame.stage_post_commands().push(command.clone());
            }
            let before = (
                bitcode::encode(frame.commands()),
                bitcode::encode(frame.post_commands()),
            );
            let current = if pre_command { 42 } else { 41 };
            let previous = stage_terminal_campaign_update(
                true,
                &host.transport,
                &mut frame,
                GameCode::LevelFailed,
                Default::default(),
                current,
            );
            assert_eq!(previous, 41);
            assert_eq!(
                (
                    bitcode::encode(frame.commands()),
                    bitcode::encode(frame.post_commands())
                ),
                before,
                "playback must retain exactly the recorded command, timestamp and nonce"
            );
            assert_eq!(
                terminal_campaign_update_applied(current, previous),
                pre_command
            );
            assert!(terminal_campaign_update_applied(42, previous));
        }
        let mut frame = MissionFrame::new(0);
        let previous = stage_terminal_campaign_update(
            true,
            &host.transport,
            &mut frame,
            GameCode::LevelFailed,
            Default::default(),
            41,
        );
        assert!(
            frame.post_commands().is_empty(),
            "await a later recorded multiplayer echo"
        );
        assert!(!terminal_campaign_update_applied(41, previous));
        assert!(terminal_campaign_update_applied(42, previous));
    }

    #[test]
    fn live_terminal_still_stages_one_campaign_update() {
        let host = Host::scratch(800.0, 600.0);
        let mut frame = MissionFrame::new(0);
        let previous = stage_terminal_campaign_update(
            false,
            &host.transport,
            &mut frame,
            GameCode::LevelFailed,
            Default::default(),
            41,
        );
        assert_eq!(previous, 41);
        assert_eq!(frame.post_commands().len(), 1);
        assert!(matches!(
            frame.post_commands()[0].command,
            PlayerCommand::ApplyQuitMissionUpdates {
                exit_code: GameCode::LevelFailed,
                ..
            }
        ));
    }

    #[test]
    fn mission_completion_clock_returns_consistent_epoch_units() {
        let (seconds, nanos) = mission_completion_clock();
        let seconds = seconds.expect("current clock must be representable as Unix seconds");
        let nanos = nanos.expect("current clock must be representable as Unix nanoseconds");
        assert!(seconds > 0);
        assert_eq!(nanos / 1_000_000_000, seconds as u64);
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

    #[test]
    fn terminal_text_table_follows_win_loss_outcome() {
        assert_eq!(debriefing_text_table_id(true, 100, 200), 100);
        assert_eq!(debriefing_text_table_id(false, 100, 200), 200);
    }

    #[test]
    fn leaderboard_outcome_is_defined_only_for_terminal_attempts() {
        assert_eq!(
            leaderboard_outcome(GameCode::LevelSucceeded),
            Some(crate::leaderboard_mission_end::MissionEndOutcome::Won)
        );
        assert_eq!(
            leaderboard_outcome(GameCode::LevelFailed),
            Some(crate::leaderboard_mission_end::MissionEndOutcome::Lost)
        );
        assert_eq!(
            leaderboard_outcome(GameCode::LevelInterrupted),
            Some(crate::leaderboard_mission_end::MissionEndOutcome::Interrupted)
        );
        assert_eq!(leaderboard_outcome(GameCode::LevelInProgress), None);
        assert_eq!(leaderboard_outcome(GameCode::LevelRestart), None);
    }

    #[test]
    fn debriefing_waits_for_the_terminal_campaign_command() {
        assert!(!terminal_campaign_update_applied(41, 41));
        assert!(!terminal_campaign_update_applied(40, 41));
        assert!(terminal_campaign_update_applied(42, 41));
    }
}
