//! Research probe for further sprite-bank compression (docs/COMPRESSION.md).
//!
//! Three questions, three modes:
//!
//! 1. `--stats <char>` — what is actually inside a per-character RHS chunk:
//!    RLE vs VQ split, control vs literal words, unique colors, pixels.
//! 2. `--recolor <charA>:<charB>` — are variant characters (Knight01/02/03,
//!    Guard A00..A05, …) exact per-pixel color remaps of each other? If a
//!    consistent global color map exists, variants can ship as base + LUT.
//! 3. `--streams <char> --out <dir>` — OpenZL-style format-aware stream
//!    separation: write the packed data split into elementary streams
//!    (RLE control / RLE literals / VQ indices, byte planes, RGB565 channel
//!    planes, per-run deltas) as raw files. A shell driver then compresses
//!    stream combinations with zstd/xz to compare against the monolithic
//!    packed baseline.
//!
//!   cargo run --release --example sprite_compression_probe -- \
//!       --data-dir datadirs/fullgame_linux --stats RobinTown
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;

use robin_assets::frame_holder::{FrameHolder, TRANSPARENT_COLOR_16, UNMAPPED_DICT};
use robin_engine::sprite_script::SpriteScriptor;

#[derive(Parser, Debug)]
#[command(about = "Sprite compression research probe (stats / recolor / stream-split)")]
struct Cli {
    /// Data directory (the one containing `Data/…` or `DATA/…`).
    #[arg(long, default_value = "datadirs/fullgame_linux")]
    data_dir: PathBuf,

    /// Print per-character composition stats.
    #[arg(long)]
    stats: Vec<String>,

    /// Test whether charB is a consistent color remap of charA ("A:B").
    #[arg(long)]
    recolor: Vec<String>,

    /// Write elementary stream files for one character.
    #[arg(long)]
    streams: Vec<String>,

    /// Write direction/frame-interleaved atlas sheets + raw video streams
    /// for one character (aligned via per-frame offsets): per-action PNG
    /// sheets (16 directions across, frames down) and one rgb24 rawvideo
    /// stream where each video frame is a 4x4 grid of the 16 directions.
    #[arg(long)]
    atlas: Vec<String>,

    /// Compute conditional entropies of the VQ tile-index grid for one
    /// character (order-0, |left, |above, |left+above): the headroom a
    /// context-modeling entropy coder would have over zstd.
    #[arg(long)]
    entropy: Vec<String>,

    /// Simulate a realistic adaptive context-model coder (PPM-style with
    /// escapes, single pass, online learning) over the VQ index grid and
    /// report achievable coded size.
    #[arg(long)]
    cm: Vec<String>,

    /// Cross-variant conditional entropy "A:B": how much does variant A's
    /// tile at the same grid position help code variant B?
    #[arg(long)]
    entropy2: Vec<String>,

    /// Adaptive PPM simulation of coding variant B against base A ("A:B").
    #[arg(long)]
    cm2: Vec<String>,

    /// Walk every Characters/*.rhs, detect variant families by trailing
    /// digits, and project corpus size under cm/cm2 coding.
    #[arg(long)]
    corpus: bool,

    /// Encode+decode one character with the real `sprite_codec` range coder
    /// and verify the roundtrip bit-exactly.
    #[arg(long)]
    code: Vec<String>,

    /// Encode+decode variant B against base A ("A:B") with the real codec.
    #[arg(long)]
    code2: Vec<String>,

    /// Verify a converted shipping output `Data/` directory against the
    /// source bank: decode every RHS chunk sprite to pixels through the
    /// shipping dictionaries and compare with the loose-bank decode.
    /// Validates dictionary rank permutation end to end.
    #[arg(long)]
    verify_shipping: Option<PathBuf>,

    /// Output directory for --streams / --atlas files.
    #[arg(long, default_value = "/tmp/sprite_streams")]
    out: PathBuf,
}

fn rhs_path(data_dir: &PathBuf, name: &str) -> Result<PathBuf> {
    for case in ["Data", "DATA"] {
        let p = data_dir.join(format!("{case}/Characters/{name}.rhs"));
        if p.is_file() {
            return Ok(p);
        }
    }
    bail!(
        "no {name}.rhs under {}/(Data|DATA)/Characters",
        data_dir.display()
    )
}

/// All frame ids referenced by a character, in script order (with dups) and
/// as a sorted-deduped bank-order list.
fn char_frame_ids(data_dir: &PathBuf, name: &str) -> Result<(Vec<u32>, Vec<u32>)> {
    let path = rhs_path(data_dir, name)?;
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

/// Decode a sprite to raw stored colors (no arno-law shadow rewrite, no
/// variant effects). Transparent regions become `TRANSPARENT_COLOR_16`.
fn decode_raw(holder: &FrameHolder, id: u32) -> Option<(u16, u16, Vec<u16>)> {
    let s = &holder.sprites()[id as usize];
    let (w, h) = (s.width as usize, s.height as usize);
    if w == 0 || h == 0 {
        return None;
    }
    let packed = holder.packed_data(id)?;
    let mut dst = vec![TRANSPARENT_COLOR_16; w * h];
    if s.dictionary_index == UNMAPPED_DICT {
        let mut p = 0;
        for y in 0..h {
            let first = packed[p];
            let size = packed[p + 1];
            p += 2;
            if size != 0xFFFF {
                let run = (size + 1 - first) as usize;
                dst[y * w + first as usize..y * w + first as usize + run]
                    .copy_from_slice(&packed[p..p + run]);
                p += run;
            }
        }
    } else {
        let dict = holder.dictionary(s.dictionary_index)?;
        let mut p = 0;
        for y in 0..h {
            for x in (0..w).step_by(4) {
                let px = dict.lookup_pixels(packed[p]);
                p += 1;
                dst[y * w + x..y * w + x + 4].copy_from_slice(px);
            }
        }
    }
    Some((s.width, s.height, dst))
}

fn stats(holder: &FrameHolder, data_dir: &PathBuf, name: &str) -> Result<()> {
    let (script_order, ids) = char_frame_ids(data_dir, name)?;
    let mut n_rle = 0u64;
    let mut n_vq = 0u64;
    let mut rle_ctl_words = 0u64;
    let mut rle_lit_words = 0u64;
    let mut vq_idx_words = 0u64;
    let mut total_px = 0u64;
    let mut opaque_px = 0u64;
    let mut colors: HashSet<u16> = HashSet::new();
    let mut dicts: HashSet<u16> = HashSet::new();
    let mut wsum = 0u64;
    let mut hsum = 0u64;
    for &id in &ids {
        let s = &holder.sprites()[id as usize];
        let Some(packed) = holder.packed_data(id) else {
            continue;
        };
        wsum += s.width as u64;
        hsum += s.height as u64;
        total_px += s.width as u64 * s.height as u64;
        if s.dictionary_index == UNMAPPED_DICT {
            n_rle += 1;
            let used = walk_rle(
                packed,
                s.height as usize,
                |_f, _s| rle_ctl_words += 2,
                |lit| {
                    rle_lit_words += lit.len() as u64;
                    opaque_px += lit.len() as u64;
                    colors.extend(lit.iter().copied());
                },
            );
            if used != packed.len() {
                eprintln!(
                    "# sprite {id}: rle walk consumed {used}/{} words",
                    packed.len()
                );
            }
        } else {
            n_vq += 1;
            dicts.insert(s.dictionary_index);
            vq_idx_words += packed.len() as u64;
            let dict = holder
                .dictionary(s.dictionary_index)
                .ok_or_else(|| anyhow!("missing dictionary {}", s.dictionary_index))?;
            for &idx in packed.iter() {
                for &px in dict.lookup_pixels(idx) {
                    if px != TRANSPARENT_COLOR_16 {
                        opaque_px += 1;
                        colors.insert(px);
                    }
                }
            }
        }
    }
    let n = ids.len() as u64;
    println!("## {name}");
    println!("  frame refs in scripts:  {}", script_order.len());
    println!(
        "  unique bank sprites:    {n}  ({n_rle} RLE, {n_vq} VQ, {} dicts)",
        dicts.len()
    );
    println!(
        "  avg dims:               {:.1} x {:.1}",
        wsum as f64 / n as f64,
        hsum as f64 / n as f64
    );
    println!(
        "  canvas pixels:          {total_px}  opaque: {opaque_px} ({:.1}%)",
        100.0 * opaque_px as f64 / total_px as f64
    );
    println!(
        "  packed words:           rle_ctl {rle_ctl_words}  rle_lit {rle_lit_words}  vq_idx {vq_idx_words}  = {} bytes",
        2 * (rle_ctl_words + rle_lit_words + vq_idx_words)
    );
    println!("  unique opaque colors:   {}", colors.len());
    Ok(())
}

fn recolor(holder: &FrameHolder, data_dir: &PathBuf, a: &str, b: &str) -> Result<()> {
    let (sa, _) = char_frame_ids(data_dir, a)?;
    let (sb, _) = char_frame_ids(data_dir, b)?;
    println!("## recolor {a} -> {b}");
    if sa.len() != sb.len() {
        println!(
            "  script-order id counts differ: {} vs {} — pairing first {}",
            sa.len(),
            sb.len(),
            sa.len().min(sb.len())
        );
    }
    let mut pairs: Vec<(u32, u32)> = sa.iter().copied().zip(sb.iter().copied()).collect();
    pairs.sort_unstable();
    pairs.dedup();
    // A frame id in A must always pair with the same id in B, otherwise the
    // positional correspondence itself is broken.
    let mut seen: HashMap<u32, u32> = HashMap::new();
    let mut incoherent = 0u64;
    for &(ia, ib) in &pairs {
        if let Some(&prev) = seen.get(&ia) {
            if prev != ib {
                incoherent += 1;
            }
        } else {
            seen.insert(ia, ib);
        }
    }
    let mut dim_mismatch = 0u64;
    let mut frames = 0u64;
    let mut identical_packed = 0u64;
    let mut px_total = 0u64;
    let mut px_diff = 0u64;
    // (colorA, colorB) → pixel count, for majority-map consistency stats.
    let mut freq: HashMap<(u16, u16), u64> = HashMap::new();
    for &(ia, ib) in &pairs {
        let (Some((wa, ha, da)), Some((wb, hb, db))) =
            (decode_raw(holder, ia), decode_raw(holder, ib))
        else {
            continue;
        };
        if (wa, ha) != (wb, hb) {
            dim_mismatch += 1;
            continue;
        }
        frames += 1;
        if holder.packed_data(ia) == holder.packed_data(ib) {
            identical_packed += 1;
        }
        px_total += da.len() as u64;
        for (&pa, &pb) in da.iter().zip(db.iter()) {
            if pa != pb {
                px_diff += 1;
            }
            *freq.entry((pa, pb)).or_default() += 1;
        }
    }
    let mut best: HashMap<u16, (u16, u64)> = HashMap::new();
    let mut per_src_total: HashMap<u16, u64> = HashMap::new();
    let mut per_src_targets: HashMap<u16, u64> = HashMap::new();
    for (&(pa, pb), &n) in &freq {
        *per_src_total.entry(pa).or_default() += n;
        *per_src_targets.entry(pa).or_default() += 1;
        let e = best.entry(pa).or_insert((pb, 0));
        if n > e.1 {
            *e = (pb, n);
        }
    }
    let mapped_ok: u64 = best.values().map(|&(_, n)| n).sum();
    let mapped_total: u64 = per_src_total.values().sum();
    let conflicted = per_src_targets.values().filter(|&&t| t > 1).count();
    println!(
        "  unique frame pairs:     {} ({} incoherent repeats)",
        pairs.len(),
        incoherent
    );
    println!("  comparable frames:      {frames} ({dim_mismatch} dim mismatches)");
    println!("  identical packed data:  {identical_packed}");
    println!(
        "  pixels differing raw:   {px_diff} / {px_total} ({:.2}%)",
        100.0 * px_diff as f64 / px_total as f64
    );
    println!(
        "  colors in A:            {}   conflicted: {conflicted}",
        per_src_total.len()
    );
    println!(
        "  best-map residual px:   {} / {mapped_total} ({:.4}%)",
        mapped_total - mapped_ok,
        100.0 * (mapped_total - mapped_ok) as f64 / mapped_total as f64
    );
    let mut worst: Vec<(u64, u16, u64)> = per_src_targets
        .iter()
        .filter(|&(_, &t)| t > 1)
        .map(|(&c, &t)| (per_src_total[&c] - best[&c].1, c, t))
        .collect();
    worst.sort_unstable_by(|x, y| y.cmp(x));
    for (residual, color, nalts) in worst.iter().take(8) {
        println!("    color {color:#06x}: {nalts} targets, residual {residual} px");
    }
    Ok(())
}

fn push_u16s(out: &mut Vec<u8>, words: impl IntoIterator<Item = u16>) {
    for w in words {
        out.extend_from_slice(&w.to_le_bytes());
    }
}

fn streams(holder: &FrameHolder, data_dir: &PathBuf, name: &str, out_dir: &PathBuf) -> Result<()> {
    let (_, ids) = char_frame_ids(data_dir, name)?;
    fs::create_dir_all(out_dir)?;

    // Baseline: the shipping-analog monolithic blob (bench `z22-orig`).
    let mut baseline = Vec::new();
    // Elementary streams.
    let mut hdr_w = Vec::new();
    let mut hdr_h = Vec::new();
    let mut hdr_d = Vec::new();
    let mut hdr_len: Vec<u32> = Vec::new();
    let mut rle_first: Vec<u16> = Vec::new();
    let mut rle_size: Vec<u16> = Vec::new();
    let mut rle_px: Vec<u16> = Vec::new();
    let mut rle_run_starts: Vec<u32> = Vec::new();
    let mut vq_idx: Vec<u16> = Vec::new();
    let mut vq_row_starts: Vec<u32> = Vec::new();

    for &id in &ids {
        let s = &holder.sprites()[id as usize];
        let Some(pd) = holder.packed_data(id) else {
            continue;
        };
        baseline.extend_from_slice(&s.width.to_le_bytes());
        baseline.extend_from_slice(&s.height.to_le_bytes());
        baseline.extend_from_slice(&s.dictionary_index.to_le_bytes());
        baseline.extend_from_slice(&(pd.len() as u32).to_le_bytes());
        baseline.extend_from_slice(bytemuck::cast_slice::<u16, u8>(pd));
        hdr_w.push(s.width);
        hdr_h.push(s.height);
        hdr_d.push(s.dictionary_index);
        hdr_len.push(pd.len() as u32);
        if s.dictionary_index == UNMAPPED_DICT {
            walk_rle(
                pd,
                s.height as usize,
                |f, sz| {
                    rle_first.push(f);
                    rle_size.push(sz);
                },
                |lit| {
                    rle_run_starts.push(rle_px.len() as u32);
                    rle_px.extend_from_slice(lit);
                },
            );
        } else {
            let per_row = (s.width / 4) as usize;
            for row in pd.chunks_exact(per_row) {
                vq_row_starts.push(vq_idx.len() as u32);
                vq_idx.extend_from_slice(row);
            }
        }
    }

    let w = |n: &str, bytes: &[u8]| -> Result<()> {
        fs::write(out_dir.join(n), bytes).with_context(|| format!("write {n}"))
    };
    let wu16 = |n: &str, words: &[u16]| -> Result<()> {
        let mut v = Vec::with_capacity(words.len() * 2);
        push_u16s(&mut v, words.iter().copied());
        fs::write(out_dir.join(n), v).with_context(|| format!("write {n}"))
    };
    let planes = |n: &str, words: &[u16]| -> Result<()> {
        let lo: Vec<u8> = words.iter().map(|&x| x as u8).collect();
        let hi: Vec<u8> = words.iter().map(|&x| (x >> 8) as u8).collect();
        fs::write(out_dir.join(format!("{n}.lo")), lo)?;
        fs::write(out_dir.join(format!("{n}.hi")), hi)?;
        Ok(())
    };

    w("baseline.bin", &baseline)?;
    wu16("hdr_w.u16", &hdr_w)?;
    wu16("hdr_h.u16", &hdr_h)?;
    wu16("hdr_d.u16", &hdr_d)?;
    {
        let mut v = Vec::new();
        for l in &hdr_len {
            v.extend_from_slice(&l.to_le_bytes());
        }
        w("hdr_len.u32", &v)?;
    }
    wu16("rle_first.u16", &rle_first)?;
    wu16("rle_size.u16", &rle_size)?;
    // Run length = size+1-first for rows that have a literal run at all —
    // a friendlier stream than raw `size`.
    let runlen: Vec<u16> = rle_first
        .iter()
        .zip(rle_size.iter())
        .filter(|&(_, &sz)| sz != 0xFFFF)
        .map(|(&f, &sz)| sz + 1 - f)
        .collect();
    wu16("rle_runlen.u16", &runlen)?;
    wu16("rle_px.u16", &rle_px)?;
    wu16("vq_idx.u16", &vq_idx)?;
    planes("rle_px", &rle_px)?;
    planes("vq_idx", &vq_idx)?;
    planes("rle_first", &rle_first)?;
    planes("rle_size", &rle_size)?;

    // RGB565 channel planes for RLE literals.
    let r: Vec<u8> = rle_px.iter().map(|&p| (p >> 11) as u8).collect();
    let g: Vec<u8> = rle_px.iter().map(|&p| ((p >> 5) & 0x3F) as u8).collect();
    let b: Vec<u8> = rle_px.iter().map(|&p| (p & 0x1F) as u8).collect();
    w("rle_px_r.u8", &r)?;
    w("rle_px_g.u8", &g)?;
    w("rle_px_b.u8", &b)?;

    // Per-run previous-pixel delta (reset at each run start), as full u16
    // wrapping delta and as per-channel wrapping deltas.
    let mut is_start = vec![false; rle_px.len()];
    for &s in &rle_run_starts {
        if (s as usize) < is_start.len() {
            is_start[s as usize] = true;
        }
    }
    let mut d16: Vec<u16> = Vec::with_capacity(rle_px.len());
    let mut dr: Vec<u8> = Vec::with_capacity(rle_px.len());
    let mut dg: Vec<u8> = Vec::with_capacity(rle_px.len());
    let mut db: Vec<u8> = Vec::with_capacity(rle_px.len());
    let mut prev: u16 = 0;
    for (i, &p) in rle_px.iter().enumerate() {
        if is_start[i] {
            prev = 0;
        }
        d16.push(p.wrapping_sub(prev));
        dr.push(((p >> 11) as u8).wrapping_sub((prev >> 11) as u8) & 0x1F);
        dg.push((((p >> 5) & 0x3F) as u8).wrapping_sub(((prev >> 5) & 0x3F) as u8) & 0x3F);
        db.push(((p & 0x1F) as u8).wrapping_sub((prev & 0x1F) as u8) & 0x1F);
        prev = p;
    }
    wu16("rle_px_d.u16", &d16)?;
    planes("rle_px_d", &d16)?;
    w("rle_px_dr.u8", &dr)?;
    w("rle_px_dg.u8", &dg)?;
    w("rle_px_db.u8", &db)?;

    // VQ index delta within each row.
    let mut vq_start = vec![false; vq_idx.len()];
    for &s in &vq_row_starts {
        if (s as usize) < vq_start.len() {
            vq_start[s as usize] = true;
        }
    }
    let mut vqd: Vec<u16> = Vec::with_capacity(vq_idx.len());
    let mut prev: u16 = 0;
    for (i, &x) in vq_idx.iter().enumerate() {
        if vq_start[i] {
            prev = 0;
        }
        vqd.push(x.wrapping_sub(prev));
        prev = x;
    }
    wu16("vq_idx_d.u16", &vqd)?;
    planes("vq_idx_d", &vqd)?;

    // VQ-focused transforms. Characters are 100% VQ (4x1-pixel tiles), so
    // the index grid is (w/4) x h and vertical neighbors are adjacent pixel
    // rows of the same columns — prime delta territory.
    {
        // Per-sprite up-delta: index minus the index directly above.
        let mut up: Vec<u16> = Vec::with_capacity(vq_idx.len());
        for &id in &ids {
            let s = &holder.sprites()[id as usize];
            if s.dictionary_index == UNMAPPED_DICT {
                continue;
            }
            let Some(pd) = holder.packed_data(id) else {
                continue;
            };
            let per_row = (s.width / 4) as usize;
            for (i, &x) in pd.iter().enumerate() {
                let above = if i >= per_row { pd[i - per_row] } else { 0 };
                up.push(x.wrapping_sub(above));
            }
        }
        wu16("vq_up_d.u16", &up)?;
        planes("vq_up_d", &up)?;

        // Frequency-ranked index remap (dictionary permutation is free at
        // conversion time — the dictionary ships alongside).
        let mut counts: HashMap<u16, u64> = HashMap::new();
        for &x in &vq_idx {
            *counts.entry(x).or_default() += 1;
        }
        let mut by_freq: Vec<(u64, u16)> = counts.iter().map(|(&c, &n)| (n, c)).collect();
        by_freq.sort_unstable_by(|a, b| b.cmp(a));
        let rank: HashMap<u16, u16> = by_freq
            .iter()
            .enumerate()
            .map(|(i, &(_, c))| (c, i as u16))
            .collect();
        let ranked: Vec<u16> = vq_idx.iter().map(|&x| rank[&x]).collect();
        wu16("vq_idx_rank.u16", &ranked)?;
        planes("vq_idx_rank", &ranked)?;
        let mut rank_d: Vec<u16> = Vec::with_capacity(ranked.len());
        let mut prev: u16 = 0;
        for (i, &x) in ranked.iter().enumerate() {
            if vq_start.get(i).copied().unwrap_or(false) {
                prev = 0;
            }
            rank_d.push(x.wrapping_sub(prev));
            prev = x;
        }
        wu16("vq_idx_rank_d.u16", &rank_d)?;
        planes("vq_idx_rank_d", &rank_d)?;
        println!("  vq distinct indices: {}", by_freq.len());
    }

    // The dictionaries used by this character, as raw tile pixel values and
    // as RGB565 channel planes.
    {
        let mut used: Vec<u16> = hdr_d
            .iter()
            .copied()
            .filter(|&d| d != UNMAPPED_DICT)
            .collect();
        used.sort_unstable();
        used.dedup();
        let mut dict_words: Vec<u16> = Vec::new();
        for &di in &used {
            let dict = holder
                .dictionary(di)
                .ok_or_else(|| anyhow!("missing dictionary {di}"))?;
            for i in 0..dict.num_entries() {
                dict_words.extend_from_slice(dict.lookup_pixels(i));
            }
            println!("  dict {di}: {} entries", dict.num_entries());
        }
        wu16("dict.u16", &dict_words)?;
        planes("dict", &dict_words)?;
        let r: Vec<u8> = dict_words.iter().map(|&p| (p >> 11) as u8).collect();
        let g: Vec<u8> = dict_words
            .iter()
            .map(|&p| ((p >> 5) & 0x3F) as u8)
            .collect();
        let b: Vec<u8> = dict_words.iter().map(|&p| (p & 0x1F) as u8).collect();
        w("dict_r.u8", &r)?;
        w("dict_g.u8", &g)?;
        w("dict_b.u8", &b)?;
    }

    // Frequency-ranked palette remap of RLE literals: most frequent color
    // becomes index 0. Tests "the u16 color values themselves are the
    // problem" separately from spatial structure.
    let mut counts: BTreeMap<u16, u64> = BTreeMap::new();
    for &p in &rle_px {
        *counts.entry(p).or_default() += 1;
    }
    let mut by_freq: Vec<(u64, u16)> = counts.iter().map(|(&c, &n)| (n, c)).collect();
    by_freq.sort_unstable_by(|a, b| b.cmp(a));
    let rank: HashMap<u16, u16> = by_freq
        .iter()
        .enumerate()
        .map(|(i, &(_, c))| (c, i as u16))
        .collect();
    let ranked: Vec<u16> = rle_px.iter().map(|&p| rank[&p]).collect();
    wu16("rle_px_pal.u16", &ranked)?;
    planes("rle_px_pal", &ranked)?;

    println!(
        "## streams {name}: {} sprites, rle_px {} words ({} runs), vq_idx {} words, palette {} colors -> {}",
        ids.len(),
        rle_px.len(),
        rle_run_starts.len(),
        vq_idx.len(),
        by_freq.len(),
        out_dir.display()
    );
    Ok(())
}

/// H(X | ctx) in bits/symbol: Σ_{ctx,x} p(ctx,x) · −log2 p(x|ctx).
fn cond_entropy_bits<K: std::hash::Hash + Eq>(joint: &HashMap<K, HashMap<u16, u64>>) -> f64 {
    let total: u64 = joint.values().flat_map(|m| m.values()).sum();
    let mut bits = 0.0f64;
    for per_ctx in joint.values() {
        let ctx_n: u64 = per_ctx.values().sum();
        for &n in per_ctx.values() {
            bits -= (n as f64 / total as f64) * (n as f64 / ctx_n as f64).log2();
        }
    }
    bits
}

/// Conditional-entropy analysis of the VQ index grid.
fn entropy(holder: &FrameHolder, data_dir: &PathBuf, name: &str) -> Result<()> {
    let (_, ids) = char_frame_ids(data_dir, name)?;
    let mut n_syms = 0u64;
    let mut h0: HashMap<u16, u64> = HashMap::new();
    let mut by_left: HashMap<u16, HashMap<u16, u64>> = HashMap::new();
    let mut by_above: HashMap<u16, HashMap<u16, u64>> = HashMap::new();
    let mut by_both: HashMap<(u16, u16), HashMap<u16, u64>> = HashMap::new();
    for &id in &ids {
        let s = &holder.sprites()[id as usize];
        if s.dictionary_index == UNMAPPED_DICT {
            continue;
        }
        let Some(pd) = holder.packed_data(id) else {
            continue;
        };
        let per_row = (s.width / 4) as usize;
        for (i, &x) in pd.iter().enumerate() {
            let col = i % per_row;
            let left = if col > 0 { Some(pd[i - 1]) } else { None };
            let above = if i >= per_row {
                Some(pd[i - per_row])
            } else {
                None
            };
            n_syms += 1;
            *h0.entry(x).or_default() += 1;
            *by_left
                .entry(left.unwrap_or(0xFFFF))
                .or_default()
                .entry(x)
                .or_default() += 1;
            *by_above
                .entry(above.unwrap_or(0xFFFF))
                .or_default()
                .entry(x)
                .or_default() += 1;
            *by_both
                .entry((left.unwrap_or(0xFFFF), above.unwrap_or(0xFFFF)))
                .or_default()
                .entry(x)
                .or_default() += 1;
        }
    }
    let h0_bits: f64 = {
        let total: u64 = h0.values().sum();
        h0.values()
            .map(|&n| {
                let p = n as f64 / total as f64;
                -p * p.log2()
            })
            .sum()
    };
    let hl = cond_entropy_bits(&by_left);
    let ha = cond_entropy_bits(&by_above);
    let hb = cond_entropy_bits(&by_both);
    println!("## entropy {name}: {n_syms} tile indices");
    for (label, bits, nctx) in [
        ("order-0", h0_bits, h0.len()),
        ("| left", hl, by_left.len()),
        ("| above", ha, by_above.len()),
        ("| left+above", hb, by_both.len()),
    ] {
        println!(
            "  {label:<12} {bits:>6.3} bits/tile  -> {:>9.0} bytes  ({nctx} contexts)",
            bits * n_syms as f64 / 8.0
        );
    }
    Ok(())
}

/// Cross-variant conditional entropy: code variant B's tile indices with
/// variant A's aligned tile as (part of) the context. Both characters must
/// pair positionally (verified by --recolor: same dims everywhere).
fn entropy2(holder: &FrameHolder, data_dir: &PathBuf, a: &str, b: &str) -> Result<()> {
    let (sa, _) = char_frame_ids(data_dir, a)?;
    let (sb, _) = char_frame_ids(data_dir, b)?;
    let mut pairs: Vec<(u32, u32)> = sa.iter().copied().zip(sb.iter().copied()).collect();
    pairs.sort_unstable();
    pairs.dedup();
    let mut n_syms = 0u64;
    let mut by_a: HashMap<u16, HashMap<u16, u64>> = HashMap::new();
    let mut by_above: HashMap<u16, HashMap<u16, u64>> = HashMap::new();
    let mut by_a_above: HashMap<(u16, u16), HashMap<u16, u64>> = HashMap::new();
    for &(ia, ib) in &pairs {
        let (spa, spb) = (
            &holder.sprites()[ia as usize],
            &holder.sprites()[ib as usize],
        );
        if spa.dictionary_index == UNMAPPED_DICT
            || spb.dictionary_index == UNMAPPED_DICT
            || (spa.width, spa.height) != (spb.width, spb.height)
        {
            continue;
        }
        let (Some(pa), Some(pb)) = (holder.packed_data(ia), holder.packed_data(ib)) else {
            continue;
        };
        if pa.len() != pb.len() {
            continue;
        }
        let per_row = (spb.width / 4) as usize;
        for (i, &x) in pb.iter().enumerate() {
            let above = if i >= per_row {
                pb[i - per_row]
            } else {
                0xFFFF
            };
            let ax = pa[i];
            n_syms += 1;
            *by_a.entry(ax).or_default().entry(x).or_default() += 1;
            *by_above.entry(above).or_default().entry(x).or_default() += 1;
            *by_a_above
                .entry((ax, above))
                .or_default()
                .entry(x)
                .or_default() += 1;
        }
    }
    println!("## entropy2 {a} -> {b}: {n_syms} tiles");
    for (label, bits, nctx) in [
        ("| A-tile", cond_entropy_bits(&by_a), by_a.len()),
        ("| above", cond_entropy_bits(&by_above), by_above.len()),
        (
            "| A-tile+above",
            {
                let mut tmp: HashMap<u32, HashMap<u16, u64>> = HashMap::new();
                for ((ax, ab), m) in &by_a_above {
                    let e = tmp.entry(((*ax as u32) << 16) | *ab as u32).or_default();
                    for (&x, &n) in m {
                        *e.entry(x).or_default() += n;
                    }
                }
                cond_entropy_bits(&tmp)
            },
            by_a_above.len(),
        ),
    ] {
        println!(
            "  {label:<14} {bits:>6.3} bits/tile  -> {:>9.0} bytes  ({nctx} contexts)",
            bits * n_syms as f64 / 8.0
        );
    }
    Ok(())
}

/// Simulate a PPM-style adaptive coder over the VQ index grid: contexts
/// (above,left) → (above) → order-0 → uniform, PPMC escapes (escape weight =
/// number of distinct symbols seen in the context), full count updates, no
/// exclusion. Single pass in bank order, so all learning cost is included.
/// Reports the exact -log2 product = achievable arithmetic-coded size.
/// Adaptive PPM cost of coding a character standalone.
/// Context chain: (above,left) → above → order-0 → uniform.
/// Returns (tiles, bits, order-2 context count).
fn cm_bits(holder: &FrameHolder, ids: &[u32]) -> (u64, f64, usize) {
    let mut ctx2: HashMap<u32, HashMap<u16, u32>> = HashMap::new();
    let mut ctx1: HashMap<u16, HashMap<u16, u32>> = HashMap::new();
    let mut ctx0: HashMap<u16, u32> = HashMap::new();
    let mut bits = 0.0f64;
    let mut n_syms = 0u64;
    for &id in ids {
        let s = &holder.sprites()[id as usize];
        if s.dictionary_index == UNMAPPED_DICT {
            continue;
        }
        let Some(pd) = holder.packed_data(id) else {
            continue;
        };
        let per_row = (s.width / 4) as usize;
        for (i, &x) in pd.iter().enumerate() {
            let col = i % per_row;
            let left = if col > 0 { pd[i - 1] } else { 0xFFFF };
            let above = if i >= per_row {
                pd[i - per_row]
            } else {
                0xFFFF
            };
            let key2 = ((above as u32) << 16) | left as u32;
            n_syms += 1;
            let mut coded = false;
            bits += ppm_level(ctx2.entry(key2).or_default(), x, &mut coded);
            bits += ppm_level(ctx1.entry(above).or_default(), x, &mut coded);
            bits += ppm_level(&mut ctx0, x, &mut coded);
            if !coded {
                bits += 4096f64.log2();
            }
        }
    }
    (n_syms, bits, ctx2.len())
}

fn cm(holder: &FrameHolder, data_dir: &PathBuf, name: &str) -> Result<()> {
    let (_, ids) = char_frame_ids(data_dir, name)?;
    let (n_syms, bits, nctx) = cm_bits(holder, &ids);
    println!(
        "## cm {name}: {n_syms} tiles, {:.3} bits/tile -> {:.0} bytes  ({nctx} order-2 contexts)",
        bits / n_syms as f64,
        bits / 8.0,
    );
    Ok(())
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

/// Adaptive PPM cost of coding variant B against positional base A.
/// Context chain: (A-tile, above) → A-tile → above → order-0 → uniform.
/// Returns (tiles, skipped_sprites, bits).
fn cm2_bits(holder: &FrameHolder, pairs: &[(u32, u32)]) -> (u64, u64, f64) {
    let mut c2: HashMap<u32, HashMap<u16, u32>> = HashMap::new();
    let mut ca: HashMap<u16, HashMap<u16, u32>> = HashMap::new();
    let mut cb: HashMap<u16, HashMap<u16, u32>> = HashMap::new();
    let mut c0: HashMap<u16, u32> = HashMap::new();
    let mut bits = 0.0f64;
    let mut n_syms = 0u64;
    let mut skipped = 0u64;
    for &(ia, ib) in pairs {
        let (spa, spb) = (
            &holder.sprites()[ia as usize],
            &holder.sprites()[ib as usize],
        );
        let ok = spa.dictionary_index != UNMAPPED_DICT
            && spb.dictionary_index != UNMAPPED_DICT
            && (spa.width, spa.height) == (spb.width, spb.height);
        let (Some(pa), Some(pb)) = (holder.packed_data(ia), holder.packed_data(ib)) else {
            skipped += 1;
            continue;
        };
        if !ok || pa.len() != pb.len() {
            skipped += 1;
            continue;
        }
        let per_row = (spb.width / 4) as usize;
        for (i, &x) in pb.iter().enumerate() {
            let above = if i >= per_row {
                pb[i - per_row]
            } else {
                0xFFFF
            };
            let ax = pa[i];
            let key2 = ((ax as u32) << 16) | above as u32;
            n_syms += 1;
            let mut coded = false;
            bits += ppm_level(c2.entry(key2).or_default(), x, &mut coded);
            bits += ppm_level(ca.entry(ax).or_default(), x, &mut coded);
            bits += ppm_level(cb.entry(above).or_default(), x, &mut coded);
            bits += ppm_level(&mut c0, x, &mut coded);
            if !coded {
                bits += 4096f64.log2();
            }
        }
    }
    (n_syms, skipped, bits)
}

/// Expand RGB565 to 8-bit channels by bit replication (injective, so a
/// lossless 8-bit codec preserves the exact 565 data).
fn rgb565_to_rgb8(px: u16) -> [u8; 3] {
    let r5 = ((px >> 11) & 0x1F) as u8;
    let g6 = ((px >> 5) & 0x3F) as u8;
    let b5 = (px & 0x1F) as u8;
    [
        (r5 << 3) | (r5 >> 2),
        (g6 << 2) | (g6 >> 4),
        (b5 << 3) | (b5 >> 2),
    ]
}

fn write_png(path: &PathBuf, w: u32, h: u32, rgba: bool, data: &[u8]) -> Result<()> {
    let file = fs::File::create(path).with_context(|| format!("create {}", path.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(if rgba {
        png::ColorType::Rgba
    } else {
        png::ColorType::Rgb
    });
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header()?;
    writer.write_image_data(data)?;
    Ok(())
}

/// Direction/frame-interleaved layout experiment. Emits, per character:
///   - `sheets/<profile>_<action>.rgb.png` (+ `.rgba.png`): tiles aligned by
///     frame offsets on a per-action canvas; 16 directions across, frames down.
///   - `video.rgb24`: raw rgb24 stream (constant size, see `video.txt`),
///     each video frame = a 4x4 grid of the 16 directions of one animation
///     frame, actions concatenated in order. For ffmpeg AV1/FFV1 lossless.
///   - `interleaved.rgb565`: the same tile sequence as raw RGB565 words, for
///     a layout-only zstd/xz comparison.
fn atlas(holder: &FrameHolder, data_dir: &PathBuf, name: &str, out_dir: &PathBuf) -> Result<()> {
    let path = rhs_path(data_dir, name)?;
    let (_sig, profiles) = SpriteScriptor::load_all_profiles(path.to_str().unwrap())
        .map_err(|e| anyhow!("load rhs {}: {e}", path.display()))?;
    let sheets_dir = out_dir.join("sheets");
    fs::create_dir_all(&sheets_dir)?;

    // Decode each unique frame once.
    let mut decoded: HashMap<u32, (u16, u16, Vec<u16>)> = HashMap::new();
    let mut decode = |id: u32, cache: &mut HashMap<u32, (u16, u16, Vec<u16>)>| -> bool {
        if cache.contains_key(&id) {
            return true;
        }
        match decode_raw(holder, id) {
            Some(v) => {
                cache.insert(id, v);
                true
            }
            None => false,
        }
    };

    // Group rows by (profile, action). Rows sharing an action id are the
    // direction variants, in file order.
    struct Group {
        profile: usize,
        action: u16,
        rows: Vec<(Vec<u32>, Vec<(i32, i32)>)>, // (frame ids, int offsets)
        // Union bbox of all placed frames: x0,y0,x1,y1 in offset space.
        bbox: (i32, i32, i32, i32),
        max_f: usize,
    }
    let mut groups: Vec<Group> = Vec::new();
    for (pi, (_pname, info)) in profiles.iter().enumerate() {
        let mut by_action: BTreeMap<u16, Vec<&robin_engine::sprite_script::SpriteScript>> =
            BTreeMap::new();
        for s in info.scripts.iter() {
            by_action.entry(s.action_id).or_default().push(s);
        }
        for (&action, rows) in &by_action {
            let mut g = Group {
                profile: pi,
                action,
                rows: Vec::new(),
                bbox: (i32::MAX, i32::MAX, i32::MIN, i32::MIN),
                max_f: 0,
            };
            for r in rows {
                let mut offs = Vec::with_capacity(r.frame_ids.len());
                for (fi, &id) in r.frame_ids.iter().enumerate() {
                    if !decode(id, &mut decoded) {
                        offs.push((0, 0));
                        continue;
                    }
                    let (w, h, _) = decoded[&id];
                    let (ox, oy) = r
                        .offsets
                        .get(fi)
                        .map(|o| (o.x.round() as i32, o.y.round() as i32))
                        .unwrap_or((0, 0));
                    g.bbox.0 = g.bbox.0.min(ox);
                    g.bbox.1 = g.bbox.1.min(oy);
                    g.bbox.2 = g.bbox.2.max(ox + w as i32);
                    g.bbox.3 = g.bbox.3.max(oy + h as i32);
                    offs.push((ox, oy));
                }
                g.max_f = g.max_f.max(r.frame_ids.len());
                g.rows.push((r.frame_ids.clone(), offs));
            }
            if g.max_f > 0 && g.bbox.0 <= g.bbox.2 {
                groups.push(g);
            }
        }
    }

    // Global tile size for the constant-dimension video stream.
    let vt_w = groups
        .iter()
        .map(|g| (g.bbox.2 - g.bbox.0) as usize)
        .max()
        .unwrap_or(0);
    let vt_h = groups
        .iter()
        .map(|g| (g.bbox.3 - g.bbox.1) as usize)
        .max()
        .unwrap_or(0);

    let key_rgb = rgb565_to_rgb8(TRANSPARENT_COLOR_16);
    let mut video = std::io::BufWriter::new(fs::File::create(out_dir.join("video.rgb24"))?);
    let mut interleaved: Vec<u8> = Vec::new();
    let mut n_video_frames = 0u64;
    let mut n_tiles = 0u64;
    let mut sheet_px = 0u64;

    for g in &groups {
        let tw = (g.bbox.2 - g.bbox.0) as usize;
        let th = (g.bbox.3 - g.bbox.1) as usize;
        let ndir = g.rows.len();

        // --- per-action sheet: ndir across, max_f down ---
        let sw = tw * ndir;
        let sh = th * g.max_f;
        let mut rgb = vec![0u8; sw * sh * 3];
        for p in rgb.chunks_exact_mut(3) {
            p.copy_from_slice(&key_rgb);
        }
        let mut rgba = vec![0u8; sw * sh * 4];
        for (d, (ids, offs)) in g.rows.iter().enumerate() {
            for (f, &id) in ids.iter().enumerate() {
                let Some((w, h, px)) = decoded.get(&id) else {
                    continue;
                };
                let bx = d * tw + (offs[f].0 - g.bbox.0) as usize;
                let by = f * th + (offs[f].1 - g.bbox.1) as usize;
                for y in 0..*h as usize {
                    for x in 0..*w as usize {
                        let c = px[y * *w as usize + x];
                        let o = ((by + y) * sw + bx + x) * 3;
                        let o4 = ((by + y) * sw + bx + x) * 4;
                        let c8 = rgb565_to_rgb8(c);
                        rgb[o..o + 3].copy_from_slice(&c8);
                        if c != TRANSPARENT_COLOR_16 {
                            rgba[o4..o4 + 3].copy_from_slice(&c8);
                            rgba[o4 + 3] = 255;
                        }
                    }
                }
            }
        }
        let base = format!("p{}_a{}", g.profile, g.action);
        write_png(
            &sheets_dir.join(format!("{base}.rgb.png")),
            sw as u32,
            sh as u32,
            false,
            &rgb,
        )?;
        write_png(
            &sheets_dir.join(format!("{base}.rgba.png")),
            sw as u32,
            sh as u32,
            true,
            &rgba,
        )?;
        sheet_px += (sw * sh) as u64;

        // --- video frames: 4x4 grid of directions per animation frame ---
        let vw = vt_w * 4;
        let vh = vt_h * 4;
        for f in 0..g.max_f {
            let mut frame = vec![0u8; vw * vh * 3];
            for p in frame.chunks_exact_mut(3) {
                p.copy_from_slice(&key_rgb);
            }
            let mut tile565 = vec![TRANSPARENT_COLOR_16; vt_w * vt_h];
            for (d, (ids, offs)) in g.rows.iter().enumerate().take(16) {
                let Some(&id) = ids.get(f) else { continue };
                let Some((w, h, px)) = decoded.get(&id) else {
                    continue;
                };
                let gx = (d % 4) * vt_w;
                let gy = (d / 4) * vt_h;
                let bx = (offs[f].0 - g.bbox.0) as usize;
                let by = (offs[f].1 - g.bbox.1) as usize;
                for p in tile565.iter_mut() {
                    *p = TRANSPARENT_COLOR_16;
                }
                for y in 0..*h as usize {
                    for x in 0..*w as usize {
                        let c = px[y * *w as usize + x];
                        let o = ((gy + by + y) * vw + gx + bx + x) * 3;
                        frame[o..o + 3].copy_from_slice(&rgb565_to_rgb8(c));
                        tile565[(by + y) * vt_w + bx + x] = c;
                    }
                }
                // layout-only stream: aligned tiles, direction-major.
                for &c in &tile565 {
                    interleaved.extend_from_slice(&c.to_le_bytes());
                }
                n_tiles += 1;
            }
            std::io::Write::write_all(&mut video, &frame)?;
            n_video_frames += 1;
        }
    }
    drop(video);
    fs::write(out_dir.join("interleaved.rgb565"), &interleaved)?;
    fs::write(
        out_dir.join("video.txt"),
        format!(
            "width={} height={} frames={}\n",
            vt_w * 4,
            vt_h * 4,
            n_video_frames
        ),
    )?;
    println!(
        "## atlas {name}: {} groups, tile {}x{} (video {}x{}), {} video frames, {} aligned tiles, {} sheet px -> {}",
        groups.len(),
        vt_w,
        vt_h,
        vt_w * 4,
        vt_h * 4,
        n_video_frames,
        n_tiles,
        sheet_px,
        out_dir.display()
    );
    Ok(())
}

fn positional_pairs(data_dir: &PathBuf, a: &str, b: &str) -> Result<Vec<(u32, u32)>> {
    let (sa, _) = char_frame_ids(data_dir, a)?;
    let (sb, _) = char_frame_ids(data_dir, b)?;
    let mut pairs: Vec<(u32, u32)> = sa.iter().copied().zip(sb.iter().copied()).collect();
    pairs.sort_unstable();
    pairs.dedup();
    Ok(pairs)
}

fn cm2(holder: &FrameHolder, data_dir: &PathBuf, a: &str, b: &str) -> Result<()> {
    let pairs = positional_pairs(data_dir, a, b)?;
    let (n_syms, skipped, bits) = cm2_bits(holder, &pairs);
    println!(
        "## cm2 {a} -> {b}: {n_syms} tiles ({skipped} sprites skipped), {:.3} bits/tile -> {:.0} bytes",
        bits / n_syms as f64,
        bits / 8.0,
    );
    Ok(())
}

/// Corpus projection: every Characters/*.rhs coded standalone (cm) or, for
/// detected variant families, against the family base (cm2). Also reports
/// zstd-19 of the shipping-analog packed blob as the "current" reference.
fn corpus(holder: &FrameHolder, data_dir: &PathBuf) -> Result<()> {
    let mut chars_dir = data_dir.join("Data/Characters");
    if !chars_dir.is_dir() {
        chars_dir = data_dir.join("DATA/Characters");
    }
    let mut names: Vec<String> = fs::read_dir(&chars_dir)?
        .filter_map(|e| {
            let p = e.ok()?.path();
            (p.extension()?.to_str()? == "rhs")
                .then(|| p.file_stem().unwrap().to_string_lossy().into_owned())
        })
        .collect();
    names.sort();

    // Family = trailing two digits over a shared prefix with >1 member.
    let family_key = |n: &str| -> Option<String> {
        let stripped = n.trim_end_matches(|c: char| c.is_ascii_digit());
        (stripped.len() + 2 == n.len() && !stripped.is_empty()).then(|| stripped.to_string())
    };
    let mut families: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for n in &names {
        if let Some(k) = family_key(n) {
            families.entry(k).or_default().push(n.clone());
        }
    }
    families.retain(|_, v| v.len() > 1);
    let base_of: HashMap<String, String> = families
        .values()
        .flat_map(|v| {
            let base = v[0].clone();
            v.iter().skip(1).map(move |n| (n.clone(), base.clone()))
        })
        .collect();

    println!(
        "## corpus: {} characters, {} families",
        names.len(),
        families.len()
    );
    println!(
        "{:<24} {:>10} {:>10} {:>10} {:>9}  base",
        "character", "packed", "zstd19", "cm/cm2", "bits/tile"
    );
    let (mut t_packed, mut t_zstd, mut t_cm) = (0u64, 0u64, 0u64);
    for n in &names {
        let Ok((_, ids)) = char_frame_ids(data_dir, n) else {
            eprintln!("# skip {n}: rhs load failed");
            continue;
        };
        let mut blob = Vec::new();
        for &id in &ids {
            let s = &holder.sprites()[id as usize];
            let Some(pd) = holder.packed_data(id) else {
                continue;
            };
            blob.extend_from_slice(&s.width.to_le_bytes());
            blob.extend_from_slice(&s.height.to_le_bytes());
            blob.extend_from_slice(&s.dictionary_index.to_le_bytes());
            blob.extend_from_slice(&(pd.len() as u32).to_le_bytes());
            blob.extend_from_slice(bytemuck::cast_slice::<u16, u8>(pd));
        }
        let zstd_len = zstd::stream::encode_all(&blob[..], 19)?.len() as u64;
        // Real-codec sizes (encode only; roundtrip is covered by --code and
        // the module tests).
        use robin_assets::sprite_codec::{SpriteGrid, encode_grids};
        let standalone = |ids: &[u32]| -> Result<(String, u64, u64)> {
            let (_gids, dims, slices, alphabet) = codec_grids(holder, ids)?;
            if slices.is_empty() {
                return Ok((String::new(), 0, 0));
            }
            let grids: Vec<SpriteGrid> = dims
                .iter()
                .zip(slices.iter())
                .map(|(&(c, r), &s)| SpriteGrid {
                    cols: c,
                    rows: r,
                    indices: s,
                })
                .collect();
            let n_tiles: u64 = slices.iter().map(|s| s.len() as u64).sum();
            let blob = encode_grids(alphabet, &grids, None)?;
            Ok((String::new(), n_tiles, blob.len() as u64))
        };
        let (label, n_syms, cm_bytes) = if let Some(base) = base_of.get(n) {
            let pairs = positional_pairs(data_dir, base, n)?;
            let (dims, slices, bases, alphabet, unbased) = codec_grids2(holder, &pairs)?;
            if slices.is_empty() || unbased > slices.len() / 10 {
                // Family structure mismatch — fall back to standalone.
                let (_, n_tiles, bytes) = standalone(&ids)?;
                (format!("(standalone, {unbased} unbased)"), n_tiles, bytes)
            } else {
                let grids: Vec<SpriteGrid> = dims
                    .iter()
                    .zip(slices.iter())
                    .map(|(&(c, r), &s)| SpriteGrid {
                        cols: c,
                        rows: r,
                        indices: s,
                    })
                    .collect();
                let n_tiles: u64 = slices.iter().map(|s| s.len() as u64).sum();
                let coded = encode_grids(alphabet, &grids, Some(&bases))?;
                (format!("<- {base}"), n_tiles, coded.len() as u64)
            }
        } else {
            standalone(&ids)?
        };
        let bits = cm_bytes as f64 * 8.0;
        t_packed += blob.len() as u64;
        t_zstd += zstd_len;
        t_cm += cm_bytes;
        println!(
            "{n:<24} {:>10} {zstd_len:>10} {cm_bytes:>10} {:>9.3}  {label}",
            blob.len(),
            bits / n_syms.max(1) as f64,
        );
    }
    println!(
        "{:<24} {t_packed:>10} {t_zstd:>10} {t_cm:>10}   ({:.2}x vs zstd19)",
        "TOTAL",
        t_zstd as f64 / t_cm as f64
    );
    Ok(())
}

/// Collect a character's VQ sprites as codec grids plus the dictionary
/// alphabet size. Returns (bank ids in grid order, dims, index slices).
#[allow(clippy::type_complexity)]
fn codec_grids<'h>(
    holder: &'h FrameHolder,
    ids: &[u32],
) -> Result<(Vec<u32>, Vec<(u16, u16)>, Vec<&'h [u16]>, u16)> {
    let mut grid_ids = Vec::new();
    let mut dims = Vec::new();
    let mut slices: Vec<&[u16]> = Vec::new();
    let mut alphabet: u16 = 0;
    for &id in ids {
        let s = &holder.sprites()[id as usize];
        if s.dictionary_index == UNMAPPED_DICT {
            continue;
        }
        let Some(pd) = holder.packed_data(id) else {
            continue;
        };
        let dict = holder
            .dictionary(s.dictionary_index)
            .ok_or_else(|| anyhow!("missing dictionary {}", s.dictionary_index))?;
        alphabet = alphabet.max(dict.num_entries());
        grid_ids.push(id);
        dims.push((s.width / 4, s.height));
        slices.push(pd);
    }
    Ok((grid_ids, dims, slices, alphabet))
}

fn code(holder: &FrameHolder, data_dir: &PathBuf, name: &str) -> Result<()> {
    use robin_assets::sprite_codec::{SpriteGrid, decode_grids, encode_grids};
    let (_, ids) = char_frame_ids(data_dir, name)?;
    let (_gids, dims, slices, alphabet) = codec_grids(holder, &ids)?;
    let grids: Vec<SpriteGrid> = dims
        .iter()
        .zip(slices.iter())
        .map(|(&(c, r), &s)| SpriteGrid {
            cols: c,
            rows: r,
            indices: s,
        })
        .collect();
    let n_tiles: usize = slices.iter().map(|s| s.len()).sum();
    let t0 = std::time::Instant::now();
    let blob = encode_grids(alphabet, &grids, None)?;
    let t_enc = t0.elapsed();
    let t0 = std::time::Instant::now();
    let decoded = decode_grids(alphabet, &dims, None, &blob)?;
    let t_dec = t0.elapsed();
    for (i, (d, s)) in decoded.iter().zip(slices.iter()).enumerate() {
        if d.as_slice() != *s {
            return Err(anyhow!("{name}: roundtrip mismatch at grid {i}"));
        }
    }
    println!(
        "## code {name}: {} sprites, {n_tiles} tiles, alphabet {alphabet} -> {} bytes ({:.3} bits/tile), enc {:.1}s dec {:.1}s, roundtrip OK",
        grids.len(),
        blob.len(),
        blob.len() as f64 * 8.0 / n_tiles as f64,
        t_enc.as_secs_f64(),
        t_dec.as_secs_f64(),
    );
    Ok(())
}

/// Gather variant B's VQ grids with aligned base-A slices for cross-variant
/// coding. Returns (dims, indices, bases, alphabet, unbased count).
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

fn code2(holder: &FrameHolder, data_dir: &PathBuf, a: &str, b: &str) -> Result<()> {
    use robin_assets::sprite_codec::{SpriteGrid, decode_grids, encode_grids};
    let pairs = positional_pairs(data_dir, a, b)?;
    let (dims, slices, bases, alphabet, unbased) = codec_grids2(holder, &pairs)?;
    let grids: Vec<SpriteGrid> = dims
        .iter()
        .zip(slices.iter())
        .map(|(&(c, r), &s)| SpriteGrid {
            cols: c,
            rows: r,
            indices: s,
        })
        .collect();
    let n_tiles: usize = slices.iter().map(|s| s.len()).sum();
    let t0 = std::time::Instant::now();
    let blob = encode_grids(alphabet, &grids, Some(&bases))?;
    let t_enc = t0.elapsed();
    let decoded = decode_grids(alphabet, &dims, Some(&bases), &blob)?;
    for (i, (d, s)) in decoded.iter().zip(slices.iter()).enumerate() {
        if d.as_slice() != *s {
            return Err(anyhow!("{a}:{b}: roundtrip mismatch at grid {i}"));
        }
    }
    println!(
        "## code2 {a} -> {b}: {} sprites ({unbased} unbased), {n_tiles} tiles -> {} bytes ({:.3} bits/tile), enc {:.1}s, roundtrip OK",
        grids.len(),
        blob.len(),
        blob.len() as f64 * 8.0 / n_tiles as f64,
        t_enc.as_secs_f64(),
    );
    Ok(())
}

/// Decode one shipping sprite (RLE or VQ through the given dictionary set)
/// to raw canvas pixels; mirrors `decode_raw` for the loose bank.
fn decode_shipping_sprite(
    sprite: &robin_assets::shipping_datadir::ShippingSprite,
    dicts: &[robin_assets::frame_holder::FrameDictionary],
) -> Result<Vec<u16>> {
    let (w, h) = (sprite.width as usize, sprite.height as usize);
    let mut dst = vec![TRANSPARENT_COLOR_16; w * h];
    let packed = &sprite.packed_data[..];
    if sprite.dictionary_index == UNMAPPED_DICT {
        let mut p = 0;
        for y in 0..h {
            let first = packed[p];
            let size = packed[p + 1];
            p += 2;
            if size != 0xFFFF {
                let run = (size + 1 - first) as usize;
                dst[y * w + first as usize..y * w + first as usize + run]
                    .copy_from_slice(&packed[p..p + run]);
                p += run;
            }
        }
    } else {
        let dict = dicts
            .get(sprite.dictionary_index as usize)
            .ok_or_else(|| anyhow!("missing shipping dictionary {}", sprite.dictionary_index))?;
        let mut p = 0;
        for y in 0..h {
            for x in (0..w).step_by(4) {
                let px = dict.lookup_pixels(packed[p]);
                p += 1;
                dst[y * w + x..y * w + x + 4].copy_from_slice(px);
            }
        }
    }
    Ok(dst)
}

fn verify_shipping(holder: &FrameHolder, data_out: &PathBuf) -> Result<()> {
    use robin_assets::shipping_datadir::{ShippingDatadir, decode_mission_compressed};
    let dd = ShippingDatadir::load_from_file(&data_out.join("datadir.bin"))?;
    let bank = dd
        .sprite_bank
        .as_ref()
        .ok_or_else(|| anyhow!("boot manifest has no sprite bank"))?;
    let mut chunks: Vec<PathBuf> = fs::read_dir(data_out.join("rhs"))?
        .filter_map(|e| Some(e.ok()?.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "zst"))
        .collect();
    chunks.sort();
    let (mut n_sprites, mut n_vq, mut n_px) = (0u64, 0u64, 0u64);
    for chunk in &chunks {
        let mission = decode_mission_compressed(&fs::read(chunk)?)
            .with_context(|| format!("decode {}", chunk.display()))?;
        let Some(chunk_bank) = mission.sprite_bank.as_ref() else {
            continue;
        };
        for (id, sprite) in &chunk_bank.sprites {
            if sprite.width == 0 || sprite.height == 0 {
                continue;
            }
            let shipped = decode_shipping_sprite(sprite, &bank.dictionaries)?;
            let source = decode_raw(holder, *id)
                .ok_or_else(|| anyhow!("source bank cannot decode sprite {id}"))?;
            if (source.0, source.1) != (sprite.width, sprite.height) || source.2 != shipped {
                bail!(
                    "sprite {id} in {} decodes differently from the source bank",
                    chunk.display()
                );
            }
            n_sprites += 1;
            n_px += shipped.len() as u64;
            if sprite.dictionary_index != UNMAPPED_DICT {
                n_vq += 1;
            }
        }
    }
    println!(
        "## verify-shipping {}: {} chunks, {n_sprites} sprites ({n_vq} VQ), {n_px} pixels — all identical to source bank",
        data_out.display(),
        chunks.len()
    );
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let data_dir_s = cli.data_dir.to_str().unwrap();
    let mut holder = FrameHolder::new();
    holder
        .initialize_sprite_bank(data_dir_s)
        .context("initialize_sprite_bank")?;
    eprintln!(
        "# bank loaded: {} sprites, {} dictionaries",
        holder.num_sprites(),
        holder.dictionaries().len()
    );
    for name in &cli.stats {
        stats(&holder, &cli.data_dir, name)?;
    }
    for pair in &cli.recolor {
        let (a, b) = pair
            .split_once(':')
            .ok_or_else(|| anyhow!("--recolor wants A:B, got {pair}"))?;
        recolor(&holder, &cli.data_dir, a, b)?;
    }
    for name in &cli.streams {
        streams(&holder, &cli.data_dir, name, &cli.out.join(name))?;
    }
    for name in &cli.atlas {
        atlas(&holder, &cli.data_dir, name, &cli.out.join(name))?;
    }
    for name in &cli.entropy {
        entropy(&holder, &cli.data_dir, name)?;
    }
    for name in &cli.cm {
        cm(&holder, &cli.data_dir, name)?;
    }
    for pair in &cli.entropy2 {
        let (a, b) = pair
            .split_once(':')
            .ok_or_else(|| anyhow!("--entropy2 wants A:B, got {pair}"))?;
        entropy2(&holder, &cli.data_dir, a, b)?;
    }
    for pair in &cli.cm2 {
        let (a, b) = pair
            .split_once(':')
            .ok_or_else(|| anyhow!("--cm2 wants A:B, got {pair}"))?;
        cm2(&holder, &cli.data_dir, a, b)?;
    }
    if cli.corpus {
        corpus(&holder, &cli.data_dir)?;
    }
    for name in &cli.code {
        code(&holder, &cli.data_dir, name)?;
    }
    for pair in &cli.code2 {
        let (a, b) = pair
            .split_once(':')
            .ok_or_else(|| anyhow!("--code2 wants A:B, got {pair}"))?;
        code2(&holder, &cli.data_dir, a, b)?;
    }
    if let Some(dir) = &cli.verify_shipping {
        verify_shipping(&holder, dir)?;
    }
    Ok(())
}
