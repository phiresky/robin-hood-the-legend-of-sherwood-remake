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
//! defined in [`save_file::special_slots`]. Browser Restart is an in-memory
//! checkpoint owned by the running mission; durable browser autosaves have
//! their separate storage backend in [`crate::autosave`].

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
use sha2::{Digest, Sha256};

use crate::save_file::{
    self, GameSaveFile, PreparedGameSave, ReplaySaveIdentity, SaveHeader, SaveProvenance, Thumbnail,
};

mod catalog;
mod persistence;
mod recovery;
use catalog::SlotCatalog;

/// A portable basename, never a path. Deserialization applies the same checks
/// as runtime construction so persisted identities cannot escape their store.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SlotName(String);

impl TryFrom<String> for SlotName {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        if value.is_empty()
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
            || matches!(
                value.to_ascii_lowercase().as_str(),
                "saves"
                    | "autosaves"
                    | "quick-save-recovery"
                    | "save-delete-recovery"
                    | "owned-save-recovery"
                    | "con"
                    | "prn"
                    | "aux"
                    | "nul"
            )
            || (value.len() == 4
                && (value[..3].eq_ignore_ascii_case("com")
                    || value[..3].eq_ignore_ascii_case("lpt"))
                && matches!(value.as_bytes()[3], b'1'..=b'9'))
        {
            return Err(format!("invalid save slot basename {value:?}"));
        }
        Ok(Self(value))
    }
}

impl From<SlotName> for String {
    fn from(value: SlotName) -> Self {
        value.0
    }
}

impl SlotName {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        Self::try_from(value.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

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
        SlotName::try_from(self.filename.clone()).map_err(anyhow::Error::msg)?;
        anyhow::ensure!(
            self.special == SpecialSlot::from_filename(&self.filename),
            "save slot special kind disagrees with filename"
        );
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
        .application_context()
        .active_profile_snapshot()
        .map_err(anyhow::Error::msg)
        .context("save requires an active player profile")?;
    SaveProvenance::new(mission_name, player.id, player.name)
}

/// Manages a collection of save games for a player profile.
// Runtime directory authority must never be reconstructed by serde.
#[derive(Debug)]
pub struct SaveGameManager {
    catalog: SlotCatalog,
    operations: crate::save_operation::SaveOperationOwner,
    operation_error: Option<String>,
    operation_error_reported: bool,
    save_directory: String,
    next_id: u32,
    /// Browser Restart is a session checkpoint, not a durable/manual save.
    /// Its metadata and payload are published together after successful capture.
    session_restart: Option<std::sync::Arc<PreparedGameSave>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SlotState {
    Draft,
    Published,
    Session,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SaveWriteStatus {
    Queued,
    Completed,
}

/// Evidence returned only after payload, index and receipt retirement succeed.
/// Serialization is diagnostic only; deserialization cannot grant commit proof.
#[derive(Debug, Clone, Serialize)]
pub struct CommittedSave {
    slot: SlotHandle,
    digest: [u8; 32],
}

impl<'de> Deserialize<'de> for CommittedSave {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> std::result::Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "save commit evidence is process-local",
        ))
    }
}

impl CommittedSave {
    pub fn slot(&self) -> &SlotHandle {
        &self.slot
    }
    pub fn digest(&self) -> &[u8; 32] {
        &self.digest
    }
}

/// Stable in-process selection. Serialized data cannot restore owner authority:
/// decoded handles have owner/generation zero and are rejected by every manager.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotHandle {
    name: SlotName,
    #[serde(skip)]
    owner_id: u64,
    #[serde(skip)]
    generation: u64,
}

impl SlotHandle {
    pub fn name(&self) -> &SlotName {
        &self.name
    }
}

fn next_store_owner() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.try_update(
        std::sync::atomic::Ordering::Relaxed,
        std::sync::atomic::Ordering::Relaxed,
        |value| value.checked_add(1),
    )
    .expect("save owner identity space exhausted")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SaveIndex {
    saves: Vec<SaveGame>,
    next_id: u32,
    /// Compatibility metadata only; the caller always supplies runtime authority.
    #[serde(default)]
    save_directory: String,
}

/// Durable intent: once published, opening the store finishes this deletion.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeleteRecovery {
    filename: SlotName,
}

/// Recovery metadata is published before either payload. Digests bind each
/// prospective index entry to the exact payload that actually reached disk.
#[derive(Serialize, Deserialize)]
struct QuickSaveRecovery {
    slots: Vec<(SaveGame, [u8; 32])>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SpecialSaveRecovery {
    slot: SaveGame,
    digest: [u8; 32],
}

impl SaveGameManager {
    pub fn new(save_directory: String) -> Self {
        SaveGameManager {
            catalog: SlotCatalog::default(),
            operations: Default::default(),
            operation_error: None,
            operation_error_reported: false,
            save_directory,
            next_id: 0,
            session_restart: None,
        }
    }

    /// Create a manager rooted at the active profile's save subdirectory
    /// (`<root>/Profile_NNN/`). Loads the existing slot index from
    /// `saves.json` if present; otherwise starts empty.
    pub fn open_for_context(
        application_context: &crate::host::ApplicationContext,
    ) -> Result<Self, String> {
        let dir = application_context
            .active_profile_save_directory()
            .map_err(|error| format!("save manager requires an active profile: {error}"))?;
        let dir_str = dir.to_string_lossy().into_owned();
        #[cfg(not(target_arch = "wasm32"))]
        let mut manager = Self::load_index(&dir_str).map_err(|error| {
            format!("save store {dir_str} needs recovery; existing files preserved: {error}")
        })?;
        // The browser has no desktop manual-index backend. Select its known
        // memory-owned store explicitly; only autosaves use localStorage.
        // This is not a fallback from a corrupt or unreadable persisted index.
        #[cfg(target_arch = "wasm32")]
        let mut manager = Self::new(dir_str);
        crate::autosave::load_into_manager(&mut manager)
            .map_err(|error| format!("load autosave manifest: {error:#}"))?;
        Ok(manager)
    }

    pub fn save_directory(&self) -> &str {
        &self.save_directory
    }

    pub fn saves(&self) -> impl ExactSizeIterator<Item = &SaveGame> + DoubleEndedIterator {
        self.catalog.iter()
    }

    pub fn slot_handle(&self, index: usize) -> Result<SlotHandle> {
        self.catalog.handle(index)
    }

    pub fn resolve_handle(&self, handle: &SlotHandle) -> Result<usize> {
        self.catalog.resolve(handle)
    }

    pub fn slot_state(&self, name: &SlotName) -> Result<SlotState> {
        self.catalog.state(name)
    }

    pub fn rename_slot(&mut self, handle: &SlotHandle, text: String) -> Result<()> {
        self.finish_background()?;
        let index = self.resolve_handle(handle)?;
        self.catalog.rename(index, text)
    }

    /// Polling never blocks on serialization. Errors remain sticky until the
    /// caller explicitly reopens the store, so failed publication cannot be
    /// mistaken for an idle successful writer on the next frame.
    pub fn poll_background(&mut self) -> Result<bool> {
        if self.operation_error.is_some() {
            if self.operation_error_reported {
                return Ok(false);
            }
            self.operation_error_reported = true;
            return self.check_operation_error().map(|()| false);
        }
        self.check_operation_error()?;
        if !self.operations.is_finished() {
            return Ok(false);
        }
        if let Err(error) = self.finish_background() {
            self.operation_error_reported = true;
            return Err(error);
        }
        Ok(true)
    }

    pub fn finish_background(&mut self) -> Result<()> {
        self.check_operation_error()?;
        let completion = self.operations.finish();
        let result = (|| {
            let Some((name, metadata)) = completion? else {
                return Ok(());
            };
            let index = self
                .find_by_filename(name.as_str())
                .context("completed save lost its owned slot")?;
            self.finish_publication(index, metadata)
        })();
        if let Err(error) = &result {
            self.operation_error = Some(format!(
                "owned save publication failed: {error:#}; reopen the save store before further operations"
            ));
        }
        result
    }

    fn check_operation_error(&self) -> Result<()> {
        if let Some(error) = &self.operation_error {
            anyhow::bail!("{error}");
        }
        Ok(())
    }

    fn owned_recovery_path(&self) -> PathBuf {
        Path::new(&self.save_directory).join("owned-save-recovery.json")
    }

    fn retire_owned_receipt(&self) -> Result<()> {
        match std::fs::remove_file(self.owned_recovery_path()) {
            Ok(()) => sync_save_directory(&self.save_directory),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).context("retire owned save recovery receipt"),
        }
    }

    fn reconcile_owned_save(&mut self) -> Result<()> {
        if let Some(slot) =
            recovery::owned_candidate(&self.save_directory, &self.owned_recovery_path())?
        {
            self.catalog.upsert(slot, SlotState::Published)?;
            self.publish_index().map_err(anyhow::Error::msg)?;
        }
        self.retire_owned_receipt()
    }

    #[cfg(test)]
    pub(crate) fn insert_test_slot(&mut self, slot: SaveGame, state: SlotState) {
        self.catalog.insert_fixture(slot, state);
    }

    pub fn slot_name(&self, index: usize) -> Result<SlotName, String> {
        self.catalog.name(index).map_err(|error| error.to_string())
    }

    /// Find the slot for one of the well-known special filenames, or
    /// create a new slot if none exists yet.  Used to manage the
    /// Continue / Restart / Sherwood / QuickSave auto-slots.
    fn ensure_special_slot(&mut self, filename: &str, display_text: &str) -> Result<usize> {
        self.finish_background()?;
        self.ensure_no_pending_delete()?;
        anyhow::ensure!(
            SpecialSlot::from_filename(filename).is_some(),
            "special-save API requires a special slot"
        );
        if let Some(index) = self.find_by_filename(filename) {
            Ok(index)
        } else {
            self.allocate_named_draft(filename.into(), display_text.into(), 0)
        }
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
        let idx = self.ensure_special_slot(save_file::special_slots::CONTINUE, "Continue")?;
        self.write_save_from_engine(host, game, idx, engine, mission_id, profiles, thumbnail)
            .map(|_| ())
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
    ) -> Result<SaveWriteStatus> {
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
        self.finish_background()?;
        self.ensure_no_pending_delete()?;
        self.reconcile_quick_slots()?;
        // Validate and serialize before touching either published quick slot.
        // A failed capture must not rotate a player's recoverable saves.
        let mut current = self
            .find_by_filename(save_file::special_slots::QUICK)
            .map(|index| self.catalog[index].clone())
            .unwrap_or_else(|| {
                SaveGame::new(
                    save_file::special_slots::QUICK.to_owned(),
                    "Quick Save".to_owned(),
                    mission_id,
                )
            });
        let provenance = required_save_provenance(host, engine, mission_id, profiles)?;
        let save = GameSaveFile::capture_with_game(
            engine,
            host,
            game,
            mission_id,
            game.mission_assets().map_err(anyhow::Error::msg)?.clone(),
            current.text.clone(),
            provenance,
        )?;
        let bytes = serde_json::to_vec_pretty(&save).context("preparing quick save")?;
        Self::sync_slot_metadata_from_header(&mut current, &save.header)?;
        Self::sync_slot_campaign_metadata(
            &mut current,
            save.engine.campaign(),
            profiles.context("quick save requires profiles")?,
        );
        let mut recovery = QuickSaveRecovery {
            slots: vec![(current, Sha256::digest(&bytes).into())],
        };
        if let Some(index) = self.find_by_filename(save_file::special_slots::QUICK)
            && self.slot_file_exists(index)
        {
            let previous_bytes =
                std::fs::read(self.save_path(index)).context("preparing previous quick save")?;
            let mut previous = self.catalog[index].clone();
            previous.filename = save_file::special_slots::EX_QUICK.to_owned();
            previous.special = Some(SpecialSlot::ExQuickSave);
            previous.text = self
                .find_by_filename(save_file::special_slots::EX_QUICK)
                .map(|index| self.catalog[index].text.clone())
                .unwrap_or_else(|| "Previous Quick Save".to_owned());
            previous.validate_published_metadata()?;
            recovery
                .slots
                .push((previous, Sha256::digest(&previous_bytes).into()));
        }
        save_file::atomic_write(&self.quick_recovery_path(), &serde_json::to_vec(&recovery)?)?;
        // Rotate: QuickSave → ExQuickSave
        if let Some(quick_idx) = self.find_by_filename(save_file::special_slots::QUICK)
            && self.slot_file_exists(quick_idx)
        {
            // Ensure an ExQuickSave slot exists, then copy the file.
            let ex_idx = self
                .ensure_special_slot(save_file::special_slots::EX_QUICK, "Previous Quick Save")?;
            self.copy_files(quick_idx, ex_idx)
                .map_err(|e| anyhow::anyhow!(e))?;
            self.copy_display_metadata(quick_idx, ex_idx)?;
        }
        let idx = self.ensure_special_slot(save_file::special_slots::QUICK, "Quick Save")?;
        save_file::atomic_write(&self.save_path(idx), &bytes)?;
        self.publish_thumbnail(idx, thumbnail);
        self.sync_slot_metadata_from_save(idx, &save, profiles)?;
        self.publish_index().map_err(anyhow::Error::msg)?;
        Ok(())
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
        #[cfg(target_arch = "wasm32")]
        {
            let _ = thumbnail;
            return self.write_session_restart(host, game, engine, mission_id, profiles);
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let idx =
                self.ensure_special_slot(save_file::special_slots::RESTART, "Restart Point")?;
            self.write_save_from_engine(host, game, idx, engine, mission_id, profiles, thumbnail)
                .map(|_| ())
        }
    }

    /// Like [`write_restart_save`](Self::write_restart_save), but captures
    /// the engine state on the calling thread and moves the expensive JSON
    /// serialization + disk write to a background thread. Browser builds
    /// publish an immediately loadable session checkpoint without serialization.
    pub fn write_restart_save_background(
        &mut self,
        host: &mut Host,
        game: &crate::game::Game,
        engine: &Engine,
        mission_id: u32,
        profiles: Option<&ProfileManager>,
        thumbnail: Option<&Thumbnail>,
    ) -> Result<SaveWriteStatus> {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = thumbnail;
            self.write_session_restart(host, game, engine, mission_id, profiles)?;
            return Ok(SaveWriteStatus::Completed);
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
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
    }

    #[allow(clippy::too_many_arguments)]
    fn write_special_save_background(
        &mut self,
        filename: &str,
        display_text: &str,
        _thread_name: &str,
        _log_label: &'static str,
        host: &mut Host,
        game: &crate::game::Game,
        engine: &Engine,
        mission_id: u32,
        profiles: Option<&ProfileManager>,
        thumbnail: Option<&Thumbnail>,
    ) -> Result<SaveWriteStatus> {
        self.finish_background()?;
        self.ensure_no_pending_delete()?;
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (
                filename,
                display_text,
                host,
                game,
                engine,
                mission_id,
                profiles,
                thumbnail,
            );
            anyhow::bail!(
                "browser manual special-save persistence is unavailable; use durable autosaves"
            );
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            self.reconcile_quick_slots()?;
            let idx = self.ensure_special_slot(filename, display_text)?;
            let display_text = self.catalog[idx].text.clone();
            let provenance = required_save_provenance(host, engine, mission_id, profiles)?;
            // Capture (clone) on the main thread — fast.
            let save = GameSaveFile::capture_with_game(
                engine,
                host,
                game,
                mission_id,
                game.mission_assets().map_err(anyhow::Error::msg)?.clone(),
                display_text,
                provenance,
            )?;
            let path = self.save_path(idx);
            let thumb_data = thumbnail.cloned();
            let thumb_path = self.thumb_path(idx);
            let mut metadata = self.catalog[idx].clone();
            Self::sync_slot_metadata_from_header(&mut metadata, &save.header)?;
            Self::sync_slot_campaign_metadata(
                &mut metadata,
                save.engine.campaign(),
                profiles.context("save metadata requires profiles")?,
            );
            metadata.validate_published_metadata()?;
            let name = self.slot_name(idx).map_err(anyhow::Error::msg)?;
            let recovery_path = self.owned_recovery_path();
            self.operations.start(name, move || {
                save.validate_current_schema()?;
                let bytes =
                    serde_json::to_vec_pretty(&save).context("serialize owned save payload")?;
                let receipt = SpecialSaveRecovery {
                    slot: metadata.clone(),
                    digest: Sha256::digest(&bytes).into(),
                };
                persistence::publish_payload(&recovery_path, &path, &receipt, &bytes, true)?;
                if let Some(thumb) = thumb_data
                    && let Err(err) = thumb.write_to(&thumb_path)
                {
                    tracing::warn!("Owned save thumbnail failed (payload completed): {err:#}");
                }
                Ok(metadata)
            })?;
            Ok(SaveWriteStatus::Queued)
        }
    }

    /// Capture and publish a session-only Restart atomically. No disk index is
    /// written: this checkpoint is intentionally gone with its owning manager.
    #[cfg(any(target_arch = "wasm32", test))]
    fn write_session_restart(
        &mut self,
        host: &Host,
        game: &crate::game::Game,
        engine: &Engine,
        mission_id: u32,
        profiles: Option<&ProfileManager>,
    ) -> Result<()> {
        // Invalidate the previous mission's checkpoint even if capture fails.
        self.session_restart = None;
        let provenance = required_save_provenance(host, engine, mission_id, profiles)?;
        let header = SaveHeader::new(
            mission_id,
            game.mission_assets().map_err(anyhow::Error::msg)?.clone(),
            "Restart Point".into(),
            provenance,
        )?;
        let save = PreparedGameSave::capture_session_restart(engine, host, game, header)?;
        let mut slot = SaveGame::new(
            save_file::special_slots::RESTART.into(),
            "Restart Point".into(),
            mission_id,
        );
        Self::sync_slot_metadata_from_header(&mut slot, &save.header)?;
        Self::sync_slot_campaign_metadata(
            &mut slot,
            save.engine.campaign(),
            profiles.context("restart requires mission profiles")?,
        );
        // This runtime-only checkpoint must not consult a filesystem receipt
        // or require a writable desktop save root.
        self.catalog.upsert(slot, SlotState::Session)?;
        self.session_restart = Some(std::sync::Arc::new(save));
        Ok(())
    }

    pub(crate) fn clear_session_restart(&mut self) {
        self.session_restart = None;
    }

    pub(crate) fn restart_session_identity(&self) -> Option<ReplaySaveIdentity> {
        self.session_restart
            .as_ref()
            .and_then(|save| save.session_identity())
    }

    /// Whether a Restart snapshot exists on disk or in session memory. The
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
    pub(crate) fn preflight_restart_save(&self) -> Result<Option<(usize, PreparedGameSave)>> {
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
        let idx = self.ensure_special_slot(save_file::special_slots::SHERWOOD, "Sherwood")?;
        self.write_save_from_engine(host, game, idx, engine, mission_id, profiles, thumbnail)
            .map(|_| ())
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

    /// Play resumes the latest published checkpoint. A lifecycle autosave can
    /// be newer than Continue, particularly after advancing to another mission.
    pub fn find_resume_target(&self) -> Option<usize> {
        self.saves()
            .enumerate()
            .filter(|(_, save)| save.is_continue() || save.is_autosave())
            .max_by_key(|(_, save)| {
                (
                    save.timestamp
                        .parse::<u64>()
                        .expect("published resume checkpoint has an invalid timestamp"),
                    save.is_continue(),
                    save.filename.as_str(),
                )
            })
            .map(|(index, _)| index)
    }

    /// Decode and validate the selected save before constructing a
    /// destination mission Engine. Callers use its campaign, RNG state, and
    /// SimConfig for level initialization, then apply the full payload once
    /// the destination's immutable level assets are attached.
    pub(crate) fn preflight_load(
        &self,
        explicit: Option<usize>,
    ) -> Result<Option<(usize, PreparedGameSave)>> {
        let Some(index) = self.find_load_target(explicit) else {
            return Ok(None);
        };
        let save = self.preflight_exact_slot(index)?;
        Ok(Some((index, save)))
    }

    /// Decode exactly the requested slot without falling back to Continue.
    /// This is used when a UI decision and the later apply must refer to the
    /// same selected file even if the directory changes concurrently.
    pub(crate) fn preflight_exact_slot(&self, index: usize) -> Result<PreparedGameSave> {
        self.check_operation_error()?;
        let slot = self
            .catalog
            .get(index)
            .ok_or_else(|| anyhow::anyhow!("save slot index {index} is out of range"))?;
        anyhow::ensure!(
            self.operations
                .pending_name()
                .is_none_or(|name| name.as_str() != slot.filename),
            "save slot {} is still being published; poll completion first",
            slot.filename
        );
        anyhow::ensure!(
            self.slot_state(&SlotName::new(slot.filename.clone()).map_err(anyhow::Error::msg)?)?
                != SlotState::Draft,
            "save slot {} is an unpublished draft",
            slot.filename
        );
        if slot.is_restart() {
            if let Some(save) = &self.session_restart {
                return Ok(save.as_ref().clone());
            }
            #[cfg(target_arch = "wasm32")]
            anyhow::bail!("session Restart checkpoint is unavailable");
        }
        if slot.is_autosave() {
            return crate::autosave::read_payload(&self.save_directory, &slot.filename)
                .map(PreparedGameSave::from)
                .with_context(|| {
                    format!(
                        "failed to decode exact autosave slot {index} ({})",
                        slot.filename
                    )
                });
        }
        let path = self.save_path(index);
        GameSaveFile::read_from(&path)
            .map(PreparedGameSave::from)
            .with_context(|| {
                format!(
                    "failed to decode exact save slot {index} ({})",
                    slot.filename
                )
            })
    }

    /// Allocate a draft and return its stable owner-bound identity.
    pub fn create_draft(&mut self, text: String, mission_id: u32) -> Result<SlotHandle> {
        self.finish_background()?;
        self.ensure_no_pending_delete()?;
        let filename = self.next_filename()?;
        let save = SaveGame::new(filename, text, mission_id);
        let index = self.catalog.insert(save, SlotState::Draft)?;
        self.slot_handle(index)
    }

    #[cfg(test)]
    pub(crate) fn create(&mut self, text: String, mission_id: u32) -> usize {
        let handle = self
            .create_draft(text, mission_id)
            .expect("test draft allocation");
        self.resolve_handle(&handle).expect("new test slot")
    }

    /// Create a save with a specific filename.
    fn allocate_named_draft(
        &mut self,
        filename: String,
        text: String,
        mission_id: u32,
    ) -> Result<usize> {
        self.finish_background()?;
        self.ensure_no_pending_delete()?;
        SlotName::new(filename.clone()).map_err(anyhow::Error::msg)?;
        anyhow::ensure!(
            self.find_by_filename(&filename).is_none(),
            "duplicate save slot {filename}"
        );
        let save = SaveGame::new(filename, text, mission_id);
        self.catalog.insert(save, SlotState::Draft)
    }

    #[cfg(test)]
    pub(crate) fn create_with_filename(
        &mut self,
        filename: String,
        text: String,
        mission_id: u32,
    ) -> usize {
        self.allocate_named_draft(filename, text, mission_id)
            .expect("test named slot")
    }

    /// Find by filename, or create if not found. Updates text either way.
    #[cfg(test)]
    fn find_or_create_by_filename(&mut self, filename: &str, text: &str) -> usize {
        self.finish_background()
            .expect("previous save failed; reopen store before updating slots");
        if let Some(idx) = self.find_by_filename(filename) {
            self.catalog[idx].text = text.to_string();
            idx
        } else {
            self.create_with_filename(filename.to_string(), text.to_string(), 0)
        }
    }

    pub fn get(&self, index: usize) -> Option<&SaveGame> {
        self.catalog.get(index)
    }

    #[cfg(test)]
    pub(crate) fn get_mut(&mut self, index: usize) -> Option<&mut SaveGame> {
        self.catalog.metadata_mut(index)
    }

    pub fn slot_mission_id(&self, index: usize) -> Option<u32> {
        self.catalog
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
            .catalog
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
        self.catalog.iter().position(|s| s.text == text)
    }

    pub fn find_by_filename(&self, filename: &str) -> Option<usize> {
        self.catalog.find(filename)
    }

    pub fn count(&self) -> usize {
        self.catalog.len()
    }

    pub fn remove(&mut self, index: usize) -> Result<()> {
        self.finish_background()?;
        let slot = self
            .catalog
            .get(index)
            .context("delete slot no longer exists")?;
        if self.slot_state(&self.slot_name(index).map_err(anyhow::Error::msg)?)? == SlotState::Draft
        {
            // A failed/new draft never acquired authority to delete a payload
            // that another writer may have created at the selected basename.
            self.catalog.remove(index)?;
            return Ok(());
        }
        if slot.is_restart() && (self.session_restart.is_some() || cfg!(target_arch = "wasm32")) {
            self.session_restart = None;
            self.catalog.remove(index)?;
            return Ok(());
        }
        anyhow::ensure!(
            !slot.is_autosave(),
            "cannot manually delete an auto-managed autosave"
        );
        let receipt = DeleteRecovery {
            filename: self.slot_name(index).map_err(anyhow::Error::msg)?,
        };
        self.reconcile_quick_slots()?;
        // Finish a previous intent before replacing its only recovery record.
        self.reconcile_delete()?;
        let bytes = serde_json::to_vec_pretty(&receipt)?;
        if let Err(error) = save_file::atomic_write(&self.delete_recovery_path(), &bytes) {
            // A directory-sync error may occur after rename. Reflect a
            // visible committed intent immediately, but still return failure.
            if std::fs::read(self.delete_recovery_path()).ok().as_deref() == Some(bytes.as_slice())
            {
                self.catalog.remove_named(receipt.filename.as_str())?;
            }
            return Err(error).context(
                "publishing deletion intent; reopen store before retry if publication is uncertain",
            );
        }
        self.finish_delete(receipt)
    }

    /// Remove by filename.
    pub fn remove_by_filename(&mut self, filename: &str) -> Result<()> {
        let index = self
            .find_by_filename(filename)
            .context("delete slot no longer exists")?;
        self.remove(index)
    }

    fn delete_recovery_path(&self) -> PathBuf {
        Path::new(&self.save_directory).join("save-delete-recovery.json")
    }

    fn ensure_no_pending_delete(&self) -> Result<()> {
        self.check_operation_error()?;
        #[cfg(not(target_arch = "wasm32"))]
        match std::fs::symlink_metadata(self.owned_recovery_path()) {
            Ok(_) => anyhow::bail!(
                "owned save recovery is pending; reopen the store before further writes"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("checking owned save recovery"),
        }
        #[cfg(target_arch = "wasm32")]
        return Ok(()); // Desktop deletion receipts do not exist in the memory backend.
        #[cfg(not(target_arch = "wasm32"))]
        match std::fs::symlink_metadata(self.delete_recovery_path()) {
            Ok(_) => anyhow::bail!(
                "save deletion recovery is pending; reopen the store before further writes"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).context("checking pending save deletion"),
        }
    }

    fn reconcile_delete(&mut self) -> Result<()> {
        let bytes = match std::fs::read(self.delete_recovery_path()) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error).context("read deletion recovery intent"),
        };
        self.finish_delete(
            serde_json::from_slice(&bytes).context("decode deletion recovery intent")?,
        )
    }

    fn finish_delete(&mut self, receipt: DeleteRecovery) -> Result<()> {
        let filename = receipt.filename.as_str();
        anyhow::ensure!(
            !is_generated_autosave_filename(filename),
            "deletion intent cannot target an autosave"
        );
        // Intent is the authoritative logical deletion even if publication or
        // cleanup fails. Keep memory consistent and retain intent for reopen.
        self.catalog.remove_named(filename)?;
        self.publish_index()
            .map_err(anyhow::Error::msg)
            .context("deletion recorded, index publication incomplete; recovery intent retained")?;
        for suffix in [".json", "_thumb.png"] {
            let path = Path::new(&self.save_directory).join(format!("{filename}{suffix}"));
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!(
                            "save logically deleted; cleanup of {} pending on reopen",
                            path.display()
                        )
                    });
                }
            }
        }
        sync_save_directory(&self.save_directory)?;
        std::fs::remove_file(self.delete_recovery_path()).context("retire deletion intent")?;
        sync_save_directory(&self.save_directory)
    }

    /// Sort saves by timestamp (oldest first).  The load/save menu
    /// iterates this list forward to populate its entries.
    pub fn sort_by_time(&mut self) {
        self.catalog.sort_by_time();
    }

    /// Thumbnail file path.
    pub fn thumb_path(&self, index: usize) -> PathBuf {
        let filename = self.slot_name(index).expect("invalid save slot identity");
        Path::new(&self.save_directory).join(format!("{}_thumb.png", filename.as_str()))
    }

    /// Full path to a save file on disk (JSON format, with `.json` extension).
    pub fn save_path(&self, index: usize) -> PathBuf {
        let filename = self.slot_name(index).expect("invalid save slot identity");
        Path::new(&self.save_directory).join(format!("{}.json", filename.as_str()))
    }

    /// Copy save + thumbnail files from `src` slot to `dst` slot.
    ///
    /// Copies both the JSON payload (`<name>.json`) and any thumbnail.
    /// Used by the quick-save rotation to preserve the previous quick-save
    /// as ExQuickSave.
    ///
    pub fn copy_files(&mut self, src: usize, dst: usize) -> Result<(), String> {
        self.finish_background()
            .map_err(|error| format!("{error:#}"))?;
        self.ensure_no_pending_delete()
            .map_err(|error| format!("{error:#}"))?;
        // JSON payload
        let src_json = self.save_path(src);
        let dst_json = self.save_path(dst);
        let bytes = std::fs::read(&src_json).map_err(|e| format!("read save json: {e}"))?;
        save_file::atomic_write(&dst_json, &bytes).map_err(|e| format!("copy save json: {e:#}"))?;
        // Thumbnail (used by both formats)
        let src_thumb = self.thumb_path(src);
        let dst_thumb = self.thumb_path(dst);
        if src_thumb.exists()
            && let Err(error) = std::fs::read(&src_thumb)
                .map_err(anyhow::Error::from)
                .and_then(|bytes| save_file::atomic_write(&dst_thumb, &bytes))
        {
            tracing::warn!("Could not rotate save thumbnail: {error:#}");
        }

        Ok(())
    }

    fn copy_display_metadata(&mut self, src: usize, dst: usize) -> Result<()> {
        let state = self.slot_state(&self.slot_name(src).map_err(anyhow::Error::msg)?)?;
        let src = self
            .catalog
            .get(src)
            .with_context(|| format!("cannot copy metadata from missing save slot {src}"))?
            .clone();
        let destination = dst;
        let mut dst = self
            .catalog
            .get(dst)
            .with_context(|| format!("cannot copy metadata to missing save slot {dst}"))?
            .clone();

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
        self.catalog.replace(destination, dst, state)
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
    ) -> Result<CommittedSave> {
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
    ) -> Result<CommittedSave> {
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
    ) -> Result<CommittedSave> {
        Self::require_synchronous_storage()?;
        self.finish_background()?;
        self.ensure_no_pending_delete()?;
        self.reconcile_quick_slots()?;
        let display_text = self
            .catalog
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
            game.mission_assets().map_err(anyhow::Error::msg)?.clone(),
            display_text,
            provenance,
        )?;
        save.header.multiplayer_diagnostic = multiplayer_diagnostic;
        let mut metadata = self.catalog[index].clone();
        Self::sync_slot_metadata_from_header(&mut metadata, &save.header)?;
        Self::sync_slot_campaign_metadata(
            &mut metadata,
            save.engine.campaign(),
            profiles.context("save metadata requires mission profiles")?,
        );
        metadata.validate_published_metadata()?;
        save.validate_current_schema()?;
        let bytes =
            serde_json::to_vec_pretty(&save).context("serialize synchronous save payload")?;
        self.commit_synchronous(index, metadata, &bytes, thumbnail)
    }

    fn commit_synchronous(
        &mut self,
        index: usize,
        metadata: SaveGame,
        bytes: &[u8],
        thumbnail: Option<&Thumbnail>,
    ) -> Result<CommittedSave> {
        Self::require_synchronous_storage()?;
        let handle = self.slot_handle(index)?;
        anyhow::ensure!(
            metadata.filename == handle.name().as_str(),
            "publication changed slot identity"
        );
        metadata.validate_published_metadata()?;
        let receipt = SpecialSaveRecovery {
            slot: metadata.clone(),
            digest: Sha256::digest(bytes).into(),
        };
        let overwrite =
            self.catalog[index].is_special() || self.slot_state(handle.name())? != SlotState::Draft;
        let payload_path = self.save_path(index);
        if let Err(error) = persistence::publish_payload(
            &self.owned_recovery_path(),
            &payload_path,
            &receipt,
            bytes,
            overwrite,
        ) {
            // Atomic publication may fail after rename. Only a definitely
            // uncommitted payload permits retiring prospective evidence.
            let rejected_new_target = !overwrite
                && error.chain().any(|cause| {
                    cause
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|error| error.kind() == std::io::ErrorKind::AlreadyExists)
                });
            let recovery = if rejected_new_target {
                // The no-clobber primitive explicitly rejected this write.
                // Even identical bytes belong to the pre-existing orphan.
                Ok(None)
            } else {
                recovery::owned_candidate(&self.save_directory, &self.owned_recovery_path())
            };
            match recovery {
                Ok(None) => {
                    if let Err(retirement) = self.retire_owned_receipt() {
                        self.operation_error = Some(format!(
                            "save receipt cleanup failed: {retirement:#}; reopen the save store"
                        ));
                    }
                }
                Ok(Some(_)) | Err(_) => {
                    self.operation_error = Some(format!(
                        "save payload publication uncertain: {error:#}; reopen the save store"
                    ));
                }
            }
            return Err(error).context("save payload publication failed");
        }
        self.publish_thumbnail(index, thumbnail);
        let result = self.finish_publication(index, metadata);
        if let Err(error) = &result {
            self.operation_error = Some(format!(
                "save index publication failed: {error:#}; reopen the save store"
            ));
        }
        result?;
        Ok(CommittedSave {
            slot: handle,
            digest: receipt.digest,
        })
    }

    fn require_synchronous_storage() -> Result<()> {
        anyhow::ensure!(
            !cfg!(target_arch = "wasm32"),
            "browser manual-save persistence is unavailable; use durable autosaves"
        );
        Ok(())
    }

    fn finish_publication(&mut self, index: usize, metadata: SaveGame) -> Result<()> {
        self.catalog
            .replace(index, metadata, SlotState::Published)?;
        self.publish_index().map_err(anyhow::Error::msg)?;
        self.retire_owned_receipt()
    }

    fn publish_thumbnail(&self, index: usize, thumbnail: Option<&Thumbnail>) {
        // A preview is not part of the authoritative payload transaction.
        if let Some(thumb) = thumbnail {
            let thumb_path = self.thumb_path(index);
            if let Err(err) = thumb.write_to(&thumb_path) {
                // Non-fatal — the save payload is already on disk.
                tracing::warn!("Failed to write thumbnail for slot {index}: {err:#}");
            }
        }
    }

    fn sync_slot_metadata_from_save(
        &mut self,
        index: usize,
        save: &GameSaveFile,
        profiles: Option<&ProfileManager>,
    ) -> Result<()> {
        let mut slot = self
            .catalog
            .get(index)
            .with_context(|| format!("cannot synchronize missing save slot {index}"))?
            .clone();
        Self::sync_slot_metadata_from_header(&mut slot, &save.header)?;
        let profiles = profiles.context("save metadata requires mission profiles")?;
        Self::sync_slot_campaign_metadata(&mut slot, save.engine.campaign(), profiles);
        self.catalog.replace(index, slot, SlotState::Published)
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
        let slot = self.catalog.get(index)?;
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
        let save = self.preflight_exact_slot(index)?;
        save.apply_to_with_game(engine, host, game, assets)?;
        Ok(())
    }

    /// Does this slot have a stored payload, including a session checkpoint?
    pub fn slot_file_exists(&self, index: usize) -> bool {
        if let Some(slot) = self.catalog.get(index) {
            if self
                .operations
                .pending_name()
                .is_some_and(|name| name.as_str() == slot.filename)
            {
                return false; // A queued publication is explicitly not loadable yet.
            }
            let name = SlotName::new(slot.filename.clone()).expect("validated runtime slot");
            if self
                .catalog
                .state(&name)
                .expect("validated runtime slot state")
                == SlotState::Draft
            {
                return false;
            }
        }
        if self.catalog.get(index).is_some_and(SaveGame::is_restart) {
            if self.session_restart.is_some() {
                return true;
            }
            #[cfg(target_arch = "wasm32")]
            return false;
        }

        if let Some(slot) = self.catalog.get(index)
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
    pub(crate) fn replace_autosaves(&mut self, autosaves: Vec<SaveGame>) -> Result<()> {
        self.finish_background()?;
        self.catalog.replace_autosaves(autosaves)
    }

    /// Persist the save manager index itself (the list of saves).
    pub fn save_index(&self) -> Result<(), String> {
        self.check_operation_error()
            .map_err(|error| format!("{error:#}"))?;
        if self.operations.pending_name().is_some() {
            return Err(
                "save publication still running; finish it before publishing an index".into(),
            );
        }
        match std::fs::symlink_metadata(self.owned_recovery_path()) {
            Ok(_) => {
                return Err(
                    "owned save recovery is pending; reopen before index publication".into(),
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("checking owned save recovery: {error}")),
        }
        self.ensure_no_pending_delete()
            .map_err(|error| format!("{error:#}"))?;
        match std::fs::symlink_metadata(self.quick_recovery_path()) {
            Ok(_) => {
                return Err(
                    "quick-save recovery is pending; reopen the store before index writes".into(),
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("checking quick-save recovery: {error}")),
        }
        self.publish_index()
    }

    fn publish_index(&self) -> Result<(), String> {
        let index = SaveIndex {
            saves: self
                .catalog
                .published()
                .map_err(|error| format!("validate: {error:#}"))?,
            next_id: self.next_id,
            save_directory: self.save_directory.clone(),
        };
        persistence::publish_index(&self.save_directory, &index, &self.quick_recovery_path())
    }

    /// Load the save manager index from disk.
    pub fn load_index(save_directory: &str) -> Result<Self, String> {
        let path = Path::new(save_directory).join("saves.json");
        let mut manager = match std::fs::read_to_string(&path) {
            Ok(data) => {
                // Legacy save_directory is decoded only as compatibility metadata.
                let index: SaveIndex =
                    serde_json::from_str(&data).map_err(|e| format!("parse: {e}"))?;
                let mut manager = Self::new(save_directory.to_owned());
                manager.next_id = index.next_id;
                for slot in index.saves {
                    manager
                        .catalog
                        .insert(slot, SlotState::Published)
                        .map_err(|error| format!("validate: {error:#}"))?;
                }
                manager
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Self::new(save_directory.to_owned())
            }
            Err(error) => return Err(format!("read: {error}")),
        };
        // Validate the entire untrusted index before any recovery performs I/O.
        manager
            .reconcile_quick_slots()
            .map_err(|error| format!("recover quick saves: {error:#}"))?;
        manager
            .reconcile_owned_save()
            .map_err(|error| format!("recover owned save: {error:#}"))?;
        manager
            .reconcile_delete()
            .map_err(|error| format!("recover deletion: {error:#}"))?;
        for save in manager.catalog.iter() {
            save.validate_published_metadata()
                .map_err(|error| format!("validate: {error:#}"))?;
        }
        Ok(manager)
    }

    fn quick_recovery_path(&self) -> PathBuf {
        Path::new(&self.save_directory).join("quick-save-recovery.json")
    }

    /// Publish only receipt entries whose payload actually reached disk.
    /// A crash between rotation and new-save publication keeps the old quick
    /// entry and recovers the previous slot independently.
    fn reconcile_quick_slots(&mut self) -> Result<()> {
        let Some(slots) =
            recovery::quick_candidates(&self.save_directory, &self.quick_recovery_path())?
        else {
            return Ok(());
        };
        for slot in slots {
            self.catalog.upsert(slot, SlotState::Published)?;
        }
        self.publish_index().map_err(anyhow::Error::msg)
    }

    fn next_filename(&mut self) -> Result<String> {
        loop {
            let name = format!("Savegame_{:03}", self.next_id);
            self.next_id = self
                .next_id
                .checked_add(1)
                .context("save slot identifier space exhausted")?;
            #[cfg(not(target_arch = "wasm32"))]
            let root = Path::new(&self.save_directory);
            // symlink_metadata counts broken symlinks as occupied too; access
            // failures are not evidence that it is safe to replace a target.
            #[cfg(target_arch = "wasm32")]
            let occupied = false; // Browser manual slots are memory-only until an explicit unsupported write.
            #[cfg(not(target_arch = "wasm32"))]
            let occupied = {
                let mut occupied = false;
                for filename in [format!("{name}.json"), format!("{name}_thumb.png")] {
                    match std::fs::symlink_metadata(root.join(filename)) {
                        Ok(_) => occupied = true,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => {
                            return Err(error).context("cannot safely allocate save slot");
                        }
                    }
                }
                occupied
            };
            if !occupied && self.find_by_filename(&name).is_none() {
                return Ok(name);
            }
        }
    }
}

impl Drop for SaveGameManager {
    fn drop(&mut self) {
        if self.operations.pending_name().is_some() {
            tracing::warn!(
                "save manager retired without explicit finish; joining outstanding publication"
            );
            if let Err(error) = self.finish_background() {
                tracing::error!("retired save publication failed: {error:#}");
            }
        }
    }
}

fn validate_slot_names(saves: &[SaveGame]) -> Result<()> {
    let mut names = std::collections::HashSet::new();
    for slot in saves {
        SlotName::new(slot.filename.clone()).map_err(anyhow::Error::msg)?;
        anyhow::ensure!(
            names.insert(slot.filename.to_ascii_lowercase()),
            "duplicate save slot name {:?}",
            slot.filename
        );
    }
    Ok(())
}

fn sync_save_directory(directory: &str) -> Result<()> {
    #[cfg(all(unix, not(target_arch = "wasm32")))]
    std::fs::File::open(directory)?
        .sync_all()
        .context("sync save directory")?;
    #[cfg(not(all(unix, not(target_arch = "wasm32"))))]
    let _ = directory;
    Ok(())
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

    #[test]
    fn play_resumes_newer_mission_autosave_instead_of_stale_continue() {
        let mut manager = SaveGameManager::new(String::new());
        assert_eq!(manager.find_resume_target(), None);
        let mut first = published_slot("Continue");
        first.timestamp = "100".into();
        let mut second = published_slot("Autosave_200_0003");
        second.timestamp = "200".into();
        second.mission_id = 2;
        let mut restart = published_slot("Restart");
        restart.timestamp = "300".into();
        for save in [first, second, restart] {
            manager.insert_test_slot(save, SlotState::Published);
        }
        assert_eq!(manager.find_resume_target(), Some(1));
        assert_eq!(manager.slot_mission_id(1), Some(2));
        manager.catalog[0].timestamp = "400".into();
        assert_eq!(manager.find_resume_target(), Some(0));
        manager.catalog[1].timestamp = "400".into();
        assert_eq!(manager.find_resume_target(), Some(0));
    }

    fn published_slot(filename: &str) -> SaveGame {
        let mut slot = SaveGame::new(filename.into(), filename.into(), 1);
        slot.timestamp = "123".into();
        slot.mission_name = "Mission 1".into();
        slot.player_profile_id = Some(0);
        slot.player_name = "Player".into();
        slot.campaign_progress = Some(0);
        slot.missions_done = Some(0);
        slot.missions_total = Some(1);
        slot.gang_size = Some(1);
        slot.ransom = Some(0);
        slot.blazons = Some(0);
        slot.amulets = Some(0);
        slot.validate_published_metadata().unwrap();
        slot
    }

    fn indexed_store(root: &Path, names: &[&str]) -> SaveGameManager {
        let mut manager = SaveGameManager::new(root.to_str().unwrap().into());
        for name in names {
            manager.insert_test_slot(published_slot(name), SlotState::Published);
        }
        manager.save_index().unwrap();
        manager
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn synchronous_publication_failure_matrix_recovers_only_completed_payloads() {
        use persistence::FailurePoint::*;
        let (engine, _, profiles, mut host) = fresh_save_session("Transaction player");
        let game = game_for_save(&profiles, 17);
        for overwrite in [false, true] {
            for diagnostic in [false, true] {
                for stage in [
                    BeforeReceipt,
                    BeforePayload,
                    AfterPayload,
                    BeforeIndex,
                    AfterIndex,
                ] {
                    let root = tempfile::tempdir().unwrap();
                    let mut manager = indexed_store(root.path(), &["Unrelated"]);
                    let untouched = manager.get(0).unwrap().clone();
                    std::fs::write(manager.save_path(0), b"unrelated payload").unwrap();
                    let handle = manager.create_draft("Transaction".into(), 17).unwrap();
                    let index = manager.resolve_handle(&handle).unwrap();
                    if overwrite {
                        manager
                            .write_save_from_engine(
                                &mut host,
                                &game,
                                index,
                                &engine,
                                17,
                                Some(&profiles),
                                None,
                            )
                            .unwrap();
                    }
                    let old = manager.get(index).unwrap().clone();
                    let old_payload = std::fs::read(manager.save_path(index)).ok();
                    // Distinguish the replacement from the previous successful
                    // write even when wall-clock timestamps have not advanced.
                    manager
                        .rename_slot(&handle, format!("Replacement {stage:?}"))
                        .unwrap();
                    persistence::inject_failure(stage);
                    let error = manager
                        .write_save_from_engine_with_diagnostic(
                            &mut host,
                            &game,
                            index,
                            &engine,
                            17,
                            Some(&profiles),
                            None,
                            diagnostic,
                        )
                        .unwrap_err();
                    assert!(
                        format!("{error:#}").contains("injected"),
                        "{stage:?}: {error:#}"
                    );
                    let landed = matches!(stage, AfterPayload | BeforeIndex | AfterIndex);
                    if landed {
                        assert!(manager.owned_recovery_path().exists());
                        assert!(manager.create_draft("Blocked".into(), 17).is_err());
                    } else {
                        assert!(!manager.owned_recovery_path().exists());
                        assert_eq!(std::fs::read(manager.save_path(index)).ok(), old_payload);
                        assert_eq!(
                            manager.slot_state(handle.name()).unwrap(),
                            if overwrite {
                                SlotState::Published
                            } else {
                                SlotState::Draft
                            }
                        );
                    }
                    let recovered =
                        SaveGameManager::load_index(root.path().to_str().unwrap()).unwrap();
                    assert_eq!(
                        recovered
                            .get(recovered.find_by_filename("Unrelated").unwrap())
                            .unwrap(),
                        &untouched
                    );
                    assert_eq!(
                        std::fs::read(root.path().join("Unrelated.json")).unwrap(),
                        b"unrelated payload"
                    );
                    let recovered_index = recovered.find_by_filename(handle.name().as_str());
                    if landed {
                        let recovered_index = recovered_index.unwrap();
                        let payload =
                            GameSaveFile::read_from(&recovered.save_path(recovered_index)).unwrap();
                        recovered
                            .validate_slot_identity(recovered_index, &payload)
                            .unwrap();
                        assert_eq!(payload.header.multiplayer_diagnostic, diagnostic);
                        assert_eq!(
                            recovered
                                .get(recovered_index)
                                .unwrap()
                                .multiplayer_diagnostic,
                            diagnostic
                        );
                        assert_eq!(
                            recovered.get(recovered_index).unwrap().text,
                            format!("Replacement {stage:?}")
                        );
                    } else if overwrite {
                        assert_eq!(recovered.get(recovered_index.unwrap()).unwrap(), &old);
                    } else {
                        assert!(recovered_index.is_none());
                    }
                    assert!(!recovered.owned_recovery_path().exists());
                }
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn synchronous_commit_evidence_requires_durable_index_and_preserves_identity() {
        let root = tempfile::tempdir().unwrap();
        let mut manager = indexed_store(root.path(), &["Unrelated"]);
        let (engine, _, profiles, mut host) = fresh_save_session("Committed player");
        let game = game_for_save(&profiles, 17);
        let handle = manager.create_draft("New".into(), 17).unwrap();
        let index = manager.resolve_handle(&handle).unwrap();
        for diagnostic in [false, true] {
            let committed = if diagnostic {
                manager.write_multiplayer_diagnostic_from_engine(
                    &mut host,
                    &game,
                    index,
                    &engine,
                    17,
                    Some(&profiles),
                    None,
                )
            } else {
                manager.write_save_from_engine(
                    &mut host,
                    &game,
                    index,
                    &engine,
                    17,
                    Some(&profiles),
                    None,
                )
            }
            .unwrap();
            assert_eq!(committed.slot(), &handle);
            let encoded = serde_json::to_string(&committed).unwrap();
            assert!(
                serde_json::from_str::<CommittedSave>(&encoded)
                    .unwrap_err()
                    .to_string()
                    .contains("save commit evidence is process-local")
            );
            assert_eq!(
                *committed.digest(),
                <[u8; 32]>::from(Sha256::digest(
                    std::fs::read(manager.save_path(index)).unwrap()
                ))
            );
            assert_eq!(
                manager.slot_state(handle.name()).unwrap(),
                SlotState::Published
            );
            assert!(!manager.owned_recovery_path().exists());
            let reopened = SaveGameManager::load_index(root.path().to_str().unwrap()).unwrap();
            assert_eq!(reopened.count(), 2);
            assert_eq!(
                reopened
                    .get(reopened.find_by_filename(handle.name().as_str()).unwrap())
                    .unwrap()
                    .multiplayer_diagnostic,
                diagnostic
            );
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn synchronous_new_target_collision_never_promotes_identical_orphan_bytes() {
        let root = tempfile::tempdir().unwrap();
        let mut manager = indexed_store(root.path(), &["Unrelated"]);
        let handle = manager.create_draft("New".into(), 1).unwrap();
        let index = manager.resolve_handle(&handle).unwrap();
        std::fs::write(manager.save_path(index), b"identical payload").unwrap();
        let metadata = published_slot(handle.name().as_str());
        assert!(
            manager
                .commit_synchronous(index, metadata, b"identical payload", None)
                .is_err()
        );
        assert_eq!(manager.slot_state(handle.name()).unwrap(), SlotState::Draft);
        assert!(!manager.owned_recovery_path().exists());
        assert!(manager.operation_error.is_none());
        manager.remove(index).unwrap();
        assert_eq!(
            std::fs::read(root.path().join(format!("{}.json", handle.name().as_str()))).unwrap(),
            b"identical payload"
        );
        let reopened = SaveGameManager::load_index(root.path().to_str().unwrap()).unwrap();
        assert_eq!(reopened.count(), 1);
    }

    #[cfg(target_arch = "wasm32")]
    #[wasm_bindgen_test::wasm_bindgen_test]
    fn browser_sync_publication_rejects_without_changing_session_or_autosave_catalog() {
        let mut manager = SaveGameManager::new("browser-memory".into());
        manager.insert_test_slot(published_slot("Restart"), SlotState::Session);
        manager.insert_test_slot(published_slot("Autosave_1_0000"), SlotState::Published);
        let handle = manager.create_draft("Manual".into(), 17).unwrap();
        let before = manager.saves().cloned().collect::<Vec<_>>();
        let index = manager.resolve_handle(&handle).unwrap();
        let error = manager
            .commit_synchronous(
                index,
                published_slot(handle.name().as_str()),
                b"payload",
                None,
            )
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("browser manual-save persistence is unavailable")
        );
        assert!(manager.saves().eq(before.iter()));
        assert_eq!(
            manager
                .slot_state(&SlotName::new("Restart").unwrap())
                .unwrap(),
            SlotState::Session
        );
        assert!(manager.operation_error.is_none());
    }

    #[test]
    fn stable_handles_reject_retired_generations_other_owners_and_serde() {
        let root = tempfile::tempdir().unwrap();
        let mut manager = SaveGameManager::new(root.path().to_str().unwrap().into());
        let first = manager.create_draft("Draft".into(), 17).unwrap();
        assert_eq!(manager.slot_state(first.name()).unwrap(), SlotState::Draft);
        let decoded: SlotHandle =
            serde_json::from_str(&serde_json::to_string(&first).unwrap()).unwrap();
        assert!(manager.resolve_handle(&decoded).is_err());
        let mut other = SaveGameManager::new(root.path().to_str().unwrap().into());
        other.create_draft("Other owner".into(), 17).unwrap();
        assert!(other.resolve_handle(&first).is_err());
        let index = manager.resolve_handle(&first).unwrap();
        manager.remove(index).unwrap();
        let replacement = manager
            .allocate_named_draft(first.name().as_str().into(), "Replacement".into(), 17)
            .unwrap();
        assert!(manager.resolve_handle(&first).is_err());
        assert_ne!(manager.slot_handle(replacement).unwrap(), first);
    }

    #[test]
    fn explicit_draft_state_cannot_be_promoted_by_filling_timestamp() {
        let root = tempfile::tempdir().unwrap();
        let mut manager = SaveGameManager::new(root.path().to_str().unwrap().into());
        let handle = manager.create_draft("Draft".into(), 17).unwrap();
        let index = manager.resolve_handle(&handle).unwrap();
        manager.catalog[index] = published_slot(handle.name().as_str());
        manager.save_index().unwrap();
        assert!(
            SaveGameManager::load_index(root.path().to_str().unwrap())
                .unwrap()
                .saves()
                .len()
                == 0
        );
        std::fs::write(manager.save_path(index), b"unrelated writer").unwrap();
        manager.remove(index).unwrap();
        assert_eq!(
            std::fs::read(root.path().join(format!("{}.json", handle.name().as_str()))).unwrap(),
            b"unrelated writer"
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn queued_special_save_publishes_payload_then_owned_metadata_and_index() {
        let root = tempfile::tempdir().unwrap();
        let mut manager = SaveGameManager::new(root.path().to_str().unwrap().into());
        let (engine, _, profiles, mut host) = fresh_save_session("Owned completion");
        let game = game_for_save(&profiles, 17);
        assert_eq!(
            manager
                .write_continue_save_background(
                    &mut host,
                    &game,
                    &engine,
                    17,
                    Some(&profiles),
                    None
                )
                .unwrap(),
            SaveWriteStatus::Queued
        );
        let slot = manager.find_by_filename("Continue").unwrap();
        assert_eq!(
            manager
                .slot_state(&SlotName::new("Continue").unwrap())
                .unwrap(),
            SlotState::Draft
        );
        assert!(manager.preflight_exact_slot(slot).is_err());
        assert!(manager.save_index().is_err());
        manager.finish_background().unwrap();
        assert_eq!(
            manager
                .slot_state(&SlotName::new("Continue").unwrap())
                .unwrap(),
            SlotState::Published
        );
        let reopened = SaveGameManager::load_index(root.path().to_str().unwrap()).unwrap();
        let decoded = reopened
            .preflight_exact_slot(reopened.find_by_filename("Continue").unwrap())
            .unwrap();
        assert_eq!(decoded.header.provenance.player_name, "Owned completion");
        assert!(!manager.owned_recovery_path().exists());
        assert!(!manager.poll_background().unwrap());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn payload_failure_preserves_old_metadata_and_error_is_sticky_but_notice_once() {
        let root = tempfile::tempdir().unwrap();
        let mut manager = indexed_store(root.path(), &["Continue"]);
        let before = manager.get(0).unwrap().clone();
        std::fs::create_dir(manager.save_path(0)).unwrap();
        let (engine, _, profiles, mut host) = fresh_save_session("Failed owner");
        let game = game_for_save(&profiles, 17);
        manager
            .write_continue_save_background(&mut host, &game, &engine, 17, Some(&profiles), None)
            .unwrap();
        assert!(manager.finish_background().is_err());
        assert_eq!(manager.get(0).unwrap(), &before);
        assert!(manager.poll_background().is_err());
        assert!(!manager.poll_background().unwrap());
        assert!(
            manager
                .create_draft("Must remain blocked".into(), 17)
                .is_err()
        );
        assert!(manager.finish_background().is_err());
        std::fs::remove_dir(manager.save_path(0)).unwrap();
        let recovered = SaveGameManager::load_index(root.path().to_str().unwrap()).unwrap();
        assert_eq!(recovered.get(0).unwrap(), &before);
        assert!(!manager.owned_recovery_path().exists());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn completed_payload_index_failure_is_recoverable_after_store_reopen() {
        let root = tempfile::tempdir().unwrap();
        let mut manager = indexed_store(root.path(), &["Continue"]);
        let index_path = root.path().join("saves.json");
        let old_index = std::fs::read(&index_path).unwrap();
        std::fs::remove_file(&index_path).unwrap();
        std::fs::create_dir(&index_path).unwrap();
        let (engine, _, profiles, mut host) = fresh_save_session("Recover owned metadata");
        let game = game_for_save(&profiles, 17);
        manager
            .write_continue_save_background(&mut host, &game, &engine, 17, Some(&profiles), None)
            .unwrap();
        assert!(manager.finish_background().is_err());
        assert!(manager.owned_recovery_path().exists());
        assert_eq!(
            GameSaveFile::read_from(&manager.save_path(0))
                .unwrap()
                .header
                .provenance
                .player_name,
            "Recover owned metadata"
        );
        std::fs::remove_dir(&index_path).unwrap();
        std::fs::write(&index_path, old_index).unwrap();
        let recovered = SaveGameManager::load_index(root.path().to_str().unwrap()).unwrap();
        assert_eq!(
            recovered.get(0).unwrap().player_name,
            "Recover owned metadata"
        );
        assert_eq!(recovered.get(0).unwrap().mission_id, 17);
        assert!(!manager.owned_recovery_path().exists());
    }

    #[test]
    fn owned_receipt_cannot_publish_autosave_or_control_slots() {
        let root = tempfile::tempdir().unwrap();
        let manager = indexed_store(root.path(), &["Continue"]);
        let before = std::fs::read(root.path().join("saves.json")).unwrap();
        for name in [
            "Autosave_1_0000",
            "autosaves",
            "../escape",
            "owned-save-recovery",
        ] {
            let mut slot = published_slot("Continue");
            slot.filename = name.into();
            slot.special = SpecialSlot::from_filename(name);
            let receipt = SpecialSaveRecovery {
                slot,
                digest: Sha256::digest(b"payload").into(),
            };
            std::fs::write(
                manager.owned_recovery_path(),
                serde_json::to_vec(&receipt).unwrap(),
            )
            .unwrap();
            assert!(SaveGameManager::load_index(root.path().to_str().unwrap()).is_err());
            assert_eq!(
                std::fs::read(root.path().join("saves.json")).unwrap(),
                before
            );
            assert!(manager.owned_recovery_path().exists());
        }
    }

    #[test]
    fn quick_owned_and_delete_recovery_preserve_each_others_metadata() {
        let root = tempfile::tempdir().unwrap();
        let manager = indexed_store(root.path(), &["Continue", "Savegame_000"]);
        let quick_bytes = b"quick recovery payload";
        let owned_bytes = b"owned recovery payload";
        std::fs::write(root.path().join("QuickSave.json"), quick_bytes).unwrap();
        std::fs::write(root.path().join("Continue.json"), owned_bytes).unwrap();
        let quick = QuickSaveRecovery {
            slots: vec![(
                published_slot("QuickSave"),
                Sha256::digest(quick_bytes).into(),
            )],
        };
        let mut owned_slot = published_slot("Continue");
        owned_slot.player_name = "Recovered owned player".into();
        let owned = SpecialSaveRecovery {
            slot: owned_slot,
            digest: Sha256::digest(owned_bytes).into(),
        };
        let delete = DeleteRecovery {
            filename: SlotName::new("Savegame_000").unwrap(),
        };
        std::fs::write(
            manager.quick_recovery_path(),
            serde_json::to_vec(&quick).unwrap(),
        )
        .unwrap();
        std::fs::write(
            manager.owned_recovery_path(),
            serde_json::to_vec(&owned).unwrap(),
        )
        .unwrap();
        std::fs::write(
            manager.delete_recovery_path(),
            serde_json::to_vec(&delete).unwrap(),
        )
        .unwrap();
        let reopened = SaveGameManager::load_index(root.path().to_str().unwrap()).unwrap();
        assert_eq!(reopened.count(), 2);
        assert!(reopened.find_by_filename("QuickSave").is_some());
        assert_eq!(
            reopened
                .get(reopened.find_by_filename("Continue").unwrap())
                .unwrap()
                .player_name,
            "Recovered owned player"
        );
        assert!(!manager.quick_recovery_path().exists());
        assert!(!manager.owned_recovery_path().exists());
        assert!(!manager.delete_recovery_path().exists());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn delayed_old_write_is_drained_before_delete_and_cannot_resurrect_slot() {
        use std::sync::mpsc;
        let root = tempfile::tempdir().unwrap();
        let mut manager = indexed_store(root.path(), &["Continue"]);
        let path = manager.save_path(0);
        std::fs::write(&path, b"old payload").unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        manager
            .operations
            .start(SlotName::new("Continue").unwrap(), move || {
                started_tx.send(()).unwrap();
                release_rx
                    .recv_timeout(std::time::Duration::from_secs(10))
                    .context("release latch timed out")?;
                std::fs::write(path, b"late old write")?;
                Ok(published_slot("Continue"))
            })
            .unwrap();
        started_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let deletion = std::thread::spawn(move || {
            entered_tx.send(()).unwrap();
            let result = manager.remove(0);
            done_tx.send(result).unwrap();
        });
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        let pending = done_rx.recv_timeout(std::time::Duration::from_millis(50));
        release_tx.send(()).unwrap();
        assert!(matches!(pending, Err(mpsc::RecvTimeoutError::Timeout)));
        done_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap()
            .unwrap();
        deletion.join().unwrap();
        assert!(!root.path().join("Continue.json").exists());
        assert!(
            SaveGameManager::load_index(root.path().to_str().unwrap())
                .unwrap()
                .saves()
                .len()
                == 0
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn queued_writes_complete_in_order_and_retirement_cannot_touch_next_profile() {
        let first_root = tempfile::tempdir().unwrap();
        let second_root = tempfile::tempdir().unwrap();
        let mut manager = SaveGameManager::new(first_root.path().to_str().unwrap().into());
        let (mut engine, _, profiles, mut host) = fresh_save_session("Ordered save");
        let game = game_for_save(&profiles, 17);
        engine.test_set_frame_counter(10);
        manager
            .write_continue_save_background(&mut host, &game, &engine, 17, Some(&profiles), None)
            .unwrap();
        engine.test_set_frame_counter(20);
        manager
            .write_continue_save_background(&mut host, &game, &engine, 17, Some(&profiles), None)
            .unwrap();
        manager.finish_background().unwrap();
        drop(manager);
        let successor = indexed_store(second_root.path(), &["Continue"]);
        std::fs::write(successor.save_path(0), b"successor profile").unwrap();
        let recovered = SaveGameManager::load_index(first_root.path().to_str().unwrap()).unwrap();
        assert_eq!(
            recovered
                .preflight_exact_slot(0)
                .unwrap()
                .engine
                .frame_counter(),
            20
        );
        assert_eq!(
            std::fs::read(successor.save_path(0)).unwrap(),
            b"successor profile"
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "requires LLVM unwinding; run explicitly with robin_rs test codegen-backend=llvm"]
    fn llvm_owned_worker_panic_is_joined_and_reported_once() {
        let root = tempfile::tempdir().unwrap();
        let mut manager = indexed_store(root.path(), &["Continue"]);
        let before = manager.get(0).unwrap().clone();
        let path = manager.save_path(0);
        manager
            .operations
            .start(SlotName::new("Continue").unwrap(), move || {
                struct TerminalMarker(PathBuf);
                impl Drop for TerminalMarker {
                    fn drop(&mut self) {
                        std::fs::write(&self.0, b"unwind completed").unwrap();
                    }
                }
                let _terminal = TerminalMarker(path);
                panic!("injected owned worker panic");
            })
            .unwrap();
        assert!(
            manager
                .finish_background()
                .unwrap_err()
                .to_string()
                .contains("panicked")
        );
        assert!(manager.operations.pending_name().is_none());
        assert_eq!(
            std::fs::read(manager.save_path(0)).unwrap(),
            b"unwind completed"
        );
        assert_eq!(manager.get(0).unwrap(), &before);
        assert!(manager.poll_background().is_err());
        assert!(!manager.poll_background().unwrap());
        assert!(manager.create_draft("blocked".into(), 1).is_err());
    }

    #[test]
    fn emitted_index_preserves_legacy_directory_without_trusting_it() {
        let source = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        indexed_store(source.path(), &["Savegame_000"]);
        let bytes = std::fs::read(source.path().join("saves.json")).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["save_directory"], source.path().to_str().unwrap());
        std::fs::write(destination.path().join("saves.json"), bytes).unwrap();
        let manager = SaveGameManager::load_index(destination.path().to_str().unwrap()).unwrap();
        assert_eq!(
            manager.save_directory(),
            destination.path().to_str().unwrap()
        );
        manager.save_index().unwrap();
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(destination.path().join("saves.json")).unwrap())
                .unwrap();
        assert_eq!(json["save_directory"], destination.path().to_str().unwrap());
    }

    #[test]
    fn autosave_manifest_is_not_a_manual_slot() {
        let root = tempfile::tempdir().unwrap();
        let manifest = root.path().join("autosaves.json");
        std::fs::write(&manifest, b"manifest must survive").unwrap();
        for name in ["autosaves", "AUTOSAVES"] {
            assert!(SlotName::new(name).is_err());
            let mut manager = indexed_store(root.path(), &["Savegame_000"]);
            manager.catalog[0].filename = name.into();
            assert!(manager.remove(0).is_err());
            assert!(manager.save_index().is_err());
            assert_eq!(std::fs::read(&manifest).unwrap(), b"manifest must survive");
        }
    }

    #[test]
    fn failed_new_save_then_delete_other_slot_keeps_index_readable() {
        let root = tempfile::tempdir().unwrap();
        let mut manager = indexed_store(root.path(), &["Savegame_000"]);
        std::fs::write(manager.save_path(0), b"old save").unwrap();
        let draft = manager.create("Failed save".into(), 17);
        let (engine, _assets, profiles, mut host) = fresh_save_session("Failed new draft");
        let game = game_for_save(&profiles, 17);
        // A concurrent target causes the actual no-clobber save path to fail.
        let draft_path = manager.save_path(draft);
        std::fs::write(&draft_path, b"other writer").unwrap();
        assert!(
            manager
                .write_save_from_engine(&mut host, &game, draft, &engine, 17, Some(&profiles), None)
                .is_err()
        );
        manager.remove(0).unwrap();
        let reopened = SaveGameManager::load_index(root.path().to_str().unwrap()).unwrap();
        assert!(reopened.catalog.is_empty());
        assert_eq!(
            manager.count(),
            1,
            "draft remains available for retry in memory"
        );
        manager.remove(0).unwrap();
        assert_eq!(std::fs::read(draft_path).unwrap(), b"other writer");
    }

    #[test]
    fn live_delete_recovers_pending_quick_metadata_before_retiring_receipt() {
        let root = tempfile::tempdir().unwrap();
        let mut manager = indexed_store(root.path(), &["Savegame_000"]);
        let bytes = b"published quick payload";
        std::fs::write(root.path().join("QuickSave.json"), bytes).unwrap();
        let receipt = QuickSaveRecovery {
            slots: vec![(published_slot("QuickSave"), Sha256::digest(bytes).into())],
        };
        std::fs::write(
            manager.quick_recovery_path(),
            serde_json::to_vec(&receipt).unwrap(),
        )
        .unwrap();
        assert!(
            manager.save_index().is_err(),
            "ordinary stale publication must not retire receipt"
        );
        manager.remove(0).unwrap();
        let reopened = SaveGameManager::load_index(root.path().to_str().unwrap()).unwrap();
        assert_eq!(reopened.count(), 1);
        assert_eq!(reopened.catalog[0].filename, "QuickSave");
        assert!(!manager.quick_recovery_path().exists());
    }

    #[test]
    fn slot_names_reject_paths_devices_and_store_control_files() {
        for name in [
            "",
            "..",
            "../Savegame_000",
            "/tmp/save",
            "C:\\save",
            "folder\\save",
            "save.json",
            "saves",
            "CON",
            "aux",
            "Lpt9",
            "COM1",
            "quick-save-recovery",
            "save-delete-recovery",
        ] {
            assert!(SlotName::new(name).is_err(), "accepted {name:?}");
            assert!(serde_json::from_value::<SlotName>(serde_json::json!(name)).is_err());
        }
        for name in [
            "Savegame_000",
            "QuickSave",
            "Autosave_123_0000",
            "custom-save",
        ] {
            assert_eq!(SlotName::new(name).unwrap().as_str(), name);
        }
    }

    #[test]
    fn legacy_index_is_rebound_before_recovery_and_deletion() {
        let old = tempfile::tempdir().unwrap();
        let current = tempfile::tempdir().unwrap();
        let original = indexed_store(old.path(), &["Savegame_000"]);
        std::fs::write(original.save_path(0), b"old payload").unwrap();
        let mut json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(old.path().join("saves.json")).unwrap()).unwrap();
        json["save_directory"] = serde_json::json!(old.path().to_str().unwrap());
        std::fs::write(
            current.path().join("saves.json"),
            serde_json::to_vec(&json).unwrap(),
        )
        .unwrap();
        // A bad receipt in the old root must never be consulted.
        std::fs::write(old.path().join("quick-save-recovery.json"), b"invalid").unwrap();
        std::fs::write(current.path().join("Savegame_000.json"), b"copied payload").unwrap();
        let mut reopened = SaveGameManager::load_index(current.path().to_str().unwrap()).unwrap();
        assert_eq!(reopened.save_directory(), current.path().to_str().unwrap());
        reopened.remove(0).unwrap();
        assert_eq!(
            std::fs::read(original.save_path(0)).unwrap(),
            b"old payload"
        );
        assert!(
            SaveGameManager::load_index(current.path().to_str().unwrap())
                .unwrap()
                .saves()
                .next()
                .is_none()
        );
    }

    #[test]
    fn invalid_and_duplicate_index_names_are_rejected_before_recovery() {
        let root = tempfile::tempdir().unwrap();
        for names in [
            vec!["../outside"],
            vec!["Savegame_000", "Savegame_000"],
            vec!["Savegame_000", "savegame_000"],
        ] {
            let slots: Vec<_> = names
                .iter()
                .map(|name| {
                    let mut slot = published_slot("Savegame_000");
                    slot.filename = (*name).into();
                    slot
                })
                .collect();
            let bytes = serde_json::to_vec(&SaveIndex {
                saves: slots,
                next_id: 0,
                save_directory: root.path().to_str().unwrap().into(),
            })
            .unwrap();
            std::fs::write(root.path().join("saves.json"), &bytes).unwrap();
            std::fs::write(root.path().join("quick-save-recovery.json"), b"invalid").unwrap();
            let error = SaveGameManager::load_index(root.path().to_str().unwrap()).unwrap_err();
            // Basenames are now validated while binding explicit runtime slot
            // state; duplicate checks follow before recovery. Assert the
            // actual rejected invariant rather than one former phase prefix.
            let expected = if names.len() == 1 {
                "invalid save slot basename"
            } else {
                "duplicate save slot name"
            };
            assert!(error.contains(expected), "{error}");
            assert!(!error.contains("recover quick saves"), "{error}");
            assert_eq!(
                std::fs::read(root.path().join("saves.json")).unwrap(),
                bytes
            );
            assert_eq!(
                std::fs::read(root.path().join("quick-save-recovery.json")).unwrap(),
                b"invalid"
            );
        }
    }

    #[test]
    fn corrupt_obsolete_and_unreadable_indexes_do_not_reset_existing_saves() {
        let root = tempfile::tempdir().unwrap();
        let payload = root.path().join("Savegame_000.json");
        let index = root.path().join("saves.json");
        std::fs::write(&payload, b"precious payload").unwrap();
        std::fs::write(&index, b"broken JSON").unwrap();
        assert!(SaveGameManager::load_index(root.path().to_str().unwrap()).is_err());
        let mut slot = published_slot("Savegame_000");
        slot.version = 0;
        std::fs::write(
            &index,
            serde_json::to_vec(&SaveIndex {
                saves: vec![slot],
                next_id: 0,
                save_directory: root.path().to_str().unwrap().into(),
            })
            .unwrap(),
        )
        .unwrap();
        assert!(SaveGameManager::load_index(root.path().to_str().unwrap()).is_err());
        std::fs::remove_file(&index).unwrap();
        // A directory at the index path is a deterministic read error even
        // when tests run as a privileged user (unlike chmod-based fixtures).
        std::fs::create_dir(&index).unwrap();
        assert!(SaveGameManager::load_index(root.path().to_str().unwrap()).is_err());
        assert_eq!(std::fs::read(payload).unwrap(), b"precious payload");
    }

    #[test]
    fn missing_and_stale_indexes_allocate_past_orphans_and_indexed_slots() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("Savegame_000.json"), b"orphan").unwrap();
        std::fs::write(root.path().join("Savegame_001_thumb.png"), b"preview").unwrap();
        let mut missing = SaveGameManager::load_index(root.path().to_str().unwrap()).unwrap();
        assert!(missing.slot_name(missing.count()).is_err());
        let slot = missing.create("New".into(), 1);
        assert_eq!(missing.slot_name(slot).unwrap().as_str(), "Savegame_002");
        let mut stale = indexed_store(root.path(), &["Savegame_002"]);
        let slot = stale.create("Next".into(), 1);
        assert_eq!(stale.slot_name(slot).unwrap().as_str(), "Savegame_003");
        assert_eq!(
            std::fs::read(root.path().join("Savegame_000.json")).unwrap(),
            b"orphan"
        );
    }

    #[test]
    fn delete_is_durable_and_selection_survives_reopen() {
        let root = tempfile::tempdir().unwrap();
        let mut manager = indexed_store(root.path(), &["Savegame_000", "Savegame_001"]);
        std::fs::write(manager.save_path(0), b"first").unwrap();
        std::fs::write(manager.save_path(1), b"selected").unwrap();
        let selection = manager.slot_name(1).unwrap();
        manager.remove(0).unwrap();
        let reopened = SaveGameManager::load_index(root.path().to_str().unwrap()).unwrap();
        let selected = reopened.find_by_filename(selection.as_str()).unwrap();
        assert_eq!(selected, 0);
        assert_eq!(
            std::fs::read(reopened.save_path(selected)).unwrap(),
            b"selected"
        );
        assert!(reopened.find_by_filename("Savegame_000").is_none());
        assert!(!root.path().join("Savegame_000.json").exists());
        assert!(!manager.delete_recovery_path().exists());
    }

    #[test]
    fn delete_cleanup_failure_is_visible_and_recoverable() {
        let root = tempfile::tempdir().unwrap();
        let mut manager = indexed_store(root.path(), &["Savegame_000"]);
        let payload = manager.save_path(0);
        std::fs::create_dir(&payload).unwrap();
        let error = manager.remove(0).unwrap_err();
        assert!(format!("{error:#}").contains("cleanup"));
        assert!(manager.catalog.is_empty());
        assert!(
            manager
                .save_index()
                .unwrap_err()
                .contains("recovery is pending")
        );
        assert!(manager.delete_recovery_path().exists());
        assert!(SaveGameManager::load_index(root.path().to_str().unwrap()).is_err());
        std::fs::remove_dir(payload).unwrap();
        assert!(
            SaveGameManager::load_index(root.path().to_str().unwrap())
                .unwrap()
                .saves()
                .next()
                .is_none()
        );
        assert!(!manager.delete_recovery_path().exists());
    }

    #[test]
    fn delete_index_failure_keeps_payload_and_intent_for_retry() {
        let root = tempfile::tempdir().unwrap();
        let mut manager = indexed_store(root.path(), &["Savegame_000"]);
        let payload = manager.save_path(0);
        std::fs::write(&payload, b"preserved until indexed").unwrap();
        let index = root.path().join("saves.json");
        std::fs::remove_file(&index).unwrap();
        std::fs::create_dir(&index).unwrap();
        assert!(manager.remove(0).is_err());
        assert!(manager.catalog.is_empty());
        assert!(manager.delete_recovery_path().exists());
        assert_eq!(std::fs::read(&payload).unwrap(), b"preserved until indexed");
        std::fs::remove_dir(&index).unwrap();
        let reopened = SaveGameManager::load_index(root.path().to_str().unwrap()).unwrap();
        assert!(reopened.catalog.is_empty());
        assert!(!payload.exists());
    }

    #[test]
    fn failed_delete_intent_does_not_remove_slot_or_payload() {
        let root = tempfile::tempdir().unwrap();
        let mut manager = indexed_store(root.path(), &["Savegame_000"]);
        std::fs::write(manager.save_path(0), b"payload").unwrap();
        std::fs::create_dir(manager.delete_recovery_path()).unwrap();
        assert!(manager.remove(0).is_err());
        assert_eq!(manager.count(), 1);
        assert_eq!(std::fs::read(manager.save_path(0)).unwrap(), b"payload");
    }

    #[test]
    fn quick_and_delete_receipts_recover_together_without_losing_quick_metadata() {
        let root = tempfile::tempdir().unwrap();
        let manager = indexed_store(root.path(), &["Savegame_000"]);
        std::fs::write(manager.save_path(0), b"delete me").unwrap();
        let quick_bytes = b"digest bound quick payload";
        std::fs::write(root.path().join("QuickSave.json"), quick_bytes).unwrap();
        let quick = QuickSaveRecovery {
            slots: vec![(
                published_slot("QuickSave"),
                Sha256::digest(quick_bytes).into(),
            )],
        };
        std::fs::write(
            manager.quick_recovery_path(),
            serde_json::to_vec(&quick).unwrap(),
        )
        .unwrap();
        let delete = DeleteRecovery {
            filename: SlotName::new("Savegame_000").unwrap(),
        };
        std::fs::write(
            manager.delete_recovery_path(),
            serde_json::to_vec(&delete).unwrap(),
        )
        .unwrap();
        let recovered = SaveGameManager::load_index(root.path().to_str().unwrap()).unwrap();
        assert_eq!(recovered.count(), 1);
        assert_eq!(recovered.catalog[0].filename, "QuickSave");
        assert!(!manager.quick_recovery_path().exists());
        assert!(!manager.delete_recovery_path().exists());
        assert_eq!(
            std::fs::read(root.path().join("QuickSave.json")).unwrap(),
            quick_bytes
        );
    }

    #[test]
    fn corrupt_quick_receipt_blocks_open_and_preserves_index_and_payload() {
        let root = tempfile::tempdir().unwrap();
        let manager = indexed_store(root.path(), &["Savegame_000"]);
        std::fs::write(manager.save_path(0), b"payload").unwrap();
        let before = std::fs::read(root.path().join("saves.json")).unwrap();
        std::fs::write(manager.quick_recovery_path(), b"broken").unwrap();
        assert!(
            SaveGameManager::load_index(root.path().to_str().unwrap())
                .unwrap_err()
                .contains("recover quick saves")
        );
        assert_eq!(
            std::fs::read(root.path().join("saves.json")).unwrap(),
            before
        );
        assert_eq!(std::fs::read(manager.save_path(0)).unwrap(), b"payload");
    }

    #[test]
    fn new_manual_save_cannot_clobber_a_payload_created_after_selection() {
        let root = tempfile::tempdir().unwrap();
        let mut manager = SaveGameManager::new(root.path().to_str().unwrap().into());
        let slot = manager.create("New".into(), 17);
        let (engine, _assets, profiles, mut host) = fresh_save_session("Concurrent writer test");
        let game = game_for_save(&profiles, 17);
        std::fs::write(manager.save_path(slot), b"concurrent writer").unwrap();
        assert!(
            manager
                .write_save_from_engine(&mut host, &game, slot, &engine, 17, Some(&profiles), None)
                .is_err()
        );
        assert_eq!(
            std::fs::read(manager.save_path(slot)).unwrap(),
            b"concurrent writer"
        );
        assert!(manager.catalog[slot].timestamp.is_empty());
    }
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
                proto_level_filename: format!("Map_{mission_id}"),
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
            crate::player_profile_store::PlayerProfileStore::for_directory(&save_root),
            engine_api::GlobalOptions::default(),
            players,
            KeyConfigStore::new(save_root),
            None,
        )
        .expect("complete test application context");
        let host = Host::new(application_context.try_into().unwrap(), 800.0, 600.0).unwrap();
        (engine, assets, profiles, host)
    }

    fn game_for_save(profiles: &ProfileManager, mission_id: u32) -> Game {
        let profile = profiles
            .missions
            .iter()
            .find(|profile| profile.id == mission_id)
            .expect("test mission profile");
        let mut game = Game::default();
        game.set_mission_assets(
            robin_engine::mission_assets::MissionAssetDescriptor::built_in(
                profile.mission_filename.clone(),
                profile.proto_level_filename.clone(),
                profile.proto_level_filename.clone(),
            )
            .unwrap(),
        )
        .unwrap();
        game
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
        let before = mgr.saves().cloned().collect::<Vec<_>>();

        let missing_source = mgr.copy_display_metadata(usize::MAX, slot).unwrap_err();
        // Stable-identity lookup rejects the missing source before attempting
        // to read its explicit lifecycle state or copy any metadata.
        assert_eq!(
            missing_source.to_string(),
            format!("missing save slot {}", usize::MAX)
        );

        let missing_destination = mgr.copy_display_metadata(slot, usize::MAX).unwrap_err();
        assert!(
            missing_destination
                .to_string()
                .contains("cannot copy metadata to missing save slot")
        );
        assert!(mgr.saves().eq(before.iter()));
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

        let json = serde_json::to_string(&SaveIndex {
            saves: mgr.catalog.iter().cloned().collect(),
            next_id: mgr.next_id,
            save_directory: mgr.save_directory.clone(),
        })
        .unwrap();
        let mgr2: SaveIndex = serde_json::from_str(&json).unwrap();
        assert_eq!(mgr2.saves.len(), 2);
        assert_eq!(mgr2.saves[0].text, "Save 1");
        assert_eq!(mgr2.saves[1].mission_id, 20);
    }

    #[test]
    fn auto_incrementing_filenames() {
        let mut mgr = SaveGameManager::new("/tmp/test_saves".into());
        mgr.create("A".into(), 1);
        mgr.create("B".into(), 2);
        mgr.create("C".into(), 3);
        assert_eq!(mgr.catalog[0].filename, "Savegame_000");
        assert_eq!(mgr.catalog[1].filename, "Savegame_001");
        assert_eq!(mgr.catalog[2].filename, "Savegame_002");
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
    fn session_restart_restores_persisted_state_without_filesystem_or_json_identity() {
        let directory = tempfile::tempdir().unwrap();
        let blocked_root = directory.path().join("not-a-directory");
        std::fs::write(&blocked_root, b"filesystem writes must fail here").unwrap();
        let mut manager = SaveGameManager::new(blocked_root.to_string_lossy().into_owned());
        let (mut engine, assets, profiles, mut host) = fresh_save_session("Session Restart");
        let mut game = game_for_save(&profiles, 17);
        host.frontend.input.draw_hidden = true;
        game.persistent.campaign_map_displayed = true;
        engine.test_set_frame_counter(42);
        manager
            .write_session_restart(&host, &game, &engine, 17, Some(&profiles))
            .unwrap();
        assert!(manager.has_restart_save());
        let (index, prepared) = manager.preflight_restart_save().unwrap().unwrap();
        assert_eq!(manager.get(index).unwrap().player_name, "Session Restart");
        assert!(matches!(
            prepared.replay_identity().unwrap(),
            ReplaySaveIdentity::SessionRestart(_)
        ));
        assert_eq!(
            prepared.session_identity(),
            manager.restart_session_identity()
        );
        assert_eq!(
            manager
                .preflight_load(Some(index))
                .unwrap()
                .unwrap()
                .1
                .replay_identity()
                .unwrap(),
            prepared.replay_identity().unwrap()
        );
        assert!(!manager.save_path(index).exists());

        // A real disk round-trip provides the reference persisted projection.
        let disk_path = directory.path().join("reference.json");
        prepared.write_to(&disk_path).unwrap();
        let disk = GameSaveFile::read_from(&disk_path).unwrap();
        assert_eq!(
            GameSaveFile::replay_identity(&prepared).unwrap(),
            disk.replay_identity().unwrap()
        );
        let decoded: PreparedGameSave =
            serde_json::from_str(&serde_json::to_string(&prepared).unwrap()).unwrap();
        assert_eq!(decoded.session_identity(), None);
        assert_eq!(
            decoded.replay_identity().unwrap(),
            disk.replay_identity().unwrap()
        );

        engine.test_set_frame_counter(99);
        host.frontend.input.draw_hidden = false;
        game.persistent.campaign_map_displayed = false;
        let mut disk_engine = engine.clone();
        let mut disk_host = Host::scratch(800.0, 600.0);
        let mut disk_game = Game::default();
        disk.apply_to_with_game(&mut disk_engine, &mut disk_host, &mut disk_game, &assets)
            .unwrap();
        prepared
            .clone()
            .apply_to_with_game(&mut engine, &mut host, &mut game, &assets)
            .unwrap();
        assert!(host.frontend.input.draw_hidden);
        assert!(game.persistent.campaign_map_displayed);
        assert_eq!(
            crate::save_file::GameRuntimeSnapshot::identity_of_live(&engine, &host, &game).unwrap(),
            crate::save_file::GameRuntimeSnapshot::identity_of_live(
                &disk_engine,
                &disk_host,
                &disk_game
            )
            .unwrap()
        );

        // Index serialization and new profile managers cannot resurrect memory
        // checkpoints or their process-local identity authority.
        let index_data = SaveIndex {
            saves: manager.catalog.iter().cloned().collect(),
            next_id: manager.next_id,
            save_directory: manager.save_directory.clone(),
        };
        let index_data: SaveIndex =
            serde_json::from_str(&serde_json::to_string(&index_data).unwrap()).unwrap();
        let mut reopened = SaveGameManager::new(blocked_root.to_str().unwrap().into());
        reopened.next_id = index_data.next_id;
        for slot in index_data.saves {
            reopened.insert_test_slot(slot, SlotState::Published);
        }
        assert!(!reopened.has_restart_save());
        assert_eq!(reopened.restart_session_identity(), None);
        let other_profile = SaveGameManager::new(
            directory
                .path()
                .join("other-profile")
                .to_string_lossy()
                .into_owned(),
        );
        assert!(!other_profile.has_restart_save());

        let old_identity = prepared.replay_identity().unwrap();
        manager
            .write_session_restart(&host, &game, &engine, 17, Some(&profiles))
            .unwrap();
        assert_ne!(manager.restart_session_identity(), Some(old_identity));
        assert_eq!(prepared.replay_identity().unwrap(), old_identity);
        manager.remove(index).unwrap();
        assert!(!manager.has_restart_save());
    }

    #[test]
    fn failed_session_restart_capture_invalidates_previous_checkpoint() {
        let directory = tempfile::tempdir().unwrap();
        let mut manager = SaveGameManager::new(directory.path().to_string_lossy().into_owned());
        let (engine, _, profiles, host) = fresh_save_session("Failed Session Restart");
        let game = game_for_save(&profiles, 17);
        manager
            .write_session_restart(&host, &game, &engine, 17, Some(&profiles))
            .unwrap();
        let missing_profile_host = Host::scratch(800.0, 600.0);
        assert!(
            manager
                .write_session_restart(&missing_profile_host, &game, &engine, 17, Some(&profiles))
                .is_err()
        );
        assert!(!manager.has_restart_save());
        assert!(manager.preflight_restart_save().unwrap().is_none());
        assert_eq!(manager.restart_session_identity(), None);
    }

    #[test]
    fn engine_round_trip_via_manager() {
        use tempfile::tempdir;

        let tmp = tempdir().unwrap();
        let mut mgr = SaveGameManager::new(tmp.path().to_string_lossy().into_owned());

        // Build a live engine with some distinctive state.
        let (mut engine, assets, mut profiles, mut host) = fresh_save_session("Alice");
        let game = game_for_save(&profiles, 17);
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
        assert_eq!(mgr.catalog[idx].mission_name, "Mission 17");
        assert_eq!(mgr.catalog[idx].player_profile_id, Some(0));
        assert_eq!(mgr.catalog[idx].player_name, "Alice");
        profiles.missions[2].mission_name = "Mission 17 (renamed)".into();
        assert_eq!(mgr.catalog[idx].mission_name, "Mission 17");
        assert_eq!(decoded.header.provenance.mission_name, "Mission 17");
        mgr.catalog[idx].mission_id = 99;
        assert!(
            mgr.validate_slot_identity(idx, &decoded)
                .unwrap_err()
                .to_string()
                .contains("metadata does not match decoded payload")
        );
        mgr.catalog[idx].mission_id = 17;
        mgr.catalog[idx].player_name = "Mallory".into();
        assert!(
            mgr.validate_slot_identity(idx, &decoded)
                .unwrap_err()
                .to_string()
                .contains("provenance does not match")
        );
        mgr.catalog[idx].player_name = "Alice".into();

        host.application_context()
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
        assert_eq!(mgr.catalog[idx].player_name, "Alice");
        assert_eq!(mgr.catalog[continue_idx].player_name, "Renamed Alice");
        assert_eq!(
            mgr.catalog[continue_idx].mission_name,
            "Mission 17 (renamed)"
        );

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
        let game = game_for_save(&profiles, 17);
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
        let game = game_for_save(&profiles, 1);
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
        let game = game_for_save(&profiles, 3);

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
        assert_eq!(mgr.catalog[quick_idx].player_name, "Alice");
        assert_eq!(mgr.catalog[ex_idx].player_name, "Alice");
    }

    #[test]
    fn quick_save_recovers_payloads_after_index_publication_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let directory = tmp.path().to_string_lossy().into_owned();
        let mut manager = SaveGameManager::new(directory.clone());
        let (mut engine, _, profiles, mut host) = fresh_save_session("Recovery");
        let game = game_for_save(&profiles, 3);
        engine.test_set_frame_counter(1);
        manager
            .write_quick_save(&mut host, &game, &engine, 3, Some(&profiles), None)
            .unwrap();
        let index_path = tmp.path().join("saves.json");
        std::fs::rename(&index_path, tmp.path().join("old-index.json")).unwrap();
        std::fs::create_dir(&index_path).unwrap();
        engine.test_set_frame_counter(2);
        assert!(
            manager
                .write_quick_save(&mut host, &game, &engine, 3, Some(&profiles), None)
                .is_err()
        );
        std::fs::remove_dir(&index_path).unwrap();
        let mut recovered = SaveGameManager::load_index(&directory).unwrap();
        for (name, frame) in [(special_slots::QUICK, 2), (special_slots::EX_QUICK, 1)] {
            let index = recovered.find_by_filename(name).expect("recovered slot");
            let save = GameSaveFile::read_from(&recovered.save_path(index)).unwrap();
            assert_eq!(save.engine.frame_counter(), frame);
            assert_eq!(recovered.catalog[index].player_name, "Recovery");
        }
        assert!(!recovered.quick_recovery_path().exists());
        let quick = recovered.find_by_filename(special_slots::QUICK).unwrap();
        recovered.get_mut(quick).unwrap().text = "Edited after recovery".to_owned();
        recovered.save_index().unwrap();
        let reopened = SaveGameManager::load_index(&directory).unwrap();
        let quick = reopened.find_by_filename(special_slots::QUICK).unwrap();
        assert_eq!(reopened.get(quick).unwrap().text, "Edited after recovery");
    }

    #[test]
    fn quick_save_preparation_failure_does_not_rotate_existing_payloads() {
        let tmp = tempfile::tempdir().unwrap();
        let mut manager = SaveGameManager::new(tmp.path().to_string_lossy().into_owned());
        let (engine, _, profiles, mut host) = fresh_save_session("Preparation");
        let game = game_for_save(&profiles, 3);
        manager
            .write_quick_save(&mut host, &game, &engine, 3, Some(&profiles), None)
            .unwrap();
        let quick = manager.save_path(manager.find_by_filename(special_slots::QUICK).unwrap());
        let before = std::fs::read(&quick).unwrap();
        assert!(
            manager
                .write_quick_save(&mut host, &game, &engine, 3, None, None)
                .is_err()
        );
        assert_eq!(std::fs::read(&quick).unwrap(), before);
        assert!(!tmp.path().join("ExQuickSave.json").exists());
    }

    #[test]
    fn missing_rotation_source_preserves_previous_payload() {
        let tmp = tempfile::tempdir().unwrap();
        let mut manager = SaveGameManager::new(tmp.path().to_string_lossy().into_owned());
        let quick = manager
            .ensure_special_slot(special_slots::QUICK, "Quick Save")
            .unwrap();
        let previous = manager
            .ensure_special_slot(special_slots::EX_QUICK, "Previous Quick Save")
            .unwrap();
        let previous_path = manager.save_path(previous);
        std::fs::write(&previous_path, b"previous save payload").unwrap();
        assert!(manager.copy_files(quick, previous).is_err());
        assert_eq!(
            std::fs::read(previous_path).unwrap(),
            b"previous save payload"
        );
    }

    #[test]
    fn save_write_rejects_missing_profiles_or_active_player() {
        let tmp = tempfile::tempdir().unwrap();
        let mut mgr = SaveGameManager::new(tmp.path().to_string_lossy().into_owned());
        let (engine, _assets, profiles, _host) = fresh_save_session("Alice");
        let mut scratch_host = Host::scratch(800.0, 600.0);
        let game = game_for_save(&profiles, 1);
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
            mgr.catalog[slot].timestamp = timestamp.into();
        }
        mgr.sort_by_time();
        assert_eq!(
            mgr.catalog
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
        let error = serde_json::from_value::<SaveIndex>(json).unwrap_err();
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
        let game = game_for_save(&profiles, 1);

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
        let root = tempfile::tempdir().unwrap();
        let mut mgr = SaveGameManager::new(root.path().to_str().unwrap().into());
        mgr.create("A".into(), 1);
        mgr.create_with_filename("Continue".into(), "Continue".into(), 0);
        assert_eq!(mgr.count(), 2);
        mgr.remove_by_filename("Continue").unwrap();
        assert_eq!(mgr.count(), 1);
        assert_eq!(mgr.catalog[0].filename, "Savegame_000");
    }

    #[test]
    fn manual_remove_apis_refuse_autosaves() {
        let mut mgr = SaveGameManager::new("/tmp/test_saves".into());
        mgr.insert_test_slot(
            SaveGame::new("Autosave_1_0000".into(), "Mission".into(), 1),
            SlotState::Published,
        );
        assert!(mgr.remove(0).is_err());
        assert!(mgr.remove_by_filename("Autosave_1_0000").is_err());
        assert_eq!(mgr.count(), 1);
    }
}
