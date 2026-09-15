//! Encoded image bytes of a converted shipping datadir, by category and
//! codec — for comparing web recipes (JPEG XL vs AVIF).
//!
//! Categories: interface pictures (boot and locale `.res` resources plus
//! `.pak` pictures), terrain maps (`.map`), minimaps (`.min`), and RLE
//! sprite atlases (mission part files). Lazy mission part files are read
//! once each even when several missions reference them.
//!
//! ```text
//! cargo run --release -p robin_assets --example datadir_image_bytes -- <Data>
//! ```

use anyhow::{Context, Result, ensure};
use robin_assets::resource_manager::EncodedPicture;
use robin_assets::shipping_datadir::{ShippingDatadir, decode_mission_compressed};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

#[derive(Default)]
struct Tally(BTreeMap<(String, String), (u64, u64)>);

impl Tally {
    fn add(&mut self, category: &str, codec: &str, bytes: usize) {
        let entry = self
            .0
            .entry((category.to_owned(), codec.to_owned()))
            .or_default();
        entry.0 += 1;
        entry.1 += bytes as u64;
    }
}

fn blob_codec(bytes: &[u8]) -> &'static str {
    if robin_assets::browser_images::is_avif(bytes) {
        "avif"
    } else if robin_assets::picture::is_jxl_signature(bytes) {
        "jxl"
    } else {
        "sixteen/raw"
    }
}

fn picture_codec(picture: &EncodedPicture) -> String {
    format!("{:?}", picture.codec)
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    ensure!(args.len() == 2, "usage: datadir_image_bytes <Data>");
    let root = PathBuf::from(&args[1]);
    let datadir = ShippingDatadir::from_compressed_bytes(
        &std::fs::read(root.join("datadir.bin")).context("read datadir.bin")?,
    )?;
    let mut tally = Tally::default();

    let mut interface = |pictures: &mut dyn Iterator<Item = &EncodedPicture>| {
        for picture in pictures {
            tally.add("interface", &picture_codec(picture), picture.bytes.len());
        }
    };
    let mut boot_sources = vec![(&datadir.res_files, &datadir.pak_files)];
    for locale in datadir.locales.values() {
        boot_sources.push((&locale.res_files, &locale.pak_files));
    }
    for (res_files, pak_files) in boot_sources {
        for manager in res_files.values() {
            interface(&mut manager.encoded_picture_slots());
        }
        for pictures in pak_files.values() {
            interface(&mut pictures.iter());
        }
    }

    let mut raw_blobs: Vec<(String, Vec<u8>)> = datadir
        .raw
        .iter()
        .map(|(name, bytes)| (name.clone(), bytes.clone()))
        .collect();
    let part_files: BTreeSet<&String> = datadir
        .missions
        .values()
        .flat_map(|mission| mission.files.iter())
        .collect();
    for file in &part_files {
        let part = decode_mission_compressed(
            &std::fs::read(root.join(file)).with_context(|| format!("read {file}"))?,
        )
        .with_context(|| format!("decode {file}"))?;
        if let Some(bank) = part.payload.sprite_bank.as_ref() {
            for chunk in &bank.rle_jxl_chunks {
                for blob in &chunk.jxl_blobs {
                    tally.add("rle_sprite_atlas", blob_codec(blob), blob.len());
                }
            }
        }
        raw_blobs.extend(part.payload.raw.into_iter());
    }
    let mut seen = BTreeSet::new();
    for (name, bytes) in &raw_blobs {
        let lower = name.to_ascii_lowercase();
        let category = if lower.ends_with(".map") {
            "terrain_map"
        } else if lower.ends_with(".min") {
            "minimap"
        } else {
            continue;
        };
        if seen.insert((lower.clone(), bytes.len())) {
            tally.add(category, blob_codec(bytes), bytes.len());
        }
    }

    let mut totals: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    for ((category, codec), (count, bytes)) in &tally.0 {
        println!(
            "{{\"category\":{category:?},\"codec\":{codec:?},\"images\":{count},\"bytes\":{bytes}}}"
        );
        let total = totals.entry(category.clone()).or_default();
        total.0 += count;
        total.1 += bytes;
    }
    for (category, (count, bytes)) in totals {
        println!(
            "{{\"category\":{category:?},\"codec\":\"ALL\",\"images\":{count},\"bytes\":{bytes}}}"
        );
    }
    Ok(())
}
