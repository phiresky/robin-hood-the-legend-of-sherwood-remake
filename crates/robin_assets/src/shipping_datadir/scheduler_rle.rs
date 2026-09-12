//! Shipping scheduler_rle boundary; payload wire shapes remain in the parent.
use super::*;

/// Largest independent atlas groups first, so one long RLE decode starts
/// alongside VQ instead of remaining behind many tiny animation groups.
#[cfg(any(test, all(target_arch = "wasm32", feature = "wasm-threads")))]
pub(super) fn order_rle_chunks_by_size(pending: &mut [SpriteRleJxlChunk]) {
    pending.sort_by_cached_key(|chunk| {
        std::cmp::Reverse(chunk.jxl_blobs.iter().map(Vec::len).sum::<usize>())
    });
}

/// Dispatcher state for worker-pool RLE-JXL chunk decode (wasm-threads
/// builds), mirroring [`VqDecodeScheduler`]. RLE-JXL chunks have no
/// cross-chunk dependencies — a chunk is ready as soon as its own sprite
/// rows (which ship in the same mission part) have merged.
///
/// Transient scheduling state, never serialized — deliberately no serde.
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
#[derive(Default)]
pub struct RleJxlDecodeScheduler {
    in_flight: super::decode_jobs::DecodeJobs<
        SpriteRleJxlChunk,
        Vec<(u32, crate::frame_holder::SpriteRaster)>,
    >,
}

#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
impl RleJxlDecodeScheduler {
    /// Move every ready chunk of `pending` onto the worker pool.
    pub fn dispatch_ready(
        &mut self,
        bank: &ShippingSpriteBank,
        pending: &mut Vec<SpriteRleJxlChunk>,
    ) -> Result<()> {
        self.dispatch_ready_with_limit(bank, pending, None, false)
    }

    /// Bounded jobs decode atlases serially on their assigned worker, so
    /// nested Rayon work cannot consume the worker reserved for part decode.
    pub fn dispatch_ready_bounded(
        &mut self,
        bank: &ShippingSpriteBank,
        pending: &mut Vec<SpriteRleJxlChunk>,
        max_in_flight: usize,
    ) -> Result<()> {
        self.dispatch_ready_with_limit(bank, pending, Some(max_in_flight), false)
    }

    /// Bounded admission with longest-first RLE ordering, used by the
    /// balanced mission policy independently of the VQ-first baseline.
    pub fn dispatch_ready_prioritized(
        &mut self,
        bank: &ShippingSpriteBank,
        pending: &mut Vec<SpriteRleJxlChunk>,
        max_in_flight: usize,
    ) -> Result<()> {
        self.dispatch_ready_with_limit(bank, pending, Some(max_in_flight), true)
    }

    fn dispatch_ready_with_limit(
        &mut self,
        bank: &ShippingSpriteBank,
        pending: &mut Vec<SpriteRleJxlChunk>,
        limit: Option<usize>,
        prioritize: bool,
    ) -> Result<()> {
        let max_in_flight = limit.unwrap_or(usize::MAX);
        if self.in_flight.len() >= max_in_flight {
            return Ok(());
        }
        if prioritize {
            order_rle_chunks_by_size(pending);
        }
        let mut index = 0;
        while index < pending.len() && self.in_flight.len() < max_in_flight {
            if !bank.rle_jxl_chunk_ready_lenient(&pending[index]) {
                index += 1;
                continue;
            }
            let ready = tracing::enabled!(tracing::Level::DEBUG).then(js_sys::Date::now);
            let chunk = if prioritize {
                pending.remove(index)
            } else {
                pending.swap_remove(index)
            };
            let dims = bank
                .prepare_rle_jxl_chunk_dims(&chunk)
                .with_context(|| format!("decode RLE-JXL sprite chunk for {}", chunk.rhs))?;
            self.in_flight.spawn(chunk, ready, move |chunk| {
                ShippingSpriteBank::run_rle_jxl_chunk_decode_with_parallelism(
                    chunk,
                    &dims,
                    limit.is_none(),
                )
            });
        }
        Ok(())
    }

    /// Await the next completed decode. `Ok(None)` when none is in flight.
    pub async fn next_decoded(
        &mut self,
    ) -> Result<
        Option<(
            SpriteRleJxlChunk,
            Vec<(u32, crate::frame_holder::SpriteRaster)>,
        )>,
    > {
        let Some((chunk, packed, timing)) = self.in_flight.next("RLE-JXL").await? else {
            return Ok(None);
        };
        let packed =
            packed.with_context(|| format!("decode RLE-JXL sprite chunk for {}", chunk.rhs))?;
        if let Some([ready_ms, enqueued_ms, worker_start_ms, worker_end_ms]) = timing {
            let received_ms = js_sys::Date::now();
            tracing::debug!(chunk = %chunk.rhs, first_sprite = ?chunk.sprite_ids.first(), ready_ms, enqueued_ms, worker_start_ms,
                worker_end_ms, received_ms, decode_ms = worker_end_ms - worker_start_ms,
                "RLE-JXL sprite chunk decoded on worker");
        }
        Ok(Some((chunk, packed)))
    }

    /// Includes completed results until the caller consumes them.
    pub fn in_flight_count(&self) -> usize {
        self.in_flight.len()
    }

    /// True while at least one decode is running on the pool.
    pub fn has_in_flight(&self) -> bool {
        !self.in_flight.is_empty()
    }
}
