//! Reconstruction state; rewind storage and its checker always reset together.

use crate::rewind::RewindBuffer;
use crate::rollback_checker::RollbackChecker;
use robin_engine::engine::{Engine, SimulationFrameInput};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub(super) struct ReconstructionHistory {
    pub(super) buffer: RewindBuffer,
    checker: Option<RollbackChecker>,
}

impl ReconstructionHistory {
    pub(super) fn new(buffer: RewindBuffer, checker: Option<RollbackChecker>) -> Self {
        Self { buffer, checker }
    }

    pub(super) fn reset_checker(&mut self) {
        if let Some(checker) = self.checker.as_mut() {
            checker.reset();
        }
    }

    pub(super) fn adopt_snapshot(&mut self, frame: u32, engine: &Engine) {
        self.buffer = RewindBuffer::new();
        self.buffer.seed_initial_anchor(frame, engine);
        self.reset_checker();
        // Loading occurs after open_frame; reopen its capture against adopted state.
        self.buffer.begin_frame(frame, engine);
    }

    pub(super) fn finish_restore(&mut self, frame: u32) {
        self.reset_checker();
        self.buffer.truncate_recent_after(frame);
    }

    pub(super) fn commit(&mut self, input: SimulationFrameInput, engine: &Engine) {
        self.buffer.end_frame_input(input);
        if let Some(checker) = self.checker.as_mut() {
            checker.check_after_commit(&self.buffer, engine);
        }
    }
}

impl Serialize for ReconstructionHistory {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("ReconstructionHistory", 2)?;
        state.serialize_field("next_record_frame", &self.buffer.next_record_frame())?;
        state.serialize_field("has_checker", &self.checker.is_some())?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for ReconstructionHistory {
    fn deserialize<D: Deserializer<'de>>(_deserializer: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "reconstruction history is live timeline authority, not a saved game",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_engine::engine::LevelAssets;

    #[test]
    fn snapshot_adoption_replaces_history_and_reopens_the_adopted_boundary() {
        let mut assets = LevelAssets::default();
        let engine = Engine::new_for_test(640.0, 480.0, Default::default(), &mut assets).unwrap();
        let mut history = ReconstructionHistory::new(RewindBuffer::new(), None);
        history.buffer.begin_frame(0, &engine);
        history
            .buffer
            .end_frame_input(SimulationFrameInput::default());
        history.adopt_snapshot(9, &engine);
        assert!(history.buffer.frame_for(0).is_none());
        assert_eq!(history.buffer.oldest_reachable_frame(), Some(9));
        history
            .buffer
            .end_frame_input(SimulationFrameInput::default());
        assert_eq!(history.buffer.next_record_frame(), 10);
        assert!(history.buffer.frame_for(9).is_some());
    }

    #[test]
    fn diagnostic_serialization_cannot_restore_live_reconstruction_authority() {
        let history = ReconstructionHistory::new(RewindBuffer::new(), None);
        let json = serde_json::to_value(&history).unwrap();
        assert!(serde_json::from_value::<ReconstructionHistory>(json).is_err());
    }
}
