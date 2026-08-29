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

use robin_assets::shipping_datadir::{
    ShippingDatadir, ShippingMission, decode_mission_compressed,
};
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
    let missions: BTreeMap<&String, &Vec<String>> =
        dd.missions.iter().map(|(name, m)| (name, &m.files)).collect();
    let json = serde_json::to_string(&missions).map_err(|e| JsError::new(&e.to_string()))?;
    DATADIR.with(|d| *d.borrow_mut() = Some(dd));
    Ok(json)
}

/// Decode one mission from its compressed part files (a JS array of
/// Uint8Array, in `ShippingMissionRef::files` order) and materialize its VQ
/// sprite chunks. Returns JSON phase timings and corpus counters.
#[wasm_bindgen]
pub fn bench_mission(parts: js_sys::Array) -> Result<String, JsError> {
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
    let decode_ms = now() - t0;

    let (mut chunks, mut blob_bytes, mut vq_sprites) = (0usize, 0usize, 0usize);
    let t1 = now();
    if let Some(bank) = merged.sprite_bank.as_mut() {
        chunks = bank.vq_chunks.len();
        blob_bytes = bank.vq_chunks.iter().map(|c| c.blob.len()).sum();
        vq_sprites = bank.vq_chunks.iter().map(|c| c.sprite_ids.len()).sum();
        bank.materialize_vq_chunks(&merged.rhs_files).map_err(js_err)?;
    }
    let materialize_ms = now() - t1;

    Ok(serde_json::json!({
        "part_bytes": part_bytes,
        "decode_ms": decode_ms,
        "materialize_ms": materialize_ms,
        "vq_chunks": chunks,
        "vq_blob_bytes": blob_bytes,
        "vq_sprites": vq_sprites,
    })
    .to_string())
}
