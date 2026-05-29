//! Game state change tracking for undo/replay.
//!
//! Flat struct with a type tag, paired with a [`ChangeLog`] backed by a `Vec`.

use crate::coordinates::{MapPoint, ScreenPoint};
use serde::{Deserialize, Serialize};

/// Axis-aligned bounding box stored in [`Change`] records.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
)]
pub struct BoundingBox2D {
    pub min: ScreenPoint,
    pub max: ScreenPoint,
}

/// Surface material for sound interaction.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
)]
pub struct Material(pub u16);

/// Opaque handle to a sound source.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub struct SoundSourceId(pub u32);

// ---------------------------------------------------------------------------
// ChangeType
// ---------------------------------------------------------------------------

/// Discriminant for [`Change`].
///
/// A "none" variant is not represented — a [`Change`] always carries valid data.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub enum ChangeType {
    Mouse,
    Transition,
    Rectangle,
    Sound,
    SoundStop,
}

// ---------------------------------------------------------------------------
// Change
// ---------------------------------------------------------------------------

/// A single recorded engine-state change.
///
/// Variant-specific fields are only meaningful when `change_type` matches;
/// constructors enforce this.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct Change {
    pub change_type: ChangeType,

    // -- Mouse --
    pub mouse_pointer_id: u16,
    pub mouse_flags: u32,

    // -- Rectangle --
    pub dirty_rectangle: BoundingBox2D,

    // -- Sound / SoundStop --
    pub is_fx: bool,
    pub sound_source: Option<SoundSourceId>,
    pub sound_id: u16,
    pub material: Material,
    pub position: MapPoint,
}

impl Change {
    /// Create a mouse-pointer change.
    pub fn mouse(pointer_id: u16, flags: u32) -> Self {
        Self {
            change_type: ChangeType::Mouse,
            mouse_pointer_id: pointer_id,
            mouse_flags: flags,
            ..Self::zeroed(ChangeType::Mouse)
        }
    }

    /// Create a transition change.
    pub fn transition() -> Self {
        Self::zeroed(ChangeType::Transition)
    }

    /// Create a dirty-rectangle change.
    pub fn rectangle(rect: BoundingBox2D) -> Self {
        Self {
            dirty_rectangle: rect,
            ..Self::zeroed(ChangeType::Rectangle)
        }
    }

    /// Create an FX sound change (one-shot, position-based).
    pub fn sound_fx(sound_id: u16, position: MapPoint, material: Material) -> Self {
        Self {
            is_fx: true,
            sound_id,
            position,
            material,
            ..Self::zeroed(ChangeType::Sound)
        }
    }

    /// Create a source-based sound change.
    pub fn sound_source(source: SoundSourceId) -> Self {
        Self {
            is_fx: false,
            sound_source: Some(source),
            ..Self::zeroed(ChangeType::Sound)
        }
    }

    /// Create a sound-stop change.
    pub fn sound_stop(source: SoundSourceId) -> Self {
        Self {
            sound_source: Some(source),
            ..Self::zeroed(ChangeType::SoundStop)
        }
    }

    /// Helper: zero-initialised struct with a given type tag.
    fn zeroed(change_type: ChangeType) -> Self {
        Self {
            change_type,
            mouse_pointer_id: 0,
            mouse_flags: 0,
            dirty_rectangle: BoundingBox2D::default(),
            is_fx: false,
            sound_source: None,
            sound_id: 0,
            material: Material::default(),
            position: MapPoint::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// ChangeLog
// ---------------------------------------------------------------------------

/// Ordered log of [`Change`]s.
#[derive(Debug, Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct ChangeLog {
    changes: Vec<Change>,
}

impl ChangeLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a change to the log.
    pub fn record(&mut self, change: Change) {
        self.changes.push(change);
    }

    /// Remove and return the most recent change, or `None` if empty.
    pub fn undo_last(&mut self) -> Option<Change> {
        self.changes.pop()
    }

    /// Discard all recorded changes.
    pub fn clear(&mut self) {
        self.changes.clear();
    }

    /// Number of recorded changes.
    pub fn count(&self) -> usize {
        self.changes.len()
    }

    /// Iterate over changes in recording order.
    pub fn iter(&self) -> impl Iterator<Item = &Change> {
        self.changes.iter()
    }

    /// Access a change by index.
    pub fn get(&self, index: usize) -> Option<&Change> {
        self.changes.get(index)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_and_count() {
        let mut log = ChangeLog::new();
        assert_eq!(log.count(), 0);

        log.record(Change::mouse(1, 0));
        log.record(Change::transition());
        assert_eq!(log.count(), 2);
    }

    #[test]
    fn undo_last_returns_most_recent() {
        let mut log = ChangeLog::new();
        log.record(Change::mouse(1, 0x10));
        log.record(Change::rectangle(BoundingBox2D {
            min: ScreenPoint { x: 0.0, y: 0.0 },
            max: ScreenPoint { x: 100.0, y: 50.0 },
        }));

        let undone = log.undo_last().expect("should have a change");
        assert_eq!(undone.change_type, ChangeType::Rectangle);
        assert_eq!(log.count(), 1);
    }

    #[test]
    fn undo_last_empty_returns_none() {
        let mut log = ChangeLog::new();
        assert!(log.undo_last().is_none());
    }

    #[test]
    fn clear_removes_all() {
        let mut log = ChangeLog::new();
        log.record(Change::mouse(1, 0));
        log.record(Change::mouse(2, 0));
        log.clear();
        assert_eq!(log.count(), 0);
    }

    #[test]
    fn mouse_change_fields() {
        let c = Change::mouse(42, 0xFF);
        assert_eq!(c.change_type, ChangeType::Mouse);
        assert_eq!(c.mouse_pointer_id, 42);
        assert_eq!(c.mouse_flags, 0xFF);
    }

    #[test]
    fn sound_fx_fields() {
        let pos = MapPoint { x: 10.0, y: 20.0 };
        let mat = Material(3);
        let c = Change::sound_fx(7, pos, mat);
        assert_eq!(c.change_type, ChangeType::Sound);
        assert!(c.is_fx);
        assert_eq!(c.sound_id, 7);
        assert_eq!(c.position, pos);
        assert_eq!(c.material, mat);
    }

    #[test]
    fn sound_source_change() {
        let src = SoundSourceId(99);
        let c = Change::sound_source(src);
        assert_eq!(c.change_type, ChangeType::Sound);
        assert!(!c.is_fx);
        assert_eq!(c.sound_source, Some(src));
    }

    #[test]
    fn sound_stop_change() {
        let src = SoundSourceId(5);
        let c = Change::sound_stop(src);
        assert_eq!(c.change_type, ChangeType::SoundStop);
        assert_eq!(c.sound_source, Some(src));
    }

    #[test]
    fn serde_roundtrip() {
        let mut log = ChangeLog::new();
        log.record(Change::mouse(1, 0));
        log.record(Change::sound_fx(
            10,
            MapPoint { x: 1.0, y: 2.0 },
            Material(1),
        ));
        log.record(Change::sound_stop(SoundSourceId(42)));

        let json = serde_json::to_string(&log).expect("serialize");
        let restored: ChangeLog = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.count(), 3);

        let first = restored.iter().next().unwrap();
        assert_eq!(first.change_type, ChangeType::Mouse);
    }

    #[test]
    fn iter_preserves_order() {
        let mut log = ChangeLog::new();
        log.record(Change::mouse(1, 0));
        log.record(Change::transition());
        log.record(Change::rectangle(BoundingBox2D::default()));

        let types: Vec<_> = log.iter().map(|c| c.change_type).collect();
        assert_eq!(
            types,
            vec![
                ChangeType::Mouse,
                ChangeType::Transition,
                ChangeType::Rectangle
            ]
        );
    }
}
