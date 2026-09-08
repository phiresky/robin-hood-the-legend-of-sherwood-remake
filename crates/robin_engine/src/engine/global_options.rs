//! Application-wide startup options (`GlobalOptions`).

use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::gameplay_config::ItemGameplayConfig;
use crate::player_profile::DifficultyLevel;
use robin_run_protocol::{
    RankedSimulationDifficultyV1, RankedSimulationPolicyV1, RankedSimulationPresetV1, Validate as _,
};

const fn enabled_by_default() -> bool {
    true
}

/// Immutable gameplay configuration copied out of application/profile state.
///
/// This is deliberately separate from [`GlobalOptions`]: filesystem paths,
/// audio switches, and host resources are application concerns, while these
/// values can change deterministic simulation results and must belong to one
/// game context.  The engine receives a copy before ticking and never reaches
/// back into the process-global player-profile manager.
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
pub struct SimConfig {
    pub difficulty: DifficultyLevel,
    /// Fix the original game's Hard-difficulty reaction-time copy-paste bug.
    // A missing field identifies deterministic state written before this
    // extension existed and therefore preserves the original game's bug.
    #[serde(default)]
    pub fix_hard_reaction_times: bool,
    /// Enable the post-port player interaction for releasing tied NPCs.
    #[serde(default = "enabled_by_default")]
    pub enable_unbinding: bool,
    /// Optional Clean Hands rule for deaths caused by non-player NPCs.
    #[serde(default)]
    pub clean_hands_npc_kills_invalidate: bool,
    /// Enable the deterministic reusable-cloak extension for this session.
    /// Missing state predates the extension and retains Original behavior.
    #[serde(default)]
    pub reusable_cloaks: bool,
    /// Opt-in animated terrain reversibility, resolved when loading the level.
    #[serde(default)]
    pub reversible_background_patches: bool,
    /// Deterministic item rules selected by the active profile.
    #[serde(default = "ItemGameplayConfig::classic")]
    pub item_gameplay: ItemGameplayConfig,
    /// Optional distraction impact cue. Kept in snapshot state so peers agree
    /// on the side-effect stream.
    #[serde(default)]
    pub noise_distraction_feedback: bool,
    /// Apply mission-authored diplomacy instead of the legacy distinct-ID
    /// hostility rule. Serialized because it affects simulation outcomes.
    #[serde(default)]
    pub diplomacy: bool,
    #[serde(default = "default_enabled")]
    pub npc_faction_wars: bool,
    /// Authoritative opt-out for the nine composite player sword techniques.
    pub more_combat_gestures: bool,
    /// Authoritative opt-out for gesture-quality damage scaling.
    pub gesture_quality_damage: bool,
    /// Authoritative shared player visibility. Original-parity construction
    /// normalizes this off; current native state must carry it explicitly.
    pub fog_of_war: bool,
    pub script_enabled: bool,
    pub highlander: bool,
    pub highlander2: bool,
    pub golden_eye: bool,
    pub ignore_default_loose: bool,
    pub bypass_fog_sprites_crash: bool,
    /// Active player-profile speech density. This affects authoritative
    /// chorus suppression and deterministic speech timing.
    pub amount_of_speaking: u16,
    /// Resolve A* requests inline with sequence translation. Used by the
    /// original-game parity harness so path-result timing is independent of
    /// worker/scheduler cadence.
    pub synchronous_pathfinding: bool,
    /// Authoritative switch for Sherwood inventory trading.  Missing fields in
    /// old deterministic state deserialize off; newly constructed contexts use
    /// the active profile's explicit default-on value.
    #[serde(default)]
    pub sherwood_trading: bool,
    /// Authoritative opt-out for Rust-authored mission time limits.
    #[serde(default = "default_enabled")]
    pub enable_timed_missions: bool,
    /// Authoritative opt-out for runtime ambience gameplay effects.
    #[serde(default = "default_enabled")]
    pub enable_dynamic_ambience: bool,
}

const fn default_enabled() -> bool {
    true
}

/// Exhaustive identity of one deterministic ranked configuration field.
/// `SimConfig::first_ranked_difference` destructures the complete config so a
/// later field cannot silently escape ranked-policy admission.
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
#[serde(rename_all = "snake_case")]
pub enum RankedSimulationConfigField {
    Difficulty,
    FixHardReactionTimes,
    EnableUnbinding,
    CleanHandsNpcKillsInvalidate,
    ReusableCloaks,
    ReversibleBackgroundPatches,
    ItemGameplay,
    NoiseDistractionFeedback,
    Diplomacy,
    NpcFactionWars,
    /// Runtime-authored diplomacy edges are deterministic policy state even
    /// though they do not live in the immutable `SimConfig` value itself.
    DiplomacyRelationshipGraph,
    MoreCombatGestures,
    GestureQualityDamage,
    /// The one runtime command atomically replaces both gesture-rule fields.
    CombatGestureRules,
    FogOfWar,
    ScriptEnabled,
    Highlander,
    Highlander2,
    GoldenEye,
    IgnoreDefaultLoose,
    BypassFogSpritesCrash,
    AmountOfSpeaking,
    SynchronousPathfinding,
    SherwoodTrading,
    EnableTimedMissions,
    EnableDynamicAmbience,
}

impl RankedSimulationConfigField {
    pub const fn config_field(self) -> &'static str {
        match self {
            Self::Difficulty => "sim_config.difficulty",
            Self::FixHardReactionTimes => "sim_config.fix_hard_reaction_times",
            Self::EnableUnbinding => "sim_config.enable_unbinding",
            Self::CleanHandsNpcKillsInvalidate => "sim_config.clean_hands_npc_kills_invalidate",
            Self::ReversibleBackgroundPatches => "sim_config.reversible_background_patches",
            Self::ReusableCloaks => "sim_config.reusable_cloaks",
            Self::ItemGameplay => "sim_config.item_gameplay",
            Self::NoiseDistractionFeedback => "sim_config.noise_distraction_feedback",
            Self::Diplomacy => "sim_config.diplomacy",
            Self::NpcFactionWars => "sim_config.npc_faction_wars",
            Self::DiplomacyRelationshipGraph => "diplomacy.relationship_graph",
            Self::MoreCombatGestures => "sim_config.more_combat_gestures",
            Self::GestureQualityDamage => "sim_config.gesture_quality_damage",
            Self::CombatGestureRules => "sim_config.combat_gesture_rules",
            Self::FogOfWar => "sim_config.fog_of_war",
            Self::ScriptEnabled => "sim_config.script_enabled",
            Self::Highlander => "sim_config.highlander",
            Self::Highlander2 => "sim_config.highlander2",
            Self::GoldenEye => "sim_config.golden_eye",
            Self::IgnoreDefaultLoose => "sim_config.ignore_default_loose",
            Self::BypassFogSpritesCrash => "sim_config.bypass_fog_sprites_crash",
            Self::AmountOfSpeaking => "sim_config.amount_of_speaking",
            Self::SynchronousPathfinding => "sim_config.synchronous_pathfinding",
            Self::SherwoodTrading => "sim_config.sherwood_trading",
            Self::EnableTimedMissions => "sim_config.enable_timed_missions",
            Self::EnableDynamicAmbience => "sim_config.enable_dynamic_ambience",
        }
    }
}

impl std::fmt::Display for RankedSimulationConfigField {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.config_field())
    }
}

/// Engine-owned immutable ranked-policy capability derived only from the
/// signed typed rules identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RankedSimulationPolicy {
    identity: RankedSimulationPolicyV1,
    expected_config: SimConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RankedSimulationPolicyError {
    #[error("unsupported ranked simulation-policy document: {0}")]
    InvalidIdentity(robin_run_protocol::ValidationError),
    #[error("ranked simulation policy config differs at {field}")]
    ConfigMismatch { field: RankedSimulationConfigField },
}

impl RankedSimulationPolicy {
    pub fn from_identity(
        identity: RankedSimulationPolicyV1,
    ) -> Result<Self, RankedSimulationPolicyError> {
        identity
            .validate()
            .map_err(RankedSimulationPolicyError::InvalidIdentity)?;
        let difficulty = match identity.difficulty {
            RankedSimulationDifficultyV1::Easy => DifficultyLevel::Easy,
            RankedSimulationDifficultyV1::Medium => DifficultyLevel::Medium,
            RankedSimulationDifficultyV1::Hard => DifficultyLevel::Hard,
        };
        let expected_config = match identity.preset {
            RankedSimulationPresetV1::Standard => SimConfig::standard_ranked(difficulty),
            RankedSimulationPresetV1::OriginalParity => {
                SimConfig::original_parity_ranked(difficulty)
            }
        };
        Ok(Self {
            identity,
            expected_config,
        })
    }

    pub fn standard(
        difficulty: RankedSimulationDifficultyV1,
    ) -> Result<Self, RankedSimulationPolicyError> {
        Self::from_identity(RankedSimulationPolicyV1::standard(difficulty))
    }

    pub fn original_parity(
        difficulty: RankedSimulationDifficultyV1,
    ) -> Result<Self, RankedSimulationPolicyError> {
        Self::from_identity(RankedSimulationPolicyV1::original_parity(difficulty))
    }

    pub fn standard_medium() -> Self {
        Self::standard(RankedSimulationDifficultyV1::Medium)
            .expect("the Standard/Medium ranked policy is a protocol constant")
    }

    pub fn standard_easy() -> Self {
        Self::standard(RankedSimulationDifficultyV1::Easy)
            .expect("the Standard/Easy ranked policy is a protocol constant")
    }

    pub fn standard_hard() -> Self {
        Self::standard(RankedSimulationDifficultyV1::Hard)
            .expect("the Standard/Hard ranked policy is a protocol constant")
    }

    pub fn original_easy() -> Self {
        Self::original_parity(RankedSimulationDifficultyV1::Easy)
            .expect("the Original/Easy ranked policy is a protocol constant")
    }

    pub fn original_medium() -> Self {
        Self::original_parity(RankedSimulationDifficultyV1::Medium)
            .expect("the Original/Medium ranked policy is a protocol constant")
    }

    pub fn original_hard() -> Self {
        Self::original_parity(RankedSimulationDifficultyV1::Hard)
            .expect("the Original/Hard ranked policy is a protocol constant")
    }

    pub const fn identity(self) -> RankedSimulationPolicyV1 {
        self.identity
    }

    pub const fn expected_config(self) -> SimConfig {
        self.expected_config
    }

    pub const fn is_original_parity(self) -> bool {
        matches!(
            self.identity.preset,
            RankedSimulationPresetV1::OriginalParity
        )
    }

    pub fn validate_config(self, observed: SimConfig) -> Result<(), RankedSimulationPolicyError> {
        if let Some(field) = observed.first_ranked_difference(self.expected_config) {
            return Err(RankedSimulationPolicyError::ConfigMismatch { field });
        }
        Ok(())
    }
}

impl SimConfig {
    /// Canonical current-feature configuration for a Standard ranked board.
    /// This complete literal represents fresh profile defaults and cannot be
    /// influenced by process options.
    pub fn standard_ranked(difficulty: DifficultyLevel) -> Self {
        assert!(
            matches!(
                difficulty,
                DifficultyLevel::Easy | DifficultyLevel::Medium | DifficultyLevel::Hard
            ),
            "ranked policy V1 supports only Easy/Medium/Hard"
        );
        Self {
            difficulty,
            fix_hard_reaction_times: true,
            enable_unbinding: true,
            clean_hands_npc_kills_invalidate: false,
            reusable_cloaks: true,
            reversible_background_patches: false,
            item_gameplay: ItemGameplayConfig::default(),
            noise_distraction_feedback: true,
            diplomacy: true,
            npc_faction_wars: true,
            more_combat_gestures: true,
            gesture_quality_damage: true,
            fog_of_war: false,
            script_enabled: true,
            highlander: false,
            highlander2: false,
            golden_eye: false,
            ignore_default_loose: false,
            bypass_fog_sprites_crash: false,
            amount_of_speaking: 5,
            synchronous_pathfinding: false,
            sherwood_trading: true,
            enable_timed_missions: true,
            enable_dynamic_ambience: true,
        }
    }

    /// Canonical shipped-game policy for an Original-parity ranked board.
    /// Raw Original RNG traces remain a separate diagnostic capability and
    /// are never fabricated by ranking.
    pub fn original_parity_ranked(difficulty: DifficultyLevel) -> Self {
        let mut config = Self::standard_ranked(difficulty);
        config.fix_hard_reaction_times = false;
        config.enable_unbinding = false;
        config.reusable_cloaks = false;
        config.item_gameplay = ItemGameplayConfig::classic();
        config.noise_distraction_feedback = false;
        config.diplomacy = false;
        config.npc_faction_wars = false;
        config.more_combat_gestures = false;
        config.gesture_quality_damage = false;
        config.fog_of_war = false;
        config.sherwood_trading = false;
        config.enable_timed_missions = false;
        config.enable_dynamic_ambience = false;
        config
    }

    pub fn first_ranked_difference(self, expected: Self) -> Option<RankedSimulationConfigField> {
        let Self {
            difficulty,
            fix_hard_reaction_times,
            enable_unbinding,
            clean_hands_npc_kills_invalidate,
            reusable_cloaks,
            reversible_background_patches,
            item_gameplay,
            noise_distraction_feedback,
            diplomacy,
            npc_faction_wars,
            more_combat_gestures,
            gesture_quality_damage,
            fog_of_war,
            script_enabled,
            highlander,
            highlander2,
            golden_eye,
            ignore_default_loose,
            bypass_fog_sprites_crash,
            amount_of_speaking,
            synchronous_pathfinding,
            sherwood_trading,
            enable_timed_missions,
            enable_dynamic_ambience,
        } = self;
        let Self {
            difficulty: expected_difficulty,
            fix_hard_reaction_times: expected_fix_hard_reaction_times,
            enable_unbinding: expected_enable_unbinding,
            clean_hands_npc_kills_invalidate: expected_clean_hands_npc_kills_invalidate,
            reusable_cloaks: expected_reusable_cloaks,
            reversible_background_patches: expected_reversible_background_patches,
            item_gameplay: expected_item_gameplay,
            noise_distraction_feedback: expected_noise_distraction_feedback,
            diplomacy: expected_diplomacy,
            npc_faction_wars: expected_npc_faction_wars,
            more_combat_gestures: expected_more_combat_gestures,
            gesture_quality_damage: expected_gesture_quality_damage,
            fog_of_war: expected_fog_of_war,
            script_enabled: expected_script_enabled,
            highlander: expected_highlander,
            highlander2: expected_highlander2,
            golden_eye: expected_golden_eye,
            ignore_default_loose: expected_ignore_default_loose,
            bypass_fog_sprites_crash: expected_bypass_fog_sprites_crash,
            amount_of_speaking: expected_amount_of_speaking,
            synchronous_pathfinding: expected_synchronous_pathfinding,
            sherwood_trading: expected_sherwood_trading,
            enable_timed_missions: expected_enable_timed_missions,
            enable_dynamic_ambience: expected_enable_dynamic_ambience,
        } = expected;

        [
            (difficulty != expected_difficulty).then_some(RankedSimulationConfigField::Difficulty),
            (fix_hard_reaction_times != expected_fix_hard_reaction_times)
                .then_some(RankedSimulationConfigField::FixHardReactionTimes),
            (enable_unbinding != expected_enable_unbinding)
                .then_some(RankedSimulationConfigField::EnableUnbinding),
            (clean_hands_npc_kills_invalidate != expected_clean_hands_npc_kills_invalidate)
                .then_some(RankedSimulationConfigField::CleanHandsNpcKillsInvalidate),
            (reversible_background_patches != expected_reversible_background_patches)
                .then_some(RankedSimulationConfigField::ReversibleBackgroundPatches),
            (reusable_cloaks != expected_reusable_cloaks)
                .then_some(RankedSimulationConfigField::ReusableCloaks),
            (item_gameplay != expected_item_gameplay)
                .then_some(RankedSimulationConfigField::ItemGameplay),
            (noise_distraction_feedback != expected_noise_distraction_feedback)
                .then_some(RankedSimulationConfigField::NoiseDistractionFeedback),
            (diplomacy != expected_diplomacy).then_some(RankedSimulationConfigField::Diplomacy),
            (npc_faction_wars != expected_npc_faction_wars)
                .then_some(RankedSimulationConfigField::NpcFactionWars),
            (more_combat_gestures != expected_more_combat_gestures)
                .then_some(RankedSimulationConfigField::MoreCombatGestures),
            (gesture_quality_damage != expected_gesture_quality_damage)
                .then_some(RankedSimulationConfigField::GestureQualityDamage),
            (fog_of_war != expected_fog_of_war).then_some(RankedSimulationConfigField::FogOfWar),
            (script_enabled != expected_script_enabled)
                .then_some(RankedSimulationConfigField::ScriptEnabled),
            (highlander != expected_highlander).then_some(RankedSimulationConfigField::Highlander),
            (highlander2 != expected_highlander2)
                .then_some(RankedSimulationConfigField::Highlander2),
            (golden_eye != expected_golden_eye).then_some(RankedSimulationConfigField::GoldenEye),
            (ignore_default_loose != expected_ignore_default_loose)
                .then_some(RankedSimulationConfigField::IgnoreDefaultLoose),
            (bypass_fog_sprites_crash != expected_bypass_fog_sprites_crash)
                .then_some(RankedSimulationConfigField::BypassFogSpritesCrash),
            (amount_of_speaking != expected_amount_of_speaking)
                .then_some(RankedSimulationConfigField::AmountOfSpeaking),
            (synchronous_pathfinding != expected_synchronous_pathfinding)
                .then_some(RankedSimulationConfigField::SynchronousPathfinding),
            (sherwood_trading != expected_sherwood_trading)
                .then_some(RankedSimulationConfigField::SherwoodTrading),
            (enable_timed_missions != expected_enable_timed_missions)
                .then_some(RankedSimulationConfigField::EnableTimedMissions),
            (enable_dynamic_ambience != expected_enable_dynamic_ambience)
                .then_some(RankedSimulationConfigField::EnableDynamicAmbience),
        ]
        .into_iter()
        .flatten()
        .next()
    }

    pub fn from_options(options: &GlobalOptions, difficulty: DifficultyLevel) -> Self {
        difficulty
            .validate()
            .expect("cannot construct simulation config with invalid difficulty rules");
        Self {
            difficulty,
            fix_hard_reaction_times: true,
            enable_unbinding: true,
            clean_hands_npc_kills_invalidate: false,
            reusable_cloaks: true,
            reversible_background_patches: false,
            item_gameplay: ItemGameplayConfig::classic(),
            noise_distraction_feedback: true,
            diplomacy: true,
            npc_faction_wars: true,
            more_combat_gestures: true,
            gesture_quality_damage: true,
            fog_of_war: false,
            script_enabled: options.script_enabled,
            highlander: options.highlander,
            highlander2: options.highlander2,
            golden_eye: options.golden_eye,
            ignore_default_loose: options.ignore_default_loose,
            bypass_fog_sprites_crash: options.bypass_fog_sprites_crash,
            amount_of_speaking: 5,
            synchronous_pathfinding: false,
            sherwood_trading: true,
            enable_timed_missions: true,
            enable_dynamic_ambience: true,
        }
    }

    pub fn validate(self) -> Result<Self, crate::player_profile::InvalidDifficultyRules> {
        self.difficulty.validate()?;
        Ok(self)
    }
}

impl Default for SimConfig {
    fn default() -> Self {
        Self::from_options(&GlobalOptions::default(), DifficultyLevel::Medium)
    }
}

// ─── Global options ──────────────────────────────────────────────────

/// Application-wide startup options.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct GlobalOptions {
    pub major_version: u16,
    pub minor_version: u16,
    pub build_number: u16,
    pub release_name: String,

    // Directories
    pub save_directory: String,
    pub level_directory: String,
    pub sound_directory: String,
    pub music_directory: String,
    pub character_directory: String,
    pub animation_directory: String,
    pub configuration_directory: String,
    pub interface_directory: String,
    pub text_directory: String,
    pub cinematics_directory: String,

    // Runtime flags
    pub quit: bool,
    pub console: bool,
    pub sound_enabled: bool,
    pub check_sound_data: bool,
    pub patch_characters: bool,
    pub highlander: bool,
    pub highlander2: bool,
    pub whatsup: bool,
    pub debug_surfaces: bool,
    pub ezekiel2517: bool,
    pub golden_eye: bool,
    pub script_enabled: bool,
    pub ignore_default_loose: bool,
    pub set_reg: bool,
    pub bypass_fog_sprites_crash: bool,
}

impl Default for GlobalOptions {
    fn default() -> Self {
        Self {
            major_version: 1,
            minor_version: 2,
            build_number: 0,
            release_name: String::new(),

            save_directory: "Data/Savegame".into(),
            level_directory: "Data/Levels".into(),
            sound_directory: "Data/Sounds".into(),
            music_directory: "Data/Musics".into(),
            character_directory: "Data/Characters".into(),
            animation_directory: "Data/Animations".into(),
            configuration_directory: "Data/Configuration".into(),
            interface_directory: "Data/Interface".into(),
            text_directory: "Data/Text".into(),
            cinematics_directory: "Data/Cinematics".into(),

            quit: false,
            console: true,
            sound_enabled: true,
            check_sound_data: false,
            patch_characters: false,
            highlander: false,
            highlander2: false,
            whatsup: false,
            debug_surfaces: false,
            ezekiel2517: false,
            golden_eye: false,
            script_enabled: true,
            ignore_default_loose: false,
            set_reg: false,
            bypass_fog_sprites_crash: false,
        }
    }
}

// ─── Global singleton ───────────────────────────────────────────────
//
// A process-wide store the menu layer reaches without having to thread
// `&GlobalOptions` through every UI call.  Populated by
// `main_entry::parse_cli` once the CLI has been walked.

static GLOBAL_OPTIONS: Mutex<Option<GlobalOptions>> = Mutex::new(None);

impl GlobalOptions {
    /// Install the process-wide `GlobalOptions`.  Usually called once
    /// from `main_entry::parse_cli` after argument parsing.
    pub fn set_global(opts: GlobalOptions) {
        *GLOBAL_OPTIONS.lock().unwrap() = Some(opts);
    }

    /// Acquire the process-wide `GlobalOptions`.  Returns `None` if
    /// `set_global` has not been called yet (tests, headless tooling).
    pub fn global() -> std::sync::MutexGuard<'static, Option<GlobalOptions>> {
        GLOBAL_OPTIONS.lock().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::{RankedSimulationConfigField, SimConfig};
    use crate::gameplay_config::ItemGameplayConfig;
    use crate::player_profile::DifficultyLevel;

    #[test]
    fn background_reversal_is_authoritative_and_not_ranked() {
        let original = SimConfig::default();
        assert!(!original.reversible_background_patches);
        let opted_in = SimConfig {
            reversible_background_patches: true,
            ..original
        };
        assert_ne!(
            robin_util::state_hash::compute(&original),
            robin_util::state_hash::compute(&opted_in)
        );
        assert_eq!(
            opted_in.first_ranked_difference(original),
            Some(RankedSimulationConfigField::ReversibleBackgroundPatches)
        );
        let roundtrip: SimConfig =
            serde_json::from_str(&serde_json::to_string(&opted_in).unwrap()).unwrap();
        assert!(roundtrip.reversible_background_patches);
        assert!(
            !super::super::cloak::preserve_original_gameplay_behavior(opted_in)
                .reversible_background_patches
        );
    }

    #[test]
    fn hard_reaction_time_fix_is_the_fresh_simulation_default() {
        assert!(SimConfig::default().fix_hard_reaction_times);
        assert!(SimConfig::default().enable_unbinding);
        assert_eq!(
            SimConfig::default().item_gameplay,
            ItemGameplayConfig::classic()
        );
        assert!(SimConfig::default().noise_distraction_feedback);
        assert!(SimConfig::default().sherwood_trading);
        assert!(SimConfig::default().diplomacy);
        assert!(SimConfig::default().more_combat_gestures);
        assert!(SimConfig::default().gesture_quality_damage);
    }

    #[test]
    fn state_without_the_setting_retains_original_reaction_times() {
        let mut serialized =
            serde_json::to_value(SimConfig::default()).expect("serialize simulation config");
        serialized
            .as_object_mut()
            .expect("simulation config is an object")
            .remove("fix_hard_reaction_times");
        serialized
            .as_object_mut()
            .expect("simulation config is an object")
            .remove("enable_unbinding");
        serialized
            .as_object_mut()
            .expect("simulation config is an object")
            .remove("reusable_cloaks");
        serialized
            .as_object_mut()
            .expect("simulation config is an object")
            .remove("item_gameplay");
        serialized
            .as_object_mut()
            .expect("simulation config is an object")
            .remove("noise_distraction_feedback");

        let config: SimConfig =
            serde_json::from_value(serialized).expect("deserialize legacy simulation config");
        assert!(!config.fix_hard_reaction_times);
        assert!(config.enable_unbinding);
        assert!(!config.reusable_cloaks);
        assert_eq!(config.item_gameplay, ItemGameplayConfig::classic());
        assert!(!config.noise_distraction_feedback);
    }

    #[test]
    fn state_without_trading_does_not_opt_into_the_new_economy() {
        let mut serialized =
            serde_json::to_value(SimConfig::default()).expect("serialize simulation config");
        serialized
            .as_object_mut()
            .expect("simulation config is an object")
            .remove("sherwood_trading");
        let config: SimConfig =
            serde_json::from_value(serialized).expect("deserialize legacy simulation config");
        assert!(!config.sherwood_trading);
    }

    #[test]
    fn old_sim_state_preserves_legacy_diplomacy_behavior() {
        let mut serialized = serde_json::to_value(SimConfig::default()).unwrap();
        let object = serialized.as_object_mut().unwrap();
        object.remove("diplomacy");
        object.remove("npc_faction_wars");
        let config: SimConfig = serde_json::from_value(serialized).unwrap();
        assert!(!config.diplomacy);
        assert!(config.npc_faction_wars);
    }

    #[test]
    fn current_rules_are_required_in_native_simulation_state() {
        for field in [
            "more_combat_gestures",
            "gesture_quality_damage",
            "fog_of_war",
        ] {
            let mut serialized = serde_json::to_value(SimConfig::default()).unwrap();
            serialized.as_object_mut().unwrap().remove(field);
            let error = serde_json::from_value::<SimConfig>(serialized)
                .expect_err("native simulation state must contain combat gesture rules");
            assert!(
                error.to_string().contains(field),
                "missing-field error did not name {field}: {error}"
            );
        }
    }

    #[test]
    fn ranked_presets_and_difference_detection_cover_current_simulation_rules() {
        let standard = SimConfig::standard_ranked(DifficultyLevel::Medium);
        assert!(standard.diplomacy);
        assert!(standard.npc_faction_wars);
        assert!(standard.more_combat_gestures);
        assert!(standard.gesture_quality_damage);
        assert!(!standard.fog_of_war);

        let original = SimConfig::original_parity_ranked(DifficultyLevel::Medium);
        assert!(!original.diplomacy);
        assert!(!original.npc_faction_wars);
        assert!(!original.more_combat_gestures);
        assert!(!original.gesture_quality_damage);
        assert!(!original.fog_of_war);

        for (field, mutate) in [
            (
                RankedSimulationConfigField::Diplomacy,
                (|config: &mut SimConfig| config.diplomacy = !config.diplomacy)
                    as fn(&mut SimConfig),
            ),
            (
                RankedSimulationConfigField::NpcFactionWars,
                (|config: &mut SimConfig| config.npc_faction_wars = !config.npc_faction_wars)
                    as fn(&mut SimConfig),
            ),
            (
                RankedSimulationConfigField::MoreCombatGestures,
                (|config: &mut SimConfig| {
                    config.more_combat_gestures = !config.more_combat_gestures
                }) as fn(&mut SimConfig),
            ),
            (
                RankedSimulationConfigField::GestureQualityDamage,
                (|config: &mut SimConfig| {
                    config.gesture_quality_damage = !config.gesture_quality_damage
                }) as fn(&mut SimConfig),
            ),
            (
                RankedSimulationConfigField::FogOfWar,
                (|config: &mut SimConfig| config.fog_of_war = !config.fog_of_war)
                    as fn(&mut SimConfig),
            ),
        ] {
            let mut observed = standard;
            mutate(&mut observed);
            assert_eq!(observed.first_ranked_difference(standard), Some(field));
        }
    }
}
