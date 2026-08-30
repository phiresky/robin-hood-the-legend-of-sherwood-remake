//! Per-profile settings for optional post-port gameplay extensions.

use serde::{Deserialize, Serialize};

const fn enabled_by_default() -> bool {
    true
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

    /// Enable world-camera pan, pinch zoom, and inertial motion for touch
    /// input. Tap and drag emulation remains available when this is off.
    #[serde(default = "default_touch_camera_gestures")]
    pub touch_camera_gestures: bool,
}

const fn default_touch_camera_gestures() -> bool {
    true
}

impl Default for GameplayConfig {
    fn default() -> Self {
        Self {
            fix_hard_reaction_times: true,
            control_tactical_units: false,
            enable_unbinding: true,
            touch_camera_gestures: true,
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
        assert!(config.touch_camera_gestures);
    }

    #[test]
    fn previous_allied_control_setting_name_remains_loadable() {
        let config: GameplayConfig = serde_json::from_str(r#"{"control_allied_soldiers":true}"#)
            .expect("legacy gameplay config");
        assert!(config.control_tactical_units);
    }
}
