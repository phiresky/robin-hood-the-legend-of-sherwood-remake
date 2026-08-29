//! Encoder-side rate-distortion (RDO) tile-assignment experiment for the VQ
//! sprite codec (docs/COMPRESSION.md, "Sprite research" / "Implementation:
//! sprite_codec" 2026-08-28).
//!
//! Character sprites are grids of 12-bit indices into a per-character
//! 4096-entry dictionary of 4x1-pixel RGB565 tiles. When several dictionary
//! tiles are identical or nearly identical, re-pointing grid positions at a
//! "cheaper" candidate reduces entropy at zero or bounded visual cost. Four
//! measurements per character:
//!
//! 1. Exact duplicates (lossless): group identical 4-pixel patterns inside
//!    the dictionary, canonicalize every sprite index to the group
//!    representative (lowest id), measure the real codec size.
//! 2. Near-duplicate potential: histogram of nearest-neighbor distance
//!    between dictionary tiles (max per-channel delta over the 4 pixels in
//!    R5/G6/B5 space; key pixels — transparency AND the shadow key, both
//!    rewritten specially at render time — must match exactly).
//! 3. Greedy RDO (lossy, bounded): for each eps, re-assign each grid
//!    position to the candidate within eps of the ORIGINAL tile that scores
//!    highest under an online adaptive model (the doc's cm simulation:
//!    (above,left) -> above -> order-0 chain, `ppm_level` copied from
//!    sprite_compression_probe), sweeping sprites in raster order with the
//!    already-re-assigned neighbors as context. Ties keep the original.
//!    This is a heuristic pre-pass; the deliverable number is the REAL
//!    codec size of the modified grids via `sprite_codec::encode_grids`.
//! 4. Quality evidence: side-by-side PNGs (original | re-assigned) of three
//!    representative sprites (largest, median, most substitutions) per eps.
//!
//!   cargo run --release --example sprite_probe_rdo -- \
//!       --data-dir datadirs/fullgame_linux
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;

use robin_assets::frame_holder::{
    FrameDictionary, FrameHolder, SHADOW_KEY, TRANSPARENT_COLOR_16, UNMAPPED_DICT,
};
use robin_assets::sprite_codec::{SpriteGrid, encode_grids};
use robin_engine::sprite_script::SpriteScriptor;

#[derive(Parser, Debug)]
#[command(about = "Encoder-side RDO tile-assignment experiment for the VQ sprite codec")]
struct Cli {
    /// Data directory (the one containing `Data/…` or `DATA/…`).
    #[arg(long, default_value = "datadirs/fullgame_linux")]
    data_dir: PathBuf,

    /// Characters to measure.
    #[arg(long, default_values_t = [
        "RobinTown".to_string(),
        "Knight01".to_string(),
        "Guard A00".to_string(),
    ])]
    chars: Vec<String>,

    /// Distortion bounds (max per-channel delta in R5/G6/B5 units).
    #[arg(long, default_values_t = [1u32, 2u32])]
    eps: Vec<u32>,

    /// Where the side-by-side preview PNGs go.
    #[arg(long, default_value = "/tmp/robin-rdo-preview")]
    preview_dir: PathBuf,

    /// Skip writing preview PNGs.
    #[arg(long)]
    no_previews: bool,
}

// ---------------------------------------------------------------------------
// Helpers copied from sprite_compression_probe.rs (that file is owned by the
// codec-research task; this experiment must not modify it).
// ---------------------------------------------------------------------------

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

/// All frame ids referenced by a character, as a sorted-deduped bank-order
/// list.
fn char_frame_ids(data_dir: &PathBuf, name: &str) -> Result<Vec<u32>> {
    let path = rhs_path(data_dir, name)?;
    let (_sig, profiles) = SpriteScriptor::load_all_profiles(path.to_str().unwrap())
        .map_err(|e| anyhow!("load rhs {}: {e}", path.display()))?;
    let mut ids = Vec::new();
    for (_p, info) in &profiles {
        for s in info.scripts.iter() {
            ids.extend_from_slice(&s.frame_ids);
        }
    }
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
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

/// Expand RGB565 to 8-bit channels by bit replication.
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

// ---------------------------------------------------------------------------
// Dictionary analysis
// ---------------------------------------------------------------------------

/// Distance between two dictionary tiles: max per-channel delta (R5/G6/B5
/// units) over the 4 pixels. `None` when the tiles are incompatible for
/// substitution: a key pixel (transparent color or shadow key — both are
/// sentinels rewritten at render time, never plain colors) on either side
/// must be matched exactly by the other.
fn tile_dist(a: &[u16; 4], b: &[u16; 4]) -> Option<u32> {
    let mut d = 0u32;
    for k in 0..4 {
        let (pa, pb) = (a[k], b[k]);
        if pa == pb {
            continue;
        }
        if pa == TRANSPARENT_COLOR_16
            || pb == TRANSPARENT_COLOR_16
            || pa == SHADOW_KEY
            || pb == SHADOW_KEY
        {
            return None;
        }
        let dr = ((pa >> 11) & 0x1F).abs_diff((pb >> 11) & 0x1F) as u32;
        let dg = ((pa >> 5) & 0x3F).abs_diff((pb >> 5) & 0x3F) as u32;
        let db = (pa & 0x1F).abs_diff(pb & 0x1F) as u32;
        d = d.max(dr).max(dg).max(db);
    }
    Some(d)
}

/// Histogram buckets for nearest-neighbor distances: 0,1,2,3,4,5-8,>8.
const NN_BUCKETS: usize = 7;

fn nn_bucket(d: u32) -> usize {
    match d {
        0..=4 => d as usize,
        5..=8 => 5,
        _ => 6,
    }
}

const NN_LABELS: [&str; NN_BUCKETS] = ["d=0", "d=1", "d=2", "d=3", "d=4", "d=5-8", "d>8"];

struct DictAnalysis {
    /// Exact-duplicate canonical map: index -> lowest index with the same
    /// 4-pixel pattern.
    canonical: Vec<u16>,
    /// Number of pattern groups with >= 2 members.
    dup_groups: usize,
    /// Entries beyond the first of their group (== positions the lossless
    /// canonicalization can retarget).
    dup_extra: usize,
    /// Nearest-neighbor distance histogram over all entries.
    nn_hist: [u64; NN_BUCKETS],
    /// Entries with no compatible neighbor at all (key-pattern mismatch
    /// against every other tile).
    nn_none: u64,
    /// Per entry: all `(dist, other)` neighbors with `dist <= max_eps`
    /// (exact duplicates included at dist 0).
    near: Vec<Vec<(u32, u16)>>,
}

fn analyze_dictionary(dict: &FrameDictionary, max_eps: u32) -> DictAnalysis {
    let n = dict.num_entries() as usize;
    let patterns: Vec<[u16; 4]> = (0..n)
        .map(|i| {
            let px = dict.lookup_pixels(i as u16);
            [px[0], px[1], px[2], px[3]]
        })
        .collect();

    // Exact-duplicate groups: representative = lowest index.
    let mut first_of: HashMap<[u16; 4], u16> = HashMap::new();
    let mut group_sizes: HashMap<u16, u32> = HashMap::new();
    let mut canonical = Vec::with_capacity(n);
    for (i, p) in patterns.iter().enumerate() {
        let rep = *first_of.entry(*p).or_insert(i as u16);
        canonical.push(rep);
        *group_sizes.entry(rep).or_default() += 1;
    }
    let dup_groups = group_sizes.values().filter(|&&c| c >= 2).count();
    let dup_extra = n - first_of.len();

    // Pairwise nearest-neighbor distances + near lists (one O(n^2) pass;
    // 4096 entries -> 8.4M distance evaluations, cheap in release).
    let mut nn: Vec<Option<u32>> = vec![None; n];
    let mut near: Vec<Vec<(u32, u16)>> = vec![Vec::new(); n];
    for i in 0..n {
        for j in (i + 1)..n {
            let Some(d) = tile_dist(&patterns[i], &patterns[j]) else {
                continue;
            };
            if nn[i].is_none_or(|m| d < m) {
                nn[i] = Some(d);
            }
            if nn[j].is_none_or(|m| d < m) {
                nn[j] = Some(d);
            }
            if d <= max_eps {
                near[i].push((d, j as u16));
                near[j].push((d, i as u16));
            }
        }
    }
    let mut nn_hist = [0u64; NN_BUCKETS];
    let mut nn_none = 0u64;
    for m in &nn {
        match m {
            Some(d) => nn_hist[nn_bucket(*d)] += 1,
            None => nn_none += 1,
        }
    }

    DictAnalysis {
        canonical,
        dup_groups,
        dup_extra,
        nn_hist,
        nn_none,
        near,
    }
}

// ---------------------------------------------------------------------------
// Grid collection and coding
// ---------------------------------------------------------------------------

struct CharGrids {
    /// Bank sprite id per grid.
    ids: Vec<u32>,
    /// (cols, rows) per grid.
    dims: Vec<(u16, u16)>,
    /// Owned index grids (mutated copies are derived from these).
    grids: Vec<Vec<u16>>,
    /// Dictionary index per grid.
    dict_of: Vec<u16>,
    alphabet: u16,
    n_tiles: u64,
}

fn collect_grids(holder: &FrameHolder, ids: &[u32]) -> Result<CharGrids> {
    let mut out = CharGrids {
        ids: Vec::new(),
        dims: Vec::new(),
        grids: Vec::new(),
        dict_of: Vec::new(),
        alphabet: 0,
        n_tiles: 0,
    };
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
        if let Some(&bad) = pd.iter().find(|&&x| x >= dict.num_entries()) {
            bail!(
                "sprite {id}: index {bad} >= dictionary size {}",
                dict.num_entries()
            );
        }
        out.alphabet = out.alphabet.max(dict.num_entries());
        out.ids.push(id);
        out.dims.push((s.width / 4, s.height));
        out.n_tiles += pd.len() as u64;
        out.grids.push(pd.to_vec());
        out.dict_of.push(s.dictionary_index);
    }
    Ok(out)
}

/// Real coded size of a set of grids (encode only; the codec's roundtrip is
/// covered by its module tests and the probe's --code mode).
fn coded_size(alphabet: u16, dims: &[(u16, u16)], grids: &[Vec<u16>]) -> Result<u64> {
    let refs: Vec<SpriteGrid> = dims
        .iter()
        .zip(grids.iter())
        .map(|(&(c, r), g)| SpriteGrid {
            cols: c,
            rows: r,
            indices: g,
        })
        .collect();
    Ok(encode_grids(alphabet, &refs, None)?.len() as u64)
}

struct SweepResult {
    grids: Vec<Vec<u16>>,
    per_grid_changed: Vec<u32>,
    changed: u64,
    /// Heuristic model estimate (the cm simulation's own bits) — reported
    /// for context only; the real codec size is the deliverable.
    est_bits: f64,
}

/// Greedy RDO pre-pass: sweep every sprite in raster order, re-assigning
/// each position to the candidate tile (within `eps` of the ORIGINAL tile's
/// pixels) that maximizes the current adaptive-model count in the
/// (above, left) context chain, ties -> original. Contexts use the
/// already-re-assigned neighbors, exactly what the real encoder will see,
/// and the model updates with the chosen symbol at every level (the doc's
/// cm simulation).
fn rdo_sweep(cg: &CharGrids, analyses: &HashMap<u16, DictAnalysis>, eps: u32) -> SweepResult {
    let mut ctx2: HashMap<u32, HashMap<u16, u32>> = HashMap::new();
    let mut ctx1: HashMap<u16, HashMap<u16, u32>> = HashMap::new();
    let mut ctx0: HashMap<u16, u32> = HashMap::new();
    let mut out_grids = Vec::with_capacity(cg.grids.len());
    let mut per_grid_changed = Vec::with_capacity(cg.grids.len());
    let mut changed = 0u64;
    let mut est_bits = 0.0f64;
    let uniform_bits = (cg.alphabet as f64).log2();

    for (g, grid) in cg.grids.iter().enumerate() {
        let cols = cg.dims[g].0 as usize;
        let near = &analyses[&cg.dict_of[g]].near;
        let mut out: Vec<u16> = Vec::with_capacity(grid.len());
        let mut changed_here = 0u32;
        for (i, &orig) in grid.iter().enumerate() {
            let above = if i >= cols { out[i - cols] } else { 0xFFFF };
            let left = if i % cols > 0 { out[i - 1] } else { 0xFFFF };
            let key2 = ((above as u32) << 16) | left as u32;

            let mut chosen = orig;
            let cands = &near[orig as usize];
            if !cands.is_empty() {
                let m2 = ctx2.get(&key2);
                let m1 = ctx1.get(&above);
                // Score = counts down the escape chain, lexicographic; a
                // candidate must strictly beat the original to replace it.
                let score = |x: u16| -> (u32, u32, u32) {
                    (
                        m2.and_then(|m| m.get(&x)).copied().unwrap_or(0),
                        m1.and_then(|m| m.get(&x)).copied().unwrap_or(0),
                        ctx0.get(&x).copied().unwrap_or(0),
                    )
                };
                let mut best = score(orig);
                for &(d, cand) in cands {
                    if d > eps {
                        continue;
                    }
                    let s = score(cand);
                    if s > best {
                        best = s;
                        chosen = cand;
                    }
                }
            }
            if chosen != orig {
                changed_here += 1;
            }

            let mut coded = false;
            est_bits += ppm_level(ctx2.entry(key2).or_default(), chosen, &mut coded);
            est_bits += ppm_level(ctx1.entry(above).or_default(), chosen, &mut coded);
            est_bits += ppm_level(&mut ctx0, chosen, &mut coded);
            if !coded {
                est_bits += uniform_bits;
            }
            out.push(chosen);
        }
        changed += changed_here as u64;
        per_grid_changed.push(changed_here);
        out_grids.push(out);
    }
    SweepResult {
        grids: out_grids,
        per_grid_changed,
        changed,
        est_bits,
    }
}

// ---------------------------------------------------------------------------
// Preview PNGs
// ---------------------------------------------------------------------------

/// Render one grid into an RGBA canvas at `(x0, 0)`. Transparent key pixels
/// stay alpha 0; the shadow key renders as its raw color (pure blue) — it is
/// exact-matched by construction, so both panes show it identically.
fn render_grid(
    buf: &mut [u8],
    buf_w: usize,
    x0: usize,
    dict: &FrameDictionary,
    cols: usize,
    rows: usize,
    indices: &[u16],
) {
    for y in 0..rows {
        for c in 0..cols {
            let px4 = dict.lookup_pixels(indices[y * cols + c]);
            for (k, &px) in px4.iter().enumerate() {
                if px == TRANSPARENT_COLOR_16 {
                    continue;
                }
                let o = (y * buf_w + x0 + c * 4 + k) * 4;
                buf[o..o + 3].copy_from_slice(&rgb565_to_rgb8(px));
                buf[o + 3] = 255;
            }
        }
    }
}

fn side_by_side_png(
    path: &PathBuf,
    dict: &FrameDictionary,
    cols: usize,
    rows: usize,
    orig: &[u16],
    modified: &[u16],
) -> Result<()> {
    const GAP: usize = 4;
    let w = cols * 4;
    let total_w = w * 2 + GAP;
    let mut buf = vec![0u8; total_w * rows * 4];
    render_grid(&mut buf, total_w, 0, dict, cols, rows, orig);
    render_grid(&mut buf, total_w, w + GAP, dict, cols, rows, modified);
    write_png(path, total_w as u32, rows as u32, true, &buf)
}

/// Pick (grid index, tag) for the three representative sprites: largest,
/// median tile count, and most substitutions (deduplicated in that order).
fn pick_representatives(cg: &CharGrids, per_grid_changed: &[u32]) -> Vec<(usize, &'static str)> {
    let tiles = |g: usize| cg.grids[g].len();
    let mut order: Vec<usize> = (0..cg.grids.len()).collect();
    order.sort_by_key(|&g| (tiles(g), g));
    let largest = *order.last().unwrap();
    let median = order[order.len() / 2];
    let mut picks = vec![(largest, "largest")];
    if median != largest {
        picks.push((median, "median"));
    }
    if let Some(most) = (0..cg.grids.len())
        .filter(|g| picks.iter().all(|&(p, _)| p != *g))
        .max_by_key(|&g| (per_grid_changed[g], tiles(g)))
    {
        picks.push((most, "mostsub"));
    }
    picks
}

// ---------------------------------------------------------------------------
// Per-character driver
// ---------------------------------------------------------------------------

struct CharResult {
    name: String,
    n_tiles: u64,
    baseline: u64,
    canonical: u64,
    /// Per eps: (real coded size, positions changed).
    eps: Vec<(u32, u64, u64)>,
}

fn run_character(
    holder: &FrameHolder,
    data_dir: &PathBuf,
    name: &str,
    eps_list: &[u32],
    preview_dir: &PathBuf,
    previews: bool,
) -> Result<CharResult> {
    let ids = char_frame_ids(data_dir, name)?;
    let cg = collect_grids(holder, &ids)?;
    if cg.grids.is_empty() {
        bail!("{name}: no VQ sprites");
    }
    let max_eps = eps_list.iter().copied().max().unwrap_or(0);

    println!(
        "## {name}: {} VQ sprites, {} tiles, alphabet {}",
        cg.grids.len(),
        cg.n_tiles,
        cg.alphabet
    );

    // Dictionary analysis (characters have one dictionary each, but stay
    // correct if several appear).
    let mut dict_ids: Vec<u16> = cg.dict_of.clone();
    dict_ids.sort_unstable();
    dict_ids.dedup();
    let mut analyses: HashMap<u16, DictAnalysis> = HashMap::new();
    for &di in &dict_ids {
        let dict = holder
            .dictionary(di)
            .ok_or_else(|| anyhow!("missing dictionary {di}"))?;
        let t0 = std::time::Instant::now();
        let an = analyze_dictionary(dict, max_eps);
        println!(
            "  dict {di}: {} entries, {} duplicate groups, {} redundant entries  ({:.1}s pairwise)",
            dict.num_entries(),
            an.dup_groups,
            an.dup_extra,
            t0.elapsed().as_secs_f64()
        );
        let hist: Vec<String> = NN_LABELS
            .iter()
            .zip(an.nn_hist.iter())
            .map(|(l, c)| format!("{l}: {c}"))
            .collect();
        println!(
            "  NN-distance histogram: {}  no-candidate: {}",
            hist.join("  "),
            an.nn_none
        );
        analyses.insert(di, an);
    }

    // Baseline: real codec on the untouched grids.
    let t0 = std::time::Instant::now();
    let baseline = coded_size(cg.alphabet, &cg.dims, &cg.grids)?;
    println!(
        "  baseline real codec:   {baseline} B ({:.3} bits/tile, enc {:.1}s)",
        baseline as f64 * 8.0 / cg.n_tiles as f64,
        t0.elapsed().as_secs_f64()
    );

    // Lossless: canonicalize exact duplicates to the group representative.
    let mut canon_changed = 0u64;
    let canon_grids: Vec<Vec<u16>> = cg
        .grids
        .iter()
        .enumerate()
        .map(|(g, grid)| {
            let canonical = &analyses[&cg.dict_of[g]].canonical;
            grid.iter()
                .map(|&x| {
                    let y = canonical[x as usize];
                    if y != x {
                        canon_changed += 1;
                    }
                    y
                })
                .collect()
        })
        .collect();
    let canonical = coded_size(cg.alphabet, &cg.dims, &canon_grids)?;
    println!(
        "  exact-dup canonical:   {canonical} B ({:+.3}% vs baseline), {canon_changed} positions retargeted ({:.3}%) — lossless",
        100.0 * (canonical as f64 - baseline as f64) / baseline as f64,
        100.0 * canon_changed as f64 / cg.n_tiles as f64
    );

    // Greedy RDO per eps.
    let mut eps_rows = Vec::new();
    for &eps in eps_list {
        let t0 = std::time::Instant::now();
        let sweep = rdo_sweep(&cg, &analyses, eps);
        let size = coded_size(cg.alphabet, &cg.dims, &sweep.grids)?;
        println!(
            "  eps={eps} greedy RDO:     {size} B ({:+.3}% vs baseline), {} positions changed ({:.3}%), cm-sim estimate {:.0} B  ({:.1}s sweep+enc)",
            100.0 * (size as f64 - baseline as f64) / baseline as f64,
            sweep.changed,
            100.0 * sweep.changed as f64 / cg.n_tiles as f64,
            sweep.est_bits / 8.0,
            t0.elapsed().as_secs_f64()
        );

        if previews {
            fs::create_dir_all(preview_dir)?;
            let safe: String = name.replace(' ', "_");
            for (g, tag) in pick_representatives(&cg, &sweep.per_grid_changed) {
                let dict = holder.dictionary(cg.dict_of[g]).unwrap();
                let (cols, rows) = (cg.dims[g].0 as usize, cg.dims[g].1 as usize);
                let path = preview_dir.join(format!(
                    "{safe}_eps{eps}_{tag}_id{}_sub{}.png",
                    cg.ids[g], sweep.per_grid_changed[g]
                ));
                side_by_side_png(&path, dict, cols, rows, &cg.grids[g], &sweep.grids[g])?;
                println!(
                    "    preview {tag}: {} ({}x{}, {} of {} tiles substituted)",
                    path.display(),
                    cols * 4,
                    rows,
                    sweep.per_grid_changed[g],
                    cg.grids[g].len()
                );
            }
        }
        eps_rows.push((eps, size, sweep.changed));
    }

    Ok(CharResult {
        name: name.to_string(),
        n_tiles: cg.n_tiles,
        baseline,
        canonical,
        eps: eps_rows,
    })
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
    let mut eps_list = cli.eps.clone();
    eps_list.sort_unstable();
    eps_list.dedup();

    let mut results = Vec::new();
    for name in &cli.chars {
        results.push(run_character(
            &holder,
            &cli.data_dir,
            name,
            &eps_list,
            &cli.preview_dir,
            !cli.no_previews,
        )?);
    }

    println!("\n## summary (real codec bytes)");
    let eps_hdr: Vec<String> = eps_list
        .iter()
        .map(|e| format!("{:>12}", format!("eps={e}")))
        .collect();
    println!(
        "{:<14} {:>12} {:>12} {}",
        "character",
        "baseline",
        "canonical",
        eps_hdr.join(" ")
    );
    let mut tot = vec![0u64; 2 + eps_list.len()];
    for r in &results {
        let eps_cols: Vec<String> = r.eps.iter().map(|&(_, s, _)| format!("{s:>12}")).collect();
        println!(
            "{:<14} {:>12} {:>12} {}",
            r.name,
            r.baseline,
            r.canonical,
            eps_cols.join(" ")
        );
        tot[0] += r.baseline;
        tot[1] += r.canonical;
        for (i, &(_, s, _)) in r.eps.iter().enumerate() {
            tot[2 + i] += s;
        }
    }
    let tot_cols: Vec<String> = tot[2..].iter().map(|s| format!("{s:>12}")).collect();
    println!(
        "{:<14} {:>12} {:>12} {}",
        "TOTAL",
        tot[0],
        tot[1],
        tot_cols.join(" ")
    );
    for (i, &eps) in eps_list.iter().enumerate() {
        println!(
            "  eps={eps}: {:+.3}% vs baseline total",
            100.0 * (tot[2 + i] as f64 - tot[0] as f64) / tot[0] as f64
        );
    }
    println!(
        "  canonical: {:+.4}% vs baseline total (lossless)",
        100.0 * (tot[1] as f64 - tot[0] as f64) / tot[0] as f64
    );
    let n_tiles: u64 = results.iter().map(|r| r.n_tiles).sum();
    println!("  tiles total: {n_tiles}");
    Ok(())
}
