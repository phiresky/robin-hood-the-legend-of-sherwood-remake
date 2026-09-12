//! Platform-specific fetch at the asynchronous mission-load boundary.

#[cfg(any(target_arch = "wasm32", test))]
use std::collections::BTreeSet;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
#[cfg(not(all(target_arch = "wasm32", feature = "wasm-threads")))]
use futures::StreamExt as _;
#[cfg(not(all(target_arch = "wasm32", feature = "wasm-threads")))]
use robin_assets::shipping_datadir::ShippingMission;
use robin_assets::shipping_datadir::{ShippingDatadir, decode_mission_compressed};

mod planning;
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
mod streaming;

use planning::{prioritize_mission_downloads, required_dependencies};
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
use streaming::{fetch_merge_materialize_streaming, spawn_deferred_sprite_tail};

enum CompressedPayload {
    Owned(Vec<u8>),
    #[cfg(target_arch = "wasm32")]
    Shared(Arc<Vec<u8>>),
}

impl std::ops::Deref for CompressedPayload {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Owned(bytes) => bytes,
            #[cfg(target_arch = "wasm32")]
            Self::Shared(bytes) => bytes,
        }
    }
}

const MISSION_FETCH_CONCURRENCY: usize = 8;

/// Keep the speculative prefix identical to the normal authoritative order.
#[cfg(any(target_arch = "wasm32", test))]
fn early_download_prefix(mut files: Vec<String>) -> Vec<String> {
    prioritize_mission_downloads(&mut files);
    files.truncate(MISSION_FETCH_CONCURRENCY);
    files
}

/// Validate the entire batch before allocating an owner or issuing requests.
/// A failed second registration cannot remove an earlier owner's handoffs.
#[cfg(any(target_arch = "wasm32", test))]
fn early_download_keys(
    files: Vec<String>,
    is_pending: impl Fn(&str) -> bool,
) -> Result<Vec<String>> {
    let keys = files
        .iter()
        .map(|file| canonical_relative_file_key(file))
        .collect::<Result<Vec<_>>>()?;
    let mut unique = BTreeSet::new();
    for key in &keys {
        if !unique.insert(key) {
            return Err(anyhow!("duplicate early mission file {key}"));
        }
        if is_pending(key) {
            return Err(anyhow!("early mission file {key} is already pending"));
        }
    }
    Ok(keys)
}

#[cfg(target_arch = "wasm32")]
type EarlyDownload = futures::future::Shared<
    futures::future::LocalBoxFuture<'static, std::result::Result<Arc<Vec<u8>>, String>>,
>;

#[cfg(target_arch = "wasm32")]
thread_local! {
    // JS futures stay on the browser main thread. The owner retains the exact
    // datadir allocation, preventing pointer reuse while entries are present.
    static EARLY_DOWNLOADS: std::cell::RefCell<std::collections::BTreeMap<(usize, String), (std::rc::Rc<()>, EarlyDownload)>> =
        const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
}

#[cfg(any(target_arch = "wasm32", test))]
fn remove_early_owner<T>(
    pending: &mut std::collections::BTreeMap<(usize, String), (std::rc::Rc<()>, T)>,
    identity: usize,
    files: &[String],
    owner: &std::rc::Rc<()>,
) {
    for file in files {
        let key = (identity, file.clone());
        if pending
            .get(&key)
            .is_some_and(|(current, _)| std::rc::Rc::ptr_eq(current, owner))
        {
            pending.remove(&key);
        }
    }
}

/// Replay-owned, nonserializable browser I/O lifetime. Dropping a failed or
/// abandoned launch aborts its requests and removes every unused handoff.
#[cfg(target_arch = "wasm32")]
pub(crate) struct EarlyMissionDownloads {
    datadir: Arc<ShippingDatadir>,
    files: Vec<String>,
    abort: web_sys::AbortController,
    token: std::rc::Rc<()>,
}

#[cfg(target_arch = "wasm32")]
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
#[cfg(target_arch = "wasm32")]
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

#[cfg(target_arch = "wasm32")]
fn take_early_download(datadir: &ShippingDatadir, key: &str) -> Option<EarlyDownload> {
    EARLY_DOWNLOADS.with(|pending| {
        pending
            .borrow_mut()
            .remove(&(datadir as *const ShippingDatadir as usize, key.to_owned()))
            .map(|(_, download)| download)
    })
}

/// One observable step at the asynchronous shipping-data boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissionLoadPhase {
    Data,
    Audio,
}

pub struct MissionLoadProgress<'a> {
    pub phase: MissionLoadPhase,
    pub completed: usize,
    pub total: usize,
    pub file: Option<&'a str>,
}

/// Validate and stage one exact local Full-content payload before the game
/// future can select a mission. The installed shipping index remains the
/// authority for which relative files are admissible.
pub fn preload_compressed(
    shipping: &ShippingDatadir,
    relative: &str,
    compressed: &[u8],
) -> Result<()> {
    let key = canonical_relative_file_key(relative)?;
    let referenced = shipping
        .missions
        .values()
        .flat_map(|mission| mission.files.iter())
        .chain(
            shipping
                .character_rhs_files
                .values()
                .flat_map(|files| files.iter()),
        )
        .chain(
            shipping
                .character_audio_files
                .values()
                .flat_map(|files| files.iter()),
        )
        .chain(shipping.saved_world_rhs_files.iter())
        .try_fold(false, |found, manifest_path| {
            Ok::<_, anyhow::Error>(found || canonical_relative_file_key(manifest_path)? == key)
        })?;
    if !referenced {
        return Err(anyhow!(
            "shipping file {relative:?} is not referenced by the installed manifest"
        ));
    }
    if shipping.preloaded_file(&key).is_some() {
        return Err(anyhow!("shipping file {relative:?} is already preloaded"));
    }
    decode_mission_compressed(compressed)
        .with_context(|| format!("decode preloaded shipping file {relative}"))?;
    shipping.cache_preloaded_file(key, compressed.to_vec())
}

fn canonical_relative_file_key(relative: &str) -> Result<String> {
    if relative.is_empty() || relative.trim() != relative {
        return Err(anyhow!(
            "shipping file path must be a non-empty relative path without surrounding whitespace"
        ));
    }
    if relative
        .chars()
        .any(|character| character.is_control() || matches!(character, ':' | '?' | '#' | '%'))
    {
        return Err(anyhow!(
            "shipping file path {relative:?} contains a forbidden character"
        ));
    }
    let normalized = relative.replace('\\', "/");
    if normalized.starts_with('/')
        || normalized
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        return Err(anyhow!(
            "shipping file path {relative:?} is not a contained relative path"
        ));
    }
    Ok(normalized)
}

/// Ensure the selected mission's independently compressed shipping payload is
/// decoded and mounted before any synchronous level/resource loader runs.
pub async fn ensure_loaded<F>(
    _application: &crate::host::ApplicationContext,
    shipping: Option<&Arc<ShippingDatadir>>,
    mission: &str,
    campaign: &robin_engine::campaign::Campaign,
    profiles: &robin_engine::profiles::ProfileManager,
    has_decoded_saved_world: bool,
    _warm_audio: bool,
    mut progress: F,
) -> Result<()>
where
    F: FnMut(MissionLoadProgress<'_>),
{
    let Some(datadir) = shipping else {
        return Ok(());
    };
    // An empty mission manifest is the loose-file/non-split compatibility
    // shape used by unit tests and development datadirs.
    if datadir.missions.is_empty() {
        return Ok(());
    }
    let plan_start = web_time::Instant::now();
    let dependencies = required_dependencies(
        datadir,
        mission,
        campaign,
        profiles,
        has_decoded_saved_world,
    )?;
    tracing::info!(
        mission,
        elapsed_ms = plan_start.elapsed().as_secs_f64() * 1000.0,
        "startup timing: mission dependency planning"
    );
    let total = dependencies.files.len();
    #[cfg(all(target_arch = "wasm32", feature = "audio"))]
    let audio = if _warm_audio {
        Some(_application.browser_audio().map_err(anyhow::Error::msg)?)
    } else {
        None
    };
    // Pause speculative menu/mission warmup while critical data is loading.
    // Required playback bypasses the pause; cancellation/errors release it.
    #[cfg(all(target_arch = "wasm32", feature = "audio"))]
    let mut audio_download_pause = audio
        .as_ref()
        .map(|audio| audio.pause_startup_warmup())
        .transpose()
        .map_err(anyhow::Error::msg)?;
    progress(MissionLoadProgress {
        phase: MissionLoadPhase::Data,
        completed: 0,
        total,
        file: None,
    });
    #[cfg(all(target_arch = "wasm32", feature = "audio"))]
    if _warm_audio && datadir.active_mission_name().as_deref() != Some(mission) {
        crate::audio_backend::clear_mission(
            audio
                .as_ref()
                .expect("warm audio session initialized above"),
        )
        .map_err(anyhow::Error::msg)
        .context("clear browser audio for mission transition")?;
    }
    if datadir.is_mission_loaded(mission) {
        datadir
            .activate_mission(mission)
            .with_context(|| format!("activate shipping mission {mission}"))?;
        datadir.set_active_exclamation_ids(dependencies.exclamation_ids);
        progress(MissionLoadProgress {
            phase: MissionLoadPhase::Data,
            completed: total,
            total,
            file: None,
        });
        #[cfg(all(target_arch = "wasm32", feature = "audio"))]
        if _warm_audio {
            crate::audio_backend::preload_active_mission_in_background(
                audio
                    .as_ref()
                    .expect("warm audio session initialized above"),
            )
            .map_err(anyhow::Error::msg)
            .context("start active mission browser audio warmup")?;
        }
        return Ok(());
    }
    let mut files = dependencies.files;
    prioritize_mission_downloads(&mut files);
    let exclamation_ids = dependencies.exclamation_ids;
    // Only the browser/audio closure mutates its captured pause guard.
    #[cfg_attr(not(all(target_arch = "wasm32", feature = "audio")), allow(unused_mut))]
    let mut downloads_finished = || {
        #[cfg(all(target_arch = "wasm32", feature = "audio"))]
        drop(audio_download_pause.take());
    };
    // Native (and plain single-threaded wasm) path: bounded-concurrency
    // fetch, merge on arrival, materialize inside `install_mission`.
    #[cfg(not(all(target_arch = "wasm32", feature = "wasm-threads")))]
    let (merged, fetched_bytes) = {
        use futures::TryStreamExt as _;
        let mut fetched = futures::stream::iter(files.iter().cloned().map(|file| async move {
            let compressed = fetch(datadir, &file)
                .await
                .with_context(|| format!("fetch shipping file {file}"))?;
            let bytes = compressed.len();
            let payload = decode_mission_compressed(&compressed)
                .with_context(|| format!("decode shipping file {file}"))?;
            Ok::<_, anyhow::Error>((file, bytes, payload))
        }))
        .buffer_unordered(MISSION_FETCH_CONCURRENCY);
        let mut fetched_bytes = 0usize;
        let mut completed = 0usize;
        let mut merged = ShippingMission::default();
        while let Some((file, bytes, payload)) = fetched.try_next().await? {
            fetched_bytes += bytes;
            tracing::debug!(mission, file, bytes, "shipping mission dependency fetched");
            merged
                .merge_part(payload)
                .with_context(|| format!("merge shipping file {file}"))?;
            completed += 1;
            progress(MissionLoadProgress {
                phase: MissionLoadPhase::Data,
                completed,
                total,
                file: Some(&file),
            });
            // On wasm, presenting from the observer does not become visible
            // until this task yields back to the browser event loop. Native
            // builds make this a no-op.
            crate::window::yield_to_runtime().await;
        }
        downloads_finished();
        (merged, fetched_bytes)
    };
    // Browser worker-pool build: prioritized requests, parts merged
    // as they arrive, and critical VQ sprite chunks materialized concurrently
    // with the remaining downloads; reinforcement-only chunks return as a
    // deferred tail that streams after activation. `install_mission` still
    // runs its own (now no-op for the critical set) materialization pass —
    // deferred chunks are held out of the bank's chunk list entirely.
    #[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
    let (merged, fetched_bytes, deferred_tail, early_terrain) = fetch_merge_materialize_streaming(
        datadir,
        mission,
        campaign,
        profiles,
        has_decoded_saved_world,
        &files,
        &mut progress,
        &mut downloads_finished,
    )
    .await?;
    let install_start = web_time::Instant::now();
    datadir
        .install_mission_parts(mission, std::iter::once(merged))
        .with_context(|| format!("install shipping mission {mission}"))?;
    tracing::info!(
        mission,
        elapsed_ms = install_start.elapsed().as_secs_f64() * 1000.0,
        "startup timing: mission install"
    );
    #[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
    if let Some(job) = early_terrain {
        _application
            .asset_cache()
            .map_err(anyhow::Error::msg)?
            .publish_early_terrain(job, datadir)
            .map_err(anyhow::Error::msg)
            .context("publish early terrain decode for installed mission")?;
    }
    datadir.set_active_exclamation_ids(exclamation_ids);
    #[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
    if let Some(tail) = deferred_tail {
        let payload = datadir.loaded_mission(mission).ok_or_else(|| {
            anyhow!("shipping mission {mission} disappeared before sprite streaming")
        })?;
        spawn_deferred_sprite_tail(mission.to_owned(), payload.sprite_streaming(), tail);
    }
    let payload = datadir
        .loaded_mission(mission)
        .ok_or_else(|| anyhow!("shipping mission {mission} disappeared after installation"))?;
    tracing::info!(
        mission,
        files = files.len(),
        fetched_files = files.len(),
        bytes = fetched_bytes,
        rhs_files = payload.rhs_files.len(),
        "shipping mission payload loaded"
    );
    #[cfg(all(target_arch = "wasm32", feature = "audio"))]
    if _warm_audio {
        crate::audio_backend::preload_active_mission_in_background(
            audio
                .as_ref()
                .expect("warm audio session initialized above"),
        )
        .map_err(anyhow::Error::msg)
        .context("start active mission browser audio warmup")?;
    }
    Ok(())
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
async fn fetch(datadir: &ShippingDatadir, relative: &str) -> Result<CompressedPayload> {
    let path = datadir.source_file_path(relative)?;
    std::fs::read(&path)
        .map(CompressedPayload::Owned)
        .with_context(|| format!("read {}", path.display()))
}

#[cfg(target_os = "android")]
async fn fetch(_datadir: &ShippingDatadir, relative: &str) -> Result<CompressedPayload> {
    crate::android::read_bundled_asset(&format!("Data/{relative}")).map(CompressedPayload::Owned)
}

#[cfg(all(target_arch = "wasm32", not(feature = "wasm-threads")))]
async fn fetch(datadir: &ShippingDatadir, relative: &str) -> Result<CompressedPayload> {
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

#[cfg(test)]
mod early_download_tests {
    #[test]
    fn dropping_consumed_owner_does_not_remove_replacement_or_other_datadir() {
        let first = std::rc::Rc::new(());
        let replacement = std::rc::Rc::new(());
        let files = vec!["rhs/a".to_owned(), "rhs/b".to_owned()];
        let mut entries = std::collections::BTreeMap::from([
            ((1, files[0].clone()), (first.clone(), 1)),
            ((1, files[1].clone()), (first.clone(), 2)),
            ((2, files[1].clone()), (first.clone(), 3)),
        ]);
        entries.remove(&(1, files[0].clone())); // Normal loader consumes A.
        entries.insert((1, files[0].clone()), (replacement.clone(), 4));
        super::remove_early_owner(&mut entries, 1, &files, &first);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[&(1, files[0].clone())].1, 4);
        assert_eq!(entries[&(2, files[1].clone())].1, 3);
        super::remove_early_owner(&mut entries, 1, &files, &replacement);
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn preflight_canonicalizes_owner_keys_and_rejects_aliases() {
        let keys = super::early_download_keys(vec![r"rhs\bank".into()], |_| false).unwrap();
        assert_eq!(keys, ["rhs/bank"]);
        assert!(
            super::early_download_keys(vec![r"rhs\bank".into(), "rhs/bank".into()], |_| false)
                .is_err()
        );
    }

    #[test]
    fn invalid_batch_never_observes_or_modifies_the_registry() {
        let reads = std::cell::Cell::new(0);
        let result =
            super::early_download_keys(vec!["rhs/valid".into(), "../escape".into()], |_| {
                reads.set(reads.get() + 1);
                false
            });
        assert!(result.is_err());
        assert_eq!(reads.get(), 0);
    }

    #[test]
    fn conflicting_batch_preserves_prior_owner() {
        let existing = std::collections::BTreeMap::from([("rhs/old".to_owned(), 7u64)]);
        let result = super::early_download_keys(vec!["rhs/new".into(), r"rhs\old".into()], |key| {
            existing.contains_key(key)
        });
        assert!(result.is_err());
        assert_eq!(
            existing,
            std::collections::BTreeMap::from([("rhs/old".to_owned(), 7u64)])
        );
    }

    #[test]
    fn prefix_uses_normal_priority_and_never_adds_dependencies() {
        let files = (0..12)
            .map(|i| format!("rhs/{i}"))
            .chain(["audio/a".into(), "terrain/a".into(), "missions/a".into()])
            .collect::<Vec<_>>();
        let mut normal = files.clone();
        super::prioritize_mission_downloads(&mut normal);
        assert_eq!(
            super::early_download_prefix(files),
            normal[..super::MISSION_FETCH_CONCURRENCY]
        );
    }
}

#[cfg(test)]
mod tests {
    use super::preload_compressed;
    use robin_assets::shipping_datadir::{
        ShippingDatadir, ShippingMission, ShippingMissionRef, encode_mission_native,
        zstd_max_compress,
    };

    fn compressed_empty_payload() -> Vec<u8> {
        zstd_max_compress(&encode_mission_native(&ShippingMission::default())).unwrap()
    }

    #[test]
    fn full_content_preload_accepts_only_manifest_references() {
        let mut datadir = ShippingDatadir::default();
        datadir.missions.insert(
            "Mission".into(),
            ShippingMissionRef {
                forest_level: false,
                files: vec!["missions/part.rhmission.zst".into()],
            },
        );
        let compressed = compressed_empty_payload();
        preload_compressed(&datadir, "missions\\part.rhmission.zst", &compressed).unwrap();
        assert!(
            datadir
                .preloaded_file("missions/part.rhmission.zst")
                .is_some()
        );
        assert!(preload_compressed(&datadir, "../part", &compressed).is_err());
        assert!(preload_compressed(&datadir, "missions/other", &compressed).is_err());
    }

    #[test]
    fn full_content_preload_fails_before_caching_invalid_payload() {
        let mut datadir = ShippingDatadir::default();
        datadir.missions.insert(
            "Mission".into(),
            ShippingMissionRef {
                forest_level: false,
                files: vec!["missions/part".into()],
            },
        );
        assert!(preload_compressed(&datadir, "missions/part", b"not zstd").is_err());
        assert!(datadir.preloaded_file("missions/part").is_none());
    }
}
