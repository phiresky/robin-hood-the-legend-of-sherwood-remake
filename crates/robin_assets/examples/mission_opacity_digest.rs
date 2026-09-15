//! Digest of a shipping mission's sprite OPACITY inputs, for comparing two
//! datadir builds (e.g. the JPEG XL and AVIF web recipes) natively.
//!
//! `FrameHolder::simulation_opacity_sha256` — the `sprite_opacity_sha256` in
//! the prepared-mission run projection — hashes, per simulation-reachable
//! sprite, its bank id and dimensions plus two bitmaps: "not the transparent
//! key" and "not the transparent key and not the shadow colour". Colour never
//! enters it. This tool materializes the mission's sprite bank exactly like a
//! mission install (VQ chunks, then RLE image atlases — AVIF decodes natively
//! through rav1d) and hashes the same class bitmaps for every RLE sprite, plus
//! the packed VQ grids (whose codec is identical in both recipes). Equal
//! digests mean equal opacity inputs for every reachable subset.
//!
//! ```text
//! cargo run --release -p robin_assets --example mission_opacity_digest -- <Data> <mission>
//! ```

use anyhow::{Context, Result, ensure};
use robin_assets::frame_holder::{SHADOW_KEY, TRANSPARENT_COLOR_16, UNMAPPED_DICT};
use robin_assets::shipping_datadir::{ShippingDatadir, ShippingMission, decode_mission_compressed};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    ensure!(
        args.len() == 3,
        "usage: mission_opacity_digest <Data> <mission>"
    );
    let root = PathBuf::from(&args[1]);
    let mission = &args[2];
    let datadir = ShippingDatadir::from_compressed_bytes(
        &std::fs::read(root.join("datadir.bin")).context("read datadir.bin")?,
    )?;
    let reference = datadir
        .missions
        .get(mission)
        .with_context(|| format!("mission {mission} absent from datadir.bin"))?;
    let mut merged = ShippingMission::default();
    for part in &reference.files {
        merged.merge_part(
            decode_mission_compressed(
                &std::fs::read(root.join(part)).with_context(|| format!("read {part}"))?,
            )
            .with_context(|| format!("decode {part}"))?,
        )?;
    }
    let rhs_files = merged.payload.rhs_files.clone();
    let bank = merged
        .payload
        .sprite_bank
        .as_mut()
        .context("mission has no sprite bank")?;
    let rle_image_chunks = bank.rle_jxl_chunks.len();
    let rle_image_atlases: usize = bank.rle_jxl_chunks.iter().map(|c| c.jxl_blobs.len()).sum();
    bank.materialize_vq_chunks(&rhs_files)?;
    bank.materialize_rle_jxl_chunks()?;

    let mut classes = Sha256::new();
    classes.update(b"robinhood-sprite-opacity-classes-v1\0");
    let mut vq_grids = Sha256::new();
    let (mut rle_sprites, mut raster_sprites, mut vq_sprites) = (0usize, 0usize, 0usize);
    for (id, sprite) in &bank.sprites {
        let (width, height) = (usize::from(sprite.width), usize::from(sprite.height));
        if sprite.dictionary_index != UNMAPPED_DICT {
            vq_sprites += 1;
            vq_grids.update(id.to_le_bytes());
            vq_grids.update(sprite.width.to_le_bytes());
            vq_grids.update(sprite.height.to_le_bytes());
            vq_grids.update(bytemuck::cast_slice::<u16, u8>(&sprite.packed_data));
            continue;
        }
        let pixels: Vec<u16> = if let Some(raster) = &sprite.raster {
            raster_sprites += 1;
            (0..height)
                .flat_map(|y| {
                    raster
                        .row(y, width)
                        .unwrap_or_else(|| panic!("sprite {id} raster row {y} out of range"))
                        .iter()
                        .copied()
                        .collect::<Vec<_>>()
                })
                .collect()
        } else if sprite.packed_data.is_empty() {
            continue;
        } else {
            let (canvas, _) =
                robin_assets::rle_jxl::decode_rle_canvas(width, height, &sprite.packed_data)
                    .with_context(|| format!("decode RLE sprite {id}"))?;
            canvas
        };
        rle_sprites += 1;
        let bytes = (width * height).div_ceil(8);
        let (mut ordinary, mut blipped) = (vec![0u8; bytes], vec![0u8; bytes]);
        for (index, pixel) in pixels.into_iter().enumerate() {
            if pixel == TRANSPARENT_COLOR_16 {
                continue;
            }
            blipped[index / 8] |= 1 << (index % 8);
            if pixel != SHADOW_KEY {
                ordinary[index / 8] |= 1 << (index % 8);
            }
        }
        classes.update(id.to_le_bytes());
        classes.update(sprite.width.to_le_bytes());
        classes.update(sprite.height.to_le_bytes());
        classes.update([0]);
        classes.update(&ordinary);
        classes.update([1]);
        classes.update(&blipped);
    }
    let hex = |digest: [u8; 32]| {
        digest
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };
    println!(
        "{{\"mission\":{mission:?},\"rle_image_chunks\":{rle_image_chunks},\"rle_image_atlases\":{rle_image_atlases},\"rle_sprites\":{rle_sprites},\"raster_sprites\":{raster_sprites},\"vq_sprites\":{vq_sprites},\"rle_opacity_classes_sha256\":\"{}\",\"vq_grids_sha256\":\"{}\"}}",
        hex(classes.finalize().into()),
        hex(vq_grids.finalize().into())
    );
    Ok(())
}
