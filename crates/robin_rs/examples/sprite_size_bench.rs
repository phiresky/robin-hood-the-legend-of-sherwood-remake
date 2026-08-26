//! Benchmark image/sprite compression formats for Robin Hood data.
//!
//!   cargo run --release --example sprite_size_bench -- --data-dir datadirs/demo_leicester_ecoste
//!
//! Measures on-disk size for:
//!   • Background maps (`Data/Levels/*/<name>.map` — SBPictureSixteen, bzip2 RGB565)
//!   • Character sprite-sheets (frames from `.rhs` + `robinhood.bks/.dic`)
//!
//! Formats tried (all lossless so comparisons are apples-to-apples):
//!   - PNG          (png crate → oxipng post-process)
//!   - Lossless JXL (cjxl -d 0 -e 9)
//!   - Lossless AVIF (avifenc --lossless)
//!   - QOI          (qoi crate)
//!   - RGB565 + zstd L22 (keeps native 16-bit, no channel expansion)
//!   - RGBA8  + zstd L22
//!   - Animated JXL (character anims only) — one file containing all frames
//!   - AV1 lossless via ffmpeg (character anims only)
//!   - Original bytes (bank-packed + rhs header, for reference)
#![allow(clippy::print_stdout, clippy::print_stderr, clippy::too_many_arguments)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;

use robin_assets::frame_holder::{FrameHolder, TRANSPARENT_COLOR_16};
use robin_assets::picture::Picture;
use robin_engine::sbfile::{self, SbFile};
use robin_engine::sprite_script::SpriteScriptor;
use robin_engine::sprite_variant::SpriteVariant;

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Parser, Debug)]
#[command(about = "Benchmark lossless image compression formats for RH maps and sprites")]
struct Cli {
    /// Data directory (the one containing `DATA/…`).
    #[arg(long)]
    data_dir: PathBuf,

    /// Limit map benchmark to this many `.map` files (0 = all).
    #[arg(long, default_value_t = 1)]
    max_maps: usize,

    /// How many random animations to sample across all characters.
    #[arg(long, default_value_t = 5)]
    anim_samples: usize,

    /// Minimum sequential frames required for an animation to be sampled.
    #[arg(long, default_value_t = 4)]
    min_anim_frames: usize,

    /// Seed for animation sampling RNG.
    #[arg(long, default_value_t = 42)]
    seed: u64,

    /// cjxl effort level (1-9). 9 is glacial but best; 7 is the sweet spot.
    #[arg(long, default_value_t = 7)]
    jxl_effort: u8,

    /// avifenc speed (0-10). 0 = slowest/best, 6 = reasonable default.
    #[arg(long, default_value_t = 6)]
    avif_speed: u8,

    /// If set, keep the temp dir around (for inspection).
    #[arg(long)]
    keep_temp: bool,

    /// Skip the map benchmark.
    #[arg(long)]
    skip_maps: bool,

    /// Skip the sprite benchmark.
    #[arg(long)]
    skip_sprites: bool,

    /// Enable AV1 lossless encoding via ffmpeg (slow but interesting).
    #[arg(long)]
    av1: bool,

    /// Run the losing codecs too (AVIF, QOI, rgba+z22, 565+z3, animated JXL,
    /// AV1, palette, XOR delta). By default only the winners are measured
    /// so reports finish quickly.
    #[arg(long)]
    all_codecs: bool,

    /// Also encode every frame of one or more whole characters (all profiles,
    /// all rows, all frames) as a single RGB565 + zstd22 blob — to see how
    /// much cross-animation redundancy the bank has.
    #[arg(long)]
    whole_character: Vec<String>,

    /// Also test the whole-bank reorder hypothesis: concat every sprite's
    /// packed_data in (a) bank order and (b) sorted by (character-first-
    /// using-action, action_id, frame_idx, direction) — then compress each
    /// at zstd-22 with `--long=31` (the actual shipping settings).
    /// Takes ~10-15 minutes because of the ~500 MiB compression pass.
    #[arg(long)]
    whole_bank: bool,

    /// Report how much bank storage goes to characters vs map patches vs
    /// unreferenced sprites, plus a size histogram. Fast (a few seconds).
    #[arg(long)]
    sprite_breakdown: bool,
}

#[derive(Clone, Copy)]
struct CodecOpts {
    jxl_effort: u8,
    avif_speed: u8,
    all_codecs: bool,
}

// ---------------------------------------------------------------------------
// Shared data types
// ---------------------------------------------------------------------------

struct BenchFrame {
    w: u32,
    h: u32,
    rgb565: Vec<u8>,
}

#[derive(Default, Debug)]
struct Metrics {
    png_opti: u64,
    jxl_lossless: u64,
    /// cjxl -q 90 — visually lossless, much smaller than -d 0.
    jxl_q90: u64,
    rgb565_zstd22: u64,
    /// For sprites only: sum of per-frame tight-bounded RGB565, concat + zstd22.
    /// This is the fair analog to the RLE/VQ bank, without rectangular padding.
    bounded_frames_zstd: Option<u64>,
    /// Sprites only: concat of the bank's RLE/VQ packed_data bytes (+ tiny
    /// per-sprite header), then zstd22. "Don't change the bank format,
    /// just zstd it."
    z22_orig: Option<u64>,
    /// Sprites only: place each frame on a shared canvas using its
    /// script.offset, XOR with the previous frame's canvas, zstd22.
    /// Tests the intra-animation delta idea.
    z22_delt: Option<u64>,

    // "All codecs" columns — 0 unless `--all-codecs` is set.
    png_raw: u64,
    avif_lossless: u64,
    qoi: u64,
    rgb565_zstd3: u64,
    rgba_zstd22: u64,
    animated_jxl: Option<u64>,
    av1_lossless: Option<u64>,
}

// ---------------------------------------------------------------------------

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();
    let data_dir = cli
        .data_dir
        .canonicalize()
        .with_context(|| format!("failed to resolve --data-dir {}", cli.data_dir.display()))?;

    let tmp = tempfile::Builder::new()
        .prefix("rh_sprite_bench_")
        .tempdir()?;
    let tmp_path = tmp.path().to_path_buf();
    println!("# temp work dir: {}", tmp_path.display());

    let codec = CodecOpts {
        jxl_effort: cli.jxl_effort,
        avif_speed: cli.avif_speed,
        all_codecs: cli.all_codecs,
    };

    if !cli.skip_maps {
        bench_maps(&data_dir, &tmp_path, cli.max_maps, codec)?;
    }

    if !cli.skip_sprites {
        bench_anim_samples(
            &data_dir,
            &tmp_path,
            cli.anim_samples,
            cli.min_anim_frames,
            cli.seed,
            cli.av1,
            codec,
        )?;
    }

    if !cli.whole_character.is_empty() {
        bench_whole_character(&data_dir, &cli.whole_character)?;
    }

    if cli.whole_bank {
        bench_whole_bank(&data_dir)?;
    }

    if cli.sprite_breakdown {
        bench_sprite_breakdown(&data_dir)?;
    }

    if cli.keep_temp {
        let _ = tmp.keep();
        println!("# temp kept at above path");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Map benchmark
// ---------------------------------------------------------------------------

fn bench_maps(data_dir: &Path, tmp: &Path, max: usize, codec: CodecOpts) -> Result<()> {
    let mut maps: Vec<PathBuf> = walk_ext(data_dir.join("DATA/Levels"), "map");
    maps.sort();
    if max > 0 {
        maps.truncate(max);
    }
    if maps.is_empty() {
        println!(
            "# no .map files found under {}/DATA/Levels",
            data_dir.display()
        );
        return Ok(());
    }

    println!();
    println!("## Background maps");
    print_map_header(codec.all_codecs);

    for map_path in &maps {
        let original_bytes = fs::metadata(map_path)?.len();

        let mut file = SbFile::open(map_path.to_str().unwrap(), sbfile::SB_FILE_READ)
            .map_err(|e| anyhow!("open {}: {e}", map_path.display()))?;
        let pic = Picture::load_sixteen_from_stream(&mut file)
            .with_context(|| format!("decode {}", map_path.display()))?;
        let w = pic.width as u32;
        let h = pic.height as u32;

        let rgb565: Vec<u8> = pic.data.clone();
        let rgba: Vec<u8> = pic.to_rgba8888(None);

        let label = map_path
            .strip_prefix(data_dir)
            .unwrap_or(map_path)
            .display()
            .to_string();

        eprintln!("# encoding {}...", label);
        let metrics = measure_single_image(tmp, &label, w, h, &rgb565, &rgba, false, codec)?;
        print_map_row(&label, w, h, original_bytes, &metrics, codec.all_codecs);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Sprite benchmark — sample N random animations across all characters
// ---------------------------------------------------------------------------

fn bench_anim_samples(
    data_dir: &Path,
    tmp: &Path,
    n_samples: usize,
    min_frames: usize,
    seed: u64,
    enable_av1: bool,
    codec: CodecOpts,
) -> Result<()> {
    use rand::{SeedableRng, seq::SliceRandom};

    let data_dir_s = data_dir.to_str().unwrap();
    println!();
    println!("## Animation samples ({} random, seed={})", n_samples, seed);

    let mut holder = FrameHolder::new();
    holder
        .initialize_sprite_bank(data_dir_s)
        .context("initialize_sprite_bank")?;
    eprintln!(
        "# bank loaded: {} sprites, {} dictionaries",
        holder.num_sprites(),
        holder.dictionaries().len()
    );

    // Enumerate every (character, profile, row) → frame sequence.
    #[derive(Clone)]
    struct Candidate {
        character: String,
        profile: String,
        row_idx: usize,
        action_id: u16,
        frame_ids: Vec<u32>,
        /// Per-frame (x, y) offsets from the SpriteScript. Combined with
        /// the sprite's own width/height, these place each frame on a
        /// shared canvas so we can delta-encode in the RGB565 domain.
        offsets: Vec<(f32, f32)>,
    }
    let mut candidates: Vec<Candidate> = Vec::new();

    let chars_dir = data_dir.join("DATA/Characters");
    let mut rhs_files: Vec<PathBuf> = walk_ext(chars_dir, "rhs");
    rhs_files.sort();

    for rhs_path in &rhs_files {
        let character = rhs_path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let (_sig, profiles) = match SpriteScriptor::load_all_profiles(rhs_path.to_str().unwrap()) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("# skip {}: {}", rhs_path.display(), e);
                continue;
            }
        };
        for (profile_name, info) in &profiles {
            for (row_idx, script) in info.scripts.iter().enumerate() {
                if script.frame_ids.len() >= min_frames {
                    candidates.push(Candidate {
                        character: character.clone(),
                        profile: profile_name.clone(),
                        row_idx,
                        action_id: script.action_id,
                        frame_ids: script.frame_ids.clone(),
                        offsets: script.offsets.iter().map(|o| (o.x, o.y)).collect(),
                    });
                }
            }
        }
    }
    eprintln!(
        "# {} candidate animations ≥{} frames across {} .rhs files",
        candidates.len(),
        min_frames,
        rhs_files.len()
    );
    if candidates.is_empty() {
        return Ok(());
    }

    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
    candidates.shuffle(&mut rng);
    candidates.truncate(n_samples);

    print_sprite_header(codec.all_codecs);

    for c in &candidates {
        let decode = |holder: &FrameHolder, id: u32| -> Option<BenchFrame> {
            let w = holder.sprite_width(id) as usize;
            let h = holder.sprite_height(id) as usize;
            if w == 0 || h == 0 {
                return None;
            }
            let mut dst = vec![TRANSPARENT_COLOR_16; w * h];
            holder.uncompress_frame(&mut dst, w, id, SpriteVariant::Day, 0x0010, 16);
            let rgb565: Vec<u8> = bytemuck::cast_slice::<u16, u8>(&dst).to_vec();
            Some(BenchFrame {
                w: w as u32,
                h: h as u32,
                rgb565,
            })
        };

        let frames: Vec<BenchFrame> = c
            .frame_ids
            .iter()
            .filter_map(|id| decode(&holder, *id))
            .collect();
        if frames.is_empty() {
            continue;
        }

        // Original cost = sum of bank-packed bytes for this row's frames
        // (sum of referenced IDs — inflates if the row re-uses a frame, but
        // that matches the animated stream we feed the video/jxl encoder).
        let original_packed: u64 = c
            .frame_ids
            .iter()
            .map(|&i| holder.sprites()[i as usize].packed_size as u64)
            .sum();

        // Sprite-sheet: frames laid out side-by-side with the tallest height.
        let sheet_h = frames.iter().map(|f| f.h).max().unwrap_or(0);
        let sheet_w: u32 = frames.iter().map(|f| f.w).sum();
        let pitch_bytes = sheet_w as usize * 2;
        let mut sheet_565 = vec![0u8; pitch_bytes * sheet_h as usize];
        {
            let key = TRANSPARENT_COLOR_16.to_le_bytes();
            for px in sheet_565.as_chunks_mut::<2>().0 {
                px.copy_from_slice(&key);
            }
        }
        let mut x_off: u32 = 0;
        for f in &frames {
            for y in 0..f.h as usize {
                let src_off = y * f.w as usize * 2;
                let dst_off = y * pitch_bytes + x_off as usize * 2;
                sheet_565[dst_off..dst_off + f.w as usize * 2]
                    .copy_from_slice(&f.rgb565[src_off..src_off + f.w as usize * 2]);
            }
            x_off += f.w;
        }
        let sheet_rgba = rgb565_to_rgba_keyed(&sheet_565, sheet_w, sheet_h);

        let label = format!(
            "{}/{}:row{}(act{}, {}f)",
            c.character,
            c.profile,
            c.row_idx,
            c.action_id,
            frames.len()
        );
        eprintln!(
            "# {}: sheet {}x{}, encoding stills...",
            label, sheet_w, sheet_h
        );
        let mut metrics = measure_single_image(
            tmp,
            &label,
            sheet_w,
            sheet_h,
            &sheet_565,
            &sheet_rgba,
            true,
            codec,
        )?;

        // Tight-bounded per-frame RGB565 + zstd22 — the fair analog of
        // RLE/VQ, no rectangular padding. For each frame, crop to its
        // opaque bounding box, concat all cropped pixels, and zstd.
        let mut bounded = Vec::new();
        for f in &frames {
            let (xmin, ymin, bw, bh) =
                opaque_bounds_565(&f.rgb565, f.w, f.h).unwrap_or((0, 0, f.w, f.h));
            // 8-byte per-frame header (w,h,xmin,ymin).
            bounded.extend_from_slice(&(bw as u16).to_le_bytes());
            bounded.extend_from_slice(&(bh as u16).to_le_bytes());
            bounded.extend_from_slice(&(xmin as u16).to_le_bytes());
            bounded.extend_from_slice(&(ymin as u16).to_le_bytes());
            for y in 0..bh as usize {
                let src_off = ((ymin as usize + y) * f.w as usize + xmin as usize) * 2;
                bounded.extend_from_slice(&f.rgb565[src_off..src_off + bw as usize * 2]);
            }
        }
        metrics.bounded_frames_zstd = Some(zstd_size(&bounded, 22)?);

        // z22-orig — same bytes the shipping bank would store for this
        // animation row: RLE/VQ packed_data for each frame (including the
        // repeats — animated-format comparisons use play order) plus a
        // tiny per-sprite header. Then zstd22.
        let mut blob_orig: Vec<u8> = Vec::new();
        for &id in &c.frame_ids {
            let s = &holder.sprites()[id as usize];
            let Some(pd) = holder.packed_data(id) else {
                continue;
            };
            blob_orig.extend_from_slice(&s.width.to_le_bytes());
            blob_orig.extend_from_slice(&s.height.to_le_bytes());
            blob_orig.extend_from_slice(&s.dictionary_index.to_le_bytes());
            blob_orig.extend_from_slice(&(pd.len() as u32).to_le_bytes());
            blob_orig.extend_from_slice(bytemuck::cast_slice::<u16, u8>(pd));
        }
        metrics.z22_orig = Some(zstd_size(&blob_orig, 22)?);

        // z22-delt — canvas-aligned XOR delta. Place each frame on a
        // shared canvas at (offset.x - min.x, offset.y - min.y), XOR
        // with the previous frame's canvas, concat + zstd22. If the
        // character body stays aligned across frames (hotspot convention)
        // the delta stream is mostly zero bytes outside the moving pixels.
        if c.frame_ids.len() == c.offsets.len() && !c.frame_ids.is_empty() {
            // Compute per-frame canvas-space rect from (offset, sprite size).
            let mut rects: Vec<(i32, i32, u32, u32)> = Vec::with_capacity(c.frame_ids.len());
            for (i, &id) in c.frame_ids.iter().enumerate() {
                let w = holder.sprite_width(id) as u32;
                let h = holder.sprite_height(id) as u32;
                let (ox, oy) = c.offsets[i];
                rects.push((ox as i32, oy as i32, w, h));
            }
            let min_x = rects.iter().map(|r| r.0).min().unwrap();
            let min_y = rects.iter().map(|r| r.1).min().unwrap();
            let canvas_w = rects
                .iter()
                .map(|r| r.0 + r.2 as i32 - min_x)
                .max()
                .unwrap() as usize;
            let canvas_h = rects
                .iter()
                .map(|r| r.1 + r.3 as i32 - min_y)
                .max()
                .unwrap() as usize;

            let mut canvas = vec![TRANSPARENT_COLOR_16; canvas_w * canvas_h];
            let mut prev_canvas: Option<Vec<u16>> = None;
            let mut blob_delt: Vec<u8> = Vec::new();
            blob_delt.extend_from_slice(&(canvas_w as u32).to_le_bytes());
            blob_delt.extend_from_slice(&(canvas_h as u32).to_le_bytes());

            for (i, &id) in c.frame_ids.iter().enumerate() {
                // Reset canvas to transparent each frame (frames don't
                // accumulate — each action frame is independent).
                for px in canvas.iter_mut() {
                    *px = TRANSPARENT_COLOR_16;
                }
                // Decode this frame's tight sprite.
                let w = holder.sprite_width(id) as usize;
                let h = holder.sprite_height(id) as usize;
                if w == 0 || h == 0 {
                    continue;
                }
                let mut frame = vec![TRANSPARENT_COLOR_16; w * h];
                holder.uncompress_frame(&mut frame, w, id, SpriteVariant::Day, 0x0010, 16);

                // Blit onto the canvas at its aligned position.
                let dx = (rects[i].0 - min_x) as usize;
                let dy = (rects[i].1 - min_y) as usize;
                for y in 0..h {
                    let src_off = y * w;
                    let dst_off = (dy + y) * canvas_w + dx;
                    canvas[dst_off..dst_off + w].copy_from_slice(&frame[src_off..src_off + w]);
                }

                // Emit either raw canvas (first frame) or XOR delta.
                if let Some(prev) = prev_canvas.as_ref() {
                    for (c, p) in canvas.iter().zip(prev.iter()) {
                        blob_delt.extend_from_slice(&(c ^ p).to_le_bytes());
                    }
                } else {
                    blob_delt.extend_from_slice(bytemuck::cast_slice::<u16, u8>(&canvas));
                }
                prev_canvas = Some(canvas.clone());
            }
            metrics.z22_delt = Some(zstd_size(&blob_delt, 22)?);
        }

        if codec.all_codecs {
            let anim_key = format!(
                "{}_{}_r{}",
                sanitize(&c.character),
                sanitize(&c.profile),
                c.row_idx
            );
            eprintln!("#   animated-jxl...");
            metrics.animated_jxl = Some(encode_animated_jxl(
                tmp,
                &anim_key,
                &frames,
                codec.jxl_effort,
            )?);
            if enable_av1 {
                eprintln!("#   av1-lossless...");
                metrics.av1_lossless = encode_av1_lossless(tmp, &anim_key, &frames).ok();
            }
        }

        print_sprite_row(
            &label,
            sheet_w,
            sheet_h,
            original_packed,
            &metrics,
            codec.all_codecs,
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Whole-character bench — every frame of every profile/row concat + zstd22
// ---------------------------------------------------------------------------

fn bench_whole_character(data_dir: &Path, characters: &[String]) -> Result<()> {
    let data_dir_s = data_dir.to_str().unwrap();
    println!();
    println!("## Whole-character (bank RLE/VQ vs format tweaks, all frames)");
    println!(
        "{:<14} {:>6} {:>9} {:>9} {:>11} {:>12} {:>11} {:>12} {:>7}",
        "character",
        "unique",
        "orig-rle",
        "z22-orig",
        "rgba-LL",
        "rgba-Q90",
        "rgb-LL",
        "rgb-Q90",
        "best/o",
    );

    let tmp = tempfile::Builder::new().prefix("rh_atlas_jxl_").tempdir()?;

    let mut holder = FrameHolder::new();
    holder
        .initialize_sprite_bank(data_dir_s)
        .context("initialize_sprite_bank")?;
    eprintln!(
        "# bank loaded: {} sprites, {} dictionaries",
        holder.num_sprites(),
        holder.dictionaries().len()
    );

    for name in characters {
        let rhs_path = data_dir.join(format!("DATA/Characters/{name}.rhs"));
        if !rhs_path.is_file() {
            eprintln!("# missing {}", rhs_path.display());
            continue;
        }

        let (_sig, profiles) = SpriteScriptor::load_all_profiles(rhs_path.to_str().unwrap())
            .map_err(|e| anyhow!("load rhs {}: {e}", rhs_path.display()))?;

        // Bank order — the current shipping format. Sort unique ids.
        let mut ids_bank: Vec<u32> = Vec::new();
        for (_p, info) in &profiles {
            for s in info.scripts.iter() {
                ids_bank.extend_from_slice(&s.frame_ids);
            }
        }
        ids_bank.sort_unstable();
        ids_bank.dedup();

        // --- sprite blob helpers ---

        // Default: one sprite → [w, h, dict_index, len_u16, packed_data bytes].
        let emit_orig = |ids: &[u32], out: &mut Vec<u8>| {
            for id in ids {
                let s = &holder.sprites()[*id as usize];
                let Some(pd) = holder.packed_data(*id) else {
                    continue;
                };
                out.extend_from_slice(&s.width.to_le_bytes());
                out.extend_from_slice(&s.height.to_le_bytes());
                out.extend_from_slice(&s.dictionary_index.to_le_bytes());
                out.extend_from_slice(&(pd.len() as u32).to_le_bytes());
                out.extend_from_slice(bytemuck::cast_slice::<u16, u8>(pd));
            }
        };

        // Original size (sum of packed_size), for the "vs orig" column.
        let orig_packed: u64 = ids_bank
            .iter()
            .map(|&i| holder.sprites()[i as usize].packed_size as u64)
            .sum();

        // z22-orig (baseline, bank order).
        let mut blob = Vec::new();
        emit_orig(&ids_bank, &mut blob);
        let z22_orig = zstd::stream::encode_all(&blob[..], 22)?.len() as u64;

        // atlas-jxlLL / atlas-jxlQ90 — pack every unique frame into one
        // tight 2D atlas (shelf-packing), render to RGBA with the 0x07C0
        // key → alpha=0, and run cjxl once. Tests whether amortising JXL's
        // per-image overhead across 5-8k frames beats per-animation-strip
        // (which loses to packed+z22 at small scale).
        let mut frames: Vec<(u32, u16, u16, Vec<u16>)> = Vec::with_capacity(ids_bank.len());
        for &id in &ids_bank {
            let w = holder.sprite_width(id) as usize;
            let h = holder.sprite_height(id) as usize;
            if w == 0 || h == 0 {
                continue;
            }
            let mut dst = vec![TRANSPARENT_COLOR_16; w * h];
            holder.uncompress_frame(&mut dst, w, id, SpriteVariant::Day, 0x0010, 16);
            frames.push((id, w as u16, h as u16, dst));
        }
        // Shelf-pack with atlas width ≈ sqrt(total_pixels) for squareness.
        let total_pixels: u64 = frames.iter().map(|f| f.1 as u64 * f.2 as u64).sum();
        let atlas_w: u32 = ((total_pixels as f64 * 1.15).sqrt() as u32)
            .next_multiple_of(8)
            .max(512);
        // Sort by height descending for packing efficiency.
        let mut order: Vec<usize> = (0..frames.len()).collect();
        order.sort_by(|&a, &b| frames[b].2.cmp(&frames[a].2));
        let mut cx: u32 = 0;
        let mut cy: u32 = 0;
        let mut row_h: u32 = 0;
        let mut placements: Vec<(u32, u32)> = vec![(0, 0); frames.len()];
        for i in order {
            let fw = frames[i].1 as u32;
            let fh = frames[i].2 as u32;
            if cx + fw > atlas_w {
                cx = 0;
                cy += row_h;
                row_h = 0;
            }
            placements[i] = (cx, cy);
            cx += fw;
            if fh > row_h {
                row_h = fh;
            }
        }
        let atlas_h: u32 = cy + row_h;
        // Build two atlases:
        //   (a) RGBA — 0x07C0 key → alpha=0, shadow (0x001F) kept as opaque
        //       pure blue. What we were doing before.
        //   (b) RGB verbatim — key and shadow colors stay as opaque pixel
        //       values. JXL sees a 3-channel image where "transparent"
        //       is just "very common pure-green color" which entropy
        //       coding dedupes for free, without the per-pixel alpha cost.
        let atlas_px = atlas_w as usize * atlas_h as usize;
        let mut atlas_rgba = vec![0u8; atlas_px * 4];
        let mut atlas_rgb = vec![0u8; atlas_px * 3];
        // Default fill: RGBA → all-zero transparent; RGB → pure green (the key).
        // 0x07C0 = (R=0, G=0x3E in 6-bit, B=0); 6-bit→8-bit expand below.
        let key_g8 = (0x3Eu8 << 2) | (0x3Eu8 >> 4);
        for chunk in atlas_rgb.as_chunks_mut::<3>().0 {
            chunk[0] = 0;
            chunk[1] = key_g8;
            chunk[2] = 0;
        }
        for (i, (_id, w, h, px)) in frames.iter().enumerate() {
            let (dx, dy) = placements[i];
            let w = *w as u32;
            let h = *h as u32;
            for y in 0..h as usize {
                for x in 0..w as usize {
                    let src_off = y * w as usize + x;
                    let v = px[src_off];
                    let pix_off = (dy as usize + y) * atlas_w as usize + (dx as usize + x);

                    let r5 = ((v >> 11) & 0x1F) as u8;
                    let g6 = ((v >> 5) & 0x3F) as u8;
                    let b5 = (v & 0x1F) as u8;
                    let r8 = (r5 << 3) | (r5 >> 2);
                    let g8 = (g6 << 2) | (g6 >> 4);
                    let b8 = (b5 << 3) | (b5 >> 2);

                    // RGB verbatim — key stays as green, shadow as blue.
                    let rgb_off = pix_off * 3;
                    atlas_rgb[rgb_off] = r8;
                    atlas_rgb[rgb_off + 1] = g8;
                    atlas_rgb[rgb_off + 2] = b8;

                    // RGBA with keyed alpha.
                    let rgba_off = pix_off * 4;
                    if v == TRANSPARENT_COLOR_16 {
                        atlas_rgba[rgba_off..rgba_off + 4].copy_from_slice(&[0, 0, 0, 0]);
                    } else {
                        atlas_rgba[rgba_off] = r8;
                        atlas_rgba[rgba_off + 1] = g8;
                        atlas_rgba[rgba_off + 2] = b8;
                        atlas_rgba[rgba_off + 3] = 0xFF;
                    }
                }
            }
        }
        let rgba_png = tmp.path().join(format!("{}.rgba.png", sanitize(name)));
        let rgb_png = tmp.path().join(format!("{}.rgb.png", sanitize(name)));
        let rgba_ll_jxl = tmp.path().join(format!("{}.rgba.ll.jxl", sanitize(name)));
        let rgba_q90_jxl = tmp.path().join(format!("{}.rgba.q90.jxl", sanitize(name)));
        let rgb_ll_jxl = tmp.path().join(format!("{}.rgb.ll.jxl", sanitize(name)));
        let rgb_q90_jxl = tmp.path().join(format!("{}.rgb.q90.jxl", sanitize(name)));
        write_png_rgba(&rgba_png, atlas_w, atlas_h, &atlas_rgba)?;
        write_png_rgb(&rgb_png, atlas_w, atlas_h, &atlas_rgb)?;
        eprintln!(
            "# {}: atlas {}×{} ({} frames), encoding cjxl...",
            name,
            atlas_w,
            atlas_h,
            frames.len()
        );
        let rgba_ll = run_cjxl(&rgba_png, &rgba_ll_jxl, true, 7).unwrap_or(0);
        let rgba_q90 = run_cjxl_lossy(&rgba_png, &rgba_q90_jxl, 90, 7).unwrap_or(0);
        let rgb_ll = run_cjxl(&rgb_png, &rgb_ll_jxl, true, 7).unwrap_or(0);
        let rgb_q90 = run_cjxl_lossy(&rgb_png, &rgb_q90_jxl, 90, 7).unwrap_or(0);

        let best = z22_orig.min(rgba_ll).min(rgba_q90).min(rgb_ll).min(rgb_q90);
        let best_vs_orig = if orig_packed > 0 {
            format!("{:.2}×", best as f64 / orig_packed as f64)
        } else {
            "-".into()
        };

        println!(
            "{:<14} {:>6} {:>9} {:>9} {:>11} {:>12} {:>11} {:>12} {:>7}",
            truncate(name, 14),
            ids_bank.len(),
            human(orig_packed),
            human(z22_orig),
            human(rgba_ll),
            human(rgba_q90),
            human(rgb_ll),
            human(rgb_q90),
            best_vs_orig,
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Whole-bank reorder bench — zstd22 with shipping settings (long=31)
// ---------------------------------------------------------------------------

fn bench_whole_bank(data_dir: &Path) -> Result<()> {
    use std::collections::{HashMap, HashSet};
    use zstd::stream::raw::CParameter;

    let data_dir_s = data_dir.to_str().unwrap();
    println!();
    println!("## Whole-bank reorder — zstd22, windowLog=31, long-distance matching");

    let mut holder = FrameHolder::new();
    holder
        .initialize_sprite_bank(data_dir_s)
        .context("initialize_sprite_bank")?;
    eprintln!(
        "# bank loaded: {} sprites, {} dictionaries",
        holder.num_sprites(),
        holder.dictionaries().len()
    );

    // -- Build the "reordered" ID permutation --
    //
    // For every .rhs file in Characters/, walk its profiles → actions →
    // frame index → direction, emitting sprite IDs in that order. Sprite
    // IDs are globally unique in the bank so we can just push them to a
    // single "ordered_ids" vector, skipping duplicates. After walking all
    // characters, append any bank IDs not yet seen (unreferenced by any
    // .rhs — e.g. menu/HUD sprites) in bank order.
    let chars_dir = data_dir.join("DATA/Characters");
    let mut rhs_files: Vec<PathBuf> = walk_ext(chars_dir, "rhs");
    rhs_files.sort();

    let mut seen = HashSet::<u32>::new();
    let mut ordered_ids: Vec<u32> = Vec::with_capacity(holder.num_sprites());

    for rhs_path in &rhs_files {
        let (_sig, profiles) = match SpriteScriptor::load_all_profiles(rhs_path.to_str().unwrap()) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("# skip {}: {e}", rhs_path.display());
                continue;
            }
        };
        for (_p, info) in &profiles {
            let mut by_action: HashMap<u16, Vec<&robin_engine::sprite_script::SpriteScript>> =
                HashMap::new();
            for s in info.scripts.iter() {
                by_action.entry(s.action_id).or_default().push(s);
            }
            let mut action_ids: Vec<u16> = by_action.keys().copied().collect();
            action_ids.sort_unstable();
            for a in action_ids {
                let rows = &by_action[&a];
                let max_f = rows.iter().map(|r| r.frame_ids.len()).max().unwrap_or(0);
                // frame-index-first interleave: frame 0 across all 16 dirs,
                // then frame 1 across all 16 dirs, …
                for f in 0..max_f {
                    for r in rows {
                        if let Some(&id) = r.frame_ids.get(f)
                            && seen.insert(id)
                        {
                            ordered_ids.push(id);
                        }
                    }
                }
            }
        }
    }
    // Append any sprites no .rhs referenced, in bank order.
    for id in 0..holder.num_sprites() as u32 {
        if !seen.contains(&id) {
            ordered_ids.push(id);
        }
    }
    eprintln!("# ordering walked {} rhs files", rhs_files.len());

    // Helper: concat packed_data bytes in a given id order.
    let build_blob = |ids: &[u32]| -> Vec<u8> {
        let mut out: Vec<u8> = Vec::with_capacity(256 * 1024 * 1024);
        for &id in ids {
            let s = &holder.sprites()[id as usize];
            let Some(pd) = holder.packed_data(id) else {
                continue;
            };
            out.extend_from_slice(&s.width.to_le_bytes());
            out.extend_from_slice(&s.height.to_le_bytes());
            out.extend_from_slice(&s.dictionary_index.to_le_bytes());
            out.extend_from_slice(&(pd.len() as u32).to_le_bytes());
            out.extend_from_slice(bytemuck::cast_slice::<u16, u8>(pd));
        }
        out
    };

    let shipping_compress = |blob: &[u8]| -> Result<u64> {
        let mut out = Vec::new();
        let mut enc = zstd::stream::write::Encoder::new(&mut out, 22)?;
        enc.set_parameter(CParameter::WindowLog(31))?;
        enc.set_parameter(CParameter::EnableLongDistanceMatching(true))?;
        std::io::Write::write_all(&mut enc, blob)?;
        enc.finish()?;
        Ok(out.len() as u64)
    };

    // -- Bank order (what shipping datadir currently uses) --
    eprintln!("# building bank-order blob (~500 MiB)...");
    let bank_ids: Vec<u32> = (0..holder.num_sprites() as u32).collect();
    let blob_bank = build_blob(&bank_ids);
    let raw_bytes = blob_bank.len() as u64;
    eprintln!(
        "# raw blob = {} ({} sprites incl. empty)",
        human(raw_bytes),
        bank_ids.len()
    );
    eprintln!("# compressing bank-order blob at zstd22 long=31...");
    let t0 = std::time::Instant::now();
    let size_bank = shipping_compress(&blob_bank)?;
    eprintln!(
        "#   → {} ({:.1}s)",
        human(size_bank),
        t0.elapsed().as_secs_f32()
    );
    drop(blob_bank);

    // -- Reordered (frame-index-first per character-action) --
    eprintln!("# building reordered blob...");
    let blob_reord = build_blob(&ordered_ids);
    assert_eq!(
        blob_reord.len() as u64,
        raw_bytes,
        "ordering must be a permutation"
    );
    eprintln!("# compressing reordered blob at zstd22 long=31...");
    let t0 = std::time::Instant::now();
    let size_reord = shipping_compress(&blob_reord)?;
    eprintln!(
        "#   → {} ({:.1}s)",
        human(size_reord),
        t0.elapsed().as_secs_f32()
    );
    drop(blob_reord);

    println!(
        "{:<28} {:>12} {:>12} {:>10}",
        "ordering", "raw", "zstd22 long=31", "ratio"
    );
    println!(
        "{:<28} {:>12} {:>12} {:>9.2}×",
        "bank (shipping today)",
        human(raw_bytes),
        human(size_bank),
        size_bank as f64 / raw_bytes as f64
    );
    println!(
        "{:<28} {:>12} {:>12} {:>9.2}×",
        "reordered (action/frame/dir)",
        human(raw_bytes),
        human(size_reord),
        size_reord as f64 / raw_bytes as f64
    );
    let delta = size_reord as i64 - size_bank as i64;
    let pct = 100.0 * delta as f64 / size_bank as f64;
    println!(
        "{:<28} {:>12} {:>12} {:>+9.2}%",
        "reorder vs bank",
        "",
        human(delta.unsigned_abs()),
        pct
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Sprite bank breakdown — characters vs patches vs unreferenced, by size
// ---------------------------------------------------------------------------

fn bench_sprite_breakdown(data_dir: &Path) -> Result<()> {
    use std::collections::HashMap;

    let data_dir_s = data_dir.to_str().unwrap();
    println!();
    println!("## Sprite bank breakdown — by source bucket and size");

    let mut holder = FrameHolder::new();
    holder
        .initialize_sprite_bank(data_dir_s)
        .context("initialize_sprite_bank")?;

    // Collect every sprite id referenced from any .rhs file, tagged by
    // which top-level directory the .rhs came from. `Characters/` →
    // characters; `Animations/*/` → map patches, effects, UI panels, etc.
    // First-seen tag wins so character-shared sprites stay tagged as
    // characters even if also referenced from an animation overlay.
    let mut referenced: HashMap<u32, &'static str> = HashMap::new();
    let mut file_counts = [0usize; 2];
    for (idx, (rel, tag)) in [
        ("DATA/Characters", "character"),
        ("DATA/Animations", "patch/anim"),
    ]
    .iter()
    .enumerate()
    {
        let root = data_dir.join(rel);
        if !root.exists() {
            continue;
        }
        let files: Vec<PathBuf> = walk_ext(root, "rhs");
        file_counts[idx] = files.len();
        for p in &files {
            let Ok((_sig, profiles)) = SpriteScriptor::load_all_profiles(p.to_str().unwrap())
            else {
                continue;
            };
            for (_p, info) in &profiles {
                for s in info.scripts.iter() {
                    for &id in &s.frame_ids {
                        referenced.entry(id).or_insert(tag);
                    }
                }
            }
        }
    }
    eprintln!(
        "# scanned {} Characters/*.rhs + {} Animations/**/*.rhs",
        file_counts[0], file_counts[1]
    );

    // Walk every bank sprite; categorize and accumulate.
    struct Bucket {
        sprites: u64,
        bytes: u64,
        size_hist: [u64; 8], // 0..16, 16..64, 64..256, ..., 16384..65536, 65536+
    }
    impl Bucket {
        fn empty() -> Self {
            Self {
                sprites: 0,
                bytes: 0,
                size_hist: [0; 8],
            }
        }
        fn add(&mut self, packed: u64) {
            self.sprites += 1;
            self.bytes += packed;
            let b = match packed {
                0..=15 => 0,
                16..=63 => 1,
                64..=255 => 2,
                256..=1_023 => 3,
                1_024..=4_095 => 4,
                4_096..=16_383 => 5,
                16_384..=65_535 => 6,
                _ => 7,
            };
            self.size_hist[b] += 1;
        }
    }

    let mut by_tag: HashMap<&'static str, Bucket> = HashMap::new();
    let mut unref = Bucket::empty();
    let mut empty_slots = 0u64;
    for (id, s) in holder.sprites().iter().enumerate() {
        if holder.packed_data(id as u32).is_none() || s.packed_size == 0 {
            empty_slots += 1;
            continue;
        }
        let packed = s.packed_size as u64;
        match referenced.get(&(id as u32)) {
            Some(tag) => by_tag.entry(tag).or_insert_with(Bucket::empty).add(packed),
            None => unref.add(packed),
        }
    }

    // Also report the actual max dimensions per bucket, since "character
    // sprites" are tiny (~30-80px) and "patches" can be huge (thousands).
    let mut dims_by_tag: HashMap<&'static str, (u32, u32, u64)> = HashMap::new();
    let mut unref_dims = (0u32, 0u32, 0u64);
    for (id, s) in holder.sprites().iter().enumerate() {
        if holder.packed_data(id as u32).is_none() {
            continue;
        }
        let (mw, mh, mpix) = match referenced.get(&(id as u32)) {
            Some(tag) => dims_by_tag.entry(tag).or_insert((0, 0, 0)),
            None => &mut unref_dims,
        };
        if s.width as u32 > *mw {
            *mw = s.width as u32;
        }
        if s.height as u32 > *mh {
            *mh = s.height as u32;
        }
        *mpix = (*mpix).max(s.width as u64 * s.height as u64);
    }

    let total_bytes: u64 = by_tag.values().map(|b| b.bytes).sum::<u64>() + unref.bytes;

    println!();
    println!(
        "{:<18} {:>8} {:>10} {:>7} {:>12} {:>30}",
        "bucket", "sprites", "bytes", "share", "max w×h", "size histogram (bytes)",
    );
    let print_bucket = |name: &str, b: &Bucket, dims: (u32, u32, u64)| {
        println!(
            "{:<18} {:>8} {:>10} {:>6.1}% {:>12} {:>4}/{:>4}/{:>4}/{:>4}/{:>4}/{:>4}/{:>4}/{:>4}",
            name,
            b.sprites,
            human(b.bytes),
            if total_bytes > 0 {
                100.0 * b.bytes as f64 / total_bytes as f64
            } else {
                0.0
            },
            format!("{}×{}", dims.0, dims.1),
            b.size_hist[0],
            b.size_hist[1],
            b.size_hist[2],
            b.size_hist[3],
            b.size_hist[4],
            b.size_hist[5],
            b.size_hist[6],
            b.size_hist[7],
        );
    };
    // Print tags in a stable order.
    for tag in ["character", "patch/anim"] {
        if let Some(b) = by_tag.get(tag) {
            let dims = dims_by_tag.get(tag).copied().unwrap_or((0, 0, 0));
            print_bucket(tag, b, dims);
        }
    }
    print_bucket("unreferenced", &unref, unref_dims);
    println!(
        "{:<18} {:>8}  (filtered out of totals)",
        "empty slots", empty_slots,
    );
    println!("# histogram buckets: <16B / 16-63 / 64-255 / 256-1K / 1-4K / 4-16K / 16-64K / ≥64K");

    // Now also show the 20 largest sprites by packed_size across the
    // whole bank, annotated by bucket — these are the big overlays /
    // UI panels / patches the user wants us to check.
    let mut all: Vec<(u32, u64, u16, u16, &'static str)> = Vec::new();
    for (id, s) in holder.sprites().iter().enumerate() {
        if holder.packed_data(id as u32).is_none() {
            continue;
        }
        let tag = referenced
            .get(&(id as u32))
            .copied()
            .unwrap_or("unreferenced");
        all.push((id as u32, s.packed_size as u64, s.width, s.height, tag));
    }
    all.sort_by_key(|r| std::cmp::Reverse(r.1));
    all.truncate(20);
    println!();
    println!("## Top 20 largest sprites + JXL trial (packed_size vs JXL lossless / q90)");
    println!(
        "{:>6} {:>10} {:>9} {:>9} {:>9} {:>9} {:<13}",
        "id", "w×h", "packed", "pkd+z22", "jxl-ll", "jxl-q90", "bucket"
    );

    let tmp = tempfile::Builder::new().prefix("rh_patch_jxl_").tempdir()?;
    let mut sum_packed = 0u64;
    let mut sum_jxl_ll = 0u64;
    let mut sum_jxl_q90 = 0u64;
    for (id, packed, w, h, tag) in &all {
        let s = &holder.sprites()[*id as usize];
        // Decode to RGB565 → RGBA (opacity keyed on 0x07C0) → PNG → JXL.
        let pw = *w as usize;
        let ph = *h as usize;
        let mut dst = vec![TRANSPARENT_COLOR_16; pw * ph];
        holder.uncompress_frame(&mut dst, pw, *id, SpriteVariant::Day, 0x0010, 16);
        let rgb565: Vec<u8> = bytemuck::cast_slice::<u16, u8>(&dst).to_vec();
        let rgba = rgb565_to_rgba_keyed(&rgb565, *w as u32, *h as u32);

        let png_path = tmp.path().join(format!("{id}.png"));
        let jxl_ll_path = tmp.path().join(format!("{id}.ll.jxl"));
        let jxl_q90_path = tmp.path().join(format!("{id}.q90.jxl"));
        write_png_rgba(&png_path, *w as u32, *h as u32, &rgba)?;
        let jxl_ll = run_cjxl(&png_path, &jxl_ll_path, true, 7).unwrap_or(0);
        let jxl_q90 = run_cjxl_lossy(&png_path, &jxl_q90_path, 90, 7).unwrap_or(0);

        // Packed bytes + zstd22 — the "already-in-shipping-format" baseline.
        let pkd_z22 = s
            .packed_data
            .as_ref()
            .map(|pd| {
                zstd::stream::encode_all(bytemuck::cast_slice::<u16, u8>(pd), 22)
                    .map(|v| v.len() as u64)
                    .unwrap_or(0)
            })
            .unwrap_or(0);

        sum_packed += *packed;
        sum_jxl_ll += jxl_ll;
        sum_jxl_q90 += jxl_q90;
        println!(
            "{:>6} {:>10} {:>9} {:>9} {:>9} {:>9} {:<13}",
            id,
            format!("{w}×{h}"),
            human(*packed),
            human(pkd_z22),
            human(jxl_ll),
            human(jxl_q90),
            tag,
        );
    }
    println!();
    println!(
        "sum of top-20: packed = {}, jxl-ll = {} ({:.2}×), jxl-q90 = {} ({:.2}×)",
        human(sum_packed),
        human(sum_jxl_ll),
        sum_jxl_ll as f64 / sum_packed as f64,
        human(sum_jxl_q90),
        sum_jxl_q90 as f64 / sum_packed as f64,
    );

    // Full patch-bucket aggregate: encode every patch/anim sprite and
    // also concat their packed_data in a single zstd22 stream — the
    // one-stream number is the closest proxy for "shipping bank cost
    // if this sprite group were stored separately".
    let patch_ids: Vec<u32> = holder
        .sprites()
        .iter()
        .enumerate()
        .filter(|(id, s)| {
            s.packed_data.is_some() && referenced.get(&(*id as u32)).copied() == Some("patch/anim")
        })
        .map(|(id, _)| id as u32)
        .collect();
    eprintln!(
        "# encoding all {} patch/anim sprites through cjxl (can take a couple minutes)...",
        patch_ids.len()
    );
    let mut sum_packed = 0u64;
    let mut sum_jxl_ll = 0u64;
    let mut sum_jxl_q90 = 0u64;
    let mut stream_packed: Vec<u8> = Vec::new();
    let mut stream_jxl_ll: Vec<u8> = Vec::new();
    let mut stream_jxl_q90: Vec<u8> = Vec::new();
    for id in &patch_ids {
        let s = &holder.sprites()[*id as usize];
        let Some(pd) = s.packed_data.as_ref() else {
            continue;
        };
        let pw = s.width as u32;
        let ph = s.height as u32;
        if pw == 0 || ph == 0 {
            continue;
        }
        let packed = s.packed_size as u64;

        // Decode + PNG + JXL.
        let mut dst = vec![TRANSPARENT_COLOR_16; pw as usize * ph as usize];
        holder.uncompress_frame(&mut dst, pw as usize, *id, SpriteVariant::Day, 0x0010, 16);
        let rgb565: Vec<u8> = bytemuck::cast_slice::<u16, u8>(&dst).to_vec();
        let rgba = rgb565_to_rgba_keyed(&rgb565, pw, ph);
        let png_path = tmp.path().join(format!("all_{id}.png"));
        let jxl_ll_path = tmp.path().join(format!("all_{id}.ll.jxl"));
        let jxl_q90_path = tmp.path().join(format!("all_{id}.q90.jxl"));
        write_png_rgba(&png_path, pw, ph, &rgba)?;
        let jxl_ll = run_cjxl(&png_path, &jxl_ll_path, true, 7).unwrap_or(0);
        let jxl_q90 = run_cjxl_lossy(&png_path, &jxl_q90_path, 90, 7).unwrap_or(0);
        sum_packed += packed;
        sum_jxl_ll += jxl_ll;
        sum_jxl_q90 += jxl_q90;

        // Concat blobs for the in-one-stream zstd comparison.
        stream_packed.extend_from_slice(&s.width.to_le_bytes());
        stream_packed.extend_from_slice(&s.height.to_le_bytes());
        stream_packed.extend_from_slice(&(pd.len() as u32).to_le_bytes());
        stream_packed.extend_from_slice(bytemuck::cast_slice::<u16, u8>(pd));
        if let Ok(b) = fs::read(&jxl_ll_path) {
            stream_jxl_ll.extend_from_slice(&(b.len() as u32).to_le_bytes());
            stream_jxl_ll.extend_from_slice(&b);
        }
        if let Ok(b) = fs::read(&jxl_q90_path) {
            stream_jxl_q90.extend_from_slice(&(b.len() as u32).to_le_bytes());
            stream_jxl_q90.extend_from_slice(&b);
        }

        let _ = fs::remove_file(&png_path);
        let _ = fs::remove_file(&jxl_ll_path);
        let _ = fs::remove_file(&jxl_q90_path);
    }
    let z_packed_full = zstd_size(&stream_packed, 22)?;
    let z_jxl_ll_full = zstd_size(&stream_jxl_ll, 22)?;
    let z_jxl_q90_full = zstd_size(&stream_jxl_q90, 22)?;
    println!();
    println!(
        "## Full patch bucket ({} sprites, all individually JXL'd)",
        patch_ids.len()
    );
    println!(
        "{:<28} {:>12} {:>12} {:>12}",
        "format", "raw sum", "zstd22 (in-stream)", "vs packed+z22"
    );
    println!(
        "{:<28} {:>12} {:>12} {:>12}",
        "packed RLE/VQ",
        human(sum_packed),
        human(z_packed_full),
        "1.00×",
    );
    println!(
        "{:<28} {:>12} {:>12} {:>11.2}×",
        "JXL lossless",
        human(sum_jxl_ll),
        human(z_jxl_ll_full),
        z_jxl_ll_full as f64 / z_packed_full as f64,
    );
    println!(
        "{:<28} {:>12} {:>12} {:>11.2}×",
        "JXL q90",
        human(sum_jxl_q90),
        human(z_jxl_q90_full),
        z_jxl_q90_full as f64 / z_packed_full as f64,
    );
    let save_ll = z_packed_full.saturating_sub(z_jxl_ll_full);
    let save_q90 = z_packed_full.saturating_sub(z_jxl_q90_full);
    println!(
        "# patch bucket savings if converted: lossless {}, q90 {}",
        human(save_ll),
        human(save_q90)
    );

    // Fair-to-shipping comparison: concat the top-20 sprites' packed_data
    // in a single zstd22 stream (which captures cross-sprite LZ matches
    // the way the real shipping bank does), vs an identical concat of
    // their JXL bytes + zstd22 wrapper.
    let mut blob_packed: Vec<u8> = Vec::new();
    let mut blob_jxl_ll: Vec<u8> = Vec::new();
    let mut blob_jxl_q90: Vec<u8> = Vec::new();
    for (id, _bytes, w, h, _tag) in &all {
        let s = &holder.sprites()[*id as usize];
        let Some(pd) = s.packed_data.as_ref() else {
            continue;
        };
        blob_packed.extend_from_slice(&w.to_le_bytes());
        blob_packed.extend_from_slice(&h.to_le_bytes());
        blob_packed.extend_from_slice(&(pd.len() as u32).to_le_bytes());
        blob_packed.extend_from_slice(bytemuck::cast_slice::<u16, u8>(pd));

        // JXL already has entropy coding so zstd's job is a no-op wrapper;
        // we write each blob verbatim to test the "cross-sprite" angle too.
        let jxl_ll_path = tmp.path().join(format!("{id}.ll.jxl"));
        let jxl_q90_path = tmp.path().join(format!("{id}.q90.jxl"));
        if let Ok(b) = fs::read(&jxl_ll_path) {
            blob_jxl_ll.extend_from_slice(&(b.len() as u32).to_le_bytes());
            blob_jxl_ll.extend_from_slice(&b);
        }
        if let Ok(b) = fs::read(&jxl_q90_path) {
            blob_jxl_q90.extend_from_slice(&(b.len() as u32).to_le_bytes());
            blob_jxl_q90.extend_from_slice(&b);
        }
    }
    let z_packed = zstd_size(&blob_packed, 22)?;
    let z_jxl_ll = zstd_size(&blob_jxl_ll, 22)?;
    let z_jxl_q90 = zstd_size(&blob_jxl_q90, 22)?;
    println!(
        "top-20 inside one zstd22 stream: packed+z22 = {}, jxl-ll+z22 = {} ({:.2}×), \
         jxl-q90+z22 = {} ({:.2}×)",
        human(z_packed),
        human(z_jxl_ll),
        z_jxl_ll as f64 / z_packed as f64,
        human(z_jxl_q90),
        z_jxl_q90 as f64 / z_packed as f64,
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Per-image measurement
// ---------------------------------------------------------------------------

fn measure_single_image(
    tmp: &Path,
    label: &str,
    w: u32,
    h: u32,
    rgb565: &[u8],
    rgba: &[u8],
    is_sprite: bool,
    codec: CodecOpts,
) -> Result<Metrics> {
    let key = sanitize(label);
    let png_path = tmp.join(format!("{key}.png"));
    let jxl_path = tmp.join(format!("{key}.jxl"));
    let jxl_q90_path = tmp.join(format!("{key}.q90.jxl"));

    // PNG — always write, it's the input to the cjxl/avifenc pipes.
    write_png_rgba(&png_path, w, h, rgba)?;
    let png_raw_size = fs::metadata(&png_path)?.len();

    // Winners (always on).
    let png_opti_path = tmp.join(format!("{key}.oxi.png"));
    fs::copy(&png_path, &png_opti_path)?;
    let _ = Command::new("oxipng")
        .args(["-o", "4", "--strip", "safe", "-q"])
        .arg(&png_opti_path)
        .status();
    let png_opti_size = fs::metadata(&png_opti_path)
        .map(|m| m.len())
        .unwrap_or(png_raw_size);

    let jxl_lossless = run_cjxl(&png_path, &jxl_path, is_sprite, codec.jxl_effort)?;
    let jxl_q90 = run_cjxl_lossy(&png_path, &jxl_q90_path, 90, codec.jxl_effort)?;
    let rgb565_zstd22 = zstd_size(rgb565, 22)?;

    let mut m = Metrics {
        png_opti: png_opti_size,
        jxl_lossless,
        jxl_q90,
        rgb565_zstd22,
        ..Default::default()
    };

    // Losers (only with --all-codecs).
    if codec.all_codecs {
        let avif_path = tmp.join(format!("{key}.avif"));
        let qoi_path = tmp.join(format!("{key}.qoi"));
        m.png_raw = png_raw_size;
        m.avif_lossless = run_avifenc(&png_path, &avif_path, codec.avif_speed).unwrap_or(0);
        m.qoi = write_qoi(&qoi_path, w, h, rgba)?;
        m.rgb565_zstd3 = zstd_size(rgb565, 3)?;
        m.rgba_zstd22 = zstd_size(rgba, 22)?;
    }

    Ok(m)
}

// ---------------------------------------------------------------------------
// Codec helpers
// ---------------------------------------------------------------------------

fn write_png_rgba(path: &Path, w: u32, h: u32, rgba: &[u8]) -> Result<()> {
    let file = fs::File::create(path)?;
    let wbuf = std::io::BufWriter::new(file);
    let mut enc = png::Encoder::new(wbuf, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::High);
    let mut writer = enc.write_header()?;
    writer.write_image_data(rgba)?;
    Ok(())
}

fn write_png_rgb(path: &Path, w: u32, h: u32, rgb: &[u8]) -> Result<()> {
    let file = fs::File::create(path)?;
    let wbuf = std::io::BufWriter::new(file);
    let mut enc = png::Encoder::new(wbuf, w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::High);
    let mut writer = enc.write_header()?;
    writer.write_image_data(rgb)?;
    Ok(())
}

fn write_qoi(path: &Path, w: u32, h: u32, rgba: &[u8]) -> Result<u64> {
    let encoded = qoi::encode_to_vec(rgba, w, h).map_err(|e| anyhow!("qoi encode: {e}"))?;
    fs::write(path, &encoded)?;
    Ok(encoded.len() as u64)
}

fn run_cjxl(png: &Path, out: &Path, is_sprite: bool, effort: u8) -> Result<u64> {
    let effort = effort.clamp(1, 9).to_string();
    let mut cmd = Command::new("cjxl");
    cmd.args(["-d", "0", "-e", &effort]);
    if is_sprite {
        cmd.args(["--modular=1"]);
    }
    cmd.arg(png).arg(out).stdout(std::process::Stdio::null());
    let status = cmd.status().context("spawn cjxl")?;
    if !status.success() {
        bail!("cjxl failed for {}", png.display());
    }
    Ok(fs::metadata(out)?.len())
}

/// cjxl in lossy mode (VarDCT) at a given quality (0-100, 90 ≈ visually lossless).
fn run_cjxl_lossy(png: &Path, out: &Path, quality: u8, effort: u8) -> Result<u64> {
    let effort = effort.clamp(1, 9).to_string();
    let q = quality.clamp(1, 100).to_string();
    let status = Command::new("cjxl")
        .args(["-q", &q, "-e", &effort])
        .arg(png)
        .arg(out)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("spawn cjxl (lossy)")?;
    if !status.success() {
        bail!("cjxl (lossy) failed");
    }
    Ok(fs::metadata(out)?.len())
}

fn run_avifenc(png: &Path, out: &Path, speed: u8) -> Result<u64> {
    let speed = speed.clamp(0, 10).to_string();
    let status = Command::new("avifenc")
        .args(["--lossless", "-s", &speed, "--ignore-icc"])
        .arg(png)
        .arg(out)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("spawn avifenc")?;
    if !status.success() {
        bail!("avifenc failed");
    }
    Ok(fs::metadata(out)?.len())
}

fn zstd_size(data: &[u8], level: i32) -> Result<u64> {
    let out = zstd::stream::encode_all(data, level)?;
    Ok(out.len() as u64)
}

fn encode_animated_jxl(tmp: &Path, name: &str, frames: &[BenchFrame], effort: u8) -> Result<u64> {
    let apng = tmp.join(format!("{}.apng.png", sanitize(name)));
    write_apng(&apng, frames)?;
    let out = tmp.join(format!("{}.anim.jxl", sanitize(name)));
    let effort = effort.clamp(1, 9).to_string();
    let status = Command::new("cjxl")
        .args(["-d", "0", "-e", &effort, "--modular=1"])
        .arg(&apng)
        .arg(&out)
        .stdout(std::process::Stdio::null())
        .status()
        .context("spawn cjxl for animation")?;
    if !status.success() {
        bail!("cjxl (animated) failed");
    }
    Ok(fs::metadata(&out)?.len())
}

fn encode_av1_lossless(tmp: &Path, name: &str, frames: &[BenchFrame]) -> Result<u64> {
    let frames_dir = tmp.join(format!("{}_frames", sanitize(name)));
    fs::create_dir_all(&frames_dir)?;
    let w = frames.iter().map(|f| f.w).max().unwrap_or(0);
    let h = frames.iter().map(|f| f.h).max().unwrap_or(0);
    for (i, f) in frames.iter().enumerate() {
        let padded = pad_rgb565(&f.rgb565, f.w, f.h, w, h);
        let rgba = rgb565_to_rgba_keyed(&padded, w, h);
        let p = frames_dir.join(format!("f{:05}.png", i));
        write_png_rgba(&p, w, h, &rgba)?;
    }
    let out = tmp.join(format!("{}.mkv", sanitize(name)));
    let status = Command::new("ffmpeg")
        .args(["-y", "-f", "image2", "-framerate", "10", "-i"])
        .arg(frames_dir.join("f%05d.png"))
        .args([
            "-c:v",
            "libaom-av1",
            "-cpu-used",
            "6",
            "-crf",
            "0",
            "-b:v",
            "0",
            "-aom-params",
            "lossless=1",
            "-pix_fmt",
            "yuv444p",
        ])
        .arg(&out)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("spawn ffmpeg")?;
    if !status.success() {
        bail!("ffmpeg av1 lossless failed");
    }
    Ok(fs::metadata(&out)?.len())
}

fn write_apng(path: &Path, frames: &[BenchFrame]) -> Result<()> {
    if frames.is_empty() {
        bail!("write_apng: no frames");
    }
    let w = frames.iter().map(|f| f.w).max().unwrap_or(0);
    let h = frames.iter().map(|f| f.h).max().unwrap_or(0);
    let file = fs::File::create(path)?;
    let wbuf = std::io::BufWriter::new(file);
    let mut enc = png::Encoder::new(wbuf, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_animated(frames.len() as u32, 0)?;
    enc.set_frame_delay(1, 10)?;
    enc.set_compression(png::Compression::High);
    let mut writer = enc.write_header()?;
    for f in frames {
        let padded = pad_rgb565(&f.rgb565, f.w, f.h, w, h);
        let rgba = rgb565_to_rgba_keyed(&padded, w, h);
        writer.write_image_data(&rgba)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

/// Tight opaque bounds (x,y,w,h) of an RGB565 buffer using the engine's
/// 0x07C0 transparent key. `None` if all pixels are transparent.
fn opaque_bounds_565(rgb565: &[u8], w: u32, h: u32) -> Option<(u32, u32, u32, u32)> {
    let key = TRANSPARENT_COLOR_16;
    let w = w as usize;
    let h = h as usize;
    if w == 0 || h == 0 || rgb565.len() < w * h * 2 {
        return None;
    }
    let mut xmin = usize::MAX;
    let mut ymin = usize::MAX;
    let mut xmax = 0usize;
    let mut ymax = 0usize;
    for y in 0..h {
        for x in 0..w {
            let off = (y * w + x) * 2;
            let px = u16::from_le_bytes([rgb565[off], rgb565[off + 1]]);
            if px != key {
                if x < xmin {
                    xmin = x;
                }
                if x > xmax {
                    xmax = x;
                }
                if y < ymin {
                    ymin = y;
                }
                if y > ymax {
                    ymax = y;
                }
            }
        }
    }
    if xmin > xmax {
        None
    } else {
        Some((
            xmin as u32,
            ymin as u32,
            (xmax - xmin + 1) as u32,
            (ymax - ymin + 1) as u32,
        ))
    }
}

fn pad_rgb565(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    if sw == dw && sh == dh {
        return src.to_vec();
    }
    let mut out = vec![0u8; dw as usize * dh as usize * 2];
    let key = TRANSPARENT_COLOR_16.to_le_bytes();
    for px in out.as_chunks_mut::<2>().0 {
        px.copy_from_slice(&key);
    }
    for y in 0..sh as usize {
        let src_off = y * sw as usize * 2;
        let dst_off = y * dw as usize * 2;
        out[dst_off..dst_off + sw as usize * 2]
            .copy_from_slice(&src[src_off..src_off + sw as usize * 2]);
    }
    out
}

fn rgb565_to_rgba_keyed(rgb565: &[u8], w: u32, h: u32) -> Vec<u8> {
    let n = w as usize * h as usize;
    let mut out = Vec::with_capacity(n * 4);
    for i in 0..n {
        let lo = rgb565[i * 2] as u16;
        let hi = rgb565[i * 2 + 1] as u16;
        let px = lo | (hi << 8);
        if px == TRANSPARENT_COLOR_16 {
            out.extend_from_slice(&[0, 0, 0, 0]);
        } else {
            let r5 = ((px >> 11) & 0x1F) as u8;
            let g6 = ((px >> 5) & 0x3F) as u8;
            let b5 = (px & 0x1F) as u8;
            out.push((r5 << 3) | (r5 >> 2));
            out.push((g6 << 2) | (g6 >> 4));
            out.push((b5 << 3) | (b5 >> 2));
            out.push(0xFF);
        }
    }
    out
}

fn walk_ext(root: PathBuf, ext: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    fn rec(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                rec(&p, ext, out);
            } else if p.extension().and_then(|s| s.to_str()) == Some(ext) {
                out.push(p);
            }
        }
    }
    rec(&root, ext, &mut out);
    out
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn human(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{:.2} MiB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn print_map_header(all_codecs: bool) {
    if all_codecs {
        println!(
            "{:<54} {:>5} {:>5} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}",
            "asset",
            "w",
            "h",
            "orig",
            "png",
            "png-oxi",
            "jxl-ll",
            "jxl-q90",
            "avif-ll",
            "qoi",
            "565+z22",
            "565+z3",
            "rgba+z22",
        );
    } else {
        println!(
            "{:<54} {:>5} {:>5} {:>9} {:>9} {:>9} {:>9} {:>9}",
            "asset", "w", "h", "orig", "png-oxi", "jxl-ll", "jxl-q90", "565+z22",
        );
    }
}

fn print_map_row(label: &str, w: u32, h: u32, original: u64, m: &Metrics, all_codecs: bool) {
    if all_codecs {
        println!(
            "{:<54} {:>5} {:>5} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}",
            truncate(label, 54),
            w,
            h,
            human(original),
            human(m.png_raw),
            human(m.png_opti),
            human(m.jxl_lossless),
            human(m.jxl_q90),
            human(m.avif_lossless),
            human(m.qoi),
            human(m.rgb565_zstd22),
            human(m.rgb565_zstd3),
            human(m.rgba_zstd22),
        );
    } else {
        println!(
            "{:<54} {:>5} {:>5} {:>9} {:>9} {:>9} {:>9} {:>9}",
            truncate(label, 54),
            w,
            h,
            human(original),
            human(m.png_opti),
            human(m.jxl_lossless),
            human(m.jxl_q90),
            human(m.rgb565_zstd22),
        );
    }
}

fn print_sprite_header(all_codecs: bool) {
    if all_codecs {
        println!(
            "{:<54} {:>5} {:>5} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}",
            "asset",
            "w",
            "h",
            "orig",
            "png",
            "png-oxi",
            "jxl-ll",
            "jxl-q90",
            "avif-ll",
            "qoi",
            "565+z22",
            "565+z3",
            "rgba+z22",
            "bound+z",
            "anim-jxl",
            "av1-ll",
        );
    } else {
        println!(
            "{:<54} {:>5} {:>5} {:>9} {:>9} {:>9} {:>9} {:>9}",
            "asset", "w", "h", "orig", "z22-orig", "565+z22", "bound+z", "z22-delt",
        );
    }
}

fn print_sprite_row(label: &str, w: u32, h: u32, original: u64, m: &Metrics, all_codecs: bool) {
    let bound = m
        .bounded_frames_zstd
        .map(human)
        .unwrap_or_else(|| "-".into());
    let z22_orig = m.z22_orig.map(human).unwrap_or_else(|| "-".into());
    let z22_delt = m.z22_delt.map(human).unwrap_or_else(|| "-".into());
    if all_codecs {
        let anim_jxl = m.animated_jxl.map(human).unwrap_or_else(|| "-".into());
        let av1 = m.av1_lossless.map(human).unwrap_or_else(|| "-".into());
        println!(
            "{:<54} {:>5} {:>5} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}",
            truncate(label, 54),
            w,
            h,
            human(original),
            human(m.png_raw),
            human(m.png_opti),
            human(m.jxl_lossless),
            human(m.jxl_q90),
            human(m.avif_lossless),
            human(m.qoi),
            human(m.rgb565_zstd22),
            human(m.rgb565_zstd3),
            human(m.rgba_zstd22),
            bound,
            anim_jxl,
            av1,
        );
    } else {
        println!(
            "{:<54} {:>5} {:>5} {:>9} {:>9} {:>9} {:>9} {:>9}",
            truncate(label, 54),
            w,
            h,
            human(original),
            z22_orig,
            human(m.rgb565_zstd22),
            bound,
            z22_delt,
        );
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}
