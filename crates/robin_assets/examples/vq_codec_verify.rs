//! VQ sprite-codec verification + timing harness for one shipping mission.
//!
//! cargo build -p robin_assets --release --example vq_codec_verify
//! RAYON_NUM_THREADS=1 target/release/examples/vq_codec_verify <Data> <mission> [repeats]
//!
//! 1. Materializes every VQ chunk serially (production path) and prints the
//!    same FNV-1a over (bank id, packed grid) that `wasm_decode_bench` reports.
//! 2. Re-encodes every chunk with `encode_grids_shipping` and requires the
//!    output to equal the shipped blob byte for byte (encoder compatibility).
//! 3. Times pure `decode_grids_shipping` over all chunks (bases taken from the
//!    materialized bank) for `repeats` rounds and reports min/median.

use anyhow::{Context, Result, ensure};
use robin_assets::{
    shipping_datadir::{
        ShippingDatadir, ShippingMission, decode_mission_compressed, derive_chunk_self_refs,
    },
    sprite_codec::{SpriteGrid, decode_grids_shipping, encode_grids_shipping},
};
use std::{path::PathBuf, time::Instant};

#[derive(clap::Parser, serde::Serialize, serde::Deserialize)]
struct Args {
    root: PathBuf,
    mission: String,
    #[arg(default_value_t = 5)]
    repeats: usize,
    /// Skip the (slow) encoder byte-identity pass, e.g. for decode-only
    /// instruction counts.
    #[arg(long)]
    no_encode: bool,
}

fn main() -> Result<()> {
    let args = <Args as clap::Parser>::parse();
    let boot = std::fs::read(args.root.join("datadir.bin"))?;
    let manifest = ShippingDatadir::from_compressed_bytes(&boot)?;
    let reference = manifest
        .missions
        .get(&args.mission)
        .context("mission absent from manifest")?;
    let mut merged = ShippingMission::default();
    for file in &reference.files {
        let bytes = std::fs::read(args.root.join(file))?;
        merged.merge_part(decode_mission_compressed(&bytes)?)?;
    }
    let payload = &mut merged.payload;
    let bank = payload.sprite_bank.as_mut().context("no sprite bank")?;
    let chunks = bank.vq_chunks.clone();

    let start = Instant::now();
    bank.materialize_vq_chunks(&payload.rhs_files)?;
    let materialize_ms = start.elapsed().as_secs_f64() * 1000.0;
    let mut fnv: u64 = 0xcbf2_9ce4_8422_2325;
    for (id, sprite) in &bank.sprites {
        for byte in id
            .to_le_bytes()
            .iter()
            .chain(bytemuck::cast_slice::<u16, u8>(&sprite.packed_data))
        {
            fnv = (fnv ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3);
        }
    }
    println!(
        "materialize (serial, first run) {materialize_ms:.0} ms, grids_fnv {fnv:016x}, {} chunks",
        chunks.len()
    );

    let sprite = |id: u32| -> Result<&robin_assets::shipping_datadir::ShippingSprite> {
        let index = bank
            .sprites
            .binary_search_by_key(&id, |(id, _)| *id)
            .map_err(|_| anyhow::anyhow!("missing decoded sprite {id}"))?;
        Ok(&bank.sprites[index].1)
    };

    // Prepare owned-per-chunk inputs once.
    struct Prepared<'a> {
        alphabet: u16,
        grids: Vec<SpriteGrid<'a>>,
        dims: Vec<(u16, u16)>,
        bases: Vec<Option<&'a [u16]>>,
        bases2: Vec<Option<&'a [u16]>>,
        selfref: Vec<Option<robin_assets::sprite_codec::SelfRef>>,
        blob: &'a [u8],
    }
    let mut prepared = Vec::with_capacity(chunks.len());
    for chunk in &chunks {
        let grids = chunk
            .sprite_ids
            .iter()
            .map(|&id| {
                let row = sprite(id)?;
                Ok(SpriteGrid {
                    cols: row.width / 4,
                    rows: row.height,
                    indices: row.packed_data.as_slice(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let resolve = |ids: &[Option<u32>]| -> Result<Vec<Option<&[u16]>>> {
            if ids.is_empty() {
                return Ok(vec![None; chunk.sprite_ids.len()]);
            }
            ids.iter()
                .map(|id| {
                    id.map(|id| sprite(id).map(|s| s.packed_data.as_slice()))
                        .transpose()
                })
                .collect()
        };
        let selfref = if chunk.self_refs {
            let rhs = payload
                .rhs_files
                .get(&chunk.rhs)
                .context("missing RHS metadata")?;
            derive_chunk_self_refs(&rhs.profiles, &chunk.sprite_ids)
        } else {
            vec![None; chunk.sprite_ids.len()]
        };
        prepared.push(Prepared {
            alphabet: chunk.alphabet,
            dims: grids.iter().map(|g| (g.cols, g.rows)).collect(),
            grids,
            bases: resolve(&chunk.base_ids)?,
            bases2: resolve(&chunk.base2_ids)?,
            selfref,
            blob: &chunk.blob,
        });
    }

    // Encoder byte-identity.
    let start = Instant::now();
    let mut encoded_bytes = 0usize;
    for (index, p) in prepared.iter().enumerate().filter(|_| !args.no_encode) {
        let blob = encode_grids_shipping(
            p.alphabet,
            &p.grids,
            Some(&p.bases),
            Some(&p.bases2),
            &p.selfref,
        )?;
        ensure!(
            blob == p.blob,
            "re-encoded chunk {index} ({}) differs from the shipped blob ({} vs {} bytes)",
            chunks[index].rhs,
            blob.len(),
            p.blob.len()
        );
        encoded_bytes += blob.len();
    }
    println!(
        "encode: all {} chunks byte-identical ({encoded_bytes} bytes) in {:.0} ms",
        prepared.len(),
        start.elapsed().as_secs_f64() * 1000.0
    );

    // Pure decode timing.
    let mut rounds = Vec::with_capacity(args.repeats);
    for _ in 0..args.repeats {
        let start = Instant::now();
        for p in &prepared {
            let decoded = decode_grids_shipping(
                p.alphabet,
                &p.dims,
                Some(&p.bases),
                Some(&p.bases2),
                &p.selfref,
                p.blob,
            )?;
            ensure!(
                decoded.len() == p.grids.len()
                    && decoded
                        .iter()
                        .zip(&p.grids)
                        .all(|(d, g)| d.as_slice() == g.indices),
                "decode output mismatch"
            );
        }
        rounds.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    rounds.sort_by(f64::total_cmp);
    println!(
        "decode_grids_shipping x{}: min {:.0} ms, median {:.0} ms (all rounds {:?})",
        rounds.len(),
        rounds[0],
        rounds[rounds.len() / 2],
        rounds.iter().map(|r| r.round() as i64).collect::<Vec<_>>()
    );
    Ok(())
}
