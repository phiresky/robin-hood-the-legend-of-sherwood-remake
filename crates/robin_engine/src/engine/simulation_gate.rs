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
pub struct SimulationGateState {
    /// Whether sequence/camera state locks the engine's post-counter work.
    pub(crate) lock_engine: bool,
    /// Whether actor updates are frozen by script or cheat state.
    pub(crate) freeze_all: bool,
    /// Presentation frames left in a blocking fade.
    ///
    /// The triggering tick presents the first frame, hence a `2 * speed` fade
    /// stores `2 * speed - 1` here.
    #[serde(default)]
    pub(crate) fade_freeze_frames_remaining: u32,
}
