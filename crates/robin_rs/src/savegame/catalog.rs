//! In-memory slot invariants. Filesystem recovery never mutates parallel maps:
//! metadata, lifecycle and generation are inserted/replaced/removed together.

use super::{SaveGame, SlotHandle, SlotName, SlotState, next_store_owner, validate_slot_names};
use anyhow::{Context, Result};

/// Runtime record, deliberately not deserializable. The persisted index stores
/// only validated metadata; generation and state are granted by this catalog.
#[derive(Debug)]
struct SlotEntry {
    name: SlotName,
    metadata: SaveGame,
    state: SlotState,
    generation: u64,
}

#[derive(Debug)]
pub(super) struct SlotCatalog {
    entries: Vec<SlotEntry>,
    owner_id: u64,
    next_generation: u64,
}

impl Default for SlotCatalog {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            owner_id: next_store_owner(),
            next_generation: 1,
        }
    }
}

impl SlotCatalog {
    pub(super) fn iter(&self) -> impl ExactSizeIterator<Item = &SaveGame> + DoubleEndedIterator {
        self.entries.iter().map(|entry| &entry.metadata)
    }
    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }
    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub(super) fn get(&self, index: usize) -> Option<&SaveGame> {
        self.entries.get(index).map(|entry| &entry.metadata)
    }
    pub(super) fn find(&self, name: &str) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| entry.name.as_str() == name)
    }

    fn entry(&self, index: usize) -> Result<&SlotEntry> {
        let entry = self
            .entries
            .get(index)
            .with_context(|| format!("missing save slot {index}"))?;
        anyhow::ensure!(
            entry.name.as_str() == entry.metadata.filename,
            "save metadata identity differs from its catalog entry"
        );
        Ok(entry)
    }

    pub(super) fn name(&self, index: usize) -> Result<SlotName> {
        Ok(self.entry(index)?.name.clone())
    }
    pub(super) fn handle(&self, index: usize) -> Result<SlotHandle> {
        let entry = self.entry(index)?;
        Ok(SlotHandle {
            name: entry.name.clone(),
            owner_id: self.owner_id,
            generation: entry.generation,
        })
    }
    pub(super) fn resolve(&self, handle: &SlotHandle) -> Result<usize> {
        let index = self
            .find(handle.name.as_str())
            .context("selected save slot no longer exists")?;
        let entry = self.entry(index)?;
        anyhow::ensure!(
            handle.owner_id == self.owner_id && handle.generation == entry.generation,
            "save selection is stale or belongs to another store"
        );
        Ok(index)
    }
    pub(super) fn state(&self, name: &SlotName) -> Result<SlotState> {
        let index = self
            .find(name.as_str())
            .with_context(|| format!("save slot {} no longer exists", name.as_str()))?;
        self.state_at(index)
    }

    pub(super) fn state_at(&self, index: usize) -> Result<SlotState> {
        Ok(self.entry(index)?.state)
    }

    pub(super) fn insert(&mut self, metadata: SaveGame, state: SlotState) -> Result<usize> {
        let name = SlotName::new(metadata.filename.clone()).map_err(anyhow::Error::msg)?;
        anyhow::ensure!(
            !self
                .entries
                .iter()
                .any(|entry| entry.name.as_str().eq_ignore_ascii_case(name.as_str())),
            "duplicate save slot name {:?}",
            name.as_str()
        );
        if state != SlotState::Draft {
            metadata.validate_published_metadata()?;
        }
        self.insert_validated(name, metadata, state)
    }

    fn insert_validated(
        &mut self,
        name: SlotName,
        metadata: SaveGame,
        state: SlotState,
    ) -> Result<usize> {
        let generation = self.next_generation;
        self.next_generation = generation
            .checked_add(1)
            .context("save generation space exhausted")?;
        self.entries.push(SlotEntry {
            name,
            metadata,
            state,
            generation,
        });
        Ok(self.entries.len() - 1)
    }

    pub(super) fn replace(
        &mut self,
        index: usize,
        metadata: SaveGame,
        state: SlotState,
    ) -> Result<()> {
        let entry = self.entry(index)?;
        anyhow::ensure!(
            entry.name.as_str() == metadata.filename,
            "cannot replace catalog slot identity"
        );
        if state != SlotState::Draft {
            metadata.validate_published_metadata()?;
        }
        let entry = &mut self.entries[index];
        entry.metadata = metadata;
        entry.state = state;
        Ok(())
    }

    pub(super) fn upsert(&mut self, metadata: SaveGame, state: SlotState) -> Result<usize> {
        if let Some(index) = self.find(&metadata.filename) {
            self.replace(index, metadata, state)?;
            Ok(index)
        } else {
            self.insert(metadata, state)
        }
    }

    pub(super) fn rename(&mut self, index: usize, text: String) -> Result<()> {
        self.entry(index)?;
        self.entries[index].metadata.text = text;
        Ok(())
    }

    pub(super) fn remove(&mut self, index: usize) -> Result<SaveGame> {
        self.entry(index)?;
        Ok(self.entries.remove(index).metadata)
    }

    pub(super) fn remove_named(&mut self, name: &str) -> Result<()> {
        if let Some(index) = self.find(name) {
            self.remove(index)?;
        }
        Ok(())
    }

    pub(super) fn sort_by_time(&mut self) {
        self.entries.sort_by_cached_key(|entry| {
            let timestamp = entry.metadata.timestamp.parse::<u64>().ok();
            // Parse once per row. Valid equal timestamps retain insertion
            // order; malformed legacy timestamps sort last, by slot name.
            (
                timestamp.is_none(),
                timestamp,
                timestamp.is_none().then(|| entry.name.as_str().to_owned()),
            )
        });
    }

    pub(super) fn published(&self) -> Result<Vec<&SaveGame>> {
        let mut names = std::collections::HashSet::new();
        let mut published = Vec::new();
        for (index, entry) in self.entries.iter().enumerate() {
            self.entry(index)?;
            anyhow::ensure!(
                names.insert(entry.name.as_str().to_ascii_lowercase()),
                "duplicate save slot name"
            );
            if entry.state == SlotState::Published {
                entry.metadata.validate_published_metadata()?;
                published.push(&entry.metadata);
            }
        }
        Ok(published)
    }

    pub(super) fn replace_autosaves(&mut self, metadata: Vec<SaveGame>) -> Result<()> {
        validate_slot_names(&metadata)?;
        for slot in &metadata {
            anyhow::ensure!(
                slot.is_autosave(),
                "autosave replacement received a manual slot"
            );
            slot.validate_published_metadata()?;
        }
        self.next_generation
            .checked_add(metadata.len() as u64)
            .context("save generation space exhausted")?;
        for slot in &metadata {
            anyhow::ensure!(
                !self
                    .entries
                    .iter()
                    .any(|entry| !entry.metadata.is_autosave()
                        && entry.name.as_str().eq_ignore_ascii_case(&slot.filename)),
                "autosave replacement collides with retained slot {}",
                slot.filename
            );
        }
        self.entries.retain(|entry| !entry.metadata.is_autosave());
        // All fallible checks precede mutation; generation capacity and names
        // were validated for the complete replacement, not one row at a time.
        for slot in metadata {
            let name = SlotName::new(slot.filename.clone()).expect("validated replacement name");
            self.insert_validated(name, slot, SlotState::Published)
                .expect("prevalidated generation capacity");
        }
        self.sort_by_time();
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn insert_fixture(&mut self, metadata: SaveGame, state: SlotState) {
        let name = SlotName::new(metadata.filename.clone()).expect("fixture basename");
        self.insert_validated(name, metadata, state)
            .expect("fixture generation");
    }
    #[cfg(test)]
    pub(super) fn metadata_mut(&mut self, index: usize) -> Option<&mut SaveGame> {
        self.entries.get_mut(index).map(|entry| &mut entry.metadata)
    }
}

impl std::ops::Index<usize> for SlotCatalog {
    type Output = SaveGame;
    fn index(&self, index: usize) -> &SaveGame {
        &self.entries[index].metadata
    }
}

#[cfg(test)]
impl std::ops::IndexMut<usize> for SlotCatalog {
    fn index_mut(&mut self, index: usize) -> &mut SaveGame {
        &mut self.entries[index].metadata
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_state_lookup_retains_identity_validation() {
        let mut catalog = SlotCatalog::default();
        for (name, state) in [
            ("Draft", SlotState::Draft),
            ("Manual", SlotState::Published),
            ("Restart", SlotState::Session),
        ] {
            let index = catalog.insert(published(name), state).unwrap();
            assert_eq!(catalog.state_at(index).unwrap(), state);
            assert_eq!(catalog.state(&catalog.name(index).unwrap()).unwrap(), state);
        }
        assert!(catalog.state_at(catalog.len()).is_err());
        let original_name = catalog.name(0).unwrap();
        catalog.metadata_mut(0).unwrap().filename = "Mismatched".into();
        assert!(catalog.state_at(0).is_err());
        assert!(catalog.state(&original_name).is_err());
    }

    fn published(name: &str) -> SaveGame {
        let mut slot = SaveGame::new(name.into(), name.into(), 1);
        slot.timestamp = "123".into();
        slot.mission_name = "Mission".into();
        slot.player_profile_id = Some(0);
        slot.player_name = "Player".into();
        slot.campaign_progress = Some(0);
        slot.missions_done = Some(0);
        slot.missions_total = Some(1);
        slot.gang_size = Some(1);
        slot.ransom = Some(0);
        slot.blazons = Some(0);
        slot.amulets = Some(0);
        slot
    }

    #[test]
    fn publication_borrows_metadata_and_preserves_owned_index_encoding() {
        let mut catalog = SlotCatalog::default();
        catalog
            .insert(published("Manual"), SlotState::Published)
            .unwrap();
        catalog
            .insert(published("Restart"), SlotState::Session)
            .unwrap();
        let slots = catalog.published().unwrap();
        assert_eq!(slots.len(), 1);
        assert!(std::ptr::eq(slots[0], catalog.get(0).unwrap()));
        let owned = super::super::SaveIndex {
            saves: slots.iter().map(|slot| (*slot).clone()).collect::<Vec<_>>(),
            next_id: 7,
            save_directory: "test-root".into(),
        };
        let borrowed = super::super::SaveIndex {
            saves: slots,
            next_id: owned.next_id,
            save_directory: owned.save_directory.clone(),
        };
        assert_eq!(
            serde_json::to_vec_pretty(&borrowed).unwrap(),
            serde_json::to_vec_pretty(&owned).unwrap()
        );
    }

    #[test]
    fn timestamp_sort_preserves_equal_time_order_and_handle_identity() {
        let mut catalog = SlotCatalog::default();
        let mut handles = Vec::new();
        for (name, timestamp) in [
            ("BadZ", "invalid"),
            ("Later", "10"),
            ("TieZ", "2"),
            ("BadA", "18446744073709551616"),
            ("TieA", "02"),
            ("First", "0"),
            ("Maximum", "18446744073709551615"),
        ] {
            let mut metadata = SaveGame::new(name.into(), name.into(), 1);
            metadata.timestamp = timestamp.into();
            catalog.insert_fixture(metadata, SlotState::Draft);
            handles.push(catalog.handle(catalog.len() - 1).unwrap());
        }
        for _ in 0..2 {
            catalog.sort_by_time();
            assert_eq!(
                catalog
                    .iter()
                    .map(|slot| slot.filename.as_str())
                    .collect::<Vec<_>>(),
                ["First", "TieZ", "TieA", "Later", "Maximum", "BadA", "BadZ"]
            );
            for handle in &handles {
                assert_eq!(
                    &catalog.handle(catalog.resolve(handle).unwrap()).unwrap(),
                    handle
                );
            }
        }
    }

    #[test]
    fn transitions_sort_and_refresh_preserve_only_surviving_identities() {
        let mut catalog = SlotCatalog::default();
        let index = catalog
            .insert(
                SaveGame::new("Manual".into(), "Draft".into(), 1),
                SlotState::Draft,
            )
            .unwrap();
        let manual = catalog.handle(index).unwrap();
        catalog
            .replace(index, published("Manual"), SlotState::Published)
            .unwrap();
        catalog
            .insert(published("Restart"), SlotState::Session)
            .unwrap();
        catalog
            .insert(
                SaveGame::new("Draft".into(), "Draft".into(), 1),
                SlotState::Draft,
            )
            .unwrap();
        catalog
            .replace_autosaves(vec![published("Autosave_1_0000")])
            .unwrap();
        let autosave = catalog
            .handle(catalog.find("Autosave_1_0000").unwrap())
            .unwrap();
        catalog.sort_by_time();
        let index = catalog.resolve(&manual).unwrap();
        catalog.rename(index, "Renamed".into()).unwrap();
        assert_eq!(catalog.handle(index).unwrap(), manual);
        assert_eq!(catalog.state(manual.name()).unwrap(), SlotState::Published);
        assert_eq!(catalog.published().unwrap().len(), 2);
        catalog
            .replace_autosaves(vec![published("Autosave_1_0000")])
            .unwrap();
        assert!(catalog.resolve(&autosave).is_err());
        let index = catalog.resolve(&manual).unwrap();
        catalog.remove(index).unwrap();
        catalog
            .insert(published("Manual"), SlotState::Published)
            .unwrap();
        assert!(catalog.resolve(&manual).is_err());
    }

    #[test]
    fn rejected_identity_and_refresh_leave_metadata_states_and_handles_unchanged() {
        let mut catalog = SlotCatalog::default();
        let manual_index = catalog
            .insert(published("autosave_2_0000"), SlotState::Published)
            .unwrap();
        catalog
            .replace_autosaves(vec![published("Autosave_1_0000")])
            .unwrap();
        let before: Vec<_> = catalog.iter().cloned().collect();
        let handles: Vec<_> = (0..catalog.len())
            .map(|i| catalog.handle(i).unwrap())
            .collect();
        let generation = catalog.next_generation;
        assert!(
            catalog
                .replace(manual_index, published("Different"), SlotState::Published)
                .is_err()
        );
        // Case-insensitive collision with a retained manual row, after one
        // otherwise valid candidate: nothing may be removed or inserted.
        assert!(
            catalog
                .replace_autosaves(vec![
                    published("Autosave_3_0000"),
                    published("Autosave_2_0000")
                ])
                .is_err()
        );
        assert!(catalog.iter().eq(before.iter()));
        assert_eq!(catalog.next_generation, generation);
        for handle in handles {
            let index = catalog.resolve(&handle).unwrap();
            assert_eq!(catalog.handle(index).unwrap(), handle);
            assert_eq!(catalog.state(handle.name()).unwrap(), SlotState::Published);
        }
        catalog.next_generation = u64::MAX;
        assert!(
            catalog
                .replace_autosaves(vec![published("Autosave_4_0000")])
                .is_err()
        );
        assert!(catalog.iter().eq(before.iter()));
    }
}
