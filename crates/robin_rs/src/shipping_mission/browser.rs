//! Browser-only shipping I/O: speculative early mission downloads handed to
//! the normal loader, and the single-threaded fetch path.

use std::sync::Arc;

use anyhow::{Result, anyhow};
use robin_assets::shipping_datadir::ShippingDatadir;

use super::{
    early_download_keys, early_download_prefix, remove_early_owner, required_dependencies,
};

type EarlyDownload = futures::future::Shared<
    futures::future::LocalBoxFuture<'static, std::result::Result<Arc<Vec<u8>>, String>>,
>;

thread_local! {
    // JS futures stay on the browser main thread. The owner retains the exact
    // datadir allocation, preventing pointer reuse while entries are present.
    static EARLY_DOWNLOADS: std::cell::RefCell<std::collections::BTreeMap<(usize, String), (std::rc::Rc<()>, EarlyDownload)>> =
        const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
}

/// Replay-owned, nonserializable browser I/O lifetime. Dropping a failed or
/// abandoned launch aborts its requests and removes every unused handoff.
pub(crate) struct EarlyMissionDownloads {
    datadir: Arc<ShippingDatadir>,
    files: Vec<String>,
    abort: web_sys::AbortController,
    token: std::rc::Rc<()>,
}

impl Drop for EarlyMissionDownloads {
    fn drop(&mut self) {
        let identity = Arc::as_ptr(&self.datadir) as usize;
        EARLY_DOWNLOADS.with(|pending| {
            remove_early_owner(
                &mut pending.borrow_mut(),
                identity,
                &self.files,
                &self.token,
            );
        });
        self.abort.abort();
    }
}

/// Start only the first normal fetch batch. Decode, publication, audio setup,
/// renderer preparation and subsequent batches remain in ensure_loaded.
pub(crate) fn start_early_downloads(
    datadir: Arc<ShippingDatadir>,
    mission: &str,
    campaign: &robin_engine::campaign::Campaign,
    profiles: &robin_engine::profiles::ProfileManager,
) -> Result<EarlyMissionDownloads> {
    use futures::FutureExt as _;
    let dependencies = required_dependencies(&datadir, mission, campaign, profiles, false)?;
    let identity = Arc::as_ptr(&datadir) as usize;
    let files = early_download_keys(early_download_prefix(dependencies.files), |key| {
        EARLY_DOWNLOADS.with(|pending| pending.borrow().contains_key(&(identity, key.to_owned())))
    })?;
    let base = datadir
        .remote_base_url()
        .ok_or_else(|| anyhow!("early mission download requires a remote base URL"))?;
    let abort = web_sys::AbortController::new()
        .map_err(|error| anyhow!("create early mission abort controller: {error:?}"))?;
    let owner = EarlyMissionDownloads {
        datadir: datadir.clone(),
        files: files.clone(),
        abort,
        token: std::rc::Rc::new(()),
    };
    for file in files {
        let key = file.clone();
        if datadir.preloaded_file(&key).is_some() {
            continue;
        }
        let url = format!("{base}/{file}");
        let signal = owner.abort.signal();
        let pending = async move {
            use wasm_bindgen::JsCast as _;
            use wasm_bindgen_futures::JsFuture;
            let request = web_sys::RequestInit::new();
            request.set_signal(Some(&signal));
            let window =
                web_sys::window().ok_or_else(|| "browser window is unavailable".to_string())?;
            let response = JsFuture::from(window.fetch_with_str_and_init(&url, &request))
                .await
                .map_err(|error| format!("early fetch {url}: {error:?}"))?
                .dyn_into::<web_sys::Response>()
                .map_err(|_| format!("early fetch {url}: not a Response"))?;
            if !response.ok() {
                return Err(format!("early fetch {url}: HTTP {}", response.status()));
            }
            let buffer = response
                .array_buffer()
                .map_err(|error| format!("early fetch {url}: arrayBuffer: {error:?}"))?;
            let buffer = JsFuture::from(buffer)
                .await
                .map_err(|error| format!("early fetch {url}: body: {error:?}"))?;
            Ok(Arc::new(js_sys::Uint8Array::new(&buffer).to_vec()))
        }
        .boxed_local()
        .shared();
        EARLY_DOWNLOADS.with(|entries| {
            entries
                .borrow_mut()
                .insert((identity, key), (owner.token.clone(), pending.clone()))
        });
        // Poll now: constructing a future alone does not issue a fetch.
        let _ = pending.clone().now_or_never();
        wasm_bindgen_futures::spawn_local(async move {
            let _ = pending.await;
        });
    }
    tracing::info!(
        files = owner.files.len(),
        "startup timing: early replay fetch batch started"
    );
    Ok(owner)
}

pub(super) fn take_early_download(datadir: &ShippingDatadir, key: &str) -> Option<EarlyDownload> {
    EARLY_DOWNLOADS.with(|pending| {
        pending
            .borrow_mut()
            .remove(&(datadir as *const ShippingDatadir as usize, key.to_owned()))
            .map(|(_, download)| download)
    })
}

#[cfg(not(feature = "wasm-threads"))]
pub(super) async fn fetch(
    datadir: &ShippingDatadir,
    relative: &str,
) -> Result<super::CompressedPayload> {
    use super::{CompressedPayload, canonical_relative_file_key};
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen_futures::JsFuture;

    let key = canonical_relative_file_key(relative)?;
    if let Some(bytes) = datadir.preloaded_file(&key) {
        return Ok(CompressedPayload::Shared(bytes));
    }
    if let Some(pending) = take_early_download(datadir, &key) {
        let bytes = pending.await.map_err(anyhow::Error::msg)?;
        return Ok(CompressedPayload::Shared(bytes));
    }

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
    Ok(CompressedPayload::Owned(
        js_sys::Uint8Array::new(&buffer).to_vec(),
    ))
}
