//! Portrait bar — manages the row of character portraits in the game UI.
//!
//! The portrait bar holds references to portrait widgets (identified by
//! [`PortraitId`]) and handles deferred addition/removal, priority-based
//! sorting, placement, and scrolling when there are more portraits than
//! [`MAX_VISIBLE_PORTRAITS`].
//!
//! **Status in the Rust port**: the module is preserved as a parity-ready
//! retained model — all the add/remove/reset queue semantics are
//! faithfully reproduced and covered by unit tests — but it is **not
//! wired into the live HUD**.  The production renderer (`ui_panel.rs`)
//! iterates `engine.pc_ids()` directly each frame, and `pc_ids` is
//! priority-sorted at level load (see
//! `EngineInner::sort_pc_ids_by_priority` in
//! `crates/robin_engine/src/engine/selection.rs`) so it already matches
//! the ordering this module would produce.  Mission transitions rebuild
//! engine state from scratch, `HideInterface` flags are read live from
//! `PcData::interface_hidden`, and the Sherwood widget gating that used
//! to live inside the delayed-portrait update is ported into
//! [`crate::sherwood_hud::SherwoodButtonEnable::apply_update_portraits_delayed`]
//! driven from `handle_sherwood_hud_buttons` in `game_session.rs`.
//!
//! If the HUD later grows a proper retained portrait bar (for scroll
//! support, dynamic hide/show during a mission, etc.), this module is
//! the intended host-side container — `add_portrait` / `remove_portrait`
//! / `reset` are ready to accept callers.  Until then, everything outside
//! of tests stays dead code and [`PortraitBar::new`] is not instantiated
//! in production.

use serde::{Deserialize, Serialize};

use crate::profiles::MAX_NUMBER_OF_PC;

/// Maximum number of portraits visible at once.
pub const MAX_VISIBLE_PORTRAITS: usize = MAX_NUMBER_OF_PC;

// ─── ID types ────────────────────────────────────────────────────────

/// Opaque identifier for a portrait widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PortraitId(pub u32);

/// Opaque identifier for a playable character (PC) actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PcId(pub u32);

/// Opaque identifier for a PC description.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PcDescriptionId(pub u32);

// ─── Portrait info ───────────────────────────────────────────────────

/// Snapshot of the data the portrait bar needs from each portrait widget.
///
/// State is stored explicitly so the bar logic is self-contained and
/// testable without a full UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortraitInfo {
    pub id: PortraitId,
    pub pc_id: PcId,
    pub pc_description_id: PcDescriptionId,
    /// Priority from `CharacterProfile::priority` — higher means further left.
    pub priority: u16,
    pub enabled: bool,
    pub displayed: bool,
    pub is_open: bool,
    pub is_burned: bool,
    pub is_trumpet_enabled: bool,
    // Checked on the attached PC when adding:
    pub pc_is_playable: bool,
    pub pc_is_in_coma: bool,
    pub pc_is_guarded: bool,
    pub pc_is_waiting_for_reinforcement: bool,
    pub pc_is_selectable: bool,
}

// ─── Callbacks ───────────────────────────────────────────────────────

/// Trait for the UI layer to receive placement commands from the portrait bar.
pub trait PortraitBarCallbacks {
    /// Called when a portrait should be added to the UI window.
    fn add_window(&mut self, portrait_id: PortraitId);
    /// Called when a portrait should be removed from the UI window.
    fn remove_window(&mut self, portrait_id: PortraitId);
    /// Called to move the minimap to the front (after adding portraits).
    fn move_minimap_to_front(&mut self);
    /// Called to place a portrait at a given display slot index.
    fn displace_portrait(&mut self, portrait_id: PortraitId, slot_index: usize);
    /// Called to enable/disable a portrait widget.
    fn set_portrait_enabled(&mut self, portrait_id: PortraitId, enabled: bool);
}

// ─── Portrait bar ────────────────────────────────────────────────────

/// The portrait bar: manages a sorted list of character portraits with
/// deferred add/remove, scrolling, and placement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortraitBar {
    portraits: Vec<PortraitInfo>,
    portraits_to_add: Vec<PortraitInfo>,
    portraits_to_remove: Vec<PortraitId>,
    first_visible_portrait: usize,
    make_placement: bool,
}

impl PortraitBar {
    pub fn new() -> Self {
        Self {
            portraits: Vec::new(),
            portraits_to_add: Vec::new(),
            portraits_to_remove: Vec::new(),
            first_visible_portrait: 0,
            make_placement: false,
        }
    }

    /// Queue a portrait for addition.
    ///
    /// If it's already queued for removal, cancel that removal.
    pub fn add_portrait(&mut self, mut info: PortraitInfo) {
        if self.portraits_to_add.iter().any(|p| p.id == info.id) {
            debug_assert!(info.displayed);
            return;
        }

        info.displayed = true;
        let id = info.id;
        self.portraits_to_add.push(info);

        // Cancel any pending removal for this portrait.
        self.portraits_to_remove.retain(|&rid| rid != id);
    }

    /// Queue a portrait for removal.
    ///
    /// If it's already queued for addition, cancel that addition.
    pub fn remove_portrait(&mut self, portrait_id: PortraitId) {
        if self.portraits_to_remove.contains(&portrait_id) {
            return;
        }

        self.portraits_to_remove.push(portrait_id);

        // Mark as not displayed in the pending-add list (or main list).
        if let Some(p) = self
            .portraits_to_add
            .iter_mut()
            .find(|p| p.id == portrait_id)
        {
            p.displayed = false;
        }
        if let Some(p) = self.portraits.iter_mut().find(|p| p.id == portrait_id) {
            p.displayed = false;
        }

        // Cancel any pending addition.
        self.portraits_to_add.retain(|p| p.id != portrait_id);
    }

    /// Queue all current portraits for removal.
    pub fn remove_all_portraits(&mut self) {
        let ids: Vec<PortraitId> = self.portraits.iter().map(|p| p.id).collect();
        for id in ids {
            self.remove_portrait(id);
        }
    }

    /// Number of portraits currently in the bar (not counting pending adds/removes).
    pub fn portrait_count(&self) -> usize {
        self.portraits.len()
    }

    /// Get a reference to the portrait list.
    pub fn portraits(&self) -> &[PortraitInfo] {
        &self.portraits
    }

    /// Find a portrait by its attached PC id.
    pub fn find_portrait_by_pc(&self, pc_id: PcId) -> Option<&PortraitInfo> {
        self.portraits
            .iter()
            .find(|p| p.pc_id == pc_id)
            .or_else(|| self.portraits_to_add.iter().find(|p| p.pc_id == pc_id))
    }

    /// Find a portrait by its attached PC description id.
    pub fn find_portrait_by_description(&self, desc_id: PcDescriptionId) -> Option<&PortraitInfo> {
        self.portraits
            .iter()
            .find(|p| p.pc_description_id == desc_id)
    }

    /// Get the index of a portrait in the bar, or `None` if not present.
    pub fn portrait_index(&self, portrait_id: PortraitId) -> Option<usize> {
        self.portraits.iter().position(|p| p.id == portrait_id)
    }

    /// Open or close all portraits.
    pub fn set_all_portraits_opened(&mut self, opened: bool) {
        for p in &mut self.portraits {
            if opened {
                if !p.is_burned && p.pc_is_selectable {
                    p.is_open = true;
                }
            } else {
                p.is_open = false;
            }
        }
    }

    /// Open or close the portrait for a specific PC.
    ///
    /// Searches both the active portraits and the deferred
    /// `portraits_to_add` queue, so a call made after `add_portrait` but
    /// before the next `update()` takes effect on the queued portrait
    /// too.
    pub fn set_portrait_opened(&mut self, pc_id: PcId, opened: bool) {
        if let Some(p) = self
            .portraits
            .iter_mut()
            .chain(self.portraits_to_add.iter_mut())
            .find(|p| p.pc_id == pc_id)
        {
            if opened {
                assert!(!p.is_burned, "Cannot open a burned portrait");
                p.is_open = true;
            } else {
                p.is_open = false;
            }
        }
    }

    /// Shift the visible window one position to the left (wrapping).
    pub fn shift_portraits_to_left(&mut self) {
        if self.portraits.len() > MAX_VISIBLE_PORTRAITS {
            let n = self.portraits.len();
            self.first_visible_portrait = (self.first_visible_portrait + n - 1) % n;
            self.make_placement = true;
        }
    }

    /// Shift the visible window one position to the right (wrapping).
    pub fn shift_portraits_to_right(&mut self) {
        if self.portraits.len() > MAX_VISIBLE_PORTRAITS {
            self.first_visible_portrait = (self.first_visible_portrait + 1) % self.portraits.len();
            self.make_placement = true;
        }
    }

    /// Reset the portrait bar to its initial empty state.
    pub fn reset(&mut self, callbacks: &mut dyn PortraitBarCallbacks) {
        self.portraits_to_add.clear();
        self.remove_all_portraits();
        self.update(callbacks);
        self.first_visible_portrait = 0;
        self.make_placement = false;
    }

    /// Recalculate positions of visible portraits (e.g. after a window resize).
    pub fn resize(&self, callbacks: &mut dyn PortraitBarCallbacks) {
        let max_count = self.portraits.len().min(MAX_VISIBLE_PORTRAITS);
        let n = self.portraits.len();
        if n == 0 {
            return;
        }
        for slot in 0..max_count {
            let idx = (self.first_visible_portrait + slot) % n;
            callbacks.displace_portrait(self.portraits[idx].id, slot);
        }
    }

    /// Run per-frame update: process pending additions/removals and placement.
    pub fn update(&mut self, callbacks: &mut dyn PortraitBarCallbacks) {
        let needs_update =
            !self.portraits_to_add.is_empty() || !self.portraits_to_remove.is_empty();

        if needs_update {
            self.make_all_removals(callbacks);
            self.make_all_additions(callbacks);
        }

        if needs_update || self.make_placement {
            self.make_placement(callbacks);
        }
    }

    // ─── Private helpers ─────────────────────────────────────────────

    /// Process all pending removals.
    fn make_all_removals(&mut self, callbacks: &mut dyn PortraitBarCallbacks) {
        let to_remove = std::mem::take(&mut self.portraits_to_remove);
        for id in &to_remove {
            if let Some(pos) = self.portraits.iter().position(|p| p.id == *id) {
                self.portraits.remove(pos);
                callbacks.remove_window(*id);
            }
        }
    }

    /// Process all pending additions, then sort by priority (descending).
    fn make_all_additions(&mut self, callbacks: &mut dyn PortraitBarCallbacks) {
        let to_add = std::mem::take(&mut self.portraits_to_add);
        for info in to_add {
            // Validate: must be playable, trumpet-enabled, in coma, guarded,
            // or waiting for reinforcement.
            assert!(
                info.pc_is_playable
                    || info.is_trumpet_enabled
                    || info.pc_is_in_coma
                    || info.pc_is_guarded
                    || info.pc_is_waiting_for_reinforcement,
                "Portrait {:?} added but PC is in invalid state",
                info.id
            );

            if self.portrait_index(info.id).is_none() {
                let id = info.id;
                self.portraits.push(info);
                callbacks.add_window(id);
            }

            callbacks.move_minimap_to_front();
        }

        // Stable sort by priority descending.
        self.portraits
            .sort_by_key(|p| std::cmp::Reverse(p.priority));
    }

    /// Assign display slots to all portraits.
    fn make_placement(&mut self, callbacks: &mut dyn PortraitBarCallbacks) {
        let n = self.portraits.len();

        if n <= MAX_VISIBLE_PORTRAITS {
            // All portraits fit — place them sequentially.
            for (slot, portrait) in self.portraits.iter().enumerate() {
                callbacks.displace_portrait(portrait.id, slot);
                callbacks.set_portrait_enabled(portrait.id, true);
            }
            self.first_visible_portrait = 0;
        } else {
            // More portraits than slots — use scrolling window.
            self.first_visible_portrait %= n;

            for (i, portrait) in self.portraits.iter().enumerate() {
                let display_pos = (i + n - self.first_visible_portrait) % n;

                if display_pos < MAX_VISIBLE_PORTRAITS {
                    callbacks.displace_portrait(portrait.id, display_pos);
                    callbacks.set_portrait_enabled(portrait.id, true);
                } else {
                    callbacks.set_portrait_enabled(portrait.id, false);
                }
            }
        }

        self.make_placement = false;
    }
}

impl Default for PortraitBar {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Test callback recorder — captures all calls for assertions.
    #[derive(Debug, Default)]
    struct MockCallbacks {
        added: Vec<PortraitId>,
        removed: Vec<PortraitId>,
        minimap_fronted: usize,
        placements: Vec<(PortraitId, usize)>,
        enabled: Vec<(PortraitId, bool)>,
    }

    impl MockCallbacks {
        fn clear(&mut self) {
            self.added.clear();
            self.removed.clear();
            self.minimap_fronted = 0;
            self.placements.clear();
            self.enabled.clear();
        }
    }

    impl PortraitBarCallbacks for MockCallbacks {
        fn add_window(&mut self, id: PortraitId) {
            self.added.push(id);
        }
        fn remove_window(&mut self, id: PortraitId) {
            self.removed.push(id);
        }
        fn move_minimap_to_front(&mut self) {
            self.minimap_fronted += 1;
        }
        fn displace_portrait(&mut self, id: PortraitId, slot: usize) {
            self.placements.push((id, slot));
        }
        fn set_portrait_enabled(&mut self, id: PortraitId, enabled: bool) {
            self.enabled.push((id, enabled));
        }
    }

    fn make_portrait(id: u32, priority: u16) -> PortraitInfo {
        PortraitInfo {
            id: PortraitId(id),
            pc_id: PcId(id),
            pc_description_id: PcDescriptionId(id),
            priority,
            enabled: true,
            displayed: false,
            is_open: false,
            is_burned: false,
            is_trumpet_enabled: false,
            pc_is_playable: true,
            pc_is_in_coma: false,
            pc_is_guarded: false,
            pc_is_waiting_for_reinforcement: false,
            pc_is_selectable: true,
        }
    }

    #[test]
    fn add_and_count() {
        let mut bar = PortraitBar::new();
        let mut cb = MockCallbacks::default();

        bar.add_portrait(make_portrait(1, 10));
        bar.add_portrait(make_portrait(2, 5));
        bar.update(&mut cb);

        assert_eq!(bar.portrait_count(), 2);
        assert_eq!(cb.added.len(), 2);
    }

    #[test]
    fn sorted_by_priority_descending() {
        let mut bar = PortraitBar::new();
        let mut cb = MockCallbacks::default();

        bar.add_portrait(make_portrait(1, 3));
        bar.add_portrait(make_portrait(2, 10));
        bar.add_portrait(make_portrait(3, 7));
        bar.update(&mut cb);

        let priorities: Vec<u16> = bar.portraits().iter().map(|p| p.priority).collect();
        assert_eq!(priorities, vec![10, 7, 3]);
    }

    #[test]
    fn remove_portrait() {
        let mut bar = PortraitBar::new();
        let mut cb = MockCallbacks::default();

        bar.add_portrait(make_portrait(1, 10));
        bar.add_portrait(make_portrait(2, 5));
        bar.update(&mut cb);
        assert_eq!(bar.portrait_count(), 2);

        cb.clear();
        bar.remove_portrait(PortraitId(1));
        bar.update(&mut cb);

        assert_eq!(bar.portrait_count(), 1);
        assert_eq!(cb.removed, vec![PortraitId(1)]);
        assert_eq!(bar.portraits()[0].id, PortraitId(2));
    }

    #[test]
    fn remove_all_portraits() {
        let mut bar = PortraitBar::new();
        let mut cb = MockCallbacks::default();

        bar.add_portrait(make_portrait(1, 10));
        bar.add_portrait(make_portrait(2, 5));
        bar.add_portrait(make_portrait(3, 7));
        bar.update(&mut cb);

        cb.clear();
        bar.remove_all_portraits();
        bar.update(&mut cb);

        assert_eq!(bar.portrait_count(), 0);
        assert_eq!(cb.removed.len(), 3);
    }

    #[test]
    fn find_portrait_by_pc() {
        let mut bar = PortraitBar::new();
        let mut cb = MockCallbacks::default();

        bar.add_portrait(make_portrait(1, 10));
        bar.add_portrait(make_portrait(2, 5));
        bar.update(&mut cb);

        assert!(bar.find_portrait_by_pc(PcId(1)).is_some());
        assert!(bar.find_portrait_by_pc(PcId(99)).is_none());
    }

    #[test]
    fn find_portrait_by_description() {
        let mut bar = PortraitBar::new();
        let mut cb = MockCallbacks::default();

        bar.add_portrait(make_portrait(1, 10));
        bar.update(&mut cb);

        assert!(
            bar.find_portrait_by_description(PcDescriptionId(1))
                .is_some()
        );
        assert!(
            bar.find_portrait_by_description(PcDescriptionId(99))
                .is_none()
        );
    }

    #[test]
    fn portrait_index() {
        let mut bar = PortraitBar::new();
        let mut cb = MockCallbacks::default();

        bar.add_portrait(make_portrait(1, 10));
        bar.add_portrait(make_portrait(2, 5));
        bar.update(&mut cb);

        assert_eq!(bar.portrait_index(PortraitId(1)), Some(0));
        assert_eq!(bar.portrait_index(PortraitId(2)), Some(1));
        assert_eq!(bar.portrait_index(PortraitId(99)), None);
    }

    #[test]
    fn set_all_portraits_opened() {
        let mut bar = PortraitBar::new();
        let mut cb = MockCallbacks::default();

        bar.add_portrait(make_portrait(1, 10));
        bar.add_portrait(make_portrait(2, 5));
        bar.update(&mut cb);

        bar.set_all_portraits_opened(true);
        assert!(bar.portraits().iter().all(|p| p.is_open));

        bar.set_all_portraits_opened(false);
        assert!(bar.portraits().iter().all(|p| !p.is_open));
    }

    #[test]
    fn burned_portrait_not_opened() {
        let mut bar = PortraitBar::new();
        let mut cb = MockCallbacks::default();

        let mut p = make_portrait(1, 10);
        p.is_burned = true;
        bar.add_portrait(p);
        bar.update(&mut cb);

        bar.set_all_portraits_opened(true);
        assert!(!bar.portraits()[0].is_open);
    }

    #[test]
    fn set_portrait_opened_specific() {
        let mut bar = PortraitBar::new();
        let mut cb = MockCallbacks::default();

        bar.add_portrait(make_portrait(1, 10));
        bar.add_portrait(make_portrait(2, 5));
        bar.update(&mut cb);

        bar.set_portrait_opened(PcId(1), true);
        assert!(bar.portraits()[0].is_open);
        assert!(!bar.portraits()[1].is_open);

        bar.set_portrait_opened(PcId(1), false);
        assert!(!bar.portraits()[0].is_open);
    }

    #[test]
    #[should_panic(expected = "Cannot open a burned portrait")]
    fn open_burned_portrait_panics() {
        let mut bar = PortraitBar::new();
        let mut cb = MockCallbacks::default();

        let mut p = make_portrait(1, 10);
        p.is_burned = true;
        bar.add_portrait(p);
        bar.update(&mut cb);

        bar.set_portrait_opened(PcId(1), true);
    }

    #[test]
    fn placement_all_fit() {
        let mut bar = PortraitBar::new();
        let mut cb = MockCallbacks::default();

        for i in 0..3 {
            bar.add_portrait(make_portrait(i, 10 - i as u16));
        }
        bar.update(&mut cb);

        // All 3 should be placed in slots 0..2 and enabled.
        assert_eq!(cb.placements.len(), 3);
        assert!(cb.enabled.iter().all(|&(_, e)| e));
    }

    #[test]
    fn scrolling_when_more_than_max() {
        let mut bar = PortraitBar::new();
        let mut cb = MockCallbacks::default();

        // Add 7 portraits (more than MAX_VISIBLE_PORTRAITS = 5).
        for i in 0..7 {
            bar.add_portrait(make_portrait(i, 10 - i as u16));
        }
        bar.update(&mut cb);

        // 5 enabled, 2 disabled.
        let enabled_count = cb.enabled.iter().filter(|&&(_, e)| e).count();
        let disabled_count = cb.enabled.iter().filter(|&&(_, e)| !e).count();
        assert_eq!(enabled_count, 5);
        assert_eq!(disabled_count, 2);
    }

    #[test]
    fn shift_left_and_right() {
        let mut bar = PortraitBar::new();
        let mut cb = MockCallbacks::default();

        for i in 0..7 {
            bar.add_portrait(make_portrait(i, 10 - i as u16));
        }
        bar.update(&mut cb);

        // Shift right, then update to apply placement.
        cb.clear();
        bar.shift_portraits_to_right();
        bar.update(&mut cb);
        assert!(!cb.placements.is_empty());

        // Shift left reverses the shift.
        cb.clear();
        bar.shift_portraits_to_left();
        bar.update(&mut cb);
        assert!(!cb.placements.is_empty());
    }

    #[test]
    fn shift_ignored_when_all_fit() {
        let mut bar = PortraitBar::new();
        let mut cb = MockCallbacks::default();

        bar.add_portrait(make_portrait(1, 10));
        bar.update(&mut cb);

        cb.clear();
        bar.shift_portraits_to_right();
        bar.update(&mut cb);

        // No placement callbacks because shift is a no-op when <= MAX.
        assert!(cb.placements.is_empty());
    }

    #[test]
    fn reset_clears_everything() {
        let mut bar = PortraitBar::new();
        let mut cb = MockCallbacks::default();

        bar.add_portrait(make_portrait(1, 10));
        bar.add_portrait(make_portrait(2, 5));
        bar.update(&mut cb);

        bar.reset(&mut cb);

        assert_eq!(bar.portrait_count(), 0);
    }

    #[test]
    fn add_then_cancel_with_remove_before_update() {
        let mut bar = PortraitBar::new();
        let mut cb = MockCallbacks::default();

        bar.add_portrait(make_portrait(1, 10));
        bar.remove_portrait(PortraitId(1));
        bar.update(&mut cb);

        // Should not have been added at all.
        assert_eq!(bar.portrait_count(), 0);
        assert!(cb.added.is_empty());
    }

    #[test]
    fn remove_then_cancel_with_add_before_update() {
        let mut bar = PortraitBar::new();
        let mut cb = MockCallbacks::default();

        bar.add_portrait(make_portrait(1, 10));
        bar.update(&mut cb);

        cb.clear();
        bar.remove_portrait(PortraitId(1));
        bar.add_portrait(make_portrait(1, 10));
        bar.update(&mut cb);

        // Removal should have been cancelled — portrait still present.
        assert_eq!(bar.portrait_count(), 1);
    }

    #[test]
    fn duplicate_add_ignored() {
        let mut bar = PortraitBar::new();
        let mut cb = MockCallbacks::default();

        bar.add_portrait(make_portrait(1, 10));
        bar.update(&mut cb);

        cb.clear();
        let mut p = make_portrait(1, 10);
        p.displayed = true; // already displayed
        bar.add_portrait(p);
        bar.update(&mut cb);

        assert_eq!(bar.portrait_count(), 1);
    }

    #[test]
    fn resize_displaces_visible() {
        let mut bar = PortraitBar::new();
        let mut cb = MockCallbacks::default();

        for i in 0..3 {
            bar.add_portrait(make_portrait(i, 10 - i as u16));
        }
        bar.update(&mut cb);

        cb.clear();
        bar.resize(&mut cb);

        assert_eq!(cb.placements.len(), 3);
    }

    #[test]
    fn find_in_pending_adds() {
        let mut bar = PortraitBar::new();

        bar.add_portrait(make_portrait(1, 10));
        // Not yet updated — should still be findable.
        assert!(bar.find_portrait_by_pc(PcId(1)).is_some());
    }

    #[test]
    #[should_panic(expected = "PC is in invalid state")]
    fn add_invalid_portrait_panics() {
        let mut bar = PortraitBar::new();
        let mut cb = MockCallbacks::default();

        let mut p = make_portrait(1, 10);
        p.pc_is_playable = false;
        // All conditions false — should panic during update.
        bar.add_portrait(p);
        bar.update(&mut cb);
    }

    #[test]
    fn serde_roundtrip() {
        let mut bar = PortraitBar::new();
        let mut cb = MockCallbacks::default();

        bar.add_portrait(make_portrait(1, 10));
        bar.add_portrait(make_portrait(2, 5));
        bar.update(&mut cb);

        let json = serde_json::to_string(&bar).expect("serialize");
        let bar2: PortraitBar = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(bar2.portrait_count(), 2);
        assert_eq!(bar2.portraits()[0].priority, 10);
        assert_eq!(bar2.portraits()[1].priority, 5);
    }
}
