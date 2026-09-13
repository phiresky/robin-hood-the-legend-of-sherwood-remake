//! Save projections for AI owners.
//!
//! The runtime AI owners (`AiController`, `AiGlobalState`,
//! `QueuedSelfStimulus`, `Stimulus`, `AiOutbox`, `AiDetectionOutbox`,
//! `AiReentrantOutbox`, `EnemyAi`, `FriendlyAi`) derive `Serialize` /
//! `Deserialize` directly. Runtime-only scratch fields carry `#[serde(skip)]`
//! and decode to their type default; each of them is also
//! `#[state_hash(skip)]` (or excluded by a manual `StateHash` impl), so the
//! derived hash byte stream is unaffected.
//!
//! The persisted engine state is additionally reconstructed in memory without
//! running a codec (`Engine::capture_persisted_state` /
//! `from_persisted_state`). [`PersistedProjection::persisted_clone`] yields
//! exactly what a serde round trip yields: a clone whose `#[serde(skip)]`
//! fields are reset. Raw runtime `Clone` remains rollback-exact.
//!
//! When adding a field to one of these owners, decide whether it persists; a
//! skipped field must also be reset in `clear_runtime_only_state` below. The
//! exhaustive-destructure guards in `tests/field_guards.rs` fail to compile
//! until a new field is classified, and the golden/projection tests compare
//! `persisted_clone` against a serde round trip.

use super::*;
use crate::ai_enemy::EnemyAi;
use crate::ai_friendly::FriendlyAi;

#[cfg(test)]
mod tests;

/// Owners whose save projection drops runtime-only scratch state.
pub(crate) trait PersistedProjection: Clone {
    /// Reset every `#[serde(skip)]` field (recursing into nested owners) to
    /// the value serde's derive decodes for it.
    fn clear_runtime_only_state(&mut self);

    /// Clone as a save projection; equal to a serde round trip of `self`.
    fn persisted_clone(&self) -> Self {
        let mut value = self.clone();
        value.clear_runtime_only_state();
        value
    }
}

impl PersistedProjection for AiController {
    fn clear_runtime_only_state(&mut self) {
        self.open_end_think_frames = Default::default();
        self.engine_deferred_end_think_frames = Default::default();
        self.engine_completion_verdict_resolved = Default::default();
        self.stimulus_queue
            .iter_mut()
            .for_each(Stimulus::clear_runtime_only_state);
        self.outbox.clear_runtime_only_state();
    }
}

impl PersistedProjection for AiGlobalState {
    fn clear_runtime_only_state(&mut self) {
        self.primary_target_multiplicity_scratch = Default::default();
        self.primary_target_multiplicity_initialized = Default::default();
    }
}

impl PersistedProjection for QueuedSelfStimulus {
    fn clear_runtime_only_state(&mut self) {
        self.origin = Default::default();
    }
}

impl PersistedProjection for Stimulus {
    fn clear_runtime_only_state(&mut self) {
        self.self_origin = Default::default();
    }
}

impl PersistedProjection for AiOutbox {
    fn clear_runtime_only_state(&mut self) {
        self.detection.clear_runtime_only_state();
        self.reentrant.clear_runtime_only_state();
    }
}

impl PersistedProjection for AiDetectionOutbox {
    fn clear_runtime_only_state(&mut self) {
        self.stimuli
            .iter_mut()
            .for_each(Stimulus::clear_runtime_only_state);
    }
}

impl PersistedProjection for AiReentrantOutbox {
    fn clear_runtime_only_state(&mut self) {
        self.engine_drains_after_script_go_on = Default::default();
        self.self_stimuli
            .iter_mut()
            .for_each(QueuedSelfStimulus::clear_runtime_only_state);
    }
}

impl PersistedProjection for EnemyAi {
    fn clear_runtime_only_state(&mut self) {
        self.base.clear_runtime_only_state();
        self.last_stimulus_dispatched_to_patrol
            .iter_mut()
            .for_each(Stimulus::clear_runtime_only_state);
    }
}

impl PersistedProjection for FriendlyAi {
    fn clear_runtime_only_state(&mut self) {
        self.base.clear_runtime_only_state();
    }
}
