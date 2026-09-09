//! Mission-owned post-activation sprite streaming. Simulation-required opacity
//! must already be resident before activation; pending visual rows may skip draws.
//!
//! The installed mission and its frame-holder generations share [`SpriteStreaming`].
//! A [`SpritePublisher`] holds only a weak handle: abandoned preparations do not
//! survive solely because a background decode is still running. Successful mission
//! replacement retires the old handle; failed replacement leaves it usable. Neither
//! operation affects a stream belonging to another asset installation.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};

/// One decoded sprite grid, filled at most once and shared by holder clones.
pub type LateGridCell = Arc<OnceLock<Arc<Vec<u16>>>>;

#[derive(Debug, Default)]
struct Registry {
    cells: HashMap<u32, LateGridCell>,
    retired: bool,
    failed: bool,
    total_bytes: u64,
    done_bytes: u64,
    total_chunks: usize,
    done_chunks: usize,
    skipped_draws: u64,
}

/// Shared by one installed mission and its immutable frame-holder generations.
/// Serialization never persists runtime publication or progress state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SpriteStreaming {
    #[serde(skip)]
    registry: Arc<Mutex<Registry>>,
}

/// Background work does not keep an abandoned mission alive.
#[derive(Debug, Serialize, Deserialize)]
pub struct SpritePublisher {
    #[serde(skip)]
    registry: Weak<Mutex<Registry>>,
}

impl SpriteStreaming {
    pub fn cell(&self, sprite_id: u32) -> LateGridCell {
        let mut reg = self.registry.lock().expect("sprite streaming poisoned");
        Arc::clone(reg.cells.entry(sprite_id).or_default())
    }

    pub fn publisher(&self, chunks: usize, blob_bytes: u64) -> SpritePublisher {
        let mut reg = self.registry.lock().expect("sprite streaming poisoned");
        assert!(!reg.retired, "cannot start a retired sprite stream");
        assert_eq!(reg.total_chunks, 0, "sprite tail already started");
        reg.total_chunks = chunks;
        reg.total_bytes = blob_bytes;
        SpritePublisher {
            registry: Arc::downgrade(&self.registry),
        }
    }

    /// Called only after a replacement mission successfully commits.
    pub fn retire(&self) {
        self.registry
            .lock()
            .expect("sprite streaming poisoned")
            .retired = true;
    }

    pub fn tail_status(&self) -> Option<(f32, usize, usize)> {
        let reg = self.registry.lock().expect("sprite streaming poisoned");
        if reg.retired || reg.failed || reg.total_chunks == 0 || reg.done_chunks >= reg.total_chunks
        {
            return None;
        }
        let fraction = if reg.total_bytes == 0 {
            0.0
        } else {
            (reg.done_bytes as f64 / reg.total_bytes as f64) as f32
        };
        Some((fraction, reg.done_chunks, reg.total_chunks))
    }

    pub fn note_skipped_draw(&self) -> u64 {
        let mut reg = self.registry.lock().expect("sprite streaming poisoned");
        reg.skipped_draws += 1;
        reg.skipped_draws
    }
}

impl SpritePublisher {
    pub fn is_retired(&self) -> bool {
        self.registry
            .upgrade()
            .is_none_or(|registry| registry.lock().expect("sprite streaming poisoned").retired)
    }

    pub fn publish_chunk(&self, blob_bytes: u64, grids: &[(u32, Arc<Vec<u16>>)]) -> bool {
        let Some(registry) = self.registry.upgrade() else {
            return false;
        };
        let mut reg = registry.lock().expect("sprite streaming poisoned");
        if reg.retired {
            return false;
        }
        for (id, grid) in grids {
            // The strict decoder validates duplicate sprite rows as identical.
            let _ = reg.cells.entry(*id).or_default().set(Arc::clone(grid));
        }
        reg.done_chunks += 1;
        reg.done_bytes += blob_bytes;
        true
    }

    pub fn fail_tail(&self) {
        if let Some(registry) = self.registry.upgrade() {
            registry.lock().expect("sprite streaming poisoned").failed = true;
        }
    }

    pub fn skipped_draws(&self) -> Option<u64> {
        self.registry.upgrade().map(|registry| {
            registry
                .lock()
                .expect("sprite streaming poisoned")
                .skipped_draws
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_ids_and_retirement_are_mission_local() {
        let first = SpriteStreaming::default();
        let second = SpriteStreaming::default();
        let old_cell = first.cell(7);
        let new_cell = second.cell(7);
        let old = first.publisher(2, 100);
        let new = second.publisher(2, 100);
        assert!(old.publish_chunk(75, &[(7, Arc::new(vec![1]))]));
        assert_eq!(first.tail_status(), Some((0.75, 1, 2)));
        assert!(new_cell.get().is_none());
        first.note_skipped_draw();
        assert_eq!(old.skipped_draws(), Some(1));
        assert_eq!(new.skipped_draws(), Some(0));
        first.retire();
        assert!(!old.publish_chunk(25, &[(8, Arc::new(vec![3]))]));
        assert!(new.publish_chunk(75, &[(7, Arc::new(vec![2]))]));
        assert_eq!(old_cell.get().unwrap().as_slice(), &[1]);
        assert_eq!(new_cell.get().unwrap().as_slice(), &[2]);
        assert_eq!(first.tail_status(), None);
        assert_eq!(second.tail_status(), Some((0.75, 1, 2)));
        assert!(new.publish_chunk(25, &[]));
        assert_eq!(second.tail_status(), None);
    }

    #[test]
    fn failed_tail_and_dropped_owner_do_not_affect_another_mission() {
        let first = SpriteStreaming::default();
        let second = SpriteStreaming::default();
        let old = first.publisher(3, 300);
        let new = second.publisher(3, 300);
        old.fail_tail();
        assert_eq!(first.tail_status(), None);
        assert!(second.tail_status().is_some());
        drop(first);
        assert!(old.is_retired());
        assert!(!old.publish_chunk(100, &[]));
        assert!(!new.is_retired());
    }
}
