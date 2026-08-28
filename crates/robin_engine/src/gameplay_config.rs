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

    /// Enable high-level commands for actors authored with the tactical
    /// command interface, regardless of archetype or allegiance.
    ///
    /// This defaults off so existing profiles and original-parity sessions
    /// retain the shipped game's input behaviour until the player opts in.
    #[serde(default, alias = "control_allied_soldiers")]
    pub control_tactical_units: bool,

    /// Include the live item-production forecast in the Sherwood report.
    /// This is presentation-only and may be disabled independently from the
    /// underlying production simulation.
    #[serde(default = "default_show_production_forecast")]
    pub show_production_forecast: bool,
}

const fn default_show_production_forecast() -> bool {
    true
}

impl Default for GameplayConfig {
    fn default() -> Self {
        Self {
            fix_hard_reaction_times: true,
            control_tactical_units: false,
            show_production_forecast: default_show_production_forecast(),
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
        assert!(config.show_production_forecast);
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
}
