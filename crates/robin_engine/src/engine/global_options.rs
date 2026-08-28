//! Application-wide startup options (`GlobalOptions`).

use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::gameplay_config::ItemGameplayConfig;
use crate::player_profile::DifficultyLevel;

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

fn default_enabled() -> bool {
    true
}

const fn default_enabled() -> bool {
    true
}

impl SimConfig {
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
            item_gameplay: ItemGameplayConfig::classic(),
            noise_distraction_feedback: true,
            diplomacy: true,
            npc_faction_wars: true,
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
    use super::SimConfig;
    use crate::gameplay_config::ItemGameplayConfig;

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
    fn old_sim_state_disables_diplomacy_extensions() {
        let mut serialized = serde_json::to_value(SimConfig::default()).unwrap();
        serialized.as_object_mut().unwrap().remove("diplomacy");
        let config: SimConfig = serde_json::from_value(serialized).unwrap();
        assert!(!config.diplomacy);
    }
}
