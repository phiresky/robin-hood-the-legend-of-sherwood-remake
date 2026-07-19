//! Quick smoke test: load a shipping `datadir.bin`, find every `.map` entry
//! in `dd.raw`, and try to decode it via `Picture::load_terrain_from_bytes`.
//! Prints `key  WxH  jxl|sixteen  ok|err`.
//!
//!   cargo run --release --example jxl_map_roundtrip -- <path-to-datadir.bin>
#![allow(clippy::print_stdout)]

use std::path::PathBuf;

use anyhow::{Result, anyhow};
use robin_assets::picture::Picture;
use robin_assets::shipping_datadir::ShippingDatadir;

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

    let mut keys: Vec<&String> = dd
        .raw
        .keys()
        .filter(|k| k.ends_with(".map") || k.ends_with(".min"))
        .collect();
    keys.sort();
    if keys.is_empty() {
        println!("# no .map/.min entries in dd.raw");
        return Ok(());
    }

    println!(
        "{:<48} {:>12} {:>9} {:>14} result",
        "key", "bytes", "format", "dims"
    );
    for k in keys {
        let bytes = dd.raw.get(k).unwrap();
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
