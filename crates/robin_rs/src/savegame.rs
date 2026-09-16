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
//! their separate storage backend in [`autosave_store`].

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

pub(crate) mod autosave_store;
mod background;
mod catalog;
mod index;
mod load;
mod operation;
mod persistence;
mod publication;
mod recovery;
mod slots;
#[cfg(test)]
mod tests;
mod writes;
use catalog::SlotCatalog;

/// A portable basename, never a path. Deserialization applies the same checks
/// as runtime construction so persisted identities cannot escape their store.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SlotName(String);

impl TryFrom<String> for SlotName {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Self::validate(&value)?;
        Ok(Self(value))
    }
}

impl SlotName {
    fn validate(value: &str) -> Result<(), String> {
        if value.is_empty()
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
            || [
                "saves",
                "autosaves",
                "quick-save-recovery",
                "save-delete-recovery",
                "owned-save-recovery",
                "con",
                "prn",
                "aux",
                "nul",
            ]
            .iter()
            .any(|reserved| value.eq_ignore_ascii_case(reserved))
            || (value.len() == 4
                && (value[..3].eq_ignore_ascii_case("com")
                    || value[..3].eq_ignore_ascii_case("lpt"))
                && matches!(value.as_bytes()[3], b'1'..=b'9'))
        {
            return Err(format!("invalid save slot basename {value:?}"));
        }
        Ok(())
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
    /// Elapsed simulation seconds; absent for Sherwood and older catalog entries.
    // TODO: Backfill older catalogs from payloads without blocking the save picker.
    #[serde(default)]
    pub mission_elapsed_seconds: Option<u32>,
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
            mission_elapsed_seconds: None,
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

    /// Clone this snapshot for another slot, preserving that slot's identity
    /// and label. Lifecycle state remains the catalog owner's responsibility.
    fn cloned_for_slot(&self, destination: &Self) -> Self {
        Self {
            filename: destination.filename.clone(),
            text: destination.text.clone(),
            special: destination.special,
            ..self.clone()
        }
    }

    /// Refresh snapshot fields without changing slot identity or its label.
    pub(crate) fn update_snapshot_metadata(
        &mut self,
        header: &SaveHeader,
        campaign: &engine_campaign::Campaign,
        profiles: &ProfileManager,
    ) {
        self.mission_id = header.mission_id;
        self.version = header.version;
        self.timestamp = header.timestamp_unix.to_string();
        self.multiplayer_diagnostic = header.multiplayer_diagnostic;
        self.mission_name = header.provenance.mission_name.clone();
        self.player_profile_id = Some(header.provenance.player_profile_id);
        self.player_name = header.provenance.player_name.clone();
        self.update_campaign_metadata(campaign, profiles);
    }

    fn update_campaign_metadata(
        &mut self,
        campaign: &engine_campaign::Campaign,
        profiles: &ProfileManager,
    ) {
        let mission = campaign
            .get_mission(self.mission_id, profiles)
            .expect("saved mission must exist in the campaign");
        self.mission_elapsed_seconds = (mission.profile(profiles).location
            != robin_engine::profiles::MissionLocation::Sherwood)
            .then(|| campaign.get_value(CampaignValue::MissionLength).max(0) as u32);
        self.missions_done = Some(campaign.get_number_of_missions_done());
        self.missions_total = Some(campaign.missions.len());
        self.gang_size = Some(campaign.gang_indices.len());
        self.ransom = Some(campaign.values[CampaignValue::Ransom]);
        self.blazons = Some(campaign.values[CampaignValue::Blazon]);
        self.amulets = Some(campaign.values[CampaignValue::Amulets]);
        self.campaign_progress = Some(campaign.get_progression(profiles));
    }

    pub(crate) fn validate_published_metadata(&self) -> Result<()> {
        SlotName::validate(&self.filename).map_err(anyhow::Error::msg)?;
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
    let (player_id, player_name) = host
        .application_context()
        .with_active_profile(|player| (player.id, player.name.clone()))
        .map_err(anyhow::Error::msg)
        .context("save requires an active player profile")?;
    SaveProvenance::new(mission_name, player_id, player_name)
}

/// Manages a collection of save games for a player profile.
// Runtime directory authority must never be reconstructed by serde.
#[derive(Debug)]
pub struct SaveGameManager {
    storage_disabled: bool,
    catalog: SlotCatalog,
    operations: operation::SaveOperationOwner,
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

robin_util::deny_deserialize!(CommittedSave, "save commit evidence is process-local");

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
struct SaveIndex<Slot = SaveGame> {
    saves: Vec<Slot>,
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
            storage_disabled: false,
            catalog: SlotCatalog::default(),
            operations: Default::default(),
            operation_error: None,
            operation_error_reported: false,
            save_directory,
            next_id: 0,
            session_restart: None,
        }
    }

    /// Replay save boundaries are owned by the playback timeline in memory.
    /// This manager has no authority to access player saves, even on request.
    pub(crate) fn disabled() -> Self {
        let mut manager = Self::new(String::new());
        manager.storage_disabled = true;
        manager
    }

    pub(crate) fn require_storage(&self) -> Result<()> {
        anyhow::ensure!(
            !self.storage_disabled,
            "save storage is disabled during replay playback"
        );
        Ok(())
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
        manager
            .load_autosaves()
            .map_err(|error| format!("load autosave manifest: {error:#}"))?;
        Ok(manager)
    }

    pub fn save_directory(&self) -> &str {
        self.require_storage()
            .expect("save directory requires storage authority");
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
}

fn validate_slot_names(saves: &[SaveGame]) -> Result<()> {
    let mut names = std::collections::HashSet::new();
    for slot in saves {
        SlotName::validate(&slot.filename).map_err(anyhow::Error::msg)?;
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
