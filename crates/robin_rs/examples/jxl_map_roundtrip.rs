//! Quick smoke test: load a shipping `datadir.bin`, find every `.map` entry
//! in the boot manifest and lazy mission files, and decode each one.
//! Prints `key  WxH  jxl|sixteen  ok|err`.
//!
//!   cargo run --release --example jxl_map_roundtrip -- <path-to-datadir.bin>
#![allow(clippy::print_stdout)]

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use robin_assets::picture::Picture;
use robin_assets::shipping_datadir::{ShippingDatadir, decode_mission_compressed};

#[derive(clap::Parser, serde::Serialize, serde::Deserialize)]
struct Args {
    path: PathBuf,
}

fn main() -> Result<()> {
    let path = <Args as clap::Parser>::parse().path;

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();
    run(&path, &mut std::io::stdout().lock())
}

fn run(path: &Path, output: &mut impl Write) -> Result<()> {
    let mut dd = ShippingDatadir::load_from_file(path)?;
    writeln!(
        output,
        "# loaded {} ({} raw entries)",
        path.display(),
        dd.raw.len()
    )?;

    let mut terrain = BTreeMap::<String, Vec<u8>>::new();
    terrain.extend(
        std::mem::take(&mut dd.raw)
            .into_iter()
            .filter(|(key, _)| key.ends_with(".map") || key.ends_with(".min")),
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
                    .payload
                    .raw
                    .into_iter()
                    .filter(|(key, _)| key.ends_with(".map") || key.ends_with(".min")),
            );
        }
    }
    if terrain.is_empty() {
        writeln!(output, "# no .map/.min entries in shipping files")?;
        return Ok(());
    }

    writeln!(
        output,
        "{:<48} {:>12} {:>9} {:>14} result",
        "key", "bytes", "format", "dims"
    )?;
    let mut failed = 0usize;
    for (k, bytes) in &terrain {
        let format = match bytes.first().copied() {
            Some(0xFF) if bytes.get(1) == Some(&0x0A) => "jxl",
            Some(0x00) if bytes.starts_with(b"\x00\x00\x00\x0CJXL ") => "jxl",
            _ => "sixteen",
        };
        let t0 = std::time::Instant::now();
        match Picture::load_terrain_from_bytes(bytes) {
            Ok(p) => writeln!(
                output,
                "{:<48} {:>12} {:>9} {:>14} ok ({:.2}s)",
                k,
                bytes.len(),
                format,
                format!("{}×{}", p.width, p.height),
                t0.elapsed().as_secs_f32(),
            )?,
            Err(e) => {
                failed += 1;
                writeln!(
                    output,
                    "{:<48} {:>12} {:>9} {:>14} ERR: {}",
                    k,
                    bytes.len(),
                    format,
                    "-",
                    e
                )?;
            }
        }
    }
    if failed > 0 {
        return Err(anyhow!(
            "{failed} terrain inspections failed ({} passed)",
            terrain.len() - failed
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_assets::picture::{PixelFormat, SixteenPacking};
    use robin_assets::shipping_datadir::{
        ShippingMission, ShippingMissionRef, encode_mission_native, encode_native,
        zstd_max_compress,
    };

    fn valid_terrain(width: u16) -> Vec<u8> {
        Picture {
            width,
            height: 1,
            pitch: width * 2,
            pixel_format: PixelFormat::Rgb16,
            data: vec![0; usize::from(width) * 2],
            palette: None,
        }
        .write_sixteen_to_bytes(SixteenPacking::None)
        .unwrap()
    }

    fn write_manifest(root: &Path, datadir: &ShippingDatadir) -> PathBuf {
        let path = root.join("datadir.bin");
        std::fs::write(&path, zstd_max_compress(&encode_native(datadir)).unwrap()).unwrap();
        path
    }

    #[test]
    fn malformed_terrain_reports_all_failures_and_continues_to_later_entries() {
        for include_valid in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let mut datadir = ShippingDatadir::default();
            datadir.raw.insert("a-broken.map".into(), vec![0xff]);
            datadir.raw.insert("b-broken.min".into(), vec![0xff]);
            if include_valid {
                datadir.raw.insert("z-valid.map".into(), valid_terrain(1));
            }
            let path = write_manifest(root.path(), &datadir);
            let mut output = Vec::new();
            let error = run(&path, &mut output).unwrap_err();
            assert_eq!(
                error.to_string(),
                format!(
                    "2 terrain inspections failed ({} passed)",
                    usize::from(include_valid)
                )
            );
            let output = String::from_utf8(output).unwrap();
            let rows: Vec<_> = output.lines().skip(2).collect();
            assert_eq!(rows.len(), 2 + usize::from(include_valid));
            for (row, key) in rows.iter().zip(["a-broken.map", "b-broken.min"]) {
                assert!(row.starts_with(key), "{row}");
                assert!(row.contains("ERR:"), "{row}");
            }
            if include_valid {
                assert!(rows[2].starts_with("z-valid.map"));
                assert!(rows[2].contains("sixteen"));
                assert!(rows[2].contains("1×1 ok ("));
            }
        }
    }

    #[test]
    fn empty_terrain_selection_is_successful() {
        let root = tempfile::tempdir().unwrap();
        let mut datadir = ShippingDatadir::default();
        datadir.raw.insert("ignored.bin".into(), vec![0xff]);
        let path = write_manifest(root.path(), &datadir);
        let mut output = Vec::new();
        run(&path, &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert_eq!(output.lines().count(), 2);
        assert!(output.ends_with("# no .map/.min entries in shipping files\n"));
    }

    #[test]
    fn boot_and_mission_terrain_preserve_filter_order_override_and_dedup() {
        let root = tempfile::tempdir().unwrap();
        let mut datadir = ShippingDatadir::default();
        datadir.raw.insert("a-boot.min".into(), valid_terrain(1));
        datadir.raw.insert("b-shared.map".into(), valid_terrain(1));
        datadir.raw.insert("ignored.bin".into(), vec![0xff]);
        let mut first = ShippingMission::default();
        first.raw.insert("b-shared.map".into(), vec![0xff]);
        first.raw.insert("c-only.map".into(), valid_terrain(1));
        first.raw.insert("ignored.bin".into(), vec![0xff]);
        let mut second = ShippingMission::default();
        second.raw.insert("b-shared.map".into(), valid_terrain(2));
        for (file, mission) in [
            ("first.rhmission.zst", first),
            ("second.rhmission.zst", second),
        ] {
            std::fs::write(
                root.path().join(file),
                zstd_max_compress(&encode_mission_native(&mission)).unwrap(),
            )
            .unwrap();
        }
        datadir.missions.insert(
            "A".into(),
            ShippingMissionRef {
                forest_level: false,
                files: vec!["first.rhmission.zst".into(), "second.rhmission.zst".into()],
            },
        );
        // A repeated first payload must not replace the second payload's valid map.
        datadir.missions.insert(
            "B".into(),
            ShippingMissionRef {
                forest_level: false,
                files: vec!["first.rhmission.zst".into()],
            },
        );
        let path = write_manifest(root.path(), &datadir);
        let mut output = Vec::new();
        run(&path, &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();
        let rows: Vec<_> = output.lines().skip(2).collect();
        assert_eq!(rows.len(), 3);
        for (row, key) in rows
            .iter()
            .zip(["a-boot.min", "b-shared.map", "c-only.map"])
        {
            assert!(row.starts_with(key), "{row}");
            assert!(row.contains("sixteen"));
            assert!(row.contains(" ok ("));
        }
        assert!(rows[1].contains("2×1 ok ("));
    }
}
