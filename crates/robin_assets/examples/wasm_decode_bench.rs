//! Wasm sprite-decode benchmark.
//!
//! Times the shipping-datadir mission decode path on wasm32-unknown-unknown,
//! phase by phase, exactly as the web loader runs it at mission install:
//! `decode_mission_compressed` (zstd + bitcode) for each part file, then
//! [`robin_assets::shipping_datadir::ShippingSpriteBank::materialize_vq_chunks`]
//! (the `sprite_codec` hot path). Global installation/mounting is skipped —
//! this measures decode work only.
//!
//! Build and run (the wasm-bindgen CLI version must match Cargo.lock):
//!
//! ```text
//! cargo build --profile wasm-release --target wasm32-unknown-unknown \
//!     -p robin_assets --example wasm_decode_bench
//! wasm-bindgen --target web --out-dir <outdir> \
//!     target/wasm32-unknown-unknown/wasm-release/examples/wasm_decode_bench.wasm
//! node scripts/wasm_decode_bench.mjs <converted-datadir-root> <outdir>
//! ```
//!
//! where `<converted-datadir-root>` is a `convert_datadir --format shipping`
//! output directory (contains `Data/datadir.bin`).

#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::collections::BTreeMap;

use robin_assets::shipping_datadir::{ShippingDatadir, ShippingMission, decode_mission_compressed};
use wasm_bindgen::prelude::*;

thread_local! {
    static DATADIR: RefCell<Option<ShippingDatadir>> = const { RefCell::new(None) };
}

fn js_err(e: anyhow::Error) -> JsError {
    JsError::new(&format!("{e:#}"))
}

/// Decode `Data/datadir.bin` bytes and remember the datadir. Returns
/// `{"mission": ["relative part file", ...], ...}` as JSON; part paths are
/// relative to the directory containing `datadir.bin`.
#[wasm_bindgen]
pub fn bench_init(datadir: &[u8]) -> Result<String, JsError> {
    let dd = ShippingDatadir::from_compressed_bytes(datadir).map_err(js_err)?;
    let missions: BTreeMap<&String, &Vec<String>> = dd
        .missions
        .iter()
        .map(|(name, m)| (name, &m.files))
        .collect();
    let json = serde_json::to_string(&missions).map_err(|e| JsError::new(&e.to_string()))?;
    DATADIR.with(|d| *d.borrow_mut() = Some(dd));
    Ok(json)
}

/// Initialize the wasm-bindgen-rayon worker pool (wasm-threads builds only).
/// Must resolve before [`bench_mission_parallel`] runs; requires an
/// environment with Web Workers and `SharedArrayBuffer`.
#[cfg(feature = "wasm-threads")]
#[wasm_bindgen]
pub async fn bench_init_threads(threads: usize) -> Result<(), JsError> {
    robin_assets::wasm_threads::init_pool(threads)
        .await
        .map_err(js_err)
}

/// Decode one mission from its compressed part files (a JS array of
/// Uint8Array, in `ShippingMissionRef::files` order) and materialize its VQ
/// sprite chunks. Returns JSON phase timings and corpus counters.
#[wasm_bindgen]
pub fn bench_mission(parts: js_sys::Array) -> Result<String, JsError> {
    let now = js_sys::Date::now;
    let (mut merged, part_bytes, decode_ms) = decode_parts(parts)?;

    let stats = vq_stats(&merged);
    let t1 = now();
    if let Some(bank) = merged.sprite_bank.as_mut() {
        bank.materialize_vq_chunks(&merged.rhs_files)
            .map_err(js_err)?;
    }
    let materialize_ms = now() - t1;

    report(&merged, part_bytes, decode_ms, materialize_ms, stats)
}

/// [`bench_mission`] with the worker-pool materialization path of the
/// wasm-threads build ([`bench_init_threads`] must have resolved first;
/// without it this transparently measures the serial fallback).
#[cfg(feature = "wasm-threads")]
#[wasm_bindgen]
pub async fn bench_mission_parallel(parts: js_sys::Array) -> Result<String, JsError> {
    let now = js_sys::Date::now;
    let (mut merged, part_bytes, decode_ms) = decode_parts(parts)?;

    let stats = vq_stats(&merged);
    let t1 = now();
    if let Some(bank) = merged.sprite_bank.as_mut() {
        // Split borrow: the bank lives inside `merged`, whose `rhs_files`
        // field is read concurrently by the materializer.
        let rhs_files = &merged.rhs_files;
        bank.materialize_vq_chunks_parallel(rhs_files)
            .await
            .map_err(js_err)?;
    }
    let materialize_ms = now() - t1;

    report(&merged, part_bytes, decode_ms, materialize_ms, stats)
}

/// zstd+bitcode-decode and merge every part buffer of one mission.
fn decode_parts(parts: js_sys::Array) -> Result<(ShippingMission, usize, f64), JsError> {
    let now = js_sys::Date::now;
    let t0 = now();
    let mut merged = ShippingMission::default();
    let mut part_bytes = 0usize;
    for part in parts.iter() {
        let bytes = js_sys::Uint8Array::new(&part).to_vec();
        part_bytes += bytes.len();
        let part = decode_mission_compressed(&bytes).map_err(js_err)?;
        merged.merge_part(part).map_err(js_err)?;
    }
    Ok((merged, part_bytes, now() - t0))
}

/// VQ corpus counters captured before materialization consumes the chunks.
#[derive(Default)]
struct VqStats {
    chunks: usize,
    blob_bytes: usize,
    sprites: usize,
}

fn vq_stats(merged: &ShippingMission) -> VqStats {
    let Some(bank) = merged.sprite_bank.as_ref() else {
        return VqStats::default();
    };
    VqStats {
        chunks: bank.vq_chunks.len(),
        blob_bytes: bank.vq_chunks.iter().map(|c| c.blob.len()).sum(),
        sprites: bank.vq_chunks.iter().map(|c| c.sprite_ids.len()).sum(),
    }
}

fn report(
    merged: &ShippingMission,
    part_bytes: usize,
    decode_ms: f64,
    materialize_ms: f64,
    stats: VqStats,
) -> Result<String, JsError> {
    // FNV-1a over every sprite's materialized packed data, in bank-id order:
    // two builds of this bench must report identical hashes regardless of
    // compiler flags, or one of them miscompiled the codec.
    let mut grids_fnv: u64 = 0xcbf2_9ce4_8422_2325;
    if let Some(bank) = merged.sprite_bank.as_ref() {
        for (id, sprite) in &bank.sprites {
            for byte in id
                .to_le_bytes()
                .iter()
                .chain(bytemuck::cast_slice::<u16, u8>(&sprite.packed_data))
            {
                grids_fnv = (grids_fnv ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3);
            }
        }
    }

    Ok(serde_json::json!({
        "part_bytes": part_bytes,
        "decode_ms": decode_ms,
        "materialize_ms": materialize_ms,
        "vq_chunks": stats.chunks,
        "vq_blob_bytes": stats.blob_bytes,
        "vq_sprites": stats.sprites,
        "grids_fnv": format!("{grids_fnv:016x}"),
    })
    .to_string())
}
