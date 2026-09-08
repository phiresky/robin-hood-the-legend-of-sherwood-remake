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
//! Deterministic orientation commands also read sprite opacity. The opt-in
//! startup experiment therefore installs exact resident opacity before exposing
//! deferred grids, and presentation must wait for required grids explicitly.
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
    failure: Option<String>,
    experimental: bool,
    tail_downloads_pending: bool,
    opacity: HashMap<u32, Arc<crate::sprite_residency::SpriteOpacity>>,
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
    reg.failure = None;
    reg.experimental = false;
    reg.tail_downloads_pending = false;
    reg.opacity.clear();
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

/// Keep readiness pending while experimental tail parts are still being fetched.
pub fn begin_download_tail(epoch: u64) {
    let mut reg = registry().lock().expect("late-sprite registry poisoned");
    if reg.epoch == epoch {
        reg.tail_downloads_pending = true;
    }
}

pub fn extend_tail_work(epoch: u64, chunks: usize, blob_bytes: u64) {
    let mut reg = registry().lock().expect("late-sprite registry poisoned");
    if reg.epoch != epoch {
        return;
    }
    reg.tail_chunks_total = reg
        .tail_chunks_total
        .checked_add(chunks)
        .expect("sprite tail chunk count overflow");
    reg.tail_blob_total = reg
        .tail_blob_total
        .checked_add(blob_bytes)
        .expect("sprite tail byte count overflow");
}

pub fn finish_download_tail(epoch: u64) {
    let mut reg = registry().lock().expect("late-sprite registry poisoned");
    if reg.epoch == epoch {
        reg.tail_downloads_pending = false;
    }
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
    reg.failure
        .get_or_insert_with(|| "sprite streaming tail failed; see decode log".to_owned());
}

/// Explicit readiness for the opt-in host presentation gate.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Readiness {
    Pending,
    Ready,
    Failed(String),
    Superseded,
}

pub fn set_experimental(epoch: u64, enabled: bool) -> bool {
    let mut reg = registry().lock().expect("late-sprite registry poisoned");
    if reg.epoch != epoch {
        return false;
    }
    reg.experimental = enabled;
    true
}

pub fn experimental_epoch() -> Option<u64> {
    let reg = registry().lock().expect("late-sprite registry poisoned");
    reg.experimental.then_some(reg.epoch)
}

pub fn install_opacity(
    epoch: u64,
    batch: crate::sprite_residency::OpacityBatch,
) -> anyhow::Result<()> {
    batch.validate()?;
    let mut reg = registry().lock().expect("late-sprite registry poisoned");
    anyhow::ensure!(reg.epoch == epoch, "opacity install epoch was superseded");
    for sprite in &batch.sprites {
        if let Some(existing) = reg.opacity.get(&sprite.bank_id) {
            anyhow::ensure!(
                existing.as_ref() == sprite,
                "conflicting opacity for sprite {}",
                sprite.bank_id
            );
        }
    }
    for sprite in batch.sprites {
        reg.opacity.insert(sprite.bank_id, Arc::new(sprite));
    }
    Ok(())
}

pub(crate) fn opacity(sprite_id: u32) -> Option<Arc<crate::sprite_residency::SpriteOpacity>> {
    let reg = registry().lock().expect("late-sprite registry poisoned");
    if !reg.experimental {
        return None;
    }
    reg.opacity.get(&sprite_id).cloned()
}

pub fn fail_tail_with_error(epoch: u64, error: String) {
    let mut reg = registry().lock().expect("late-sprite registry poisoned");
    if reg.epoch != epoch {
        return;
    }
    reg.tail_failed = true;
    reg.failure = Some(error);
}

/// IDs absent from the late-cell registry are resident bank rows. A completed
/// driver with an empty requested cell is corruption, never a successful wait.
pub fn readiness(epoch: u64, sprite_ids: &[u32]) -> Readiness {
    let reg = registry().lock().expect("late-sprite registry poisoned");
    if reg.epoch != epoch {
        return Readiness::Superseded;
    }
    if let Some(error) = &reg.failure {
        return Readiness::Failed(error.clone());
    }
    let pending = sprite_ids
        .iter()
        .find(|id| reg.cells.get(id).is_some_and(|cell| cell.get().is_none()));
    if let Some(id) = pending {
        if !reg.tail_downloads_pending && reg.tail_chunks_done >= reg.tail_chunks_total {
            return Readiness::Failed(format!("completed sprite tail did not publish sprite {id}"));
        }
        Readiness::Pending
    } else {
        Readiness::Ready
    }
}

pub fn all_readiness(epoch: u64) -> Readiness {
    let reg = registry().lock().expect("late-sprite registry poisoned");
    if reg.epoch != epoch {
        return Readiness::Superseded;
    }
    if let Some(error) = &reg.failure {
        return Readiness::Failed(error.clone());
    }
    if reg.tail_downloads_pending || reg.tail_chunks_done < reg.tail_chunks_total {
        return Readiness::Pending;
    }
    if reg.cells.values().any(|cell| cell.get().is_none()) {
        return Readiness::Failed("completed sprite tail has unfilled cells".to_owned());
    }
    Readiness::Ready
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
    fn experimental_readiness_preserves_failures_and_epoch_identity() {
        let _guard = test_lock();
        let epoch = begin_epoch();
        assert_eq!(experimental_epoch(), None);
        assert!(set_experimental(epoch, true));
        assert_eq!(experimental_epoch(), Some(epoch));
        let _cell = cell(42);
        set_tail_work(epoch, 1, 10);
        assert_eq!(readiness(epoch, &[42]), Readiness::Pending);
        assert_eq!(readiness(epoch, &[99]), Readiness::Ready);
        fail_tail_with_error(epoch, "decode corrupt".to_owned());
        assert_eq!(
            readiness(epoch, &[42]),
            Readiness::Failed("decode corrupt".to_owned())
        );
        assert_eq!(
            all_readiness(epoch),
            Readiness::Failed("decode corrupt".to_owned())
        );
        let next = begin_epoch();
        assert_eq!(readiness(epoch, &[42]), Readiness::Superseded);
        assert_eq!(experimental_epoch(), None);
        let _cell = cell(42);
        set_tail_work(next, 1, 10);
        publish_chunk(next, 10, &[]);
        assert!(matches!(readiness(next, &[42]), Readiness::Failed(_)));
        assert!(matches!(all_readiness(next), Readiness::Failed(_)));
    }

    #[test]
    fn incremental_downloads_keep_unfilled_cells_pending() {
        let _guard = test_lock();
        let epoch = begin_epoch();
        begin_download_tail(epoch);
        let _first = cell(1);
        let _second = cell(2);
        assert_eq!(all_readiness(epoch), Readiness::Pending);
        assert_eq!(readiness(epoch, &[1]), Readiness::Pending);
        extend_tail_work(epoch, 1, 4);
        publish_chunk(epoch, 4, &[(1, Arc::new(vec![0]))]);
        assert_eq!(readiness(epoch, &[1]), Readiness::Ready);
        assert_eq!(readiness(epoch, &[2]), Readiness::Pending);
        assert_eq!(all_readiness(epoch), Readiness::Pending);
        extend_tail_work(epoch, 1, 8);
        publish_chunk(epoch, 8, &[(2, Arc::new(vec![0]))]);
        finish_download_tail(epoch);
        assert_eq!(all_readiness(epoch), Readiness::Ready);
        begin_epoch();
        finish_download_tail(epoch);
        assert_eq!(all_readiness(epoch), Readiness::Superseded);
    }

    #[test]
    fn opacity_install_rejects_conflicts_and_stale_epochs() {
        let _guard = test_lock();
        let epoch = begin_epoch();
        set_experimental(epoch, true);
        let mut batch = crate::sprite_residency::OpacityBatch {
            sprites: vec![crate::sprite_residency::SpriteOpacity {
                bank_id: 7,
                width: 4,
                height: 1,
                dictionary_index: 0,
                ordinary: vec![4],
                blipped: vec![6],
            }],
        };
        install_opacity(epoch, batch.clone()).unwrap();
        assert!(opacity(7).unwrap().is_opaque(1, 0, true));
        install_opacity(epoch, batch.clone()).unwrap();
        batch.sprites[0].ordinary[0] = 0;
        assert!(install_opacity(epoch, batch.clone()).is_err());
        begin_epoch();
        assert!(install_opacity(epoch, batch).is_err());
        assert!(opacity(7).is_none());
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
