//! Sprite transformation and original RLE/VQ/JXL quality rules.
use super::*;

/// Expected packed word count of a VQ sprite's `(width/4) x height` index
/// grid, or `None` when the dims cannot form one (zero-sized or ragged).
pub(super) fn vq_grid_words(width: u16, height: u16) -> Option<usize> {
    (width > 0 && height > 0 && width.is_multiple_of(4))
        .then(|| (width as usize / 4) * height as usize)
}

/// A sprite's packed words after the dictionary rank permutation (identity
/// when ranking is disabled). `None` when the bank holds no data for it.
pub(super) fn remapped_packed(
    holder: &FrameHolder,
    dict_remaps: Option<&[Vec<u16>]>,
    idx: u32,
) -> Result<Option<Vec<u16>>> {
    let sprite = holder
        .sprites()
        .get(idx as usize)
        .ok_or_else(|| anyhow!("sprite {idx} beyond the shipping bank"))?;
    let Some(packed) = holder.packed_data(idx) else {
        return Ok(None);
    };
    Ok(Some(match dict_remaps {
        Some(remaps) if sprite.dictionary_index != UNMAPPED_DICT => {
            let remap = remaps
                .get(sprite.dictionary_index as usize)
                .ok_or_else(|| {
                    anyhow!(
                        "sprite {idx} references dictionary {} without a rank remap",
                        sprite.dictionary_index
                    )
                })?;
            packed
                .iter()
                .map(|&i| {
                    remap.get(i as usize).copied().ok_or_else(|| {
                        anyhow!(
                            "sprite {idx} index {i} out of range for dictionary {}",
                            sprite.dictionary_index
                        )
                    })
                })
                .collect::<Result<Vec<u16>>>()?
        }
        _ => packed.to_vec(),
    }))
}

/// The RLE-bucket files eligible for `--rle-sprite-format jxl-*`: the
/// content class where lossy JXL was measured to WIN (docs/COMPRESSION.md,
/// 2026-08-29 "the RLE/patch bucket"). RLE sprites in other RHS chunks
/// (e.g. stray RLE frames inside character banks) keep exact words — they
/// were not part of the measured/visually-vetted corpus.
pub(super) fn is_rle_jxl_bucket_rel(rel: &str) -> bool {
    let lower = rel.replace('\\', "/").to_ascii_lowercase();
    if lower.starts_with("animations/") {
        return true;
    }
    lower.strip_prefix("characters/").is_some_and(|name| {
        ["accessories_", "bonus_", "relic_", "tg_"]
            .iter()
            .any(|prefix| name.starts_with(prefix))
    })
}

/// Sprites below this dimension keep exact RLE: the per-image JXL header
/// tax dominates and the worst visual outliers live here (the probe's
/// `--min-dim` selection).
const RLE_JXL_MIN_DIM: u16 = 20;
/// Animation groups with at least this many eligible frames get packed
/// into grid atlases (cjxl's patch/context machinery recovers ~20% over
/// per-sprite encodes on the measured corpus).
const RLE_JXL_MIN_ATLAS_FRAMES: usize = 4;
/// Cap on a single atlas image, splitting big groups into sub-atlases so
/// one worker's transient RGBA decode buffer stays bounded on wasm.
const RLE_JXL_MAX_ATLAS_PIXELS: usize = 4 << 20;
/// Conversion-time per-sprite quality floor. A sprite whose opaque-pixel
/// PSNR (scored over the exact requantized RGB565 values materialization
/// ships) falls below this keeps its exact RLE words instead — the worst
/// q70 outliers are tiny dithered pickup/effect sprites that contribute
/// almost no bytes (docs/COMPRESSION.md). Keeping every sprite at or above
/// this floor also guarantees the per-chunk aggregate floor that
/// `sprite_compression_probe --verify-shipping` enforces.
const RLE_JXL_MIN_SPRITE_PSNR_DB: f64 = 24.0;

#[derive(Debug, Default, Clone, Copy)]
pub(super) struct RleJxlChunkStats {
    pub(super) atlased: usize,
    pub(super) individual: usize,
    pub(super) kept_smaller: usize,
    pub(super) kept_small_dim: usize,
    pub(super) kept_shared: usize,
    pub(super) kept_irregular: usize,
    pub(super) kept_low_psnr: usize,
    pub(super) jxl_bytes: u64,
    pub(super) raw_words_replaced: u64,
    /// Pixels of the decoded atlases these chunks keep resident (2 B each),
    /// gutters included.
    pub(super) atlas_pixels: u64,
    /// Pixels the sprites themselves occupy — the difference from
    /// `atlas_pixels` is atlas gutter waste.
    pub(super) sprite_canvas_pixels: u64,
}

impl RleJxlChunkStats {
    pub(super) fn add(&mut self, other: &RleJxlChunkStats) {
        self.atlased += other.atlased;
        self.individual += other.individual;
        self.kept_smaller += other.kept_smaller;
        self.kept_small_dim += other.kept_small_dim;
        self.kept_shared += other.kept_shared;
        self.kept_irregular += other.kept_irregular;
        self.kept_low_psnr += other.kept_low_psnr;
        self.jxl_bytes += other.jxl_bytes;
        self.raw_words_replaced += other.raw_words_replaced;
        self.atlas_pixels += other.atlas_pixels;
        self.sprite_canvas_pixels += other.sprite_canvas_pixels;
    }
    pub(super) fn lossy(&self) -> usize {
        self.atlased + self.individual
    }
}

/// Check one encoded member region against its source canvas: the CLASS of
/// every pixel must survive exactly (that is the whole contract the alpha
/// channel carries — a single reclassified edge pixel is a visible
/// artifact), and the visible pixels are scored for PSNR over the exact
/// RGB565 values materialization will ship. Errors, rather than a low
/// score, on any class mismatch.
pub(super) fn member_quality(
    candidate: &RleJxlCandidate,
    rgba: &[u8],
    rgba_width: usize,
    x0: usize,
    y0: usize,
) -> Result<f64> {
    use robin_assets::rle_jxl::{self, CL_OPAQUE};
    anyhow::ensure!(
        candidate.pixels.len() == usize::from(candidate.width) * usize::from(candidate.height),
        "sprite {} source canvas dimensions do not match its pixels",
        candidate.id
    );
    let row_bytes = rgba_width
        .checked_mul(4)
        .filter(|&bytes| bytes != 0)
        .with_context(|| {
            format!(
                "sprite {} has an invalid decoded canvas width",
                candidate.id
            )
        })?;
    anyhow::ensure!(
        rgba.len().is_multiple_of(row_bytes),
        "sprite {} decoded canvas contains an incomplete RGBA row",
        candidate.id
    );
    let rgba_height = rgba.len() / row_bytes;
    anyhow::ensure!(
        x0.checked_add(usize::from(candidate.width))
            .is_some_and(|end| end <= rgba_width)
            && y0
                .checked_add(usize::from(candidate.height))
                .is_some_and(|end| end <= rgba_height),
        "sprite {} member region lies outside the decoded canvas",
        candidate.id
    );
    let (mut sse, mut opaque) = (0.0f64, 0u64);
    for y in 0..candidate.height as usize {
        for x in 0..candidate.width as usize {
            let i = y * candidate.width as usize + x;
            let source = candidate.pixels[i];
            let offset = ((y0 + y) * rgba_width + x0 + x) * 4;
            let class = rle_jxl::alpha_to_class(rgba[offset + 3])
                .with_context(|| format!("sprite {} pixel ({x},{y})", candidate.id))?;
            if class != rle_jxl::class_of(source) {
                bail!(
                    "sprite {} pixel ({x},{y}) came back in class {class} instead of {} — \
                     cjxl did not code the alpha channel losslessly",
                    candidate.id,
                    rle_jxl::class_of(source),
                );
            }
            if class != CL_OPAQUE {
                continue;
            }
            let shipped = rle_jxl::dodge_keys(rle_jxl::quant565(
                rgba[offset],
                rgba[offset + 1],
                rgba[offset + 2],
            ));
            let a = rle_jxl::expand565(source);
            let b = rle_jxl::expand565(shipped);
            for channel in 0..3 {
                let d = a[channel] as f64 - b[channel] as f64;
                sse += d * d;
            }
            opaque += 1;
        }
    }
    if opaque == 0 || sse == 0.0 {
        return Ok(f64::INFINITY);
    }
    let mse = sse / (opaque as f64 * 3.0);
    Ok(10.0 * (255.0f64 * 255.0 / mse).log10())
}

/// One eligible RLE sprite expanded for the JXL path.
pub(super) struct RleJxlCandidate {
    pub(super) id: u32,
    pub(super) width: u16,
    pub(super) height: u16,
    /// Decoded canvas (RGB565 with the key values in place) — exactly what
    /// materialization must reproduce.
    pub(super) pixels: Vec<u16>,
    /// Exact packed words (for the keep-exact size comparison).
    pub(super) raw_words: usize,
}

impl RleJxlCandidate {
    /// Encoder input for this sprite alone: class-carrying alpha plus
    /// edge-extended color.
    pub(super) fn smeared_rgba(&self) -> Result<Vec<u8>> {
        use robin_assets::rle_jxl;
        let mut rgba = rle_jxl::canvas_to_rgba(&self.pixels)?;
        rle_jxl::smear_invisible_rgb(&mut rgba, self.width as usize, self.height as usize);
        Ok(rgba)
    }

    pub(super) fn raw_le_bytes<'a>(
        &self,
        sprites: &'a [(u32, ShippingSprite)],
    ) -> impl Iterator<Item = u8> + 'a {
        let position = sprites
            .binary_search_by_key(&self.id, |(id, _)| *id)
            .expect("candidate came from these rows");
        sprites[position]
            .1
            .packed_data
            .iter()
            .flat_map(|w| w.to_le_bytes())
    }
}

/// An accepted lossy sprite waiting for chunk assembly.
pub(super) struct RleJxlAccepted {
    pub(super) id: u32,
    pub(super) blob: u32,
    pub(super) x: u16,
    pub(super) y: u16,
    pub(super) atlased: bool,
    pub(super) raw_words: usize,
    /// Pixels this sprite occupies in its atlas (resident-cost accounting).
    pub(super) canvas_pixels: u64,
}

/// Compressed-size estimate for the keep-exact side of the per-sprite
/// decision: the raw words as they would sit in the outer chunk zstd.
pub(super) fn zstd19_len(bytes: &[u8]) -> Result<usize> {
    let mut output = CompressedByteCount::default();
    zstd::stream::copy_encode(bytes, &mut output, 19).context("zstd19 size estimate")?;
    Ok(output.bytes)
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct CompressedByteCount {
    bytes: usize,
}

impl std::io::Write for CompressedByteCount {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .ok_or_else(|| std::io::Error::other("compressed byte count exceeds usize"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Build the lossy-JXL payload for one RHS chunk's eligible RLE sprites and
/// blank the rows it covers. See `--rle-sprite-format` and the schema v13
/// comment in `shipping_datadir.rs`.
pub(super) fn build_rle_jxl_chunk(
    rel: &str,
    prep: &RhsChunkPrep,
    sprites: &mut [(u32, ShippingSprite)],
    quality: u8,
    multi_chunk_ids: &std::collections::HashSet<u32>,
) -> Result<(Option<SpriteRleJxlChunk>, RleJxlChunkStats)> {
    use robin_assets::rle_jxl;

    let mut stats = RleJxlChunkStats::default();
    let Some(rhs_data) = &prep.rhs_data else {
        return Ok((None, stats));
    };
    // Expand every eligible candidate.
    let mut candidates: Vec<RleJxlCandidate> = Vec::new();
    for (id, sprite) in sprites.iter() {
        if sprite.dictionary_index != UNMAPPED_DICT
            || sprite.packed_data.is_empty()
            || sprite.width == 0
            || sprite.height == 0
        {
            continue;
        }
        if multi_chunk_ids.contains(id) {
            // Two chunks may not lossy-code one bank slot independently
            // (their decodes would conflict at mission merge), so shared
            // sprites keep the exact words in every chunk that ships them.
            stats.kept_shared += 1;
            continue;
        }
        let (pixels, used) = match rle_jxl::decode_rle_canvas(
            sprite.width as usize,
            sprite.height as usize,
            &sprite.packed_data,
        ) {
            Ok(decoded) => decoded,
            Err(error) => {
                tracing::warn!(rhs = rel, sprite = id, %error, "RLE sprite does not walk; keeping exact words");
                stats.kept_irregular += 1;
                continue;
            }
        };
        if used != sprite.packed_data.len() {
            // Trailing words beyond the row walk cannot round-trip through
            // a canvas (none measured in the bucket, but never guess).
            stats.kept_irregular += 1;
            continue;
        }
        if sprite.width < RLE_JXL_MIN_DIM || sprite.height < RLE_JXL_MIN_DIM {
            stats.kept_small_dim += 1;
            continue;
        }
        candidates.push(RleJxlCandidate {
            id: *id,
            width: sprite.width,
            height: sprite.height,
            pixels,
            raw_words: sprite.packed_data.len(),
        });
    }
    if candidates.is_empty() {
        return Ok((None, stats));
    }
    let by_id: std::collections::HashMap<u32, usize> = candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| (candidate.id, index))
        .collect();

    // First-claim (profile, action) animation groups over the shipped
    // scripts — the probe's grouping rule.
    let mut claimed = std::collections::HashSet::<u32>::new();
    let mut groups: Vec<Vec<u32>> = Vec::new();
    for (_name, info) in &rhs_data.profiles {
        let mut by_action = std::collections::BTreeMap::<u16, Vec<u32>>::new();
        for script in info.scripts.iter() {
            let entry = by_action.entry(script.action_id).or_default();
            for &frame_id in &script.frame_ids {
                if by_id.contains_key(&frame_id) && claimed.insert(frame_id) {
                    entry.push(frame_id);
                }
            }
        }
        groups.extend(by_action.into_values().filter(|ids| !ids.is_empty()));
    }
    // Script-order claiming covers every candidate (chunk rows come from
    // script frame ids), but stay defensive about synthesized extras.
    let mut ungrouped: Vec<u32> = candidates
        .iter()
        .map(|candidate| candidate.id)
        .filter(|id| !claimed.contains(id))
        .collect();

    let mut blobs: Vec<Vec<u8>> = Vec::new();
    let mut accepted: Vec<RleJxlAccepted> = Vec::new();
    for mut members in groups {
        if members.len() < RLE_JXL_MIN_ATLAS_FRAMES {
            ungrouped.extend(members);
            continue;
        }
        // Quality-gated atlas encode: members scoring below the PSNR floor
        // are ejected and the remainder re-packed, so one bad dithered
        // frame cannot drag a whole chunk under the verification floor.
        // Ejected members retry individually below (a dedicated encode may
        // still clear the floor); each round removes at least one member.
        let accepted_group = loop {
            if members.len() < RLE_JXL_MIN_ATLAS_FRAMES {
                break None;
            }
            // Split into sub-atlases under the pixel cap. Cell dims are
            // the sub-atlas max, so splitting also tightens cells when a
            // group mixes frame sizes.
            let mut sub_atlases: Vec<(&[u32], usize, usize)> = Vec::new();
            let mut start = 0usize;
            while start < members.len() {
                let mut end = start;
                let (mut cell_w, mut cell_h) = (0usize, 0usize);
                while end < members.len() {
                    let candidate = &candidates[by_id[&members[end]]];
                    let w = cell_w.max(candidate.width as usize);
                    let h = cell_h.max(candidate.height as usize);
                    let count = end - start + 1;
                    let cols = (count as f64).sqrt().ceil() as usize;
                    let rows = count.div_ceil(cols);
                    if end > start && cols * w * rows * h > RLE_JXL_MAX_ATLAS_PIXELS {
                        break;
                    }
                    (cell_w, cell_h) = (w, h);
                    end += 1;
                }
                sub_atlases.push((&members[start..end], cell_w, cell_h));
                start = end;
            }
            let mut group_jxl: Vec<(Vec<u8>, Vec<(u32, u16, u16)>)> = Vec::new();
            let mut group_jxl_bytes = 0usize;
            let mut low_psnr: Vec<u32> = Vec::new();
            for (sub, cell_w, cell_h) in sub_atlases {
                let cols = (sub.len() as f64).sqrt().ceil() as usize;
                let rows = sub.len().div_ceil(cols);
                let (atlas_w, atlas_h) = (cols * cell_w, rows * cell_h);
                // Gutters start fully transparent, so the edge extension
                // below flows sprite color across cell boundaries too.
                let mut rgba = vec![0u8; atlas_w * atlas_h * 4];
                let mut placements = Vec::with_capacity(sub.len());
                for (k, id) in sub.iter().enumerate() {
                    let frame = &candidates[by_id[id]];
                    let (x0, y0) = ((k % cols) * cell_w, (k / cols) * cell_h);
                    let src = rle_jxl::canvas_to_rgba(&frame.pixels)?;
                    for y in 0..frame.height as usize {
                        let dst = ((y0 + y) * atlas_w + x0) * 4;
                        let n = frame.width as usize * 4;
                        rgba[dst..dst + n].copy_from_slice(&src[y * n..(y + 1) * n]);
                    }
                    placements.push((frame.id, x0 as u16, y0 as u16));
                }
                rle_jxl::smear_invisible_rgb(&mut rgba, atlas_w, atlas_h);
                let jxl = encode_pixels_to_jxl(
                    atlas_w as u32,
                    atlas_h as u32,
                    &rgba,
                    png::ColorType::Rgba,
                    Some(quality),
                    7,
                )
                .with_context(|| format!("cjxl atlas for {rel}"))?;
                let (dec_w, _dec_h, decoded) = rle_jxl::decode_jxl_rgba8(&jxl)
                    .with_context(|| format!("decode encoded atlas for {rel}"))?;
                for &(id, x0, y0) in &placements {
                    let candidate = &candidates[by_id[&id]];
                    // Class mismatches error out of the whole conversion —
                    // only the quality score demotes a member.
                    if member_quality(candidate, &decoded, dec_w, x0 as usize, y0 as usize)?
                        < RLE_JXL_MIN_SPRITE_PSNR_DB
                    {
                        low_psnr.push(id);
                    }
                }
                group_jxl_bytes += jxl.len();
                group_jxl.push((jxl, placements));
            }
            if !low_psnr.is_empty() {
                members.retain(|id| !low_psnr.contains(id));
                ungrouped.extend(low_psnr);
                continue;
            }
            // Size gate: the atlas must beat the outer-zstd cost of the
            // exact words (tiny/flat groups lose; per-sprite decides then).
            let mut raw_le = Vec::new();
            for id in &members {
                raw_le.extend(candidates[by_id[id]].raw_le_bytes(sprites));
            }
            if group_jxl_bytes >= zstd19_len(&raw_le)? {
                break None;
            }
            break Some(group_jxl);
        };
        match accepted_group {
            Some(group_jxl) => {
                for (jxl, placements) in group_jxl {
                    let blob = blobs.len() as u32;
                    blobs.push(jxl);
                    for (id, x, y) in placements {
                        let candidate = &candidates[by_id[&id]];
                        accepted.push(RleJxlAccepted {
                            id,
                            blob,
                            x,
                            y,
                            atlased: true,
                            raw_words: candidate.raw_words,
                            canvas_pixels: candidate.pixels.len() as u64,
                        });
                    }
                }
            }
            None => ungrouped.extend(members),
        }
    }
    // Ungrouped / demoted sprites: individual JXL, kept only when it clears
    // the PSNR floor and beats the outer-zstd cost of the exact words.
    for id in ungrouped {
        let candidate = &candidates[by_id[&id]];
        let jxl = encode_pixels_to_jxl(
            candidate.width as u32,
            candidate.height as u32,
            &candidate.smeared_rgba()?,
            png::ColorType::Rgba,
            Some(quality),
            7,
        )
        .with_context(|| format!("cjxl sprite {id} of {rel}"))?;
        let (dec_w, _dec_h, decoded) = rle_jxl::decode_jxl_rgba8(&jxl)
            .with_context(|| format!("decode encoded sprite {id} of {rel}"))?;
        if member_quality(candidate, &decoded, dec_w, 0, 0)? < RLE_JXL_MIN_SPRITE_PSNR_DB {
            stats.kept_low_psnr += 1;
            continue;
        }
        let raw_le: Vec<_> = candidate.raw_le_bytes(sprites).collect();
        if jxl.len() >= zstd19_len(&raw_le)? {
            stats.kept_smaller += 1;
            continue;
        }
        let blob = blobs.len() as u32;
        blobs.push(jxl);
        accepted.push(RleJxlAccepted {
            id,
            blob,
            x: 0,
            y: 0,
            atlased: false,
            raw_words: candidate.raw_words,
            canvas_pixels: candidate.pixels.len() as u64,
        });
    }
    if accepted.is_empty() {
        return Ok((None, stats));
    }
    accepted.sort_by_key(|entry| entry.id);
    let mut chunk = SpriteRleJxlChunk {
        rhs: rel.to_owned(),
        jxl_blobs: blobs,
        sprite_ids: Vec::with_capacity(accepted.len()),
        placements: Vec::with_capacity(accepted.len()),
    };
    for entry in &accepted {
        chunk.sprite_ids.push(entry.id);
        chunk.placements.push(RleJxlPlacement {
            blob: entry.blob,
            x: entry.x,
            y: entry.y,
        });
        if entry.atlased {
            stats.atlased += 1;
        } else {
            stats.individual += 1;
        }
        stats.raw_words_replaced += entry.raw_words as u64;
        stats.sprite_canvas_pixels += entry.canvas_pixels;
        // Blank the row: at mission install the JXL decodes to a shared
        // raster and the row points into it, exactly like VQ grids come
        // out of their blob.
        let position = sprites
            .binary_search_by_key(&entry.id, |(id, _)| *id)
            .expect("accepted sprite came from these rows");
        sprites[position].1.packed_data = Arc::new(Vec::new());
    }
    stats.jxl_bytes = chunk.jxl_blobs.iter().map(|blob| blob.len() as u64).sum();
    // Resident cost of this chunk once decoded: the whole atlas raster at
    // 2 B/px, gutters included (see the memory note in the ledger).
    for blob in &chunk.jxl_blobs {
        let (width, height, _rgba) = rle_jxl::decode_jxl_rgba8(blob)
            .with_context(|| format!("measure decoded atlas of {rel}"))?;
        stats.atlas_pixels += (width * height) as u64;
    }
    Ok((Some(chunk), stats))
}

/// Assemble one shared RHS chunk: sprite rows for every reachable bank slot,
/// with all well-formed VQ grids coded into a single `sprite_codec` blob
/// (cross-variant against `prep.base_ids` where present) and RLE/ragged
/// sprites keeping raw packed words.
pub(super) fn build_rhs_chunk_payload(
    holder: &FrameHolder,
    dict_remaps: Option<&[Vec<u16>]>,
    rel: &str,
    prep: RhsChunkPrep,
    rle_format: RleSpriteFormat,
    vq_group_tiles: usize,
    rle_group_blobs: usize,
    multi_chunk_ids: &std::collections::HashSet<u32>,
) -> Result<(ShippingMission, RleJxlChunkStats)> {
    let mut payload = ShippingMission::default();
    let mut sprites = Vec::with_capacity(prep.used_sprite_ids.len());
    let mut blob_ids = Vec::new();
    let mut blob_dims = Vec::new();
    let mut blob_grids: Vec<Vec<u16>> = Vec::new();
    let mut blob_bases: Vec<Option<Vec<u16>>> = Vec::new();
    let mut blob_base_ids: Vec<Option<u32>> = Vec::new();
    let mut blob_base2s: Vec<Option<Vec<u16>>> = Vec::new();
    let mut blob_base2_ids: Vec<Option<u32>> = Vec::new();
    let mut alphabet: u16 = 0;
    for &idx in &prep.used_sprite_ids {
        let sprite = holder
            .sprites()
            .get(idx as usize)
            .ok_or_else(|| anyhow!("RHS {rel} references sprite {idx} beyond the shipping bank"))?;
        let row_packed = match remapped_packed(holder, dict_remaps, idx)? {
            Some(packed)
                if sprite.dictionary_index != UNMAPPED_DICT
                    && vq_grid_words(sprite.width, sprite.height) == Some(packed.len()) =>
            {
                let dict = holder.dictionary(sprite.dictionary_index).ok_or_else(|| {
                    anyhow!(
                        "sprite {idx} references missing dictionary {}",
                        sprite.dictionary_index
                    )
                })?;
                alphabet = alphabet.max(dict.num_entries());
                match prep.base_ids.get(&idx) {
                    Some(&base_id) => {
                        let base =
                            remapped_packed(holder, dict_remaps, base_id)?.ok_or_else(|| {
                                anyhow!(
                                    "family base sprite {base_id} for RHS {rel} has no packed data"
                                )
                            })?;
                        blob_bases.push(Some(base));
                        blob_base_ids.push(Some(base_id));
                    }
                    None => {
                        blob_bases.push(None);
                        blob_base_ids.push(None);
                    }
                }
                match prep.base2_ids.get(&idx) {
                    Some(&base2_id) => {
                        if !prep.base_ids.contains_key(&idx) {
                            bail!(
                                "sprite {idx} of RHS {rel} plans a base2 predecessor without a base"
                            );
                        }
                        let base2 =
                            remapped_packed(holder, dict_remaps, base2_id)?.ok_or_else(|| {
                                anyhow!(
                                    "family base2 sprite {base2_id} for RHS {rel} has no packed \
                                     data"
                                )
                            })?;
                        blob_base2s.push(Some(base2));
                        blob_base2_ids.push(Some(base2_id));
                    }
                    None => {
                        blob_base2s.push(None);
                        blob_base2_ids.push(None);
                    }
                }
                blob_ids.push(idx);
                blob_dims.push((sprite.width / 4, sprite.height));
                blob_grids.push(packed);
                Vec::new()
            }
            Some(packed) => {
                if sprite.dictionary_index != UNMAPPED_DICT {
                    // VQ words that disagree with the sprite's grid shape
                    // cannot ride the codec blob; ship them raw rather than
                    // guessing at dimensions.
                    tracing::warn!(
                        rhs = rel,
                        sprite = idx,
                        words = packed.len(),
                        width = sprite.width,
                        height = sprite.height,
                        "VQ sprite length does not match its grid; keeping raw indices"
                    );
                }
                packed
            }
            None if sprite.width == 0 || sprite.height == 0 => Vec::new(),
            None => bail!("RHS {rel} references non-empty sprite {idx} with no packed bank data"),
        };
        sprites.push((
            idx,
            ShippingSprite {
                width: sprite.width,
                height: sprite.height,
                dictionary_index: sprite.dictionary_index,
                packed_data: Arc::new(row_packed),
                raster: None,
            },
        ));
    }
    // Web recipe: swap eligible RLE bucket sprites to lossy JXL atlases,
    // blanking the rows the chunk covers.
    let (rle_jxl_chunk, rle_stats) = match rle_format.jxl_quality() {
        Some(quality) if is_rle_jxl_bucket_rel(rel) => {
            build_rle_jxl_chunk(rel, &prep, &mut sprites, quality, multi_chunk_ids)?
        }
        _ => (None, RleJxlChunkStats::default()),
    };

    let coded_sprites = blob_ids.len();
    let mut vq_chunks = Vec::new();
    let mut blob_bytes = 0usize;
    if !blob_ids.is_empty() {
        let grids: Vec<robin_assets::sprite_codec::SpriteGrid> = blob_dims
            .iter()
            .zip(&blob_grids)
            .map(
                |(&(cols, rows), grid)| robin_assets::sprite_codec::SpriteGrid {
                    cols,
                    rows,
                    indices: grid,
                },
            )
            .collect();
        let bases: Vec<Option<&[u16]>> = blob_bases.iter().map(|base| base.as_deref()).collect();
        let base2s: Vec<Option<&[u16]>> = blob_base2s.iter().map(|base| base.as_deref()).collect();
        let has_base2 = blob_base2_ids.iter().any(Option::is_some);
        // Standalone chunks gain within-chunk self-references (temporal /
        // adjacent-direction), derived from the SHIPPED profile set — the
        // decoder re-derives the identical map from the chunk's RhsData, so
        // the rule must run over exactly what ships.
        let has_self_refs = match (&prep.rhs_data, &prep.base_rel) {
            (Some(rhs_data), None) => robin_assets::shipping_datadir::derive_chunk_self_refs(
                &rhs_data.profiles,
                &blob_ids,
            )
            .iter()
            .any(Option::is_some),
            _ => false,
        };
        if has_base2 && prep.base2_rel.is_none() {
            bail!("RHS {rel} coded base2 sprites without a planned base2 chunk");
        }
        let template = SpriteVqChunk {
            rhs: rel.to_owned(),
            base_rhs: prep.base_rel.clone(),
            base2_rhs: if has_base2 {
                prep.base2_rel.clone().unwrap_or_default()
            } else {
                String::new()
            },
            alphabet,
            sprite_ids: blob_ids,
            base_ids: blob_base_ids,
            base2_ids: if has_base2 {
                blob_base2_ids
            } else {
                Vec::new()
            },
            self_refs: has_self_refs,
            blob: Vec::new(),
        };
        vq_chunks = robin_assets::sprite_groups::encode_vq_groups(
            &template,
            &grids,
            &bases,
            &base2s,
            prep.rhs_data.as_ref(),
            vq_group_tiles,
        )
        .with_context(|| format!("encode VQ groups for {rel}"))?;
        blob_bytes = vq_chunks.iter().map(|chunk| chunk.blob.len()).sum();
    }
    payload.sprite_bank = Some(ShippingSpriteBank {
        signature: holder.signature(),
        dictionaries: Vec::new(),
        sprite_count: holder.sprites().len() as u32,
        sprites,
        vq_chunks,
        rle_jxl_chunks: match rle_jxl_chunk {
            Some(chunk) => {
                robin_assets::sprite_groups::split_rle_jxl_chunk(chunk, rle_group_blobs)?
            }
            None => Vec::new(),
        },
    });
    tracing::info!(
        rhs = rel,
        sprites = prep.used_sprite_ids.len(),
        required_rhs_profiles = prep.matched_profiles,
        vq_sprites = coded_sprites,
        vq_blob_bytes = blob_bytes,
        base = prep.base_rel.as_deref().unwrap_or(""),
        base2 = prep.base2_rel.as_deref().unwrap_or(""),
        rle_jxl_sprites = rle_stats.lossy(),
        rle_jxl_bytes = rle_stats.jxl_bytes,
        "built shared RHS sprite payload"
    );
    if let Some(rhs_data) = prep.rhs_data {
        payload.rhs_files.insert(rel.to_owned(), rhs_data);
    }
    Ok((payload, rle_stats))
}

#[cfg(test)]
mod ownership_tests {
    use super::*;

    #[test]
    fn member_quality_checks_canvas_shape_and_region_before_indexing() {
        let candidate = RleJxlCandidate {
            id: 7,
            width: 2,
            height: 1,
            pixels: vec![0xf800, 0x001f],
            raw_words: 2,
        };
        let rgba = robin_assets::rle_jxl::canvas_to_rgba(&candidate.pixels).unwrap();
        assert_eq!(
            member_quality(&candidate, &rgba, 2, 0, 0).unwrap(),
            f64::INFINITY
        );
        let mut atlas = vec![0; 4 * 4 * 3];
        atlas[20..28].copy_from_slice(&rgba);
        assert_eq!(
            member_quality(&candidate, &atlas, 4, 1, 1).unwrap(),
            f64::INFINITY
        );
        atlas[23] = 0;
        assert!(
            member_quality(&candidate, &atlas, 4, 1, 1)
                .unwrap_err()
                .to_string()
                .contains("class")
        );
        for (bytes, width, x, y) in [
            (rgba.as_slice(), 0, 0, 0),
            (rgba.as_slice(), usize::MAX, 0, 0),
            (&rgba[..7], 2, 0, 0),
            (rgba.as_slice(), 2, 1, 0),
            (rgba.as_slice(), 2, 0, 1),
            (rgba.as_slice(), 2, usize::MAX, 0),
            (rgba.as_slice(), 2, 0, usize::MAX),
        ] {
            assert!(member_quality(&candidate, bytes, width, x, y).is_err());
        }
        let mut wrong_source = candidate;
        wrong_source.pixels.pop();
        assert!(member_quality(&wrong_source, &rgba, 2, 0, 0).is_err());
    }

    #[test]
    fn compressed_size_matches_materialized_output() {
        for length in [0, 1, 8191, 8192, 8193, 131_072, 300_000] {
            let mut state = 0x1234_5678_u32;
            let mixed: Vec<_> = (0..length)
                .map(|_| {
                    state ^= state << 13;
                    state ^= state >> 17;
                    state ^= state << 5;
                    state as u8
                })
                .collect();
            for bytes in [vec![0; length], mixed] {
                assert_eq!(
                    zstd19_len(&bytes).unwrap(),
                    zstd::stream::encode_all(bytes.as_slice(), 19)
                        .unwrap()
                        .len(),
                    "input length {length}"
                );
            }
        }
    }

    #[test]
    fn compressed_byte_counter_reports_overflow_without_wrapping() {
        use std::io::Write as _;
        let mut counter = CompressedByteCount {
            bytes: usize::MAX - 1,
        };
        assert_eq!(counter.write(&[1]).unwrap(), 1);
        assert!(counter.write(&[2]).is_err());
        assert_eq!(counter.bytes, usize::MAX);
        assert_eq!(counter.write(&[]).unwrap(), 0);
    }

    #[test]
    fn raw_word_iteration_preserves_all_little_endian_values() {
        let sprites = [(
            7,
            ShippingSprite {
                width: 0,
                height: 0,
                dictionary_index: UNMAPPED_DICT,
                packed_data: Arc::new((0..=u16::MAX).collect()),
                raster: None,
            },
        )];
        let candidate = RleJxlCandidate {
            id: 7,
            width: 0,
            height: 0,
            pixels: Vec::new(),
            raw_words: sprites[0].1.packed_data.len(),
        };
        let mut bytes = vec![0xab, 0xcd];
        bytes.extend(candidate.raw_le_bytes(&sprites));
        assert_eq!(&bytes[..2], &[0xab, 0xcd]);
        assert_eq!(bytes.len(), 2 + 2 * candidate.raw_words);
        for (word, encoded) in (0..=u16::MAX).zip(bytes[2..].chunks_exact(2)) {
            assert_eq!(encoded, &[word as u8, (word >> 8) as u8]);
        }
    }

    #[test]
    fn payload_takes_ownership_of_prepared_profile_storage() {
        let profiles = Vec::with_capacity(4);
        let storage = profiles.as_ptr();
        let capacity = profiles.capacity();
        let prep = RhsChunkPrep {
            rhs_data: Some(RhsData {
                signature: 123,
                profiles,
            }),
            matched_profiles: 0,
            script_order: Vec::new(),
            used_sprite_ids: BTreeSet::new(),
            base_rel: None,
            base_ids: Default::default(),
            base2_rel: None,
            base2_ids: Default::default(),
        };
        let (payload, _) = build_rhs_chunk_payload(
            &FrameHolder::new(),
            None,
            "Characters/Hero.rhs",
            prep,
            RleSpriteFormat::Exact,
            1,
            1,
            &Default::default(),
        )
        .unwrap();
        let rhs = &payload.rhs_files["Characters/Hero.rhs"];
        assert_eq!(rhs.signature, 123);
        assert!(rhs.profiles.is_empty());
        assert_eq!(rhs.profiles.as_ptr(), storage);
        assert_eq!(rhs.profiles.capacity(), capacity);
        assert!(payload.sprite_bank.as_ref().unwrap().sprites.is_empty());
    }
}
