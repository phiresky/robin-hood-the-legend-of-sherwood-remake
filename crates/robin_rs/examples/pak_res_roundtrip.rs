//! Smoke test: load a shipping `datadir.bin`, dump every `.pak`/`.res` raw
//! entry from the boot manifest and lazy mission files, and re-parse it
//! (`read_pak_pictures`, `ResourceManager::attach_resource_file`). Confirms
//! the converter's bzip2-stripping rewrite produces blobs the runtime can
//! still read.
//!
//!   cargo run --release --example pak_res_roundtrip -- <path-to-datadir.bin>
#![allow(clippy::print_stdout)]

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use robin_assets::picture::Picture;
use robin_assets::resource_manager::ResourceManager;
use robin_assets::shipping_datadir::{ShippingDatadir, decode_mission_compressed};
use robin_engine::sbfile::{SB_FILE_READ, SbFile};

fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("usage: pak_res_roundtrip <datadir.bin>"))?;

    let dd = ShippingDatadir::load_from_file(&path)?;
    println!("# loaded {} ({} raw entries)", path.display(), dd.raw.len());

    let tmp = tempfile::Builder::new().prefix("rh_pakres_").tempdir()?;

    let mut assets = std::collections::BTreeMap::<String, Vec<u8>>::new();
    assets.extend(
        dd.raw
            .iter()
            .filter(|(key, _)| key.ends_with(".pak") || key.ends_with(".res"))
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
            assets.extend(
                payload
                    .raw
                    .into_iter()
                    .filter(|(key, _)| key.ends_with(".pak") || key.ends_with(".res")),
            );
        }
    }

    println!("{:<48} {:>10} {:<5} result", "key", "bytes", "ext");
    let mut ok = 0usize;
    let mut fail = 0usize;
    for (k, bytes) in &assets {
        let scratch = tmp.path().join("scratch");
        std::fs::write(&scratch, bytes)?;
        let result = if k.ends_with(".pak") {
            // Manual walk: read back-to-back SBPictureSixteen via the
            // public Picture::load_sixteen_from_stream entry point.
            let mut file = SbFile::open(scratch.to_str().unwrap(), SB_FILE_READ)
                .map_err(|e| anyhow!("open: {e}"))?;
            let total = file.get_size();
            let mut count = 0;
            let mut last = String::new();
            while file.tell() < total {
                match Picture::load_sixteen_from_stream(&mut file) {
                    Ok(p) => {
                        count += 1;
                        last = format!("{}×{}", p.width, p.height);
                    }
                    Err(e) => {
                        println!(
                            "{:<48} {:>10} {:<5} ERR after {} pics: {}",
                            k,
                            bytes.len(),
                            "pak",
                            count,
                            e
                        );
                        return Ok(());
                    }
                }
            }
            Ok(format!("{count} pictures, last {last}"))
        } else {
            let mut rm = ResourceManager::new();
            match rm.attach_resource_file(scratch.to_str().unwrap()) {
                Ok(()) => Ok(format!("{} resources", rm.resource_ids_with_types().len())),
                Err(e) => Err(e),
            }
        };
        match result {
            Ok(detail) => {
                println!(
                    "{:<48} {:>10} {:<5} ok ({})",
                    k,
                    bytes.len(),
                    if k.ends_with(".pak") { "pak" } else { "res" },
                    detail
                );
                ok += 1;
            }
            Err(e) => {
                println!(
                    "{:<48} {:>10} {:<5} ERR: {}",
                    k,
                    bytes.len(),
                    if k.ends_with(".pak") { "pak" } else { "res" },
                    e
                );
                fail += 1;
            }
        }
    }
    println!("# {ok} ok, {fail} failed");
    if fail > 0 {
        std::process::exit(1);
    }
    Ok(())
}
