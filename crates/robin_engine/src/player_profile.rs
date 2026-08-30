//! Player profile and profile manager.
//!
//! Profiles store per-player settings (graphics, sound, keys) and gameplay
//! stats (score, ransom, difficulty, etc.).  The manager owns the collection,
//! handles persistence (JSON via serde), and tracks which profile is active.
//!
//! The global singleton is the authoritative data store for persistence.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::campaign::CampaignValue;
use crate::gameplay_config::GameplayConfig;
use crate::graphic_config::GraphicConfig;
use crate::sound_config::SoundConfig;

// ─── Types ──────────────────────────────────────────────────────

/// The three difficulty values understood by retail scripts and resources.
///
/// New presets deliberately map onto one of these values at the legacy
/// boundary.  This keeps `GetDifficultyLevel` and the three-entry scroll
/// presence table ABI-compatible without constraining the Rust simulation to
/// three presets.
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
pub enum LegacyDifficultyLevel {
    Easy,
    Medium,
    Hard,
}

impl LegacyDifficultyLevel {
    pub const fn to_u32(self) -> u32 {
        match self {
            Self::Easy => 0,
            Self::Medium => 1,
            Self::Hard => 2,
        }
    }
}

/// Fully resolved, deterministic rules for one difficulty.
///
/// Percentages are integers on purpose: custom settings are serialized into
/// saves/replays and synchronized by the host, so no locale parsing or
/// platform-dependent float state enters the authoritative configuration.
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
pub struct DifficultyRules {
    pub legacy_level: LegacyDifficultyLevel,
    pub enemy_fighting_percent: u16,
    pub enemy_shooting_percent: u16,
    pub enemy_iq_percent: u16,
    pub enemy_life_points_percent: u16,
    pub reaction_time_percent: u16,
    /// Hostile soldiers' optical radius after the authored live view is built.
    pub hostile_soldier_view_distance_percent: u16,
    /// Hostile soldiers' optical half-aperture after authored posture modifiers.
    pub hostile_soldier_view_angle_percent: u16,
    /// Hostile soldiers' effective range for PC-produced noises.
    pub hostile_soldier_noise_sensitivity_percent: u16,
    pub blip_detection_range_percent: u16,
    pub carnage_percent: u16,
    pub six_capacity: u16,
    pub twelve_capacity: u16,
    /// Zero disables automatic PC healing; otherwise this is its frame cadence.
    pub pc_auto_heal_interval_frames: u16,
    pub accurate_net_preview: bool,
    pub protect_allies_from_pc_arrows: bool,
    pub special_strike_base_frames: u16,
    pub pc_punch_concussion_percent: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DifficultyRuleField {
    EnemyFightingPercent,
    EnemyShootingPercent,
    EnemyIqPercent,
    EnemyLifePointsPercent,
    ReactionTimePercent,
    HostileSoldierViewDistancePercent,
    HostileSoldierViewAnglePercent,
    HostileSoldierNoiseSensitivityPercent,
    BlipDetectionRangePercent,
    CarnagePercent,
    SixCapacity,
    TwelveCapacity,
    PcAutoHealIntervalFrames,
    SpecialStrikeBaseFrames,
    PcPunchConcussionPercent,
}

impl DifficultyRuleField {
    pub const fn name(self) -> &'static str {
        match self {
            Self::EnemyFightingPercent => "enemy_fighting_percent",
            Self::EnemyShootingPercent => "enemy_shooting_percent",
            Self::EnemyIqPercent => "enemy_iq_percent",
            Self::EnemyLifePointsPercent => "enemy_life_points_percent",
            Self::ReactionTimePercent => "reaction_time_percent",
            Self::HostileSoldierViewDistancePercent => "hostile_soldier_view_distance_percent",
            Self::HostileSoldierViewAnglePercent => "hostile_soldier_view_angle_percent",
            Self::HostileSoldierNoiseSensitivityPercent => {
                "hostile_soldier_noise_sensitivity_percent"
            }
            Self::BlipDetectionRangePercent => "blip_detection_range_percent",
            Self::CarnagePercent => "carnage_percent",
            Self::SixCapacity => "six_capacity",
            Self::TwelveCapacity => "twelve_capacity",
            Self::PcAutoHealIntervalFrames => "pc_auto_heal_interval_frames",
            Self::SpecialStrikeBaseFrames => "special_strike_base_frames",
            Self::PcPunchConcussionPercent => "pc_punch_concussion_percent",
        }
    }
}

/// Why a custom difficulty rule set was rejected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvalidDifficultyRules {
    pub field: DifficultyRuleField,
    pub value: u16,
    pub min: u16,
    pub max: u16,
}

impl std::fmt::Display for InvalidDifficultyRules {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "difficulty field '{}' is {}, expected {}..={}",
            self.field.name(),
            self.value,
            self.min,
            self.max
        )
    }
}

impl std::error::Error for InvalidDifficultyRules {}

impl DifficultyRules {
    pub const EASY: Self = Self {
        legacy_level: LegacyDifficultyLevel::Easy,
        enemy_fighting_percent: 50,
        enemy_shooting_percent: 50,
        enemy_iq_percent: 50,
        enemy_life_points_percent: 50,
        reaction_time_percent: 200,
        hostile_soldier_view_distance_percent: 100,
        hostile_soldier_view_angle_percent: 100,
        hostile_soldier_noise_sensitivity_percent: 100,
        blip_detection_range_percent: 130,
        carnage_percent: 50,
        six_capacity: 8,
        twelve_capacity: 15,
        pc_auto_heal_interval_frames: 100,
        accurate_net_preview: true,
        protect_allies_from_pc_arrows: true,
        special_strike_base_frames: 13,
        pc_punch_concussion_percent: 100,
    };

    pub const MEDIUM: Self = Self {
        legacy_level: LegacyDifficultyLevel::Medium,
        enemy_fighting_percent: 100,
        enemy_shooting_percent: 100,
        enemy_iq_percent: 100,
        enemy_life_points_percent: 100,
        reaction_time_percent: 100,
        hostile_soldier_view_distance_percent: 100,
        hostile_soldier_view_angle_percent: 100,
        hostile_soldier_noise_sensitivity_percent: 100,
        blip_detection_range_percent: 100,
        carnage_percent: 100,
        six_capacity: 6,
        twelve_capacity: 12,
        pc_auto_heal_interval_frames: 0,
        accurate_net_preview: false,
        protect_allies_from_pc_arrows: true,
        special_strike_base_frames: 10,
        pc_punch_concussion_percent: 100,
    };

    pub const HARD: Self = Self {
        legacy_level: LegacyDifficultyLevel::Hard,
        enemy_fighting_percent: 200,
        enemy_shooting_percent: 200,
        enemy_iq_percent: 200,
        enemy_life_points_percent: 150,
        reaction_time_percent: 50,
        hostile_soldier_view_distance_percent: 100,
        hostile_soldier_view_angle_percent: 100,
        hostile_soldier_noise_sensitivity_percent: 100,
        blip_detection_range_percent: 70,
        carnage_percent: 200,
        six_capacity: 4,
        twelve_capacity: 9,
        pc_auto_heal_interval_frames: 0,
        accurate_net_preview: false,
        protect_allies_from_pc_arrows: false,
        special_strike_base_frames: 0,
        pc_punch_concussion_percent: 150,
    };

    /// Initial systematic balance for Legendary. Values continue the same
    /// progression as the retail presets instead of adding unrelated rules.
    pub const LEGENDARY: Self = Self {
        legacy_level: LegacyDifficultyLevel::Hard,
        enemy_fighting_percent: 400,
        enemy_shooting_percent: 400,
        enemy_iq_percent: 400,
        enemy_life_points_percent: 200,
        reaction_time_percent: 25,
        // These three post-port rules make stealth materially less forgiving
        // without granting guards omnidirectional or map-wide perception.
        // A stock 400-unit, ~57-degree cone becomes 540 units and ~72 degrees;
        // hearing range grows linearly by half. Easy/Medium/Hard stay exact.
        hostile_soldier_view_distance_percent: 135,
        hostile_soldier_view_angle_percent: 125,
        hostile_soldier_noise_sensitivity_percent: 150,
        blip_detection_range_percent: 40,
        carnage_percent: 400,
        six_capacity: 2,
        twelve_capacity: 6,
        pc_auto_heal_interval_frames: 0,
        accurate_net_preview: false,
        protect_allies_from_pc_arrows: false,
        special_strike_base_frames: 0,
        pc_punch_concussion_percent: 200,
    };

    /// Validate all user-editable fields. Invalid persisted values are an
    /// error rather than being silently clamped into a different ruleset.
    pub fn validate(self) -> Result<Self, InvalidDifficultyRules> {
        fn range(
            field: DifficultyRuleField,
            value: u16,
            min: u16,
            max: u16,
        ) -> Result<(), InvalidDifficultyRules> {
            if (min..=max).contains(&value) {
                Ok(())
            } else {
                Err(InvalidDifficultyRules {
                    field,
                    value,
                    min,
                    max,
                })
            }
        }

        range(
            DifficultyRuleField::EnemyFightingPercent,
            self.enemy_fighting_percent,
            25,
            400,
        )?;
        range(
            DifficultyRuleField::EnemyShootingPercent,
            self.enemy_shooting_percent,
            25,
            400,
        )?;
        range(
            DifficultyRuleField::EnemyIqPercent,
            self.enemy_iq_percent,
            25,
            400,
        )?;
        range(
            DifficultyRuleField::EnemyLifePointsPercent,
            self.enemy_life_points_percent,
            25,
            400,
        )?;
        range(
            DifficultyRuleField::ReactionTimePercent,
            self.reaction_time_percent,
            10,
            400,
        )?;
        range(
            DifficultyRuleField::HostileSoldierViewDistancePercent,
            self.hostile_soldier_view_distance_percent,
            25,
            200,
        )?;
        range(
            DifficultyRuleField::HostileSoldierViewAnglePercent,
            self.hostile_soldier_view_angle_percent,
            25,
            200,
        )?;
        range(
            DifficultyRuleField::HostileSoldierNoiseSensitivityPercent,
            self.hostile_soldier_noise_sensitivity_percent,
            25,
            200,
        )?;
        range(
            DifficultyRuleField::BlipDetectionRangePercent,
            self.blip_detection_range_percent,
            10,
            200,
        )?;
        range(
            DifficultyRuleField::CarnagePercent,
            self.carnage_percent,
            25,
            400,
        )?;
        range(DifficultyRuleField::SixCapacity, self.six_capacity, 0, 99)?;
        range(
            DifficultyRuleField::TwelveCapacity,
            self.twelve_capacity,
            0,
            99,
        )?;
        range(
            DifficultyRuleField::PcAutoHealIntervalFrames,
            self.pc_auto_heal_interval_frames,
            0,
            3600,
        )?;
        range(
            DifficultyRuleField::SpecialStrikeBaseFrames,
            self.special_strike_base_frames,
            0,
            60,
        )?;
        range(
            DifficultyRuleField::PcPunchConcussionPercent,
            self.pc_punch_concussion_percent,
            25,
            400,
        )?;
        Ok(self)
    }

    pub fn scale_capacity(self, base: u16, percent: u16, max_allowed: u16) -> u16 {
        let scaled = u32::from(base) * u32::from(percent) / 100;
        scaled.min(u32::from(max_allowed)) as u16
    }

    pub fn enemy_fighting(self, base: u16, max_allowed: u16) -> u16 {
        self.scale_capacity(base, self.enemy_fighting_percent, max_allowed)
    }

    pub fn enemy_shooting(self, base: u16, max_allowed: u16) -> u16 {
        self.scale_capacity(base, self.enemy_shooting_percent, max_allowed)
    }

    pub fn enemy_iq(self, base: u16, max_allowed: u16) -> u16 {
        self.scale_capacity(base, self.enemy_iq_percent, max_allowed)
    }

    pub fn enemy_life_points(self, base: u16, max_allowed: u16) -> u16 {
        self.scale_capacity(base, self.enemy_life_points_percent, max_allowed)
    }

    pub fn ammo_capacity(self, base: u16) -> u16 {
        match base {
            6 => self.six_capacity,
            12 => self.twelve_capacity,
            other => other,
        }
    }

    pub fn percent_as_f32(percent: u16) -> f32 {
        percent as f32 / 100.0
    }

    pub fn scale_hostile_soldier_view_radius(self, radius: u16) -> u16 {
        self.scale_capacity(radius, self.hostile_soldier_view_distance_percent, u16::MAX)
    }

    pub fn hostile_soldier_view_angle_factor(self) -> f32 {
        Self::percent_as_f32(self.hostile_soldier_view_angle_percent)
    }

    pub fn hostile_soldier_noise_factor(self) -> f32 {
        Self::percent_as_f32(self.hostile_soldier_noise_sensitivity_percent)
    }
}

/// Application difficulty preset. `Custom` contains the resolved rules so the
/// simulation, save, replay and multiplayer state are self-contained.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum DifficultyLevel {
    Easy,
    Medium,
    Hard,
    Legendary,
    Custom(DifficultyRules),
}

#[derive(Serialize, Deserialize)]
enum DifficultyLevelWire {
    Easy,
    Medium,
    Hard,
    Legendary,
    Custom(DifficultyRules),
}

impl Serialize for DifficultyLevel {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let wire = match *self {
            Self::Easy => DifficultyLevelWire::Easy,
            Self::Medium => DifficultyLevelWire::Medium,
            Self::Hard => DifficultyLevelWire::Hard,
            Self::Legendary => DifficultyLevelWire::Legendary,
            Self::Custom(rules) => DifficultyLevelWire::Custom(rules),
        };
        wire.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for DifficultyLevel {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        match DifficultyLevelWire::deserialize(deserializer)? {
            DifficultyLevelWire::Easy => Ok(Self::Easy),
            DifficultyLevelWire::Medium => Ok(Self::Medium),
            DifficultyLevelWire::Hard => Ok(Self::Hard),
            DifficultyLevelWire::Legendary => Ok(Self::Legendary),
            DifficultyLevelWire::Custom(rules) => {
                Self::custom(rules).map_err(serde::de::Error::custom)
            }
        }
    }
}

impl Default for DifficultyLevel {
    fn default() -> Self {
        Self::Medium
    }
}

/// Invalid conversion from the retail numeric difficulty ABI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvalidDifficultyLevel(pub u32);

impl std::fmt::Display for InvalidDifficultyLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown legacy difficulty level {}", self.0)
    }
}

impl std::error::Error for InvalidDifficultyLevel {}

impl DifficultyLevel {
    pub fn from_u32(v: u32) -> Result<Self, InvalidDifficultyLevel> {
        match v {
            0 => Ok(Self::Easy),
            1 => Ok(Self::Medium),
            2 => Ok(Self::Hard),
            _ => Err(InvalidDifficultyLevel(v)),
        }
    }

    pub const fn rules(self) -> DifficultyRules {
        match self {
            Self::Easy => DifficultyRules::EASY,
            Self::Medium => DifficultyRules::MEDIUM,
            Self::Hard => DifficultyRules::HARD,
            Self::Legendary => DifficultyRules::LEGENDARY,
            Self::Custom(rules) => rules,
        }
    }

    pub fn custom(rules: DifficultyRules) -> Result<Self, InvalidDifficultyRules> {
        Ok(Self::Custom(rules.validate()?))
    }

    pub fn validate(self) -> Result<Self, InvalidDifficultyRules> {
        self.rules().validate()?;
        Ok(self)
    }

    pub const fn to_u32(self) -> u32 {
        self.rules().legacy_level.to_u32()
    }

    /// Resolve extensions to one of the three retail presets for the
    /// Original-RNG parity harness. No custom or Legendary rule is allowed to
    /// leak into a trace that claims original behavior.
    pub const fn original_parity_preset(self) -> Self {
        match self.rules().legacy_level {
            LegacyDifficultyLevel::Easy => Self::Easy,
            LegacyDifficultyLevel::Medium => Self::Medium,
            LegacyDifficultyLevel::Hard => Self::Hard,
        }
    }

    /// V1 leaderboard identities exist only for the immutable retail presets.
    pub const fn is_ranked_v1_eligible(self) -> bool {
        matches!(self, Self::Easy | Self::Medium | Self::Hard)
    }

    /// Apply difficulty scaling to a base capacity value.
    ///
    /// Only meaningful for Lacklandist (enemy) entities — callers must
    /// check camp before calling, or pass the base value unchanged for
    /// non-Lacklandist entities.
    ///
    /// - `Easy`: base * easy_factor, capped at max_allowed
    /// - `Medium`: base unchanged
    /// - `Hard`: base * hard_factor, capped at max_allowed
    pub fn modify_capacity(
        self,
        base: u16,
        easy_factor: f32,
        hard_factor: f32,
        max_allowed: u16,
    ) -> u16 {
        let factor = match self.rules().legacy_level {
            LegacyDifficultyLevel::Easy => easy_factor,
            LegacyDifficultyLevel::Medium => 1.0,
            LegacyDifficultyLevel::Hard => hard_factor,
        };
        let scaled = (base as f32 * factor) as u16;
        scaled.min(max_allowed)
    }
}

// ─── Difficulty parameters ─────────────────────────────────────

/// Difficulty modifier constants.
pub mod difficulty_params {
    // Carnage/warcrime — affects post-mission team recruitment
    pub const EASY_CARNAGE: f32 = 0.5;
    pub const HARD_CARNAGE: f32 = 2.0;

    // Reaction time — how quickly enemies respond (higher = slower)
    pub const EASY_REACTIONTIME: f32 = 2.0;
    pub const HARD_REACTIONTIME: f32 = 0.5;

    // Fighting ability — melee combat effectiveness
    pub const EASY_ENEMY_FIGHTING: f32 = 0.5;
    pub const HARD_ENEMY_FIGHTING: f32 = 2.0;

    // Shooting ability — ranged combat effectiveness
    pub const EASY_ENEMY_SHOOTING: f32 = 0.5;
    pub const HARD_ENEMY_SHOOTING: f32 = 2.0;

    // IQ — AI decision-making quality
    pub const EASY_ENEMY_IQ: f32 = 0.5;
    pub const HARD_ENEMY_IQ: f32 = 2.0;

    // Life points — enemy health pools
    pub const EASY_ENEMY_LIFEPOINTS: f32 = 0.5;
    pub const HARD_ENEMY_LIFEPOINTS: f32 = 1.5;

    // Blip detection range — player's ability to spot enemies on minimap
    pub const EASY_BLIP_DETECTION_RANGE: f32 = 1.3;
    pub const HARD_BLIP_DETECTION_RANGE: f32 = 0.7;
}

// `modify_enemy_capacity` is an alias used by the other worktree branch —
// delegate to the identical `modify_capacity` method above.
impl DifficultyLevel {}

/// Initial ransom value for a new profile.
const INITIAL_RANSOM: u32 = 100;

/// Per-profile savegame subdirectory name: the profile id, zero-padded
/// to width 3.
pub fn profile_save_subdirectory(profile_id: u32) -> String {
    format!("Profile_{profile_id:03}")
}

/// A single player profile containing settings and gameplay state.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct PlayerProfile {
    pub name: String,
    pub id: u32,
    pub difficulty: DifficultyLevel,
    pub score: u32,
    pub ransom: u32,
    pub preserved_lives: u32,
    pub play_time: u32,
    pub progression: u32,
    /// Lossless all-time history, independent from replaceable campaign save
    /// slots. Campaign attempts are promoted idempotently at synchronization.
    pub campaign_history: crate::campaign_history::ProfileCampaignHistory,
    pub minimap_x: f32,
    pub minimap_y: f32,
    pub graphic_config: GraphicConfig,
    #[serde(default = "GameplayConfig::migrated")]
    pub gameplay_config: GameplayConfig,
    #[serde(default)]
    pub multiplayer_config: crate::multiplayer_config::MultiplayerConfig,
    pub sound_config: SoundConfig,
    // KeyConfig moved to host (robin_rs) — it's input binding config,
    // not sim state. See Decision 5B. Host keeps a parallel KeyConfig
    // store keyed by profile id.
    /// Whether this profile is the active one.
    pub active: bool,
}

impl PlayerProfile {
    /// Create a new profile with the given name and difficulty, using default
    /// configs.
    pub fn new(id: u32, name: String, difficulty: DifficultyLevel) -> Self {
        difficulty
            .validate()
            .expect("cannot create a player profile with invalid difficulty rules");
        Self {
            name,
            id,
            difficulty,
            score: 0,
            ransom: INITIAL_RANSOM,
            preserved_lives: 0,
            play_time: 0,
            progression: 0,
            campaign_history: crate::campaign_history::ProfileCampaignHistory::default(),
            minimap_x: 65536.0,
            minimap_y: 65536.0,
            graphic_config: GraphicConfig::default(),
            gameplay_config: GameplayConfig::default(),
            multiplayer_config: crate::multiplayer_config::MultiplayerConfig::default(),
            sound_config: SoundConfig::default(),
            active: false,
        }
    }

    pub fn earned_achievements(&self) -> crate::achievement::AchievementSet {
        self.achievement_aggregation().earned()
    }

    pub fn achievement_aggregation(&self) -> crate::achievement::AchievementAggregationSummary {
        self.campaign_history.achievement_aggregation()
    }

    pub fn promote_campaign_history(
        &mut self,
        campaign: &crate::campaign::Campaign,
        profiles: &crate::profiles::ProfileManager,
    ) -> Result<usize, crate::campaign_history::ProfileHistoryPromotionError> {
        self.campaign_history.promote_campaign(campaign, profiles)
    }

    pub fn lifetime_campaign_totals(&self) -> crate::campaign_history::CampaignHistoryTotals {
        self.campaign_history.totals()
    }
}

// ─── Manager ────────────────────────────────────────────────────

/// Manages a collection of player profiles, with one optionally active.
///
/// Profiles are persisted as a JSON file (`profiles.json`) inside
/// `save_directory`.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct PlayerProfileManager {
    pub profiles: Vec<PlayerProfile>,
    /// Index of the active profile, or `None` if no profile is active.
    pub active_index: Option<usize>,
    /// Directory where the profile file is stored.
    pub save_directory: String,
    /// Counter for generating unique profile IDs.
    next_id: u32,
    /// Whether the profiles were auto-created defaults.
    pub default_profiles: bool,
}

impl PlayerProfileManager {
    /// Create an empty manager that will persist to `save_directory`.
    pub fn new(save_directory: String) -> Self {
        Self {
            profiles: Vec::new(),
            active_index: None,
            save_directory,
            next_id: 0,
            default_profiles: false,
        }
    }

    /// Load profiles from `<directory>/profiles.json`.
    ///
    /// If the file does not exist a default manager with a single "Robin"
    /// profile is created and saved.
    pub fn load(directory: &str) -> std::io::Result<Self> {
        let path = Self::profiles_path(directory);

        if path.exists() {
            let data = fs::read_to_string(&path)?;
            let mut mgr: PlayerProfileManager = serde_json::from_str(&data)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            for (index, profile) in mgr.profiles.iter().enumerate() {
                profile.campaign_history.validate_schema().map_err(|version| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!(
                            "player profile {index} has invalid or unsupported campaign-history schema {version}"
                        ),
                    )
                })?;
                profile.difficulty.rules().validate().map_err(|error| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("profile '{}' has invalid {error}", profile.name),
                    )
                })?;
            }
            mgr.save_directory = directory.to_owned();
            Ok(mgr)
        } else {
            let mut mgr = Self::new(directory.to_owned());
            let idx = mgr.create_profile("Robin".to_owned(), DifficultyLevel::Medium);
            mgr.set_active(idx);
            mgr.default_profiles = true;
            mgr.save()?;
            Ok(mgr)
        }
    }

    /// Persist the current state to `<save_directory>/profiles.json`.
    pub fn save(&self) -> std::io::Result<()> {
        fs::create_dir_all(&self.save_directory)?;
        let path = Self::profiles_path(&self.save_directory);
        let data = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        fs::write(path, data)
    }

    /// Return a reference to the active profile, or `None`.
    pub fn get_active(&self) -> Option<&PlayerProfile> {
        self.active_index.map(|i| &self.profiles[i])
    }

    /// Return a mutable reference to the active profile, or `None`.
    pub fn get_active_mut(&mut self) -> Option<&mut PlayerProfile> {
        self.active_index.map(|i| &mut self.profiles[i])
    }

    /// Set the active profile by index.  If `index` is out of range the
    /// active slot is silently cleared rather than panicking.
    pub fn set_active(&mut self, index: usize) {
        if index >= self.profiles.len() {
            // Clear active on overflow.
            if let Some(prev) = self.active_index {
                self.profiles[prev].active = false;
            }
            self.active_index = None;
            return;
        }

        // Clear old active flag.
        if let Some(prev) = self.active_index {
            self.profiles[prev].active = false;
        }
        self.profiles[index].active = true;
        self.active_index = Some(index);
    }

    /// Create a new profile and return its index.
    ///
    /// Convenience wrapper around [`create_profile_with_screen_dims`]
    /// for callers that have no live screen dimensions to offer (the
    /// resolution-fallback chain collapses to `active.resolution → 1024×768`).
    pub fn create_profile(&mut self, name: String, difficulty: DifficultyLevel) -> usize {
        self.create_profile_with_screen_dims(name, difficulty, None)
    }

    /// Create a new profile, picking the initial resolution by this
    /// priority chain:
    ///   1. If an active profile exists, copy its resolution.
    ///   2. Else if `screen_dims` is `Some` (window already open), use it.
    ///   3. Else fall back to 1024×768.
    pub fn create_profile_with_screen_dims(
        &mut self,
        name: String,
        difficulty: DifficultyLevel,
        screen_dims: Option<(u32, u32)>,
    ) -> usize {
        let id = self.next_id;
        self.next_id += 1;

        let mut profile = PlayerProfile::new(id, name, difficulty);
        if let Some(active) = self.get_active() {
            profile.graphic_config.resolution_x = active.graphic_config.resolution_x;
            profile.graphic_config.resolution_y = active.graphic_config.resolution_y;
        } else if let Some((w, h)) = screen_dims {
            profile.graphic_config.resolution_x = w as f32;
            profile.graphic_config.resolution_y = h as f32;
        }
        // Else: GraphicConfig::default() already produces 1024×768.

        self.profiles.push(profile);
        self.profiles.len() - 1
    }

    /// Delete the profile at `index`.
    ///
    /// If the deleted profile was active, the active profile is cleared.
    /// Also wipes the on-disk per-profile savegame directory
    /// (`<save_directory>/Profile_NNN`).
    ///
    /// # Panics
    /// Panics if `index` is out of bounds.
    pub fn delete_profile(&mut self, index: usize) {
        assert!(
            index < self.profiles.len(),
            "profile index {index} out of range (have {})",
            self.profiles.len()
        );

        // Snapshot the id before removal so the disk cleanup can target
        // the correct `Profile_NNN/` subdirectory.
        let profile_id = self.profiles[index].id;

        self.profiles.remove(index);

        // Wipe per-profile savegame folder. Best-effort: the directory
        // may not exist (e.g. a profile that never wrote a save); that
        // is not an error.
        let save_dir = Path::new(&self.save_directory).join(profile_save_subdirectory(profile_id));
        if save_dir.exists()
            && let Err(err) = fs::remove_dir_all(&save_dir)
        {
            tracing::warn!(
                "delete_profile: failed to remove {} ({err:#})",
                save_dir.display()
            );
        }

        // Fix up active_index after removal.
        self.active_index = match self.active_index {
            Some(ai) if ai == index => {
                // The active profile was deleted — clear it.
                None
            }
            Some(ai) if ai > index => Some(ai - 1),
            other => other,
        };

        // Sync the `active` flag on the profile.
        for (i, p) in self.profiles.iter_mut().enumerate() {
            p.active = self.active_index == Some(i);
        }
    }

    /// Check whether a profile with the given name exists.
    pub fn has_profile(&self, name: &str) -> bool {
        self.profiles.iter().any(|p| p.name == name)
    }

    /// Return the number of profiles.
    pub fn profile_count(&self) -> usize {
        self.profiles.len()
    }

    fn profiles_path(directory: &str) -> PathBuf {
        Path::new(directory).join("profiles.json")
    }
}

// ─── Global singleton ───────────────────────────────────────────

static GLOBAL_PPM: Mutex<Option<PlayerProfileManager>> = Mutex::new(None);

impl PlayerProfileManager {
    /// Get a lock on the global profile manager.
    pub fn global() -> std::sync::MutexGuard<'static, Option<PlayerProfileManager>> {
        GLOBAL_PPM.lock().unwrap()
    }
}

// ─── repr(C) bridge types ───────────────────────────────────────

/// C-compatible struct for exchanging scalar profile data across FFI.
#[repr(C)]
pub struct CProfileScalars {
    pub id: u32,
    pub score: u32,
    pub ransom: u32,
    pub preserved_lives: u32,
    pub play_time: u32,
    pub progression: u32,
    pub difficulty: u32,
    pub minimap_x: f32,
    pub minimap_y: f32,
}

// (C ABI FFI section removed)
// The following FFI functions were removed:
// robin_ppm_load, robin_ppm_save, robin_ppm_profile_count, robin_ppm_get_active_index,
// robin_ppm_set_active, robin_ppm_create_profile, robin_ppm_delete_profile,
// robin_ppm_has_profile_name, robin_ppm_is_default_profiles, robin_ppm_reset_default_profiles,
// robin_ppm_get_save_directory, robin_pp_get_scalars, robin_pp_set_scalars,
// robin_pp_get_name, robin_pp_set_name, robin_pp_get_graphic_config, robin_pp_set_graphic_config,
// robin_pp_get_sound_config, robin_pp_set_sound_config, robin_pp_get_key_config,
// robin_pp_set_key_config, robin_pp_synchronize_with_campaign

// FFI removed — only synchronize_with_campaign kept as normal Rust.
/// Sync end-of-mission values from `campaign` into profile `idx`.
///
/// `mission_play_time_secs` is the total play time for the mission just
/// ending, in seconds.  Callers pass `GameCallbacks::get_current_playing_time`
/// so any live segment that suspend-play-time has queued but not yet
/// flushed to the campaign's mission-length counter is still counted —
/// the callback boundary forces the split, so we take the authoritative
/// value from the caller rather than re-reading the campaign value.
pub fn synchronize_with_campaign(
    idx: usize,
    campaign: &crate::campaign::Campaign,
    profiles: &crate::profiles::ProfileManager,
    mission_play_time_secs: u32,
) {
    let mut ppm_guard = GLOBAL_PPM.lock().unwrap();
    let profile = match ppm_guard.as_mut().and_then(|m| m.profiles.get_mut(idx)) {
        Some(p) => p,
        None => return,
    };

    profile.score = campaign.get_value(CampaignValue::Score) as u32;
    profile.ransom = campaign.get_value(CampaignValue::Ransom) as u32;
    profile.progression = campaign.get_progression(profiles);
    profile.play_time += mission_play_time_secs;
    profile
        .promote_campaign_history(campaign, profiles)
        .unwrap_or_else(|error| panic!("cannot promote campaign history into profile: {error}"));

    let dead = campaign.get_value(CampaignValue::DeadSoldiers) as u32;
    let alive = campaign.get_value(CampaignValue::LivingSoldiers) as u32;
    if dead != 0 || alive != 0 {
        profile.preserved_lives = (100.0 * alive as f32 / (dead + alive) as f32) as u32;
    } else {
        profile.preserved_lives = 0;
    }
}

// ─── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_profile_default_values() {
        let mut mgr = PlayerProfileManager::new("/tmp/test_profiles".into());
        let idx = mgr.create_profile("Alice".into(), DifficultyLevel::Easy);

        assert_eq!(idx, 0);
        assert_eq!(mgr.profiles[0].name, "Alice");
        assert_eq!(mgr.profiles[0].difficulty, DifficultyLevel::Easy);
        assert_eq!(mgr.profiles[0].ransom, 100);
        assert_eq!(mgr.profiles[0].score, 0);
        assert!(!mgr.profiles[0].active);
    }

    #[test]
    fn set_active_and_get_active() {
        let mut mgr = PlayerProfileManager::new("/tmp/test_profiles".into());
        mgr.create_profile("Alice".into(), DifficultyLevel::Easy);
        mgr.create_profile("Bob".into(), DifficultyLevel::Hard);

        assert!(mgr.get_active().is_none());

        mgr.set_active(1);
        let active = mgr.get_active().unwrap();
        assert_eq!(active.name, "Bob");
        assert!(active.active);
        assert!(!mgr.profiles[0].active);
    }

    #[test]
    fn switch_active_clears_previous() {
        let mut mgr = PlayerProfileManager::new("/tmp/test_profiles".into());
        mgr.create_profile("Alice".into(), DifficultyLevel::Easy);
        mgr.create_profile("Bob".into(), DifficultyLevel::Hard);

        mgr.set_active(0);
        assert!(mgr.profiles[0].active);

        mgr.set_active(1);
        assert!(!mgr.profiles[0].active);
        assert!(mgr.profiles[1].active);
    }

    #[test]
    fn set_active_out_of_bounds_clears() {
        // Out-of-range silently clears the active slot rather than panicking.
        let mut mgr = PlayerProfileManager::new("/tmp/test_profiles".into());
        mgr.create_profile("Alice".into(), DifficultyLevel::Easy);
        mgr.set_active(0);
        assert_eq!(mgr.active_index, Some(0));
        assert!(mgr.profiles[0].active);

        mgr.set_active(99);
        assert_eq!(mgr.active_index, None);
        assert!(!mgr.profiles[0].active);
    }

    #[test]
    fn delete_profile_clears_active() {
        let mut mgr = PlayerProfileManager::new("/tmp/test_profiles".into());
        mgr.create_profile("Alice".into(), DifficultyLevel::Easy);
        mgr.create_profile("Bob".into(), DifficultyLevel::Hard);
        mgr.set_active(0);

        mgr.delete_profile(0);
        assert!(mgr.get_active().is_none());
        assert_eq!(mgr.profile_count(), 1);
        assert_eq!(mgr.profiles[0].name, "Bob");
    }

    #[test]
    fn delete_profile_adjusts_active_index() {
        let mut mgr = PlayerProfileManager::new("/tmp/test_profiles".into());
        mgr.create_profile("Alice".into(), DifficultyLevel::Easy);
        mgr.create_profile("Bob".into(), DifficultyLevel::Hard);
        mgr.create_profile("Carol".into(), DifficultyLevel::Medium);
        mgr.set_active(2);

        // Delete a profile before the active one.
        mgr.delete_profile(0);
        assert_eq!(mgr.get_active().unwrap().name, "Carol");
        assert_eq!(mgr.active_index, Some(1));
    }

    #[test]
    fn has_profile_by_name() {
        let mut mgr = PlayerProfileManager::new("/tmp/test_profiles".into());
        mgr.create_profile("Robin".into(), DifficultyLevel::Medium);
        assert!(mgr.has_profile("Robin"));
        assert!(!mgr.has_profile("Marian"));
    }

    #[test]
    fn create_profile_inherits_resolution() {
        let mut mgr = PlayerProfileManager::new("/tmp/test_profiles".into());
        let idx0 = mgr.create_profile("Alice".into(), DifficultyLevel::Easy);
        mgr.profiles[idx0]
            .graphic_config
            .set_resolution(1920.0, 1080.0);
        mgr.set_active(idx0);

        let idx1 = mgr.create_profile("Bob".into(), DifficultyLevel::Medium);
        assert_eq!(mgr.profiles[idx1].graphic_config.resolution_x, 1920.0);
        assert_eq!(mgr.profiles[idx1].graphic_config.resolution_y, 1080.0);
    }

    #[test]
    fn serde_roundtrip() {
        let mut mgr = PlayerProfileManager::new("/tmp/test_profiles".into());
        mgr.create_profile("Robin".into(), DifficultyLevel::Medium);
        mgr.create_profile("Marian".into(), DifficultyLevel::Hard);
        mgr.set_active(1);
        mgr.profiles[0].score = 42;
        mgr.profiles[1].sound_config.music_volume = 5;

        let json = serde_json::to_string_pretty(&mgr).unwrap();
        let restored: PlayerProfileManager = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.profiles.len(), 2);
        assert_eq!(restored.active_index, Some(1));
        assert_eq!(restored.profiles[0].score, 42);
        assert_eq!(restored.profiles[1].sound_config.music_volume, 5);
        assert!(restored.profiles[1].active);
        assert!(!restored.profiles[0].active);
    }

    #[test]
    fn profile_without_gameplay_object_migrates_item_features_off() {
        let profile = PlayerProfile::new(7, "Legacy".into(), DifficultyLevel::Medium);
        let mut value = serde_json::to_value(profile).expect("serialize profile");
        value
            .as_object_mut()
            .expect("profile object")
            .remove("gameplay_config");

        let restored: PlayerProfile = serde_json::from_value(value).expect("legacy profile");
        assert_eq!(
            restored.gameplay_config.item_gameplay,
            crate::gameplay_config::ItemGameplayConfig::classic()
        );
        assert_eq!(
            restored.gameplay_config.item_previews,
            crate::gameplay_config::ItemPreviewConfig::classic()
        );
        assert!(!restored.gameplay_config.noise_distraction_feedback);
        assert!(restored.gameplay_config.enable_unbinding);
        assert!(restored.gameplay_config.autosave_enabled);
        assert!(!restored.gameplay_config.reusable_cloaks);
    }

    #[test]
    fn profile_json_without_campaign_history_is_rejected() {
        let mut mgr = PlayerProfileManager::new("/tmp/test_profiles".into());
        mgr.create_profile("Robin".into(), DifficultyLevel::Medium);
        let mut json = serde_json::to_value(&mgr).unwrap();
        json["profiles"][0]
            .as_object_mut()
            .unwrap()
            .remove("campaign_history");

        assert!(serde_json::from_value::<PlayerProfileManager>(json).is_err());
    }

    #[test]
    fn profile_unlock_union_is_derived_from_attested_lifetime_attempts() {
        let mut profile = PlayerProfile::new(0, "Robin".into(), DifficultyLevel::Medium);
        let mut profiles = crate::profiles::ProfileManager::new();
        profiles.missions.push(crate::profiles::MissionProfile {
            id: 42,
            mission_name: "The Rescue".into(),
            ..Default::default()
        });
        let mut campaign = crate::campaign::Campaign::default();
        campaign.missions.push(crate::mission::Mission {
            profile_idx: Some(0),
            ..crate::mission::Mission::new()
        });
        campaign.current_mission_idx = Some(0);
        let mut tracker = crate::achievement::MissionAchievementState::from_mission_start();
        tracker
            .record_evaluation(
                crate::achievement::AchievementId::PileOBones,
                crate::achievement::AchievementEvaluation::Earned,
            )
            .unwrap();
        let results = *tracker.finalize_success();
        campaign.record_mission_attempt(
            0,
            crate::campaign_history::MissionAttemptOutcome::Won,
            Some(100),
            Some(0xbeef),
            60,
            crate::engine::SimConfig::default(),
            &crate::mission_stat::MissionStat::default(),
            Some(results),
        );
        campaign
            .attest_mission_achievement_attempt(
                campaign.latest_mission_attempt_key().unwrap(),
                crate::achievement::AchievementUnlockPolicy::default(),
                crate::achievement::AchievementRunContext::default(),
                &profiles,
            )
            .unwrap();

        assert_eq!(
            profile
                .promote_campaign_history(&campaign, &profiles)
                .unwrap(),
            1
        );
        assert_eq!(
            profile
                .promote_campaign_history(&campaign, &profiles)
                .unwrap(),
            0
        );
        assert_eq!(profile.earned_achievements().len(), 1);
        assert!(
            profile
                .earned_achievements()
                .contains(crate::achievement::AchievementId::PileOBones)
        );
    }

    #[test]
    fn load_creates_default_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = PlayerProfileManager::load(dir.path().to_str().unwrap()).unwrap();

        assert_eq!(mgr.profile_count(), 1);
        assert_eq!(mgr.profiles[0].name, "Robin");
        assert_eq!(mgr.active_index, Some(0));
        assert!(mgr.default_profiles);

        // File should have been written.
        assert!(dir.path().join("profiles.json").exists());
    }

    #[test]
    fn load_roundtrip_via_filesystem() {
        let dir = tempfile::tempdir().unwrap();
        let dir_str = dir.path().to_str().unwrap();

        // Create and save.
        {
            let mut mgr = PlayerProfileManager::new(dir_str.into());
            mgr.create_profile("Alice".into(), DifficultyLevel::Hard);
            mgr.set_active(0);
            mgr.save().unwrap();
        }

        // Load back.
        let mgr = PlayerProfileManager::load(dir_str).unwrap();
        assert_eq!(mgr.profile_count(), 1);
        assert_eq!(mgr.profiles[0].name, "Alice");
        assert_eq!(mgr.profiles[0].difficulty, DifficultyLevel::Hard);
        assert_eq!(mgr.active_index, Some(0));
        assert!(mgr.profiles[0].campaign_history.attempts().is_empty());
    }

    #[test]
    fn load_rejects_aggregate_only_rust_profile() {
        let dir = tempfile::tempdir().unwrap();
        let dir_str = dir.path().to_str().unwrap();
        let mut mgr = PlayerProfileManager::new(dir_str.into());
        let idx = mgr.create_profile("Legacy Robin".into(), DifficultyLevel::Medium);
        mgr.profiles[idx].score = 42;
        mgr.profiles[idx].play_time = 99;
        let mut document = serde_json::to_value(&mgr).unwrap();
        document["profiles"][idx]
            .as_object_mut()
            .unwrap()
            .remove("campaign_history");
        fs::create_dir_all(dir_str).unwrap();
        fs::write(
            PlayerProfileManager::profiles_path(dir_str),
            serde_json::to_vec_pretty(&document).unwrap(),
        )
        .unwrap();

        let error = PlayerProfileManager::load(dir_str).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    #[should_panic(expected = "out of range")]
    fn delete_profile_out_of_bounds_panics() {
        let mut mgr = PlayerProfileManager::new("/tmp/test_profiles".into());
        mgr.delete_profile(0);
    }

    #[test]
    fn unique_ids_across_profiles() {
        let mut mgr = PlayerProfileManager::new("/tmp/test_profiles".into());
        mgr.create_profile("A".into(), DifficultyLevel::Easy);
        mgr.create_profile("B".into(), DifficultyLevel::Easy);
        mgr.create_profile("C".into(), DifficultyLevel::Easy);

        let ids: Vec<u32> = mgr.profiles.iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![0, 1, 2]);
    }

    #[test]
    fn difficulty_level_roundtrip() {
        assert_eq!(DifficultyLevel::from_u32(0), Ok(DifficultyLevel::Easy));
        assert_eq!(DifficultyLevel::from_u32(1), Ok(DifficultyLevel::Medium));
        assert_eq!(DifficultyLevel::from_u32(2), Ok(DifficultyLevel::Hard));
        assert_eq!(
            DifficultyLevel::from_u32(99),
            Err(InvalidDifficultyLevel(99))
        );

        assert_eq!(DifficultyLevel::Easy.to_u32(), 0);
        assert_eq!(DifficultyLevel::Medium.to_u32(), 1);
        assert_eq!(DifficultyLevel::Hard.to_u32(), 2);
        assert_eq!(DifficultyLevel::Legendary.to_u32(), 2);
    }

    #[test]
    fn legendary_rules_continue_the_hard_progression() {
        let rules = DifficultyLevel::Legendary.rules();
        assert_eq!(rules.enemy_fighting(40, 100), 100);
        assert_eq!(rules.enemy_life_points(100, 10_000), 200);
        assert_eq!(rules.ammo_capacity(6), 2);
        assert_eq!(rules.ammo_capacity(12), 6);
        assert_eq!(rules.reaction_time_percent, 25);
        assert_eq!(rules.scale_hostile_soldier_view_radius(400), 540);
        assert_eq!(rules.hostile_soldier_view_angle_percent, 125);
        assert_eq!(rules.hostile_soldier_noise_sensitivity_percent, 150);
        assert_eq!(rules.legacy_level, LegacyDifficultyLevel::Hard);
    }

    #[test]
    fn retail_presets_preserve_original_soldier_perception() {
        for difficulty in [
            DifficultyLevel::Easy,
            DifficultyLevel::Medium,
            DifficultyLevel::Hard,
        ] {
            let rules = difficulty.rules();
            assert_eq!(rules.scale_hostile_soldier_view_radius(400), 400);
            assert_eq!(rules.hostile_soldier_view_angle_percent, 100);
            assert_eq!(rules.hostile_soldier_noise_sensitivity_percent, 100);
        }
    }

    #[test]
    fn original_parity_and_ranked_v1_reject_extension_identities() {
        let mut custom = DifficultyRules::MEDIUM;
        custom.legacy_level = LegacyDifficultyLevel::Easy;
        custom.hostile_soldier_view_distance_percent = 175;
        let custom = DifficultyLevel::custom(custom).unwrap();

        assert_eq!(
            DifficultyLevel::Legendary.original_parity_preset(),
            DifficultyLevel::Hard
        );
        assert_eq!(custom.original_parity_preset(), DifficultyLevel::Easy);
        assert!(!DifficultyLevel::Legendary.is_ranked_v1_eligible());
        assert!(!custom.is_ranked_v1_eligible());
        assert!(DifficultyLevel::Hard.is_ranked_v1_eligible());
    }

    #[test]
    fn invalid_custom_rules_are_rejected() {
        let mut rules = DifficultyRules::MEDIUM;
        rules.reaction_time_percent = 0;
        let error = DifficultyLevel::custom(rules).unwrap_err();
        assert_eq!(error.field, DifficultyRuleField::ReactionTimePercent);
    }

    #[test]
    fn custom_rules_serialize_with_the_profile() {
        let mut rules = DifficultyRules::MEDIUM;
        rules.enemy_life_points_percent = 175;
        let difficulty = DifficultyLevel::custom(rules).unwrap();
        let json = serde_json::to_string(&difficulty).unwrap();
        let restored: DifficultyLevel = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, difficulty);
    }

    #[test]
    fn classic_profile_json_remains_backward_compatible() {
        assert_eq!(
            serde_json::from_str::<DifficultyLevel>(r#""Hard""#).unwrap(),
            DifficultyLevel::Hard
        );
        assert_eq!(
            serde_json::to_string(&DifficultyLevel::Medium).unwrap(),
            r#""Medium""#
        );
    }

    #[test]
    fn invalid_custom_json_is_rejected_during_deserialization() {
        let mut rules = DifficultyRules::MEDIUM;
        rules.enemy_iq_percent = 0;
        let unchecked = DifficultyLevelWire::Custom(rules);
        let json = serde_json::to_string(&unchecked).unwrap();
        let error = serde_json::from_str::<DifficultyLevel>(&json).unwrap_err();
        assert!(error.to_string().contains("enemy_iq_percent"));
    }
}
