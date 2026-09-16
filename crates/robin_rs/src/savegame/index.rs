//! `saves.json` index publication/loading, quick-save recovery, filename
//! allocation, and the autosave manifest merge.
use super::*;

impl SaveGameManager {
    /// Persist the save manager index itself (the list of saves).
    pub fn save_index(&self) -> Result<(), String> {
        self.require_storage().map_err(|error| error.to_string())?;
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

    pub(super) fn publish_index(&self) -> Result<(), String> {
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

    pub(super) fn quick_recovery_path(&self) -> PathBuf {
        Path::new(&self.save_directory).join("quick-save-recovery.json")
    }

    /// Publish only receipt entries whose payload actually reached disk.
    /// A crash between rotation and new-save publication keeps the old quick
    /// entry and recovers the previous slot independently.
    pub(super) fn reconcile_quick_slots(&mut self) -> Result<()> {
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

    pub(super) fn next_filename(&mut self) -> Result<String> {
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

    /// Merge the independently committed autosave manifest into this manager.
    pub(crate) fn load_autosaves(&mut self) -> Result<()> {
        self.require_storage()?;
        use autosave_store::*;
        // The manifest owns menu metadata. Decode and validate only the selected
        // payload on load, so opening the menu never reads every saved simulation.
        let legacy_seed = AutosaveManifest {
            version: AUTOSAVE_MANIFEST_VERSION,
            saves: self
                .saves()
                .filter(|save| save.is_autosave())
                .cloned()
                .collect(),
        };
        self.replace_autosaves(Vec::new())?;
        let manifest = load_manifest(self.save_directory())?.unwrap_or(legacy_seed);
        manifest.validate()?;
        garbage_collect_orphans(self.save_directory(), &manifest)?;
        self.replace_autosaves(manifest.saves)?;
        Ok(())
    }
}
