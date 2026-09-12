//! Capture and publication entry points. Named operations preserve their distinct
//! replay-boundary, quick-slot rotation, and synchronous/background ordering.
use super::*;

impl SaveGameManager {
    /// Save the current engine state to the "Continue" auto-save slot.
    /// Called after every successful manual save and at mission quit.
    ///
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
            host,
            game,
            engine,
            mission_id,
            profiles,
            thumbnail,
        )
    }

    /// Mirror a successfully loaded save without capturing a new replay marker.
    pub(crate) fn write_loaded_continue_background(
        &mut self,
        mut save: GameSaveFile,
        profiles: &ProfileManager,
        thumbnail: Option<&Thumbnail>,
    ) -> Result<SaveWriteStatus> {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (&mut save, profiles, thumbnail);
            anyhow::bail!(
                "browser manual special-save persistence is unavailable; use durable autosaves"
            );
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.finish_background()?;
            self.ensure_no_pending_delete()?;
            self.reconcile_quick_slots()?;
            let index = self.ensure_special_slot(save_file::special_slots::CONTINUE, "Continue")?;
            save.header.display_text = self.catalog[index].text.clone();
            self.queue_special_save(index, save, profiles, thumbnail)
        }
    }

    /// Save the current engine state to the "QuickSave" slot.
    /// The previous quick save (if any) is rotated to "ExQuickSave".
    pub(super) fn write_quick_save_payload(
        &mut self,
        host: &mut Host,
        game: &crate::game::Game,
        engine: &Engine,
        mission_id: u32,
        profiles: Option<&ProfileManager>,
        thumbnail: Option<&Thumbnail>,
    ) -> Result<save_file::SerializedSave> {
        Self::require_synchronous_storage()?;
        self.finish_background()?;
        self.ensure_no_pending_delete()?;
        self.reconcile_quick_slots()?;
        // Validate and serialize before touching either published quick slot.
        // A failed capture must not rotate a player's recoverable saves.
        let quick_index = self.find_by_filename(save_file::special_slots::QUICK);
        let mut current = quick_index
            .map(|index| self.catalog[index].clone())
            .unwrap_or_else(|| {
                SaveGame::new(
                    save_file::special_slots::QUICK.to_owned(),
                    "Quick Save".to_owned(),
                    mission_id,
                )
            });
        let mut save = capture_save(
            host,
            game,
            engine,
            mission_id,
            profiles,
            current.text.clone(),
        )?;
        host.application_context()
            .replay_recording()
            .attach_save_boundary(&mut save)?;
        let payload = save_file::SerializedSave::new(&save)?;
        let bytes = payload.encode(&current.text)?;
        current.update_snapshot_metadata(
            &save.header,
            save.engine.campaign(),
            profiles.context("quick save requires profiles")?,
        );
        let mut recovery = QuickSaveRecovery {
            slots: vec![(current, Sha256::digest(&bytes).into())],
        };
        if let Some(index) = quick_index
            && self.slot_file_exists(index)
        {
            let previous_digest = recovery::payload_digest(&self.save_path(index))
                .context("preparing previous quick save")?
                .context("previous quick save disappeared during preparation")?;
            let mut previous = self.catalog[index].clone();
            previous.filename = save_file::special_slots::EX_QUICK.to_owned();
            previous.special = Some(SpecialSlot::ExQuickSave);
            previous.text = self
                .find_by_filename(save_file::special_slots::EX_QUICK)
                .map(|index| self.catalog[index].text.clone())
                .unwrap_or_else(|| "Previous Quick Save".to_owned());
            previous.validate_published_metadata()?;
            recovery.slots.push((previous, previous_digest));
        }
        save_file::atomic_write(&self.quick_recovery_path(), &serde_json::to_vec(&recovery)?)?;
        // Rotate: QuickSave → ExQuickSave
        if let Some(quick_idx) = quick_index
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
        Ok(payload)
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
                host,
                game,
                engine,
                mission_id,
                profiles,
                thumbnail,
            )
        }
    }

    pub(super) fn write_special_save_background(
        &mut self,
        filename: &str,
        display_text: &str,
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
            // Capture (clone) on the main thread before starting the writer.
            let mut save = capture_save(host, game, engine, mission_id, profiles, display_text)?;
            host.application_context()
                .replay_recording()
                .attach_save_boundary(&mut save)?;
            self.queue_special_save(
                idx,
                save,
                profiles.context("save metadata requires profiles")?,
                thumbnail,
            )
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn queue_special_save(
        &mut self,
        idx: usize,
        save: GameSaveFile,
        profiles: &ProfileManager,
        thumbnail: Option<&Thumbnail>,
    ) -> Result<SaveWriteStatus> {
        let path = self.save_path(idx);
        let thumb_data = thumbnail.cloned();
        let thumb_path = self.thumb_path(idx);
        let mut metadata = self.catalog[idx].clone();
        metadata.update_snapshot_metadata(&save.header, save.engine.campaign(), profiles);
        metadata.validate_published_metadata()?;
        let name = self.slot_name(idx).map_err(anyhow::Error::msg)?;
        let recovery_path = self.owned_recovery_path();
        self.operations.start(name, move || {
            save.validate_current_schema()?;
            let bytes = serde_json::to_vec_pretty(&save).context("serialize owned save payload")?;
            let receipt = SpecialSaveRecovery {
                slot: metadata,
                digest: Sha256::digest(&bytes).into(),
            };
            persistence::publish_payload(&recovery_path, &path, &receipt, &bytes, true)?;
            if let Some(thumb) = thumb_data
                && let Err(err) = thumb.write_to(&thumb_path)
            {
                tracing::warn!("Owned save thumbnail failed (payload completed): {err:#}");
            }
            Ok(receipt.slot)
        })?;
        Ok(SaveWriteStatus::Queued)
    }

    /// Capture and publish a session-only Restart atomically. No disk index is
    /// written: this checkpoint is intentionally gone with its owning manager.
    #[cfg(any(target_arch = "wasm32", test))]
    pub(super) fn write_session_restart(
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
        let mut save = PreparedGameSave::capture_session_restart(engine, host, game, header)?;
        save.record_replay_boundary(&host.application_context().replay_recording())?;
        let mut slot = SaveGame::new(
            save_file::special_slots::RESTART.into(),
            "Restart Point".into(),
            mission_id,
        );
        slot.update_snapshot_metadata(
            &save.header,
            save.engine.campaign(),
            profiles.context("restart requires mission profiles")?,
        );
        // This runtime-only checkpoint must not consult a filesystem receipt
        // or require a writable desktop save root.
        self.catalog.upsert(slot, SlotState::Session)?;
        self.session_restart = Some(std::sync::Arc::new(save));
        Ok(())
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

    /// Write a full save file (engine + campaign) to the given slot.
    ///
    /// The caller must supply the live engine; the engine must have an
    /// active campaign (panics otherwise).  If `thumbnail` is `Some`, it
    /// is also written to the sibling thumb file alongside the payload.
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
        .map(|(committed, _)| committed)
    }

    /// Write a local multiplayer diagnostic. It is deliberately tagged in
    /// both the payload and slot index and is never suitable as session state.
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
        .map(|(committed, _)| committed)
    }

    pub(super) fn write_save_from_engine_with_diagnostic(
        &mut self,
        host: &mut Host,
        game: &crate::game::Game,
        index: usize,
        engine: &Engine,
        mission_id: u32,
        profiles: Option<&ProfileManager>,
        thumbnail: Option<&Thumbnail>,
        multiplayer_diagnostic: bool,
    ) -> Result<(CommittedSave, save_file::SerializedSave)> {
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
        let mut save = capture_save(host, game, engine, mission_id, profiles, display_text)?;
        save.header.multiplayer_diagnostic = multiplayer_diagnostic;
        let mut metadata = self.catalog[index].clone();
        metadata.update_snapshot_metadata(
            &save.header,
            save.engine.campaign(),
            profiles.context("save metadata requires mission profiles")?,
        );
        metadata.validate_published_metadata()?;
        save.validate_current_schema()?;
        host.application_context()
            .replay_recording()
            .attach_save_boundary(&mut save)?;
        let payload = save_file::SerializedSave::new(&save)?;
        let bytes = payload.encode(&metadata.text)?;
        let committed = self.commit_synchronous(index, metadata, &bytes, thumbnail)?;
        Ok((committed, payload))
    }

    /// Publish the selected slot first, then mirror the same capture if its
    /// slot policy requires it. Some reports only mirror failure; Err means
    /// the primary publication failed.
    pub(crate) fn write_save_and_continue(
        &mut self,
        host: &mut Host,
        game: &crate::game::Game,
        index: usize,
        engine: &Engine,
        mission_id: u32,
        profiles: Option<&ProfileManager>,
        thumbnail: Option<&Thumbnail>,
    ) -> Result<Option<String>> {
        let (committed, payload) = self.write_save_from_engine_with_diagnostic(
            host, game, index, engine, mission_id, profiles, thumbnail, false,
        )?;
        let index = self.resolve_handle(committed.slot())?;
        if matches!(
            self.catalog[index].special,
            Some(SpecialSlot::Continue | SpecialSlot::Restart)
        ) {
            return Ok(None);
        }
        Ok(self
            .mirror_captured_continue(index, &payload, thumbnail)
            .err()
            .map(|e| format!("{e:#}")))
    }

    pub(crate) fn write_quick_save_and_continue(
        &mut self,
        host: &mut Host,
        game: &crate::game::Game,
        engine: &Engine,
        mission_id: u32,
        profiles: Option<&ProfileManager>,
        thumbnail: Option<&Thumbnail>,
    ) -> Result<Option<String>> {
        let payload =
            self.write_quick_save_payload(host, game, engine, mission_id, profiles, thumbnail)?;
        let index = self
            .find_by_filename(save_file::special_slots::QUICK)
            .context("published quick save lost its slot")?;
        Ok(self
            .mirror_captured_continue(index, &payload, thumbnail)
            .err()
            .map(|e| format!("{e:#}")))
    }

    #[cfg(test)]
    pub(super) fn write_quick_save(
        &mut self,
        host: &mut Host,
        game: &crate::game::Game,
        engine: &Engine,
        mission_id: u32,
        profiles: Option<&ProfileManager>,
        thumbnail: Option<&Thumbnail>,
    ) -> Result<()> {
        self.write_quick_save_payload(host, game, engine, mission_id, profiles, thumbnail)
            .map(|_| ())
    }

    pub(super) fn mirror_captured_continue(
        &mut self,
        source: usize,
        payload: &save_file::SerializedSave,
        thumbnail: Option<&Thumbnail>,
    ) -> Result<()> {
        let index = self.ensure_special_slot(save_file::special_slots::CONTINUE, "Continue")?;
        let metadata = self.catalog[source].cloned_for_slot(&self.catalog[index]);
        let bytes = payload.encode(&metadata.text)?;
        self.commit_synchronous(index, metadata, &bytes, thumbnail)
            .map(|_| ())
    }
}

/// Capture only: each publication workflow owns when to attach its replay boundary.
fn capture_save(
    host: &Host,
    game: &crate::game::Game,
    engine: &Engine,
    mission_id: u32,
    profiles: Option<&ProfileManager>,
    display_text: String,
) -> Result<GameSaveFile> {
    let provenance = required_save_provenance(host, engine, mission_id, profiles)?;
    GameSaveFile::capture_with_game(
        engine,
        host,
        game,
        mission_id,
        game.mission_assets().map_err(anyhow::Error::msg)?.clone(),
        display_text,
        provenance,
    )
}
