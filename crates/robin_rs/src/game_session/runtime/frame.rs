//! One host-frame transaction: input journals, execution admission, and recorder ownership.

use super::{TimelineFrame, TimelineTransition};
use robin_engine::player_command::{FrameCommands, PlayerCommand};
use robin_engine::replay::ReplayPlayer;
use serde::{Deserialize, Serialize};

/// Ephemeral state for one host-loop iteration.
///
/// Both mission drivers use this shell. The graphical driver additionally uses
/// its modal-dismissal queue while the frontend owns native process resources.
#[derive(Debug)]
pub(in crate::game_session) struct MissionFrame {
    pub(in crate::game_session) started_at_ms: u32,
    pub(super) commands: FrameCommands,
    pub(super) external_actions: Vec<robin_engine::engine::ExternalAction>,
    external_actions_applied: usize,
    pub(super) post_commands: FrameCommands,
    pub(super) post_external_actions: Vec<robin_engine::engine::ExternalAction>,
    pub(super) post_external_actions_applied: usize,
    pub(super) external_facts: robin_engine::engine::ExternalFacts,
    execution: FrameExecution,
    /// Recorded lockstep/history transition for a disk-replay host frame.
    pub(in crate::game_session) replay_timeline_transition: Option<TimelineTransition>,
    pub(in crate::game_session) modal_dismissals: Vec<PlayerCommand>,
    pub(in crate::game_session) replay_modal_dismissals:
        crate::game_session::modal_state::ReplayModalDismissals,
    pub(in crate::game_session) recorder_hash: Option<u64>,
    pub(super) timeline_before: Option<TimelineFrame>,
    pub(super) timeline_after: Option<TimelineFrame>,
    pub(super) replay_record_consumed: bool,
    pub(super) recorder_state: RecorderFrameState,
}

/// Copyable evidence, never a runnable transaction or a recorder ownership token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::game_session) struct MissionFrameSnapshot {
    pub(super) started_at_ms: u32,
    pub(super) input: robin_engine::engine::SimulationFrameInput,
    pub(super) modal_dismissals: Vec<PlayerCommand>,
    pub(super) recorder_hash: Option<u64>,
    pub(super) timeline_before: Option<TimelineFrame>,
    pub(super) timeline_after: Option<TimelineFrame>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_engine::engine::{ExternalAction, ExternalFacts, SoundBoundary};

    #[test]
    fn input_projections_preserve_journal_and_select_pending_pre_actions() {
        let action = |name: &str| ExternalAction::Native {
            name: name.to_owned(),
            args: vec![1, 2],
            this_actor: None,
        };
        let mut frame = MissionFrame::new(0);
        frame.record_applied_external_action(action("already applied"));
        frame.stage_external_actions().push(action("pending"));
        frame.stage_post_external_actions().push(action("post"));
        frame.commands.commands.push(
            PlayerCommand::RegisterPeasantName {
                name: "pre command".to_owned(),
            }
            .into(),
        );
        frame.post_commands.commands.push(
            PlayerCommand::RegisterPeasantName {
                name: "post command".to_owned(),
            }
            .into(),
        );
        frame.external_facts =
            ExternalFacts::new(Vec::new(), Some(SoundBoundary::live(Vec::new())));
        frame.execution.run_hourglass = false;
        frame.execution.simulation_body_allowed = false;

        let hourglass = frame.hourglass_input();
        let authoritative = frame.authoritative_input();
        let json = |value| serde_json::to_value(value).unwrap();
        assert_eq!(
            json(&hourglass.external_actions),
            json(&vec![action("pending")])
        );
        assert_eq!(
            json(&authoritative.external_actions),
            json(&frame.external_actions)
        );
        assert_eq!(authoritative.external_actions.len(), 2);
        assert_eq!(frame.external_actions_applied, 1);
        assert_eq!(
            json(&authoritative.post_external_actions),
            json(&frame.post_external_actions)
        );
        assert_eq!(authoritative.post_commands.len(), 1);
        assert_eq!(
            serde_json::to_value(authoritative.post_player_inputs()).unwrap(),
            serde_json::to_value(&frame.post_commands.commands).unwrap()
        );
        assert!(hourglass.post_external_actions.is_empty());
        assert!(hourglass.post_commands.is_empty());
        assert!(!hourglass.run_post_initialize);
        assert!(authoritative.run_post_initialize);
        for input in [&hourglass, &authoritative] {
            assert_eq!(
                serde_json::to_value(input.player_inputs()).unwrap(),
                serde_json::to_value(&frame.commands.commands).unwrap()
            );
            assert_eq!(
                serde_json::to_value(&input.external_facts).unwrap(),
                serde_json::to_value(&frame.external_facts).unwrap()
            );
            assert!(!input.run_hourglass);
            assert!(!input.simulation_body_allowed);
        }
        frame.execution.post_initialize = PostInitializeAdmission::Pending(false);
        assert!(!frame.authoritative_input().run_post_initialize);
    }
}

impl Serialize for MissionFrame {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        MissionFrameSnapshot {
            started_at_ms: self.started_at_ms,
            input: self.authoritative_input(),
            modal_dismissals: self.modal_dismissals.clone(),
            recorder_hash: self.recorder_hash,
            timeline_before: self.timeline_before,
            timeline_after: self.timeline_after,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for MissionFrame {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "live mission frame authority cannot be deserialized; decode MissionFrameSnapshot instead",
        ))
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct FrameExecution {
    run_hourglass: bool,
    simulation_body_allowed: bool,
    post_initialize: PostInitializeAdmission,
    simulation_admitted: bool,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
enum PostInitializeAdmission {
    Pending(bool),
    Running(bool),
    Completed(bool),
    /// Manual ticks execute both phases together and retain the input policy,
    /// rather than recording a separately observed deferred initialization.
    Inline(bool),
}

impl FrameExecution {
    fn assert_pre_simulation(&self) {
        assert!(
            !self.simulation_admitted
                && matches!(self.post_initialize, PostInitializeAdmission::Pending(_)),
            "frame execution policy changed after execution admission"
        );
    }

    fn post_initialize_requested_or_completed(&self) -> bool {
        match self.post_initialize {
            PostInitializeAdmission::Pending(value)
            | PostInitializeAdmission::Running(value)
            | PostInitializeAdmission::Completed(value)
            | PostInitializeAdmission::Inline(value) => value,
        }
    }
}

/// Whether this host iteration owns an open replay-recorder frame.
///
/// The token lives on [`MissionFrame`], so the command write and the eventual
/// `end_frame` cannot silently drift onto different iterations.  Only
/// [`TimelineRuntime`](super::TimelineRuntime) may change it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum RecorderFrameState {
    Inactive,
    Open,
    Finished,
}

// These adapters expose only a fresh batch, never the journal they append to.
// Legacy command producers may clear/reorder their batch without invalidating
// already-admitted input or the applied-action cursor. Admission occurs once,
// at the end of the producer's scope, including its early-return paths.
macro_rules! frame_append_batch {
    ($name:ident, $batch:ty, $append:expr) => {
        #[derive(Serialize)]
        pub(in crate::game_session) struct $name<'a> {
            #[serde(skip)]
            journal: &'a mut $batch,
            staged: $batch,
        }

        impl<'a> $name<'a> {
            pub(super) fn new(journal: &'a mut $batch) -> Self {
                Self {
                    journal,
                    staged: Default::default(),
                }
            }
        }

        impl<'de> Deserialize<'de> for $name<'_> {
            fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
                Err(serde::de::Error::custom(
                    "frame append authority cannot be deserialized",
                ))
            }
        }

        impl std::ops::Deref for $name<'_> {
            type Target = $batch;
            fn deref(&self) -> &Self::Target {
                &self.staged
            }
        }

        impl std::ops::DerefMut for $name<'_> {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.staged
            }
        }

        impl Drop for $name<'_> {
            fn drop(&mut self) {
                ($append)(self.journal, &mut self.staged);
            }
        }
    };
}

frame_append_batch!(
    FrameCommandBatch,
    FrameCommands,
    |journal: &mut FrameCommands, staged: &mut FrameCommands| {
        journal.commands.append(&mut staged.commands);
    }
);
frame_append_batch!(
    FrameActionBatch,
    Vec<robin_engine::engine::ExternalAction>,
    |journal: &mut Vec<robin_engine::engine::ExternalAction>,
     staged: &mut Vec<robin_engine::engine::ExternalAction>| {
        journal.append(staged);
    }
);

impl MissionFrame {
    /// Scheduling may suppress a recorded tick, never promote a suppressed tick.
    pub(in crate::game_session) fn restrict_hourglass(&mut self, allowed: bool) {
        self.execution.assert_pre_simulation();
        self.execution.run_hourglass &= allowed;
    }

    pub(in crate::game_session) fn host_controls_only(&mut self) {
        self.execution.assert_pre_simulation();
        self.execution.run_hourglass = false;
        self.execution.simulation_body_allowed = false;
        self.execution.post_initialize = PostInitializeAdmission::Pending(false);
    }

    /// Admit the simulation phase exactly once, separately from copying its data.
    pub(in crate::game_session) fn admit_simulation(&mut self) {
        self.execution.assert_pre_simulation();
        self.execution.simulation_admitted = true;
    }

    pub(in crate::game_session) fn admit_inline_transaction(&mut self) {
        self.admit_simulation();
        let requested = self.execution.post_initialize_requested_or_completed();
        self.execution.post_initialize = PostInitializeAdmission::Inline(requested);
    }

    /// A rewind or terminal frame can cross this boundary without simulation.
    pub(in crate::game_session) fn begin_post_initialize(&mut self) -> bool {
        let PostInitializeAdmission::Pending(requested) = self.execution.post_initialize else {
            panic!("post-initialize phase admitted more than once");
        };
        self.execution.post_initialize = PostInitializeAdmission::Running(requested);
        requested
    }

    pub(in crate::game_session) fn complete_post_initialize(&mut self, initialized: bool) {
        let PostInitializeAdmission::Running(requested) = self.execution.post_initialize else {
            panic!("post-initialize phase completed without admission or more than once");
        };
        assert!(
            requested || !initialized,
            "suppressed post-initialize phase reported initialization"
        );
        self.execution.post_initialize = PostInitializeAdmission::Completed(initialized);
    }

    pub(in crate::game_session) fn commands(&self) -> &[robin_engine::player_command::PlayerInput] {
        &self.commands.commands
    }

    pub(in crate::game_session) fn post_commands(
        &self,
    ) -> &[robin_engine::player_command::PlayerInput] {
        &self.post_commands.commands
    }

    pub(in crate::game_session) fn external_actions(
        &self,
    ) -> &[robin_engine::engine::ExternalAction] {
        &self.external_actions
    }

    pub(in crate::game_session) fn stage_commands(&mut self) -> FrameCommandBatch<'_> {
        FrameCommandBatch::new(&mut self.commands)
    }

    pub(in crate::game_session) fn stage_post_commands(&mut self) -> FrameCommandBatch<'_> {
        FrameCommandBatch::new(&mut self.post_commands)
    }

    pub(in crate::game_session) fn stage_external_actions(&mut self) -> FrameActionBatch<'_> {
        FrameActionBatch::new(&mut self.external_actions)
    }

    pub(in crate::game_session) fn stage_post_external_actions(&mut self) -> FrameActionBatch<'_> {
        FrameActionBatch::new(&mut self.post_external_actions)
    }

    /// Discard live pre-tick commands superseded by replay or a loaded state.
    pub(in crate::game_session) fn discard_commands(&mut self) {
        self.commands.commands.clear();
    }

    /// A terminal restore starts a new recording attempt, retaining scheduling
    /// policy but none of the replaced state's admitted effects or recorder token.
    pub(in crate::game_session) fn reset_after_terminal_restore(&mut self, hash: u64) {
        self.external_actions.clear();
        self.external_actions_applied = 0;
        self.post_commands.commands.clear();
        self.post_external_actions.clear();
        self.post_external_actions_applied = 0;
        self.external_facts = Default::default();
        self.modal_dismissals.clear();
        self.replay_modal_dismissals = Default::default();
        self.replay_timeline_transition = None;
        self.replay_record_consumed = false;
        self.recorder_state = RecorderFrameState::Inactive;
        self.recorder_hash = Some(hash);
    }

    pub(in crate::game_session) fn new(started_at_ms: u32) -> Self {
        Self {
            started_at_ms,
            commands: FrameCommands::new(),
            external_actions: Vec::new(),
            external_actions_applied: 0,
            post_commands: FrameCommands::new(),
            post_external_actions: Vec::new(),
            post_external_actions_applied: 0,
            external_facts: robin_engine::engine::ExternalFacts::default(),
            execution: FrameExecution {
                run_hourglass: true,
                simulation_body_allowed: true,
                post_initialize: PostInitializeAdmission::Pending(true),
                simulation_admitted: false,
            },
            replay_timeline_transition: None,
            modal_dismissals: Vec::new(),
            replay_modal_dismissals:
                crate::game_session::modal_state::ReplayModalDismissals::default(),
            recorder_hash: None,
            timeline_before: None,
            timeline_after: None,
            replay_record_consumed: false,
            recorder_state: RecorderFrameState::Inactive,
        }
    }

    pub(in crate::game_session) fn authoritative_input(
        &self,
    ) -> robin_engine::engine::SimulationFrameInput {
        self.pre_refresh_input(&self.external_actions)
            .with_post_external_actions(self.post_external_actions.clone())
            .with_post_commands(
                self.post_commands
                    .commands
                    .iter()
                    .cloned()
                    .map(robin_engine::engine::SimCommand::from)
                    .collect(),
            )
            .with_post_initialize(self.execution.post_initialize_requested_or_completed())
    }

    pub(super) fn bind_timeline(&mut self, before: TimelineFrame) {
        assert!(
            self.timeline_before.replace(before).is_none(),
            "mission frame bound to the timeline more than once"
        );
    }

    pub(super) fn rebind_timeline_after_discontinuity(&mut self, before: TimelineFrame) {
        self.timeline_before = Some(before);
        self.timeline_after = None;
    }

    pub(in crate::game_session) fn commit_timeline_after(&mut self, after: TimelineFrame) {
        if let Some(expected) = self.replay_timeline_transition {
            assert_eq!(
                expected.after, after,
                "replay host transaction produced the wrong lockstep frame"
            );
        }
        assert!(
            self.timeline_after.replace(after).is_none(),
            "mission frame timeline transition committed more than once"
        );
    }

    pub(in crate::game_session) fn timeline_transition(&self) -> TimelineTransition {
        TimelineTransition {
            before: self
                .timeline_before
                .expect("unbound mission frame cannot become a replay record"),
            after: self
                .timeline_after
                .expect("uncommitted mission frame cannot become a replay record"),
        }
    }

    /// Authoritative input for the pre-refresh simulation phase. Late host
    /// commands remain staged until the explicit post-initialize admission.
    pub(in crate::game_session) fn hourglass_input(
        &self,
    ) -> robin_engine::engine::SimulationFrameInput {
        self.pre_refresh_input(self.unapplied_external_actions())
    }

    fn pre_refresh_input(
        &self,
        external_actions: &[robin_engine::engine::ExternalAction],
    ) -> robin_engine::engine::SimulationFrameInput {
        robin_engine::engine::SimulationFrameInput::from_player_inputs(
            self.commands.commands.clone(),
        )
        .with_external_facts(self.external_facts.clone())
        .with_external_actions(external_actions.to_vec())
        .with_simulation_body_allowed(self.execution.simulation_body_allowed)
        .with_hourglass(self.execution.run_hourglass)
    }

    /// Replace the authoritative portion of this host frame with a recorded
    /// transaction. Presentation-only modal acknowledgements remain owned by
    /// the live/replay adapter around the transaction.
    pub(in crate::game_session) fn adopt_authoritative_input(
        &mut self,
        input: robin_engine::engine::SimulationFrameInput,
    ) {
        self.execution.assert_pre_simulation();
        self.commands.commands = input
            .commands
            .into_iter()
            .map(robin_engine::engine::SimCommand::into_player_input)
            .collect();
        self.post_commands.commands = input
            .post_commands
            .into_iter()
            .map(robin_engine::engine::SimCommand::into_player_input)
            .collect();
        self.external_actions = input.external_actions;
        self.external_actions_applied = 0;
        self.post_external_actions = input.post_external_actions;
        self.post_external_actions_applied = 0;
        self.external_facts = input.external_facts;
        self.execution.run_hourglass = input.run_hourglass;
        self.execution.simulation_body_allowed = input.simulation_body_allowed;
        self.execution.post_initialize =
            PostInitializeAdmission::Pending(input.run_post_initialize);
    }

    pub(in crate::game_session) fn record_applied_external_action(
        &mut self,
        action: robin_engine::engine::ExternalAction,
    ) {
        assert_eq!(
            self.external_actions_applied,
            self.external_actions.len(),
            "new synchronous console action recorded before replayed actions were applied",
        );
        self.external_actions_applied += 1;
        self.external_actions.push(action);
    }

    fn unapplied_external_actions(&self) -> &[robin_engine::engine::ExternalAction] {
        &self.external_actions[self.external_actions_applied..]
    }

    pub(super) fn record_applied_post_external_actions(
        &mut self,
        actions: Vec<robin_engine::engine::ExternalAction>,
    ) {
        if !actions.is_empty() {
            assert_eq!(
                self.post_external_actions_applied,
                self.post_external_actions.len(),
                "new synchronous RPC action recorded before replayed actions were applied",
            );
            self.post_external_actions_applied += actions.len();
            self.post_external_actions.extend(actions);
        }
    }

    pub(super) fn mark_post_external_actions_applied(&mut self) {
        self.post_external_actions_applied = self.post_external_actions.len();
    }

    pub(in crate::game_session) fn unapplied_post_external_actions(
        &self,
    ) -> &[robin_engine::engine::ExternalAction] {
        &self.post_external_actions[self.post_external_actions_applied..]
    }

    pub(in crate::game_session) fn has_recorded_input(&self) -> bool {
        self.replay_record_consumed
    }

    /// Adopt one complete replay frame, splitting presentation-only modal
    /// acknowledgements out of both command phases before engine admission.
    pub(in crate::game_session) fn inject_replay_input(&mut self, player: &mut ReplayPlayer) {
        self.execution.assert_pre_simulation();
        assert!(
            !self.replay_record_consumed,
            "replay input injected into the same mission frame more than once"
        );
        assert!(
            !player.is_finished(),
            "replay injection requested after the replay finished"
        );
        self.replay_record_consumed = true;
        let recorded = player.next_frame().clone();
        self.replay_timeline_transition = Some(TimelineTransition {
            before: TimelineFrame::from_wire(recorded.timeline_before),
            after: TimelineFrame::from_wire(recorded.timeline_after),
        });
        self.rebind_timeline_after_discontinuity(
            self.replay_timeline_transition
                .expect("recorded replay transition was just installed")
                .before,
        );
        self.replay_modal_dismissals.begin_replay_frame();
        for control in recorded.host_controls {
            match control {
                robin_engine::replay::ReplayHostControl::ModalDismiss { modal, result } => self
                    .replay_modal_dismissals
                    .push_back(PlayerCommand::ModalDismiss {
                        kind: modal,
                        result,
                    }),
            }
        }
        self.adopt_authoritative_input(recorded.input);
    }

    pub(in crate::game_session) fn timeline_advances(&self, live_default: bool) -> bool {
        let Some(transition) = self.replay_timeline_transition else {
            return live_default;
        };
        if transition.after == transition.before {
            return false;
        }
        assert_eq!(
            transition.after,
            transition.before.next(),
            "replay timeline transition must stay put or advance exactly once"
        );
        true
    }

    pub(in crate::game_session) fn assert_replay_timeline_before(&self, current: TimelineFrame) {
        if let Some(transition) = self.replay_timeline_transition {
            assert_eq!(
                current, transition.before,
                "replay host transaction admitted at the wrong lockstep frame"
            );
        }
    }

    pub(super) fn open_recording(&mut self) {
        assert_eq!(
            self.recorder_state,
            RecorderFrameState::Inactive,
            "recorder frame began more than once"
        );
        self.recorder_state = RecorderFrameState::Open;
    }

    pub(super) fn close_recording(&mut self) -> bool {
        match self.recorder_state {
            RecorderFrameState::Inactive => {
                self.recorder_state = RecorderFrameState::Finished;
                false
            }
            RecorderFrameState::Finished => panic!("recorder frame finalized more than once"),
            RecorderFrameState::Open => {
                self.recorder_state = RecorderFrameState::Finished;
                true
            }
        }
    }
}
