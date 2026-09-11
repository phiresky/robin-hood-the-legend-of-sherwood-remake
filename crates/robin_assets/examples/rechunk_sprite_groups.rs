//! Regenerate a benchmark copy of one mission's sprite groups, preserving its
//! already-converted pixels/audio. This does not rebuild authenticated package
//! manifests; use the production converter for distributable packages.
//!
//! rechunk_sprite_groups <input Data> <new output Data> <mission> <VQ tiles, 0=unsplit> <JXL blobs, 0=unsplit>
use anyhow::{Context, Result, ensure};
use robin_assets::{
    shipping_datadir::{
        ShippingDatadir, ShippingMission, decode_mission_compressed, derive_chunk_self_refs,
        encode_mission_native, zstd_compress_with_window,
    },
    sprite_codec::{SpriteGrid, decode_grids_shipping},
    sprite_groups::{encode_vq_groups, split_rle_jxl_chunk},
};
use std::{collections::BTreeMap, path::Path, time::Instant};

fn copy_tree(source: &Path, target: &Path) -> Result<()> {
    std::fs::create_dir(target)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        ensure!(
            !kind.is_symlink(),
            "benchmark copy refuses symlink {}",
            entry.path().display()
        );
        if kind.is_dir() {
            copy_tree(&entry.path(), &target.join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), target.join(entry.file_name()))?;
        }
    }
    Ok(())
}
#[derive(clap::Parser, serde::Serialize, serde::Deserialize)]
struct Args {
    source: std::path::PathBuf,
    target: std::path::PathBuf,
    mission: String,
    max_tiles: usize,
    max_blobs: usize,
}

fn main() -> Result<()> {
    let args = <Args as clap::Parser>::parse();
    let source = args.source.as_path();
    let target = args.target.as_path();
    let max_tiles = args.max_tiles;
    let max_blobs = args.max_blobs;
    ensure!(
        !target.exists(),
        "output already exists: {}",
        target.display()
    );
    let dd = ShippingDatadir::from_compressed_bytes(&std::fs::read(source.join("datadir.bin"))?)?;
    let mission = dd.missions.get(&args.mission).context("mission absent")?;
    let mut inputs = BTreeMap::new();
    let mut merged = ShippingMission::default();
    for name in &mission.files {
        ensure!(
            Path::new(name)
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_))),
            "invalid part path {name}"
        );
        if inputs.contains_key(name) {
            continue;
        }
        let bytes = std::fs::read(source.join(name))?;
        merged.merge_part(decode_mission_compressed(&bytes)?)?;
        inputs.insert(name.clone(), bytes);
    }
    let payload = &mut merged.payload;
    let bank = payload.sprite_bank.as_mut().context("no sprite bank")?;
    bank.materialize_vq_chunks(&payload.rhs_files)?;
    let sprite = |id: u32| -> Result<&robin_assets::shipping_datadir::ShippingSprite> {
        let index = bank
            .sprites
            .binary_search_by_key(&id, |(id, _)| *id)
            .map_err(|_| anyhow::anyhow!("missing decoded sprite {id}"))?;
        Ok(&bank.sprites[index].1)
    };
    std::fs::create_dir_all(target.parent().context("output has no parent")?)?;
    copy_tree(source, target)?;
    let mut before_bytes = 0;
    let mut after_bytes = 0;
    let mut before_vq = 0;
    let mut after_vq = 0;
    let mut vq_groups = 0;
    let mut rle_groups = 0;
    for (name, bytes) in inputs {
        before_bytes += bytes.len();
        let mut part = decode_mission_compressed(&bytes)?;
        let Some(part_bank) = part.payload.sprite_bank.as_mut() else {
            after_bytes += bytes.len();
            continue;
        };
        for chunk in std::mem::take(&mut part_bank.vq_chunks) {
            before_vq += chunk.blob.len();
            let grids = chunk
                .sprite_ids
                .iter()
                .map(|&id| {
                    let row = sprite(id)?;
                    Ok(SpriteGrid {
                        cols: row.width / 4,
                        rows: row.height,
                        indices: &row.packed_data,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let resolve = |ids: &[Option<u32>]| -> Result<Vec<Option<&[u16]>>> {
                if ids.is_empty() {
                    return Ok(vec![None; grids.len()]);
                }
                ids.iter()
                    .map(|id| {
                        id.map(|id| sprite(id).map(|s| s.packed_data.as_slice()))
                            .transpose()
                    })
                    .collect()
            };
            let bases = resolve(&chunk.base_ids)?;
            let bases2 = resolve(&chunk.base2_ids)?;
            let rhs = payload.rhs_files.get(&chunk.rhs);
            let start = Instant::now();
            let groups = encode_vq_groups(&chunk, &grids, &bases, &bases2, rhs, max_tiles)?;
            let encode_ms = start.elapsed().as_secs_f64() * 1000.;
            if max_tiles == 0 {
                ensure!(
                    groups.len() == 1 && groups[0].blob == chunk.blob,
                    "unsplit bitstream changed for {}",
                    chunk.rhs
                );
            }
            let mut offset = 0;
            let mut decode_ms = Vec::new();
            for group in &groups {
                let end = offset + group.sprite_ids.len();
                let refs = if group.self_refs {
                    derive_chunk_self_refs(&rhs.context("missing RHS")?.profiles, &group.sprite_ids)
                } else {
                    vec![None; end - offset]
                };
                let dims: Vec<_> = grids[offset..end]
                    .iter()
                    .map(|g| (g.cols, g.rows))
                    .collect();
                let decode_start = Instant::now();
                let decoded = decode_grids_shipping(
                    group.alphabet,
                    &dims,
                    Some(&bases[offset..end]),
                    Some(&bases2[offset..end]),
                    &refs,
                    &group.blob,
                )?;
                decode_ms.push(decode_start.elapsed().as_secs_f64() * 1000.);
                for (actual, expected) in decoded.iter().zip(&grids[offset..end]) {
                    ensure!(
                        actual.as_slice() == expected.indices,
                        "group output changed for {}",
                        chunk.rhs
                    );
                }
                offset = end;
            }
            let group_bytes: usize = groups.iter().map(|g| g.blob.len()).sum();
            after_vq += group_bytes;
            vq_groups += groups.len();
            println!(
                "{}",
                serde_json::json!({"rhs":chunk.rhs,"groups":groups.len(),"before_vq_bytes":chunk.blob.len(),"after_vq_bytes":group_bytes,"encode_ms":encode_ms,"decode_group_ms":decode_ms,"tiles":grids.iter().map(|g|g.indices.len()).sum::<usize>()})
            );
            part_bank.vq_chunks.extend(groups);
        }
        for chunk in std::mem::take(&mut part_bank.rle_jxl_chunks) {
            let mut atlases = Vec::new();
            for blob in &chunk.jxl_blobs {
                let start = Instant::now();
                let (width, height, _) = robin_assets::rle_jxl::decode_jxl_rgba8(blob)?;
                atlases.push(serde_json::json!({"bytes":blob.len(), "width":width, "height":height, "decode_ms":start.elapsed().as_secs_f64()*1000.}));
            }
            println!(
                "{}",
                serde_json::json!({"rle_rhs":chunk.rhs,"atlases":atlases})
            );
            part_bank
                .rle_jxl_chunks
                .extend(split_rle_jxl_chunk(chunk, max_blobs)?);
        }
        rle_groups += part_bank.rle_jxl_chunks.len();
        let compressed = zstd_compress_with_window(&encode_mission_native(&part), 30)?;
        after_bytes += compressed.len();
        std::fs::write(target.join(&name), compressed)?;
    }
    println!(
        "{}",
        serde_json::json!({"mission":args.mission,"max_tiles":max_tiles,"max_blobs":max_blobs,"before_part_bytes":before_bytes,"after_part_bytes":after_bytes,"before_vq_bytes":before_vq,"after_vq_bytes":after_vq,"vq_groups":vq_groups,"rle_groups":rle_groups})
    );
    Ok(())
}
