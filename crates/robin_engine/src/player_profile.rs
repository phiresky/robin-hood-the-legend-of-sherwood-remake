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
use crate::graphic_config::GraphicConfig;
use crate::sound_config::SoundConfig;

// ─── Types ──────────────────────────────────────────────────────

/// Difficulty levels.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub enum DifficultyLevel {
    Easy,
    #[default]
    Medium,
    Hard,
}

impl DifficultyLevel {
    pub fn from_u32(v: u32) -> Self {
        match v {
            0 => Self::Easy,
            1 => Self::Medium,
            2 => Self::Hard,
            _ => Self::Medium,
        }
    }

    pub fn to_u32(self) -> u32 {
        match self {
            Self::Easy => 0,
            Self::Medium => 1,
            Self::Hard => 2,
        }
    }

    /// Get the current difficulty from the global profile manager.
    /// Returns `Medium` if no profile is active.
    pub fn current() -> Self {
        let ppm = PlayerProfileManager::global();
        ppm.as_ref()
            .and_then(|mgr| mgr.get_active())
            .map(|p| p.difficulty)
            .unwrap_or(Self::Medium)
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
        match self {
            Self::Easy => {
                let scaled = (base as f32 * easy_factor) as u16;
                scaled.min(max_allowed)
            }
            Self::Medium => base,
            Self::Hard => {
                let scaled = (base as f32 * hard_factor) as u16;
                scaled.min(max_allowed)
            }
        }
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
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct PlayerProfile {
    pub name: String,
    pub id: u32,
    pub difficulty: DifficultyLevel,
    pub score: u32,
    pub ransom: u32,
    pub preserved_lives: u32,
    pub play_time: u32,
    pub progression: u32,
    pub minimap_x: f32,
    pub minimap_y: f32,
    pub graphic_config: GraphicConfig,
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
        Self {
            name,
            id,
            difficulty,
            score: 0,
            ransom: INITIAL_RANSOM,
            preserved_lives: 0,
            play_time: 0,
            progression: 0,
            minimap_x: 65536.0,
            minimap_y: 65536.0,
            graphic_config: GraphicConfig::default(),
            sound_config: SoundConfig::default(),
            active: false,
        }
    }
}

// ─── Manager ────────────────────────────────────────────────────

/// Manages a collection of player profiles, with one optionally active.
///
/// Profiles are persisted as a JSON file (`profiles.json`) inside
/// `save_directory`.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
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
    /// resolution-fallback chain collapses to `active.resolution → 800×600`).
    pub fn create_profile(&mut self, name: String, difficulty: DifficultyLevel) -> usize {
        self.create_profile_with_screen_dims(name, difficulty, None)
    }

    /// Create a new profile, picking the initial resolution by this
    /// priority chain:
    ///   1. If an active profile exists, copy its resolution.
    ///   2. Else if `screen_dims` is `Some` (window already open), use it.
    ///   3. Else fall back to 800×600.
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
        // Else: GraphicConfig::default() already produces 800×600.

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
        assert_eq!(DifficultyLevel::from_u32(0), DifficultyLevel::Easy);
        assert_eq!(DifficultyLevel::from_u32(1), DifficultyLevel::Medium);
        assert_eq!(DifficultyLevel::from_u32(2), DifficultyLevel::Hard);
        assert_eq!(DifficultyLevel::from_u32(99), DifficultyLevel::Medium);

        assert_eq!(DifficultyLevel::Easy.to_u32(), 0);
        assert_eq!(DifficultyLevel::Medium.to_u32(), 1);
        assert_eq!(DifficultyLevel::Hard.to_u32(), 2);
    }
}
