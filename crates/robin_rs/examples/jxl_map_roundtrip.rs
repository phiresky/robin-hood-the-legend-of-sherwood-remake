//! Quick smoke test: load a shipping `datadir.bin`, find every `.map` entry
//! in the boot manifest and lazy mission files, and decode each one.
//! Prints `key  WxH  jxl|sixteen  ok|err`.
//!
//!   cargo run --release --example jxl_map_roundtrip -- <path-to-datadir.bin>
#![allow(clippy::print_stdout)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use robin_assets::picture::Picture;
use robin_assets::shipping_datadir::{ShippingDatadir, decode_mission_compressed};

fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("usage: jxl_map_roundtrip <datadir.bin>"))?;

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let dd = ShippingDatadir::load_from_file(&path)?;
    println!("# loaded {} ({} raw entries)", path.display(), dd.raw.len());

    let mut terrain = BTreeMap::<String, Vec<u8>>::new();
    terrain.extend(
        dd.raw
            .iter()
            .filter(|(key, _)| key.ends_with(".map") || key.ends_with(".min"))
            .map(|(key, bytes)| (key.clone(), bytes.clone())),
    );
    let root = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let mut seen_files = std::collections::BTreeSet::new();
    for reference in dd.missions.values() {
        for file in &reference.files {
            if !seen_files.insert(file) {
                continue;
            }
            let payload_path = root.join(file);
            let compressed = std::fs::read(&payload_path)
                .with_context(|| format!("read {}", payload_path.display()))?;
            let payload = decode_mission_compressed(&compressed)?;
            terrain.extend(
                payload
                    .raw
                    .into_iter()
                    .filter(|(key, _)| key.ends_with(".map") || key.ends_with(".min")),
            );
        }
    }
    if terrain.is_empty() {
        println!("# no .map/.min entries in shipping files");
        return Ok(());
    }

    println!(
        "{:<48} {:>12} {:>9} {:>14} result",
        "key", "bytes", "format", "dims"
    );
    for (k, bytes) in &terrain {
        let format = match bytes.first().copied() {
            Some(0xFF) if bytes.get(1) == Some(&0x0A) => "jxl",
            Some(0x00) if bytes.starts_with(b"\x00\x00\x00\x0CJXL ") => "jxl",
            _ => "sixteen",
        };
        let t0 = std::time::Instant::now();
        match Picture::load_terrain_from_bytes(bytes) {
            Ok(p) => println!(
                "{:<48} {:>12} {:>9} {:>14} ok ({:.2}s)",
                k,
                bytes.len(),
                format,
                format!("{}×{}", p.width, p.height),
                t0.elapsed().as_secs_f32(),
            ),
            Err(e) => println!(
                "{:<48} {:>12} {:>9} {:>14} ERR: {}",
                k,
                bytes.len(),
                format,
                "-",
                e
            ),
        }
    }
    Ok(())
}
