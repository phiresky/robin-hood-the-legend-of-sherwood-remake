//! Per-profile settings for optional post-port gameplay extensions.

use serde::{Deserialize, Serialize};

/// Gameplay extensions which intentionally differ from the original game.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub struct GameplayConfig {
    /// Allow direct selection and command of Royalist soldier NPCs.
    ///
    /// This defaults off so existing profiles and original-parity sessions
    /// retain the shipped game's input behaviour until the player opts in.
    #[serde(default)]
    pub control_allied_soldiers: bool,
}
