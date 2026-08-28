//! Per-profile settings for optional post-port gameplay extensions.

use serde::{Deserialize, Serialize};

/// Gameplay extensions which intentionally differ from the original game.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct GameplayConfig {
    /// Use the intended Hard-difficulty reaction-time multiplier instead of
    /// the Easy multiplier selected by the original game's copy-paste bug.
    // A missing field identifies a profile written before this opt-in existed;
    // retain the original game's reaction-time behaviour for that profile.
    #[serde(default)]
    pub fix_hard_reaction_times: bool,

    /// Allow direct selection and command of Royalist soldier NPCs.
    ///
    /// This defaults off so existing profiles and original-parity sessions
    /// retain the shipped game's input behaviour until the player opts in.
    #[serde(default)]
    pub control_allied_soldiers: bool,

    /// Allow PCs to put their shipped cape disguise back on.
    ///
    /// Missing means an existing/migrated profile and deliberately preserves
    /// Original behavior (one-way cape removal only). Fresh profiles use the
    /// `Default` value below and opt into the extension.
    #[serde(default)]
    pub reusable_cloaks: bool,
}

impl Default for GameplayConfig {
    fn default() -> Self {
        Self {
            fix_hard_reaction_times: true,
            control_allied_soldiers: false,
            reusable_cloaks: true,
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
    fn profiles_without_the_setting_retain_original_reaction_times() {
        let config: GameplayConfig = serde_json::from_str("{}").expect("gameplay config");
        assert!(!config.fix_hard_reaction_times);
        assert!(!config.control_allied_soldiers);
        assert!(!config.reusable_cloaks);
    }

    #[test]
    fn fresh_profiles_enable_reusable_cloaks() {
        assert!(GameplayConfig::default().reusable_cloaks);
    }
}
