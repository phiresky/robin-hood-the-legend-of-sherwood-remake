//! Presentation-independent modal ownership shared by mission drivers.
//!
//! A batch captures only the requests admitted when its lane becomes active.
//! Requests emitted later stay in the host queue: aborting an active batch
//! cannot retire a future batch merely because it has the same modal kind.

use robin_engine::player_command::{DialogResult, ModalKind, PlayerCommand};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ModalBatchState<T> {
    pending: VecDeque<T>,
    active: Option<ModalKind>,
}

impl<T> ModalBatchState<T> {
    pub(super) fn new(pending: VecDeque<T>) -> Self {
        Self {
            pending,
            active: None,
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.pending.is_empty() && self.active.is_none()
    }

    pub(super) fn current_kind(&self, kind: impl FnOnce(&T) -> ModalKind) -> Option<ModalKind> {
        self.active
            .clone()
            .or_else(|| self.pending.front().map(kind))
    }

    pub(super) fn start_next(&mut self, kind: impl FnOnce(&T) -> ModalKind) -> Option<T> {
        assert!(
            self.active.is_none(),
            "cannot replace an active modal before its outcome"
        );
        let item = self.pending.pop_front()?;
        self.active = Some(kind(&item));
        Some(item)
    }

    pub(super) fn has_active_item(&self) -> bool {
        self.active.is_some()
    }

    pub(super) fn finish(&mut self, kind: &ModalKind, result: DialogResult) {
        validate_modal_result(kind, result)
            .unwrap_or_else(|error| panic!("session modal result admission: {error}"));
        assert_eq!(
            self.active.as_ref(),
            Some(kind),
            "modal outcome does not own the active batch item"
        );
        self.active = None;
        if matches!(kind, ModalKind::Debriefing { .. }) && result == DialogResult::Aborted {
            self.pending.clear();
        }
    }

    pub(super) fn clear(&mut self) {
        self.pending.clear();
        self.active = None;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum ScriptedModalLane {
    Dialogue,
    Popup,
    SherwoodReport,
    Debriefing,
    LeaveMission,
}

impl ScriptedModalLane {
    pub(super) const ORDER: [Self; 5] = [
        Self::Dialogue,
        Self::Popup,
        Self::SherwoodReport,
        Self::Debriefing,
        Self::LeaveMission,
    ];
}

/// Presentation-free adapter over the same active-batch owner used by widgets.
/// Only the active lane is admitted per recorded host frame. Later engine
/// effects remain queued until this captured batch has retired.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(super) struct SessionModalScheduler {
    active: Option<ModalBatchState<ModalKind>>,
    checkpoints: BTreeMap<u32, ModalCheckpoint>,
    last_observed_ordinal: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ModalCheckpoint {
    active: Option<ModalBatchState<ModalKind>>,
    pending: Vec<ModalKind>,
    leave_prompt: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum ModalDecisionSource {
    Recorded,
    Automatic,
}

/// Terminal presentation, profile promotion and the load picker are adapters;
/// the valid recorded decision order is independent of those process owners.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct TerminalDecisionOrder {
    popup: ModalKind,
    final_page: ModalKind,
    stage: TerminalDecisionStage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum TerminalDecisionStage {
    MissionState,
    FinalDebriefing,
    Settled,
}

impl TerminalDecisionOrder {
    pub(super) fn new(won: bool, text_id: robin_engine::player_command::DebriefingTextId) -> Self {
        Self {
            popup: ModalKind::MissionState {
                kind: robin_engine::player_command::MissionStateModalKind::EndState { won },
            },
            final_page: ModalKind::FinalDebriefing { text_id },
            stage: TerminalDecisionStage::MissionState,
        }
    }

    pub(super) fn current_kind(&self) -> Option<ModalKind> {
        match self.stage {
            TerminalDecisionStage::MissionState => Some(self.popup.clone()),
            TerminalDecisionStage::FinalDebriefing => Some(self.final_page.clone()),
            TerminalDecisionStage::Settled => None,
        }
    }

    pub(super) fn accept(
        &mut self,
        kind: &ModalKind,
        result: DialogResult,
    ) -> Result<(), &'static str> {
        if self.current_kind().as_ref() != Some(kind) {
            return Err("terminal decision does not match the active session boundary");
        }
        self.stage = match self.stage {
            TerminalDecisionStage::MissionState => {
                if !matches!(result, DialogResult::Completed | DialogResult::Aborted) {
                    return Err("mission-state decision must be completed or aborted");
                }
                TerminalDecisionStage::FinalDebriefing
            }
            TerminalDecisionStage::FinalDebriefing => TerminalDecisionStage::Settled,
            TerminalDecisionStage::Settled => {
                unreachable!("settled terminal boundary has no current kind")
            }
        };
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum TerminalAdapter {
    Interactive,
    ReadOnlyReplay,
}

impl TerminalAdapter {
    pub(super) fn admit_campaign_transition(self) -> Result<(), &'static str> {
        match self {
            Self::Interactive => Ok(()),
            Self::ReadOnlyReplay => Err(
                "headless replay requires terminal campaign/profile promotion or load handling; this adapter is not implemented, so terminal replay cannot be reported as complete",
            ),
        }
    }
}

impl SessionModalScheduler {
    /// Sparse pre-record state: unchanged host records share the preceding
    /// checkpoint. Retain future entries during rewind so forward replay can
    /// revisit the same dense ordinal, including stationary transactions.
    pub(super) fn checkpoint(&mut self, ordinal: u32, effects: &crate::host::HostEffectBatches) {
        let state = ModalCheckpoint {
            active: self.active.clone(),
            pending: effects.pending_modal_kinds(),
            leave_prompt: effects.has_signal(crate::host::HostSignal::MissionStatePopup),
        };
        if self
            .checkpoints
            .range(..=ordinal)
            .next_back()
            .map(|(_, value)| value)
            != Some(&state)
        {
            self.checkpoints.insert(ordinal, state);
        }
        self.last_observed_ordinal = Some(
            self.last_observed_ordinal
                .map_or(ordinal, |last| last.max(ordinal)),
        );
    }

    pub(super) fn validate_restore(&self, ordinal: u32) -> Result<(), String> {
        if self
            .last_observed_ordinal
            .is_some_and(|last| ordinal <= last)
            && self.checkpoints.range(..=ordinal).next_back().is_some()
        {
            Ok(())
        } else {
            Err(format!(
                "no retained session modal checkpoint for replay ordinal {ordinal}"
            ))
        }
    }

    pub(super) fn restore(&mut self, ordinal: u32, effects: &mut crate::host::HostEffectBatches) {
        self.validate_restore(ordinal)
            .expect("modal seek must preflight before timeline mutation");
        let state = self
            .checkpoints
            .range(..=ordinal)
            .next_back()
            .expect("validated checkpoint")
            .1
            .clone();
        // Only modal lanes belong to this owner. Trade receipts, blits and
        // unrelated host signals must not be rewound with presentation state.
        while take_next_scripted_batch(effects, true).is_some() {}
        for kind in state.pending {
            match kind {
                ModalKind::Dialog { dialog_id } => effects.extend_dialogues([dialog_id]),
                ModalKind::PopupText { text_id } => effects.extend_popup_texts([text_id]),
                ModalKind::Debriefing { text_id } => effects.extend_debriefings([text_id]),
                ModalKind::SherwoodReport => effects.request_sherwood_report(),
                other => panic!("unexpected pending scripted modal checkpoint: {other:?}"),
            }
        }
        if state.leave_prompt {
            effects.request_signal(crate::host::HostSignal::MissionStatePopup);
        }
        self.active = state.active;
    }

    /// A successful save-load replaces Host effects through post_load_reset;
    /// an old captured batch is not part of the restored save payload.
    pub(super) fn after_load_back(&mut self) {
        self.active = None;
    }

    pub(super) fn advance(
        &mut self,
        effects: &mut crate::host::HostEffectBatches,
        replay: &mut ReplayModalDismissals,
        source: ModalDecisionSource,
    ) -> Option<PlayerCommand> {
        if source == ModalDecisionSource::Recorded && replay.iter().any(|command| matches!(command,
            PlayerCommand::ModalDismiss { kind: ModalKind::FinalDebriefing { .. }
                | ModalKind::MissionState { kind: robin_engine::player_command::MissionStateModalKind::EndState { .. } }, .. }
        )) {
            TerminalAdapter::ReadOnlyReplay.admit_campaign_transition()
                .unwrap_or_else(|error| panic!("{error}"));
        }
        if self.active.is_none() {
            self.active = take_next_scripted_batch(effects, true)
                .map(|(_, items)| ModalBatchState::new(items));
        }
        let batch = self.active.as_mut()?;
        if !batch.has_active_item() {
            batch.start_next(Clone::clone);
        }
        let kind = batch
            .current_kind(Clone::clone)
            .expect("admitted empty modal batch");
        let result = match source {
            ModalDecisionSource::Recorded => match replay.admit_screen(&kind) {
                ModalScreenAdmission::Recorded(result) => result,
                ModalScreenAdmission::AwaitRecorded | ModalScreenAdmission::Interactive => {
                    return None;
                }
            },
            ModalDecisionSource::Automatic => DialogResult::Completed,
        };
        batch.finish(&kind, result);
        if batch.is_empty() {
            self.active = None;
        }
        Some(PlayerCommand::ModalDismiss { kind, result })
    }

    #[cfg(test)]
    pub(super) fn is_active(&self) -> bool {
        self.active.is_some()
    }
}

pub(super) fn take_next_scripted_batch(
    effects: &mut crate::host::HostEffectBatches,
    include_leave_prompt: bool,
) -> Option<(ScriptedModalLane, VecDeque<ModalKind>)> {
    use crate::host::HostSignal;
    use robin_engine::player_command::{DebriefingTextId, MissionStateModalKind};
    for lane in ScriptedModalLane::ORDER {
        if lane == ScriptedModalLane::LeaveMission && !include_leave_prompt {
            continue;
        }
        let items: VecDeque<_> = match lane {
            ScriptedModalLane::Dialogue => effects
                .take_dialogues()
                .into_iter()
                .map(|dialog_id| ModalKind::Dialog { dialog_id })
                .collect(),
            ScriptedModalLane::Popup => effects
                .take_popup_texts()
                .into_iter()
                .map(|text_id| ModalKind::PopupText { text_id })
                .collect(),
            ScriptedModalLane::SherwoodReport => {
                if effects.take_sherwood_report() {
                    VecDeque::from([ModalKind::SherwoodReport])
                } else {
                    VecDeque::new()
                }
            }
            ScriptedModalLane::Debriefing => {
                let (lost, won): (Vec<_>, Vec<_>) = effects
                    .take_debriefings()
                    .into_iter()
                    .partition(|id| matches!(id, DebriefingTextId::Lose { .. }));
                lost.into_iter()
                    .chain(won)
                    .map(|text_id| ModalKind::Debriefing { text_id })
                    .collect()
            }
            ScriptedModalLane::LeaveMission => {
                if effects.take_signal(HostSignal::MissionStatePopup) {
                    VecDeque::from([ModalKind::MissionState {
                        kind: MissionStateModalKind::LeaveMissionNow,
                    }])
                } else {
                    VecDeque::new()
                }
            }
        };
        if !items.is_empty() {
            return Some((lane, items));
        }
    }
    None
}

/// Per-frame queue of typed replay modal results.
///
/// `strict_replay` distinguishes a replay-fed frame from ordinary live/modal
/// state. A missing matching result is valid: the modal remains open until a
/// later replay host frame supplies the dismissal. A supplied result which is
/// still present at frame finalization is instead a precise replay mismatch.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub(super) struct ReplayModalDismissals {
    queue: VecDeque<PlayerCommand>,
    #[serde(skip)]
    strict_replay: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum ModalScreenAdmission {
    Recorded(DialogResult),
    AwaitRecorded,
    Interactive,
}

impl ReplayModalDismissals {
    pub(super) fn admit_screen(&mut self, kind: &ModalKind) -> ModalScreenAdmission {
        if let Some(result) = pop_matching_dismissal(self, kind) {
            ModalScreenAdmission::Recorded(result)
        } else if self.strict_replay {
            ModalScreenAdmission::AwaitRecorded
        } else {
            ModalScreenAdmission::Interactive
        }
    }

    pub(super) fn begin_replay_frame(&mut self) {
        self.strict_replay = true;
    }

    pub(super) fn push_back(&mut self, command: PlayerCommand) {
        self.queue.push_back(command);
    }

    pub(super) fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub(super) fn len(&self) -> usize {
        self.queue.len()
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = &PlayerCommand> {
        self.queue.iter()
    }

    pub(super) fn assert_consumed(&self) {
        if self.strict_replay && !self.queue.is_empty() {
            panic!(
                "replay desync: {} recorded modal dismissal(s) were unused in their host frame: {:?}",
                self.queue.len(),
                self.queue
            );
        }
    }
}

impl From<VecDeque<PlayerCommand>> for ReplayModalDismissals {
    fn from(queue: VecDeque<PlayerCommand>) -> Self {
        Self {
            queue,
            strict_replay: false,
        }
    }
}

/// Pop the first `ModalDismiss` whose `kind` matches the target out of
/// the per-frame replay dismissal queue, returning the recorded result.
///
/// Matching by kind keeps the queue stable even if the engine queues
/// modals in a slightly different order within a frame (e.g. a dialog
/// and a popup both fired), and lets an unrelated modal without a
/// recording fall through to interactive handling. On playback, absence is
/// valid because a modal may stay open until a later host frame supplies its
/// recorded result.
pub(super) fn pop_matching_dismissal(
    queue: &mut ReplayModalDismissals,
    target: &ModalKind,
) -> Option<DialogResult> {
    let pos = queue.queue.iter().position(|c| {
        matches!(
            c,
            PlayerCommand::ModalDismiss { kind, .. }
                if kind == target
        )
    })?;
    match queue.queue.remove(pos)? {
        PlayerCommand::ModalDismiss { result, .. } => Some(result),
        _ => None,
    }
}

pub(super) fn validate_modal_result(
    kind: &robin_engine::player_command::ModalKind,
    result: robin_engine::player_command::DialogResult,
) -> Result<(), String> {
    use robin_engine::player_command::{DialogResult, ModalKind};

    let valid = match kind {
        ModalKind::Dialog { .. }
        | ModalKind::Debriefing { .. }
        | ModalKind::MissionState { .. } => {
            matches!(result, DialogResult::Completed | DialogResult::Aborted)
        }
        ModalKind::PopupText { .. } | ModalKind::SherwoodReport => {
            result == DialogResult::Completed
        }
        ModalKind::FinalDebriefing { .. } => true,
    };
    if valid {
        Ok(())
    } else {
        Err(format!(
            "modal {} cannot accept result {}",
            serde_json::to_string(kind).expect("ModalKind serializes"),
            serde_json::to_string(&result).expect("DialogResult serializes")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparse_checkpoint_restores_captured_and_new_same_id_batches() {
        let mut scheduler = SessionModalScheduler::default();
        let mut effects = crate::host::HostEffectBatches::default();
        let mut controls = ReplayModalDismissals::default();
        scheduler.checkpoint(0, &effects);
        for ordinal in 1..100 {
            scheduler.checkpoint(ordinal, &effects);
        }
        assert_eq!(scheduler.checkpoints.len(), 1);
        effects.extend_popup_texts([7, 8]);
        scheduler.advance(&mut effects, &mut controls, ModalDecisionSource::Recorded);
        effects.extend_popup_texts([7]);
        scheduler.checkpoint(100, &effects);
        assert_eq!(scheduler.checkpoints.len(), 2);
        scheduler.after_load_back();
        assert!(!scheduler.is_active());
        assert_eq!(
            effects.pending_modal_kinds(),
            [ModalKind::PopupText { text_id: 7 }]
        );
        scheduler.restore(100, &mut effects);
        for text_id in [7, 8, 7] {
            controls.begin_replay_frame();
            controls.push_back(PlayerCommand::ModalDismiss {
                kind: ModalKind::PopupText { text_id },
                result: DialogResult::Completed,
            });
            assert!(
                scheduler
                    .advance(&mut effects, &mut controls, ModalDecisionSource::Recorded)
                    .is_some()
            );
            controls.assert_consumed();
        }
        assert!(!scheduler.is_active());
        scheduler.restore(0, &mut effects);
        assert!(!scheduler.is_active());
        assert!(effects.pending_modal_kinds().is_empty());
        // Backward navigation does not discard the recorded future.
        scheduler.restore(100, &mut effects);
        assert!(scheduler.is_active());
        assert_eq!(effects.popup_text_count(), 1);
    }
    use robin_engine::player_command::DebriefingTextId;

    #[test]
    fn same_ordinal_decisions_consume_duplicate_and_distinct_items_exactly_once() {
        let mut effects = crate::host::HostEffectBatches::default();
        effects.extend_popup_texts([1, 1, 2]);
        let mut scheduler = SessionModalScheduler::default();
        let mut controls = ReplayModalDismissals::default();
        controls.begin_replay_frame();
        for text_id in [1, 1, 2] {
            controls.push_back(PlayerCommand::ModalDismiss {
                kind: ModalKind::PopupText { text_id },
                result: DialogResult::Completed,
            });
        }
        let mut acknowledged = Vec::new();
        while let Some(command) =
            scheduler.advance(&mut effects, &mut controls, ModalDecisionSource::Recorded)
        {
            acknowledged.push(command);
        }
        assert_eq!(acknowledged.len(), 3);
        controls.assert_consumed();
        assert!(!scheduler.is_active());
    }

    #[test]
    #[should_panic(expected = "recorded modal dismissal(s) were unused")]
    fn later_item_cannot_steal_an_earlier_items_boundary() {
        let mut effects = crate::host::HostEffectBatches::default();
        effects.extend_popup_texts([1, 2]);
        let mut scheduler = SessionModalScheduler::default();
        let mut later = recorded(ModalKind::PopupText { text_id: 2 }, DialogResult::Completed);
        assert!(
            scheduler
                .advance(&mut effects, &mut later, ModalDecisionSource::Recorded)
                .is_none()
        );
        assert_eq!(later.len(), 1);
        later.assert_consumed();
    }

    #[test]
    fn repeated_id_in_future_batch_remains_pending_after_active_abort() {
        let id = DebriefingTextId::Lose { index: 4 };
        let mut effects = crate::host::HostEffectBatches::default();
        effects.extend_debriefings([id, id]);
        let mut scheduler = SessionModalScheduler::default();
        scheduler.advance(
            &mut effects,
            &mut ReplayModalDismissals::default(),
            ModalDecisionSource::Recorded,
        );
        effects.extend_debriefings([id]);
        let mut abort = recorded(ModalKind::Debriefing { text_id: id }, DialogResult::Aborted);
        scheduler
            .advance(&mut effects, &mut abort, ModalDecisionSource::Recorded)
            .unwrap();
        assert_eq!(effects.debriefing_count(), 1);
        assert!(!scheduler.is_active());
        abort.assert_consumed();
    }

    #[test]
    fn terminal_decisions_require_popup_then_final_without_fabricated_completion() {
        let mut order = TerminalDecisionOrder::new(false, DebriefingTextId::Lose { index: 3 });
        let final_page = ModalKind::FinalDebriefing {
            text_id: DebriefingTextId::Lose { index: 3 },
        };
        assert!(order.accept(&final_page, DialogResult::Aborted).is_err());
        let popup = order.current_kind().unwrap();
        assert!(order.accept(&popup, DialogResult::Restart).is_err());
        assert_eq!(order.current_kind(), Some(popup.clone()));
        order.accept(&popup, DialogResult::Completed).unwrap();
        assert_eq!(order.current_kind(), Some(final_page.clone()));
        order.accept(&final_page, DialogResult::Aborted).unwrap();
        assert_eq!(order.current_kind(), None);
        assert!(order.accept(&final_page, DialogResult::Completed).is_err());
        assert!(
            TerminalAdapter::Interactive
                .admit_campaign_transition()
                .is_ok()
        );
        assert!(
            TerminalAdapter::ReadOnlyReplay
                .admit_campaign_transition()
                .unwrap_err()
                .contains("cannot be reported as complete")
        );
    }

    #[test]
    fn shared_admission_orders_simultaneous_lanes_and_lost_before_won() {
        let mut effects = crate::host::HostEffectBatches::default();
        let win = DebriefingTextId::Win { index: 2 };
        let lose = DebriefingTextId::Lose { index: 1 };
        effects.extend_debriefings([win, lose]);
        effects.request_sherwood_report();
        effects.extend_popup_texts([8]);
        effects.extend_dialogues([9]);
        assert_eq!(
            take_next_scripted_batch(&mut effects, false).unwrap().0,
            ScriptedModalLane::Dialogue
        );
        assert_eq!(
            take_next_scripted_batch(&mut effects, false).unwrap().0,
            ScriptedModalLane::Popup
        );
        assert_eq!(
            take_next_scripted_batch(&mut effects, false).unwrap().0,
            ScriptedModalLane::SherwoodReport
        );
        let (lane, items) = take_next_scripted_batch(&mut effects, false).unwrap();
        assert_eq!(lane, ScriptedModalLane::Debriefing);
        assert_eq!(
            items,
            VecDeque::from([
                ModalKind::Debriefing { text_id: lose },
                ModalKind::Debriefing { text_id: win }
            ])
        );
    }

    fn recorded(kind: ModalKind, result: DialogResult) -> ReplayModalDismissals {
        let mut frame = ReplayModalDismissals::default();
        frame.begin_replay_frame();
        frame.push_back(PlayerCommand::ModalDismiss { kind, result });
        frame
    }

    #[test]
    fn stationary_dismissal_retires_only_captured_batch_and_preserves_new_effects() {
        let mut effects = crate::host::HostEffectBatches::default();
        let first = DebriefingTextId::Lose { index: 1 };
        let sibling = DebriefingTextId::Lose { index: 2 };
        effects.extend_debriefings([first, sibling]);
        let mut scheduler = SessionModalScheduler::default();
        let mut creation = ReplayModalDismissals::default();
        creation.begin_replay_frame();
        assert!(
            scheduler
                .advance(&mut effects, &mut creation, ModalDecisionSource::Recorded)
                .is_none()
        );
        assert!(scheduler.is_active());
        assert_eq!(effects.debriefing_count(), 0, "batch has a distinct owner");
        effects.extend_debriefings([sibling]);
        let mut abort = recorded(
            ModalKind::Debriefing { text_id: first },
            DialogResult::Aborted,
        );
        assert!(
            scheduler
                .advance(&mut effects, &mut abort, ModalDecisionSource::Recorded)
                .is_some()
        );
        abort.assert_consumed();
        assert!(!scheduler.is_active());
        assert_eq!(
            effects.debriefing_count(),
            1,
            "new batch must survive active abort"
        );
        let mut next = recorded(
            ModalKind::Debriefing { text_id: sibling },
            DialogResult::Completed,
        );
        assert!(
            scheduler
                .advance(&mut effects, &mut next, ModalDecisionSource::Recorded)
                .is_some()
        );
        next.assert_consumed();
        assert!(!scheduler.is_active());
    }

    #[test]
    fn active_popup_precedes_new_dialogue_and_unrecorded_input_cannot_close_it() {
        let mut effects = crate::host::HostEffectBatches::default();
        effects.extend_popup_texts([4]);
        let mut scheduler = SessionModalScheduler::default();
        let mut empty = ReplayModalDismissals::default();
        assert!(
            scheduler
                .advance(&mut effects, &mut empty, ModalDecisionSource::Recorded)
                .is_none()
        );
        effects.extend_dialogues([5]);
        let mut popup = recorded(ModalKind::PopupText { text_id: 4 }, DialogResult::Completed);
        assert!(
            scheduler
                .advance(&mut effects, &mut popup, ModalDecisionSource::Recorded)
                .is_some()
        );
        assert_eq!(effects.dialogue_count(), 1);
        popup.assert_consumed();
    }

    #[test]
    #[should_panic(expected = "recorded modal dismissal(s) were unused")]
    fn a_recorded_control_cannot_acknowledge_an_unadmitted_lane() {
        let mut effects = crate::host::HostEffectBatches::default();
        effects.extend_dialogues([1]);
        effects.extend_popup_texts([2]);
        let mut scheduler = SessionModalScheduler::default();
        let mut popup = recorded(ModalKind::PopupText { text_id: 2 }, DialogResult::Completed);
        assert!(
            scheduler
                .advance(&mut effects, &mut popup, ModalDecisionSource::Recorded)
                .is_none()
        );
        popup.assert_consumed();
    }

    #[test]
    fn aborted_active_batch_does_not_retire_a_future_batch() {
        let kind = |index| ModalKind::Debriefing {
            text_id: DebriefingTextId::Lose { index },
        };
        let mut active = ModalBatchState::new(VecDeque::from([kind(1), kind(2)]));
        let mut future = ModalBatchState::new(VecDeque::from([kind(2)]));
        assert_eq!(active.start_next(Clone::clone), Some(kind(1)));
        active.finish(&kind(1), DialogResult::Aborted);
        assert!(active.is_empty());
        assert_eq!(future.start_next(Clone::clone), Some(kind(2)));
    }

    #[test]
    fn completed_item_retains_siblings_and_waiting_keeps_identity() {
        let first = ModalKind::PopupText { text_id: 1 };
        let second = ModalKind::PopupText { text_id: 2 };
        let mut batch = ModalBatchState::new(VecDeque::from([first.clone(), second.clone()]));
        assert_eq!(batch.start_next(Clone::clone), Some(first.clone()));
        assert_eq!(batch.current_kind(Clone::clone), Some(first.clone()));
        batch.finish(&first, DialogResult::Completed);
        assert_eq!(batch.start_next(Clone::clone), Some(second));
    }
}
