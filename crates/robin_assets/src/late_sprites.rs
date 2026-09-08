//! Post-activation sprite-grid streaming registry.
//!
//! The browser (wasm-threads) mission installer activates a mission once the
//! *critical* VQ sprite chunks — everything referenced by entities present at
//! mission start — are materialized, and streams the remaining chunks
//! (reinforcement characters and their variants) on the worker pool while the
//! mission is already interactive. This registry is the hand-off point
//! between that background decode driver and every live [`FrameHolder`]:
//!
//! - [`FrameHolder::load_from_shipping`] wires each not-yet-materialized VQ
//!   sprite row to a shared [`LateGridCell`] obtained from [`cell`].
//! - The background driver publishes each decoded chunk's grids through
//!   [`publish_chunk`], filling those cells in place. `FrameHolder` clones and
//!   the published pixel-opacity generation all share the same cells, so the
//!   grids become visible to rendering and hit-testing without republishing a
//!   new frame-holder generation.
//!
//! Determinism note: nothing in the deterministic simulation reads sprite
//! pixel data — the only consumers are the renderer and the host-side mouse
//! hit-testing whose *results* are recorded into the replay command stream —
//! so when a grid pops in only rendering (and live-input focus) can change.
//!
//! Epochs guard against a mission switch racing a still-running background
//! driver: [`begin_epoch`] (called when a fresh mission install starts)
//! invalidates every outstanding cell and publish handle.
//!
//! [`FrameHolder`]: crate::frame_holder::FrameHolder
//! [`FrameHolder::load_from_shipping`]: crate::frame_holder::FrameHolder

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// Shared once-cell holding one sprite's decoded VQ index grid.
///
/// Empty until the background driver publishes the grid; readers
/// (`FrameHolder::sprite_packed_slice`) treat an empty cell as "pixels not
/// yet streamed" and degrade to a skipped draw / transparent hit test.
pub type LateGridCell = Arc<OnceLock<Arc<Vec<u16>>>>;

#[derive(Default)]
struct Registry {
    epoch: u64,
    cells: HashMap<u32, LateGridCell>,
    /// Deferred-tail bookkeeping for the current epoch, weighted by the
    /// chunk blob bytes (decode time tracks blob size closely).
    tail_blob_total: u64,
    tail_blob_done: u64,
    tail_chunks_total: usize,
    tail_chunks_done: usize,
    /// The tail driver gave up (decode error / stuck dependency). The
    /// warn log carries the details; the HUD indicator stops instead of
    /// sitting on a frozen fraction forever.
    tail_failed: bool,
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Registry::default()))
}

/// Draw calls skipped because a sprite's pixels were still streaming.
/// Diagnostic only; summarized when the tail completes.
static SKIPPED_DRAWS: AtomicU64 = AtomicU64::new(0);

/// Start a fresh mission-install epoch: drops every cell of the previous
/// mission and invalidates outstanding publish handles. Returns the new
/// epoch token the background driver must present when publishing.
pub fn begin_epoch() -> u64 {
    let mut reg = registry().lock().expect("late-sprite registry poisoned");
    reg.epoch += 1;
    reg.cells.clear();
    reg.tail_blob_total = 0;
    reg.tail_blob_done = 0;
    reg.tail_chunks_total = 0;
    reg.tail_chunks_done = 0;
    reg.tail_failed = false;
    SKIPPED_DRAWS.store(0, Ordering::Relaxed);
    reg.epoch
}

/// Get (or create) the shared grid cell for one bank sprite id.
pub fn cell(sprite_id: u32) -> LateGridCell {
    let mut reg = registry().lock().expect("late-sprite registry poisoned");
    Arc::clone(reg.cells.entry(sprite_id).or_default())
}

/// Record the deferred tail's total work for the given epoch (chunk count
/// and summed blob bytes). No-op when the epoch is stale.
pub fn set_tail_work(epoch: u64, chunks: usize, blob_bytes: u64) {
    let mut reg = registry().lock().expect("late-sprite registry poisoned");
    if reg.epoch != epoch {
        return;
    }
    reg.tail_chunks_total = chunks;
    reg.tail_blob_total = blob_bytes;
}

/// Publish one decoded chunk's grids. Returns `false` when the epoch is
/// stale (a different mission started installing); the caller must stop.
pub fn publish_chunk(epoch: u64, blob_bytes: u64, grids: &[(u32, Arc<Vec<u16>>)]) -> bool {
    let mut reg = registry().lock().expect("late-sprite registry poisoned");
    if reg.epoch != epoch {
        return false;
    }
    for (sprite_id, grid) in grids {
        // A sprite listed by two chunks decodes identically (validated by
        // the strict install path), so a lost set race is harmless.
        let _ = reg
            .cells
            .entry(*sprite_id)
            .or_default()
            .set(Arc::clone(grid));
    }
    reg.tail_chunks_done += 1;
    reg.tail_blob_done += blob_bytes;
    true
}

/// Mark the tail as abandoned for this epoch (details go to the caller's
/// warn log). Hides the progress indicator rather than freezing it.
pub fn fail_tail(epoch: u64) {
    let mut reg = registry().lock().expect("late-sprite registry poisoned");
    if reg.epoch != epoch {
        return;
    }
    reg.tail_failed = true;
}

/// Progress of the background sprite-streaming tail, blob-byte weighted:
/// `(fraction, chunks_done, chunks_total)`. `None` when no tail is running
/// (nothing deferred, tail finished, or tail abandoned).
pub fn tail_status() -> Option<(f32, usize, usize)> {
    let reg = registry().lock().expect("late-sprite registry poisoned");
    if reg.tail_failed
        || reg.tail_chunks_total == 0
        || reg.tail_chunks_done >= reg.tail_chunks_total
    {
        return None;
    }
    let fraction = if reg.tail_blob_total == 0 {
        0.0
    } else {
        (reg.tail_blob_done as f64 / reg.tail_blob_total as f64) as f32
    };
    Some((fraction, reg.tail_chunks_done, reg.tail_chunks_total))
}

/// Count one draw call skipped because the sprite's grid has not streamed
/// in yet. Returns the running total.
pub fn note_skipped_draw() -> u64 {
    SKIPPED_DRAWS.fetch_add(1, Ordering::Relaxed) + 1
}

/// Total draw calls skipped since the current epoch began.
pub fn skipped_draws() -> u64 {
    SKIPPED_DRAWS.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The registry is process-global; serialize the tests that reset it.
    fn test_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .expect("late-sprite test lock poisoned")
    }

    #[test]
    fn stale_epoch_publish_is_rejected_and_cells_reset() {
        let _guard = test_lock();
        let old = begin_epoch();
        let cell_before = cell(7);
        let grid = Arc::new(vec![1u16, 2, 3]);
        assert!(publish_chunk(old, 10, &[(7, Arc::clone(&grid))]));
        assert_eq!(cell_before.get(), Some(&grid));

        let new = begin_epoch();
        assert_ne!(old, new);
        // The old cell handle stays filled (harmless — its FrameHolder is
        // being replaced), but the registry no longer hands it out.
        assert!(cell(7).get().is_none());
        assert!(!publish_chunk(old, 10, &[(7, grid)]));
    }

    #[test]
    fn tail_status_tracks_blob_weighted_progress() {
        let _guard = test_lock();
        let epoch = begin_epoch();
        assert_eq!(tail_status(), None);
        set_tail_work(epoch, 2, 100);
        assert_eq!(tail_status(), Some((0.0, 0, 2)));
        assert!(publish_chunk(epoch, 75, &[(1, Arc::new(vec![0u16]))]));
        let (fraction, done, total) = tail_status().expect("tail running");
        assert!((fraction - 0.75).abs() < 1e-6);
        assert_eq!((done, total), (1, 2));
        assert!(publish_chunk(epoch, 25, &[(2, Arc::new(vec![0u16]))]));
        assert_eq!(tail_status(), None);
    }

    #[test]
    fn failed_tail_hides_the_indicator() {
        let _guard = test_lock();
        let epoch = begin_epoch();
        set_tail_work(epoch, 3, 300);
        assert!(tail_status().is_some());
        fail_tail(epoch);
        assert_eq!(tail_status(), None);
    }
}
