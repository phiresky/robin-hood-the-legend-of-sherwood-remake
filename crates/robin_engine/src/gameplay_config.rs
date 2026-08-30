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

    /// Include the live item-production forecast in the Sherwood report.
    /// This is presentation-only and may be disabled independently from the
    /// underlying production simulation.
    #[serde(default = "default_show_production_forecast")]
    pub show_production_forecast: bool,

    /// Allow PCs to put their shipped cape disguise back on.
    ///
    /// Missing means an existing/migrated profile and deliberately preserves
    /// Original behavior (one-way cape removal only). Fresh profiles use the
    /// `Default` value below and opt into the extension.
    #[serde(default)]
    pub reusable_cloaks: bool,

    /// Campaign-selection presentation. This affects visuals only; complete
    /// attempt details and completed-mission practice remain available in all
    /// modes.
    #[serde(default)]
    pub campaign_presentation: CampaignPresentationMode,
}

const fn default_show_production_forecast() -> bool {
    true
}

impl Default for GameplayConfig {
    fn default() -> Self {
        Self {
            fix_hard_reaction_times: true,
            control_tactical_units: false,
            enable_unbinding: true,
            show_production_forecast: default_show_production_forecast(),
            reusable_cloaks: true,
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
        assert!(GameplayConfig::default().show_production_forecast);
    }

    #[test]
    fn profiles_without_the_setting_retain_original_reaction_times() {
        let config: GameplayConfig = serde_json::from_str("{}").expect("gameplay config");
        assert!(!config.fix_hard_reaction_times);
        assert!(!config.control_tactical_units);
        assert!(config.enable_unbinding);
        assert!(config.show_production_forecast);
        assert!(!config.reusable_cloaks);
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

    #[test]
    fn production_forecast_toggle_round_trips_with_profile_config() {
        let config = GameplayConfig {
            show_production_forecast: false,
            ..GameplayConfig::default()
        };
        let json = serde_json::to_string(&config).expect("serialize gameplay config");
        let decoded: GameplayConfig =
            serde_json::from_str(&json).expect("deserialize gameplay config");
        assert!(!decoded.show_production_forecast);
    }

    #[test]
    fn fresh_profiles_enable_reusable_cloaks() {
        assert!(GameplayConfig::default().reusable_cloaks);
    }
}
