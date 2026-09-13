//! Shipping scheduler_rle boundary; payload wire shapes remain in the parent.
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
use super::scheduler::{DecodeCodec, DecodeScheduler};
use super::*;

/// Largest independent atlas groups first, so one long RLE decode starts
/// alongside VQ instead of remaining behind many tiny animation groups.
#[cfg(any(test, all(target_arch = "wasm32", feature = "wasm-threads")))]
pub(super) fn order_rle_chunks_by_size(pending: &mut [SpriteRleJxlChunk]) {
    pending.sort_by_cached_key(|chunk| {
        std::cmp::Reverse(chunk.jxl_blobs.iter().map(Vec::len).sum::<usize>())
    });
}

/// RLE-JXL chunk decode. RLE-JXL chunks have no cross-chunk dependencies —
/// a chunk is ready as soon as its own sprite rows (which ship in the same
/// mission part) have merged. The dispatch argument is `prioritize`:
/// longest-first ordering with order-preserving removal.
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
pub enum RleJxlCodec {}

#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
impl DecodeCodec for RleJxlCodec {
    type Chunk = SpriteRleJxlChunk;
    type Output = Vec<(u32, crate::frame_holder::SpriteRaster)>;
    type Prepared = Vec<(u16, u16)>;
    type Args<'a> = bool;
    const LABEL: &'static str = "RLE-JXL";

    fn order(pending: &mut Vec<SpriteRleJxlChunk>, prioritize: bool, _limit: Option<usize>) {
        if prioritize {
            order_rle_chunks_by_size(pending);
        }
    }

    fn ready(
        bank: &ShippingSpriteBank,
        chunk: &SpriteRleJxlChunk,
        _prioritize: bool,
    ) -> Result<bool> {
        Ok(bank.rle_jxl_chunk_ready_lenient(chunk))
    }

    fn take(
        pending: &mut Vec<SpriteRleJxlChunk>,
        index: usize,
        prioritize: bool,
    ) -> SpriteRleJxlChunk {
        if prioritize {
            pending.remove(index)
        } else {
            pending.swap_remove(index)
        }
    }

    fn prepare(
        bank: &ShippingSpriteBank,
        chunk: &SpriteRleJxlChunk,
        _prioritize: bool,
    ) -> Result<Vec<(u16, u16)>> {
        bank.prepare_rle_jxl_chunk_dims(chunk)
    }

    /// Bounded jobs decode atlases serially on their assigned worker, so
    /// nested Rayon work cannot consume the worker reserved for part decode.
    fn decode(
        chunk: &SpriteRleJxlChunk,
        dims: &Vec<(u16, u16)>,
        limit: Option<usize>,
    ) -> Result<Vec<(u32, crate::frame_holder::SpriteRaster)>> {
        ShippingSpriteBank::run_rle_jxl_chunk_decode_with_parallelism(chunk, dims, limit.is_none())
    }

    fn rhs(chunk: &SpriteRleJxlChunk) -> &str {
        &chunk.rhs
    }

    fn first_sprite(chunk: &SpriteRleJxlChunk) -> Option<&u32> {
        chunk.sprite_ids.first()
    }
}

/// Worker-pool RLE-JXL chunk decode scheduler (wasm-threads builds),
/// sharing the dispatch loop with [`super::VqDecodeScheduler`].
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
pub type RleJxlDecodeScheduler = DecodeScheduler<RleJxlCodec>;

#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
impl DecodeScheduler<RleJxlCodec> {
    /// Move every ready chunk of `pending` onto the worker pool.
    pub fn dispatch_ready(
        &mut self,
        bank: &ShippingSpriteBank,
        pending: &mut Vec<SpriteRleJxlChunk>,
    ) -> Result<()> {
        self.dispatch_ready_with_limit(bank, pending, false, None)
    }

    /// Bounded jobs decode atlases serially on their assigned worker, so
    /// nested Rayon work cannot consume the worker reserved for part decode.
    pub fn dispatch_ready_bounded(
        &mut self,
        bank: &ShippingSpriteBank,
        pending: &mut Vec<SpriteRleJxlChunk>,
        max_in_flight: usize,
    ) -> Result<()> {
        self.dispatch_ready_with_limit(bank, pending, false, Some(max_in_flight))
    }

    /// Bounded admission with longest-first RLE ordering, used by the
    /// balanced mission policy independently of the VQ-first baseline.
    pub fn dispatch_ready_prioritized(
        &mut self,
        bank: &ShippingSpriteBank,
        pending: &mut Vec<SpriteRleJxlChunk>,
        max_in_flight: usize,
    ) -> Result<()> {
        self.dispatch_ready_with_limit(bank, pending, true, Some(max_in_flight))
    }
}
