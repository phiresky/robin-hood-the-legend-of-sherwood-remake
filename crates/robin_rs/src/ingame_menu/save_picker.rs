//! Scheduling-independent save picker state. GPU resources and text input stay
//! in the adapters; persisted slot names, never list offsets, identify actions.

use serde::{Deserialize, Serialize};

use super::save_load::SaveLoadMode;
use crate::savegame::SlotName;

mod controller;
pub(crate) use controller::{
    ID_CANCEL, ID_DELETE, ID_LOAD_SAVE, PickerAction, PickerController, PickerTarget,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ListRow {
    New,
    Existing(usize),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PickerSlot {
    pub name: SlotName,
    pub manager_index: usize,
    pub special: bool,
    pub hidden_from_load: bool,
    pub autosave: bool,
    pub multiplayer_diagnostic: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Selection {
    New,
    Existing(SlotName),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PickerModel {
    mode: SaveLoadMode,
    multiplayer_connected: bool,
    slots: Vec<PickerSlot>,
    selection: Option<Selection>,
    viewport_rows: usize,
    scroll_offset: usize,
    delete_confirmation: Option<SlotName>,
    operation_error: Option<String>,
}

impl PickerModel {
    pub fn new(
        mode: SaveLoadMode,
        multiplayer_connected: bool,
        viewport_rows: usize,
        slots: Vec<PickerSlot>,
    ) -> Self {
        assert!(viewport_rows > 0, "picker viewport must contain a row");
        let mut model = Self {
            mode,
            multiplayer_connected,
            slots: Vec::new(),
            selection: (mode == SaveLoadMode::Save).then_some(Selection::New),
            viewport_rows,
            scroll_offset: 0,
            delete_confirmation: None,
            operation_error: None,
        };
        model.refresh(slots);
        model
    }

    /// Refresh after *every* storage outcome, including a published deletion
    /// whose subsequent cleanup failed. Retain identities across reordering.
    pub fn refresh(&mut self, slots: Vec<PickerSlot>) {
        let mut names = std::collections::HashSet::new();
        assert!(
            slots.iter().all(|slot| names.insert(slot.name.clone())),
            "picker requires unique slot identities"
        );
        self.slots = slots
            .into_iter()
            .filter(|slot| match self.mode {
                SaveLoadMode::Save => !slot.special,
                SaveLoadMode::Load => {
                    !slot.hidden_from_load
                        && (!self.multiplayer_connected || !slot.multiplayer_diagnostic)
                }
            })
            .collect();
        if let Some(Selection::Existing(name)) = &self.selection
            && !self.slots.iter().any(|slot| &slot.name == name)
        {
            self.selection = None;
        }
        self.scroll_offset = self
            .scroll_offset
            .min(self.total_rows().saturating_sub(self.viewport_rows));
    }

    pub fn visible(&self) -> Vec<usize> {
        self.slots.iter().map(|slot| slot.manager_index).collect()
    }

    pub fn selected_row(&self) -> Option<ListRow> {
        match &self.selection {
            None => None,
            Some(Selection::New) => Some(ListRow::New),
            Some(Selection::Existing(name)) => Some(ListRow::Existing(
                self.slots
                    .iter()
                    .position(|slot| &slot.name == name)
                    .expect("selected identity must be visible"),
            )),
        }
    }

    pub fn selected_slot(&self) -> Option<&SlotName> {
        match &self.selection {
            Some(Selection::Existing(name)) => Some(name),
            _ => None,
        }
    }

    pub fn select(&mut self, row: Option<ListRow>) {
        self.selection = row.map(|row| match row {
            ListRow::New => {
                assert_eq!(
                    self.mode,
                    SaveLoadMode::Save,
                    "load picker has no new-save row"
                );
                Selection::New
            }
            ListRow::Existing(index) => Selection::Existing(
                self.slots
                    .get(index)
                    .expect("selected presentation row must exist")
                    .name
                    .clone(),
            ),
        });
    }

    pub fn navigate(&mut self, forward: bool) -> Option<ListRow> {
        let row = match self.selected_row() {
            None => 0,
            Some(ListRow::New) => usize::from(forward),
            Some(ListRow::Existing(index)) => {
                let index = index + usize::from(self.mode == SaveLoadMode::Save);
                if forward {
                    index.saturating_add(1)
                } else {
                    index.saturating_sub(1)
                }
            }
        }
        .min(self.total_rows().saturating_sub(1));
        self.select(self.row_at(row));
        if self.selection.is_some() {
            if row < self.scroll_offset {
                self.scroll_offset = row;
            }
            if row >= self.scroll_offset + self.viewport_rows {
                self.scroll_offset = row + 1 - self.viewport_rows;
            }
        }
        self.selected_row()
    }

    pub fn row_at(&self, index: usize) -> Option<ListRow> {
        if index >= self.total_rows() {
            return None;
        }
        Some(if self.mode == SaveLoadMode::Save {
            if index == 0 {
                ListRow::New
            } else {
                ListRow::Existing(index - 1)
            }
        } else {
            ListRow::Existing(index)
        })
    }

    pub fn total_rows(&self) -> usize {
        self.slots.len() + usize::from(self.mode == SaveLoadMode::Save)
    }
    /// Synchronize the shared view while retaining geometry-free selection policy.
    pub fn set_viewport(&mut self, rows: usize, offset: usize) {
        assert!(rows > 0);
        self.viewport_rows = rows;
        self.scroll_offset = offset.min(self.total_rows().saturating_sub(rows));
    }

    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }
    pub fn can_delete(&self) -> bool {
        let Some(name) = self.selected_slot() else {
            return false;
        };
        !self
            .slots
            .iter()
            .find(|slot| &slot.name == name)
            .expect("selected identity must be visible")
            .autosave
    }

    #[cfg(test)]
    pub fn request_delete(&mut self) -> Option<SlotName> {
        if !self.can_delete() {
            return None;
        }
        let name = self
            .selected_slot()
            .expect("deletable selection must identify a slot")
            .clone();
        self.request_delete_named(name.clone())
            .expect("deletable selected identity is valid");
        Some(name)
    }

    pub fn request_delete_named(&mut self, name: SlotName) -> Result<(), String> {
        let slot = self
            .slots
            .iter()
            .find(|slot| slot.name == name)
            .ok_or_else(|| "the selected save is no longer available".to_string())?;
        if slot.autosave {
            return Err("autosaves cannot be manually deleted".into());
        }
        self.delete_confirmation = Some(name);
        self.operation_error = None;
        Ok(())
    }

    /// Resolve the exact confirmed identity, even if the backing list moved.
    pub fn confirm_delete(&mut self, confirmed: bool) -> Result<Option<SlotName>, String> {
        let name = self
            .delete_confirmation
            .take()
            .ok_or_else(|| "no delete confirmation is pending".to_string())?;
        if !confirmed {
            return Ok(None);
        }
        let slot = self
            .slots
            .iter()
            .find(|slot| slot.name == name)
            .ok_or_else(|| "the selected save is no longer available".to_string())?;
        if slot.autosave {
            return Err("autosaves cannot be manually deleted".into());
        }
        Ok(Some(name))
    }

    pub fn finish_delete(&mut self, slots: Vec<PickerSlot>, error: Option<String>) {
        self.refresh(slots);
        self.operation_error = error;
    }

    pub fn operation_error(&self) -> Option<&str> {
        self.operation_error.as_deref()
    }

    /// Includes failures while editing a save, before payload publication.
    pub fn report_error(&mut self, error: String) {
        self.operation_error = Some(error);
    }

    pub fn dismiss_error(&mut self) {
        self.operation_error = None;
    }
}

/// A thumbnail is valid for an identity, not whichever row now has its index.
pub(crate) fn retire_thumbnail(cached: Option<&SlotName>, selected: Option<&SlotName>) -> bool {
    cached.is_some() && cached != selected
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(name: &str, index: usize) -> PickerSlot {
        PickerSlot {
            name: SlotName::new(name).unwrap(),
            manager_index: index,
            special: false,
            hidden_from_load: false,
            autosave: false,
            multiplayer_diagnostic: false,
        }
    }
    fn model() -> PickerModel {
        PickerModel::new(
            SaveLoadMode::Load,
            false,
            2,
            vec![
                slot("Savegame_000", 0),
                slot("Savegame_001", 1),
                slot("Savegame_002", 2),
            ],
        )
    }

    #[test]
    fn selection_and_confirmation_survive_reordering_but_not_disappearance() {
        let mut m = model();
        m.select(Some(ListRow::Existing(1)));
        let name = m.request_delete().unwrap();
        m.refresh(vec![slot("Savegame_001", 0), slot("Savegame_000", 1)]);
        assert_eq!(m.selected_row(), Some(ListRow::Existing(0)));
        assert_eq!(m.confirm_delete(true), Ok(Some(name)));
        m.request_delete();
        m.refresh(vec![slot("Savegame_000", 0)]);
        assert_eq!(m.selected_row(), None);
        assert!(m.confirm_delete(true).is_err());
    }

    #[test]
    fn deletion_cancel_failure_and_partial_success_preserve_truth() {
        let mut m = model();
        m.navigate(true);
        m.request_delete();
        assert_eq!(m.confirm_delete(false), Ok(None));
        assert_eq!(m.selected_row(), Some(ListRow::Existing(0)));
        m.request_delete();
        m.confirm_delete(true).unwrap();
        let rows = m.slots.clone();
        m.finish_delete(rows, Some("index publication failed".into()));
        assert!(m.selected_slot().is_some());
        assert_eq!(m.operation_error(), Some("index publication failed"));
        m.finish_delete(
            vec![slot("Savegame_001", 0)],
            Some("cleanup pending".into()),
        );
        assert_eq!(m.selected_slot(), None);
        assert_eq!(m.operation_error(), Some("cleanup pending"));
    }

    #[test]
    fn navigation_and_delete_clamp_the_viewport() {
        let mut m = model();
        for _ in 0..10 {
            m.navigate(true);
        }
        assert_eq!(m.scroll_offset(), 1);
        m.finish_delete(vec![slot("Savegame_000", 0)], None);
        assert_eq!(m.scroll_offset(), 0);
        m.set_viewport(2, usize::MAX);
        assert_eq!(m.scroll_offset(), 0);
        m.navigate(false);
        assert_eq!(m.selected_row(), Some(ListRow::Existing(0)));
    }

    #[test]
    fn autosave_and_multiplayer_filter_policy_is_shared() {
        let mut auto = slot("Autosave_100_0000", 0);
        auto.autosave = true;
        auto.special = true;
        let mut diagnostic = slot("Savegame_001", 1);
        diagnostic.multiplayer_diagnostic = true;
        let mut hidden = slot("Savegame_002", 2);
        hidden.hidden_from_load = true;
        hidden.special = true;
        let rows = vec![auto, diagnostic, hidden];
        let mut load = PickerModel::new(SaveLoadMode::Load, true, 3, rows.clone());
        assert_eq!(load.visible(), vec![0]);
        load.navigate(true);
        assert!(!load.can_delete());
        assert_eq!(load.request_delete(), None);
        let save = PickerModel::new(SaveLoadMode::Save, false, 3, rows);
        assert_eq!(save.visible(), vec![1]);
        assert_eq!(save.selected_row(), Some(ListRow::New));
    }

    #[test]
    fn thumbnails_retire_on_identity_change_or_clear_not_reorder() {
        let a = SlotName::new("Savegame_000").unwrap();
        let b = SlotName::new("Savegame_001").unwrap();
        assert!(!retire_thumbnail(Some(&a), Some(&a)));
        assert!(retire_thumbnail(Some(&a), Some(&b)));
        assert!(retire_thumbnail(Some(&a), None));
        assert!(!retire_thumbnail(None, Some(&a)));
    }
}
