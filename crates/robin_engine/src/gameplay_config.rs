//! Per-profile settings for optional post-port gameplay extensions.

use serde::{Deserialize, Serialize};

const fn enabled_by_default() -> bool {
    true
}

/// Detection radius used by the original wasp victim-selection routine.
pub const CLASSIC_WASP_ACQUISITION_RADIUS: f32 = 50.0;
/// Rebalanced initial wasp acquisition radius. Chase, sting, and forget
/// distances deliberately retain their shipped values.
pub const REBALANCED_WASP_ACQUISITION_RADIUS: f32 = 75.0;
/// Radius of the deterministic ground-stone noise stimulus.
pub const STONE_DISTRACTION_RADIUS: f32 = 240.0;
/// Shipped base throw distance for Will Scarlet's stone.
pub const CLASSIC_STONE_BASE_THROW_RANGE: f32 = 200.0;
/// Rebalanced stone base distance, matching comparable throwables.
pub const REBALANCED_STONE_BASE_THROW_RANGE: f32 = 300.0;
/// Minimum blood-alcohol increment for newly ale-interested soldiers.
pub const REBALANCED_ALE_MIN_POTENCY: u16 = 20;

pub const fn effective_ale_potency(authored_beer: u16, is_vip: bool, reliable_ale: bool) -> u16 {
    if reliable_ale && !is_vip && authored_beer == 0 {
        REBALANCED_ALE_MIN_POTENCY
    } else {
        authored_beer
    }
}

/// Independently switchable deterministic item rules.
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
pub struct ItemGameplayConfig {
    #[serde(default)]
    pub apple_combat_interrupt: bool,
    #[serde(default)]
    pub wasp_reliable_acquisition: bool,
    #[serde(default)]
    pub stone_ground_distraction: bool,
    #[serde(default)]
    pub stone_longer_range: bool,
    #[serde(default)]
    pub net_selective_immunity: bool,
    #[serde(default)]
    pub ale_reliable_distraction: bool,
}

impl ItemGameplayConfig {
    pub const fn classic() -> Self {
        Self {
            apple_combat_interrupt: false,
            wasp_reliable_acquisition: false,
            stone_ground_distraction: false,
            stone_longer_range: false,
            net_selective_immunity: false,
            ale_reliable_distraction: false,
        }
    }

    pub const fn effective_for_original_parity(self, original_parity: bool) -> Self {
        if original_parity {
            Self::classic()
        } else {
            self
        }
    }
}

impl Default for ItemGameplayConfig {
    fn default() -> Self {
        Self {
            apple_combat_interrupt: true,
            wasp_reliable_acquisition: true,
            stone_ground_distraction: true,
            stone_longer_range: true,
            net_selective_immunity: true,
            ale_reliable_distraction: true,
        }
    }
}

/// Host-side targeting explanations; these never author simulation outcomes.
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
pub struct ItemPreviewConfig {
    #[serde(default)]
    pub apple_effect: bool,
    #[serde(default)]
    pub stone_direct_effect: bool,
    #[serde(default)]
    pub stone_distraction_area: bool,
    #[serde(default)]
    pub net_capture_area: bool,
    #[serde(default)]
    pub net_crumple_prediction: bool,
    #[serde(default)]
    pub ale_effect: bool,
    #[serde(default)]
    pub purse_effect: bool,
    #[serde(default)]
    pub wasp_area: bool,
}

impl ItemPreviewConfig {
    pub const fn classic() -> Self {
        Self {
            apple_effect: false,
            stone_direct_effect: false,
            stone_distraction_area: false,
            net_capture_area: false,
            net_crumple_prediction: false,
            ale_effect: false,
            purse_effect: false,
            wasp_area: false,
        }
    }

    pub const fn effective_for_original_parity(self, original_parity: bool) -> Self {
        if original_parity {
            Self::classic()
        } else {
            self
        }
    }
}

impl Default for ItemPreviewConfig {
    fn default() -> Self {
        Self {
            apple_effect: true,
            stone_direct_effect: true,
            stone_distraction_area: true,
            net_capture_area: true,
            net_crumple_prediction: true,
            ale_effect: true,
            purse_effect: true,
            wasp_area: true,
        }
    }
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

    /// Enable world-camera pan, pinch zoom, and inertial motion for touch
    /// input. Tap and drag emulation remains available when this is off.
    #[serde(default = "default_touch_camera_gestures")]
    pub touch_camera_gestures: bool,

    /// Include the live item-production forecast in the Sherwood report.
    /// This is presentation-only and may be disabled independently from the
    /// underlying production simulation.
    #[serde(default = "default_show_production_forecast")]
    pub show_production_forecast: bool,

    /// Enable authoritative Sherwood inventory sales and their trading UI.
    /// Missing fields deserialize as `false`, preserving pre-feature profiles
    /// without silently opting them into a new economy.
    #[serde(default)]
    pub sherwood_trading: bool,

    /// Allow PCs to put their shipped cape disguise back on.
    ///
    /// Missing means an existing/migrated profile and deliberately preserves
    /// Original behavior (one-way cape removal only). Fresh profiles use the
    /// `Default` value below and opt into the extension.
    #[serde(default)]
    pub reusable_cloaks: bool,

    /// Deterministic item rebalances. Missing data preserves shipped behavior.
    #[serde(default = "ItemGameplayConfig::classic")]
    pub item_gameplay: ItemGameplayConfig,

    /// Independently switchable, presentation-only item targeting aids.
    #[serde(default = "ItemPreviewConfig::classic")]
    pub item_previews: ItemPreviewConfig,

    /// Play the optional impact cue for a ground-stone distraction.
    #[serde(default)]
    pub noise_distraction_feedback: bool,

    /// Campaign-selection presentation. This affects visuals only; complete
    /// attempt details and completed-mission practice remain available in all
    /// modes.
    #[serde(default)]
    pub campaign_presentation: CampaignPresentationMode,

    /// Treat hostile deaths caused by other NPCs as failures of Clean Hands.
    /// Player/direct-control deaths always invalidate it; this optional rule
    /// is deterministic simulation state and therefore travels in commands.
    #[serde(default)]
    pub clean_hands_npc_kills_invalidate: bool,

    /// Independent achievement and detailed-XP presentation switches.
    #[serde(default)]
    pub show_detailed_xp: bool,
    #[serde(default)]
    pub show_speedrun_tracker: bool,
    #[serde(default)]
    pub show_clean_hands_tracker: bool,
    #[serde(default)]
    pub show_ghost_tracker: bool,
    #[serde(default)]
    pub show_pile_o_bones_tracker: bool,
    #[serde(default)]
    pub show_all_enemies_one_building_tracker: bool,

    /// Show named per-mission and aggregate achievement badges in campaign
    /// presentations. Calculation and storage remain active when hidden.
    #[serde(default)]
    pub show_achievement_badges: bool,

    /// Append exact calculated achievement conditions to mission debriefs.
    /// Calculation and storage remain active when hidden.
    #[serde(default)]
    pub show_achievement_debrief: bool,

    /// Keep three rotating recovery points during ordinary single-player
    /// missions. Autosave persistence is host-only and deliberately excluded
    /// from deterministic simulation state.
    #[serde(default = "enabled_by_default")]
    #[state_hash(skip)]
    pub autosave_enabled: bool,

    /// Enforce time limits authored by Rust JSON missions.
    #[serde(default = "enabled_by_default")]
    pub enable_timed_missions: bool,

    /// Advance authored day/night/fog schedules, including perception and
    /// ambience-filtered gameplay sound sources.
    #[serde(default = "enabled_by_default")]
    pub enable_dynamic_ambience: bool,

    /// Show mission/player provenance, relative age, and the expanded
    /// selected-save panel in save/load pickers. Disabling this is strictly a
    /// presentation choice: every native save still stores full provenance.
    #[serde(default = "enabled_by_default")]
    #[state_hash(skip)]
    pub detailed_save_metadata: bool,

    /// Enable authored and runtime diplomacy overrides. When disabled, every
    /// distinct valid allegiance is hostile, preserving legacy behavior.
    #[serde(default)]
    pub diplomacy: bool,
    /// Let hostile NPC soldiers perceive and fight one another. Turning this
    /// off leaves conflicts involving a player active.
    #[serde(default = "enabled_by_default")]
    pub npc_faction_wars: bool,
}

const fn default_touch_camera_gestures() -> bool {
    true
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
            autosave_enabled: true,
            detailed_save_metadata: true,
            sherwood_trading: true,
            touch_camera_gestures: true,
            show_production_forecast: default_show_production_forecast(),
            reusable_cloaks: true,
            item_gameplay: ItemGameplayConfig::default(),
            item_previews: ItemPreviewConfig::default(),
            noise_distraction_feedback: true,
            campaign_presentation: CampaignPresentationMode::ProgressTree,
            clean_hands_npc_kills_invalidate: false,
            show_detailed_xp: false,
            show_speedrun_tracker: false,
            show_clean_hands_tracker: false,
            show_ghost_tracker: false,
            show_pile_o_bones_tracker: false,
            show_all_enemies_one_building_tracker: false,
            show_achievement_badges: true,
            show_achievement_debrief: true,
            enable_timed_missions: true,
            enable_dynamic_ambience: true,
            diplomacy: true,
            npc_faction_wars: true,
        }
    }
}

impl GameplayConfig {
    /// Settings used when a profile predates the gameplay-config object.
    /// Existing extension-specific migration behavior is preserved, while
    /// Feature 16's gameplay and presentation additions remain opt-in for that
    /// migrated profile. Fresh profiles continue to use [`Default`].
    pub const fn migrated() -> Self {
        Self {
            fix_hard_reaction_times: false,
            control_tactical_units: false,
            enable_unbinding: true,
            autosave_enabled: true,
            detailed_save_metadata: true,
            sherwood_trading: true,
            touch_camera_gestures: true,
            show_production_forecast: true,
            reusable_cloaks: false,
            item_gameplay: ItemGameplayConfig::classic(),
            item_previews: ItemPreviewConfig::classic(),
            noise_distraction_feedback: false,
            campaign_presentation: CampaignPresentationMode::ProgressTree,
            clean_hands_npc_kills_invalidate: false,
            show_detailed_xp: false,
            show_speedrun_tracker: false,
            show_clean_hands_tracker: false,
            show_ghost_tracker: false,
            show_pile_o_bones_tracker: false,
            show_all_enemies_one_building_tracker: false,
            show_achievement_badges: true,
            show_achievement_debrief: true,
            enable_timed_missions: true,
            enable_dynamic_ambience: true,
            diplomacy: false,
            npc_faction_wars: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{GameplayConfig, ItemGameplayConfig, ItemPreviewConfig};

    #[test]
    fn hard_reaction_time_fix_is_the_default() {
        assert!(GameplayConfig::default().fix_hard_reaction_times);
        assert!(GameplayConfig::default().show_production_forecast);
        assert!(GameplayConfig::default().sherwood_trading);
    }

    #[test]
    fn profiles_without_the_setting_retain_original_reaction_times() {
        let config: GameplayConfig = serde_json::from_str("{}").expect("gameplay config");
        assert!(!config.fix_hard_reaction_times);
        assert!(!config.control_tactical_units);
        assert!(config.enable_unbinding);
        assert!(config.autosave_enabled);
        assert!(config.touch_camera_gestures);
        assert!(!config.clean_hands_npc_kills_invalidate);
        assert!(!config.show_detailed_xp);
        assert!(!config.show_achievement_badges);
        assert!(!config.show_achievement_debrief);
        assert!(config.show_production_forecast);
        assert!(!config.sherwood_trading);
        assert!(!config.reusable_cloaks);
        assert_eq!(config.item_gameplay, ItemGameplayConfig::classic());
        assert_eq!(config.item_previews, ItemPreviewConfig::classic());
        assert!(!config.noise_distraction_feedback);
        assert!(config.touch_camera_gestures);
        assert_eq!(
            config.campaign_presentation,
            super::CampaignPresentationMode::ProgressTree
        );
        assert!(config.enable_timed_missions);
        assert!(config.enable_dynamic_ambience);
        assert!(!config.diplomacy);
        assert!(config.npc_faction_wars);
    }

    #[test]
    fn fresh_profiles_enable_achievement_presentations() {
        let config = GameplayConfig::default();
        assert!(config.show_achievement_badges);
        assert!(config.show_achievement_debrief);
    }

    #[test]
    fn previous_allied_control_setting_name_remains_loadable() {
        let config: GameplayConfig = serde_json::from_str(r#"{"control_allied_soldiers":true}"#)
            .expect("legacy gameplay config");
        assert!(config.control_tactical_units);
    }

    #[test]
    fn autosave_is_independent_default_on_and_not_hashed() {
        use robin_util::state_hash::compute;

        let enabled = GameplayConfig::default();
        let disabled = GameplayConfig {
            autosave_enabled: false,
            ..enabled
        };
        assert!(enabled.autosave_enabled);
        assert!(!disabled.autosave_enabled);
        assert_eq!(compute(&enabled), compute(&disabled));

        let json = serde_json::to_string(&disabled).expect("serialize gameplay config");
        let decoded: GameplayConfig =
            serde_json::from_str(&json).expect("deserialize gameplay config");
        assert!(!decoded.autosave_enabled);
    }

    #[test]
    fn detailed_save_metadata_is_independent_default_on_and_not_hashed() {
        use robin_util::state_hash::compute;

        let enabled = GameplayConfig::default();
        let disabled = GameplayConfig {
            detailed_save_metadata: false,
            ..enabled
        };
        assert!(enabled.detailed_save_metadata);
        assert!(!disabled.detailed_save_metadata);
        assert_eq!(compute(&enabled), compute(&disabled));

        let json = serde_json::to_string(&disabled).expect("serialize gameplay config");
        let decoded: GameplayConfig =
            serde_json::from_str(&json).expect("deserialize gameplay config");
        assert!(!decoded.detailed_save_metadata);
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

    #[test]
    fn fresh_profiles_enable_each_item_rule_and_preview() {
        let config = GameplayConfig::default();
        assert!(config.item_gameplay.apple_combat_interrupt);
        assert!(config.item_gameplay.wasp_reliable_acquisition);
        assert!(config.item_gameplay.stone_ground_distraction);
        assert!(config.item_gameplay.stone_longer_range);
        assert!(config.item_gameplay.net_selective_immunity);
        assert!(config.item_gameplay.ale_reliable_distraction);
        assert!(config.item_previews.apple_effect);
        assert!(config.item_previews.stone_direct_effect);
        assert!(config.item_previews.stone_distraction_area);
        assert!(config.item_previews.net_capture_area);
        assert!(config.item_previews.net_crumple_prediction);
        assert!(config.item_previews.ale_effect);
        assert!(config.item_previews.purse_effect);
        assert!(config.item_previews.wasp_area);
        assert!(config.noise_distraction_feedback);
    }

    #[test]
    fn partial_item_objects_leave_unmentioned_rules_off() {
        let config: GameplayConfig = serde_json::from_str(
            r#"{"item_gameplay":{"apple_combat_interrupt":true},"item_previews":{"net_capture_area":true}}"#,
        )
        .expect("gameplay config");
        assert!(config.item_gameplay.apple_combat_interrupt);
        assert!(!config.item_gameplay.wasp_reliable_acquisition);
        assert!(!config.item_gameplay.stone_longer_range);
        assert!(config.item_previews.net_capture_area);
        assert!(!config.item_previews.net_crumple_prediction);
    }

    #[test]
    fn every_item_mechanic_can_be_disabled_without_disabling_a_sibling() {
        let enabled = ItemGameplayConfig::default();
        let variants = [
            ItemGameplayConfig {
                apple_combat_interrupt: false,
                ..enabled
            },
            ItemGameplayConfig {
                wasp_reliable_acquisition: false,
                ..enabled
            },
            ItemGameplayConfig {
                stone_ground_distraction: false,
                ..enabled
            },
            ItemGameplayConfig {
                stone_longer_range: false,
                ..enabled
            },
            ItemGameplayConfig {
                net_selective_immunity: false,
                ..enabled
            },
            ItemGameplayConfig {
                ale_reliable_distraction: false,
                ..enabled
            },
        ];
        for variant in variants {
            assert_eq!(
                [
                    variant.apple_combat_interrupt,
                    variant.wasp_reliable_acquisition,
                    variant.stone_ground_distraction,
                    variant.stone_longer_range,
                    variant.net_selective_immunity,
                    variant.ale_reliable_distraction,
                ]
                .into_iter()
                .filter(|enabled| *enabled)
                .count(),
                5
            );
        }
    }

    #[test]
    fn every_item_preview_can_be_disabled_without_disabling_a_sibling() {
        let enabled = ItemPreviewConfig::default();
        let variants = [
            ItemPreviewConfig {
                apple_effect: false,
                ..enabled
            },
            ItemPreviewConfig {
                stone_direct_effect: false,
                ..enabled
            },
            ItemPreviewConfig {
                stone_distraction_area: false,
                ..enabled
            },
            ItemPreviewConfig {
                net_capture_area: false,
                ..enabled
            },
            ItemPreviewConfig {
                net_crumple_prediction: false,
                ..enabled
            },
            ItemPreviewConfig {
                ale_effect: false,
                ..enabled
            },
            ItemPreviewConfig {
                purse_effect: false,
                ..enabled
            },
            ItemPreviewConfig {
                wasp_area: false,
                ..enabled
            },
        ];
        for variant in variants {
            assert_eq!(
                [
                    variant.apple_effect,
                    variant.stone_direct_effect,
                    variant.stone_distraction_area,
                    variant.net_capture_area,
                    variant.net_crumple_prediction,
                    variant.ale_effect,
                    variant.purse_effect,
                    variant.wasp_area,
                ]
                .into_iter()
                .filter(|enabled| *enabled)
                .count(),
                7
            );
        }
    }

    #[test]
    fn original_parity_resolves_item_settings_to_classic() {
        assert_eq!(
            ItemGameplayConfig::default().effective_for_original_parity(true),
            ItemGameplayConfig::classic()
        );
        assert_eq!(
            ItemPreviewConfig::default().effective_for_original_parity(true),
            ItemPreviewConfig::classic()
        );
    }

    #[test]
    fn sherwood_trading_toggle_round_trips_with_profile_config() {
        let config = GameplayConfig {
            sherwood_trading: false,
            ..GameplayConfig::default()
        };
        let json = serde_json::to_string(&config).expect("serialize gameplay config");
        let decoded: GameplayConfig =
            serde_json::from_str(&json).expect("deserialize gameplay config");
        assert!(!decoded.sherwood_trading);
    }
}
