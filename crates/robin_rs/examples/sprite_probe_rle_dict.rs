//! Research probe for two follow-up sprite-compression experiments
//! (docs/COMPRESSION.md, "Sprite research" / "Implementation: sprite_codec"):
//!
//! A. `--rle` — RLE bucket context modeling. Character chunks are pure VQ,
//!    but patches/overlays/accessories are RLE sprites (per scanline
//!    `[first_x, last_x, literal RGB565 pixels...]`, 0xFFFF last = empty
//!    line, `dictionary_index == UNMAPPED_DICT`). These live in
//!    `Characters/ACCESSORIES_* | BONUS_* | RELIC_* | TG_*` and in the
//!    `Data/Animations/**/*.rhs` files. Question: does pixel-domain context
//!    modeling beat zstd on this bucket the way tile-domain CM did on
//!    characters? Baselines: zstd-19 and xz -9e of the concatenated packed
//!    streams (`w,h,dict,len,packed` — the probe's corpus blob layout).
//!    Simulation: adaptive PPM over the decoded literal pixels with the chain
//!    (left,above) -> above -> left -> order-0 -> uniform(65536), PPMC
//!    escapes (same shape as `cm_bits`/`ppm_level` in
//!    `sprite_compression_probe.rs`). The RLE control words (first/last per
//!    line) and headers are estimated at their zstd size.
//!
//! B. `--dict` — family-shared dictionaries. Each character owns a
//!    4096-entry dictionary of 4x1 RGB565 tiles. For the Knight01-03,
//!    Guard A00-05, Archer00-05 and Soldier A00-05 families this measures:
//!    exact tile overlap between base and variant dictionaries, near-dup
//!    distances (max per-channel delta in R5G6B5) for the rest, and a
//!    simulated unified family dictionary (base tiles + unmatched variant
//!    tiles appended, exact matches only — lossless). Variant sprites are
//!    re-expressed in unified ids and re-encoded with the real
//!    `sprite_codec` cross-variant coder (base slices remapped too) to see
//!    whether id unification improves cross-variant coding, and how much
//!    dictionary storage sharing saves.
//!
//!   cargo run --release --example sprite_probe_rle_dict -- \
//!       --data-dir datadirs/fullgame_linux --rle --dict
//!
//! Helper functions (`walk_rle`, `decode_raw`, `ppm_level`, `char_frame_ids`,
//! `positional_pairs`, `codec_grids2`) are copied from
//! `sprite_compression_probe.rs`; the context-model codec is vendored in
//! `mod codec` below because `robin_assets::sprite_codec` does not exist on
//! this worktree's branch (see the module comment).
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;

use robin_assets::frame_holder::{FrameHolder, TRANSPARENT_COLOR_16, UNMAPPED_DICT};
use robin_engine::sprite_script::SpriteScriptor;

#[derive(Parser, Debug)]
#[command(
    about = "Sprite research probe: RLE-bucket context modeling + family-shared dictionaries"
)]
struct Cli {
    /// Data directory (the one containing `Data/…` or `DATA/…`).
    #[arg(long, default_value = "datadirs/fullgame_linux")]
    data_dir: PathBuf,

    /// Experiment A: RLE bucket gathering, zstd/xz baselines, pixel-domain
    /// adaptive-PPM simulation.
    #[arg(long)]
    rle: bool,

    /// Experiment B: family-shared dictionary overlap stats + unified-id
    /// cross-variant coding with the real codec.
    #[arg(long)]
    dict: bool,

    /// Restrict --dict to families with this base name (repeatable).
    #[arg(long)]
    family: Vec<String>,
}

// ---------------------------------------------------------------------------
// Shared helpers (copied from sprite_compression_probe.rs)
// ---------------------------------------------------------------------------

fn data_subdir(data_dir: &Path, sub: &str) -> Result<PathBuf> {
    for case in ["Data", "DATA"] {
        let p = data_dir.join(case).join(sub);
        if p.is_dir() {
            return Ok(p);
        }
    }
    bail!("no {sub} under {}/(Data|DATA)", data_dir.display())
}

/// All frame ids referenced by one RHS file, in script order (with dups) and
/// as a sorted-deduped bank-order list.
fn rhs_frame_ids(path: &Path) -> Result<(Vec<u32>, Vec<u32>)> {
    let (_sig, profiles) = SpriteScriptor::load_all_profiles(path.to_str().unwrap())
        .map_err(|e| anyhow!("load rhs {}: {e}", path.display()))?;
    let mut script_order = Vec::new();
    for (_p, info) in &profiles {
        for s in info.scripts.iter() {
            script_order.extend_from_slice(&s.frame_ids);
        }
    }
    let mut bank_order = script_order.clone();
    bank_order.sort_unstable();
    bank_order.dedup();
    Ok((script_order, bank_order))
}

fn char_frame_ids(data_dir: &Path, name: &str) -> Result<(Vec<u32>, Vec<u32>)> {
    let chars = data_subdir(data_dir, "Characters")?;
    rhs_frame_ids(&chars.join(format!("{name}.rhs")))
}

fn positional_pairs(data_dir: &Path, a: &str, b: &str) -> Result<Vec<(u32, u32)>> {
    let (sa, _) = char_frame_ids(data_dir, a)?;
    let (sb, _) = char_frame_ids(data_dir, b)?;
    let mut pairs: Vec<(u32, u32)> = sa.iter().copied().zip(sb.iter().copied()).collect();
    pairs.sort_unstable();
    pairs.dedup();
    Ok(pairs)
}

/// Walk one RLE sprite's packed words, calling `ctl(first, size)` per
/// scanline and `lit(&[u16])` for each scanline's literal pixels.
/// Returns words consumed.
fn walk_rle(
    packed: &[u16],
    height: usize,
    mut ctl: impl FnMut(u16, u16),
    mut lit: impl FnMut(&[u16]),
) -> usize {
    let mut p = 0;
    for _y in 0..height {
        let first = packed[p];
        let size = packed[p + 1];
        p += 2;
        ctl(first, size);
        if size != 0xFFFF {
            let run = (size + 1 - first) as usize;
            lit(&packed[p..p + run]);
            p += run;
        }
    }
    p
}

/// One PPM escape-chain step against a single context's symbol counts.
/// Returns the bits paid at this level; sets `coded` once the symbol is
/// actually coded (counts update at every level regardless).
fn ppm_level(m: &mut HashMap<u16, u32>, x: u16, coded: &mut bool) -> f64 {
    let mut bits = 0.0f64;
    if !*coded {
        let total: u64 = m.values().map(|&c| c as u64).sum();
        let distinct = m.len() as u64;
        if let Some(&c) = m.get(&x) {
            bits -= (c as f64 / (total + distinct) as f64).log2();
            *coded = true;
        } else if total > 0 {
            bits -= (distinct as f64 / (total + distinct) as f64).log2();
        }
    }
    *m.entry(x).or_default() += 1;
    bits
}

fn zstd19_len(data: &[u8]) -> Result<u64> {
    Ok(zstd::stream::encode_all(data, 19)?.len() as u64)
}

/// xz -9e via the CLI (same tool/version as the research sweeps). Uses a
/// per-pid temp file so parallel worktree agents cannot clobber each other.
fn xz9e_len(data: &[u8]) -> Result<u64> {
    let path = std::env::temp_dir().join(format!(
        "sprite_probe_rle_dict_{}_{:p}.bin",
        std::process::id(),
        data.as_ptr()
    ));
    fs::write(&path, data).with_context(|| format!("write {}", path.display()))?;
    let out = Command::new("xz")
        .args(["-9e", "-T1", "-c"])
        .arg(&path)
        .output()
        .context("run xz -9e")?;
    fs::remove_file(&path).ok();
    if !out.status.success() {
        bail!("xz -9e failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(out.stdout.len() as u64)
}

fn push_u16s(out: &mut Vec<u8>, words: impl IntoIterator<Item = u16>) {
    for w in words {
        out.extend_from_slice(&w.to_le_bytes());
    }
}

// ---------------------------------------------------------------------------
// Experiment A — RLE bucket
// ---------------------------------------------------------------------------

fn rhs_files_with_prefixes(dir: &Path, prefixes: &[&str]) -> Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|e| Some(e.ok()?.path()))
        .filter(|p| {
            p.extension().is_some_and(|e| e == "rhs")
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| prefixes.iter().any(|pre| n.starts_with(pre)))
        })
        .collect();
    files.sort();
    Ok(files)
}

fn rhs_files_recursive(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|e| Some(e.ok()?.path()))
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            rhs_files_recursive(&p, out)?;
        } else if p.extension().is_some_and(|e| e == "rhs") {
            out.push(p);
        }
    }
    Ok(())
}

/// Load every file's frame ids; each bank id is claimed by the first group
/// that references it. Returns (unique ids, total refs, failed files).
fn gather_group(
    holder: &FrameHolder,
    files: &[PathBuf],
    claimed: &mut HashSet<u32>,
) -> (Vec<u32>, u64, Vec<String>) {
    let mut ids = Vec::new();
    let mut refs = 0u64;
    let mut failed = Vec::new();
    for f in files {
        match rhs_frame_ids(f) {
            Ok((script_order, bank_order)) => {
                refs += script_order.len() as u64;
                for &id in &bank_order {
                    if id as usize >= holder.num_sprites() {
                        eprintln!("# {}: frame id {id} outside bank, skipped", f.display());
                        continue;
                    }
                    if claimed.insert(id) {
                        ids.push(id);
                    }
                }
            }
            Err(e) => {
                eprintln!("# {e}");
                failed.push(f.file_name().unwrap().to_string_lossy().into_owned());
            }
        }
    }
    ids.sort_unstable();
    (ids, refs, failed)
}

/// Adaptive PPM over the decoded literal pixels of RLE sprites — every
/// stored literal word is coded, including the few literals that carry the
/// transparent-key value inside a run (skipping those would silently drop
/// payload words the real stream has to ship).
/// Chain: (left,above) -> above -> left -> order-0 -> uniform(65536).
/// Contexts read the decoded canvas; transparent positions and edges read
/// TRANSPARENT_COLOR_16, which doubles as the sentinel.
/// Returns (literal pixels, bits, order-2 context count).
fn cm_rle_pixels(holder: &FrameHolder, ids: &[u32]) -> (u64, f64, usize) {
    let mut ctx2: HashMap<u32, HashMap<u16, u32>> = HashMap::new();
    let mut ctx_a: HashMap<u16, HashMap<u16, u32>> = HashMap::new();
    let mut ctx_l: HashMap<u16, HashMap<u16, u32>> = HashMap::new();
    let mut ctx0: HashMap<u16, u32> = HashMap::new();
    let mut bits = 0.0f64;
    let mut n_px = 0u64;
    for &id in ids {
        let s = &holder.sprites()[id as usize];
        let (w, h) = (s.width as usize, s.height as usize);
        if w == 0 || h == 0 {
            continue;
        }
        let Some(pd) = holder.packed_data(id) else {
            continue;
        };
        let mut dst = vec![TRANSPARENT_COLOR_16; w * h];
        let mut p = 0;
        for y in 0..h {
            let first = pd[p];
            let size = pd[p + 1];
            p += 2;
            if size == 0xFFFF {
                continue;
            }
            let run = (size + 1 - first) as usize;
            for k in 0..run {
                let x = first as usize + k;
                let i = y * w + x;
                let c = pd[p + k];
                let left = if x > 0 {
                    dst[i - 1]
                } else {
                    TRANSPARENT_COLOR_16
                };
                let above = if y > 0 {
                    dst[i - w]
                } else {
                    TRANSPARENT_COLOR_16
                };
                let key2 = ((left as u32) << 16) | above as u32;
                n_px += 1;
                let mut coded = false;
                bits += ppm_level(ctx2.entry(key2).or_default(), c, &mut coded);
                bits += ppm_level(ctx_a.entry(above).or_default(), c, &mut coded);
                bits += ppm_level(ctx_l.entry(left).or_default(), c, &mut coded);
                bits += ppm_level(&mut ctx0, c, &mut coded);
                if !coded {
                    bits += 16.0; // uniform over the full u16 alphabet
                }
                dst[i] = c;
            }
            p += run;
        }
    }
    (n_px, bits, ctx2.len())
}

fn analyze_rle_bucket(holder: &FrameHolder, label: &str, ids: &[u32]) -> Result<()> {
    let mut rle_ids: Vec<u32> = Vec::new();
    let (mut n_vq, mut vq_bytes, mut n_skip) = (0u64, 0u64, 0u64);
    for &id in ids {
        let s = &holder.sprites()[id as usize];
        let Some(pd) = holder.packed_data(id) else {
            n_skip += 1;
            continue;
        };
        if s.width == 0 || s.height == 0 {
            n_skip += 1;
            continue;
        }
        if s.dictionary_index == UNMAPPED_DICT {
            rle_ids.push(id);
        } else {
            n_vq += 1;
            vq_bytes += 2 * pd.len() as u64;
        }
    }

    // Corpus-blob baseline (w,h,dict,len,packed per sprite) + streams.
    let mut blob = Vec::new();
    let mut hdr = Vec::new();
    let mut firsts: Vec<u16> = Vec::new();
    let mut sizes: Vec<u16> = Vec::new();
    let (mut ctl_words, mut lit_words) = (0u64, 0u64);
    let mut colors: HashSet<u16> = HashSet::new();
    let mut walk_mismatch = 0u64;
    let mut canvas_px = 0u64;
    for &id in &rle_ids {
        let s = &holder.sprites()[id as usize];
        let pd = holder.packed_data(id).unwrap();
        blob.extend_from_slice(&s.width.to_le_bytes());
        blob.extend_from_slice(&s.height.to_le_bytes());
        blob.extend_from_slice(&s.dictionary_index.to_le_bytes());
        blob.extend_from_slice(&(pd.len() as u32).to_le_bytes());
        blob.extend_from_slice(bytemuck::cast_slice::<u16, u8>(pd));
        hdr.extend_from_slice(&s.width.to_le_bytes());
        hdr.extend_from_slice(&s.height.to_le_bytes());
        hdr.extend_from_slice(&(pd.len() as u32).to_le_bytes());
        canvas_px += s.width as u64 * s.height as u64;
        let used = walk_rle(
            pd,
            s.height as usize,
            |f, sz| {
                firsts.push(f);
                sizes.push(sz);
                ctl_words += 2;
            },
            |lit| {
                lit_words += lit.len() as u64;
                colors.extend(lit.iter().copied());
            },
        );
        if used != pd.len() {
            walk_mismatch += 1;
        }
    }
    if walk_mismatch > 0 {
        eprintln!("# {label}: {walk_mismatch} sprites with trailing packed words after RLE walk");
    }

    println!("## rle bucket [{label}]");
    println!(
        "  unique bank sprites:  {} ({} RLE, {n_vq} VQ [{} packed B, excluded], {n_skip} empty/missing)",
        ids.len(),
        rle_ids.len(),
        vq_bytes
    );
    println!(
        "  raw packed blob:      {:>10} B  (ctl {ctl_words} + lit {lit_words} words; canvas {canvas_px} px, opaque {:.1}%, {} colors)",
        blob.len(),
        100.0 * lit_words as f64 / canvas_px.max(1) as f64,
        colors.len()
    );
    if rle_ids.is_empty() {
        println!("  (empty bucket, skipping compression)");
        return Ok(());
    }

    let z_blob = zstd19_len(&blob)?;
    let x_blob = xz9e_len(&blob)?;
    println!(
        "  zstd-19:              {z_blob:>10} B  ({:.2}x vs raw)",
        blob.len() as f64 / z_blob as f64
    );
    println!(
        "  xz -9e:               {x_blob:>10} B  ({:.2}x vs raw)",
        blob.len() as f64 / x_blob as f64
    );

    // Control + header stream estimates (zstd; they are small).
    let mut ctl_inter = Vec::with_capacity(firsts.len() * 4);
    for (&f, &s) in firsts.iter().zip(sizes.iter()) {
        ctl_inter.extend_from_slice(&f.to_le_bytes());
        ctl_inter.extend_from_slice(&s.to_le_bytes());
    }
    let mut first_bytes = Vec::with_capacity(firsts.len() * 2);
    push_u16s(&mut first_bytes, firsts.iter().copied());
    let mut size_bytes = Vec::with_capacity(sizes.len() * 2);
    push_u16s(&mut size_bytes, sizes.iter().copied());
    let z_ctl_inter = zstd19_len(&ctl_inter)?;
    let z_first = zstd19_len(&first_bytes)?;
    let z_size = zstd19_len(&size_bytes)?;
    let ctl_est = z_ctl_inter.min(z_first + z_size);
    let z_hdr = zstd19_len(&hdr)?;

    // Pixel-domain adaptive PPM.
    let t0 = Instant::now();
    let (n_px, bits, nctx2) = cm_rle_pixels(holder, &rle_ids);
    let cm_px = (bits / 8.0).ceil() as u64;
    println!(
        "  cm pixels (PPM sim):  {cm_px:>10} B  ({:.3} bits/px over {n_px} literal px, {nctx2} order-2 contexts, {:.1}s)",
        bits / n_px.max(1) as f64,
        t0.elapsed().as_secs_f64()
    );
    println!(
        "  ctl zstd-19:          {ctl_est:>10} B  (interleaved {z_ctl_inter} vs split first {z_first} + size {z_size})"
    );
    println!("  hdr zstd-19:          {z_hdr:>10} B");
    let cm_total = cm_px + ctl_est + z_hdr;
    println!(
        "  CM TOTAL (px+ctl+hdr) {cm_total:>10} B  ({:+.1}% vs zstd19, {:+.1}% vs xz9e)",
        100.0 * (cm_total as f64 - z_blob as f64) / z_blob as f64,
        100.0 * (cm_total as f64 - x_blob as f64) / x_blob as f64
    );
    Ok(())
}

fn exp_rle(holder: &FrameHolder, data_dir: &Path) -> Result<()> {
    let chars_dir = data_subdir(data_dir, "Characters")?;
    let acc_files =
        rhs_files_with_prefixes(&chars_dir, &["ACCESSORIES_", "BONUS_", "RELIC_", "TG_"])?;
    let mut anim_files = Vec::new();
    match data_subdir(data_dir, "Animations") {
        Ok(anim_dir) => rhs_files_recursive(&anim_dir, &mut anim_files)?,
        Err(e) => eprintln!("# no Animations dir: {e}"),
    }
    println!(
        "## rle gather: {} accessory/bonus/relic/TG rhs, {} animation rhs",
        acc_files.len(),
        anim_files.len()
    );

    let mut claimed: HashSet<u32> = HashSet::new();
    let (ids_a, refs_a, fail_a) = gather_group(holder, &acc_files, &mut claimed);
    let (ids_b, refs_b, fail_b) = gather_group(holder, &anim_files, &mut claimed);
    println!(
        "  accessories: {} unique bank ids ({refs_a} script refs, {} files failed{})",
        ids_a.len(),
        fail_a.len(),
        if fail_a.is_empty() {
            String::new()
        } else {
            format!(": {}", fail_a.join(", "))
        }
    );
    println!(
        "  animations:  {} unique bank ids ({refs_b} script refs, {} files failed{})",
        ids_b.len(),
        fail_b.len(),
        if fail_b.is_empty() {
            String::new()
        } else {
            format!(": {}", fail_b.join(", "))
        }
    );

    let mut union: Vec<u32> = ids_a.iter().chain(ids_b.iter()).copied().collect();
    union.sort_unstable();
    analyze_rle_bucket(holder, "accessories/bonus/relic/TG", &ids_a)?;
    analyze_rle_bucket(holder, "animations", &ids_b)?;
    analyze_rle_bucket(holder, "UNION", &union)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Experiment B — family-shared dictionaries
// ---------------------------------------------------------------------------

const FAMILIES: &[(&str, &[&str])] = &[
    ("Knight01", &["Knight02", "Knight03"]),
    (
        "Guard A00",
        &[
            "Guard A01",
            "Guard A02",
            "Guard A03",
            "Guard A04",
            "Guard A05",
        ],
    ),
    (
        "Archer00",
        &["Archer01", "Archer02", "Archer03", "Archer04", "Archer05"],
    ),
    (
        "Soldier A00",
        &[
            "Soldier A01",
            "Soldier A02",
            "Soldier A03",
            "Soldier A04",
            "Soldier A05",
        ],
    ),
];

/// Published per-character-id cross-variant sizes (COMPRESSION.md), used as
/// a sanity check that the vendored codec reproduces the real one.
const PUBLISHED_CROSS: &[(&str, u64)] = &[
    ("Knight02", 977_814),
    ("Guard A01", 466_297),
    ("Archer01", 490_056),
];

type Tile = [u16; 4];

/// A character's single dictionary as raw 4-pixel tiles.
fn char_dict_tiles(holder: &FrameHolder, data_dir: &Path, name: &str) -> Result<Vec<Tile>> {
    let (_, ids) = char_frame_ids(data_dir, name)?;
    let mut dict_ids: HashSet<u16> = HashSet::new();
    for &id in &ids {
        let s = &holder.sprites()[id as usize];
        if s.dictionary_index != UNMAPPED_DICT && holder.packed_data(id).is_some() {
            dict_ids.insert(s.dictionary_index);
        }
    }
    if dict_ids.len() != 1 {
        bail!("{name}: expected exactly one dictionary, found {dict_ids:?}");
    }
    let di = *dict_ids.iter().next().unwrap();
    let dict = holder
        .dictionary(di)
        .ok_or_else(|| anyhow!("missing dictionary {di}"))?;
    Ok((0..dict.num_entries())
        .map(|i| dict.lookup_pixels(i).try_into().unwrap())
        .collect())
}

/// Max per-channel delta over the 4 pixels in R5G6B5 space, early-exiting
/// once `cap` is reached (only smaller results matter to the caller).
#[inline]
fn tile_dist_capped(a: &Tile, b: &Tile, cap: u32) -> u32 {
    let mut m = 0u32;
    for i in 0..4 {
        let (pa, pb) = (a[i], b[i]);
        let dr = ((pa >> 11) as i32 - (pb >> 11) as i32).unsigned_abs();
        let dg = (((pa >> 5) & 0x3F) as i32 - ((pb >> 5) & 0x3F) as i32).unsigned_abs();
        let db = ((pa & 0x1F) as i32 - (pb & 0x1F) as i32).unsigned_abs();
        m = m.max(dr).max(dg).max(db);
        if m >= cap {
            return m;
        }
    }
    m
}

/// Gather variant B's VQ grids with aligned base-A slices for cross-variant
/// coding (copied from the probe's codec_grids2).
/// Returns (dims, indices, bases, alphabet, unbased count).
#[allow(clippy::type_complexity)]
fn codec_grids2<'h>(
    holder: &'h FrameHolder,
    pairs: &[(u32, u32)],
) -> Result<(
    Vec<(u16, u16)>,
    Vec<&'h [u16]>,
    Vec<Option<&'h [u16]>>,
    u16,
    usize,
)> {
    let mut dims = Vec::new();
    let mut slices: Vec<&[u16]> = Vec::new();
    let mut bases: Vec<Option<&[u16]>> = Vec::new();
    let mut alphabet: u16 = 0;
    let mut unbased = 0usize;
    for &(ia, ib) in pairs {
        let (spa, spb) = (
            &holder.sprites()[ia as usize],
            &holder.sprites()[ib as usize],
        );
        if spb.dictionary_index == UNMAPPED_DICT {
            continue;
        }
        let Some(pb) = holder.packed_data(ib) else {
            continue;
        };
        let dict = holder
            .dictionary(spb.dictionary_index)
            .ok_or_else(|| anyhow!("missing dictionary {}", spb.dictionary_index))?;
        alphabet = alphabet.max(dict.num_entries());
        let base = holder.packed_data(ia).filter(|pa| {
            spa.dictionary_index != UNMAPPED_DICT
                && (spa.width, spa.height) == (spb.width, spb.height)
                && pa.len() == pb.len()
        });
        if base.is_none() {
            unbased += 1;
        }
        dims.push((spb.width / 4, spb.height));
        slices.push(pb);
        bases.push(base);
    }
    Ok((dims, slices, bases, alphabet, unbased))
}

struct VariantDict {
    name: String,
    tiles: Vec<Tile>,
    /// variant dictionary id -> unified id (exact content matches only).
    uni_map: Vec<u16>,
    exact_base: usize,
    exact_base_excl_transkey: usize,
    tiles_with_key_px: usize,
    exact_among_keyless: usize,
    keyless_total: usize,
    appended: usize,
    /// Near-dup histogram over non-exact tiles: dist 0 / 1 / 2 / 3+.
    neardup: [u64; 4],
}

fn family(holder: &FrameHolder, data_dir: &Path, base: &str, variants: &[&str]) -> Result<()> {
    let btiles = char_dict_tiles(holder, data_dir, base)?;
    let trans_tile: Tile = [TRANSPARENT_COLOR_16; 4];

    // Content map over the growing unified dictionary; first occurrence wins.
    let mut map: HashMap<Tile, u16> = HashMap::with_capacity(btiles.len() * 2);
    let mut base_dups = 0usize;
    for (i, t) in btiles.iter().enumerate() {
        if map.contains_key(t) {
            base_dups += 1;
        } else {
            map.insert(*t, i as u16);
        }
    }
    // The base ships its tiles as unified ids 0..N verbatim; its grids are
    // canonicalised through the content map so duplicated tiles collapse to
    // one id (deterministic on the decode side from the dictionaries alone).
    let base_canon: Vec<u16> = btiles.iter().map(|t| map[t]).collect();
    let base_set: HashMap<Tile, u16> = map.clone(); // frozen for overlap stats

    println!(
        "## family {base}: {} base tiles ({base_dups} internal duplicates)",
        btiles.len()
    );

    let mut uni_tiles: Vec<Tile> = btiles.clone();
    let mut vds: Vec<VariantDict> = Vec::new();
    for vname in variants {
        let vtiles = char_dict_tiles(holder, data_dir, vname)?;
        let mut vd = VariantDict {
            name: vname.to_string(),
            uni_map: vec![0u16; vtiles.len()],
            exact_base: 0,
            exact_base_excl_transkey: 0,
            tiles_with_key_px: 0,
            exact_among_keyless: 0,
            keyless_total: 0,
            appended: 0,
            neardup: [0u64; 4],
            tiles: vtiles,
        };
        for (v, t) in vd.tiles.iter().enumerate() {
            let has_key = t.iter().any(|&p| p == TRANSPARENT_COLOR_16);
            if has_key {
                vd.tiles_with_key_px += 1;
            } else {
                vd.keyless_total += 1;
            }
            let exact = base_set.contains_key(t);
            if exact {
                vd.exact_base += 1;
                if *t != trans_tile {
                    vd.exact_base_excl_transkey += 1;
                }
                if !has_key {
                    vd.exact_among_keyless += 1;
                }
            } else {
                // Near-dup distance to the closest base tile.
                let mut best = u32::MAX;
                for b in &btiles {
                    let d = tile_dist_capped(t, b, best);
                    if d < best {
                        best = d;
                        if best <= 1 {
                            break; // 0 is impossible for a non-exact tile
                        }
                    }
                }
                vd.neardup[best.min(3) as usize] += 1;
            }
            // Unified id: exact match anywhere in the unified dict so far
            // (base + earlier variants), else append.
            match map.get(t) {
                Some(&u) => vd.uni_map[v] = u,
                None => {
                    let u = u16::try_from(uni_tiles.len())
                        .map_err(|_| anyhow!("unified dictionary exceeds u16"))?;
                    map.insert(*t, u);
                    uni_tiles.push(*t);
                    vd.uni_map[v] = u;
                    vd.appended += 1;
                }
            }
        }
        vds.push(vd);
    }

    for vd in &vds {
        let non_exact = vd.tiles.len() - vd.exact_base;
        println!(
            "  {}: exact-in-base {}/{} ({:.1}%)  [excl transparent-key tile: {}; keyless tiles: {}/{} exact]",
            vd.name,
            vd.exact_base,
            vd.tiles.len(),
            100.0 * vd.exact_base as f64 / vd.tiles.len() as f64,
            vd.exact_base_excl_transkey,
            vd.exact_among_keyless,
            vd.keyless_total,
        );
        println!(
            "      near-dup dist of {non_exact} non-exact: 0:{} 1:{} 2:{} 3+:{}  | appended to unified: {}",
            vd.neardup[0], vd.neardup[1], vd.neardup[2], vd.neardup[3], vd.appended
        );
    }

    let members = 1 + variants.len();
    let cur_dict_bytes =
        vds.iter().map(|v| v.tiles.len() as u64 * 8).sum::<u64>() + btiles.len() as u64 * 8;
    let uni_dict_bytes = uni_tiles.len() as u64 * 8;
    println!(
        "  unified dictionary: {} tiles ({members} members) — storage {} B -> {} B ({:.1}% saved)",
        uni_tiles.len(),
        cur_dict_bytes,
        uni_dict_bytes,
        100.0 * (cur_dict_bytes - uni_dict_bytes) as f64 / cur_dict_bytes as f64
    );

    // Cross-variant coding, per-character ids vs unified ids, real codec.
    let uni_alphabet = u16::try_from(uni_tiles.len()).expect("checked above");
    let mut verified_roundtrip = false;
    for vd in &vds {
        let pairs = positional_pairs(data_dir, base, &vd.name)?;
        let (dims, slices, bases, alphabet, unbased) = codec_grids2(holder, &pairs)?;
        let n_tiles: u64 = slices.iter().map(|s| s.len() as u64).sum();

        let grids_cur: Vec<codec::SpriteGrid> = dims
            .iter()
            .zip(slices.iter())
            .map(|(&(c, r), &s)| codec::SpriteGrid {
                cols: c,
                rows: r,
                indices: s,
            })
            .collect();
        let t0 = Instant::now();
        let blob_cur = codec::encode_grids(alphabet, &grids_cur, Some(&bases))?;
        let t_cur = t0.elapsed();

        // Remap to unified ids: variant grids through uni_map, base context
        // grids through base_canon. Both stay lossless (exact content).
        let v_uni: Vec<Vec<u16>> = slices
            .iter()
            .map(|s| s.iter().map(|&x| vd.uni_map[x as usize]).collect())
            .collect();
        let b_uni: Vec<Option<Vec<u16>>> = bases
            .iter()
            .map(|o| o.map(|s| s.iter().map(|&x| base_canon[x as usize]).collect()))
            .collect();
        let grids_uni: Vec<codec::SpriteGrid> = dims
            .iter()
            .zip(v_uni.iter())
            .map(|(&(c, r), s)| codec::SpriteGrid {
                cols: c,
                rows: r,
                indices: s,
            })
            .collect();
        let b_uni_refs: Vec<Option<&[u16]>> = b_uni.iter().map(|o| o.as_deref()).collect();
        let t0 = Instant::now();
        let blob_uni = codec::encode_grids(uni_alphabet, &grids_uni, Some(&b_uni_refs))?;
        let t_uni = t0.elapsed();

        let roundtripped = if !verified_roundtrip {
            // Once per family: prove the unified-id encode decodes bit-exact
            // (exercises the alphabet-beyond-4096 path).
            let decoded = codec::decode_grids(uni_alphabet, &dims, Some(&b_uni_refs), &blob_uni)?;
            for (i, (d, s)) in decoded.iter().zip(v_uni.iter()).enumerate() {
                if d != s {
                    bail!("{base}:{}: unified roundtrip mismatch at grid {i}", vd.name);
                }
            }
            verified_roundtrip = true;
            true
        } else {
            false
        };

        let published = PUBLISHED_CROSS
            .iter()
            .find(|(n, _)| *n == vd.name)
            .map(|&(_, b)| format!("  (published {b})"))
            .unwrap_or_default();
        println!(
            "  {} vs {base}: {} sprites ({unbased} unbased), {n_tiles} tiles",
            vd.name,
            slices.len()
        );
        println!(
            "      per-char ids: {:>9} B ({:.3} bits/tile, {:.0}s){published}",
            blob_cur.len(),
            blob_cur.len() as f64 * 8.0 / n_tiles as f64,
            t_cur.as_secs_f64()
        );
        println!(
            "      unified ids:  {:>9} B ({:.3} bits/tile, {:.0}s)  [{:+.2}% vs per-char{}]",
            blob_uni.len(),
            blob_uni.len() as f64 * 8.0 / n_tiles as f64,
            t_uni.as_secs_f64(),
            100.0 * (blob_uni.len() as f64 - blob_cur.len() as f64) / blob_cur.len() as f64,
            if roundtripped { ", roundtrip OK" } else { "" }
        );
    }
    Ok(())
}

fn exp_dict(holder: &FrameHolder, data_dir: &Path, only: &[String]) -> Result<()> {
    for (base, variants) in FAMILIES {
        if !only.is_empty() && !only.iter().any(|o| o == base) {
            continue;
        }
        family(holder, data_dir, base, variants)?;
    }
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if !cli.rle && !cli.dict {
        bail!("nothing to do: pass --rle and/or --dict");
    }
    let data_dir_s = cli
        .data_dir
        .to_str()
        .ok_or_else(|| anyhow!("non-UTF8 data dir"))?;
    let mut holder = FrameHolder::new();
    holder
        .initialize_sprite_bank(data_dir_s)
        .context("initialize_sprite_bank")?;
    eprintln!(
        "# bank loaded: {} sprites, {} dictionaries",
        holder.num_sprites(),
        holder.dictionaries().len()
    );
    if cli.rle {
        exp_rle(&holder, &cli.data_dir)?;
    }
    if cli.dict {
        exp_dict(&holder, &cli.data_dir, &cli.family)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Vendored context-model codec
// ---------------------------------------------------------------------------

/// Vendored copy of `robin_assets::sprite_codec` (main repo). That module
/// does not exist on this worktree's branch and the file is owned by a
/// concurrent agent, so this research example carries its own snapshot
/// rather than modifying `robin_assets`. The ONLY change: the `foldhash`
/// hash-map alias is replaced with `std::collections::HashMap` (the hasher
/// affects speed only — context maps are keyed lookups, never iterated — so
/// coded output stays byte-identical to the real codec).
mod codec {
    use anyhow::{Result, anyhow};
    use std::collections::HashMap;

    // -----------------------------------------------------------------------
    // Range coder (LZMA-style, 32-bit range, byte renormalisation)
    // -----------------------------------------------------------------------

    const RC_TOP: u32 = 1 << 24;

    struct RangeEncoder {
        low: u64,
        range: u32,
        cache: u8,
        cache_size: u64,
        out: Vec<u8>,
    }

    impl RangeEncoder {
        fn new() -> Self {
            Self {
                low: 0,
                range: u32::MAX,
                cache: 0,
                cache_size: 1,
                out: Vec::new(),
            }
        }

        /// Encode a symbol occupying `[start, start+size)` of `total`.
        fn encode(&mut self, start: u32, size: u32, total: u32) {
            debug_assert!(size > 0 && start + size <= total);
            let r = self.range / total;
            self.low += start as u64 * r as u64;
            self.range = r * size;
            while self.range < RC_TOP {
                self.shift_low();
                self.range <<= 8;
            }
        }

        fn shift_low(&mut self) {
            if self.low < 0xFF00_0000 || self.low > 0xFFFF_FFFF {
                let carry = (self.low >> 32) as u8;
                self.out.push(self.cache.wrapping_add(carry));
                for _ in 1..self.cache_size {
                    self.out.push(0xFFu8.wrapping_add(carry));
                }
                self.cache = (self.low >> 24) as u8;
                self.cache_size = 0;
            }
            self.cache_size += 1;
            self.low = (self.low << 8) & 0xFFFF_FFFF;
        }

        fn finish(mut self) -> Vec<u8> {
            for _ in 0..5 {
                self.shift_low();
            }
            self.out
        }
    }

    struct RangeDecoder<'a> {
        range: u32,
        code: u32,
        input: &'a [u8],
        pos: usize,
    }

    impl<'a> RangeDecoder<'a> {
        fn new(input: &'a [u8]) -> Self {
            let mut d = Self {
                range: u32::MAX,
                code: 0,
                input,
                pos: 1, // first byte is the encoder's initial cache byte (0)
            };
            for _ in 0..4 {
                d.code = (d.code << 8) | d.next_byte() as u32;
            }
            d
        }

        fn next_byte(&mut self) -> u8 {
            let b = self.input.get(self.pos).copied().unwrap_or(0);
            self.pos += 1;
            b
        }

        /// Returns a value in `[0, total)`; caller finds the symbol whose
        /// interval contains it and confirms with `commit`.
        fn decode_target(&mut self, total: u32) -> u32 {
            let r = self.range / total;
            (self.code / r).min(total - 1)
        }

        fn commit(&mut self, start: u32, size: u32, total: u32) {
            let r = self.range / total;
            self.code -= start * r;
            self.range = r * size;
            while self.range < RC_TOP {
                self.code = (self.code << 8) | self.next_byte() as u32;
                self.range <<= 8;
            }
        }
    }

    // -----------------------------------------------------------------------
    // PPM contexts
    // -----------------------------------------------------------------------

    /// Halve a context's counts once its coder total reaches this.
    const CTX_HALVE_LIMIT: u32 = 1 << 14;

    /// Count increment per observation.
    const BUMP: u16 = 1;

    /// One adaptive context: seen symbols with counts, in insertion order.
    #[derive(Default)]
    struct Ctx {
        syms: Vec<(u16, u16)>,
        sum: u32,
    }

    enum CtxCode {
        /// (cum, freq, total) of the coded symbol.
        Sym(u32, u32, u32),
        /// (cum, freq, total) of the escape.
        Escape(u32, u32, u32),
        /// Context was empty: nothing is coded (probability-1 escape).
        Empty,
    }

    /// Escape weight for a context with `distinct` non-excluded symbols: PPMC.
    #[inline]
    fn escape_weight(distinct: u32) -> u32 {
        distinct
    }

    impl Ctx {
        fn total(&self) -> u32 {
            self.sum + self.syms.len() as u32
        }

        /// Locate `x` for encoding, ignoring symbols in `excl`.
        fn code_for(&self, x: u16, excl: &Excl) -> CtxCode {
            if excl.is_empty() {
                if self.syms.is_empty() {
                    return CtxCode::Empty;
                }
                let total = self.total();
                let mut cum = 0u32;
                for &(s, c) in &self.syms {
                    if s == x {
                        return CtxCode::Sym(cum, c as u32, total);
                    }
                    cum += c as u32;
                }
                return CtxCode::Escape(self.sum, escape_weight(self.syms.len() as u32), total);
            }
            let mut cum = 0u32;
            let mut found: Option<(u32, u32)> = None;
            let mut distinct = 0u32;
            for &(s, c) in &self.syms {
                if excl.contains(s) {
                    continue;
                }
                if s == x {
                    found = Some((cum, c as u32));
                }
                cum += c as u32;
                distinct += 1;
            }
            if distinct == 0 {
                return CtxCode::Empty;
            }
            let total = cum + escape_weight(distinct);
            match found {
                Some((start, freq)) => CtxCode::Sym(start, freq, total),
                None => CtxCode::Escape(cum, escape_weight(distinct), total),
            }
        }

        /// Append this context's non-excluded symbols to the exclusion list.
        fn exclude_into(&self, excl: &mut Excl) {
            for &(s, _) in &self.syms {
                excl.insert(s);
            }
        }

        fn bump(&mut self, x: u16) {
            match self.syms.iter().position(|&(s, _)| s == x) {
                Some(mut i) => {
                    self.syms[i].1 += BUMP;
                    while i > 0 && self.syms[i].1 > self.syms[i - 1].1 {
                        self.syms.swap(i, i - 1);
                        i -= 1;
                    }
                }
                None => self.syms.push((x, BUMP)),
            }
            self.sum += BUMP as u32;
            if self.total() >= CTX_HALVE_LIMIT {
                self.sum = 0;
                for (_, c) in &mut self.syms {
                    *c = (*c / 2).max(1);
                    self.sum += *c as u32;
                }
            }
        }
    }

    /// Sentinel context symbol for "no neighbor" (grid edge / no base).
    const EDGE: u16 = 0xFFFF;

    /// Per-symbol exclusion set as a generation-stamped array.
    struct Excl {
        stamp: Vec<u32>,
        generation: u32,
        inserted: u32,
    }

    impl Excl {
        fn new(alphabet: u16) -> Self {
            Self {
                stamp: vec![0; alphabet as usize],
                generation: 0,
                inserted: 0,
            }
        }

        fn begin(&mut self) {
            self.generation += 1;
            self.inserted = 0;
            if self.generation == u32::MAX {
                self.stamp.fill(0);
                self.generation = 1;
            }
        }

        #[inline]
        fn is_empty(&self) -> bool {
            self.inserted == 0
        }

        #[inline]
        fn contains(&self, s: u16) -> bool {
            self.stamp[s as usize] == self.generation
        }

        #[inline]
        fn insert(&mut self, s: u16) {
            self.stamp[s as usize] = self.generation;
            self.inserted += 1;
        }
    }

    struct Model {
        /// Most specific: (primary, second) — (above, left) standalone,
        /// (base tile, above) cross-variant.
        c2: HashMap<u32, Ctx>,
        /// primary alone (the stronger single predictor).
        c1: Vec<Ctx>,
        /// second alone.
        c1b: Vec<Ctx>,
        c0: Ctx,
        alphabet: u32,
        excl: Excl,
        /// Reusable buffer for the exclusion-aware decode path.
        scratch: Vec<(u16, u32)>,
    }

    /// Copy `ctx`'s non-excluded symbols into `out`.
    /// Returns `(sum, total)` of the reduced interval (0, 0 when empty).
    fn fill_scratch(ctx: &Ctx, excl: &Excl, out: &mut Vec<(u16, u32)>) -> (u32, u32) {
        out.clear();
        let mut sum = 0u32;
        for &(s, c) in &ctx.syms {
            if excl.contains(s) {
                continue;
            }
            out.push((s, c as u32));
            sum += c as u32;
        }
        if out.is_empty() {
            (0, 0)
        } else {
            (sum, sum + escape_weight(out.len() as u32))
        }
    }

    impl Model {
        fn new(alphabet: u16) -> Self {
            Self {
                c2: HashMap::default(),
                c1: (0..=alphabet as usize).map(|_| Ctx::default()).collect(),
                c1b: (0..=alphabet as usize).map(|_| Ctx::default()).collect(),
                c0: Ctx::default(),
                alphabet: alphabet as u32,
                excl: Excl::new(alphabet),
                scratch: Vec::new(),
            }
        }

        fn encode_sym(&mut self, enc: &mut RangeEncoder, primary: u16, second: u16, x: u16) {
            let key2 = ((primary as u32) << 16) | second as u32;
            self.excl.begin();
            let excl = &mut self.excl;
            let mut coded = false;
            for ctx in [
                self.c2.entry(key2).or_default(),
                &mut self.c1[(primary as usize).min(self.alphabet as usize)],
                &mut self.c1b[(second as usize).min(self.alphabet as usize)],
                &mut self.c0,
            ] {
                if !coded {
                    match ctx.code_for(x, excl) {
                        CtxCode::Sym(cum, f, t) => {
                            enc.encode(cum, f, t);
                            coded = true;
                        }
                        CtxCode::Escape(cum, f, t) => {
                            enc.encode(cum, f, t);
                            ctx.exclude_into(excl);
                        }
                        CtxCode::Empty => {}
                    }
                }
                ctx.bump(x);
            }
            if !coded {
                enc.encode(x as u32, 1, self.alphabet);
            }
        }

        fn decode_sym(&mut self, dec: &mut RangeDecoder, primary: u16, second: u16) -> u16 {
            let key2 = ((primary as u32) << 16) | second as u32;
            self.excl.begin();
            let excl = &mut self.excl;
            let scratch = &mut self.scratch;
            let mut decoded: Option<u16> = None;
            {
                let chain: [&Ctx; 4] = [
                    self.c2.entry(key2).or_default(),
                    &self.c1[(primary as usize).min(self.alphabet as usize)],
                    &self.c1b[(second as usize).min(self.alphabet as usize)],
                    &self.c0,
                ];
                for ctx in chain {
                    if excl.is_empty() {
                        if ctx.syms.is_empty() {
                            continue;
                        }
                        let total = ctx.total();
                        let target = dec.decode_target(total);
                        if target >= ctx.sum {
                            dec.commit(ctx.sum, escape_weight(ctx.syms.len() as u32), total);
                            ctx.exclude_into(excl);
                            continue;
                        }
                        let mut cum = 0u32;
                        for &(s, c) in &ctx.syms {
                            if target < cum + c as u32 {
                                dec.commit(cum, c as u32, total);
                                decoded = Some(s);
                                break;
                            }
                            cum += c as u32;
                        }
                        debug_assert!(decoded.is_some());
                        break;
                    }
                    let (sum, total) = fill_scratch(ctx, excl, scratch);
                    if total == 0 {
                        continue;
                    }
                    let target = dec.decode_target(total);
                    if target >= sum {
                        dec.commit(sum, escape_weight(scratch.len() as u32), total);
                        for &(s, _) in scratch.iter() {
                            excl.insert(s);
                        }
                        continue;
                    }
                    let mut cum = 0u32;
                    for &(s, c) in scratch.iter() {
                        if target < cum + c {
                            dec.commit(cum, c, total);
                            decoded = Some(s);
                            break;
                        }
                        cum += c;
                    }
                    debug_assert!(decoded.is_some());
                    break;
                }
            }
            let x = match decoded {
                Some(s) => s,
                None => {
                    let target = dec.decode_target(self.alphabet);
                    dec.commit(target, 1, self.alphabet);
                    target as u16
                }
            };
            self.c2.entry(key2).or_default().bump(x);
            self.c1[(primary as usize).min(self.alphabet as usize)].bump(x);
            self.c1b[(second as usize).min(self.alphabet as usize)].bump(x);
            self.c0.bump(x);
            x
        }
    }

    // -----------------------------------------------------------------------
    // Public API
    // -----------------------------------------------------------------------

    /// One VQ sprite's index grid: `cols = width/4`, `rows = height`,
    /// `indices.len() == cols * rows`.
    pub struct SpriteGrid<'a> {
        pub cols: u16,
        pub rows: u16,
        pub indices: &'a [u16],
    }

    /// Encode a sequence of VQ sprite grids. See `robin_assets::sprite_codec`.
    pub fn encode_grids(
        alphabet: u16,
        grids: &[SpriteGrid],
        base: Option<&[Option<&[u16]>]>,
    ) -> Result<Vec<u8>> {
        if let Some(base) = base {
            if base.len() != grids.len() {
                return Err(anyhow!(
                    "base list length {} != grid count {}",
                    base.len(),
                    grids.len()
                ));
            }
        }
        let mut enc = RangeEncoder::new();
        let mut model = Model::new(alphabet);
        for (gi, g) in grids.iter().enumerate() {
            let cols = g.cols as usize;
            if g.indices.len() != cols * g.rows as usize {
                return Err(anyhow!(
                    "grid {gi}: {} indices for {}x{}",
                    g.indices.len(),
                    g.cols,
                    g.rows
                ));
            }
            let b = base.and_then(|b| b[gi]);
            if let Some(b) = b {
                if b.len() != g.indices.len() {
                    return Err(anyhow!("grid {gi}: base length mismatch"));
                }
            }
            for (i, &x) in g.indices.iter().enumerate() {
                if x as u32 >= alphabet as u32 {
                    return Err(anyhow!("grid {gi}: index {x} >= alphabet {alphabet}"));
                }
                let above = if i >= cols { g.indices[i - cols] } else { EDGE };
                let left = if i % cols > 0 { g.indices[i - 1] } else { EDGE };
                let (primary, second) = match b {
                    Some(b) => (b[i], above),
                    None => (above, left),
                };
                model.encode_sym(&mut enc, primary, second, x);
            }
        }
        Ok(enc.finish())
    }

    /// Decode grids produced by [`encode_grids`]. `dims[i] = (cols, rows)`
    /// and `base` must match the encoding call exactly.
    pub fn decode_grids(
        alphabet: u16,
        dims: &[(u16, u16)],
        base: Option<&[Option<&[u16]>]>,
        blob: &[u8],
    ) -> Result<Vec<Vec<u16>>> {
        if let Some(base) = base {
            if base.len() != dims.len() {
                return Err(anyhow!(
                    "base list length {} != grid count {}",
                    base.len(),
                    dims.len()
                ));
            }
        }
        let mut dec = RangeDecoder::new(blob);
        let mut model = Model::new(alphabet);
        let mut out = Vec::with_capacity(dims.len());
        for (gi, &(cols16, rows)) in dims.iter().enumerate() {
            let cols = cols16 as usize;
            let n = cols * rows as usize;
            let b = base.and_then(|b| b[gi]);
            if let Some(b) = b {
                if b.len() != n {
                    return Err(anyhow!("grid {gi}: base length mismatch"));
                }
            }
            let mut g: Vec<u16> = Vec::with_capacity(n);
            for i in 0..n {
                let above = if i >= cols { g[i - cols] } else { EDGE };
                let left = if i % cols > 0 { g[i - 1] } else { EDGE };
                let (primary, second) = match b {
                    Some(b) => (b[i], above),
                    None => (above, left),
                };
                g.push(model.decode_sym(&mut dec, primary, second));
            }
            out.push(g);
        }
        Ok(out)
    }
}
