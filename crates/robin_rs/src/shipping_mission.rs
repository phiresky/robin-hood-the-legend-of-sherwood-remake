//! Platform-specific fetch at the asynchronous mission-load boundary.

#[cfg(any(target_arch = "wasm32", test))]
use std::collections::BTreeSet;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
#[cfg(not(all(target_arch = "wasm32", feature = "wasm-threads")))]
use futures::StreamExt as _;
use robin_assets::shipping_datadir::{
    ShippingDatadir, ShippingMission, StagedMissionInstall, decode_mission_compressed,
};

#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(not(target_arch = "wasm32"))]
mod native;
mod planning;
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
mod streaming;

#[cfg(all(target_arch = "wasm32", not(feature = "wasm-threads")))]
use browser::fetch;
#[cfg(target_arch = "wasm32")]
pub(crate) use browser::{EarlyMissionDownloads, start_early_downloads};
#[cfg(not(target_arch = "wasm32"))]
use native::fetch;
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

/// One observable step at the asynchronous shipping-data boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissionLoadPhase {
    /// Download and decompress mission parts. Worker-pool streaming builds
    /// also decode activation-critical sprite chunks within this phase.
    Data,
    /// Decode the remaining sprite chunks while installing the merged
    /// mission (VQ chunks and RLE-JXL atlases).
    Sprites,
    Audio,
}

pub struct MissionLoadProgress<'a> {
    pub phase: MissionLoadPhase,
    /// Counted items shown to the player.
    pub completed: usize,
    pub total: usize,
    /// Weighted completion of `phase` in `0.0..=1.0`. Counted phases use
    /// `completed / total`; sprite decode weights items by decode cost.
    pub fraction: f32,
    pub file: Option<&'a str>,
}

impl<'a> MissionLoadProgress<'a> {
    pub fn counted(
        phase: MissionLoadPhase,
        completed: usize,
        total: usize,
        file: Option<&'a str>,
    ) -> Self {
        let fraction = if total == 0 {
            1.0
        } else {
            (completed as f32 / total as f32).min(1.0)
        };
        Self {
            phase,
            completed,
            total,
            fraction,
            file,
        }
    }
}

/// Wall-clock budget for decoding sprite chunks on the calling thread before
/// the loader reports progress and yields. On the single-threaded browser
/// build every yield is a macrotask boundary at which the compositor can show
/// the loading screen's latest frame; ~30 fps keeps the bar visibly moving.
const SPRITE_DECODE_YIELD_BUDGET: std::time::Duration = std::time::Duration::from_millis(33);

/// Work items per decode step. Native rayon decodes a batch in parallel; the
/// browser main thread decodes serially, so one item keeps steps short.
fn sprite_decode_step_items() -> usize {
    #[cfg(target_arch = "wasm32")]
    {
        1
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::available_parallelism().map_or(1, |threads| threads.get())
    }
}

/// Install `merged` with cooperative sprite decode: progress is reported and
/// the task yields to the runtime whenever [`SPRITE_DECODE_YIELD_BUDGET`]
/// elapses. Decode order and output are identical to a blocking install.
async fn install_with_progress<F>(
    datadir: &ShippingDatadir,
    mission: &str,
    merged: ShippingMission,
    progress: &mut F,
) -> Result<()>
where
    F: FnMut(MissionLoadProgress<'_>),
{
    let mut staged = datadir
        .stage_mission_install(mission, merged)
        .with_context(|| format!("install shipping mission {mission}"))?;
    let report = |staged: &StagedMissionInstall, progress: &mut F| {
        let sprites = staged.progress();
        progress(MissionLoadProgress {
            phase: MissionLoadPhase::Sprites,
            completed: sprites.completed_items,
            total: sprites.total_items,
            fraction: sprites.fraction(),
            file: None,
        });
    };
    report(&staged, progress);
    crate::window::yield_to_runtime().await;
    let step_items = sprite_decode_step_items();
    let mut last_yield = web_time::Instant::now();
    let mut yields = 0usize;
    while !staged
        .step(step_items)
        .with_context(|| format!("install shipping mission {mission}"))?
    {
        if last_yield.elapsed() >= SPRITE_DECODE_YIELD_BUDGET {
            report(&staged, progress);
            crate::window::yield_to_runtime().await;
            yields += 1;
            last_yield = web_time::Instant::now();
        }
    }
    report(&staged, progress);
    tracing::debug!(mission, yields, "sprite decode yielded to the runtime");
    datadir
        .finish_mission_install(staged)
        .with_context(|| format!("install shipping mission {mission}"))
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
    progress(MissionLoadProgress::counted(
        MissionLoadPhase::Data,
        0,
        total,
        None,
    ));
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
        progress(MissionLoadProgress::counted(
            MissionLoadPhase::Data,
            total,
            total,
            None,
        ));
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
            progress(MissionLoadProgress::counted(
                MissionLoadPhase::Data,
                completed,
                total,
                Some(&file),
            ));
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
    // AVIF sprite atlases, terrain maps and minimaps need browser-decoded
    // pixels before the synchronous install / level-load decoders use them.
    // The streaming build already decoded each part as it merged, so this is
    // a cheap no-op there. No progress callback: install_with_progress
    // restarts the Sprites phase, and the bar must never move backwards.
    #[cfg(target_arch = "wasm32")]
    crate::browser_image_decode::predecode(
        &merged.browser_image_blobs(),
        robin_assets::browser_images::ImageScope::Mission,
        |_, _| {},
    )
    .await
    .with_context(|| format!("decode AVIF images of shipping mission {mission}"))?;
    let install_start = web_time::Instant::now();
    install_with_progress(datadir, mission, merged, &mut progress).await?;
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
