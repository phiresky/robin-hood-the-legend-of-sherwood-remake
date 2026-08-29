//! Measurement experiments for the sprite-compression campaign
//! (docs/COMPRESSION.md, "Sprite research" / "sprite_codec" sections).
//!
//! Three questions, three modes — all measured with the *real*
//! `robin_assets::sprite_codec` range coder against the loose fullgame bank:
//!
//! 1. `--topology` — the 9 palette-variant families currently code every
//!    variant against the lexicographically-first member (star around
//!    member 0). Would a different star base, or a chain
//!    (m0, m1|m0, m2|m1, …), code smaller? Full pairwise vs-matrix per
//!    family plus per-member standalone sizes.
//! 2. `--order` — the codec walks sprites in bank-id order. Does feeding
//!    the same sprite set in animation-script order (first occurrence of
//!    each frame id walking all profiles/scripts in file order) help the
//!    adaptive model?
//! 3. `--mirror` — direction d and (16-d)%16 of one action are left/right
//!    mirrored camera views, but lighting is not mirrored. How much do the
//!    mirrored pixels predict? Conditional entropy of the VQ tile index
//!    given the 4 RGB565 pixels at the horizontally mirrored position of
//!    the paired sprite, plus exact-pixel mirror-match rate.
//!
//!   cargo run --release --example sprite_probe_experiments -- \
//!       --data-dir datadirs/fullgame_linux --topology --order --mirror
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;

use robin_assets::frame_holder::{FrameHolder, TRANSPARENT_COLOR_16, UNMAPPED_DICT};
use robin_assets::sprite_codec::{SpriteGrid, encode_grids};
use robin_engine::sprite_script::SpriteScriptor;

#[derive(Parser, Debug)]
#[command(about = "Sprite codec measurement experiments (topology / order / mirror)")]
struct Cli {
    /// Data directory (the one containing `Data/…` or `DATA/…`).
    #[arg(long, default_value = "datadirs/fullgame_linux")]
    data_dir: PathBuf,

    /// Experiment 1: family base-topology sweep (star around every member
    /// + chain), with the real codec.
    #[arg(long)]
    topology: bool,

    /// Restrict --topology to families whose prefix contains one of these
    /// substrings (useful for a quick single-family run).
    #[arg(long)]
    family: Vec<String>,

    /// Experiment 2: bank-id order vs script order for the codec stream.
    #[arg(long)]
    order: bool,

    /// Characters for --order.
    #[arg(long, default_values_t = [
        "RobinTown".to_string(),
        "Knight01".to_string(),
        "Guard A00".to_string(),
    ])]
    order_chars: Vec<String>,

    /// Experiment 3: mirrored-direction prediction entropy.
    #[arg(long)]
    mirror: bool,

    /// Characters for --mirror.
    #[arg(long, default_values_t = [
        "Knight01".to_string(),
        "RobinTown".to_string(),
    ])]
    mirror_chars: Vec<String>,
}

// ---------------------------------------------------------------------------
// Helpers copied from sprite_compression_probe.rs (that example is owned by
// another agent; keep these in sync manually if the probe changes).
// ---------------------------------------------------------------------------

fn rhs_path(data_dir: &Path, name: &str) -> Result<PathBuf> {
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
fn char_frame_ids(data_dir: &Path, name: &str) -> Result<(Vec<u32>, Vec<u32>)> {
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

fn positional_pairs(data_dir: &Path, a: &str, b: &str) -> Result<Vec<(u32, u32)>> {
    let (sa, _) = char_frame_ids(data_dir, a)?;
    let (sb, _) = char_frame_ids(data_dir, b)?;
    let mut pairs: Vec<(u32, u32)> = sa.iter().copied().zip(sb.iter().copied()).collect();
    pairs.sort_unstable();
    pairs.dedup();
    Ok(pairs)
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

// ---------------------------------------------------------------------------
// Experiment 1: family base topology
// ---------------------------------------------------------------------------

/// Encode one member standalone with the real codec; returns coded bytes.
fn code_standalone(holder: &FrameHolder, data_dir: &Path, name: &str) -> Result<u64> {
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
    let t0 = Instant::now();
    let blob = encode_grids(alphabet, &grids, None)?;
    eprintln!(
        "#   standalone {name}: {} bytes ({:.1}s)",
        blob.len(),
        t0.elapsed().as_secs_f64()
    );
    Ok(blob.len() as u64)
}

/// Encode member `b` against base `a` (positional pairing, base slices);
/// returns coded bytes.
fn code_vs(holder: &FrameHolder, data_dir: &Path, a: &str, b: &str) -> Result<u64> {
    let pairs = positional_pairs(data_dir, a, b)?;
    let (dims, slices, bases, alphabet, unbased) = codec_grids2(holder, &pairs)?;
    if unbased > 0 {
        eprintln!(
            "#   NOTE {a} -> {b}: {unbased}/{} grids unbased",
            slices.len()
        );
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
    let t0 = Instant::now();
    let blob = encode_grids(alphabet, &grids, Some(&bases))?;
    eprintln!(
        "#   {a} -> {b}: {} bytes ({:.1}s)",
        blob.len(),
        t0.elapsed().as_secs_f64()
    );
    Ok(blob.len() as u64)
}

/// Detect variant families exactly like the probe's `--corpus`: family key =
/// name with trailing two digits stripped, families need >1 member; members
/// sorted lexicographically (member 0 = today's implicit star base).
fn detect_families(data_dir: &Path) -> Result<BTreeMap<String, Vec<String>>> {
    let mut chars_dir = data_dir.join("Data/Characters");
    if !chars_dir.is_dir() {
        chars_dir = data_dir.join("DATA/Characters");
    }
    let mut names: Vec<String> = fs::read_dir(&chars_dir)
        .with_context(|| format!("read {}", chars_dir.display()))?
        .filter_map(|e| {
            let p = e.ok()?.path();
            (p.extension()?.to_str()? == "rhs")
                .then(|| p.file_stem().unwrap().to_string_lossy().into_owned())
        })
        .collect();
    names.sort();
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
    Ok(families)
}

fn topology(holder: &FrameHolder, data_dir: &Path, filter: &[String]) -> Result<()> {
    let mut families = detect_families(data_dir)?;
    if !filter.is_empty() {
        families.retain(|k, _| filter.iter().any(|f| k.contains(f.as_str())));
    }
    eprintln!("# topology: {} families", families.len());

    struct FamilyResult {
        prefix: String,
        members: Vec<String>,
        all_standalone: u64,
        current: u64,
        best_star_base: usize,
        best_star: u64,
        chain: u64,
    }
    let mut results: Vec<FamilyResult> = Vec::new();

    for (prefix, members) in &families {
        let n = members.len();
        eprintln!("# family {prefix} ({n} members: {})", members.join(", "));
        let mut standalone = vec![0u64; n];
        for (i, m) in members.iter().enumerate() {
            standalone[i] = code_standalone(holder, data_dir, m)?;
        }
        // vs[b][m] = coded bytes of member m against base b (diagonal unused).
        let mut vs = vec![vec![0u64; n]; n];
        for b in 0..n {
            for m in 0..n {
                if b != m {
                    vs[b][m] = code_vs(holder, data_dir, &members[b], &members[m])?;
                }
            }
        }

        // Per-family detail: standalone sizes + the full matrix.
        println!("## topology {prefix}");
        println!("  {:<16} standalone", "member");
        for (i, m) in members.iter().enumerate() {
            println!("  {:<16} {:>10}", m, standalone[i]);
        }
        println!("  vs-matrix (row = base, col = coded member):");
        print!("  {:<16}", "base \\ member");
        for m in members {
            print!(" {:>10}", &m[m.len().saturating_sub(10)..]);
        }
        println!();
        for b in 0..n {
            print!("  {:<16}", members[b]);
            for (m, coded) in vs[b].iter().enumerate() {
                if b == m {
                    print!(" {:>10}", "-");
                } else {
                    print!(" {:>10}", coded);
                }
            }
            println!();
        }

        let star = |b: usize| -> u64 {
            standalone[b] + (0..n).filter(|&m| m != b).map(|m| vs[b][m]).sum::<u64>()
        };
        let current = star(0);
        let (best_star_base, best_star) = (0..n)
            .map(|b| (b, star(b)))
            .min_by_key(|&(_, t)| t)
            .unwrap();
        let chain = standalone[0] + (0..n - 1).map(|i| vs[i][i + 1]).sum::<u64>();
        for (b, member) in members.iter().enumerate() {
            println!(
                "  star@{:<14} total {:>10}{}",
                member,
                star(b),
                if b == 0 { "  (current)" } else { "" }
            );
        }
        println!("  chain m0->m1->..   total {:>10}", chain);
        results.push(FamilyResult {
            prefix: prefix.clone(),
            members: members.clone(),
            all_standalone: standalone.iter().sum(),
            current,
            best_star_base,
            best_star,
            chain,
        });
    }

    println!();
    println!("## topology summary");
    println!(
        "{:<14} {:>2} {:>12} {:>12} {:<14} {:>12} {:>12}  verdict",
        "family", "n", "standalone", "current", "best base", "best star", "chain"
    );
    for r in &results {
        let pct = |x: u64| 100.0 * (x as f64 - r.current as f64) / r.current as f64;
        let verdict = if r.chain < r.best_star {
            format!("chain ({:+.2}%)", pct(r.chain))
        } else if r.best_star_base != 0 {
            format!(
                "star@{} ({:+.2}%)",
                r.members[r.best_star_base],
                pct(r.best_star)
            )
        } else {
            "keep current".to_string()
        };
        println!(
            "{:<14} {:>2} {:>12} {:>12} {:<14} {:>12} {:>12}  {verdict}",
            r.prefix,
            r.members.len(),
            r.all_standalone,
            r.current,
            r.members[r.best_star_base],
            r.best_star,
            r.chain,
        );
    }
    let (t_cur, t_star, t_chain): (u64, u64, u64) = results.iter().fold((0, 0, 0), |acc, r| {
        (
            acc.0 + r.current,
            acc.1 + r.best_star,
            acc.2 + r.chain.min(r.best_star),
        )
    });
    println!(
        "TOTAL current {t_cur}  best-star {t_star} ({:+.2}%)  best-any {t_chain} ({:+.2}%)",
        100.0 * (t_star as f64 - t_cur as f64) / t_cur as f64,
        100.0 * (t_chain as f64 - t_cur as f64) / t_cur as f64,
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Experiment 2: sprite coding order
// ---------------------------------------------------------------------------

fn order(holder: &FrameHolder, data_dir: &Path, name: &str) -> Result<()> {
    let (script_order, bank_order) = char_frame_ids(data_dir, name)?;
    // Script order = first occurrence of each frame id walking all
    // profiles/scripts/frame_ids in file order.
    let mut seen: HashSet<u32> = HashSet::new();
    let script_first: Vec<u32> = script_order
        .iter()
        .copied()
        .filter(|id| seen.insert(*id))
        .collect();
    if script_first.len() != bank_order.len() {
        bail!(
            "{name}: script first-occurrence has {} ids, bank order {}",
            script_first.len(),
            bank_order.len()
        );
    }

    let mut sizes = Vec::new();
    for (label, ids) in [
        ("bank-id order", &bank_order),
        ("script order", &script_first),
    ] {
        let (_gids, dims, slices, alphabet) = codec_grids(holder, ids)?;
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
        let t0 = Instant::now();
        let blob = encode_grids(alphabet, &grids, None)?;
        eprintln!(
            "#   {name} {label}: {} bytes ({:.1}s)",
            blob.len(),
            t0.elapsed().as_secs_f64()
        );
        sizes.push((label, grids.len(), n_tiles, blob.len() as u64));
    }
    println!("## order {name}");
    for (label, n_grids, n_tiles, bytes) in &sizes {
        println!(
            "  {label:<16} {n_grids} sprites, {n_tiles} tiles -> {bytes:>10} bytes ({:.3} bits/tile)",
            *bytes as f64 * 8.0 / *n_tiles as f64
        );
    }
    let (a, b) = (sizes[0].3, sizes[1].3);
    println!(
        "  script vs bank: {:+} bytes ({:+.3}%)",
        b as i64 - a as i64,
        100.0 * (b as f64 - a as f64) / a as f64
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Experiment 3: mirrored-direction prediction
// ---------------------------------------------------------------------------

/// Conditional entropy from flat joint counts: H(X | ctx) =
/// Σ_{ctx,x} p(ctx,x) · −log2 p(x|ctx). (Same math as the probe's entropy2,
/// flat-mapped to keep millions of pixel-quad contexts affordable.)
fn cond_entropy_flat(joint: &HashMap<(u64, u16), u64>, ctx_tot: &HashMap<u64, u64>) -> f64 {
    let total: u64 = ctx_tot.values().sum();
    let mut bits = 0.0f64;
    for (&(ctx, _x), &n) in joint {
        let cn = ctx_tot[&ctx];
        bits -= (n as f64 / total as f64) * (n as f64 / cn as f64).log2();
    }
    bits
}

fn mirror(holder: &FrameHolder, data_dir: &Path, name: &str) -> Result<()> {
    let path = rhs_path(data_dir, name)?;
    let (_sig, profiles) = SpriteScriptor::load_all_profiles(path.to_str().unwrap())
        .map_err(|e| anyhow!("load rhs {}: {e}", path.display()))?;

    // Group rows by (profile, action id). Rows sharing an action id are the
    // 16 directions, in file order. Pair direction d with (16-d)%16 for
    // d = 1..=7 (d = 0 and d = 8 pair with themselves), frames positionally.
    let mut n_groups = 0u64;
    let mut n_skipped_groups = 0u64;
    let mut frame_pairs: Vec<(u32, u32)> = Vec::new();
    for (_pname, info) in &profiles {
        let mut by_action: BTreeMap<u16, Vec<&robin_engine::sprite_script::SpriteScript>> =
            BTreeMap::new();
        for s in info.scripts.iter() {
            by_action.entry(s.action_id).or_default().push(s);
        }
        for rows in by_action.values() {
            if rows.len() != 16 {
                n_skipped_groups += 1;
                continue;
            }
            n_groups += 1;
            for d in 1..=7usize {
                let (ra, rb) = (rows[d], rows[16 - d]);
                for (&ia, &ib) in ra.frame_ids.iter().zip(rb.frame_ids.iter()) {
                    frame_pairs.push((ia, ib));
                }
            }
        }
    }
    frame_pairs.sort_unstable();
    frame_pairs.dedup();

    let mut n_pairs = 0u64;
    let mut n_dim_mismatch = 0u64;
    let mut n_tiles = 0u64;
    let mut n_exact = 0u64;
    // Contexts: the 4 mirrored RGB565 pixels pack exactly into a u64 (no
    // lossy hashing needed). The |above and order-0 baselines are computed
    // over the same tile sample for a fair comparison.
    let mut mj: HashMap<(u64, u16), u64> = HashMap::new();
    let mut mt: HashMap<u64, u64> = HashMap::new();
    let mut aj: HashMap<(u64, u16), u64> = HashMap::new();
    let mut at: HashMap<u64, u64> = HashMap::new();
    let mut h0: HashMap<u16, u64> = HashMap::new();
    for &(ia, ib) in &frame_pairs {
        let (Some((wa, ha, da)), Some((wb, hb, db))) =
            (decode_raw(holder, ia), decode_raw(holder, ib))
        else {
            continue;
        };
        n_pairs += 1;
        if (wa, ha) != (wb, hb) {
            n_dim_mismatch += 1;
            continue;
        }
        let spb = &holder.sprites()[ib as usize];
        if spb.dictionary_index == UNMAPPED_DICT {
            continue;
        }
        let Some(pb) = holder.packed_data(ib) else {
            continue;
        };
        let w = wb as usize;
        let h = hb as usize;
        let per_row = w / 4;
        for row in 0..h {
            for col in 0..per_row {
                let sym = pb[row * per_row + col];
                // Mirror context: the pixels of A's row at the horizontally
                // mirrored positions of this tile's pixels, in B-tile pixel
                // order (x_A = w-1-x_B — the mirrored tile spans reversed
                // pixel order in A).
                let mut ctx = 0u64;
                let mut mpix = [0u16; 4];
                for (k, mp) in mpix.iter_mut().enumerate() {
                    let xb = col * 4 + k;
                    let p = da[row * w + (w - 1 - xb)];
                    *mp = p;
                    ctx = (ctx << 16) | p as u64;
                }
                let btile = &db[row * w + col * 4..row * w + col * 4 + 4];
                if btile == mpix {
                    n_exact += 1;
                }
                let above = if row > 0 {
                    pb[(row - 1) * per_row + col] as u64
                } else {
                    0xFFFF
                };
                n_tiles += 1;
                *mj.entry((ctx, sym)).or_default() += 1;
                *mt.entry(ctx).or_default() += 1;
                *aj.entry((above, sym)).or_default() += 1;
                *at.entry(above).or_default() += 1;
                *h0.entry(sym).or_default() += 1;
            }
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
    println!("## mirror {name}");
    println!("  action groups:        {n_groups} with 16 rows ({n_skipped_groups} skipped)");
    println!(
        "  unique frame pairs:   {n_pairs}, same dims {} ({:.1}%)",
        n_pairs - n_dim_mismatch,
        100.0 * (n_pairs - n_dim_mismatch) as f64 / n_pairs.max(1) as f64
    );
    println!(
        "  tiles:                {n_tiles}, exact mirrored-pixel match {n_exact} ({:.2}%)",
        100.0 * n_exact as f64 / n_tiles.max(1) as f64
    );
    for (label, bits, nctx) in [
        ("order-0", h0_bits, h0.len()),
        ("| above", cond_entropy_flat(&aj, &at), at.len()),
        ("| mirror-4px", cond_entropy_flat(&mj, &mt), mt.len()),
    ] {
        println!(
            "  {label:<12} {bits:>6.3} bits/tile  -> {:>9.0} bytes  ({nctx} contexts)",
            bits * n_tiles as f64 / 8.0
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------

fn main() -> Result<()> {
    let cli = Cli::parse();
    if !(cli.topology || cli.order || cli.mirror) {
        bail!("nothing to do: pass --topology, --order and/or --mirror");
    }
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
    if cli.mirror {
        for name in &cli.mirror_chars {
            mirror(&holder, &cli.data_dir, name)?;
        }
    }
    if cli.order {
        for name in &cli.order_chars {
            order(&holder, &cli.data_dir, name)?;
        }
    }
    if cli.topology {
        topology(&holder, &cli.data_dir, &cli.family)?;
    }
    Ok(())
}
