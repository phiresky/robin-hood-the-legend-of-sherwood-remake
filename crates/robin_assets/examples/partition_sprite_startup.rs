//! Benchmark-only corpus rewrite: first N frames per character script initially,
//! remaining exact VQ grids in rhs-tail/ dependencies. Complete boot-dictionary
//! opacity masks remain in initial raw metadata. Authentication manifests
//! are deliberately not rebuilt; never distribute this as an official corpus.
use anyhow::{Context, Result, ensure};
use robin_assets::sprite_residency::{OpacityBatch, SpriteOpacity};
use robin_assets::{
    shipping_datadir::{
        ShippingDatadir, ShippingMission, ShippingSpriteBank, decode_mission_compressed,
        encode_mission_native, encode_native, zstd_compress_with_window,
    },
    sprite_codec::SpriteGrid,
    sprite_groups::encode_vq_groups,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path},
};

fn copy_tree(source: &Path, target: &Path) -> Result<()> {
    std::fs::create_dir(target)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        ensure!(
            !kind.is_symlink(),
            "refusing symlink {}",
            entry.path().display()
        );
        if kind.is_dir() {
            copy_tree(&entry.path(), &target.join(entry.file_name()))?;
        } else {
            ensure!(
                kind.is_file(),
                "not a regular file: {}",
                entry.path().display()
            );
            std::fs::copy(entry.path(), target.join(entry.file_name()))?;
        }
    }
    Ok(())
}

fn materialize(part: &mut ShippingMission) -> Result<()> {
    let payload = &mut part.payload;
    payload
        .sprite_bank
        .as_mut()
        .context("sprite bank missing")?
        .materialize_vq_chunks(&payload.rhs_files)
}

fn grid_sha(bank: &ShippingSpriteBank) -> String {
    let mut hash = Sha256::new();
    for (id, row) in &bank.sprites {
        hash.update(id.to_le_bytes());
        hash.update(row.width.to_le_bytes());
        hash.update(row.height.to_le_bytes());
        hash.update(row.dictionary_index.to_le_bytes());
        hash.update((row.packed_data.len() as u64).to_le_bytes());
        for word in row.packed_data.iter() {
            hash.update(word.to_le_bytes());
        }
    }
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

// Raw bitplanes are benchmark sidecars, with IDs/dimensions/offsets in JSON.
// TODO: assess ArnoLaw dictionary recoloring before designing resident masks.
fn opacity_masks(
    bank: &ShippingSpriteBank,
    dictionaries: &[robin_assets::frame_holder::FrameDictionary],
    ids: &BTreeSet<u32>,
) -> Result<(Vec<u8>, serde_json::Value, OpacityBatch)> {
    use robin_assets::frame_holder::TRANSPARENT_COLOR_16;
    let mut bytes = Vec::new();
    let mut metadata = Vec::new();
    let mut opacity_sprites = Vec::new();
    let mut hash = Sha256::new();
    hash.update(b"robinhood-sprite-opacity-v1\0");
    for (id, row) in bank.sprites.iter().filter(|(id, _)| ids.contains(id)) {
        let dict = dictionaries
            .get(usize::from(row.dictionary_index))
            .context("opacity dictionary missing")?;
        ensure!(row.width.is_multiple_of(4), "invalid VQ width for {id}");
        let pixels = usize::from(row.width) * usize::from(row.height);
        ensure!(
            row.packed_data.len() * 4 == pixels,
            "invalid VQ grid for {id}"
        );
        let mut ordinary = vec![0u8; pixels.div_ceil(8)];
        let mut blipped = vec![0u8; pixels.div_ceil(8)];
        for (tile, &index) in row.packed_data.iter().enumerate() {
            let colors = dict.lookup_pixels(index);
            ensure!(colors.len() == 4, "dictionary tile is not four pixels");
            for (offset, &color) in colors.iter().enumerate() {
                let pixel = tile * 4 + offset;
                if color != TRANSPARENT_COLOR_16 {
                    blipped[pixel / 8] |= 1 << (pixel % 8);
                    if color != dict.shadow_color() {
                        ordinary[pixel / 8] |= 1 << (pixel % 8);
                    }
                }
            }
        }
        hash.update(id.to_le_bytes());
        hash.update(row.width.to_le_bytes());
        hash.update(row.height.to_le_bytes());
        hash.update([0]);
        hash.update(&ordinary);
        hash.update([1]);
        hash.update(&blipped);
        metadata.push(serde_json::json!({"id":id,"width":row.width,"height":row.height,"offset":bytes.len(),"plane_bytes":ordinary.len()}));
        bytes.extend_from_slice(&ordinary);
        bytes.extend_from_slice(&blipped);
        opacity_sprites.push(SpriteOpacity {
            bank_id: *id,
            width: row.width,
            height: row.height,
            dictionary_index: row.dictionary_index,
            ordinary,
            blipped,
        });
    }
    ensure!(metadata.len() == ids.len(), "opacity rows missing");
    let sha: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    let compressed = zstd_compress_with_window(&bytes, 30)?;
    let report = serde_json::json!({"canonical_sha256":sha,"raw_bytes":bytes.len(),"zstd30_bytes":compressed.len(),"sprites":metadata,"bit_order":"row-major, least-significant bit first; ordinary then blipped for each sprite","dictionary_state":"boot"});
    Ok((
        compressed,
        report,
        OpacityBatch {
            sprites: opacity_sprites,
        },
    ))
}

fn append_tails(files: &mut Vec<String>, tails: &BTreeMap<String, String>) {
    let extra: Vec<_> = files.iter().filter_map(|f| tails.get(f)).cloned().collect();
    for tail in extra {
        if !files.contains(&tail) {
            files.push(tail);
        }
    }
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        (3..=4).contains(&args.len()),
        "usage: partition_sprite_startup <source Data> <new output Data> <first N> [new opacity audit directory]"
    );
    let audit = args.get(3).map(Path::new);
    if let Some(audit) = audit {
        ensure!(!audit.exists(), "opacity audit directory exists");
    }
    let source = std::fs::canonicalize(&args[0])?;
    if let Some(audit) = audit {
        let audit_absolute = std::env::current_dir()?.join(audit);
        let ancestor = audit_absolute
            .ancestors()
            .find(|p| p.exists())
            .context("audit ancestor missing")?;
        ensure!(
            !std::fs::canonicalize(ancestor)?.starts_with(&source),
            "audit output cannot be inside source"
        );
    }
    let target_path = std::env::current_dir()?.join(&args[1]);
    let target = target_path.as_path();
    let first: usize = args[2].parse()?;
    ensure!(first > 0, "first N must be positive");
    ensure!(
        !target.exists(),
        "output already exists: {}",
        target.display()
    );
    let output_parent = target.parent().context("output parent missing")?;
    let existing_parent = output_parent
        .ancestors()
        .find(|p| p.exists())
        .context("output has no existing ancestor")?;
    ensure!(
        !std::fs::canonicalize(existing_parent)?.starts_with(&source),
        "output cannot be inside source"
    );
    std::fs::create_dir_all(output_parent)?;
    // TODO: choose initial frames from measured mission usage before any
    // production conversion; first-N is deliberately only a bounded experiment.
    let mut dd =
        ShippingDatadir::from_compressed_bytes(&std::fs::read(source.join("datadir.bin"))?)?;
    let mut files = BTreeSet::new();
    for m in dd.missions.values() {
        files.extend(m.files.iter().cloned());
    }
    for refs in dd.character_rhs_files.values() {
        files.extend(refs.iter().cloned());
    }
    files.extend(dd.saved_world_rhs_files.iter().cloned());
    copy_tree(&source, target)?;
    if let Some(audit) = audit {
        std::fs::create_dir_all(audit)?;
    }
    let mut masks_raw_total = 0usize;
    let mut masks_compressed_total = 0usize;
    let mut tails = BTreeMap::new();
    let (mut before_total, mut head_total, mut tail_total) = (0usize, 0usize, 0usize);
    for name in files {
        ensure!(
            Path::new(&name)
                .components()
                .all(|c| matches!(c, Component::Normal(_))),
            "invalid part path {name}"
        );
        let bytes = std::fs::read(source.join(&name))?;
        let mut head = decode_mission_compressed(&bytes)?;
        let Some(bank) = head.sprite_bank.as_ref() else {
            continue;
        };
        if bank.vq_chunks.is_empty()
            || !bank.vq_chunks.iter().all(|c| {
                c.rhs.starts_with("Characters/")
                    && c.rhs.ends_with(".rhs")
                    && c.base_rhs.is_none()
                    && c.base2_rhs.is_empty()
                    && c.base_ids.iter().all(Option::is_none)
                    && c.base2_ids.iter().all(Option::is_none)
            })
        {
            continue;
        }
        let keep: BTreeSet<_> = head
            .rhs_files
            .values()
            .flat_map(|rhs| rhs.profiles.iter())
            .flat_map(|(_, p)| p.scripts.iter())
            .flat_map(|s| s.frame_ids.iter().take(first).copied())
            .collect();
        ensure!(!keep.is_empty(), "{name}: no script frame metadata");
        let vq_ids: BTreeSet<_> = bank
            .vq_chunks
            .iter()
            .flat_map(|c| c.sprite_ids.iter().copied())
            .collect();
        let mut original = decode_mission_compressed(&bytes)?;
        materialize(&mut original)?;
        let original_bank = original
            .sprite_bank
            .as_ref()
            .context("original bank missing")?;
        let rows: BTreeMap<_, _> = original_bank
            .sprites
            .iter()
            .map(|(id, row)| (*id, row))
            .collect();
        let bank = head
            .payload
            .sprite_bank
            .as_mut()
            .context("head bank missing")?;
        let mut tail = ShippingMission::default();
        tail.sprite_bank = Some(ShippingSpriteBank {
            signature: bank.signature,
            dictionaries: vec![],
            sprite_count: bank.sprite_count,
            sprites: vec![],
            vq_chunks: vec![],
            rle_jxl_chunks: vec![],
        });
        let mut counts = [0usize; 2];
        for chunk in std::mem::take(&mut bank.vq_chunks) {
            let rhs = head
                .payload
                .rhs_files
                .get(&chunk.rhs)
                .context("chunk RHS metadata missing")?;
            for (index, selected) in [true, false].into_iter().enumerate() {
                let mut template = chunk.clone();
                template
                    .sprite_ids
                    .retain(|id| keep.contains(id) == selected);
                template.base_ids = vec![None; template.sprite_ids.len()];
                template.base2_ids.clear();
                counts[index] += template.sprite_ids.len();
                let grids = template
                    .sprite_ids
                    .iter()
                    .map(|id| {
                        let row = rows.get(id).context("decoded row missing")?;
                        Ok(SpriteGrid {
                            cols: row.width / 4,
                            rows: row.height,
                            indices: row.packed_data.as_slice(),
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                let bases = vec![None; grids.len()];
                let groups =
                    encode_vq_groups(&template, &grids, &bases, &bases, Some(rhs), 1_048_576)?;
                if selected {
                    bank.vq_chunks.extend(groups);
                } else {
                    tail.sprite_bank
                        .as_mut()
                        .context("tail bank missing")?
                        .vq_chunks
                        .extend(groups);
                }
            }
        }
        if counts[1] == 0 {
            continue;
        }
        let dictionaries = &dd
            .sprite_bank
            .as_ref()
            .context("boot dictionary bank missing")?
            .dictionaries;
        let (mask_bytes, mask_report, opacity_batch) =
            opacity_masks(original_bank, dictionaries, &vq_ids)?;
        let opacity_key = format!("__startup_opacity/{name}.bin");
        let opacity_encoded = robin_assets::sprite_residency::encode(&opacity_batch)?;
        ensure!(
            !head.raw.contains_key(&opacity_key),
            "derived opacity key already exists"
        );
        head.raw
            .insert(opacity_key.clone(), opacity_encoded.clone());
        let head_bytes = zstd_compress_with_window(&encode_mission_native(&head), 30)?;
        let tail_bytes = zstd_compress_with_window(&encode_mission_native(&tail), 30)?;
        // Exercise the default all-parts merge contract using serialized files.
        let mut merged = ShippingMission::default();
        merged.merge_part(decode_mission_compressed(&head_bytes)?)?;
        merged.merge_part(decode_mission_compressed(&tail_bytes)?)?;
        materialize(&mut merged)?;
        ensure!(
            merged.raw.remove(&opacity_key).as_ref() == Some(&opacity_encoded),
            "derived opacity metadata changed during merge"
        );
        ensure!(
            encode_mission_native(&merged) == encode_mission_native(&original),
            "{name}: full materialized payload parity failed"
        );
        let before_sha = grid_sha(original_bank);
        let after_sha = grid_sha(merged.sprite_bank.as_ref().context("merged bank missing")?);
        ensure!(before_sha == after_sha, "{name}: grid hash mismatch");
        let tail_name = format!("rhs-tail/{name}");
        let tail_path = target.join(&tail_name);
        ensure!(!tail_path.exists(), "tail destination already exists");
        std::fs::create_dir_all(tail_path.parent().context("tail parent missing")?)?;
        std::fs::write(target.join(&name), &head_bytes)?;
        std::fs::write(tail_path, &tail_bytes)?;
        masks_raw_total += mask_report["raw_bytes"]
            .as_u64()
            .context("mask raw bytes missing")? as usize;
        masks_compressed_total += mask_bytes.len();
        if let Some(audit) = audit {
            let basename = Path::new(&name)
                .file_name()
                .context("part basename missing")?
                .to_str()
                .context("non-UTF8 filename")?;
            std::fs::write(audit.join(format!("{basename}.masks.zst")), &mask_bytes)?;
            std::fs::write(
                audit.join(format!("{basename}.json")),
                serde_json::to_vec_pretty(&mask_report)?,
            )?;
        }
        before_total += bytes.len();
        head_total += head_bytes.len();
        tail_total += tail_bytes.len();
        println!(
            "{}",
            serde_json::json!({"file":name,"tail":tail_name,"before_bytes":bytes.len(),"head_bytes":head_bytes.len(),"tail_bytes":tail_bytes.len(),"head_frames":counts[0],"tail_frames":counts[1],"original_grid_sha256":before_sha,"reencoded_grid_sha256":after_sha,"full_payload_parity":true,"derived_opacity_key":opacity_key,"derived_opacity_bytes":opacity_encoded.len(),"opacity_raw_bytes":mask_report["raw_bytes"],"opacity_zstd30_bytes":mask_bytes.len(),"opacity_sha256":mask_report["canonical_sha256"]})
        );
        tails.insert(name, tail_name);
    }
    ensure!(!tails.is_empty(), "no eligible character parts partitioned");
    for mission in dd.missions.values_mut() {
        append_tails(&mut mission.files, &tails);
    }
    for refs in dd.character_rhs_files.values_mut() {
        append_tails(refs, &tails);
    }
    append_tails(&mut dd.saved_world_rhs_files, &tails);
    std::fs::write(
        target.join("datadir.bin"),
        zstd_compress_with_window(&encode_native(&dd), 30)?,
    )?;
    println!(
        "{}",
        serde_json::json!({"first_frames_per_script":first,"parts":tails.len(),"before_bytes":before_total,"head_bytes":head_total,"tail_bytes":tail_total,"initial_saved_bytes":before_total as i64-head_total as i64,"total_expansion_bytes":head_total as i64+tail_total as i64-before_total as i64,"benchmark_only":true,"opacity_raw_bytes":masks_raw_total,"opacity_zstd30_bytes":masks_compressed_total})
    );
    Ok(())
}
