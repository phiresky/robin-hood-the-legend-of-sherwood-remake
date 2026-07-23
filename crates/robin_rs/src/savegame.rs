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

use crate::save_file::{self, GameSaveFile, SaveHeader, Thumbnail};

/// Metadata for a single save game slot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveGame {
    /// Display name shown in the UI (UTF-8).
    pub text: String,
    /// Base filename (without directory or extension).
    pub filename: String,
    /// Mission profile ID at time of save.
    pub mission_id: u32,
    /// Save file version.
    pub version: u32,
    /// Timestamp (ISO 8601 string).
    pub timestamp: String,
    /// Whether this is a special slot (continue, quicksave, restart, sherwood).
    pub special: Option<SpecialSlot>,
    /// Localized/static mission name at time of save, when profile data was available.
    #[serde(default)]
    pub mission_name: String,
    /// Campaign progression percentage at time of save.
    #[serde(default)]
    pub campaign_progress: Option<u32>,
    /// Number of completed missions at time of save.
    #[serde(default)]
    pub missions_done: Option<usize>,
    /// Total missions known to the campaign at time of save.
    #[serde(default)]
    pub missions_total: Option<usize>,
    /// Gang size at time of save.
    #[serde(default)]
    pub gang_size: Option<usize>,
    /// Current ransom value at time of save.
    #[serde(default)]
    pub ransom: Option<i32>,
    /// Current blazon value at time of save.
    #[serde(default)]
    pub blazons: Option<i32>,
    /// Current amulet value at time of save.
    #[serde(default)]
    pub amulets: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpecialSlot {
    Continue,
    QuickSave,
    ExQuickSave,
    Restart,
    Sherwood,
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
            _ => None,
        }
    }
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
            campaign_progress: None,
            missions_done: None,
            missions_total: None,
            gang_size: None,
            ransom: None,
            blazons: None,
            amulets: None,
        }
    }

    pub fn is_special(&self) -> bool {
        self.special.is_some()
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
        match Self::load_index(&dir_str) {
            Ok(mgr) => mgr,
            Err(err) => {
                tracing::info!("No save index at {dir_str} ({err}) - starting fresh");
                Self::new(dir_str)
            }
        }
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
    ) {
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
        );
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
            self.copy_display_metadata(quick_idx, ex_idx);
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
    ) {
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
        );
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
    ) {
        let idx = self.ensure_special_slot(filename, display_text);
        let display_text = self.saves[idx].text.clone();
        // Capture (clone) on the main thread — fast.
        let save = GameSaveFile::capture_with_game(engine, host, game, mission_id, display_text);
        let path = self.save_path(idx);
        let thumb_data = thumbnail.cloned();
        let thumb_path = self.thumb_path(idx);

        // Eagerly update slot metadata so it's available immediately.
        self.sync_slot_metadata_from_save(idx, &save, profiles);
        if let Err(e) = self.save_index_anyhow() {
            tracing::warn!("Failed to save index for restart slot: {e:#}");
        }

        // Spawn the slow serialization + write on a background thread.
        // Wasm doesn't support threads, so run inline there — slower
        // mid-mission stall, but no other path until we either offload
        // to a Web Worker or move the write off the critical path.
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
                .expect("failed to spawn background save thread");
        }
        #[cfg(target_arch = "wasm32")]
        do_write();
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
            self.remove_files(index);
            self.saves.remove(index);
        }
    }

    /// Remove by filename.
    pub fn remove_by_filename(&mut self, filename: &str) {
        if let Some(idx) = self.find_by_filename(filename) {
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
        self.saves.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
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

    fn copy_display_metadata(&mut self, src: usize, dst: usize) {
        let Some(src) = self.saves.get(src).cloned() else {
            return;
        };
        let Some(dst) = self.saves.get_mut(dst) else {
            return;
        };

        dst.mission_id = src.mission_id;
        dst.version = src.version;
        dst.timestamp = src.timestamp;
        dst.mission_name = src.mission_name;
        dst.campaign_progress = src.campaign_progress;
        dst.missions_done = src.missions_done;
        dst.missions_total = src.missions_total;
        dst.gang_size = src.gang_size;
        dst.ransom = src.ransom;
        dst.blazons = src.blazons;
        dst.amulets = src.amulets;
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
        let display_text = self.saves[index].text.clone();
        let save = GameSaveFile::capture_with_game(engine, host, game, mission_id, display_text);
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
        self.sync_slot_metadata_from_save(index, &save, profiles);
        Ok(())
    }

    fn sync_slot_metadata_from_save(
        &mut self,
        index: usize,
        save: &GameSaveFile,
        profiles: Option<&ProfileManager>,
    ) {
        let slot = &mut self.saves[index];
        Self::sync_slot_metadata_from_header(slot, &save.header);
        Self::sync_slot_campaign_metadata(
            slot,
            save.engine.campaign(),
            save.header.mission_id,
            profiles,
        );
    }

    fn sync_slot_metadata_from_header(slot: &mut SaveGame, header: &SaveHeader) {
        slot.mission_id = header.mission_id;
        slot.version = header.version;
        slot.timestamp = header.timestamp_unix.to_string();
    }

    fn sync_slot_campaign_metadata(
        slot: &mut SaveGame,
        campaign: &engine_campaign::Campaign,
        mission_id: u32,
        profiles: Option<&ProfileManager>,
    ) {
        slot.missions_done = Some(campaign.get_number_of_missions_done());
        slot.missions_total = Some(campaign.missions.len());
        slot.gang_size = Some(campaign.gang_indices.len());
        slot.ransom = Some(campaign.values[CampaignValue::Ransom]);
        slot.blazons = Some(campaign.values[CampaignValue::Blazon]);
        slot.amulets = Some(campaign.values[CampaignValue::Amulets]);

        if let Some(profiles) = profiles {
            slot.campaign_progress = Some(campaign.get_progression(profiles));
            if let Some(mission) = campaign.get_mission(mission_id, profiles) {
                slot.mission_name = mission.profile(profiles).mission_name.clone();
            }
        }
    }

    /// Load the thumbnail for a slot if one exists on disk.
    pub fn load_thumbnail(&self, index: usize) -> Option<Thumbnail> {
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
        self.save_path(index).exists()
    }

    /// Persist the save manager index itself (the list of saves).
    pub fn save_index(&self) -> Result<(), String> {
        let path = Path::new(&self.save_directory).join("saves.json");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| format!("serialize: {e}"))?;
        std::fs::write(&path, json).map_err(|e| format!("write: {e}"))
    }

    /// Load the save manager index from disk.
    pub fn load_index(save_directory: &str) -> Result<Self, String> {
        let path = Path::new(save_directory).join("saves.json");
        let data = std::fs::read_to_string(&path).map_err(|e| format!("read: {e}"))?;
        serde_json::from_str(&data).map_err(|e| format!("parse: {e}"))
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
    use crate::save_file::special_slots;
    use robin_engine::campaign::Campaign;

    fn fresh_engine() -> (Engine, engine_api::LevelAssets) {
        let mut assets = engine_api::LevelAssets::new();
        let engine =
            Engine::new_for_test(800.0, 600.0, Campaign::default(), &mut assets).expect("engine");
        (engine, assets)
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
        let (mut engine, assets) = fresh_engine();
        let mut host = Host::scratch(800.0, 600.0);
        let game = Game::default();
        engine.test_set_frame_counter(42);
        engine.test_set_engine_scalars(0xAA55_AA55, 2.0, 0, false, false, Vec::new());

        // Write to a manual slot.
        let idx = mgr.create("Slot A".into(), 17);
        mgr.write_save_from_engine(&mut host, &game, idx, &engine, 17, None, None)
            .unwrap();
        assert!(mgr.slot_file_exists(idx));
        assert_eq!(mgr.slot_mission_id(idx), Some(17));
        let decoded = mgr.preflight_exact_slot(idx).unwrap();
        mgr.validate_slot_identity(idx, &decoded).unwrap();
        mgr.saves[idx].mission_id = 99;
        assert!(
            mgr.validate_slot_identity(idx, &decoded)
                .unwrap_err()
                .to_string()
                .contains("metadata does not match decoded payload")
        );
        mgr.saves[idx].mission_id = 17;

        // Write a Continue auto-save.
        mgr.write_continue_save(&mut host, &game, &engine, 17, None, None)
            .unwrap();
        let continue_idx = mgr
            .find_by_filename(special_slots::CONTINUE)
            .expect("continue slot should exist");
        assert!(mgr.slot_file_exists(continue_idx));

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
    fn missing_explicit_slot_never_falls_back_to_continue() {
        let tmp = tempfile::tempdir().unwrap();
        let mut mgr = SaveGameManager::new(tmp.path().to_string_lossy().into_owned());
        let (engine, _assets) = fresh_engine();
        let mut host = Host::scratch(800.0, 600.0);
        let game = Game::default();
        mgr.write_continue_save(&mut host, &game, &engine, 1, None, None)
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

        let (mut engine, assets) = fresh_engine();
        let mut host = Host::scratch(800.0, 600.0);
        let game = Game::default();

        engine.test_set_frame_counter(1);
        mgr.write_quick_save(&mut host, &game, &engine, 3, None, None)
            .unwrap();
        engine.test_set_frame_counter(2);
        mgr.write_quick_save(&mut host, &game, &engine, 3, None, None)
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

        let (mut engine, assets) = fresh_engine();
        let mut host = Host::scratch(800.0, 600.0);
        let game = Game::default();

        // Profile 0 saves frame=100 into QuickSave.
        engine.test_set_frame_counter(100);
        mgr0.write_quick_save(&mut host, &game, &engine, 1, None, None)
            .unwrap();
        let q0 = mgr0.find_by_filename(special_slots::QUICK).unwrap();
        let path0 = mgr0.save_path(q0);
        assert!(
            path0.starts_with(&p0_dir),
            "p0 save must be under Profile_000"
        );

        // Profile 1 saves frame=200 into its own QuickSave.
        engine.test_set_frame_counter(200);
        mgr1.write_quick_save(&mut host, &game, &engine, 1, None, None)
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
}
