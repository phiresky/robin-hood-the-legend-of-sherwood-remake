//! Slot catalogue operations: allocation, lookup, deletion intents, and
//! per-slot file paths.
use super::*;

impl SaveGameManager {
    #[cfg(test)]
    pub(crate) fn insert_test_slot(&mut self, slot: SaveGame, state: SlotState) {
        self.catalog.insert_fixture(slot, state);
    }

    pub fn slot_name(&self, index: usize) -> Result<SlotName, String> {
        self.catalog
            .name(index)
            .cloned()
            .map_err(|error| error.to_string())
    }

    /// Find the slot for one of the well-known special filenames, or
    /// create a new slot if none exists yet.  Used to manage the
    /// Continue / Restart / Sherwood / QuickSave auto-slots.
    pub(super) fn ensure_special_slot(
        &mut self,
        filename: &str,
        display_text: &str,
    ) -> Result<usize> {
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
    pub(super) fn allocate_named_draft(
        &mut self,
        filename: String,
        text: String,
        mission_id: u32,
    ) -> Result<usize> {
        self.finish_background()?;
        self.ensure_no_pending_delete()?;
        SlotName::validate(&filename).map_err(anyhow::Error::msg)?;
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
    pub(super) fn find_or_create_by_filename(&mut self, filename: &str, text: &str) -> usize {
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
        if self.catalog.state_at(index)? == SlotState::Draft {
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

    pub(super) fn delete_recovery_path(&self) -> PathBuf {
        Path::new(&self.save_directory).join("save-delete-recovery.json")
    }

    pub(super) fn ensure_no_pending_delete(&self) -> Result<()> {
        self.require_storage()?;
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

    pub(super) fn reconcile_delete(&mut self) -> Result<()> {
        let bytes = match std::fs::read(self.delete_recovery_path()) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error).context("read deletion recovery intent"),
        };
        self.finish_delete(
            serde_json::from_slice(&bytes).context("decode deletion recovery intent")?,
        )
    }

    pub(super) fn finish_delete(&mut self, receipt: DeleteRecovery) -> Result<()> {
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
        let filename = self
            .catalog
            .name(index)
            .expect("invalid save slot identity");
        Path::new(&self.save_directory).join(format!("{}_thumb.png", filename.as_str()))
    }

    /// Full path to a save file on disk (JSON format, with `.json` extension).
    pub fn save_path(&self, index: usize) -> PathBuf {
        let filename = self
            .catalog
            .name(index)
            .expect("invalid save slot identity");
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
        save_file::atomic_copy(&src_json, &dst_json)
            .map_err(|e| format!("copy save json: {e:#}"))?;
        // Thumbnail (used by both formats)
        let src_thumb = self.thumb_path(src);
        let dst_thumb = self.thumb_path(dst);
        if let Err(error) = save_file::atomic_copy_if_exists(&src_thumb, &dst_thumb) {
            tracing::warn!("Could not rotate save thumbnail: {error:#}");
        }

        Ok(())
    }

    pub(super) fn copy_display_metadata(&mut self, src: usize, dst: usize) -> Result<()> {
        let state = self.catalog.state_at(src)?;
        let src = self
            .catalog
            .get(src)
            .with_context(|| format!("cannot copy metadata from missing save slot {src}"))?;
        let destination = dst;
        let dst = self
            .catalog
            .get(dst)
            .with_context(|| format!("cannot copy metadata to missing save slot {dst}"))?;
        let metadata = src.cloned_for_slot(dst);
        self.catalog.replace(destination, metadata, state)
    }

    /// Replace only auto-managed slots, preserving manual and Original
    /// special slots that may have changed while the writer was active.
    pub(crate) fn replace_autosaves(&mut self, autosaves: Vec<SaveGame>) -> Result<()> {
        self.require_storage()?;
        self.finish_background()?;
        self.catalog.replace_autosaves(autosaves)
    }
}
