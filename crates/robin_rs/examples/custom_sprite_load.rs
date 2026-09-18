//! Time authored sprite-family loading and fingerprint the resulting runtime frames.
//! Pass a directory containing `.sprites.vq.zst` files or one family file.
//! Use `RUST_LOG=debug` for stage timings. Fingerprints use native-endian RLE words.
use anyhow::Result;
use sha2::{Digest, Sha256};
use std::{path::PathBuf, time::Instant};

#[derive(clap::Parser, serde::Serialize, serde::Deserialize)]
struct Args {
    path: PathBuf,
}

fn main() -> Result<()> {
    robin_rs::init_tracing();
    let args = <Args as clap::Parser>::parse();
    let mut paths = if args.path.is_dir() {
        std::fs::read_dir(args.path)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::io::Result<Vec<_>>>()?
    } else {
        vec![args.path]
    };
    paths.retain(|path| path.to_string_lossy().ends_with(".sprites.vq.zst"));
    paths.sort();
    anyhow::ensure!(!paths.is_empty(), "no sprite families found");
    let mut total = 0;
    for path in paths {
        let bytes = std::fs::read(&path)?;
        let start = Instant::now();
        let caches = robin_assets::custom_sprites::family::read_selected_bytes(&bytes, None)?;
        let elapsed_ms = start.elapsed().as_millis();
        total += elapsed_ms;
        let mut hash = Sha256::new();
        let mut frames = 0;
        for (_, cache) in caches {
            for frame in cache.frames {
                frames += 1;
                hash.update(frame.width.to_le_bytes());
                hash.update(frame.height.to_le_bytes());
                hash.update((frame.packed_data.len() as u64).to_le_bytes());
                hash.update(bytemuck::cast_slice(&frame.packed_data));
            }
        }
        tracing::info!(target: "robin_rs::custom_sprite_load", path = %path.display(), elapsed_ms, frames, digest = %hex::encode(hash.finalize()), "sprite load benchmark");
    }
    tracing::info!(target: "robin_rs::custom_sprite_load", total_ms = total, "sprite load total (excluding fingerprint)");
    Ok(())
}
