//! Deterministic gates which suspend part or all of simulation progress.

/// Engine-owned suspension state.
///
/// These values form one rollback aggregate because all three decide whether
/// a presented frame may advance simulation. Their declaration order matches
/// their former order on `EngineInner`; `StateHash` therefore emits exactly
/// the same byte sequence after the extraction.
///
/// Original provenance:
/// - `original-code/RHEngine.h:204` declares serialized `mbLockEngine`, and
///   `original-code/RHengine.cpp:3629` gates the post-counter tick on it.
/// - `original-code/RHEngine.h:415` declares serialized `mbFreezeAll`, while
///   `original-code/RHScript.cpp:5223-5226` exposes the script mutation.
/// - `original-code/RHScript.cpp:9380-9483` performs `FadeToBlack` as blocking
///   presentation-only `Flip()` loops. `fade_freeze_frames_remaining` is the
///   Rust host/sim split's deterministic representation of those blocked
///   simulation frames.
#[derive(
    Debug, Clone, Default, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash,
)]
pub(crate) struct SimulationGateState {
    /// Whether sequence/camera state locks the engine's post-counter work.
    lock_engine: bool,
    /// Whether actor updates are frozen by script or cheat state.
    freeze_all: bool,
    /// Presentation frames left in a blocking fade.
    ///
    /// The triggering tick presents the first frame, hence a `2 * speed` fade
    /// stores `2 * speed - 1` here.
    fade_freeze_frames_remaining: u32,
}

impl SimulationGateState {
    pub(super) fn engine_locked(&self) -> bool {
        self.lock_engine
    }

    pub(super) fn set_engine_locked(&mut self, locked: bool) {
        self.lock_engine = locked;
    }

    pub(super) fn actors_frozen(&self) -> bool {
        self.freeze_all
    }

    pub(super) fn set_actors_frozen(&mut self, frozen: bool) {
        self.freeze_all = frozen;
    }

    #[cfg(test)]
    pub(super) fn fade_freeze_frames_remaining(&self) -> u32 {
        self.fade_freeze_frames_remaining
    }

    pub(super) fn set_fade_freeze_frames_remaining(&mut self, frames: u32) {
        self.fade_freeze_frames_remaining = frames;
    }

    /// Consume one presentation-only fade frame without advancing simulation.
    pub(super) fn consume_fade_freeze_frame(&mut self) -> bool {
        if self.fade_freeze_frames_remaining == 0 {
            return false;
        }
        self.fade_freeze_frames_remaining -= 1;
        true
    }
}
