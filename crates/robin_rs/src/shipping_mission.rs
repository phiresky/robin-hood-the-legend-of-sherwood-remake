//! Platform-specific fetch at the asynchronous mission-load boundary.

use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use robin_assets::shipping_datadir::{ShippingDatadir, decode_mission_compressed};

/// Ensure the selected mission's independently compressed shipping payload is
/// decoded and mounted before any synchronous level/resource loader runs.
pub async fn ensure_loaded(shipping: Option<&Arc<ShippingDatadir>>, mission: &str) -> Result<()> {
    let Some(datadir) = shipping else {
        return Ok(());
    };
    // An empty mission manifest is the loose-file/non-split compatibility
    // shape used by unit tests and development datadirs.
    if datadir.missions.is_empty() {
        return Ok(());
    }
    if datadir.is_mission_loaded(mission) {
        return datadir
            .activate_mission(mission)
            .with_context(|| format!("activate shipping mission {mission}"));
    }
    let reference = datadir
        .mission_ref(mission)
        .ok_or_else(|| anyhow!("shipping datadir does not contain mission {mission}"))?;
    let files = reference.files.clone();
    let missing: Vec<String> = files
        .iter()
        .filter(|file| datadir.cached_file(file).is_none())
        .cloned()
        .collect();
    let fetched = futures::future::try_join_all(missing.iter().map(|file| async move {
        let compressed = fetch(datadir, file)
            .await
            .with_context(|| format!("fetch shipping file {file}"))?;
        let payload = decode_mission_compressed(&compressed)
            .with_context(|| format!("decode shipping file {file}"))?;
        Ok::<_, anyhow::Error>((file.clone(), compressed.len(), payload))
    }))
    .await?;
    let fetched_bytes: usize = fetched.iter().map(|(_, bytes, _)| *bytes).sum();
    for (file, _, payload) in fetched {
        datadir.cache_file(&file, payload);
    }
    let mut parts = Vec::with_capacity(files.len());
    for file in &files {
        parts.push(
            datadir
                .cached_file(file)
                .ok_or_else(|| anyhow!("shipping file {file} disappeared after fetch"))?,
        );
    }
    datadir
        .install_mission_parts(mission, parts)
        .with_context(|| format!("install shipping mission {mission}"))?;
    tracing::info!(
        mission,
        files = files.len(),
        fetched_files = missing.len(),
        bytes = fetched_bytes,
        "shipping mission payload loaded"
    );
    Ok(())
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
async fn fetch(datadir: &ShippingDatadir, relative: &str) -> Result<Vec<u8>> {
    let path = datadir.source_file_path(relative)?;
    std::fs::read(&path).with_context(|| format!("read {}", path.display()))
}

#[cfg(target_os = "android")]
async fn fetch(_datadir: &ShippingDatadir, relative: &str) -> Result<Vec<u8>> {
    crate::android::read_bundled_asset(&format!("Data/{relative}"))
}

#[cfg(target_arch = "wasm32")]
async fn fetch(datadir: &ShippingDatadir, relative: &str) -> Result<Vec<u8>> {
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen_futures::JsFuture;

    let base = datadir
        .remote_base_url()
        .ok_or_else(|| anyhow!("browser shipping manifest has no remote base URL"))?;
    let url = format!("{base}/{}", relative.trim_start_matches('/'));
    let window = web_sys::window().ok_or_else(|| anyhow!("browser window is unavailable"))?;
    let response = JsFuture::from(window.fetch_with_str(&url))
        .await
        .map_err(|error| anyhow!("fetch {url}: {error:?}"))?
        .dyn_into::<web_sys::Response>()
        .map_err(|_| anyhow!("fetch {url}: result is not a Response"))?;
    if !response.ok() {
        return Err(anyhow!("fetch {url}: HTTP {}", response.status()));
    }
    let buffer = response
        .array_buffer()
        .map_err(|error| anyhow!("fetch {url}: arrayBuffer: {error:?}"))?;
    let buffer = JsFuture::from(buffer)
        .await
        .map_err(|error| anyhow!("fetch {url}: read body: {error:?}"))?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}
