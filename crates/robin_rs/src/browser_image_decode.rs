//! Browser-native AVIF decode for the web runtime.
//!
//! The web datadir ships every lossy image as AVIF. The Rust consumers
//! (`robin_assets` resource managers, terrain/minimap loaders, RLE sprite
//! materialization) stay synchronous and read pixels from
//! [`robin_assets::browser_images`]; this module fills that cache from the
//! async boot and mission-install paths by handing batches of blobs to
//! `js/avif_decode.js` (createImageBitmap + OffscreenCanvas readback).
//!
//! Must run on the main-thread `spawn_local` context: rayon workers never
//! return to their JS event loop, so a promise awaited there never resolves.

use anyhow::{Context, Result, anyhow};
use robin_assets::browser_images::{self, DecodedRgba, ImageKey, ImageScope};

/// Decoded pixels handed to the browser per batch. Every image of a batch
/// is decoded concurrently (off the main thread in Chrome); the budget caps
/// the RGBA held in JS at once (4 bytes per pixel) while still batching the
/// hundreds of tiny interface pictures together.
const BATCH_PIXEL_BUDGET: u64 = 24_000_000;
/// Upper bound on images per batch, independent of their size.
const BATCH_MAX_IMAGES: usize = 256;

#[wasm_bindgen::prelude::wasm_bindgen(module = "/js/avif_decode.js")]
extern "C" {
    #[wasm_bindgen::prelude::wasm_bindgen(catch, js_name = robinhoodDecodeAvifBatch)]
    async fn decode_avif_batch(
        blobs: js_sys::Array,
    ) -> Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue>;
}

fn js_error(error: wasm_bindgen::JsValue) -> anyhow::Error {
    anyhow!(
        "{}",
        js_sys::Error::try_from(error.clone())
            .ok()
            .map(|error| String::from(error.message()))
            .or_else(|| error.as_string())
            .unwrap_or_else(|| format!("{error:?}"))
    )
}

fn read_u32(object: &wasm_bindgen::JsValue, field: &str) -> Result<u32> {
    let value = js_sys::Reflect::get(object, &field.into())
        .map_err(js_error)
        .with_context(|| format!("decoded AVIF result has no {field}"))?;
    let number = value
        .as_f64()
        .ok_or_else(|| anyhow!("decoded AVIF result {field} is not a number"))?;
    if number.fract() != 0.0 || !(0.0..=f64::from(u32::MAX)).contains(&number) {
        return Err(anyhow!(
            "decoded AVIF result {field} is out of range: {number}"
        ));
    }
    Ok(number as u32)
}

/// Decode every not-yet-decoded AVIF in `blobs` and record the pixels under
/// `scope`. Duplicate blobs are decoded once. `progress(done, total)` is
/// called after each batch. Returns the number of images decoded.
pub async fn predecode<F>(blobs: &[&[u8]], scope: ImageScope, mut progress: F) -> Result<usize>
where
    F: FnMut(usize, usize),
{
    let mut seen = std::collections::HashSet::<ImageKey>::new();
    let mut todo: Vec<(&[u8], u64)> = Vec::new();
    for &bytes in blobs {
        if !seen.insert(browser_images::image_key(bytes)) {
            continue;
        }
        let info = browser_images::avif_info(bytes).context("inspect AVIF before decode")?;
        if browser_images::is_decoded(bytes)? {
            if scope == ImageScope::Boot {
                // Promote a mission-scoped entry the boot payload shares.
                let decoded = browser_images::decoded_rgba(bytes)?;
                browser_images::insert_decoded(bytes, scope, (*decoded).clone())?;
            }
            continue;
        }
        todo.push((bytes, u64::from(info.width) * u64::from(info.height)));
    }
    let total = todo.len();
    progress(0, total);
    if total == 0 {
        return Ok(0);
    }
    let started = web_time::Instant::now();
    let mut done = 0usize;
    let mut batches = 0usize;
    let mut rest = todo.as_slice();
    while !rest.is_empty() {
        let mut pixels = 0u64;
        let mut count = 0usize;
        for &(_, image_pixels) in rest {
            if count > 0
                && (count == BATCH_MAX_IMAGES || pixels + image_pixels > BATCH_PIXEL_BUDGET)
            {
                break;
            }
            pixels += image_pixels;
            count += 1;
        }
        let (batch, remaining) = rest.split_at(count);
        rest = remaining;
        let array = js_sys::Array::new_with_length(batch.len() as u32);
        for (index, (bytes, _)) in batch.iter().enumerate() {
            array.set(index as u32, js_sys::Uint8Array::from(*bytes).into());
        }
        let results = decode_avif_batch(array)
            .await
            .map_err(js_error)
            .context("browser AVIF decode")?;
        let results = js_sys::Array::from(&results);
        if results.length() as usize != batch.len() {
            return Err(anyhow!(
                "browser AVIF decode returned {} results for {} images",
                results.length(),
                batch.len()
            ));
        }
        for (index, (bytes, _)) in batch.iter().enumerate() {
            let result = results.get(index as u32);
            let width = read_u32(&result, "width")?;
            let height = read_u32(&result, "height")?;
            let rgba = js_sys::Reflect::get(&result, &"rgba".into())
                .map_err(js_error)
                .context("decoded AVIF result has no rgba")?;
            let rgba = js_sys::Uint8Array::new(&rgba).to_vec();
            browser_images::insert_decoded(
                bytes,
                scope,
                DecodedRgba {
                    width,
                    height,
                    rgba,
                },
            )
            .with_context(|| format!("record browser AVIF decode ({} bytes)", bytes.len()))?;
        }
        done += batch.len();
        batches += 1;
        progress(done, total);
    }
    tracing::info!(
        ?scope,
        images = total,
        batches,
        elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
        "startup timing: browser AVIF predecode"
    );
    Ok(total)
}
