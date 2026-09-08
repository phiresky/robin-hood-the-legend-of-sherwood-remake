//! Decode a fixed shipping mission, including boot/interface, terrain and both
//! sprite formats. No network, GPU, audio playback or simulation is measured.
//!
//! cargo build -p robin_assets --release --example asset_decode_bench
//! RAYON_NUM_THREADS=1 target/release/examples/asset_decode_bench <Data> <mission> [repeats]
//! Inputs are pre-read; each iteration starts with fresh decoder state. Compare
//! output_sha256 across builds using the SAME converted tree. Hashing and asset
//! destruction are outside the phase timings. JSON lines support interleaved A/Bs.

use anyhow::{Context, Result, ensure};
use robin_assets::{
    picture::Picture,
    resource_manager::ResourceManager,
    shipping_datadir::{ShippingDatadir, ShippingMission, decode_mission_compressed},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::PathBuf, time::Instant};

#[derive(Default, Serialize, Deserialize)]
struct Report {
    mission: String,
    boot_ms: f64,
    parts_ms: f64,
    vq_ms: f64,
    rle_jxl_ms: f64,
    interface_ms: f64,
    terrain_ms: f64,
    total_ms: f64,
    compressed_bytes: usize,
    vq_chunks: usize,
    rle_jxl_chunks: usize,
    rle_jxl_atlases: usize,
    terrain_images: usize,
    interface_images: usize,
    output_sha256: String,
}

fn elapsed(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

fn hash_picture(hash: &mut Sha256, picture: &Picture) {
    hash.update(picture.width.to_le_bytes());
    hash.update(picture.height.to_le_bytes());
    hash.update(picture.pitch.to_le_bytes());
    hash.update(&picture.data);
}

fn hash_resources(
    hash: &mut Sha256,
    resources: &mut BTreeMap<String, ResourceManager>,
) -> Result<usize> {
    let mut count = 0;
    for (name, manager) in resources {
        hash.update(name.as_bytes());
        for id in manager.picture_resource_ids() {
            // Eager decode logs failures and leaves them for lazy recovery.
            // Force that path here so a failed decode never produces a timing
            // result that looks like a successful, faster run.
            let pictures = manager
                .get_pictures(id)
                .with_context(|| format!("{name} resource {id}"))?;
            hash.update(id.to_le_bytes());
            for picture in pictures {
                hash.update([u8::from(picture.is_some())]);
                if let Some(picture) = picture {
                    hash_picture(hash, picture);
                    count += 1;
                }
            }
        }
    }
    Ok(count)
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(
        (2..=3).contains(&args.len()),
        "usage: asset_decode_bench <Data dir> <mission> [repeats]"
    );
    let root = PathBuf::from(&args[0]);
    let mission = args[1].to_str().context("mission is not UTF-8")?;
    let repeats: usize = args
        .get(2)
        .map(|s| {
            s.to_str()
                .context("repeats is not UTF-8")?
                .parse()
                .context("invalid repeats")
        })
        .transpose()?
        .unwrap_or(3);
    ensure!(repeats > 0, "repeats must be positive");
    let boot = std::fs::read(root.join("datadir.bin"))?;
    let manifest = ShippingDatadir::from_compressed_bytes(&boot)?;
    let reference = manifest
        .missions
        .get(mission)
        .context("mission absent from manifest")?;
    let parts = reference
        .files
        .iter()
        .map(|p| std::fs::read(root.join(p)).with_context(|| format!("read {p}")))
        .collect::<Result<Vec<_>>>()?;
    drop(manifest);
    let compressed_bytes = boot.len() + parts.iter().map(Vec::len).sum::<usize>();
    let mut expected_hash = None;
    for _ in 0..repeats {
        let mut report = Report {
            mission: mission.to_owned(),
            compressed_bytes,
            ..Default::default()
        };
        let start = Instant::now();
        let mut dd = ShippingDatadir::from_compressed_bytes(&boot)?;
        report.boot_ms = elapsed(start);
        let start = Instant::now();
        let mut merged = ShippingMission::default();
        for part in &parts {
            merged.merge_part(decode_mission_compressed(part)?)?;
        }
        report.parts_ms = elapsed(start);
        let payload = &mut merged.payload;
        let bank = payload
            .sprite_bank
            .as_mut()
            .context("mission has no sprite bank")?;
        report.vq_chunks = bank.vq_chunks.len();
        report.rle_jxl_chunks = bank.rle_jxl_chunks.len();
        report.rle_jxl_atlases = bank
            .rle_jxl_chunks
            .iter()
            .map(|chunk| chunk.jxl_blobs.len())
            .sum();
        let start = Instant::now();
        bank.materialize_vq_chunks(&payload.rhs_files)?;
        report.vq_ms = elapsed(start);
        let start = Instant::now();
        bank.materialize_rle_jxl_chunks()?;
        report.rle_jxl_ms = elapsed(start);
        let start = Instant::now();
        for manager in dd.res_files.values_mut() {
            manager.decode_all_encoded_pictures();
        }
        let pak_pictures = dd
            .pak_files
            .values()
            .flatten()
            .map(|p| p.decode())
            .collect::<Result<Vec<_>>>()?;
        report.interface_ms = elapsed(start);
        let start = Instant::now();
        let terrain = payload
            .raw
            .iter()
            .filter(|(name, _)| {
                let name = name.to_ascii_lowercase();
                name.ends_with(".map") || name.ends_with(".min")
            })
            .map(|(name, bytes)| {
                Picture::load_terrain_from_bytes_parallel(bytes)
                    .with_context(|| format!("decode {name}"))
            })
            .collect::<Result<Vec<_>>>()?;
        report.terrain_ms = elapsed(start);
        report.terrain_images = terrain.len();
        report.total_ms = report.boot_ms
            + report.parts_ms
            + report.vq_ms
            + report.rle_jxl_ms
            + report.interface_ms
            + report.terrain_ms;

        let mut hash = Sha256::new();
        // Hash grids AND the visible raster windows. Atlas packing/gutter
        // changes cannot hide a changed sprite or make an equivalent layout fail.
        for (id, sprite) in &bank.sprites {
            hash.update(id.to_le_bytes());
            hash.update(sprite.width.to_le_bytes());
            hash.update(sprite.height.to_le_bytes());
            hash.update(bytemuck::cast_slice::<u16, u8>(&sprite.packed_data));
            if let Some(raster) = &sprite.raster {
                for y in 0..sprite.height as usize {
                    let start =
                        (raster.y as usize + y) * raster.stride as usize + raster.x as usize;
                    hash.update(bytemuck::cast_slice::<u16, u8>(
                        &raster.atlas[start..start + sprite.width as usize],
                    ));
                }
            }
        }
        report.interface_images =
            hash_resources(&mut hash, &mut dd.res_files)? + pak_pictures.len();
        for picture in pak_pictures.iter().chain(&terrain) {
            hash_picture(&mut hash, picture);
        }
        report.output_sha256 = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if let Some(expected) = &expected_hash {
            ensure!(
                &report.output_sha256 == expected,
                "decoded output changed between iterations"
            );
        } else {
            expected_hash = Some(report.output_sha256.clone());
        }
        println!("{}", serde_json::to_string(&report)?);
    }
    Ok(())
}
