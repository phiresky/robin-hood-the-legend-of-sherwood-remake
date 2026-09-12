//! Network inputs and hash samples belonging to one prediction generation.

use super::TimelineFrame;
use robin_engine::player_command::PlayerInput;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Default, Serialize)]
pub(in crate::game_session) struct NetworkReconciliation {
    pending_inputs: BTreeMap<TimelineFrame, Vec<PlayerInput>>,
    peer_hashes: BTreeMap<u32, u64>,
    local_hashes: BTreeMap<u32, u64>,
}

impl NetworkReconciliation {
    pub(in crate::game_session) fn queue_input(
        &mut self,
        frame: TimelineFrame,
        input: PlayerInput,
    ) {
        self.pending_inputs.entry(frame).or_default().push(input);
    }

    pub(in crate::game_session) fn take_inputs(
        &mut self,
        frame: TimelineFrame,
    ) -> Vec<PlayerInput> {
        self.pending_inputs.remove(&frame).unwrap_or_default()
    }

    pub(in crate::game_session) fn pending_frame_count(&self) -> usize {
        self.pending_inputs.len()
    }

    pub(in crate::game_session) fn discard_inputs_before(&mut self, frame: TimelineFrame) {
        self.pending_inputs.retain(|&queued, _| queued >= frame);
    }

    pub(in crate::game_session) fn discard_pending_inputs(&mut self) {
        self.pending_inputs.clear();
    }

    /// Reconnect/resynchronization abandons every sample derived from the old state.
    pub(in crate::game_session) fn abandon_prediction(&mut self) {
        self.pending_inputs.clear();
        self.peer_hashes.clear();
        self.local_hashes.clear();
    }

    /// Initial snapshot adoption preserves already received future wire events;
    /// replacement snapshots discard the abandoned stream completely.
    pub(in crate::game_session) fn adopt_snapshot(
        &mut self,
        frame: TimelineFrame,
        replacement: bool,
    ) {
        if replacement {
            self.abandon_prediction();
        } else {
            self.discard_inputs_before(frame);
            self.peer_hashes.retain(|&f, _| f >= frame.number());
            self.local_hashes.clear();
        }
    }

    pub(in crate::game_session) fn admit_remote_hash(&mut self, frame: u32, hash: u64) {
        self.peer_hashes.insert(frame, hash);
    }

    pub(super) fn remember_local_hash(&mut self, frame: u32, hash: u64) {
        // Preserve the first pre-tick sample across repeated paused presentations.
        self.local_hashes.entry(frame).or_insert(hash);
        while self.local_hashes.len() > 256 {
            self.local_hashes.pop_first();
        }
    }

    pub(super) fn has_local_hash(&self, frame: u32) -> bool {
        self.local_hashes.contains_key(&frame)
    }

    pub(super) fn invalidate_after(&mut self, frame: u32) {
        // Input F changes post-F state; the pre-F sample remains authoritative.
        self.local_hashes.retain(|&f, _| f <= frame);
    }

    pub(super) fn clear_local_hashes(&mut self) {
        self.local_hashes.clear();
    }

    pub(super) fn clear_hashes(&mut self) {
        self.local_hashes.clear();
        self.peer_hashes.clear();
    }

    pub(in crate::game_session) fn take_due_comparisons(
        &mut self,
        current: TimelineFrame,
    ) -> Vec<(u32, u64, Option<u64>)> {
        let mut comparisons = Vec::new();
        while self
            .peer_hashes
            .first_key_value()
            .is_some_and(|(&frame, _)| frame <= current.number())
        {
            let (frame, remote) = self.peer_hashes.pop_first().expect("due hash exists");
            comparisons.push((frame, remote, self.local_hashes.get(&frame).copied()));
        }
        comparisons
    }
}

robin_util::deny_deserialize!(
    NetworkReconciliation,
    "network reconciliation is live prediction authority, not a saved game"
);

#[cfg(test)]
mod tests {
    use super::*;
    use robin_engine::player_command::PlayerCommand;

    #[test]
    fn diagnostic_serialization_cannot_restore_prediction_authority() {
        let json = serde_json::to_value(NetworkReconciliation::default()).unwrap();
        assert!(serde_json::from_value::<NetworkReconciliation>(json).is_err());
    }

    #[test]
    fn initial_adoption_retains_future_wire_events_but_replacement_discards_generation() {
        let mut network = NetworkReconciliation::default();
        for frame in [3, 7] {
            network.queue_input(
                TimelineFrame::from_wire(frame),
                PlayerCommand::QuitMissionRequested.into(),
            );
            network.admit_remote_hash(frame, u64::from(frame));
            network.remember_local_hash(frame, u64::from(frame));
        }
        network.adopt_snapshot(TimelineFrame::from_wire(5), false);
        assert!(network.take_inputs(TimelineFrame::from_wire(3)).is_empty());
        assert_eq!(network.pending_frame_count(), 1);
        assert_eq!(
            network.take_due_comparisons(TimelineFrame::from_wire(7)),
            vec![(7, 7, None)]
        );
        network.adopt_snapshot(TimelineFrame::from_wire(7), true);
        assert_eq!(network.pending_frame_count(), 0);
        assert!(
            network
                .take_due_comparisons(TimelineFrame::from_wire(100))
                .is_empty()
        );
    }

    #[test]
    fn delayed_hashes_keep_exact_boundary_and_rollback_preserves_pre_input_sample() {
        let mut network = NetworkReconciliation::default();
        for frame in [25, 50] {
            network.remember_local_hash(frame, u64::from(frame));
            network.admit_remote_hash(frame, u64::from(frame));
        }
        network.remember_local_hash(25, 999);
        network.invalidate_after(25);
        assert_eq!(
            network.take_due_comparisons(TimelineFrame::from_wire(75)),
            vec![(25, 25, Some(25)), (50, 50, None)]
        );
        assert!(
            network
                .take_due_comparisons(TimelineFrame::from_wire(75))
                .is_empty()
        );
    }
}
