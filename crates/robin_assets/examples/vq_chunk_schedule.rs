//! Investigation tool: per-chunk serial VQ/RLE-JXL decode times for one
//! mission plus a simulated k-worker schedule over the real dependency graph.
//!
//! cargo build -p robin_assets --release --example vq_chunk_schedule
//! RAYON_NUM_THREADS=1 target/release/examples/vq_chunk_schedule <Data> <mission>
//!
//! Each chunk is materialized alone (in dependency order) so its wall time is
//! exact; the schedule simulation then assumes zero allocator contention and
//! zero main-thread apply cost, i.e. it is an upper bound on thread speedup.

use anyhow::{Context, Result, ensure};
use robin_assets::shipping_datadir::{ShippingDatadir, ShippingMission, decode_mission_compressed};
use std::{collections::HashMap, path::PathBuf, time::Instant};

#[derive(clap::Parser, serde::Serialize, serde::Deserialize)]
struct Args {
    root: PathBuf,
    mission: String,
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
    let chunks = std::mem::take(&mut bank.vq_chunks);
    let rle_chunks = std::mem::take(&mut bank.rle_jxl_chunks);

    // sprite id -> producing chunk index
    let mut producer: HashMap<u32, usize> = HashMap::new();
    for (index, chunk) in chunks.iter().enumerate() {
        for id in &chunk.sprite_ids {
            producer.entry(*id).or_insert(index);
        }
    }
    let deps: Vec<Vec<usize>> = chunks
        .iter()
        .enumerate()
        .map(|(index, chunk)| {
            let mut d: Vec<usize> = chunk
                .base_ids
                .iter()
                .chain(&chunk.base2_ids)
                .flatten()
                .filter_map(|id| producer.get(id).copied())
                .filter(|&p| p != index)
                .collect();
            d.sort_unstable();
            d.dedup();
            d
        })
        .collect();

    // Topological order, then time each chunk alone.
    let mut done = vec![false; chunks.len()];
    let mut order = Vec::new();
    while order.len() < chunks.len() {
        let before = order.len();
        for index in 0..chunks.len() {
            if !done[index] && deps[index].iter().all(|&d| done[d]) {
                done[index] = true;
                order.push(index);
            }
        }
        ensure!(order.len() > before, "dependency cycle");
    }
    let mut time_ms = vec![0.0; chunks.len()];
    for &index in &order {
        bank.vq_chunks = vec![chunks[index].clone()];
        let start = Instant::now();
        bank.materialize_vq_chunks(&payload.rhs_files)?;
        time_ms[index] = start.elapsed().as_secs_f64() * 1000.0;
    }
    let total: f64 = time_ms.iter().sum();
    let mut rows: Vec<usize> = (0..chunks.len()).collect();
    rows.sort_by(|&a, &b| time_ms[b].total_cmp(&time_ms[a]));
    println!("VQ chunks: {} serial total {:.0} ms", chunks.len(), total);
    println!("  ms      blobKB sprites deps  rhs");
    for &i in rows.iter().take(12) {
        println!(
            "{:7.0} {:9.0} {:7} {:4}  {}",
            time_ms[i],
            chunks[i].blob.len() as f64 / 1024.0,
            chunks[i].sprite_ids.len(),
            deps[i].len(),
            chunks[i].rhs
        );
    }
    // Longest weighted dependency path (infinite workers).
    let mut finish = vec![0.0f64; chunks.len()];
    for &index in &order {
        let ready = deps[index].iter().map(|&d| finish[d]).fold(0.0, f64::max);
        finish[index] = ready + time_ms[index];
    }
    let critical = finish.iter().copied().fold(0.0, f64::max);
    println!("critical path (inf workers): {critical:.0} ms");
    // k-worker greedy simulation: longest ready job first.
    for workers in [1usize, 2, 3, 4, 6, 8, 12] {
        let mut finish_at = vec![f64::INFINITY; chunks.len()];
        let mut started = vec![false; chunks.len()];
        let mut free_at = vec![0.0f64; workers];
        let mut remaining = chunks.len();
        while remaining > 0 {
            let (worker, now) = free_at
                .iter()
                .copied()
                .enumerate()
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap();
            let ready = (0..chunks.len())
                .filter(|&i| !started[i] && deps[i].iter().all(|&d| finish_at[d] <= now))
                .max_by(|&a, &b| time_ms[a].total_cmp(&time_ms[b]));
            match ready {
                Some(i) => {
                    started[i] = true;
                    finish_at[i] = now + time_ms[i];
                    free_at[worker] = finish_at[i];
                    remaining -= 1;
                }
                None => {
                    // Advance this worker to the next completion after `now`.
                    let next = finish_at
                        .iter()
                        .copied()
                        .filter(|&t| t > now && t.is_finite())
                        .fold(f64::INFINITY, f64::min);
                    ensure!(next.is_finite(), "scheduler stalled");
                    free_at[worker] = next;
                }
            }
        }
        let makespan = finish_at.iter().copied().fold(0.0, f64::max);
        println!(
            "workers {workers:2}: VQ makespan {makespan:6.0} ms (speedup {:.2}x)",
            total / makespan
        );
    }

    // RLE-JXL chunks: independent, time each.
    let mut rle_ms = Vec::new();
    for chunk in rle_chunks {
        let bytes: usize = chunk.jxl_blobs.iter().map(Vec::len).sum();
        let pixels = chunk.sprite_ids.len();
        bank.rle_jxl_chunks = vec![chunk];
        let start = Instant::now();
        bank.materialize_rle_jxl_chunks()?;
        rle_ms.push((start.elapsed().as_secs_f64() * 1000.0, bytes, pixels));
    }
    // Decoded resident size (what a decoded-asset cache would have to store).
    let vq_blob_bytes: usize = chunks.iter().map(|c| c.blob.len()).sum();
    let grid_bytes: usize = bank
        .sprites
        .iter()
        .map(|(_, s)| s.packed_data.len() * 2)
        .sum();
    let mut atlases = std::collections::HashSet::new();
    let mut atlas_bytes = 0usize;
    for (_, sprite) in &bank.sprites {
        if let Some(raster) = &sprite.raster
            && atlases.insert(std::sync::Arc::as_ptr(&raster.atlas))
        {
            atlas_bytes += raster.atlas.len() * 2;
        }
    }
    println!(
        "decoded: VQ grids {:.1} MB (from {:.1} MB blobs), RLE atlases {:.1} MB RGB565 ({} canvases)",
        grid_bytes as f64 / 1e6,
        vq_blob_bytes as f64 / 1e6,
        atlas_bytes as f64 / 1e6,
        atlases.len()
    );
    rle_ms.sort_by(|a, b| b.0.total_cmp(&a.0));
    let rle_total: f64 = rle_ms.iter().map(|r| r.0).sum();
    println!(
        "RLE-JXL chunks: {} serial total {rle_total:.0} ms; largest {:.0} ms ({} KB, {} sprites)",
        rle_ms.len(),
        rle_ms[0].0,
        rle_ms[0].1 / 1024,
        rle_ms[0].2
    );
    Ok(())
}
