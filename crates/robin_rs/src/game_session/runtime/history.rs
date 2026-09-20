//! Reconstruction state; rewind storage and its checker always reset together.

use crate::rewind::RewindBuffer;
use crate::rollback_checker::RollbackChecker;
use robin_engine::engine::{Engine, SimulationFrameInput};
use serde::{Serialize, Serializer};

pub(in crate::game_session) struct ReconstructionHistory {
    pub(super) buffer: RewindBuffer,
    checker: Option<RollbackChecker>,
    capture_enabled: bool,
}

impl ReconstructionHistory {
    pub(super) fn new(
        buffer: RewindBuffer,
        checker: Option<RollbackChecker>,
        capture_enabled: bool,
    ) -> Self {
        assert!(
            capture_enabled || checker.is_none(),
            "rollback checker requires history capture"
        );
        Self {
            buffer,
            checker,
            capture_enabled,
        }
    }

    pub(super) fn begin_frame(&mut self, frame: u32, engine: &Engine) {
        if self.capture_enabled {
            self.buffer.begin_frame(frame, engine);
        }
    }

    pub(super) fn begin_seek_frame(&mut self, frame: u32, engine: &Engine) {
        if self.checker.is_some() {
            self.begin_frame(frame, engine);
        } else if self.capture_enabled {
            self.buffer.begin_seek_frame(frame, engine);
        }
    }

    /// Read-only view of retained reconstruction frames. Mutation stays behind
    /// the lifecycle methods so the checker resets with the buffer.
    pub(in crate::game_session) fn buffer(&self) -> &RewindBuffer {
        &self.buffer
    }

    pub(in crate::game_session) fn begin_rewind_session(&mut self) {
        self.buffer.begin_session();
    }

    pub(in crate::game_session) fn end_rewind_session(&mut self) {
        self.buffer.end_session();
    }

    pub(in crate::game_session) fn checkpoint_recent(&mut self, frame: u32, engine: &Engine) {
        if self.capture_enabled {
            self.buffer.checkpoint_recent(frame, engine);
        }
    }

    #[cfg(test)]
    pub(in crate::game_session) fn append_fixture(&mut self, input: SimulationFrameInput) {
        self.buffer.end_frame_input(input);
    }

    #[cfg(test)]
    pub(in crate::game_session) fn clear_recent_fixture(&mut self) {
        self.buffer.clear_recent_checkpoints();
    }

    #[cfg(test)]
    pub(in crate::game_session) fn reconstruct_fixture(
        &mut self,
        assets: &robin_engine::engine::LevelAssets,
        frame: u32,
    ) -> Option<Engine> {
        self.buffer.rewind_to(assets, frame)
    }

    pub(in crate::game_session) fn reset_checker(&mut self) {
        if let Some(checker) = self.checker.as_mut() {
            checker.reset();
        }
    }

    pub(super) fn adopt_snapshot(&mut self, frame: u32, engine: &Engine) {
        self.buffer = RewindBuffer::new();
        self.reset_checker();
        if !self.capture_enabled {
            return;
        }
        self.buffer.seed_initial_anchor(frame, engine);
        // Loading occurs after open_frame; reopen its capture against adopted state.
        self.buffer.begin_frame(frame, engine);
    }

    pub(super) fn finish_restore(&mut self, frame: u32) {
        self.reset_checker();
        self.buffer.truncate_recent_after(frame);
    }

    pub(in crate::game_session) fn commit_paused(
        &mut self,
        boundary: u32,
        input: SimulationFrameInput,
    ) {
        if !self.capture_enabled || boundary < self.buffer.next_record_frame() {
            return;
        }
        // Do not retain empty host refreshes while a menu remains open.
        if !input.run_hourglass
            && !input.run_post_initialize
            && input.external_facts.is_empty()
            && input.external_actions.is_empty()
            && input.commands.is_empty()
            && input.post_external_actions.is_empty()
            && input.post_commands.is_empty()
        {
            return;
        }
        self.buffer.end_paused_input(boundary, input);
    }

    pub(in crate::game_session) fn commit(&mut self, input: SimulationFrameInput, engine: &Engine) {
        if !self.capture_enabled {
            return;
        }
        self.buffer.end_frame_input(input);
        if let Some(checker) = self.checker.as_mut() {
            checker.check_after_commit(&self.buffer, engine);
        }
    }
}

impl Serialize for ReconstructionHistory {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("ReconstructionHistory", 3)?;
        state.serialize_field("capture_enabled", &self.capture_enabled)?;
        state.serialize_field("next_record_frame", &self.buffer.next_record_frame())?;
        state.serialize_field("has_checker", &self.checker.is_some())?;
        state.end()
    }
}

robin_util::deny_deserialize!(
    ReconstructionHistory,
    "reconstruction history is live timeline authority, not a saved game"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paused_inputs_survive_rewind_and_recent_checkpoint_restoration() {
        use robin_engine::player_command::PlayerCommand;
        use robin_engine::replay::state_hash;
        use robin_engine::sim_timeline::{
            RestorePolicy, replay_authoritative_frame, replay_paused_inputs,
        };
        let (mut engine, assets) = robin_engine::test_support::fresh_engine_sized(640.0, 480.0);
        let mut history = ReconstructionHistory::new(RewindBuffer::new(), None, true);
        history.begin_frame(0, &engine);
        let first = SimulationFrameInput::no_hourglass();
        engine.advance_frame(&assets, first.clone()).unwrap();
        history.commit(first, &engine);
        // Retain the state before paused inputs, including at an exact checkpoint.
        history.checkpoint_recent(1, &engine);
        let before_pause = state_hash(&engine);
        for on in [true, false, true] {
            history.begin_frame(1, &engine);
            let paused = SimulationFrameInput::no_hourglass()
                .with_post_commands(vec![PlayerCommand::SetGoldenEyeMode { on }.into()]);
            engine.advance_frame(&assets, paused.clone()).unwrap();
            history.commit_paused(1, paused);
        }
        assert_eq!(
            state_hash(&history.buffer.rewind_to(&assets, 1).unwrap()),
            before_pause
        );
        assert_eq!(
            state_hash(
                &history
                    .buffer
                    .restore_recent(&assets, 1, RestorePolicy::Exact)
                    .unwrap()
                    .engine
            ),
            before_pause
        );
        history.begin_frame(1, &engine);
        let next = SimulationFrameInput::default();
        engine.advance_frame(&assets, next.clone()).unwrap();
        history.commit(next, &engine);
        assert_eq!(
            state_hash(&history.buffer.rewind_to(&assets, 2).unwrap()),
            state_hash(&engine)
        );
        let mut recent = history
            .buffer
            .restore_recent(&assets, 0, RestorePolicy::Exact)
            .unwrap();
        for frame in 0..2 {
            replay_paused_inputs(
                &mut recent.engine,
                &assets,
                history.buffer.paused_inputs_for(frame),
            )
            .unwrap();
            let _output = replay_authoritative_frame(
                &mut recent,
                &assets,
                history.buffer.frame_for(frame).unwrap(),
            );
        }
        assert_eq!(state_hash(&recent.engine), state_hash(&engine));
    }

    #[test]
    fn snapshot_adoption_replaces_history_and_reopens_the_adopted_boundary() {
        let (engine, _assets) = robin_engine::test_support::fresh_engine_sized(640.0, 480.0);
        let mut history = ReconstructionHistory::new(RewindBuffer::new(), None, true);
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
        let history = ReconstructionHistory::new(RewindBuffer::new(), None, true);
        let json = serde_json::to_value(&history).unwrap();
        assert!(serde_json::from_value::<ReconstructionHistory>(json).is_err());
    }
}
