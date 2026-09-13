//! Load-side queries: resume/load target selection, payload preflight,
//! identity validation, thumbnails, and payload existence.
use super::*;

impl SaveGameManager {
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
            self.catalog.state_at(index)? != SlotState::Draft,
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
            return autosave_store::read_payload(&self.save_directory, &slot.filename)
                .and_then(|payload| {
                    autosave_store::validate_metadata_payload_binding(slot, &payload)?;
                    Ok(PreparedGameSave::from(payload))
                })
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

    /// Load the thumbnail for a slot if one exists on disk.
    pub fn load_thumbnail(&self, index: usize) -> Option<Thumbnail> {
        let slot = self.catalog.get(index)?;
        let result = if slot.is_autosave() {
            autosave_store::read_thumbnail(&self.save_directory, &slot.filename)
        } else {
            Thumbnail::read_optional_from(&self.thumb_path(index))
        };
        match result {
            Ok(thumbnail) => thumbnail,
            Err(error) => {
                tracing::warn!(
                    filename = slot.filename,
                    "failed to load save thumbnail: {error:#}"
                );
                None
            }
        }
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
        let slot = self.catalog.get(index).expect("invalid save slot identity");
        if self
            .operations
            .pending_name()
            .is_some_and(|name| name.as_str() == slot.filename)
        {
            return false; // A queued publication is explicitly not loadable yet.
        }
        if self
            .catalog
            .state_at(index)
            .expect("validated runtime slot state")
            == SlotState::Draft
        {
            return false;
        }
        if slot.is_restart() {
            if self.session_restart.is_some() {
                return true;
            }
            #[cfg(target_arch = "wasm32")]
            return false;
        }

        if slot.is_autosave() {
            return match autosave_store::payload_exists(&self.save_directory, &slot.filename) {
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
        let path = self.save_path(index);
        match path.try_exists() {
            Ok(exists) => exists,
            Err(error) => {
                tracing::error!(path = %path.display(), "could not check save payload existence: {error}");
                false
            }
        }
    }
}
