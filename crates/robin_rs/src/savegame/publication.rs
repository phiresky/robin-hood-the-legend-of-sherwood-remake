//! Synchronous payload publication and slot metadata synchronization.
use super::*;

impl SaveGameManager {
    pub(super) fn commit_synchronous(
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
            slot: metadata,
            digest: Sha256::digest(bytes).into(),
        };
        let overwrite =
            self.catalog[index].is_special() || self.catalog.state_at(index)? != SlotState::Draft;
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
        let result = self.finish_publication(index, receipt.slot);
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

    pub(super) fn require_synchronous_storage() -> Result<()> {
        anyhow::ensure!(
            !cfg!(target_arch = "wasm32"),
            "browser manual-save persistence is unavailable; use durable autosaves"
        );
        Ok(())
    }

    pub(super) fn finish_publication(&mut self, index: usize, metadata: SaveGame) -> Result<()> {
        self.catalog
            .replace(index, metadata, SlotState::Published)?;
        self.publish_index().map_err(anyhow::Error::msg)?;
        self.retire_owned_receipt()
    }

    pub(super) fn publish_thumbnail(&self, index: usize, thumbnail: Option<&Thumbnail>) {
        // A preview is not part of the authoritative payload transaction.
        if let Some(thumb) = thumbnail {
            let thumb_path = self.thumb_path(index);
            if let Err(err) = thumb.write_to(&thumb_path) {
                // Non-fatal — the save payload is already on disk.
                tracing::warn!("Failed to write thumbnail for slot {index}: {err:#}");
            }
        }
    }

    pub(super) fn sync_slot_metadata_from_save(
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
        let profiles = profiles.context("save metadata requires mission profiles")?;
        slot.update_snapshot_metadata(&save.header, save.engine.campaign(), profiles);
        self.catalog.replace(index, slot, SlotState::Published)
    }
}
