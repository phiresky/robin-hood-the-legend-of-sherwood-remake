//! Per-profile settings for optional post-port gameplay extensions.

use serde::{Deserialize, Serialize};

const fn enabled_by_default() -> bool {
    true
}

#[repr(u8)]
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
pub enum CampaignPresentationMode {
    ClassicMap = 0,
    ProgressTree = 1,
    SherwoodMuseum = 2,
}

impl CampaignPresentationMode {
    pub const fn next(self) -> Self {
        match self {
            Self::ClassicMap => Self::ProgressTree,
            Self::ProgressTree => Self::SherwoodMuseum,
            Self::SherwoodMuseum => Self::ClassicMap,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::ClassicMap => "Classic map",
            Self::ProgressTree => "Progress tree",
            Self::SherwoodMuseum => "Sherwood museum",
        }
    }
}

impl Default for CampaignPresentationMode {
    fn default() -> Self {
        Self::ProgressTree
    }
}

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

    /// Enable high-level commands for actors authored with the tactical
    /// command interface, regardless of archetype or allegiance.
    ///
    /// This defaults off so existing profiles and original-parity sessions
    /// retain the shipped game's input behaviour until the player opts in.
    #[serde(default, alias = "control_allied_soldiers")]
    pub control_tactical_units: bool,

    /// Allow a PC with the Tie contextual action to release a tied NPC.
    ///
    /// The original shipped an unused `RHCOMMAND_UNTIE` slot but exposed no
    /// playable interaction. This post-port extension defaults on; disabling
    /// it restores the original input behavior.
    #[serde(default = "enabled_by_default")]
    pub enable_unbinding: bool,

    /// Campaign-selection presentation. This affects visuals only; complete
    /// attempt details and completed-mission practice remain available in all
    /// modes.
    #[serde(default)]
    pub campaign_presentation: CampaignPresentationMode,
}

impl Default for GameplayConfig {
    fn default() -> Self {
        Self {
            fix_hard_reaction_times: true,
            control_tactical_units: false,
            enable_unbinding: true,
            campaign_presentation: CampaignPresentationMode::ProgressTree,
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
        assert!(!config.control_tactical_units);
        assert!(config.enable_unbinding);
        assert_eq!(
            config.campaign_presentation,
            super::CampaignPresentationMode::ProgressTree
        );
    }

    #[test]
    fn previous_allied_control_setting_name_remains_loadable() {
        let config: GameplayConfig = serde_json::from_str(r#"{"control_allied_soldiers":true}"#)
            .expect("legacy gameplay config");
        assert!(config.control_tactical_units);
    }
}
