//! Smoke test: load a shipping `datadir.bin`, dump every `.pak`/`.res` raw
//! entry from the boot manifest and lazy mission files, and re-parse it
//! (`read_pak_pictures`, `ResourceManager::attach_resource_file`). Confirms
//! the converter's bzip2-stripping rewrite produces blobs the runtime can
//! still read.
//!
//!   cargo run --release --example pak_res_roundtrip -- <path-to-datadir.bin>
#![allow(clippy::print_stdout)]

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use robin_assets::picture::Picture;
use robin_assets::resource_manager::ResourceManager;
use robin_assets::shipping_datadir::{ShippingDatadir, decode_mission_compressed};
use robin_engine::sbfile::SbFile;

#[derive(clap::Parser, serde::Serialize, serde::Deserialize)]
struct Args {
    path: PathBuf,
}

fn main() -> Result<()> {
    let path = <Args as clap::Parser>::parse().path;
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

    let tmp = tempfile::Builder::new().prefix("rh_pakres_").tempdir()?;

    let mut assets = std::collections::BTreeMap::<String, Vec<u8>>::new();
    assets.extend(
        std::mem::take(&mut dd.raw)
            .into_iter()
            .filter(|(key, _)| key.ends_with(".pak") || key.ends_with(".res")),
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
            assets.extend(
                payload
                    .payload
                    .raw
                    .into_iter()
                    .filter(|(key, _)| key.ends_with(".pak") || key.ends_with(".res")),
            );
        }
    }

    writeln!(output, "{:<48} {:>10} {:<5} result", "key", "bytes", "ext")?;
    let mut ok = 0usize;
    let mut fail = 0usize;
    for (k, bytes) in &assets {
        let scratch = tmp.path().join("scratch");
        std::fs::write(&scratch, bytes)?;
        let result = if k.ends_with(".pak") {
            // Manual walk: read back-to-back packed 16-bit pictures via the
            // public Picture::load_sixteen_from_stream entry point.
            let mut file =
                SbFile::open(scratch.to_str().unwrap()).map_err(|e| anyhow!("open: {e}"))?;
            inspect_pak(&mut file)
        } else {
            let mut rm = ResourceManager::legacy_tool();
            match rm.attach_resource_file(scratch.to_str().unwrap()) {
                Ok(()) => Ok(format!("{} resources", rm.resource_ids_with_types().len())),
                Err(e) => Err(e),
            }
        };
        match result {
            Ok(detail) => {
                writeln!(
                    output,
                    "{:<48} {:>10} {:<5} ok ({})",
                    k,
                    bytes.len(),
                    if k.ends_with(".pak") { "pak" } else { "res" },
                    detail
                )?;
                ok += 1;
            }
            Err(e) => {
                writeln!(
                    output,
                    "{:<48} {:>10} {:<5} ERR: {}",
                    k,
                    bytes.len(),
                    if k.ends_with(".pak") { "pak" } else { "res" },
                    e
                )?;
                fail += 1;
            }
        }
    }
    writeln!(output, "# {ok} ok, {fail} failed")?;
    if fail > 0 {
        return Err(anyhow!("{fail} archive inspections failed ({ok} passed)"));
    }
    Ok(())
}

fn inspect_pak(file: &mut SbFile) -> Result<String> {
    let total = file.get_size();
    let mut count = 0;
    let mut last = String::new();
    while file.tell() < total {
        let picture = Picture::load_sixteen_from_stream(file)
            .map_err(|error| anyhow!("after {count} pics: {error}"))?;
        count += 1;
        last = format!("{}×{}", picture.width, picture.height);
    }
    Ok(format!("{count} pictures, last {last}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_assets::picture::{PixelFormat, SixteenPacking};
    use robin_assets::shipping_datadir::{
        ShippingMission, ShippingMissionRef, encode_mission_native, encode_native,
        zstd_max_compress,
    };

    fn valid_pak(width: u16) -> Vec<u8> {
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
    fn malformed_pak_counts_failure_and_inspects_later_archives() {
        let root = tempfile::tempdir().unwrap();
        let mut datadir = ShippingDatadir::default();
        let mut broken = valid_pak(1);
        broken.push(0xff);
        datadir.raw.insert("a-broken.pak".into(), broken);
        datadir.raw.insert("b-broken.res".into(), vec![0xff]);
        datadir.raw.insert("z-valid.pak".into(), valid_pak(1));
        let path = write_manifest(root.path(), &datadir);
        let mut output = Vec::new();
        let error = run(&path, &mut output).unwrap_err();
        assert_eq!(error.to_string(), "2 archive inspections failed (1 passed)");
        let output = String::from_utf8(output).unwrap();
        let rows: Vec<_> = output.lines().skip(2).collect();
        assert!(rows[0].starts_with("a-broken.pak"));
        assert!(rows[0].contains("ERR: after 1 pics:"));
        assert!(rows[1].starts_with("b-broken.res"));
        assert!(rows[1].contains("ERR:"));
        assert!(rows[2].starts_with("z-valid.pak"));
        assert!(rows[2].ends_with("ok (1 pictures, last 1×1)"));
        assert_eq!(rows[3], "# 1 ok, 2 failed");
    }

    #[test]
    fn boot_and_mission_archives_preserve_filter_order_override_and_dedup() {
        let root = tempfile::tempdir().unwrap();
        let mut datadir = ShippingDatadir::default();
        let mut empty_res = b"SRES".to_vec();
        empty_res.extend_from_slice(&0x0100u32.to_le_bytes());
        empty_res.extend_from_slice(&0u32.to_le_bytes());
        datadir.raw.insert("a-valid.res".into(), empty_res);
        datadir.raw.insert("b-shared.pak".into(), valid_pak(1));
        datadir.raw.insert("ignored.bin".into(), vec![0xff]);
        let mut first = ShippingMission::default();
        first.raw.insert("b-shared.pak".into(), vec![0xff]);
        first.raw.insert("c-only.pak".into(), valid_pak(1));
        first.raw.insert("ignored.bin".into(), vec![0xff]);
        let mut second = ShippingMission::default();
        second.raw.insert("b-shared.pak".into(), valid_pak(2));
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
        // Re-reading first would overwrite the valid second archive with corruption.
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
        assert_eq!(rows.len(), 4);
        for (row, key) in rows
            .iter()
            .zip(["a-valid.res", "b-shared.pak", "c-only.pak"])
        {
            assert!(row.starts_with(key), "{row}");
        }
        assert!(rows[0].ends_with("ok (0 resources)"));
        assert!(rows[1].ends_with("ok (1 pictures, last 2×1)"));
        assert!(rows[2].ends_with("ok (1 pictures, last 1×1)"));
        assert_eq!(rows[3], "# 3 ok, 0 failed");
    }
}
