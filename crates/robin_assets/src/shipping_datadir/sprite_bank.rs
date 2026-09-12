//! Shipping sprite_bank boundary; payload wire shapes remain in the parent.
use super::*;

/// Error for a fixpoint round that made no progress: every remaining chunk
/// names base sprites that never materialized, meaning the manifest omitted a
/// base RHS chunk from the mission payload.
fn vq_chunks_stuck_error(still_pending: &[SpriteVqChunk]) -> anyhow::Error {
    let stuck: Vec<String> = still_pending
        .iter()
        .map(|chunk| {
            let mut label = format!(
                "{} (base {}",
                chunk.rhs,
                chunk.base_rhs.as_deref().unwrap_or("?")
            );
            if !chunk.base2_rhs.is_empty() {
                label.push_str(&format!(", base2 {}", chunk.base2_rhs));
            }
            label.push(')');
            label
        })
        .collect();
    anyhow!(
        "VQ sprite chunks cannot be decoded because their base sprites never \
         materialized — base RHS chunk missing from the mission payload: {}",
        stuck.join(", ")
    )
}

/// Owned inputs for one chunk's grid decode, resolved from the bank by
/// [`ShippingSpriteBank::prepare_vq_chunk_inputs`]. `Send + 'static` (base
/// grids are `Arc`-shared with the bank rows) so the decode itself can run on
/// a rayon worker while the bank stays borrowed on the dispatching thread.
/// Transient decode state, never serialized — deliberately no serde derives.
pub(super) struct VqChunkDecodeInputs {
    /// Per sprite: `(width / 4, height)` — the VQ grid dimensions.
    dims: Vec<(u16, u16)>,
    selfref: Vec<Option<crate::sprite_codec::SelfRef>>,
    base_grids: Vec<Option<Arc<Vec<u16>>>>,
    base2_grids: Vec<Option<Arc<Vec<u16>>>>,
}

impl ShippingSpriteBank {
    /// Bound the merged bank, not merely each compressed input part. Includes
    /// dense runtime slots and dictionary copies; decoded atlas backing is
    /// counted once even when many sprites borrow windows from the same atlas.
    pub(crate) fn validate_resident_budget(&self) -> Result<()> {
        const LIMIT: usize = 1024 * 1024 * 1024;
        let mut bytes = (self.sprite_count as usize)
            .checked_mul(std::mem::size_of::<crate::frame_holder::PackedSprite>())
            .ok_or_else(|| anyhow!("shipping sprite slot bytes overflow"))?;
        let mut charge = |amount: usize| -> Result<()> {
            bytes = bytes
                .checked_add(amount)
                .ok_or_else(|| anyhow!("shipping sprite resident bytes overflow"))?;
            if bytes > LIMIT {
                bail!("shipping sprite bank exceeds {LIMIT} estimated resident bytes");
            }
            Ok(())
        };
        charge(0)?;
        for dictionary in &self.dictionaries {
            charge(
                dictionary
                    .resident_bytes()
                    .checked_mul(3)
                    .ok_or_else(|| anyhow!("shipping dictionary variant bytes overflow"))?,
            )?;
        }
        let mut atlases = BTreeSet::new();
        for (id, sprite) in &self.sprites {
            let pixels =
                crate::packed_sprite::pixel_count(sprite.width.into(), sprite.height.into())
                    .with_context(|| format!("shipping sprite {id}"))?;
            let packed_bytes = sprite
                .packed_data
                .len()
                .checked_mul(2)
                .ok_or_else(|| anyhow!("shipping sprite {id} packed bytes overflow"))?;
            if let Some(raster) = &sprite.raster {
                charge(packed_bytes)?;
                if atlases.insert(Arc::as_ptr(&raster.atlas)) {
                    charge(
                        raster
                            .atlas
                            .len()
                            .checked_mul(2)
                            .ok_or_else(|| anyhow!("shipping sprite atlas bytes overflow"))?,
                    )?;
                }
            } else {
                let expected = if sprite.dictionary_index == UNMAPPED_DICT {
                    pixels
                        .checked_mul(2)
                        .ok_or_else(|| anyhow!("shipping raster bytes overflow"))?
                } else {
                    pixels / 2
                };
                charge(packed_bytes.max(expected))?;
            }
        }
        Ok(())
    }
    /// Decode every [`SpriteVqChunk`] blob back into per-sprite packed index
    /// data, consuming the chunk list.
    ///
    /// Chunks coded against a family base need that base's grids first; the
    /// base always arrives in a separate chunk of the same mission closure
    /// and mission parts may merge in any fetch-completion order, so decoding
    /// iterates to a fixpoint over the chunk list. A chunk whose base sprites
    /// are missing from the payload altogether is a hard error — the
    /// conversion lists the base RHS chunk as an explicit dependency, so its
    /// absence means a broken manifest, never something to paper over.
    pub fn materialize_vq_chunks(&mut self, rhs_files: &BTreeMap<String, RhsData>) -> Result<()> {
        self.validate_resident_budget()?;
        let mut pending = std::mem::take(&mut self.vq_chunks);
        while !pending.is_empty() {
            let mut ready = Vec::new();
            let mut still_pending = Vec::new();
            for chunk in pending {
                if self.vq_chunk_bases_ready(&chunk)? {
                    ready.push(chunk);
                } else {
                    still_pending.push(chunk);
                }
            }
            let made_progress = !ready.is_empty();
            // Chunks within one fixpoint round are independent (their bases
            // are already materialized), so decode them in parallel on
            // native; wasm has no thread pool and stays serial.
            #[cfg(not(target_arch = "wasm32"))]
            let decoded: Vec<(SpriteVqChunk, Result<Vec<(u32, Vec<u16>)>>)> = {
                use rayon::prelude::*;
                ready
                    .into_par_iter()
                    .map(|chunk| {
                        let grids = self.decode_vq_chunk(&chunk, rhs_files);
                        (chunk, grids)
                    })
                    .collect()
            };
            #[cfg(target_arch = "wasm32")]
            let decoded: Vec<(SpriteVqChunk, Result<Vec<(u32, Vec<u16>)>>)> = ready
                .into_iter()
                .map(|chunk| {
                    let grids = self.decode_vq_chunk(&chunk, rhs_files);
                    (chunk, grids)
                })
                .collect();
            for (chunk, grids) in decoded {
                let grids =
                    grids.with_context(|| format!("decode VQ sprite chunk for {}", chunk.rhs))?;
                self.apply_decoded_vq_chunk(&chunk, grids)?;
            }
            if !made_progress {
                return Err(vq_chunks_stuck_error(&still_pending));
            }
            pending = still_pending;
        }
        Ok(())
    }

    /// Parallel wasm counterpart of [`Self::materialize_vq_chunks`]: each
    /// chunk's decode is dispatched to the rayon worker pool the moment its
    /// base grids exist, while this (main) thread only prepares inputs and
    /// applies results. Awaiting instead of blocking matters on the browser
    /// main thread, which must never `atomics.wait`.
    ///
    /// Unlike the serial fixpoint's rounds, there is no barrier here: one
    /// family's variants start decoding as soon as their own hub is applied,
    /// not when the slowest chunk of the previous round happens to finish —
    /// the wall time is bounded by the longest single dependency chain, and
    /// hub -> variant chains are at most a few links deep.
    ///
    /// Falls back to the serial [`Self::materialize_vq_chunks`] when the pool
    /// was never initialized (page not cross-origin isolated).
    #[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
    pub async fn materialize_vq_chunks_parallel(
        &mut self,
        rhs_files: &BTreeMap<String, RhsData>,
    ) -> Result<()> {
        if crate::wasm_threads::pool_threads() == 0 {
            return self.materialize_vq_chunks(rhs_files);
        }
        let mut pending = std::mem::take(&mut self.vq_chunks);
        let mut scheduler = VqDecodeScheduler::default();
        loop {
            scheduler.dispatch_ready(self, &mut pending, rhs_files, true)?;
            if !scheduler.apply_next(self).await? {
                break;
            }
        }
        if pending.is_empty() {
            Ok(())
        } else {
            Err(vq_chunks_stuck_error(&pending))
        }
    }

    pub(super) fn sprite_row(&self, id: u32) -> Option<&ShippingSprite> {
        self.sprites
            .binary_search_by_key(&id, |(id, _)| *id)
            .ok()
            .map(|position| &self.sprites[position].1)
    }

    /// Expected VQ grid length for a sprite row (tiles are 4x1 pixels).
    pub(super) fn vq_grid_len(sprite: &ShippingSprite) -> usize {
        (sprite.width as usize / 4) * sprite.height as usize
    }

    /// `Ok(true)` when every base grid this chunk needs is materialized.
    /// `Ok(false)` when a base sprite exists but its grid is still pending
    /// (its own chunk decodes later in the fixpoint loop). An entirely
    /// missing or non-VQ base sprite is an error.
    pub(super) fn vq_chunk_bases_ready(&self, chunk: &SpriteVqChunk) -> Result<bool> {
        if chunk.base2_rhs.is_empty() && chunk.base2_ids.iter().any(Option::is_some) {
            return Err(anyhow!(
                "VQ sprite chunk for {} carries base2 sprite ids without a base2 RHS",
                chunk.rhs
            ));
        }
        for (label, base_rhs, ids) in [
            (
                "base",
                chunk.base_rhs.as_deref().unwrap_or("?"),
                &chunk.base_ids,
            ),
            ("base2", chunk.base2_rhs.as_str(), &chunk.base2_ids),
        ] {
            for base_id in ids.iter().flatten() {
                let base = self.sprite_row(*base_id).ok_or_else(|| {
                    anyhow!(
                        "VQ sprite chunk for {} needs {label} sprite {base_id} from {base_rhs}, \
                         which is not part of this mission payload",
                        chunk.rhs
                    )
                })?;
                if base.dictionary_index == UNMAPPED_DICT {
                    return Err(anyhow!(
                        "VQ sprite chunk for {} names {label} sprite {base_id}, which is not \
                         dictionary-coded",
                        chunk.rhs
                    ));
                }
                let expected = Self::vq_grid_len(base);
                match base.packed_data.len() {
                    len if len == expected => {}
                    0 => return Ok(false),
                    len => {
                        return Err(anyhow!(
                            "{label} sprite {base_id} for chunk {} has {len} packed words, \
                             expected {expected}",
                            chunk.rhs
                        ));
                    }
                }
            }
        }
        Ok(true)
    }

    /// Streaming-time readiness: like [`Self::vq_chunk_bases_ready`], but
    /// while mission parts are still arriving nothing is allowed to be a
    /// "missing from the payload" error — a base sprite row, the chunk's own
    /// sprite rows, or its RHS metadata may simply not have been fetched yet,
    /// so all of those report `Ok(false)`. Structural contradictions in rows
    /// that DID arrive (non-VQ base, wrong grid length) still error: rows are
    /// immutable once merged, so waiting longer cannot fix them.
    #[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
    pub(super) fn vq_chunk_ready_lenient(
        &self,
        chunk: &SpriteVqChunk,
        rhs_files: &BTreeMap<String, RhsData>,
    ) -> Result<bool> {
        if chunk.self_refs && !rhs_files.contains_key(&chunk.rhs) {
            return Ok(false);
        }
        for sprite_id in &chunk.sprite_ids {
            match self.sprite_row(*sprite_id) {
                None => return Ok(false),
                Some(sprite) if sprite.dictionary_index == UNMAPPED_DICT => {
                    return Err(anyhow!(
                        "chunk for {} names sprite {sprite_id}, which is not dictionary-coded",
                        chunk.rhs
                    ));
                }
                Some(_) => {}
            }
        }
        if chunk.base2_rhs.is_empty() && chunk.base2_ids.iter().any(Option::is_some) {
            return Err(anyhow!(
                "VQ sprite chunk for {} carries base2 sprite ids without a base2 RHS",
                chunk.rhs
            ));
        }
        for ids in [&chunk.base_ids, &chunk.base2_ids] {
            for base_id in ids.iter().flatten() {
                let Some(base) = self.sprite_row(*base_id) else {
                    return Ok(false);
                };
                if base.dictionary_index == UNMAPPED_DICT {
                    return Err(anyhow!(
                        "VQ sprite chunk for {} names base sprite {base_id}, which is not \
                         dictionary-coded",
                        chunk.rhs
                    ));
                }
                let expected = Self::vq_grid_len(base);
                match base.packed_data.len() {
                    len if len == expected => {}
                    0 => return Ok(false),
                    len => {
                        return Err(anyhow!(
                            "base sprite {base_id} for chunk {} has {len} packed words, \
                             expected {expected}",
                            chunk.rhs
                        ));
                    }
                }
            }
        }
        Ok(true)
    }

    /// Resolve everything one chunk's decode needs from the bank into an
    /// owned, `Send + 'static` bundle, so [`Self::run_vq_chunk_decode`] can
    /// execute on any thread without borrowing `self`. Immutable so
    /// independent chunks of a fixpoint round can prepare/decode in parallel.
    pub(super) fn prepare_vq_chunk_inputs(
        &self,
        chunk: &SpriteVqChunk,
        rhs_files: &BTreeMap<String, RhsData>,
    ) -> Result<VqChunkDecodeInputs> {
        let selfref: Vec<Option<crate::sprite_codec::SelfRef>> = if chunk.self_refs {
            let rhs_data = rhs_files.get(&chunk.rhs).ok_or_else(|| {
                anyhow!(
                    "VQ sprite chunk for {} declares self-references but its RHS metadata is \
                     not part of this mission payload",
                    chunk.rhs
                )
            })?;
            derive_chunk_self_refs(&rhs_data.profiles, &chunk.sprite_ids)
        } else {
            vec![None; chunk.sprite_ids.len()]
        };
        if chunk.base_ids.len() != chunk.sprite_ids.len() {
            return Err(anyhow!(
                "chunk lists {} sprites but {} base entries",
                chunk.sprite_ids.len(),
                chunk.base_ids.len()
            ));
        }
        if !chunk.base2_ids.is_empty() && chunk.base2_ids.len() != chunk.sprite_ids.len() {
            return Err(anyhow!(
                "chunk lists {} sprites but {} base2 entries",
                chunk.sprite_ids.len(),
                chunk.base2_ids.len()
            ));
        }
        let mut dims = Vec::with_capacity(chunk.sprite_ids.len());
        // Cloned `Arc`s keep the base grids alive independently of `self`, so
        // the decoded grids can be written back through `&mut self` below.
        let mut base_grids: Vec<Option<Arc<Vec<u16>>>> = Vec::with_capacity(chunk.base_ids.len());
        let mut base2_grids: Vec<Option<Arc<Vec<u16>>>> = Vec::with_capacity(chunk.base_ids.len());
        for (index, (sprite_id, base_id)) in
            chunk.sprite_ids.iter().zip(&chunk.base_ids).enumerate()
        {
            let sprite = self.sprite_row(*sprite_id).ok_or_else(|| {
                anyhow!("chunk names sprite {sprite_id}, which the payload does not contain")
            })?;
            if sprite.dictionary_index == UNMAPPED_DICT {
                return Err(anyhow!(
                    "chunk names sprite {sprite_id}, which is not dictionary-coded"
                ));
            }
            dims.push((sprite.width / 4, sprite.height));
            // Availability and length were proven by `vq_chunk_bases_ready`.
            let resolve = |base_id: &Option<u32>| {
                base_id.map(|base_id| {
                    Arc::clone(
                        &self
                            .sprite_row(base_id)
                            .expect("base sprite checked by vq_chunk_bases_ready")
                            .packed_data,
                    )
                })
            };
            base_grids.push(resolve(base_id));
            base2_grids.push(resolve(chunk.base2_ids.get(index).unwrap_or(&None)));
        }
        Ok(VqChunkDecodeInputs {
            dims,
            selfref,
            base_grids,
            base2_grids,
        })
    }

    /// Decode one chunk's blob into `(sprite id, grid)` pairs. Pure compute
    /// over the prepared inputs — no `self` access, so a wasm worker thread
    /// can run it against inputs prepared on the main thread.
    pub(super) fn run_vq_chunk_decode(
        chunk: &SpriteVqChunk,
        inputs: &VqChunkDecodeInputs,
    ) -> Result<Vec<(u32, Vec<u16>)>> {
        fn as_slices(grids: &[Option<Arc<Vec<u16>>>]) -> Vec<Option<&[u16]>> {
            grids
                .iter()
                .map(|grid| grid.as_ref().map(|grid| grid.as_slice()))
                .collect()
        }
        let base_slices = as_slices(&inputs.base_grids);
        let base2_slices = as_slices(&inputs.base2_grids);
        let decoded = crate::sprite_codec::decode_grids_shipping(
            chunk.alphabet,
            &inputs.dims,
            Some(&base_slices),
            Some(&base2_slices),
            &inputs.selfref,
            &chunk.blob,
        )?;
        Ok(chunk.sprite_ids.iter().copied().zip(decoded).collect())
    }

    /// Prepare and decode in one step; the serial and rayon fixpoint rounds
    /// run this per chunk.
    pub(super) fn decode_vq_chunk(
        &self,
        chunk: &SpriteVqChunk,
        rhs_files: &BTreeMap<String, RhsData>,
    ) -> Result<Vec<(u32, Vec<u16>)>> {
        let inputs = self.prepare_vq_chunk_inputs(chunk, rhs_files)?;
        Self::run_vq_chunk_decode(chunk, &inputs)
    }

    /// Decode and apply ONE lenient-ready chunk of `pending` on the calling
    /// thread. `Ok(true)` when a chunk was materialized; `Ok(false)` when
    /// nothing in `pending` is ready yet. The serial-fallback streaming
    /// loader calls this repeatedly (yielding to the browser between calls)
    /// to overlap decode with the remaining part downloads.
    #[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
    pub fn materialize_next_ready_vq_chunk(
        &mut self,
        pending: &mut Vec<SpriteVqChunk>,
        rhs_files: &BTreeMap<String, RhsData>,
    ) -> Result<bool> {
        let Some(position) = pending
            .iter()
            .map(|chunk| self.vq_chunk_ready_lenient(chunk, rhs_files))
            .collect::<Result<Vec<bool>>>()?
            .iter()
            .position(|&ready| ready)
        else {
            return Ok(false);
        };
        let chunk = pending.swap_remove(position);
        let grids = self
            .decode_vq_chunk(&chunk, rhs_files)
            .with_context(|| format!("decode VQ sprite chunk for {}", chunk.rhs))?;
        self.apply_decoded_vq_chunk(&chunk, grids)?;
        Ok(true)
    }

    /// Write one chunk's decoded grids into the sprite rows.
    pub fn apply_decoded_vq_chunk(
        &mut self,
        _chunk: &SpriteVqChunk,
        grids: Vec<(u32, Vec<u16>)>,
    ) -> Result<()> {
        #[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
        let apply_start = tracing::enabled!(tracing::Level::DEBUG).then(js_sys::Date::now);
        for (sprite_id, grid) in grids {
            let sprite_id = &sprite_id;
            let position = self
                .sprites
                .binary_search_by_key(sprite_id, |(id, _)| *id)
                .map_err(|_| anyhow!("sprite {sprite_id} disappeared during materialization"))?;
            let sprite = &mut self.sprites[position].1;
            if !sprite.packed_data.is_empty() {
                // The same bank sprite can be listed by two chunks of one
                // closure; both blobs must decode it identically.
                if *sprite.packed_data != grid {
                    return Err(anyhow!(
                        "sprite {sprite_id} decodes differently in two VQ chunks"
                    ));
                }
                continue;
            }
            sprite.packed_data = Arc::new(grid);
        }
        #[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
        if let Some(apply_start_ms) = apply_start {
            tracing::debug!(chunk = %_chunk.rhs, first_sprite = ?_chunk.sprite_ids.first(), apply_start_ms, apply_end_ms = js_sys::Date::now(),
                "vq sprite chunk applied");
        }
        Ok(())
    }

    /// Per-sprite `(width, height)` for one RLE-JXL chunk, validating that
    /// every listed sprite row is present and RLE-coded. Rows always ship in
    /// the same mission part as their chunk, so on a fully merged payload a
    /// missing row is a broken manifest, never a timing question.
    pub(super) fn prepare_rle_jxl_chunk_dims(
        &self,
        chunk: &SpriteRleJxlChunk,
    ) -> Result<Vec<(u16, u16)>> {
        if chunk.placements.len() != chunk.sprite_ids.len() {
            return Err(anyhow!(
                "RLE-JXL chunk for {} lists {} sprites but {} placements",
                chunk.rhs,
                chunk.sprite_ids.len(),
                chunk.placements.len()
            ));
        }
        let mut dims = Vec::with_capacity(chunk.sprite_ids.len());
        for sprite_id in &chunk.sprite_ids {
            let sprite = self.sprite_row(*sprite_id).ok_or_else(|| {
                anyhow!(
                    "RLE-JXL chunk for {} names sprite {sprite_id}, which is not part of this \
                     mission payload",
                    chunk.rhs
                )
            })?;
            if sprite.dictionary_index != UNMAPPED_DICT {
                return Err(anyhow!(
                    "RLE-JXL chunk for {} names sprite {sprite_id}, which is dictionary-coded",
                    chunk.rhs
                ));
            }
            if sprite.width == 0 || sprite.height == 0 {
                return Err(anyhow!(
                    "RLE-JXL chunk for {} names empty sprite {sprite_id}",
                    chunk.rhs
                ));
            }
            dims.push((sprite.width, sprite.height));
        }
        Ok(dims)
    }

    /// Streaming-time readiness: `Ok(false)` while a listed sprite row has
    /// not been merged yet (its part is still downloading).
    #[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
    pub(super) fn rle_jxl_chunk_ready_lenient(&self, chunk: &SpriteRleJxlChunk) -> bool {
        chunk
            .sprite_ids
            .iter()
            .all(|sprite_id| self.sprite_row(*sprite_id).is_some())
    }

    /// Decode one RLE-JXL chunk into `(sprite id, raster window)` pairs.
    /// Pure compute over the prepared dims — no `self` access, so a wasm
    /// worker thread can run it.
    ///
    /// Each atlas becomes ONE shared RGB565 canvas (classes from the
    /// lossless alpha channel, visible color requantized from the lossy
    /// color channels); sprites reference sub-rects of it rather than
    /// copying pixels out. The packed RLE run format is deliberately not
    /// rebuilt — nothing draws from runs, so every consumer would only
    /// decompress them straight back to this raster.
    pub(super) fn run_rle_jxl_chunk_decode(
        chunk: &SpriteRleJxlChunk,
        dims: &[(u16, u16)],
    ) -> Result<Vec<(u32, crate::frame_holder::SpriteRaster)>> {
        Self::run_rle_jxl_chunk_decode_with_parallelism(chunk, dims, true)
    }

    pub(super) fn run_rle_jxl_chunk_decode_with_parallelism(
        chunk: &SpriteRleJxlChunk,
        dims: &[(u16, u16)],
        parallel: bool,
    ) -> Result<Vec<(u32, crate::frame_holder::SpriteRaster)>> {
        use crate::rle_jxl;
        let decode_atlas = |(index, blob): (usize, &Vec<u8>)| {
            let (width, height, rgba) = if parallel {
                rle_jxl::decode_jxl_rgba8_parallel(blob)
            } else {
                rle_jxl::decode_jxl_rgba8(blob)
            }
            .with_context(|| format!("RLE-JXL blob {index} of {}", chunk.rhs))?;
            let canvas = rle_jxl::canvas_from_rgba(&rgba).with_context(|| {
                format!("RLE-JXL blob {index} of {} has invalid classes", chunk.rhs)
            })?;
            Ok((width, height, Arc::new(canvas)))
        };
        // A mission can have only a handful of chunks, with most atlases in
        // one chunk. Let idle workers steal individual atlas decodes too.
        // On wasm, blocking rayon joins are only legal on pool workers.
        #[cfg(not(target_arch = "wasm32"))]
        let use_pool = parallel;
        #[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
        let use_pool = parallel
            && crate::wasm_threads::pool_threads() > 0
            && rayon::current_thread_index().is_some();
        #[cfg(all(target_arch = "wasm32", not(feature = "wasm-threads")))]
        let use_pool = false;
        let atlases: Vec<(usize, usize, Arc<Vec<u16>>)> = if use_pool {
            #[cfg(any(not(target_arch = "wasm32"), feature = "wasm-threads"))]
            {
                use rayon::prelude::*;
                chunk
                    .jxl_blobs
                    .par_iter()
                    .enumerate()
                    .map(decode_atlas)
                    .collect::<Result<_>>()?
            }
            #[cfg(all(target_arch = "wasm32", not(feature = "wasm-threads")))]
            unreachable!("use_pool is false without rayon")
        } else {
            chunk
                .jxl_blobs
                .iter()
                .enumerate()
                .map(decode_atlas)
                .collect::<Result<_>>()?
        };
        let mut out = Vec::with_capacity(chunk.sprite_ids.len());
        for ((&sprite_id, placement), &(width, height)) in chunk
            .sprite_ids
            .iter()
            .zip(&chunk.placements)
            .zip(dims.iter())
        {
            let (atlas_w, atlas_h, canvas) =
                atlases.get(placement.blob as usize).ok_or_else(|| {
                    anyhow!(
                        "RLE-JXL chunk for {} places sprite {sprite_id} in missing blob {}",
                        chunk.rhs,
                        placement.blob
                    )
                })?;
            if placement.x as usize + width as usize > *atlas_w
                || placement.y as usize + height as usize > *atlas_h
            {
                return Err(anyhow!(
                    "RLE-JXL chunk for {} places sprite {sprite_id} ({width}x{height}) at \
                     ({},{}) outside its {atlas_w}x{atlas_h} atlas",
                    chunk.rhs,
                    placement.x,
                    placement.y
                ));
            }
            out.push((
                sprite_id,
                crate::frame_holder::SpriteRaster {
                    atlas: Arc::clone(canvas),
                    stride: *atlas_w as u32,
                    x: placement.x,
                    y: placement.y,
                },
            ));
        }
        Ok(out)
    }

    /// Attach one chunk's decoded raster windows to its sprite rows. The
    /// converter guarantees each bank sprite is JXL-coded by at most one
    /// chunk, so a row that already carries packed words or a raster is a
    /// broken payload rather than something to reconcile.
    pub fn apply_decoded_rle_jxl_chunk(
        &mut self,
        chunk: &SpriteRleJxlChunk,
        rasters: Vec<(u32, crate::frame_holder::SpriteRaster)>,
    ) -> Result<()> {
        #[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
        let apply_start = tracing::enabled!(tracing::Level::DEBUG).then(js_sys::Date::now);
        for (sprite_id, raster) in rasters {
            let position = self
                .sprites
                .binary_search_by_key(&sprite_id, |(id, _)| *id)
                .map_err(|_| {
                    anyhow!("sprite {sprite_id} disappeared during RLE-JXL materialization")
                })?;
            let sprite = &mut self.sprites[position].1;
            if !sprite.packed_data.is_empty() {
                return Err(anyhow!(
                    "sprite {sprite_id} is JXL-coded by {} but also ships packed words",
                    chunk.rhs
                ));
            }
            if sprite.raster.is_some() {
                return Err(anyhow!(
                    "sprite {sprite_id} is JXL-coded by two RLE-JXL chunks (latest {})",
                    chunk.rhs
                ));
            }
            sprite.raster = Some(raster);
        }
        #[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
        if let Some(apply_start_ms) = apply_start {
            tracing::debug!(chunk = %chunk.rhs, first_sprite = ?chunk.sprite_ids.first(), apply_start_ms, apply_end_ms = js_sys::Date::now(),
                "rle_jxl sprite chunk applied");
        }
        Ok(())
    }

    /// Decode every [`SpriteRleJxlChunk`] back into exact-format packed RLE
    /// words, consuming the chunk list. Chunks are mutually independent, so
    /// native builds decode them in parallel; wasm (without the worker pool)
    /// stays serial.
    pub fn materialize_rle_jxl_chunks(&mut self) -> Result<()> {
        self.validate_resident_budget()?;
        let pending = std::mem::take(&mut self.rle_jxl_chunks);
        if pending.is_empty() {
            return Ok(());
        }
        let inputs = pending
            .iter()
            .map(|chunk| self.prepare_rle_jxl_chunk_dims(chunk))
            .collect::<Result<Vec<_>>>()?;
        #[cfg(not(target_arch = "wasm32"))]
        let decoded: Vec<Result<Vec<(u32, crate::frame_holder::SpriteRaster)>>> = {
            use rayon::prelude::*;
            pending
                .par_iter()
                .zip(&inputs)
                .map(|(chunk, dims)| Self::run_rle_jxl_chunk_decode(chunk, dims))
                .collect()
        };
        #[cfg(target_arch = "wasm32")]
        let decoded: Vec<Result<Vec<(u32, crate::frame_holder::SpriteRaster)>>> = pending
            .iter()
            .zip(&inputs)
            .map(|(chunk, dims)| Self::run_rle_jxl_chunk_decode(chunk, dims))
            .collect();
        for (chunk, rasters) in pending.iter().zip(decoded) {
            let rasters = rasters
                .with_context(|| format!("decode RLE-JXL sprite chunk for {}", chunk.rhs))?;
            self.apply_decoded_rle_jxl_chunk(chunk, rasters)?;
        }
        Ok(())
    }
}
