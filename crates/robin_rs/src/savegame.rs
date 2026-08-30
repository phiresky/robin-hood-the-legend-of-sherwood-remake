//! Save game management.
//!
//! Uses serde JSON for the on-disk payload.  Save files are stored under
//! the OS-appropriate per-user data directory (see
//! [`save_file::default_save_directory`]).  Per-slot layout:
//!
//!   `<save_dir>/<filename>.json`  → full payload ([`save_file::GameSaveFile`])
//!   `<save_dir>/<filename>_thumb.png` → thumbnail
//!   `<save_dir>/saves.json`       → slot index / metadata
//!
//! Special slot filenames (Continue/QuickSave/Restart/Sherwood) are
//! defined in [`save_file::special_slots`].

use crate::host::Host;
use robin_engine::campaign as engine_campaign;
use robin_engine::campaign::CampaignValue;
#[cfg(test)]
use robin_engine::engine as engine_api;
use robin_engine::engine::Engine;
use robin_engine::profiles::ProfileManager;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::save_file::{self, GameSaveFile, SaveHeader, SaveProvenance, Thumbnail};

/// Metadata for a single save game slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveGame {
    /// Display name shown in the UI (UTF-8).
    pub text: String,
    /// Base filename (without directory or extension).
    pub filename: String,
    /// Mission profile ID at time of save.
    pub mission_id: u32,
    /// Save file version.
    pub version: u32,
    /// Wall-clock timestamp as decimal Unix seconds.
    pub timestamp: String,
    /// Whether this is a special slot (continue, quicksave, restart, sherwood).
    pub special: Option<SpecialSlot>,
    /// Localized/static mission name at time of save, when profile data was available.
    pub mission_name: String,
    /// Stable profile identity at save time. Temporary slots use `None` only
    /// until their first payload is published.
    pub player_profile_id: Option<u32>,
    /// Player name frozen at save time, so later profile renames do not alter
    /// the meaning of existing saves.
    pub player_name: String,
    /// Campaign progression percentage at time of save.
    pub campaign_progress: Option<u32>,
    /// Number of completed missions at time of save.
    pub missions_done: Option<usize>,
    /// Total missions known to the campaign at time of save.
    pub missions_total: Option<usize>,
    /// Gang size at time of save.
    pub gang_size: Option<usize>,
    /// Current ransom value at time of save.
    pub ransom: Option<i32>,
    /// Current blazon value at time of save.
    pub blazons: Option<i32>,
    /// Current amulet value at time of save.
    pub amulets: Option<i32>,
    /// Mirrors the payload header so connected load pickers can hide local
    /// multiplayer diagnostics without reading every save file.
    pub multiplayer_diagnostic: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpecialSlot {
    Continue,
    QuickSave,
    ExQuickSave,
    Restart,
    Sherwood,
    Autosave,
}

impl SpecialSlot {
    /// Detect special slot type from the well-known filenames.
    pub fn from_filename(filename: &str) -> Option<Self> {
        match filename {
            "Continue" => Some(Self::Continue),
            "QuickSave" => Some(Self::QuickSave),
            "ExQuickSave" => Some(Self::ExQuickSave),
            "Restart" => Some(Self::Restart),
            "Sherwood" => Some(Self::Sherwood),
            filename if is_generated_autosave_filename(filename) => Some(Self::Autosave),
            _ => None,
        }
    }
}

/// Autosave names are storage identifiers, not arbitrary player labels. Keep
/// this recognizer strict so cleanup and manual-delete guards can never target
/// a path outside the autosave namespace.
pub(crate) fn is_generated_autosave_filename(filename: &str) -> bool {
    let Some(rest) = filename.strip_prefix("Autosave_") else {
        return false;
    };
    let Some((timestamp, sequence)) = rest.split_once('_') else {
        return false;
    };
    !timestamp.is_empty()
        && timestamp.bytes().all(|byte| byte.is_ascii_digit())
        && sequence.len() >= 4
        && sequence.bytes().all(|byte| byte.is_ascii_digit())
}

impl SaveGame {
    pub fn new(filename: String, text: String, mission_id: u32) -> Self {
        let special = SpecialSlot::from_filename(&filename);
        SaveGame {
            text,
            filename,
            mission_id,
            version: save_file::SAVE_FORMAT_VERSION,
            timestamp: String::new(),
            special,
            mission_name: String::new(),
            player_profile_id: None,
            player_name: String::new(),
            campaign_progress: None,
            missions_done: None,
            missions_total: None,
            gang_size: None,
            ransom: None,
            blazons: None,
            amulets: None,
            multiplayer_diagnostic: false,
        }
    }

    pub fn is_special(&self) -> bool {
        self.special.is_some() || self.is_autosave()
    }

    pub fn is_continue(&self) -> bool {
        self.special == Some(SpecialSlot::Continue)
    }

    pub fn is_restart(&self) -> bool {
        self.special == Some(SpecialSlot::Restart)
    }

    pub fn is_sherwood(&self) -> bool {
        self.special == Some(SpecialSlot::Sherwood)
    }

    pub fn is_autosave(&self) -> bool {
        self.special == Some(SpecialSlot::Autosave)
            || is_generated_autosave_filename(&self.filename)
    }

    pub(crate) fn validate_published_metadata(&self) -> Result<()> {
        if self.version != save_file::SAVE_FORMAT_VERSION {
            anyhow::bail!(
                "save index entry {:?} uses obsolete Rust schema {}; expected {}",
                self.filename,
                self.version,
                save_file::SAVE_FORMAT_VERSION
            );
        }
        if self.mission_id == 0 {
            anyhow::bail!(
                "save index entry {:?} has invalid mission ID zero",
                self.filename
            );
        }
        let timestamp = self.timestamp.parse::<u64>().with_context(|| {
            format!(
                "save index entry {:?} has an invalid timestamp",
                self.filename
            )
        })?;
        if timestamp == 0 {
            anyhow::bail!(
                "save index entry {:?} has Unix timestamp zero",
                self.filename
            );
        }
        if self.mission_name.trim().is_empty() {
            anyhow::bail!("save index entry {:?} has no mission name", self.filename);
        }
        if self.player_profile_id.is_none() {
            anyhow::bail!(
                "save index entry {:?} has no player identity",
                self.filename
            );
        }
        if self.player_name.trim().is_empty() {
            anyhow::bail!("save index entry {:?} has no player name", self.filename);
        }
        if self.campaign_progress.is_none()
            || self.missions_done.is_none()
            || self.missions_total.is_none()
            || self.gang_size.is_none()
            || self.ransom.is_none()
            || self.blazons.is_none()
            || self.amulets.is_none()
        {
            anyhow::bail!(
                "save index entry {:?} is missing required campaign summary metadata",
                self.filename
            );
        }
        Ok(())
    }
}

fn required_save_provenance(
    host: &Host,
    engine: &Engine,
    mission_id: u32,
    profiles: Option<&ProfileManager>,
) -> Result<SaveProvenance> {
    let profiles = profiles.context("save requires the active mission profile table")?;
    let mission = engine
        .campaign()
        .get_mission(mission_id, profiles)
        .with_context(|| format!("save mission ID {mission_id} is absent from the campaign"))?;
    let profile_idx = mission
        .profile_idx
        .context("save mission has no profile index")? as usize;
    let mission_profile = profiles.missions.get(profile_idx).with_context(|| {
        format!(
            "save mission profile index {profile_idx} is out of range (have {})",
            profiles.missions.len()
        )
    })?;
    let mission_name = if mission_profile.mission_name.trim().is_empty() {
        if mission_profile.mission_filename.trim().is_empty() {
            anyhow::bail!("save mission ID {mission_id} has neither a display name nor a filename");
        }
        // A custom mission need not ship a localized display title. Its
        // canonical filename is still authoritative provenance, not a made-up
        // placeholder.
        mission_profile.mission_filename.clone()
    } else {
        mission_profile.mission_name.clone()
    };
    let player = host
        .application_context
        .active_profile_snapshot()
        .map_err(anyhow::Error::msg)
        .context("save requires an active player profile")?;
    SaveProvenance::new(mission_name, player.id, player.name)
}

/// Manages a collection of save games for a player profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveGameManager {
    pub saves: Vec<SaveGame>,
    pub save_directory: String,
    next_id: u32,
}

impl SaveGameManager {
    pub fn new(save_directory: String) -> Self {
        SaveGameManager {
            saves: Vec::new(),
            save_directory,
            next_id: 0,
        }
    }

    /// Create a manager rooted at the active profile's save subdirectory
    /// (`<root>/Profile_NNN/`). Loads the existing slot index from
    /// `saves.json` if present; otherwise starts empty.
    pub fn open_for_context(application_context: &crate::host::ApplicationContext) -> Self {
        let dir = application_context
            .active_profile_save_directory()
            .unwrap_or_else(|error| panic!("save manager requires an active profile: {error}"));
        let dir_str = dir.to_string_lossy().into_owned();
        let mut manager = match Self::load_index(&dir_str) {
            Ok(mgr) => mgr,
            Err(err) => {
                tracing::info!("No save index at {dir_str} ({err}) - starting fresh");
                Self::new(dir_str)
            }
        };
        if let Err(error) = crate::autosave::load_into_manager(&mut manager) {
            tracing::error!(
                "Autosave manifest for {} could not be loaded: {error:#}",
                manager.save_directory
            );
        }
        manager
    }

    /// Find the slot for one of the well-known special filenames, or
    /// create a new slot if none exists yet.  Used to manage the
    /// Continue / Restart / Sherwood / QuickSave auto-slots.
    pub fn ensure_special_slot(&mut self, filename: &str, display_text: &str) -> usize {
        self.find_or_create_by_filename(filename, display_text)
    }

    /// Save the current engine state to the "Continue" auto-save slot.
    /// Called after every successful manual save and at mission quit.
    ///
    /// We re-serialize the live engine via `write_save_from_engine`
    /// rather than byte-copying from the just-written manual save.  In
    /// practice the engine is unchanged between the two writes so the
    /// contents are equivalent, and re-serializing shares the same
    /// write path with every other save kind (Quick/Restart/Sherwood).
    ///
    /// `game` is threaded through so the [`GamePersistentState`] tail
    /// (widget-enable flags + campaign-map display bits) survives the
    /// Continue slot — without it the next `apply_to_with_game` would
    /// see `game_persistent = None` and keep the live Game's values,
    /// which for the Continue flow is "whatever the player last did
    /// after the save", not the saved state.
    pub fn write_continue_save(
        &mut self,
        host: &mut Host,
        game: &crate::game::Game,
        engine: &Engine,
        mission_id: u32,
        profiles: Option<&ProfileManager>,
        thumbnail: Option<&Thumbnail>,
    ) -> Result<()> {
        let idx = self.ensure_special_slot(save_file::special_slots::CONTINUE, "Continue");
        self.write_save_from_engine(host, game, idx, engine, mission_id, profiles, thumbnail)?;
        self.save_index_anyhow()
    }

    /// Like [`write_continue_save`](Self::write_continue_save), but moves
    /// the expensive JSON serialization + disk write to a background
    /// thread. Used after load, where the player should regain control
    /// as soon as the save has been applied.
    pub fn write_continue_save_background(
        &mut self,
        host: &mut Host,
        game: &crate::game::Game,
        engine: &Engine,
        mission_id: u32,
        profiles: Option<&ProfileManager>,
        thumbnail: Option<&Thumbnail>,
    ) -> Result<()> {
        self.write_special_save_background(
            save_file::special_slots::CONTINUE,
            "Continue",
            "continue-save",
            "Background continue save",
            host,
            game,
            engine,
            mission_id,
            profiles,
            thumbnail,
        )
    }

    /// Save the current engine state to the "QuickSave" slot.
    /// The previous quick save (if any) is rotated to "ExQuickSave".
    pub fn write_quick_save(
        &mut self,
        host: &mut Host,
        game: &crate::game::Game,
        engine: &Engine,
        mission_id: u32,
        profiles: Option<&ProfileManager>,
        thumbnail: Option<&Thumbnail>,
    ) -> Result<()> {
        // Rotate: QuickSave → ExQuickSave
        if let Some(quick_idx) = self.find_by_filename(save_file::special_slots::QUICK)
            && self.slot_file_exists(quick_idx)
        {
            // Ensure an ExQuickSave slot exists, then copy the file.
            let ex_idx =
                self.ensure_special_slot(save_file::special_slots::EX_QUICK, "Previous Quick Save");
            self.copy_files(quick_idx, ex_idx)
                .map_err(|e| anyhow::anyhow!(e))?;
            self.copy_display_metadata(quick_idx, ex_idx)?;
        }
        let idx = self.ensure_special_slot(save_file::special_slots::QUICK, "Quick Save");
        self.write_save_from_engine(host, game, idx, engine, mission_id, profiles, thumbnail)?;
        self.save_index_anyhow()
    }

    /// Save the current engine state to the "Restart" auto-save slot.
    ///
    /// Captures the level start state so the player can restart without
    /// reloading the whole level from disk.
    pub fn write_restart_save(
        &mut self,
        host: &mut Host,
        game: &crate::game::Game,
        engine: &Engine,
        mission_id: u32,
        profiles: Option<&ProfileManager>,
        thumbnail: Option<&Thumbnail>,
    ) -> Result<()> {
        let idx = self.ensure_special_slot(save_file::special_slots::RESTART, "Restart Point");
        self.write_save_from_engine(host, game, idx, engine, mission_id, profiles, thumbnail)?;
        self.save_index_anyhow()
    }

    /// Like [`write_restart_save`](Self::write_restart_save), but captures
    /// the engine state on the calling thread and moves the expensive JSON
    /// serialization + disk write to a background thread.  Returns
    /// immediately so the game loop can start without blocking.
    pub fn write_restart_save_background(
        &mut self,
        host: &mut Host,
        game: &crate::game::Game,
        engine: &Engine,
        mission_id: u32,
        profiles: Option<&ProfileManager>,
        thumbnail: Option<&Thumbnail>,
    ) -> Result<()> {
        self.write_special_save_background(
            save_file::special_slots::RESTART,
            "Restart Point",
            "restart-save",
            "Background restart save",
            host,
            game,
            engine,
            mission_id,
            profiles,
            thumbnail,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn write_special_save_background(
        &mut self,
        filename: &str,
        display_text: &str,
        thread_name: &str,
        log_label: &'static str,
        host: &mut Host,
        game: &crate::game::Game,
        engine: &Engine,
        mission_id: u32,
        profiles: Option<&ProfileManager>,
        thumbnail: Option<&Thumbnail>,
    ) -> Result<()> {
        #[cfg(target_arch = "wasm32")]
        let _ = thread_name;

        let idx = self.ensure_special_slot(filename, display_text);
        let display_text = self.saves[idx].text.clone();
        let provenance = required_save_provenance(host, engine, mission_id, profiles)?;
        // Capture (clone) on the main thread — fast.
        let save = GameSaveFile::capture_with_game(
            engine,
            host,
            game,
            mission_id,
            display_text,
            provenance,
        )?;
        let path = self.save_path(idx);
        let thumb_data = thumbnail.cloned();
        let thumb_path = self.thumb_path(idx);

        // Eagerly update slot metadata so it's available immediately.
        self.sync_slot_metadata_from_save(idx, &save, profiles)?;
        self.save_index_anyhow()
            .with_context(|| format!("failed to index background {filename} save"))?;

        // Spawn the slow serialization + write on a background thread.
        // Wasm doesn't support threads; defer it to a queued task on the
        // main-thread executor instead, so mission startup (this call sits
        // between level load and the first gameplay frame) doesn't stall on
        // serializing the whole engine. The captured state is already
        // snapshotted above, so writing later loses nothing.
        let do_write = move || {
            tracing::info!("{log_label}: writing to {}", path.display());
            if let Err(err) = save.write_to(&path) {
                tracing::warn!("{log_label} failed: {err:#}");
            }
            if let Some(thumb) = thumb_data
                && let Err(err) = thumb.write_to(&thumb_path)
            {
                tracing::warn!("{log_label} thumbnail failed: {err:#}");
            }
            tracing::info!("{log_label} complete");
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            std::thread::Builder::new()
                .name(thread_name.into())
                .spawn(do_write)
                .with_context(|| format!("failed to spawn {thread_name} thread"))?;
        }
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(async move {
            do_write();
        });
        Ok(())
    }

    /// Whether a "Restart" auto-save snapshot exists on disk.  The
    /// debriefing UI uses this to decide whether the Restart click
    /// should queue a load or fall through to the stat panel.
    pub fn has_restart_save(&self) -> bool {
        let Some(idx) = self.find_by_filename(save_file::special_slots::RESTART) else {
            return false;
        };
        self.slot_file_exists(idx)
    }

    /// Decode the Restart auto-save without applying it. The caller must run
    /// the shared strict mission validation before choosing whether the
    /// payload can use the current mission's immutable assets.
    pub(crate) fn preflight_restart_save(&self) -> Result<Option<(usize, GameSaveFile)>> {
        let Some(idx) = self.find_by_filename(save_file::special_slots::RESTART) else {
            return Ok(None);
        };
        if !self.slot_file_exists(idx) {
            return Ok(None);
        }
        Ok(Some((idx, self.preflight_exact_slot(idx)?)))
    }

    /// Save the current engine state to the "Sherwood" checkpoint slot.
    ///
    /// Captures state when entering the Sherwood map so the campaign
    /// can be rewound one step.
    pub fn write_sherwood_save(
        &mut self,
        host: &mut Host,
        game: &crate::game::Game,
        engine: &Engine,
        mission_id: u32,
        profiles: Option<&ProfileManager>,
        thumbnail: Option<&Thumbnail>,
    ) -> Result<()> {
        let idx = self.ensure_special_slot(save_file::special_slots::SHERWOOD, "Sherwood");
        self.write_save_from_engine(host, game, idx, engine, mission_id, profiles, thumbnail)?;
        self.save_index_anyhow()
    }

    /// Find the save file to load given the user's request:
    ///
    ///   1. If the caller supplied a slot index, use only that exact slot
    ///      when its file exists.
    ///   2. Only an unspecified request may resolve the Continue auto-save.
    pub fn find_load_target(&self, explicit: Option<usize>) -> Option<usize> {
        if let Some(idx) = explicit {
            return self.slot_file_exists(idx).then_some(idx);
        }
        self.find_by_filename(save_file::special_slots::CONTINUE)
            .filter(|&i| self.slot_file_exists(i))
    }

    /// Decode and validate the selected save before constructing a
    /// destination mission Engine. Callers use its campaign, RNG state, and
    /// SimConfig for level initialization, then apply the full payload once
    /// the destination's immutable level assets are attached.
    pub(crate) fn preflight_load(
        &self,
        explicit: Option<usize>,
    ) -> Result<Option<(usize, GameSaveFile)>> {
        let Some(index) = self.find_load_target(explicit) else {
            return Ok(None);
        };
        let save = self.preflight_exact_slot(index)?;
        Ok(Some((index, save)))
    }

    /// Decode exactly the requested slot without falling back to Continue.
    /// This is used when a UI decision and the later apply must refer to the
    /// same selected file even if the directory changes concurrently.
    pub(crate) fn preflight_exact_slot(&self, index: usize) -> Result<GameSaveFile> {
        let slot = self
            .saves
            .get(index)
            .ok_or_else(|| anyhow::anyhow!("save slot index {index} is out of range"))?;
        if slot.is_autosave() {
            return crate::autosave::read_payload(&self.save_directory, &slot.filename)
                .with_context(|| {
                    format!(
                        "failed to decode exact autosave slot {index} ({})",
                        slot.filename
                    )
                });
        }
        let path = self.save_path(index);
        GameSaveFile::read_from(&path).with_context(|| {
            format!(
                "failed to decode exact save slot {index} ({})",
                slot.filename
            )
        })
    }

    fn save_index_anyhow(&self) -> Result<()> {
        self.save_index().map_err(|e| anyhow::anyhow!(e))
    }

    /// Create a new save game slot with auto-generated filename. Returns its index.
    pub fn create(&mut self, text: String, mission_id: u32) -> usize {
        let filename = self.next_filename();
        let save = SaveGame::new(filename, text, mission_id);
        self.saves.push(save);
        self.saves.len() - 1
    }

    /// Create a save with a specific filename.
    pub fn create_with_filename(
        &mut self,
        filename: String,
        text: String,
        mission_id: u32,
    ) -> usize {
        let save = SaveGame::new(filename, text, mission_id);
        self.saves.push(save);
        self.saves.len() - 1
    }

    /// Find by filename, or create if not found. Updates text either way.
    pub fn find_or_create_by_filename(&mut self, filename: &str, text: &str) -> usize {
        if let Some(idx) = self.find_by_filename(filename) {
            self.saves[idx].text = text.to_string();
            idx
        } else {
            self.create_with_filename(filename.to_string(), text.to_string(), 0)
        }
    }

    pub fn get(&self, index: usize) -> Option<&SaveGame> {
        self.saves.get(index)
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut SaveGame> {
        self.saves.get_mut(index)
    }

    pub fn slot_mission_id(&self, index: usize) -> Option<u32> {
        self.saves
            .get(index)
            .map(|save| save.mission_id)
            .filter(|&mission_id| mission_id != 0)
    }

    /// Verify that a decoded payload is still the file described by the
    /// selected `saves.json` entry. UI decisions based on cached metadata
    /// must reject a replaced file instead of inheriting the old slot's
    /// mission identity or confirmation decision.
    pub(crate) fn validate_slot_identity(&self, index: usize, save: &GameSaveFile) -> Result<()> {
        let slot = self
            .saves
            .get(index)
            .ok_or_else(|| anyhow::anyhow!("save slot index {index} is out of range"))?;
        let header = &save.header;
        if slot.mission_id != header.mission_id
            || slot.version != header.version
            || slot.timestamp != header.timestamp_unix.to_string()
        {
            anyhow::bail!(
                "save slot {index} metadata does not match decoded payload: cached mission/version/timestamp={}/{}/{:?}, decoded={}/{}/{:?}",
                slot.mission_id,
                slot.version,
                slot.timestamp,
                header.mission_id,
                header.version,
                header.timestamp_unix.to_string(),
            );
        }
        let provenance = &header.provenance;
        if slot.mission_name != provenance.mission_name
            || slot.player_profile_id != Some(provenance.player_profile_id)
            || slot.player_name != provenance.player_name
        {
            anyhow::bail!(
                "save slot {index} provenance does not match decoded payload: cached mission/player={:?}/{:?}/{:?}, decoded={:?}/{:?}/{:?}",
                slot.mission_name,
                slot.player_profile_id,
                slot.player_name,
                provenance.mission_name,
                provenance.player_profile_id,
                provenance.player_name,
            );
        }
        Ok(())
    }

    pub fn find_by_name(&self, text: &str) -> Option<usize> {
        self.saves.iter().position(|s| s.text == text)
    }

    pub fn find_by_filename(&self, filename: &str) -> Option<usize> {
        self.saves.iter().position(|s| s.filename == filename)
    }

    pub fn count(&self) -> usize {
        self.saves.len()
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.saves.len() {
            if self.saves[index].is_autosave() {
                tracing::warn!(
                    filename = self.saves[index].filename,
                    "refusing to delete an auto-managed autosave through the manual slot API"
                );
                return;
            }
            self.remove_files(index);
            self.saves.remove(index);
        }
    }

    /// Remove by filename.
    pub fn remove_by_filename(&mut self, filename: &str) {
        if let Some(idx) = self.find_by_filename(filename) {
            if self.saves[idx].is_autosave() {
                tracing::warn!(
                    filename,
                    "refusing to delete an auto-managed autosave through the filename API"
                );
                return;
            }
            self.remove_files(idx);
            self.saves.remove(idx);
        }
    }

    /// Delete save + thumbnail files from disk for the given slot.
    fn remove_files(&self, index: usize) {
        let _ = std::fs::remove_file(self.save_path(index));
        // Remove thumbnail
        let _ = std::fs::remove_file(self.thumb_path(index));
    }

    /// Sort saves by timestamp (oldest first).  The load/save menu
    /// iterates this list forward to populate its entries.
    pub fn sort_by_time(&mut self) {
        self.saves.sort_by(|a, b| {
            let a_timestamp = a.timestamp.parse::<u64>().ok();
            let b_timestamp = b.timestamp.parse::<u64>().ok();
            match (a_timestamp, b_timestamp) {
                (Some(a), Some(b)) => a.cmp(&b),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.filename.cmp(&b.filename),
            }
        });
    }

    /// Thumbnail file path.
    pub fn thumb_path(&self, index: usize) -> PathBuf {
        Path::new(&self.save_directory).join(format!("{}_thumb.png", self.saves[index].filename))
    }

    /// Full path to a save file on disk (JSON format, with `.json` extension).
    pub fn save_path(&self, index: usize) -> PathBuf {
        Path::new(&self.save_directory)
            .join(&self.saves[index].filename)
            .with_extension("json")
    }

    /// Copy save + thumbnail files from `src` slot to `dst` slot.
    ///
    /// Copies both the JSON payload (`<name>.json`) and any thumbnail.
    /// Used by the quick-save rotation to preserve the previous quick-save
    /// as ExQuickSave.
    ///
    pub fn copy_files(&mut self, src: usize, dst: usize) -> Result<(), String> {
        // JSON payload
        let src_json = self.save_path(src);
        let dst_json = self.save_path(dst);
        if src_json.exists() {
            if let Some(parent) = dst_json.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
            }
            std::fs::copy(&src_json, &dst_json).map_err(|e| format!("copy save json: {e}"))?;
        }
        // Thumbnail (used by both formats)
        let src_thumb = self.thumb_path(src);
        let dst_thumb = self.thumb_path(dst);
        if src_thumb.exists() {
            std::fs::copy(&src_thumb, &dst_thumb).map_err(|e| format!("copy thumb: {e}"))?;
        }

        Ok(())
    }

    fn copy_display_metadata(&mut self, src: usize, dst: usize) -> Result<()> {
        let src = self
            .saves
            .get(src)
            .with_context(|| format!("cannot copy metadata from missing save slot {src}"))?
            .clone();
        let dst = self
            .saves
            .get_mut(dst)
            .with_context(|| format!("cannot copy metadata to missing save slot {dst}"))?;

        dst.mission_id = src.mission_id;
        dst.version = src.version;
        dst.timestamp = src.timestamp;
        dst.mission_name = src.mission_name;
        dst.player_profile_id = src.player_profile_id;
        dst.player_name = src.player_name;
        dst.campaign_progress = src.campaign_progress;
        dst.missions_done = src.missions_done;
        dst.missions_total = src.missions_total;
        dst.gang_size = src.gang_size;
        dst.ransom = src.ransom;
        dst.blazons = src.blazons;
        dst.amulets = src.amulets;
        dst.multiplayer_diagnostic = src.multiplayer_diagnostic;
        Ok(())
    }

    /// Write a full save file (engine + campaign) to the given slot.
    ///
    /// The caller must supply the live engine; the engine must have an
    /// active campaign (panics otherwise).  If `thumbnail` is `Some`, it
    /// is also written to the sibling thumb file alongside the payload.
    #[allow(clippy::too_many_arguments)]
    pub fn write_save_from_engine(
        &mut self,
        host: &mut Host,
        game: &crate::game::Game,
        index: usize,
        engine: &Engine,
        mission_id: u32,
        profiles: Option<&ProfileManager>,
        thumbnail: Option<&Thumbnail>,
    ) -> Result<()> {
        self.write_save_from_engine_with_diagnostic(
            host, game, index, engine, mission_id, profiles, thumbnail, false,
        )
    }

    /// Write a local multiplayer diagnostic. It is deliberately tagged in
    /// both the payload and slot index and is never suitable as session state.
    #[allow(clippy::too_many_arguments)]
    pub fn write_multiplayer_diagnostic_from_engine(
        &mut self,
        host: &mut Host,
        game: &crate::game::Game,
        index: usize,
        engine: &Engine,
        mission_id: u32,
        profiles: Option<&ProfileManager>,
        thumbnail: Option<&Thumbnail>,
    ) -> Result<()> {
        self.write_save_from_engine_with_diagnostic(
            host, game, index, engine, mission_id, profiles, thumbnail, true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn write_save_from_engine_with_diagnostic(
        &mut self,
        host: &mut Host,
        game: &crate::game::Game,
        index: usize,
        engine: &Engine,
        mission_id: u32,
        profiles: Option<&ProfileManager>,
        thumbnail: Option<&Thumbnail>,
        multiplayer_diagnostic: bool,
    ) -> Result<()> {
        let display_text = self
            .saves
            .get(index)
            .with_context(|| format!("cannot write missing save slot {index}"))?
            .text
            .clone();
        let provenance = required_save_provenance(host, engine, mission_id, profiles)?;
        let mut save = GameSaveFile::capture_with_game(
            engine,
            host,
            game,
            mission_id,
            display_text,
            provenance,
        )?;
        save.header.multiplayer_diagnostic = multiplayer_diagnostic;
        let path = self.save_path(index);
        save.write_to(&path)?;

        // Write the thumbnail to its sibling file, if provided.
        if let Some(thumb) = thumbnail {
            let thumb_path = self.thumb_path(index);
            if let Err(err) = thumb.write_to(&thumb_path) {
                // Non-fatal — the save payload is already on disk.
                tracing::warn!("Failed to write thumbnail for slot {index}: {err:#}");
            }
        }

        // Sync slot metadata from the save we just wrote.
        self.sync_slot_metadata_from_save(index, &save, profiles)?;
        Ok(())
    }

    fn sync_slot_metadata_from_save(
        &mut self,
        index: usize,
        save: &GameSaveFile,
        profiles: Option<&ProfileManager>,
    ) -> Result<()> {
        let slot = self
            .saves
            .get_mut(index)
            .with_context(|| format!("cannot synchronize missing save slot {index}"))?;
        Self::sync_slot_metadata_from_header(slot, &save.header)?;
        let profiles = profiles.context("save metadata requires mission profiles")?;
        Self::sync_slot_campaign_metadata(slot, save.engine.campaign(), profiles);
        Ok(())
    }

    fn sync_slot_metadata_from_header(slot: &mut SaveGame, header: &SaveHeader) -> Result<()> {
        let provenance = &header.provenance;
        slot.mission_id = header.mission_id;
        slot.version = header.version;
        slot.timestamp = header.timestamp_unix.to_string();
        slot.multiplayer_diagnostic = header.multiplayer_diagnostic;
        slot.mission_name = provenance.mission_name.clone();
        slot.player_profile_id = Some(provenance.player_profile_id);
        slot.player_name = provenance.player_name.clone();
        Ok(())
    }

    fn sync_slot_campaign_metadata(
        slot: &mut SaveGame,
        campaign: &engine_campaign::Campaign,
        profiles: &ProfileManager,
    ) {
        slot.missions_done = Some(campaign.get_number_of_missions_done());
        slot.missions_total = Some(campaign.missions.len());
        slot.gang_size = Some(campaign.gang_indices.len());
        slot.ransom = Some(campaign.values[CampaignValue::Ransom]);
        slot.blazons = Some(campaign.values[CampaignValue::Blazon]);
        slot.amulets = Some(campaign.values[CampaignValue::Amulets]);

        slot.campaign_progress = Some(campaign.get_progression(profiles));
    }

    /// Load the thumbnail for a slot if one exists on disk.
    pub fn load_thumbnail(&self, index: usize) -> Option<Thumbnail> {
        let slot = self.saves.get(index)?;
        if slot.is_autosave() {
            return match crate::autosave::read_thumbnail(&self.save_directory, &slot.filename) {
                Ok(thumbnail) => thumbnail,
                Err(error) => {
                    tracing::warn!(
                        filename = slot.filename,
                        "failed to load autosave thumbnail: {error:#}"
                    );
                    None
                }
            };
        }
        let path = self.thumb_path(index);
        if !path.exists() {
            return None;
        }
        Thumbnail::read_from(&path).ok()
    }

    /// Load a save file and apply it to the given engine, replacing its
    /// mutable state and campaign.
    ///
    /// The caller must have already initialized the engine for the
    /// matching mission (level geometry loaded) — this function does
    /// **not** relaunch `initialize_for_mission`.
    #[cfg(test)]
    pub fn load_save_into_engine(
        &self,
        index: usize,
        engine: &mut Engine,
        host: &mut Host,
        game: &mut crate::game::Game,
        assets: &engine_api::LevelAssets,
    ) -> Result<()> {
        let path = self.save_path(index);
        let save = GameSaveFile::read_from(&path)?;
        save.apply_to_with_game(engine, host, game, assets)?;
        Ok(())
    }

    /// Does the save file on disk for this slot exist?
    pub fn slot_file_exists(&self, index: usize) -> bool {
        if let Some(slot) = self.saves.get(index)
            && slot.is_autosave()
        {
            return match crate::autosave::payload_exists(&self.save_directory, &slot.filename) {
                Ok(exists) => exists,
                Err(error) => {
                    tracing::error!(
                        filename = slot.filename,
                        "could not check autosave payload existence: {error:#}"
                    );
                    false
                }
            };
        }
        self.save_path(index).exists()
    }

    /// Replace only auto-managed slots, preserving manual and Original
    /// special slots that may have changed while the writer was active.
    pub(crate) fn replace_autosaves(&mut self, autosaves: Vec<SaveGame>) {
        assert!(
            autosaves.iter().all(SaveGame::is_autosave),
            "autosave replacement received a manual slot"
        );
        self.saves.retain(|save| !save.is_autosave());
        self.saves.extend(autosaves);
        self.sort_by_time();
    }

    /// Persist the save manager index itself (the list of saves).
    pub fn save_index(&self) -> Result<(), String> {
        let path = Path::new(&self.save_directory).join("saves.json");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| format!("serialize: {e}"))?;
        save_file::atomic_write(&path, json.as_bytes()).map_err(|e| format!("write: {e:#}"))
    }

    /// Load the save manager index from disk.
    pub fn load_index(save_directory: &str) -> Result<Self, String> {
        let path = Path::new(save_directory).join("saves.json");
        let data = std::fs::read_to_string(&path).map_err(|e| format!("read: {e}"))?;
        let manager: Self = serde_json::from_str(&data).map_err(|e| format!("parse: {e}"))?;
        for save in &manager.saves {
            save.validate_published_metadata()
                .map_err(|error| format!("validate: {error:#}"))?;
        }
        Ok(manager)
    }

    fn next_filename(&mut self) -> String {
        let name = format!("Savegame_{:03}", self.next_id);
        self.next_id += 1;
        name
    }
}

// ===================== Tests =====================
// ===================== Tests =====================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::Game;
    use crate::host::ApplicationContext;
    use crate::key_config_store::KeyConfigStore;
    use crate::save_file::special_slots;
    use robin_engine::campaign::Campaign;
    use robin_engine::mission::Mission;
    use robin_engine::player_profile::{DifficultyLevel, PlayerProfileManager};
    use robin_engine::profiles::{MissionProfile, ProfileManager};

    fn fresh_engine() -> (Engine, engine_api::LevelAssets) {
        let mut assets = engine_api::LevelAssets::new();
        let engine =
            Engine::new_for_test(800.0, 600.0, Campaign::default(), &mut assets).expect("engine");
        (engine, assets)
    }

    fn fresh_save_session(
        player_name: &str,
    ) -> (Engine, engine_api::LevelAssets, ProfileManager, Host) {
        let mut profiles = ProfileManager::default();
        let mut campaign = Campaign::default();
        for mission_id in [1, 3, 17] {
            let profile_idx = profiles.missions.len() as u32;
            profiles.missions.push(MissionProfile {
                id: mission_id,
                mission_filename: format!("Mission_{mission_id}"),
                mission_name: format!("Mission {mission_id}"),
                ..MissionProfile::default()
            });
            campaign.missions.push(Mission {
                profile_idx: Some(profile_idx),
                ..Mission::default()
            });
        }
        let mut assets = engine_api::LevelAssets::new();
        let engine = Engine::new_for_test(800.0, 600.0, campaign, &mut assets).expect("engine");

        let save_root = format!("/tmp/save-metadata-{player_name}");
        let mut players = PlayerProfileManager::new(save_root.clone());
        let player = players.create_profile(player_name.to_string(), DifficultyLevel::Medium);
        players.set_active(player);
        let application_context = ApplicationContext::complete(
            engine_api::GlobalOptions::default(),
            players,
            KeyConfigStore::new(save_root),
            None,
        )
        .expect("complete test application context");
        let host = Host::new(application_context, 800.0, 600.0);
        (engine, assets, profiles, host)
    }

    #[test]
    fn create_and_find() {
        let mut mgr = SaveGameManager::new("/tmp/test_saves".into());
        let idx = mgr.create("My Save".into(), 42);
        assert_eq!(idx, 0);
        assert_eq!(mgr.count(), 1);
        assert_eq!(mgr.get(0).unwrap().text, "My Save");
        assert_eq!(mgr.get(0).unwrap().mission_id, 42);
        assert_eq!(mgr.get(0).unwrap().filename, "Savegame_000");
        assert_eq!(mgr.find_by_name("My Save"), Some(0));
        assert_eq!(mgr.find_by_name("Nope"), None);
    }

    #[test]
    fn display_metadata_copy_rejects_missing_slots() {
        let mut mgr = SaveGameManager::new("/tmp/test_saves".into());
        let slot = mgr.create("My Save".into(), 42);

        let missing_source = mgr.copy_display_metadata(usize::MAX, slot).unwrap_err();
        assert!(
            missing_source
                .to_string()
                .contains("cannot copy metadata from missing save slot")
        );

        let missing_destination = mgr.copy_display_metadata(slot, usize::MAX).unwrap_err();
        assert!(
            missing_destination
                .to_string()
                .contains("cannot copy metadata to missing save slot")
        );
    }

    #[test]
    fn special_slots() {
        let save = SaveGame::new("Continue".into(), "Continue".into(), 0);
        assert!(save.is_special());
        assert!(save.is_continue());
        assert!(!save.is_restart());
    }

    #[test]
    fn special_auto_detect() {
        let save = SaveGame::new("Restart".into(), "Restart Save".into(), 0);
        assert!(save.is_special());
        assert!(save.is_restart());
        assert!(!save.is_continue());
    }

    #[test]
    fn non_special_filename() {
        let save = SaveGame::new("Savegame_005".into(), "My Save".into(), 0);
        assert!(!save.is_special());
        assert_eq!(save.version, save_file::SAVE_FORMAT_VERSION);
    }

    #[test]
    fn autosave_storage_names_are_strict_and_path_safe() {
        for valid in ["Autosave_1_0000", "Autosave_18446744073709551615_9999"] {
            assert!(is_generated_autosave_filename(valid), "{valid}");
            assert_eq!(
                SpecialSlot::from_filename(valid),
                Some(SpecialSlot::Autosave)
            );
        }
        for invalid in [
            "Autosave_1_999",
            "Autosave_1_0000.json",
            "Autosave_../0000",
            "Autosave_1_../../Continue",
            "Autosave_notes",
            "autosave_1_0000",
        ] {
            assert!(!is_generated_autosave_filename(invalid), "{invalid}");
            assert_eq!(SpecialSlot::from_filename(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn find_or_create() {
        let mut mgr = SaveGameManager::new("/tmp/test_saves".into());
        let idx1 = mgr.find_or_create_by_filename("Continue", "Continue 1");
        assert_eq!(idx1, 0);
        assert_eq!(mgr.count(), 1);
        assert!(mgr.get(0).unwrap().is_continue());

        // Same filename → updates text, same index
        let idx2 = mgr.find_or_create_by_filename("Continue", "Continue 2");
        assert_eq!(idx2, 0);
        assert_eq!(mgr.count(), 1);
        assert_eq!(mgr.get(0).unwrap().text, "Continue 2");
    }

    #[test]
    fn serde_round_trip() {
        let mut mgr = SaveGameManager::new("/tmp/test_saves".into());
        mgr.create("Save 1".into(), 10);
        mgr.create("Save 2".into(), 20);

        let json = serde_json::to_string(&mgr).unwrap();
        let mgr2: SaveGameManager = serde_json::from_str(&json).unwrap();
        assert_eq!(mgr2.count(), 2);
        assert_eq!(mgr2.saves[0].text, "Save 1");
        assert_eq!(mgr2.saves[1].mission_id, 20);
    }

    #[test]
    fn auto_incrementing_filenames() {
        let mut mgr = SaveGameManager::new("/tmp/test_saves".into());
        mgr.create("A".into(), 1);
        mgr.create("B".into(), 2);
        mgr.create("C".into(), 3);
        assert_eq!(mgr.saves[0].filename, "Savegame_000");
        assert_eq!(mgr.saves[1].filename, "Savegame_001");
        assert_eq!(mgr.saves[2].filename, "Savegame_002");
    }

    #[test]
    fn full_and_thumb_paths() {
        let mut mgr = SaveGameManager::new("/saves/profile_1".into());
        mgr.create_with_filename("Continue".into(), "Continue".into(), 5);
        assert_eq!(
            mgr.save_path(0),
            PathBuf::from("/saves/profile_1/Continue.json")
        );
        assert_eq!(
            mgr.thumb_path(0),
            PathBuf::from("/saves/profile_1/Continue_thumb.png")
        );
    }

    #[test]
    fn engine_round_trip_via_manager() {
        use tempfile::tempdir;

        let tmp = tempdir().unwrap();
        let mut mgr = SaveGameManager::new(tmp.path().to_string_lossy().into_owned());

        // Build a live engine with some distinctive state.
        let (mut engine, assets, mut profiles, mut host) = fresh_save_session("Alice");
        let game = Game::default();
        engine.test_set_frame_counter(42);
        engine.test_set_engine_scalars(0xAA55_AA55, 2.0, 0, false, false, Vec::new());

        // Write to a manual slot.
        let idx = mgr.create("Slot A".into(), 17);
        mgr.write_save_from_engine(&mut host, &game, idx, &engine, 17, Some(&profiles), None)
            .unwrap();
        assert!(mgr.slot_file_exists(idx));
        assert_eq!(mgr.slot_mission_id(idx), Some(17));
        let decoded = mgr.preflight_exact_slot(idx).unwrap();
        mgr.validate_slot_identity(idx, &decoded).unwrap();
        assert_eq!(
            decoded.header.provenance,
            SaveProvenance::new("Mission 17".into(), 0, "Alice".into()).unwrap()
        );
        assert_eq!(mgr.saves[idx].mission_name, "Mission 17");
        assert_eq!(mgr.saves[idx].player_profile_id, Some(0));
        assert_eq!(mgr.saves[idx].player_name, "Alice");
        profiles.missions[2].mission_name = "Mission 17 (renamed)".into();
        assert_eq!(mgr.saves[idx].mission_name, "Mission 17");
        assert_eq!(decoded.header.provenance.mission_name, "Mission 17");
        mgr.saves[idx].mission_id = 99;
        assert!(
            mgr.validate_slot_identity(idx, &decoded)
                .unwrap_err()
                .to_string()
                .contains("metadata does not match decoded payload")
        );
        mgr.saves[idx].mission_id = 17;
        mgr.saves[idx].player_name = "Mallory".into();
        assert!(
            mgr.validate_slot_identity(idx, &decoded)
                .unwrap_err()
                .to_string()
                .contains("provenance does not match")
        );
        mgr.saves[idx].player_name = "Alice".into();

        host.application_context
            .with_player_profiles_mut(|players| {
                players.get_active_mut().unwrap().name = "Renamed Alice".into();
            })
            .unwrap();

        // Write a Continue auto-save.
        mgr.write_continue_save(&mut host, &game, &engine, 17, Some(&profiles), None)
            .unwrap();
        let continue_idx = mgr
            .find_by_filename(special_slots::CONTINUE)
            .expect("continue slot should exist");
        assert!(mgr.slot_file_exists(continue_idx));
        assert_eq!(mgr.saves[idx].player_name, "Alice");
        assert_eq!(mgr.saves[continue_idx].player_name, "Renamed Alice");
        assert_eq!(mgr.saves[continue_idx].mission_name, "Mission 17 (renamed)");

        // find_load_target should prefer the explicit slot when supplied,
        // otherwise fall back to Continue.
        assert_eq!(mgr.find_load_target(Some(idx)), Some(idx));
        assert_eq!(mgr.find_load_target(None), Some(continue_idx));

        // Load into a fresh engine.
        let mut engine2 = fresh_engine().0;
        let mut host2 = Host::scratch(800.0, 600.0);
        let mut game2 = Game::default();
        mgr.load_save_into_engine(idx, &mut engine2, &mut host2, &mut game2, &assets)
            .unwrap();
        assert_eq!(engine2.frame_counter(), 42);
    }

    #[test]
    fn multiplayer_diagnostic_tag_is_written_to_payload_and_index() {
        let tmp = tempfile::tempdir().unwrap();
        let mut manager = SaveGameManager::new(tmp.path().to_string_lossy().into_owned());
        let (engine, _assets, profiles, mut host) = fresh_save_session("Alice");
        let game = Game::default();
        let slot = manager.create("Network diagnostic".into(), 17);

        manager
            .write_multiplayer_diagnostic_from_engine(
                &mut host,
                &game,
                slot,
                &engine,
                17,
                Some(&profiles),
                None,
            )
            .unwrap();
        assert!(manager.get(slot).unwrap().multiplayer_diagnostic);
        assert!(
            manager
                .preflight_exact_slot(slot)
                .unwrap()
                .header
                .multiplayer_diagnostic
        );

        manager
            .write_save_from_engine(&mut host, &game, slot, &engine, 17, Some(&profiles), None)
            .unwrap();
        assert!(!manager.get(slot).unwrap().multiplayer_diagnostic);
        assert!(
            !manager
                .preflight_exact_slot(slot)
                .unwrap()
                .header
                .multiplayer_diagnostic
        );
    }

    #[test]
    fn missing_explicit_slot_never_falls_back_to_continue() {
        let tmp = tempfile::tempdir().unwrap();
        let mut mgr = SaveGameManager::new(tmp.path().to_string_lossy().into_owned());
        let (engine, _assets, profiles, mut host) = fresh_save_session("Alice");
        let game = Game::default();
        mgr.write_continue_save(&mut host, &game, &engine, 1, Some(&profiles), None)
            .unwrap();
        let missing = mgr.create("Missing explicit slot".into(), 1);

        assert_eq!(mgr.find_load_target(Some(missing)), None);
        assert!(mgr.preflight_load(Some(missing)).unwrap().is_none());
        assert_eq!(
            mgr.find_load_target(None),
            mgr.find_by_filename(special_slots::CONTINUE)
        );
    }

    #[test]
    fn quick_save_rotates_previous() {
        use tempfile::tempdir;

        let tmp = tempdir().unwrap();
        let mut mgr = SaveGameManager::new(tmp.path().to_string_lossy().into_owned());

        let (mut engine, assets, profiles, mut host) = fresh_save_session("Alice");
        let game = Game::default();

        engine.test_set_frame_counter(1);
        mgr.write_quick_save(&mut host, &game, &engine, 3, Some(&profiles), None)
            .unwrap();
        engine.test_set_frame_counter(2);
        mgr.write_quick_save(&mut host, &game, &engine, 3, Some(&profiles), None)
            .unwrap();

        let quick_idx = mgr.find_by_filename(special_slots::QUICK).unwrap();
        let ex_idx = mgr.find_by_filename(special_slots::EX_QUICK).unwrap();
        assert!(mgr.slot_file_exists(quick_idx));
        assert!(mgr.slot_file_exists(ex_idx));

        let mut engine_q = fresh_engine().0;
        let mut host_q = Host::scratch(800.0, 600.0);
        let mut game_q = Game::default();
        mgr.load_save_into_engine(quick_idx, &mut engine_q, &mut host_q, &mut game_q, &assets)
            .unwrap();
        assert_eq!(engine_q.frame_counter(), 2);

        let mut engine_e = fresh_engine().0;
        let mut host_e = Host::scratch(800.0, 600.0);
        let mut game_e = Game::default();
        mgr.load_save_into_engine(ex_idx, &mut engine_e, &mut host_e, &mut game_e, &assets)
            .unwrap();
        assert_eq!(engine_e.frame_counter(), 1);
        assert_eq!(mgr.saves[quick_idx].player_name, "Alice");
        assert_eq!(mgr.saves[ex_idx].player_name, "Alice");
    }

    #[test]
    fn save_write_rejects_missing_profiles_or_active_player() {
        let tmp = tempfile::tempdir().unwrap();
        let mut mgr = SaveGameManager::new(tmp.path().to_string_lossy().into_owned());
        let (engine, _assets, profiles, _host) = fresh_save_session("Alice");
        let mut scratch_host = Host::scratch(800.0, 600.0);
        let game = Game::default();
        let slot = mgr.create("Strict metadata".into(), 1);

        let missing_profiles = mgr
            .write_save_from_engine(&mut scratch_host, &game, slot, &engine, 1, None, None)
            .unwrap_err();
        assert!(
            format!("{missing_profiles:#}").contains("active mission profile table"),
            "{missing_profiles:#}"
        );

        let missing_player = mgr
            .write_save_from_engine(
                &mut scratch_host,
                &game,
                slot,
                &engine,
                1,
                Some(&profiles),
                None,
            )
            .unwrap_err();
        assert!(
            format!("{missing_player:#}").contains("active player profile"),
            "{missing_player:#}"
        );

        let (_engine, _assets, profiles, mut host) = fresh_save_session("Alice");
        let missing_slot = mgr
            .write_save_from_engine(
                &mut host,
                &game,
                usize::MAX,
                &engine,
                1,
                Some(&profiles),
                None,
            )
            .unwrap_err();
        assert!(
            format!("{missing_slot:#}").contains("missing save slot"),
            "{missing_slot:#}"
        );
    }

    #[test]
    fn timestamp_sort_is_numeric_and_puts_invalid_legacy_values_last() {
        let mut mgr = SaveGameManager::new("/tmp/test_saves".into());
        for (name, timestamp) in [("Ten", "10"), ("Two", "2"), ("Legacy", "")] {
            let slot = mgr.create(name.into(), 1);
            mgr.saves[slot].timestamp = timestamp.into();
        }
        mgr.sort_by_time();
        assert_eq!(
            mgr.saves
                .iter()
                .map(|save| save.text.as_str())
                .collect::<Vec<_>>(),
            ["Two", "Ten", "Legacy"]
        );
    }

    #[test]
    fn native_index_without_player_metadata_is_rejected() {
        let json = serde_json::json!({
            "saves": [{
                "text": "Legacy",
                "filename": "Savegame_000",
                "mission_id": 1,
                "version": save_file::SAVE_FORMAT_VERSION,
                "timestamp": "123",
                "special": null,
                "mission_name": "Mission 1"
            }],
            "save_directory": "/tmp/test_saves",
            "next_id": 1
        });
        let error = serde_json::from_value::<SaveGameManager>(json).unwrap_err();
        assert!(error.to_string().contains("missing field"));
    }

    #[test]
    fn per_profile_save_managers_are_isolated() {
        // Gap 1 test: two profiles using the same root save dir should
        // each get their own `Profile_NNN/` subdirectory so their slot
        // lists never collide.
        use crate::save_file::{save_directory_for_profile, special_slots};
        use tempfile::tempdir;

        let root = tempdir().unwrap();

        // Build two per-profile managers rooted at Profile_000 / Profile_001
        // (independent of the global PlayerProfileManager to keep the test
        // hermetic).
        let p0_dir = root.path().join("Profile_000");
        let p1_dir = root.path().join("Profile_001");
        // Matches the `Profile_NNN` layout `save_directory_for_profile` uses.
        assert!(save_directory_for_profile(0).ends_with("Profile_000"));
        assert!(save_directory_for_profile(42).ends_with("Profile_042"));
        let mut mgr0 = SaveGameManager::new(p0_dir.to_string_lossy().into_owned());
        let mut mgr1 = SaveGameManager::new(p1_dir.to_string_lossy().into_owned());

        let (mut engine, assets, profiles, mut host) = fresh_save_session("Alice");
        let game = Game::default();

        // Profile 0 saves frame=100 into QuickSave.
        engine.test_set_frame_counter(100);
        mgr0.write_quick_save(&mut host, &game, &engine, 1, Some(&profiles), None)
            .unwrap();
        let q0 = mgr0.find_by_filename(special_slots::QUICK).unwrap();
        let path0 = mgr0.save_path(q0);
        assert!(
            path0.starts_with(&p0_dir),
            "p0 save must be under Profile_000"
        );

        // Profile 1 saves frame=200 into its own QuickSave.
        engine.test_set_frame_counter(200);
        mgr1.write_quick_save(&mut host, &game, &engine, 1, Some(&profiles), None)
            .unwrap();
        let q1 = mgr1.find_by_filename(special_slots::QUICK).unwrap();
        let path1 = mgr1.save_path(q1);
        assert!(
            path1.starts_with(&p1_dir),
            "p1 save must be under Profile_001"
        );
        assert_ne!(path0, path1, "profiles must use distinct save files");

        // Each profile loads its own snapshot back independently.
        let mut engine_a = fresh_engine().0;
        let mut host_a = Host::scratch(800.0, 600.0);
        let mut game_a = Game::default();
        mgr0.load_save_into_engine(q0, &mut engine_a, &mut host_a, &mut game_a, &assets)
            .unwrap();
        assert_eq!(engine_a.frame_counter(), 100);

        let mut engine_b = fresh_engine().0;
        let mut host_b = Host::scratch(800.0, 600.0);
        let mut game_b = Game::default();
        mgr1.load_save_into_engine(q1, &mut engine_b, &mut host_b, &mut game_b, &assets)
            .unwrap();
        assert_eq!(engine_b.frame_counter(), 200);
    }

    #[test]
    fn remove_by_filename() {
        let mut mgr = SaveGameManager::new("/tmp/test_saves".into());
        mgr.create("A".into(), 1);
        mgr.create_with_filename("Continue".into(), "Continue".into(), 0);
        assert_eq!(mgr.count(), 2);
        mgr.remove_by_filename("Continue");
        assert_eq!(mgr.count(), 1);
        assert_eq!(mgr.saves[0].filename, "Savegame_000");
    }

    #[test]
    fn manual_remove_apis_refuse_autosaves() {
        let mut mgr = SaveGameManager::new("/tmp/test_saves".into());
        mgr.saves
            .push(SaveGame::new("Autosave_1_0000".into(), "Mission".into(), 1));
        mgr.remove(0);
        mgr.remove_by_filename("Autosave_1_0000");
        assert_eq!(mgr.count(), 1);
    }
}
