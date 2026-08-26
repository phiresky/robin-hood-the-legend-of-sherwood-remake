//! Per-profile settings for optional post-port gameplay extensions.

use serde::{Deserialize, Serialize};

/// Gameplay extensions which intentionally differ from the original game.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct GameplayConfig {
    /// Use the intended Hard-difficulty reaction-time multiplier instead of
    /// the Easy multiplier selected by the original game's copy-paste bug.
    #[serde(default = "default_true")]
    pub fix_hard_reaction_times: bool,

    /// Allow direct selection and command of Royalist soldier NPCs.
    ///
    /// This defaults off so existing profiles and original-parity sessions
    /// retain the shipped game's input behaviour until the player opts in.
    #[serde(default)]
    pub control_allied_soldiers: bool,
}

const fn default_true() -> bool {
    true
}

impl Default for GameplayConfig {
    fn default() -> Self {
        Self {
            fix_hard_reaction_times: true,
            control_allied_soldiers: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::GameplayConfig;

    #[test]
    fn hard_reaction_time_fix_is_the_default() {
        assert!(GameplayConfig::default().fix_hard_reaction_times);
    }

    #[test]
    fn profiles_without_the_setting_enable_the_fix() {
        let config: GameplayConfig = serde_json::from_str("{}").expect("gameplay config");
        assert!(config.fix_hard_reaction_times);
        assert!(!config.control_allied_soldiers);
    }
}
