//! Print a size breakdown of each field in a shipping `datadir.bin`.
//!
//!   cargo run --release --example datadir_breakdown -- <path-to-datadir.bin>
//!
//! For each top-level `ShippingDatadir` field, reports both the raw
//! bitcode-serialized size and the zstd-22-long-31-compressed size in
//! isolation. The sum of compressed sizes is larger than the actual
//! blob because cross-field LZ matches are lost, but it shows where
//! the storage budget is going.
#![allow(clippy::print_stdout)]

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use bitcode::serialize;

use robin_assets::shipping_datadir::{ShippingDatadir, zstd_max_compress};

fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("usage: datadir_breakdown <datadir.bin>"))?;

    let on_disk = std::fs::metadata(&path)
        .with_context(|| format!("stat {}", path.display()))?
        .len();
    let dd = ShippingDatadir::load_from_file(&path)?;

    let full = serialize(&dd).map_err(|e| anyhow!("bitcode {e:?}"))?;
    let full_z = zstd_max_compress(&full)?;
    println!(
        "# {}: on-disk = {}, re-serialised raw = {}, re-compressed = {}",
        path.display(),
        human(on_disk),
        human(full.len() as u64),
        human(full_z.len() as u64),
    );
    if full_z.len() as u64 != on_disk {
        println!(
            "# (note: re-compressed differs from on-disk by {} — probably \
             rounding in the initial compression)",
            human((full_z.len() as u64).abs_diff(on_disk))
        );
    }

    println!();
    println!(
        "{:<24} {:>8} {:>12} {:>12} {:>7}",
        "field", "entries", "bitcode raw", "zstd22", "% raw",
    );

    let total_raw = full.len() as u64;
    let mut rows: Vec<(&'static str, usize, u64, u64)> = Vec::new();

    // Helper: serialize a single field and measure.
    macro_rules! field {
        ($name:expr, $entries:expr, $expr:expr) => {{
            let bytes = serialize(&$expr).map_err(|e| anyhow!("bitcode {e:?}"))?;
            let z = zstd_max_compress(&bytes)?;
            rows.push(($name, $entries, bytes.len() as u64, z.len() as u64));
        }};
    }

    field!(
        "profiles",
        if dd.profiles.is_some() { 1 } else { 0 },
        dd.profiles
    );
    field!("res_files", dd.res_files.len(), dd.res_files);
    field!("pak_files", dd.pak_files.len(), dd.pak_files);
    field!("red_files", dd.red_files.len(), dd.red_files);
    field!("levels", dd.levels.len(), dd.levels);
    field!("scripts", dd.scripts.len(), dd.scripts);
    field!("rhs_files", dd.rhs_files.len(), dd.rhs_files);
    field!(
        "sprite_bank",
        if dd.sprite_bank.is_some() { 1 } else { 0 },
        dd.sprite_bank
    );
    field!("raw", dd.raw.len(), dd.raw);

    // Sort descending by compressed size so the biggest is first.
    rows.sort_by_key(|r| std::cmp::Reverse(r.3));

    let mut sum_raw = 0u64;
    let mut sum_z = 0u64;
    for (name, entries, raw, z) in &rows {
        let pct = if total_raw > 0 {
            100.0 * *raw as f64 / total_raw as f64
        } else {
            0.0
        };
        println!(
            "{:<24} {:>8} {:>12} {:>12} {:>6.1}%",
            name,
            entries,
            human(*raw),
            human(*z),
            pct,
        );
        sum_raw += raw;
        sum_z += z;
    }
    println!(
        "{:<24} {:>8} {:>12} {:>12}",
        "(sum per-field)",
        "",
        human(sum_raw),
        human(sum_z),
    );
    println!(
        "# per-field zstd sum ({}) > whole-blob zstd ({}) because cross-field LZ \
         matches are lost when compressing in isolation.",
        human(sum_z),
        human(full_z.len() as u64),
    );

    // Sprite bank sub-breakdown (it's usually the lion's share).
    if let Some(bank) = &dd.sprite_bank {
        println!();
        println!("## Sprite bank sub-breakdown");
        let sig_bytes = serialize(&bank.signature).map_err(|e| anyhow!("{e:?}"))?;
        let dicts_bytes = serialize(&bank.dictionaries).map_err(|e| anyhow!("{e:?}"))?;
        let sprites_bytes = serialize(&bank.sprites).map_err(|e| anyhow!("{e:?}"))?;
        let sig_z = zstd_max_compress(&sig_bytes)?;
        let dicts_z = zstd_max_compress(&dicts_bytes)?;
        let sprites_z = zstd_max_compress(&sprites_bytes)?;
        println!(
            "{:<24} {:>8} {:>12} {:>12}",
            "subfield", "entries", "bitcode raw", "zstd22",
        );
        println!(
            "{:<24} {:>8} {:>12} {:>12}",
            "signature",
            1,
            human(sig_bytes.len() as u64),
            human(sig_z.len() as u64),
        );
        println!(
            "{:<24} {:>8} {:>12} {:>12}",
            "dictionaries",
            bank.dictionaries.len(),
            human(dicts_bytes.len() as u64),
            human(dicts_z.len() as u64),
        );
        println!(
            "{:<24} {:>8} {:>12} {:>12}",
            "sprites (RLE/VQ)",
            bank.sprites.len(),
            human(sprites_bytes.len() as u64),
            human(sprites_z.len() as u64),
        );
    }

    // raw/ sub-breakdown.
    if !dd.raw.is_empty() {
        println!();
        println!("## Raw blobs sub-breakdown");
        let mut raw_rows: Vec<(String, u64, u64)> = Vec::new();
        for (k, v) in &dd.raw {
            let z = zstd_max_compress(v)?;
            raw_rows.push((k.clone(), v.len() as u64, z.len() as u64));
        }
        raw_rows.sort_by_key(|r| std::cmp::Reverse(r.1));
        println!(
            "{:<52} {:>12} {:>12} {:<10}",
            "key", "raw bytes", "zstd22", "decode",
        );
        for (k, raw, z) in &raw_rows {
            // Try to actually decode each .map / .min blob through the
            // unified terrain loader to verify the JXL path works.
            let decode = if k.ends_with(".map") || k.ends_with(".min") {
                let bytes = dd.raw.get(k).unwrap();
                match robin_assets::picture::Picture::load_terrain_from_bytes(bytes) {
                    Ok(p) => format!("{}×{} ok", p.width, p.height),
                    Err(e) => format!("err: {e}"),
                }
            } else {
                "—".into()
            };
            println!(
                "{:<52} {:>12} {:>12} {:<10}",
                truncate(k, 52),
                human(*raw),
                human(*z),
                decode,
            );
        }
    }

    Ok(())
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

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}
