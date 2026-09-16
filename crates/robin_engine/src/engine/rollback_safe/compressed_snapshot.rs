//! Compressed rollback checkpoints contain only engine state.
use super::{Engine, LevelAssets};
use crate::engine::PersistedEngineState;
use crate::snapshot_storage::CompressedSnapshotBytes;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct CompressedEngineSnapshot {
    state: CompressedSnapshotBytes,
}

impl CompressedEngineSnapshot {
    pub fn capture(engine: &Engine) -> Result<Self, String> {
        assert!(
            engine.inner.ai.think_call_stack.is_empty(),
            "checkpoint captured during AI execution"
        );
        engine.inner.scripts.spellforge.validate_snapshot()?;
        Ok(Self {
            state: CompressedSnapshotBytes::encode(&engine.inner)?,
        })
    }

    pub fn restore(&self, assets: &LevelAssets) -> Result<Engine, String> {
        // This type has the same native field order as EngineInner. No persisted
        // projection runs here: all encoded queues retain rollback semantics.
        let state: PersistedEngineState = self.state.decode()?;
        let engine =
            Engine::adopt_authoritative_snapshot(Engine::from_persisted_state(state), assets)
                .map_err(|error| error.to_string())?;
        Ok(engine)
    }

    pub fn stored_bytes(&self) -> usize {
        self.state.stored_bytes()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[test]
    fn compressed_rollback_preserves_runtime_state_and_continuation() {
        let mut assets = LevelAssets::default();
        let mut engine =
            Engine::new_for_test(640.0, 480.0, Default::default(), &mut assets).unwrap();
        engine
            .inner
            .ai
            .global
            .primary_target_multiplicity_scratch
            .insert(17, 3);
        engine
            .inner
            .ai
            .global
            .primary_target_multiplicity_initialized = true;
        engine
            .inner
            .control
            .original_impossible_action_done_deadlines
            .insert((1, 2), VecDeque::from([42]));
        engine
            .inner
            .feedback
            .pending_side_effects
            .pending_minimap_position = Some(crate::coordinates::ScreenPoint::new(12.0, 34.0));
        let compressed = CompressedEngineSnapshot::capture(&engine).unwrap();
        let mut restored = compressed.restore(&assets).unwrap();
        assert_eq!(
            restored.inner.ai.global.primary_target_multiplicity_scratch,
            engine.inner.ai.global.primary_target_multiplicity_scratch
        );
        assert!(
            restored
                .inner
                .ai
                .global
                .primary_target_multiplicity_initialized
        );
        assert_eq!(
            restored
                .inner
                .control
                .original_impossible_action_done_deadlines,
            engine
                .inner
                .control
                .original_impossible_action_done_deadlines
        );
        assert_eq!(
            restored
                .inner
                .feedback
                .pending_side_effects
                .pending_minimap_position,
            engine
                .inner
                .feedback
                .pending_side_effects
                .pending_minimap_position
        );
        assert_eq!(
            serde_json::to_value(&restored).unwrap(),
            serde_json::to_value(&engine).unwrap()
        );
        for _ in 0..10 {
            engine.advance_frame(&assets, Default::default()).unwrap();
            restored.advance_frame(&assets, Default::default()).unwrap();
            assert_eq!(
                crate::replay::state_hash(&restored),
                crate::replay::state_hash(&engine)
            );
        }
    }
}
