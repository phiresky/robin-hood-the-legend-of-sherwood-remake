//! Platform-specific fetch at the asynchronous mission-load boundary.

use std::collections::BTreeSet;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use futures::StreamExt as _;
use robin_assets::shipping_datadir::{ShippingDatadir, ShippingMission, decode_mission_compressed};

enum CompressedPayload {
    Owned(Vec<u8>),
    Shared(Arc<Vec<u8>>),
}

impl std::ops::Deref for CompressedPayload {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Owned(bytes) => bytes,
            Self::Shared(bytes) => bytes,
        }
    }
}

#[cfg(not(all(target_arch = "wasm32", feature = "wasm-threads")))]
const MISSION_FETCH_CONCURRENCY: usize = 8;

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
    let dependencies = required_dependencies(
        datadir,
        mission,
        campaign,
        profiles,
        has_decoded_saved_world,
    )?;
    let total = dependencies.files.len();
    progress(MissionLoadProgress {
        phase: MissionLoadPhase::Data,
        completed: 0,
        total,
        file: None,
    });
    #[cfg(all(target_arch = "wasm32", feature = "audio"))]
    if _warm_audio && datadir.active_mission_name().as_deref() != Some(mission) {
        crate::audio_backend::clear_mission()
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
            crate::audio_backend::preload_active_mission(|audio| {
                progress(MissionLoadProgress {
                    phase: MissionLoadPhase::Audio,
                    completed: audio.completed,
                    total: audio.total,
                    file: audio.file,
                });
            })
            .await
            .map_err(anyhow::Error::msg)
            .context("warm active mission browser audio")?;
        }
        return Ok(());
    }
    let files = dependencies.files;
    let exclamation_ids = dependencies.exclamation_ids;
    // Fresh install epoch: drops the previous mission's late-grid cells and
    // invalidates any still-running background sprite-streaming driver.
    // Deliberately after the loaded-mission early return above — a restart
    // of the same mission keeps its (possibly still-filling) cells.
    let install_epoch = robin_assets::late_sprites::begin_epoch();
    #[cfg(not(all(target_arch = "wasm32", feature = "wasm-threads")))]
    let _ = install_epoch;
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
        (merged, fetched_bytes)
    };
    // Browser worker-pool build: all requests in flight at once, parts merged
    // as they arrive, and critical VQ sprite chunks materialized concurrently
    // with the remaining downloads; reinforcement-only chunks return as a
    // deferred tail that streams after activation. `install_mission` still
    // runs its own (now no-op for the critical set) materialization pass —
    // deferred chunks are held out of the bank's chunk list entirely.
    #[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
    let (merged, fetched_bytes, deferred_tail) = fetch_merge_materialize_streaming(
        datadir,
        mission,
        campaign,
        profiles,
        has_decoded_saved_world,
        &files,
        &mut progress,
    )
    .await?;
    datadir
        .install_mission_parts(mission, std::iter::once(merged))
        .with_context(|| format!("install shipping mission {mission}"))?;
    datadir.set_active_exclamation_ids(exclamation_ids);
    #[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
    if let Some(tail) = deferred_tail {
        spawn_deferred_sprite_tail(mission.to_owned(), install_epoch, tail);
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
        crate::audio_backend::preload_active_mission(|audio| {
            progress(MissionLoadProgress {
                phase: MissionLoadPhase::Audio,
                completed: audio.completed,
                total: audio.total,
                file: audio.file,
            });
        })
        .await
        .map_err(anyhow::Error::msg)
        .context("warm active mission browser audio")?;
    }
    Ok(())
}

/// Deferred sprite-chunk work handed to [`spawn_deferred_sprite_tail`] after
/// the mission activates: everything the background driver needs without
/// touching the installed (shared, immutable) mission payload.
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
struct DeferredSpriteTail {
    /// Sparse row clone of the mission bank (rows share their grids via
    /// `Arc`, so this is cheap). Critical chunks are already materialized
    /// into these rows; the driver applies its own decodes here so
    /// dependent deferred chunks see their base grids.
    bank: robin_assets::shipping_datadir::ShippingSpriteBank,
    /// RHS metadata for deferred chunks that derive within-chunk
    /// self-references at decode time.
    rhs_files: std::collections::BTreeMap<String, robin_assets::shipping_datadir::RhsData>,
    chunks: Vec<robin_assets::shipping_datadir::SpriteVqChunk>,
}

/// Byte-progress shared between the concurrent part fetches and the install
/// loop's progress reporting. Part sizes are learned from each response's
/// `Content-Length` header (all requests go out at once, so every size is
/// known within the first round-trips) and corrected to the actual body
/// length on completion.
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
#[derive(Default)]
struct FetchByteProgress {
    /// Body bytes received so far, across all parts.
    received: std::sync::atomic::AtomicU64,
    /// Sum of the known (claimed or completed) part sizes.
    known_total: std::sync::atomic::AtomicU64,
    /// Number of parts whose size is known.
    known_files: std::sync::atomic::AtomicUsize,
    /// Largest known part size — the estimate for still-unknown parts.
    max_known: std::sync::atomic::AtomicU64,
}

#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
impl FetchByteProgress {
    fn add_known(&self, bytes: u64) {
        use std::sync::atomic::Ordering;
        self.known_total.fetch_add(bytes, Ordering::Relaxed);
        self.known_files.fetch_add(1, Ordering::Relaxed);
        self.max_known.fetch_max(bytes, Ordering::Relaxed);
    }

    /// Estimated total bytes across `files_total` parts: known sizes plus
    /// the largest known size for each part whose headers are still in
    /// flight (a deliberate overestimate, so the fraction rises rather
    /// than retreats as real sizes arrive).
    fn estimated_total(&self, files_total: usize) -> u64 {
        use std::sync::atomic::Ordering;
        let known = self.known_total.load(Ordering::Relaxed);
        let unknown = files_total.saturating_sub(self.known_files.load(Ordering::Relaxed)) as u64;
        let per_file = self.max_known.load(Ordering::Relaxed).max(64 * 1024);
        known + unknown * per_file
    }
}

/// Weighted install-progress model: fetch progress by bytes received,
/// decode progress by critical VQ blob bytes materialized, combined into
/// one monotonic fraction reported as `completed`/`total` work units. The
/// only tuning constant is the relative per-byte cost of a blob decode
/// versus a network byte ([`Self::DECODE_BYTE_COST`], calibrated from the
/// measured install: decoding dominates the wall clock).
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
struct InstallWorkModel {
    files_total: usize,
    fetch: Arc<FetchByteProgress>,
    /// Critical (activation-blocking) VQ blob bytes discovered so far.
    decode_total: u64,
    /// Critical VQ blob bytes whose grids are materialized.
    decode_done: u64,
    /// Monotonic guard: totals grow while parts stream in, so the raw
    /// fraction can dip; never report backwards motion.
    emitted: f32,
    /// Last reported unit count — repeat emits (progress ticks) are
    /// suppressed so the loading log is not flooded with identical lines.
    last_units: usize,
}

#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
impl InstallWorkModel {
    /// Measured on the browser install: context-model blob decode costs a
    /// few times more wall clock per byte than fetching a byte does.
    const DECODE_BYTE_COST: f64 = 6.0;
    /// Reported progress granularity (`completed` out of `total`).
    const UNITS: usize = 100;

    fn fraction(&self) -> f32 {
        use std::sync::atomic::Ordering;
        let received = self.fetch.received.load(Ordering::Relaxed) as f64;
        let fetch_total = self.fetch.estimated_total(self.files_total) as f64;
        let done = received + Self::DECODE_BYTE_COST * self.decode_done as f64;
        let total = (fetch_total + Self::DECODE_BYTE_COST * self.decode_total as f64).max(1.0);
        (done / total).min(1.0) as f32
    }

    fn emit<F>(&mut self, progress: &mut F, label: &str)
    where
        F: FnMut(MissionLoadProgress<'_>),
    {
        self.emitted = self.emitted.max(self.fraction());
        let completed = (self.emitted * Self::UNITS as f32).round() as usize;
        if completed == self.last_units {
            return;
        }
        self.last_units = completed;
        progress(MissionLoadProgress {
            phase: MissionLoadPhase::Data,
            completed,
            total: Self::UNITS,
            file: Some(label),
        });
    }
}

/// Runtime partition of the mission's VQ sprite chunks into the critical
/// set (blocks activation) and the deferrable tail (streams afterwards).
///
/// Deferrable candidates are the reinforcement-only gang characters: gang
/// profiles eligible for runtime reinforcement selection whose RHS is not
/// also required by the mission team. Names are *excluded* from the
/// candidate set — turning them critical — when the level lists them among
/// its start entities (authored soldiers, civilians, PCs to rescue), and
/// *promoted* when any critical chunk names them as a coding base (family
/// hubs must decode before their variants). Everything not in the candidate
/// set is critical by default, so a misjudged chunk can only err toward
/// blocking activation — never toward a missing start-visible sprite.
///
/// Compiled on every target (only the streaming driver is browser-only) so
/// the partition rules stay unit-testable — the safety of the whole feature
/// rests on them.
#[cfg_attr(
    not(all(target_arch = "wasm32", feature = "wasm-threads")),
    allow(dead_code)
)]
struct SpriteDeferral {
    candidates: BTreeSet<String>,
    parked: Vec<robin_assets::shipping_datadir::SpriteVqChunk>,
    level_filtered: bool,
    forest_level: bool,
}

#[cfg_attr(
    not(all(target_arch = "wasm32", feature = "wasm-threads")),
    allow(dead_code)
)]
impl SpriteDeferral {
    fn new(
        datadir: &ShippingDatadir,
        mission: &str,
        campaign: &robin_engine::campaign::Campaign,
        profiles: &robin_engine::profiles::ProfileManager,
    ) -> Result<Self> {
        let reference = datadir
            .mission_ref(mission)
            .ok_or_else(|| anyhow!("shipping datadir does not contain mission {mission}"))?;
        let forest_level = reference.forest_level;
        // The Sherwood camp (always campaign mission 0) and its outro are
        // populated from the uninstanced gang itself — exactly the set that
        // is deferrable everywhere else — so nothing may be deferred there.
        let sherwood = campaign
            .missions
            .get(campaign.get_sherwood_mission_idx())
            .and_then(|m| m.profile_idx)
            .and_then(|idx| profiles.missions.get(idx as usize))
            .is_some_and(|profile| profile.mission_filename == mission)
            || mission == "SherwoodOutro";
        if sherwood {
            return Ok(Self {
                candidates: BTreeSet::new(),
                parked: Vec::new(),
                level_filtered: false,
                forest_level,
            });
        }
        // These lookups were all validated by `required_dependencies`
        // moments earlier; failures here are genuine data errors.
        let mut team = BTreeSet::new();
        for &character_index in &campaign.mission_team_indices {
            let description = campaign
                .characters
                .get(character_index)
                .ok_or_else(|| anyhow!("mission team references missing character"))?;
            let profile = description
                .character_profile_idx
                .ok_or_else(|| anyhow!("mission-team character has no profile"))?;
            team.insert(normalize_robin_profile(profiles, profile.0, forest_level)?);
        }
        let mut candidates = BTreeSet::new();
        for &character_index in &campaign.gang_indices {
            let description = campaign
                .characters
                .get(character_index)
                .ok_or_else(|| anyhow!("gang references missing campaign character"))?;
            if description.instanced {
                continue;
            }
            let profile_index = description
                .character_profile_idx
                .ok_or_else(|| anyhow!("gang character has no profile"))?;
            let profile = profiles
                .get_character(profile_index)
                .ok_or_else(|| anyhow!("gang character references missing profile"))?;
            if profile.vip {
                continue;
            }
            let normalized = normalize_robin_profile(profiles, profile_index.0, forest_level)?;
            if team.contains(&normalized) {
                continue;
            }
            let filename = &profiles.characters[normalized as usize].filename;
            candidates.insert(format!("Characters/{filename}.rhs"));
        }
        Ok(Self {
            candidates,
            parked: Vec::new(),
            level_filtered: false,
            forest_level,
        })
    }

    /// Sort freshly merged chunks into `pending` (critical) or the parked
    /// list, then promote coding bases named by critical chunks. Adds the
    /// blob bytes of every chunk that lands in `pending` to `decode_total`.
    fn absorb(
        &mut self,
        incoming: &mut Vec<robin_assets::shipping_datadir::SpriteVqChunk>,
        pending: &mut Vec<robin_assets::shipping_datadir::SpriteVqChunk>,
        decode_total: &mut u64,
    ) {
        for chunk in incoming.drain(..) {
            if self.candidates.contains(&chunk.rhs) {
                self.parked.push(chunk);
            } else {
                *decode_total += chunk.blob.len() as u64;
                pending.push(chunk);
            }
        }
        self.promote_bases(pending, decode_total);
    }

    /// Turn one candidate name critical: parked chunks for it move into
    /// `pending`. No-op for names that are not (or no longer) candidates.
    fn remove_candidate(
        &mut self,
        name: &str,
        pending: &mut Vec<robin_assets::shipping_datadir::SpriteVqChunk>,
        decode_total: &mut u64,
    ) -> bool {
        if !self.candidates.remove(name) {
            return false;
        }
        let mut index = 0;
        while index < self.parked.len() {
            if self.parked[index].rhs == name {
                let chunk = self.parked.swap_remove(index);
                *decode_total += chunk.blob.len() as u64;
                pending.push(chunk);
            } else {
                index += 1;
            }
        }
        true
    }

    /// Fixpoint: any candidate named as `base_rhs`/`base2_rhs` by a chunk in
    /// the critical `pending` list becomes critical itself (its grids gate
    /// the critical chunk's decode). Chunks moved out of the parked list are
    /// re-scanned, so hub-of-hub chains resolve fully.
    fn promote_bases(
        &mut self,
        pending: &mut Vec<robin_assets::shipping_datadir::SpriteVqChunk>,
        decode_total: &mut u64,
    ) {
        loop {
            let referenced: Vec<String> = pending
                .iter()
                .flat_map(|chunk| {
                    chunk
                        .base_rhs
                        .iter()
                        .cloned()
                        .chain((!chunk.base2_rhs.is_empty()).then(|| chunk.base2_rhs.clone()))
                })
                .filter(|name| self.candidates.contains(name))
                .collect();
            if referenced.is_empty() {
                return;
            }
            for name in referenced {
                self.remove_candidate(&name, pending, decode_total);
            }
        }
    }

    /// Once the level payload is merged: names required by start entities
    /// (authored soldiers, civilians, PCs to rescue) become critical. A
    /// candidate misclassification can only leave a start sprite streaming
    /// briefly (safe-skip draw), so unresolved profile references warn
    /// rather than fail here.
    fn exclude_level_requirements(
        &mut self,
        level: &robin_engine::level_data::LoadedLevel,
        profiles: &robin_engine::profiles::ProfileManager,
        pending: &mut Vec<robin_assets::shipping_datadir::SpriteVqChunk>,
        decode_total: &mut u64,
    ) {
        self.level_filtered = true;
        let names = self.level_start_rhs_names(level, profiles);
        self.exclude_names(&names, pending, decode_total);
    }

    /// RHS names of every character the level itself places at mission
    /// start. Unresolvable profile references only cost prioritization
    /// accuracy (worst case: a brief safe-skip), so they warn rather than
    /// fail.
    fn level_start_rhs_names(
        &self,
        level: &robin_engine::level_data::LoadedLevel,
        profiles: &robin_engine::profiles::ProfileManager,
    ) -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        for soldier in &level.mission.soldiers {
            if let Some(profile) = profiles.soldiers.get(soldier.profile_number as usize) {
                names.insert(format!("Characters/{}.rhs", profile.filename));
            }
        }
        for civilian in &level.mission.civilians {
            if let Some(profile) = profiles.civilians.get(civilian.profile_number as usize) {
                names.insert(format!("Characters/{}.rhs", profile.filename));
            }
        }
        for rescue in &level.mission.pcs_to_rescue {
            match normalize_robin_profile(profiles, rescue.profile_index, self.forest_level) {
                Ok(normalized) => {
                    if let Some(profile) = profiles.characters.get(normalized as usize) {
                        names.insert(format!("Characters/{}.rhs", profile.filename));
                    }
                }
                Err(error) => tracing::warn!(
                    profile_index = rescue.profile_index,
                    "cannot resolve rescue-PC profile for sprite prioritization: {error:#}"
                ),
            }
        }
        names
    }

    /// Make every named RHS critical, then re-run base promotion.
    fn exclude_names(
        &mut self,
        names: &BTreeSet<String>,
        pending: &mut Vec<robin_assets::shipping_datadir::SpriteVqChunk>,
        decode_total: &mut u64,
    ) {
        for name in names {
            self.remove_candidate(name, pending, decode_total);
        }
        self.promote_bases(pending, decode_total);
    }
}

/// Streaming mission load for the browser worker-pool build.
///
/// Every part request is issued simultaneously — the browser's network stack
/// multiplexes the actual transfers — and each response is processed the
/// moment it arrives: the zstd+bitcode part decode runs on a rayon worker
/// (inline on the serial fallback), the decoded part merges immediately, and
/// every dependency-ready *critical* VQ sprite chunk is dispatched to the
/// pool right away. The main thread never blocks: it awaits whichever event
/// completes next (a part arrival, a finished chunk decode, or a progress
/// tick) via `futures::select!`.
///
/// With a worker pool, chunks belonging to reinforcement-only gang
/// characters (see [`SpriteDeferral`]) are *not* decoded here: they return
/// as the third tuple element for [`spawn_deferred_sprite_tail`] to stream
/// after the mission activates. Without a pool everything stays blocking, as
/// before.
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
async fn fetch_merge_materialize_streaming<F>(
    datadir: &ShippingDatadir,
    mission: &str,
    campaign: &robin_engine::campaign::Campaign,
    profiles: &robin_engine::profiles::ProfileManager,
    has_decoded_saved_world: bool,
    files: &[String],
    progress: &mut F,
) -> Result<(ShippingMission, usize, Option<DeferredSpriteTail>)>
where
    F: FnMut(MissionLoadProgress<'_>),
{
    use futures::FutureExt as _;
    use robin_assets::shipping_datadir::{
        RleJxlDecodeScheduler, ShippingSpriteBank, SpriteRleJxlChunk, SpriteVqChunk,
        VqDecodeScheduler,
    };
    use robin_assets::wasm_threads;

    let total = files.len();
    let pooled = wasm_threads::pool_threads() > 0;
    let fetch_progress = Arc::new(FetchByteProgress::default());
    let mut work = InstallWorkModel {
        files_total: total,
        fetch: Arc::clone(&fetch_progress),
        decode_total: 0,
        decode_done: 0,
        emitted: 0.0,
        last_units: usize::MAX,
    };
    // A decoded saved world can contain any entity — reinforcements that
    // already spawned included — so "present at mission start" cannot be
    // derived from the authored level: keep every chunk activation-blocking.
    let mut deferral = if pooled && !has_decoded_saved_world {
        Some(SpriteDeferral::new(datadir, mission, campaign, profiles)?)
    } else {
        None
    };
    let mut fetched = futures::stream::iter(files.iter().cloned().map(|file| {
        let fetch_progress = Arc::clone(&fetch_progress);
        async move {
            let compressed = fetch_counted(datadir, &file, &fetch_progress)
                .await
                .with_context(|| format!("fetch shipping file {file}"))?;
            let bytes = compressed.len();
            // Pure compute; overlap it with the remaining downloads when the
            // pool exists.
            let payload = if wasm_threads::pool_threads() > 0 {
                wasm_threads::run_on_pool(move || decode_mission_compressed(&compressed)).await?
            } else {
                decode_mission_compressed(&compressed)
            };
            let payload = payload.with_context(|| format!("decode shipping file {file}"))?;
            Ok::<_, anyhow::Error>((file, bytes, payload))
        }
    }))
    .buffer_unordered(total.max(1))
    .fuse();

    enum Event {
        Part(Option<Result<(String, usize, ShippingMission)>>),
        Decoded(Result<Option<(SpriteVqChunk, Vec<(u32, Vec<u16>)>)>>),
        Tick,
    }

    let mut merged = ShippingMission::default();
    let mut fetched_bytes = 0usize;
    let mut pending_chunks: Vec<SpriteVqChunk> = Vec::new();
    let mut scheduler = VqDecodeScheduler::default();
    let mut label = String::from("downloading mission parts");
    // RLE-JXL chunks (web recipe) have no cross-chunk dependencies, so each
    // is dispatched to the pool the moment its part merges; results are
    // applied in the drain phase below. On the serial fallback they stay in
    // the bank and `install_mission` decodes them.
    let mut pending_rle: Vec<SpriteRleJxlChunk> = Vec::new();
    let mut rle_scheduler = RleJxlDecodeScheduler::default();
    loop {
        // Boxed: `select!` polls through `&mut`, which needs `Unpin`, and an
        // `async fn` future is not. One small allocation per event is noise
        // next to a network fetch or chunk decode.
        let mut tick = Box::pin(crate::window::sleep_ms(150)).fuse();
        let event = if pooled && scheduler.has_in_flight() {
            let mut next_decoded = Box::pin(scheduler.next_decoded()).fuse();
            futures::select! {
                part = fetched.next() => Event::Part(part),
                decoded = next_decoded => Event::Decoded(decoded),
                _ = tick => Event::Tick,
            }
        } else {
            futures::select! {
                part = fetched.next() => Event::Part(part),
                _ = tick => Event::Tick,
            }
        };
        match event {
            Event::Part(None) => break,
            Event::Part(Some(part)) => {
                let (file, bytes, payload) = part?;
                fetched_bytes += bytes;
                tracing::debug!(mission, file, bytes, "shipping mission dependency fetched");
                merged
                    .merge_part(payload)
                    .with_context(|| format!("merge shipping file {file}"))?;
                label = file;
                if let Some(bank) = merged.sprite_bank.as_mut() {
                    let mut incoming = std::mem::take(&mut bank.vq_chunks);
                    if let Some(deferral) = deferral.as_mut() {
                        if !deferral.level_filtered
                            && let Some(level) = merged.levels.get(mission)
                        {
                            deferral.exclude_level_requirements(
                                level,
                                profiles,
                                &mut pending_chunks,
                                &mut work.decode_total,
                            );
                        }
                        deferral.absorb(&mut incoming, &mut pending_chunks, &mut work.decode_total);
                    } else {
                        for chunk in &incoming {
                            work.decode_total += chunk.blob.len() as u64;
                        }
                        pending_chunks.append(&mut incoming);
                    }
                    let rhs_files = &merged.rhs_files;
                    if pooled {
                        pending_rle.append(&mut bank.rle_jxl_chunks);
                        rle_scheduler.dispatch_ready(bank, &mut pending_rle)?;
                        // Lenient readiness: a missing row/base only means
                        // its part has not arrived yet.
                        scheduler.dispatch_ready(bank, &mut pending_chunks, rhs_files, false)?;
                    } else {
                        // Serial fallback: no pool, but decode still overlaps
                        // the network by draining ready chunks between
                        // arrivals, yielding so the loading screen stays
                        // responsive between chunks.
                        let before: u64 = pending_chunks.iter().map(|c| c.blob.len() as u64).sum();
                        while bank
                            .materialize_next_ready_vq_chunk(&mut pending_chunks, rhs_files)?
                        {
                            crate::window::yield_to_runtime().await;
                        }
                        let after: u64 = pending_chunks.iter().map(|c| c.blob.len() as u64).sum();
                        work.decode_done += before - after;
                    }
                }
                work.emit(progress, &label);
                // Present the observer's progress frame.
                crate::window::yield_to_runtime().await;
            }
            Event::Decoded(item) => {
                let Some((chunk, grids)) = item? else {
                    continue;
                };
                let bank = merged
                    .sprite_bank
                    .as_mut()
                    .ok_or_else(|| anyhow!("decoded VQ chunk without a sprite bank"))?;
                bank.apply_decoded_vq_chunk(&chunk, grids)?;
                work.decode_done += chunk.blob.len() as u64;
                let rhs_files = &merged.rhs_files;
                scheduler.dispatch_ready(bank, &mut pending_chunks, rhs_files, false)?;
                work.emit(progress, &chunk.rhs);
            }
            Event::Tick => {
                // Real byte progress accrued inside the concurrent body
                // reads; surface it even while no part has completed.
                work.emit(progress, &label);
                crate::window::yield_to_runtime().await;
            }
        }
    }
    if let Some(bank) = merged.sprite_bank.as_mut() {
        // The level part has merged by now on any well-formed payload
        // (installation fails later otherwise); make sure its start-entity
        // requirements were applied before the final partition is fixed.
        if let Some(deferral) = deferral.as_mut()
            && !deferral.level_filtered
            && let Some(level) = merged.levels.get(mission)
        {
            deferral.exclude_level_requirements(
                level,
                profiles,
                &mut pending_chunks,
                &mut work.decode_total,
            );
        }
        let rhs_files = &merged.rhs_files;
        // Drain outstanding worker decodes; each applied chunk can unlock
        // dependents that were still pending.
        while let Some((chunk, grids)) = scheduler.next_decoded().await? {
            bank.apply_decoded_vq_chunk(&chunk, grids)?;
            work.decode_done += chunk.blob.len() as u64;
            scheduler.dispatch_ready(bank, &mut pending_chunks, rhs_files, false)?;
            work.emit(progress, &chunk.rhs);
        }
        // Strict pass for the critical remainder: with the whole payload
        // merged, "not fetched yet" is no longer an excuse, so unresolved
        // dependencies now surface as real manifest errors.
        if pooled {
            loop {
                scheduler.dispatch_ready(bank, &mut pending_chunks, rhs_files, true)?;
                let Some((chunk, grids)) = scheduler.next_decoded().await? else {
                    break;
                };
                bank.apply_decoded_vq_chunk(&chunk, grids)?;
                work.decode_done += chunk.blob.len() as u64;
                work.emit(progress, &chunk.rhs);
            }
            if !pending_chunks.is_empty() {
                let stuck: Vec<&str> = pending_chunks
                    .iter()
                    .map(|chunk| chunk.rhs.as_str())
                    .collect();
                return Err(anyhow!(
                    "critical VQ sprite chunks cannot be decoded — base sprites never \
                     materialized: {}",
                    stuck.join(", ")
                ));
            }
        } else {
            bank.vq_chunks.append(&mut pending_chunks);
            bank.materialize_vq_chunks_parallel(rhs_files)
                .await
                .with_context(|| format!("materialize VQ sprite chunks for mission {mission}"))?;
        }
        // Apply the worker-pool RLE-JXL decodes that ran alongside the
        // fetches; anything still pending falls to the strict serial pass.
        while let Some((chunk, packed)) = rle_scheduler.next_decoded().await? {
            bank.apply_decoded_rle_jxl_chunk(&chunk, packed)?;
            rle_scheduler.dispatch_ready(bank, &mut pending_rle)?;
        }
        bank.rle_jxl_chunks.append(&mut pending_rle);
        bank.materialize_rle_jxl_chunks()
            .with_context(|| format!("materialize RLE-JXL sprite chunks for mission {mission}"))?;
    }
    progress(MissionLoadProgress {
        phase: MissionLoadPhase::Data,
        completed: InstallWorkModel::UNITS,
        total: InstallWorkModel::UNITS,
        file: None,
    });
    // Package the deferred tail (post-activation background streaming).
    let tail = match (deferral, merged.sprite_bank.as_ref()) {
        (Some(deferral), Some(bank)) if !deferral.parked.is_empty() => {
            let chunks = deferral.parked;
            let rhs_files = chunks
                .iter()
                .filter(|chunk| chunk.self_refs)
                .filter_map(|chunk| merged.rhs_files.get_key_value(&chunk.rhs))
                .map(|(path, rhs)| (path.clone(), rhs.clone()))
                .collect();
            Some(DeferredSpriteTail {
                bank: ShippingSpriteBank {
                    signature: bank.signature,
                    dictionaries: Vec::new(),
                    sprite_count: bank.sprite_count,
                    sprites: bank.sprites.clone(),
                    vq_chunks: Vec::new(),
                    // The tail only carries deferred VQ work; RLE-JXL
                    // chunks are always materialized before activation.
                    rle_jxl_chunks: Vec::new(),
                },
                rhs_files,
                chunks,
            })
        }
        _ => None,
    };
    Ok((merged, fetched_bytes, tail))
}

/// Stream the deferred (reinforcement-only) sprite chunks on the worker
/// pool while the mission is already running. Decoded grids are published
/// through [`robin_assets::late_sprites`], where every live `FrameHolder`
/// row already points; a draw call that races a still-pending grid skips
/// safely and the sprite pops in a frame later.
///
/// Post-activation failures must never take the mission down: decode
/// errors, stuck dependencies, and a superseding mission install all
/// degrade to a warn/debug log plus permanently-skipped sprites.
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
fn spawn_deferred_sprite_tail(mission: String, epoch: u64, tail: DeferredSpriteTail) {
    use robin_assets::late_sprites;
    use robin_assets::shipping_datadir::VqDecodeScheduler;

    let DeferredSpriteTail {
        bank,
        rhs_files,
        chunks,
    } = tail;
    let total_chunks = chunks.len();
    let total_blob: u64 = chunks.iter().map(|chunk| chunk.blob.len() as u64).sum();
    late_sprites::set_tail_work(epoch, total_chunks, total_blob);
    tracing::info!(
        mission,
        chunks = total_chunks,
        blob_bytes = total_blob,
        "mission activated with deferred sprite chunks; streaming in background"
    );
    wasm_bindgen_futures::spawn_local(async move {
        let started = js_sys::Date::now();
        let mut bank = bank;
        let mut pending = chunks;
        let mut scheduler = VqDecodeScheduler::default();
        let mut decoded_chunks = 0usize;
        let mut decoded_sprites = 0usize;
        loop {
            // Strict readiness: the full mission payload is merged, so a
            // missing base row is a manifest error, not "not yet".
            if let Err(error) = scheduler.dispatch_ready(&bank, &mut pending, &rhs_files, true) {
                tracing::warn!(
                    mission,
                    "background sprite streaming aborted (affected sprites stay skipped): \
                     {error:#}"
                );
                late_sprites::fail_tail(epoch);
                return;
            }
            match scheduler.next_decoded().await {
                Err(error) => {
                    // One chunk lost; its sprites keep safe-skipping. Other
                    // in-flight decodes are still worth draining.
                    tracing::warn!(mission, "background sprite chunk decode failed: {error:#}");
                }
                Ok(None) => break,
                Ok(Some((chunk, grids))) => {
                    let grids: Vec<(u32, Arc<Vec<u16>>)> = grids
                        .into_iter()
                        .map(|(sprite_id, grid)| (sprite_id, Arc::new(grid)))
                        .collect();
                    if !late_sprites::publish_chunk(epoch, chunk.blob.len() as u64, &grids) {
                        tracing::debug!(
                            mission,
                            "another mission install superseded the sprite streaming tail"
                        );
                        return;
                    }
                    decoded_chunks += 1;
                    decoded_sprites += grids.len();
                    // Mirror the grids into the private row clone so
                    // dependent deferred chunks see their base grids.
                    for (sprite_id, grid) in grids {
                        if let Ok(position) =
                            bank.sprites.binary_search_by_key(&sprite_id, |(id, _)| *id)
                        {
                            let sprite = &mut bank.sprites[position].1;
                            if sprite.packed_data.is_empty() {
                                sprite.packed_data = grid;
                            }
                        }
                    }
                    tracing::debug!(
                        mission,
                        chunk = %chunk.rhs,
                        "background sprite chunk streamed in"
                    );
                }
            }
            // Keep the main thread breathing between applies.
            crate::window::yield_to_runtime().await;
        }
        if !pending.is_empty() {
            let stuck: Vec<&str> = pending.iter().map(|chunk| chunk.rhs.as_str()).collect();
            tracing::warn!(
                mission,
                "background sprite streaming stuck — base sprites never materialized \
                 (affected sprites stay skipped): {}",
                stuck.join(", ")
            );
            late_sprites::fail_tail(epoch);
            return;
        }
        tracing::info!(
            mission,
            chunks = decoded_chunks,
            sprites = decoded_sprites,
            skipped_draws = late_sprites::skipped_draws(),
            elapsed_ms = js_sys::Date::now() - started,
            "background sprite streaming complete"
        );
    });
}

struct RequiredMissionDependencies {
    files: Vec<String>,
    exclamation_ids: BTreeSet<u32>,
}

fn required_dependencies(
    datadir: &ShippingDatadir,
    mission: &str,
    campaign: &robin_engine::campaign::Campaign,
    profiles: &robin_engine::profiles::ProfileManager,
    has_decoded_saved_world: bool,
) -> Result<RequiredMissionDependencies> {
    let reference = datadir
        .mission_ref(mission)
        .ok_or_else(|| anyhow!("shipping datadir does not contain mission {mission}"))?;
    let mut files: BTreeSet<String> = reference.files.iter().cloned().collect();
    let mut exclamation_ids: BTreeSet<u32> = datadir
        .mission_exclamation_ids
        .get(mission)
        .ok_or_else(|| {
            anyhow!("shipping manifest has no authored exclamation index for mission {mission}")
        })?
        .iter()
        .copied()
        .collect();
    let mut character_profiles = BTreeSet::new();

    for &character_index in &campaign.mission_team_indices {
        let description = campaign.characters.get(character_index).ok_or_else(|| {
            anyhow!(
                "mission team references missing campaign character {character_index} while loading {mission}"
            )
        })?;
        let profile = description.character_profile_idx.ok_or_else(|| {
            anyhow!(
                "mission-team character {character_index} has no profile while loading {mission}"
            )
        })?;
        character_profiles.insert(normalize_robin_profile(
            profiles,
            profile.0,
            reference.forest_level,
        )?);
    }

    // Reinforcement selection can instantiate any uninstanced, non-VIP gang
    // member during a simulation tick. Include exactly that candidate pool at
    // the asynchronous boundary; the tick itself must remain cache-only.
    for &character_index in &campaign.gang_indices {
        let description = campaign.characters.get(character_index).ok_or_else(|| {
            anyhow!(
                "gang references missing campaign character {character_index} while loading {mission}"
            )
        })?;
        if description.instanced {
            continue;
        }
        let profile_index = description.character_profile_idx.ok_or_else(|| {
            anyhow!("gang character {character_index} has no profile while loading {mission}")
        })?;
        let profile = profiles.get_character(profile_index).ok_or_else(|| {
            anyhow!(
                "gang character {character_index} references missing profile {} while loading {mission}",
                profile_index.0
            )
        })?;
        if !profile.vip {
            character_profiles.insert(normalize_robin_profile(
                profiles,
                profile_index.0,
                reference.forest_level,
            )?);
        }
    }

    for profile_index in character_profiles {
        let dependencies = datadir.character_rhs_files.get(&profile_index).ok_or_else(|| {
            anyhow!(
                "shipping manifest has no RHS dependency index for required character profile {profile_index}"
            )
        })?;
        files.extend(dependencies.iter().cloned());
        let audio_dependencies = datadir
            .character_audio_files
            .get(&profile_index)
            .ok_or_else(|| {
                anyhow!(
                    "shipping manifest has no audio dependency index for required character profile {profile_index}"
                )
            })?;
        files.extend(audio_dependencies.iter().cloned());
        if let Some(&exclamation_id) = datadir.character_exclamation_ids.get(&profile_index) {
            exclamation_ids.insert(exclamation_id);
        }
    }
    if has_decoded_saved_world {
        if datadir.saved_world_rhs_files.is_empty() {
            return Err(anyhow!(
                "shipping manifest has no conservative saved-world RHS dependency set"
            ));
        }
        files.extend(datadir.saved_world_rhs_files.iter().cloned());
    }
    Ok(RequiredMissionDependencies {
        files: files.into_iter().collect(),
        exclamation_ids,
    })
}

/// Mirror `RHelementactorpc.cpp`: Robin's stored campaign profile may be
/// either physical variant, but level construction always selects RobinHood
/// for forests and RobinTown for towns.
fn normalize_robin_profile(
    profiles: &robin_engine::profiles::ProfileManager,
    profile_index: u32,
    forest_level: bool,
) -> Result<u32> {
    let profile = profiles
        .characters
        .get(profile_index as usize)
        .ok_or_else(|| anyhow!("required character profile {profile_index} does not exist"))?;
    if !matches!(profile.filename.as_str(), "RobinHood" | "RobinTown") {
        return Ok(profile_index);
    }
    let wanted = if forest_level {
        "RobinHood"
    } else {
        "RobinTown"
    };
    profiles
        .characters
        .iter()
        .position(|candidate| candidate.filename == wanted)
        .map(|index| index as u32)
        .ok_or_else(|| {
            anyhow!(
                "required {wanted} profile is absent while normalizing Robin for a {} mission",
                if forest_level { "forest" } else { "town" }
            )
        })
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

/// Like [`fetch`], but with live byte accounting for the install-progress
/// model: the part's size registers from `Content-Length` the moment the
/// response headers arrive (every request is issued simultaneously, so all
/// sizes are known within the first round-trips), and body bytes count into
/// `progress.received` as each network chunk lands — the body is read
/// through a `ReadableStream` reader instead of one opaque `arrayBuffer()`
/// await, so the loading bar reflects real received bytes, not completed
/// files.
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
async fn fetch_counted(
    datadir: &ShippingDatadir,
    relative: &str,
    progress: &FetchByteProgress,
) -> Result<CompressedPayload> {
    use std::sync::atomic::Ordering;
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen_futures::JsFuture;

    let key = canonical_relative_file_key(relative)?;
    if let Some(bytes) = datadir.preloaded_file(&key) {
        let len = bytes.len() as u64;
        progress.add_known(len);
        progress.received.fetch_add(len, Ordering::Relaxed);
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
    let claimed: Option<u64> = response
        .headers()
        .get("content-length")
        .ok()
        .flatten()
        .and_then(|value| value.parse().ok());
    if let Some(claimed) = claimed {
        progress.add_known(claimed);
    }
    let mut bytes: Vec<u8> = Vec::with_capacity(claimed.unwrap_or(0) as usize);
    match response.body() {
        Some(stream) => {
            let reader: web_sys::ReadableStreamDefaultReader = stream
                .get_reader()
                .dyn_into()
                .map_err(|_| anyhow!("fetch {url}: body reader is not a default reader"))?;
            loop {
                let result = JsFuture::from(reader.read())
                    .await
                    .map_err(|error| anyhow!("fetch {url}: read body: {error:?}"))?;
                let done = js_sys::Reflect::get(&result, &"done".into())
                    .map_err(|error| anyhow!("fetch {url}: read result: {error:?}"))?
                    .as_bool()
                    .unwrap_or(false);
                if done {
                    break;
                }
                let value = js_sys::Reflect::get(&result, &"value".into())
                    .map_err(|error| anyhow!("fetch {url}: read result: {error:?}"))?;
                let chunk = js_sys::Uint8Array::new(&value);
                let offset = bytes.len();
                bytes.resize(offset + chunk.length() as usize, 0);
                chunk.copy_to(&mut bytes[offset..]);
                progress
                    .received
                    .fetch_add(u64::from(chunk.length()), Ordering::Relaxed);
            }
        }
        None => {
            // No streamable body (unusual); fall back to one opaque read.
            let buffer = response
                .array_buffer()
                .map_err(|error| anyhow!("fetch {url}: arrayBuffer: {error:?}"))?;
            let buffer = JsFuture::from(buffer)
                .await
                .map_err(|error| anyhow!("fetch {url}: read body: {error:?}"))?;
            bytes = js_sys::Uint8Array::new(&buffer).to_vec();
            progress
                .received
                .fetch_add(bytes.len() as u64, Ordering::Relaxed);
        }
    }
    // Reconcile the totals with the actual decoded body length: a missing
    // header registers late; a compressed transfer's header claims the
    // encoded size while the reader yields decoded bytes.
    match claimed {
        None => progress.add_known(bytes.len() as u64),
        Some(claimed) => {
            let actual = bytes.len() as u64;
            if actual >= claimed {
                progress
                    .known_total
                    .fetch_add(actual - claimed, Ordering::Relaxed);
            } else {
                progress
                    .known_total
                    .fetch_sub(claimed - actual, Ordering::Relaxed);
            }
        }
    }
    Ok(CompressedPayload::Owned(bytes))
}

#[cfg(test)]
mod tests {
    use super::{SpriteDeferral, preload_compressed, required_dependencies};
    use robin_assets::shipping_datadir::{
        ShippingDatadir, ShippingMission, ShippingMissionRef, SpriteVqChunk, encode_mission_native,
        zstd_max_compress,
    };
    use robin_engine::campaign::{Campaign, PcDescription};
    use robin_engine::profiles::{CharacterProfile, CharacterProfileIdx, ProfileManager};

    fn description(profile: u32, instanced: bool) -> PcDescription {
        PcDescription {
            character_profile_idx: Some(CharacterProfileIdx(profile)),
            instanced,
            ..PcDescription::default()
        }
    }

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

    // ── Critical-set partition (`SpriteDeferral`) ────────────────────

    fn chunk(rhs: &str, base: Option<&str>, base2: &str, blob_len: usize) -> SpriteVqChunk {
        SpriteVqChunk {
            rhs: rhs.to_owned(),
            base_rhs: base.map(str::to_owned),
            base2_rhs: base2.to_owned(),
            alphabet: 16,
            sprite_ids: Vec::new(),
            base_ids: Vec::new(),
            base2_ids: Vec::new(),
            self_refs: false,
            blob: vec![0; blob_len],
        }
    }

    fn named(filename: &str, vip: bool) -> CharacterProfile {
        CharacterProfile {
            filename: filename.to_owned(),
            vip,
            ..CharacterProfile::default()
        }
    }

    /// Team hero (0), reinforcement-eligible Merry Men (1, 2), a VIP gang
    /// hero (3), and an already-instanced Merry Man (4).
    fn deferral_fixture() -> (ShippingDatadir, Campaign, ProfileManager) {
        let mut datadir = ShippingDatadir::default();
        datadir.missions.insert(
            "H01".into(),
            ShippingMissionRef {
                forest_level: false,
                files: vec!["missions/h01".into()],
            },
        );
        datadir.missions.insert(
            "SherwoodOutro".into(),
            ShippingMissionRef {
                forest_level: true,
                files: vec!["missions/sherwood-outro".into()],
            },
        );
        let mut profiles = ProfileManager::new();
        profiles.characters = vec![
            named("RobinTown", true),
            named("MerryManA", false),
            named("MerryManB", false),
            named("LittleJohn", true),
            named("MerryManC", false),
        ];
        let campaign = Campaign {
            characters: vec![
                description(0, false),
                description(1, false),
                description(2, false),
                description(3, false),
                description(4, true),
            ],
            mission_team_indices: vec![0],
            gang_indices: vec![1, 2, 3, 4],
            ..Default::default()
        };
        (datadir, campaign, profiles)
    }

    fn deferral(mission: &str) -> SpriteDeferral {
        let (datadir, campaign, profiles) = deferral_fixture();
        SpriteDeferral::new(&datadir, mission, &campaign, &profiles).expect("build deferral")
    }

    #[test]
    fn only_reinforcement_eligible_gang_characters_are_deferrable() {
        let mut deferral = deferral("H01");
        // Uninstanced non-VIP gang members, and only those.
        assert_eq!(
            deferral.candidates,
            ["Characters/MerryManA.rhs", "Characters/MerryManB.rhs"]
                .map(str::to_owned)
                .into()
        );

        let mut incoming = vec![
            chunk("Characters/MerryManA.rhs", None, "", 100),
            chunk("Characters/RobinTown.rhs", None, "", 200),
            chunk("Characters/LittleJohn.rhs", None, "", 300),
            chunk("Animations/Day/Cart.rhs", None, "", 400),
        ];
        let mut pending = Vec::new();
        let mut decode_total = 0u64;
        deferral.absorb(&mut incoming, &mut pending, &mut decode_total);

        let critical: Vec<&str> = pending.iter().map(|c| c.rhs.as_str()).collect();
        assert_eq!(
            critical,
            [
                "Characters/RobinTown.rhs",
                "Characters/LittleJohn.rhs",
                "Animations/Day/Cart.rhs"
            ]
        );
        assert_eq!(decode_total, 200 + 300 + 400);
        let parked: Vec<&str> = deferral.parked.iter().map(|c| c.rhs.as_str()).collect();
        assert_eq!(parked, ["Characters/MerryManA.rhs"]);
    }

    #[test]
    fn coding_bases_of_critical_chunks_are_promoted_transitively() {
        let mut deferral = deferral("H01");
        // A critical chunk codes against MerryManB, which itself codes
        // against MerryManA: both hubs must decode before activation.
        let mut incoming = vec![
            chunk("Characters/MerryManA.rhs", None, "", 10),
            chunk(
                "Characters/MerryManB.rhs",
                Some("Characters/MerryManA.rhs"),
                "",
                20,
            ),
            chunk(
                "Characters/RobinTown.rhs",
                Some("Characters/MerryManB.rhs"),
                "",
                30,
            ),
        ];
        let mut pending = Vec::new();
        let mut decode_total = 0u64;
        deferral.absorb(&mut incoming, &mut pending, &mut decode_total);

        assert!(deferral.parked.is_empty(), "every hub must be promoted");
        assert!(deferral.candidates.is_empty());
        assert_eq!(decode_total, 10 + 20 + 30);
    }

    #[test]
    fn second_predecessor_hubs_are_promoted_too() {
        let mut deferral = deferral("H01");
        let mut incoming = vec![
            chunk("Characters/MerryManB.rhs", None, "", 20),
            chunk(
                "Characters/RobinTown.rhs",
                Some("Characters/LittleJohn.rhs"),
                "Characters/MerryManB.rhs",
                30,
            ),
        ];
        let mut pending = Vec::new();
        let mut decode_total = 0u64;
        deferral.absorb(&mut incoming, &mut pending, &mut decode_total);

        assert!(deferral.parked.is_empty(), "base2 hub must be promoted");
        assert_eq!(decode_total, 20 + 30);
    }

    /// A deferrable candidate that the level actually spawns at mission
    /// start stops being deferrable — the safety property that keeps
    /// start-visible sprites out of the streaming tail.
    #[test]
    fn level_start_entities_pull_their_chunks_back_into_the_critical_set() {
        let mut deferral = deferral("H01");
        // MerryManB is deferrable but codes against MerryManA, so making
        // MerryManA critical must also promote nothing extra; making a
        // start-spawned character critical must pull its own chunk back.
        let mut incoming = vec![
            chunk("Characters/MerryManA.rhs", None, "", 10),
            chunk("Characters/MerryManB.rhs", None, "", 20),
        ];
        let mut pending = Vec::new();
        let mut decode_total = 0u64;
        deferral.absorb(&mut incoming, &mut pending, &mut decode_total);
        assert_eq!(deferral.parked.len(), 2, "both start out deferrable");
        assert_eq!(decode_total, 0);

        // The level spawns MerryManA at mission start.
        deferral.exclude_names(
            &["Characters/MerryManA.rhs".to_owned()].into(),
            &mut pending,
            &mut decode_total,
        );

        let critical: Vec<&str> = pending.iter().map(|c| c.rhs.as_str()).collect();
        assert_eq!(critical, ["Characters/MerryManA.rhs"]);
        assert_eq!(decode_total, 10);
        let parked: Vec<&str> = deferral.parked.iter().map(|c| c.rhs.as_str()).collect();
        assert_eq!(parked, ["Characters/MerryManB.rhs"]);
    }

    /// A start-spawned character that is itself a coding base drags its
    /// dependent chunk's hub chain along.
    #[test]
    fn excluding_a_name_promotes_its_dependent_hubs() {
        let mut deferral = deferral("H01");
        let mut incoming = vec![
            chunk("Characters/MerryManA.rhs", None, "", 10),
            chunk(
                "Characters/MerryManB.rhs",
                Some("Characters/MerryManA.rhs"),
                "",
                20,
            ),
        ];
        let mut pending = Vec::new();
        let mut decode_total = 0u64;
        deferral.absorb(&mut incoming, &mut pending, &mut decode_total);
        assert_eq!(deferral.parked.len(), 2);

        // The level spawns MerryManB; its base hub MerryManA must follow.
        deferral.exclude_names(
            &["Characters/MerryManB.rhs".to_owned()].into(),
            &mut pending,
            &mut decode_total,
        );
        assert!(deferral.parked.is_empty());
        assert_eq!(decode_total, 30);
    }

    /// Sherwood is populated from the uninstanced gang itself, so nothing
    /// there may be deferred.
    #[test]
    fn sherwood_defers_nothing() {
        let (datadir, campaign, profiles) = deferral_fixture();
        let deferral = SpriteDeferral::new(&datadir, "SherwoodOutro", &campaign, &profiles)
            .expect("build deferral");
        assert!(deferral.candidates.is_empty());
    }

    #[test]
    fn required_files_adds_team_and_eligible_reinforcement_profiles() {
        let mut datadir = ShippingDatadir::default();
        datadir.missions.insert(
            "H01".into(),
            ShippingMissionRef {
                forest_level: false,
                files: vec!["missions/h01".into(), "rhs/static".into()],
            },
        );
        datadir
            .mission_exclamation_ids
            .insert("H01".into(), vec![91]);
        datadir
            .character_rhs_files
            .insert(0, vec!["rhs/team".into(), "rhs/shared".into()]);
        datadir
            .character_rhs_files
            .insert(2, vec!["rhs/reinforcement".into(), "rhs/shared".into()]);
        datadir
            .character_audio_files
            .insert(0, vec!["audio/team-voice".into()]);
        datadir
            .character_audio_files
            .insert(2, vec!["audio/reinforcement-voice".into()]);
        datadir.character_exclamation_ids.insert(0, 100);
        datadir.character_exclamation_ids.insert(2, 102);

        let mut profiles = ProfileManager::new();
        profiles.characters = vec![
            CharacterProfile::default(),
            CharacterProfile {
                vip: true,
                ..CharacterProfile::default()
            },
            CharacterProfile::default(),
            CharacterProfile::default(),
        ];
        let campaign = Campaign {
            characters: vec![
                description(0, false),
                description(1, false),
                description(2, false),
                description(3, true),
            ],
            mission_team_indices: vec![0],
            gang_indices: vec![1, 2, 3],
            ..Default::default()
        };

        let dependencies =
            required_dependencies(&datadir, "H01", &campaign, &profiles, false).unwrap();
        assert_eq!(
            dependencies.files,
            vec![
                "audio/reinforcement-voice",
                "audio/team-voice",
                "missions/h01",
                "rhs/reinforcement",
                "rhs/shared",
                "rhs/static",
                "rhs/team",
            ]
        );
        assert_eq!(dependencies.exclamation_ids, [91, 100, 102].into());
    }

    #[test]
    fn required_files_adds_explicit_saved_world_closure() {
        let mut datadir = ShippingDatadir::default();
        datadir.missions.insert(
            "H01".into(),
            ShippingMissionRef {
                forest_level: false,
                files: vec!["missions/h01".into()],
            },
        );
        datadir
            .mission_exclamation_ids
            .insert("H01".into(), Vec::new());
        datadir.saved_world_rhs_files = vec!["rhs/all-saved-objects".into()];
        let dependencies = required_dependencies(
            &datadir,
            "H01",
            &Campaign::default(),
            &ProfileManager::new(),
            true,
        )
        .unwrap();
        assert_eq!(
            dependencies.files,
            vec!["missions/h01", "rhs/all-saved-objects"]
        );
    }

    #[test]
    fn required_files_rejects_missing_character_index_entry() {
        let mut datadir = ShippingDatadir::default();
        datadir.missions.insert(
            "H01".into(),
            ShippingMissionRef {
                forest_level: false,
                files: vec!["missions/h01".into()],
            },
        );
        datadir
            .mission_exclamation_ids
            .insert("H01".into(), Vec::new());
        let mut profiles = ProfileManager::new();
        profiles.characters.push(CharacterProfile::default());
        let mut campaign = Campaign::default();
        campaign.characters.push(description(0, false));
        campaign.mission_team_indices.push(0);
        let error = required_dependencies(&datadir, "H01", &campaign, &profiles, false)
            .err()
            .expect("missing profile dependency must fail");
        assert!(error.to_string().contains("profile 0"));
    }

    #[test]
    fn required_files_selects_only_the_mission_robin_variant() {
        let mut datadir = ShippingDatadir::default();
        datadir.missions.insert(
            "Forest".into(),
            ShippingMissionRef {
                forest_level: true,
                files: vec!["missions/forest".into()],
            },
        );
        datadir.missions.insert(
            "Town".into(),
            ShippingMissionRef {
                forest_level: false,
                files: vec!["missions/town".into()],
            },
        );
        datadir
            .mission_exclamation_ids
            .insert("Forest".into(), Vec::new());
        datadir
            .mission_exclamation_ids
            .insert("Town".into(), Vec::new());
        datadir
            .character_rhs_files
            .insert(0, vec!["rhs/robin-hood".into()]);
        datadir
            .character_rhs_files
            .insert(1, vec!["rhs/robin-town".into()]);
        datadir.character_audio_files.insert(0, Vec::new());
        datadir.character_audio_files.insert(1, Vec::new());

        let mut profiles = ProfileManager::new();
        profiles.characters = vec![
            CharacterProfile {
                filename: "RobinHood".into(),
                ..CharacterProfile::default()
            },
            CharacterProfile {
                filename: "RobinTown".into(),
                ..CharacterProfile::default()
            },
        ];
        let mut campaign = Campaign::default();
        campaign.characters.push(description(0, false));
        campaign.mission_team_indices.push(0);

        let forest = required_dependencies(&datadir, "Forest", &campaign, &profiles, false)
            .unwrap()
            .files;
        let town = required_dependencies(&datadir, "Town", &campaign, &profiles, false)
            .unwrap()
            .files;
        assert_eq!(
            forest,
            vec!["missions/forest".to_owned(), "rhs/robin-hood".to_owned()]
        );
        assert_eq!(
            town,
            vec!["missions/town".to_owned(), "rhs/robin-town".to_owned()]
        );
    }
}
