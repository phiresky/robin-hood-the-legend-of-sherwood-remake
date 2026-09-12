//! Application-owned asset cache warmed on a background thread.
//!
//! The original game paid for the sprite bank, sound banks, and the
//! exclamation cache once at application startup, so its loading screen
//! only covered per-level data. The port rebuilds `Host` per mission,
//! which used to re-read and re-parse all of that on every mission
//! load. This cache holds the parsed, GPU-/audio-free products of that
//! work for the lifetime of one application. Independent applications have
//! independent entries, warmup workers, invalidation epochs, and locks.
//!
//! `start_background_warmup` builds it on a plain thread while the main
//! menu runs, so the menu is visually unaffected; mission load consumes
//! it via `get_or_build`, which waits for a running warm-up or builds
//! synchronously when none was started (wasm, tests).

use std::sync::{Arc, Condvar, Mutex};

use robin_assets::frame_holder::FrameHolder;
use robin_assets::resource_manager::ResourceManager;
use robin_assets::shipping_datadir as assets_shipping_datadir;
use robin_engine::profiles::ProfileManager;
use robin_engine::sbfile::SbFileSystem;
use robin_engine::sound_cache::FxBankElement;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct CacheKey {
    reader: u64,
    mounts: robin_engine::sbfile::SbFileMountSnapshot,
    #[serde(default)]
    presentation_locale: Option<String>,
    installation: u64,
    generation: u64,
    mission_generation: u64,
    content_generation: u64,
    exclamations: Vec<u32>,
    localized_epoch: u64,
}

impl CacheKey {
    fn capture(
        shipping: Option<&assets_shipping_datadir::ShippingDatadir>,
        localized_epoch: u64,
        files: &Arc<SbFileSystem>,
    ) -> Self {
        let selection = shipping
            .map(|dd| dd.selection_snapshot())
            .unwrap_or_else(|| files.selection_snapshot());
        let mounts = files.mount_snapshot();
        #[cfg(not(target_arch = "wasm32"))]
        let mounts = {
            let mut mounts = mounts;
            if mounts.working_directory.is_none() {
                mounts.working_directory = Some(
                    std::env::current_dir().expect("cannot capture asset-cache working directory"),
                );
            }
            mounts
        };
        Self {
            reader: files.origin_identity(),
            mounts,
            presentation_locale: files.presentation_locale(),
            installation: shipping.map_or(0, |dd| dd.installation_id()),
            generation: selection.generation,
            mission_generation: selection.mission_generation,
            content_generation: selection.content_generation,
            exclamations: shipping
                .map(|dd| dd.active_exclamation_ids())
                .unwrap_or_default(),
            localized_epoch,
        }
    }

    fn same_stable_assets(&self, other: &Self) -> bool {
        self.installation == other.installation
            && self.reader == other.reader
            && self.mounts.working_directory == other.mounts.working_directory
            && self.mounts.primary_path == other.mounts.primary_path
            && self.mounts.ranked_verifier_primary_path == other.mounts.ranked_verifier_primary_path
            && self.mounts.alternate_paths == other.mounts.alternate_paths
            && self.mounts.overlay_paths == other.mounts.overlay_paths
            && self.mounts.official_projection_strict == other.mounts.official_projection_strict
            && self.mission_generation == other.mission_generation
            && self.content_generation == other.content_generation
    }
}

/// Banks are shared across locale-only rebuilds. Mission publication replaces
/// sprites, so stable here means installation + mission generation.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct StableAssetCache {
    pub sprite_bank: Option<FrameHolder>,
    pub fx_bank: Option<Vec<FxBankElement>>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ProcessAssetCache {
    /// Retain the actual read authority used to build this cache entry.
    #[serde(skip)]
    _files: Option<Arc<SbFileSystem>>,
    key: CacheKey,
    stable: Arc<StableAssetCache>,
    pub menu_bank: Option<Vec<(u32, String)>>,
    pub exclamations: Vec<Vec<(u32, Vec<String>)>>,
}

impl std::ops::Deref for ProcessAssetCache {
    type Target = StableAssetCache;
    fn deref(&self) -> &Self::Target {
        &self.stable
    }
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct State {
    ready: Option<Arc<ProcessAssetCache>>,
    #[serde(skip)]
    loading: Option<Arc<LoadingJob>>,
    localized_epoch: u64,
}

impl State {
    /// Retain completed products for possible stable-bank reuse, but prevent
    /// this job from publishing after its key or locale epoch has changed.
    fn retire_loading(&mut self) {
        if let Some(job) = self.loading.take() {
            if let Some(cache) = job.completed() {
                self.ready = Some(cache);
            }
            job.cancel();
        }
    }
}

/// A job never retains its application owner. Cancellation wakes consumers
/// immediately, even when an underlying asset read cannot be interrupted.
#[derive(serde::Serialize, serde::Deserialize)]
struct LoadingJob {
    key: CacheKey,
    #[serde(skip)]
    result: Mutex<JobResult>,
    #[serde(skip)]
    changed: Condvar,
}

#[derive(Default)]
enum JobResult {
    #[default]
    Pending,
    Complete(Arc<ProcessAssetCache>),
    Cancelled,
    Failed,
}

impl LoadingJob {
    fn new(key: CacheKey) -> Arc<Self> {
        Arc::new(Self {
            key,
            result: Mutex::new(JobResult::Pending),
            changed: Condvar::new(),
        })
    }

    fn is_cancelled(&self) -> bool {
        matches!(
            *self.result.lock().expect("asset loading job lock poisoned"),
            JobResult::Cancelled
        )
    }

    fn completed(&self) -> Option<Arc<ProcessAssetCache>> {
        match &*self.result.lock().expect("asset loading job lock poisoned") {
            JobResult::Complete(cache) => Some(cache.clone()),
            _ => None,
        }
    }

    fn cancel(&self) {
        *self.result.lock().expect("asset loading job lock poisoned") = JobResult::Cancelled;
        self.changed.notify_all();
    }

    fn finish(&self, result: JobResult) {
        let mut current = self.result.lock().expect("asset loading job lock poisoned");
        if matches!(*current, JobResult::Pending) {
            *current = result;
        }
        self.changed.notify_all();
    }

    fn wait(&self) -> Option<Arc<ProcessAssetCache>> {
        let mut result = self.result.lock().expect("asset loading job lock poisoned");
        while matches!(*result, JobResult::Pending) {
            result = self
                .changed
                .wait(result)
                .expect("asset loading job lock poisoned");
        }
        match &*result {
            JobResult::Complete(cache) => Some(cache.clone()),
            JobResult::Cancelled | JobResult::Failed => None,
            JobResult::Pending => unreachable!("asset loading wait must finish"),
        }
    }

    fn run(&self, build: impl FnOnce() -> Option<Arc<ProcessAssetCache>>) {
        if self.is_cancelled() {
            return;
        }
        // Explicit completion is important even with Cranelift configurations
        // where catch_unwind works but destructor unwinding is incomplete.
        // No owner/job mutex is held while calling user/asset parsing code.
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(build)) {
            Ok(Some(cache)) => self.finish(JobResult::Complete(cache)),
            Ok(None) => self.finish(JobResult::Cancelled),
            Err(panic) => {
                self.finish(JobResult::Failed);
                tracing::warn!("asset loading worker panicked; a subsequent caller can retry");
                std::panic::resume_unwind(panic);
            }
        }
    }
}

/// Runtime cache ownership is never restored from serialized parsed products.
/// ApplicationServices skips this owner entirely when decoding a context.
#[derive(Default, serde::Serialize, serde::Deserialize)]
pub struct ApplicationAssetCache {
    #[serde(skip)]
    state: Mutex<State>,
    #[serde(skip)]
    early_terrain: Mutex<Option<crate::level_loading_host::EarlyTerrainDecode>>,
}

impl std::fmt::Debug for ApplicationAssetCache {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ApplicationAssetCache")
            .finish_non_exhaustive()
    }
}

impl ApplicationAssetCache {
    /// Publish only after installation succeeds; dropping a failed install's
    /// local job cannot leave an entry available to a later mission.
    pub fn publish_early_terrain(
        &self,
        mut job: crate::level_loading_host::EarlyTerrainDecode,
        datadir: &assets_shipping_datadir::ShippingDatadir,
    ) -> Result<(), String> {
        job.publish(datadir)?;
        *self
            .early_terrain
            .lock()
            .expect("early terrain lock poisoned") = Some(job);
        Ok(())
    }

    pub(crate) fn take_early_terrain(
        &self,
        datadir: &assets_shipping_datadir::ShippingDatadir,
        mission: &str,
        map: &str,
        ambiance: &str,
    ) -> Option<crate::level_loading_host::EarlyTerrainDecode> {
        // Taking even a mismatched entry retires it; it must never be reused
        // after an installation switch or superseding mission activation.
        // Locale/overlay changes are checked against the final reader bytes.
        self.early_terrain
            .lock()
            .expect("early terrain lock poisoned")
            .take()
            .filter(|job| job.matches(datadir, mission, map, ambiance))
    }

    /// Retain stable banks while invalidating locale-dependent parsed tables.
    pub fn invalidate_localized(&self) {
        let mut state = self
            .state
            .lock()
            .expect("application asset cache lock poisoned");
        state.localized_epoch = state
            .localized_epoch
            .checked_add(1)
            .expect("asset cache epoch exhausted");
        state.retire_loading();
    }

    pub fn start_background_warmup(
        &self,
        shipping: Option<Arc<assets_shipping_datadir::ShippingDatadir>>,
        profiles: Arc<ProfileManager>,
        files: Arc<SbFileSystem>,
    ) {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (shipping, profiles, files);
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut state = self
                .state
                .lock()
                .expect("application asset cache lock poisoned");
            if state.loading.is_some() || state.ready.is_some() {
                return;
            }
            let key = CacheKey::capture(shipping.as_deref(), state.localized_epoch, &files);
            let job = LoadingJob::new(key.clone());
            state.loading = Some(job.clone());
            drop(state);
            let worker = job.clone();
            match std::thread::Builder::new()
                .name("asset-warmup".into())
                .spawn(move || {
                    worker.run(|| {
                        build(shipping.as_deref(), &profiles, key, None, files, &worker)
                            .map(Arc::new)
                    })
                }) {
                Ok(_) => {} // Detached: shutdown cancels; it never joins an asset read.
                Err(error) => {
                    job.finish(JobResult::Failed);
                    tracing::warn!("asset warm-up thread failed to spawn: {error}");
                }
            }
        }
    }

    /// One caller owns a build. Other callers wait on its job, never on the
    /// owner's state lock; invalidation can cancel a blocked load immediately.
    pub fn get_or_build(
        &self,
        shipping: Option<&assets_shipping_datadir::ShippingDatadir>,
        profiles: &ProfileManager,
        files: Arc<SbFileSystem>,
    ) -> Arc<ProcessAssetCache> {
        self.resolve(
            |epoch| CacheKey::capture(shipping, epoch, &files),
            |key, stable, job| {
                build(shipping, profiles, key, stable, files.clone(), job).map(Arc::new)
            },
        )
    }

    fn resolve(
        &self,
        capture: impl Fn(u64) -> CacheKey,
        mut build_cache: impl FnMut(
            CacheKey,
            Option<Arc<StableAssetCache>>,
            &LoadingJob,
        ) -> Option<Arc<ProcessAssetCache>>,
    ) -> Arc<ProcessAssetCache> {
        loop {
            let mut state = self
                .state
                .lock()
                .expect("application asset cache lock poisoned");
            let key = capture(state.localized_epoch);
            if let Some(cache) = &state.ready
                && cache.key == key
            {
                return cache.clone();
            }
            if state.loading.as_ref().is_some_and(|job| job.key != key) {
                state.retire_loading();
            }
            let stable = state
                .ready
                .as_ref()
                .filter(|cache| cache.key.same_stable_assets(&key))
                .map(|cache| cache.stable.clone());
            let (job, build_here) = match &state.loading {
                Some(job) if job.key == key => (job.clone(), false),
                _ => {
                    let job = LoadingJob::new(key.clone());
                    state.loading = Some(job.clone());
                    (job, true)
                }
            };
            drop(state);
            if build_here {
                job.run(|| build_cache(key.clone(), stable, &job));
            }
            let result = job.wait();
            let mut state = self
                .state
                .lock()
                .expect("application asset cache lock poisoned");
            if !state
                .loading
                .as_ref()
                .is_some_and(|active| Arc::ptr_eq(active, &job))
            {
                continue;
            }
            state.loading = None;
            // A locale/mission can be published independently of this cache lock.
            // Never publish a result assembled across a changed generation.
            if capture(state.localized_epoch) == key
                && let Some(cache) = result
            {
                state.ready = Some(cache.clone());
                return cache;
            }
        }
    }
}

impl Drop for ApplicationAssetCache {
    fn drop(&mut self) {
        let state = self
            .state
            .get_mut()
            .expect("application asset cache lock poisoned");
        if let Some(job) = state.loading.take() {
            job.cancel();
        }
    }
}

fn build(
    shipping: Option<&assets_shipping_datadir::ShippingDatadir>,
    profiles: &ProfileManager,
    key: CacheKey,
    stable: Option<Arc<StableAssetCache>>,
    files: Arc<SbFileSystem>,
    job: &LoadingJob,
) -> Option<ProcessAssetCache> {
    if job.is_cancelled() {
        return None;
    }
    let stable = if let Some(stable) = stable {
        stable
    } else {
        let sprite_bank = {
            let mut holder = FrameHolder::new();
            match holder.initialize_sprite_bank_with_progress_and_files(
                ".",
                &mut |_| {},
                shipping,
                &files,
            ) {
                Ok(()) => Some(holder),
                Err(e) => {
                    tracing::warn!("Failed to load sprite bank: {e}");
                    None
                }
            }
        };

        // TODO: Make sprite decoding and individual file reads cancellable.
        // For now retirement stops work at expensive stage boundaries.
        if job.is_cancelled() {
            return None;
        }

        let fx_bank_path = "Data/Sounds/robin hood.fxg";
        let fx_bank = match files.read_shared(fx_bank_path) {
            Ok(data) => match robin_engine::sound_cache::parse_fx_bank(&data) {
                Ok(elements) => Some(elements),
                Err(e) => {
                    tracing::warn!("Failed to parse FX bank: {e}");
                    None
                }
            },
            Err(e) => {
                tracing::warn!("Failed to read FX bank '{fx_bank_path}': error {e}");
                None
            }
        };

        Arc::new(StableAssetCache {
            sprite_bank,
            fx_bank,
        })
    };

    if job.is_cancelled() {
        return None;
    }

    let menu_bank_path = "Data/Sounds/Menu/menu.fxg";
    let menu_bank = match files.read_shared(menu_bank_path) {
        Ok(data) => match robin_engine::sound_cache::parse_menu_bank(&data) {
            Ok(entries) => Some(entries),
            Err(e) => {
                tracing::warn!("Failed to parse menu sound bank: {e}");
                None
            }
        },
        Err(e) => {
            tracing::warn!("Failed to read menu sound bank '{menu_bank_path}': error {e}");
            None
        }
    };

    if job.is_cancelled() {
        return None;
    }
    let exclamations = build_exclamations(shipping, profiles, files.clone());
    if job.is_cancelled() {
        return None;
    }

    Some(ProcessAssetCache {
        _files: Some(files),
        key,
        stable,
        menu_bank,
        exclamations,
    })
}

/// Load actors.res for variant-index → WAV-filename resolution, then
/// parse each active profile's .dat file into resolved speech entry lists.
/// Split shipping datadirs publish an exact mission/team closure; loose
/// datadirs preserve the original eager all-profile behavior.
fn build_exclamations(
    shipping: Option<&assets_shipping_datadir::ShippingDatadir>,
    profiles: &ProfileManager,
    files: Arc<SbFileSystem>,
) -> Vec<Vec<(u32, Vec<String>)>> {
    let mut excl_res = ResourceManager::with_files(files.clone());
    if let Err(error) =
        excl_res.attach_or_from_shipping("Data/Sounds/Exclamations/actors.res", shipping)
    {
        if shipping.is_some() {
            panic!("shipping mission is missing authoritative actors.res: {error}");
        }
        tracing::warn!("Failed to load actors.res — exclamation cache not initialized: {error}");
        return Vec::new();
    }

    build_exclamations_from(
        profiles,
        shipping,
        &mut excl_res,
        |dat_filename| {
            let path = format!("Data/Sounds/Exclamations/{dat_filename}");
            files
                .read_shared(&path)
                .map_err(|status| format!("{path}: file error {status}"))
        },
        "active language",
        shipping.is_some(),
    )
    .unwrap_or_else(|error| panic!("authoritative shipping exclamation data is invalid: {error}"))
}

/// Resolve the speech metadata for a specific installed language without
/// changing the application's active locale. This supplies the canonical
/// language-independent timing table used by multiplayer and replay.
pub fn build_exclamations_for_language(
    pack: &crate::localization::LanguagePack,
    shipping: Option<&assets_shipping_datadir::ShippingDatadir>,
    profiles: &ProfileManager,
    files: Arc<SbFileSystem>,
) -> Result<Vec<Vec<(u32, Vec<String>)>>, String> {
    let source = format!("canonical locale {}", pack.locale);
    if pack.data_root.is_empty() {
        let shipping = shipping.ok_or_else(|| {
            format!(
                "shipping voice pack {} has no shipping datadir",
                pack.locale
            )
        })?;
        let mut resources =
            match shipping.locale_resource(&pack.locale, "Data/Sounds/Exclamations/actors.res") {
                Ok(Some(resources)) => resources.clone(),
                Ok(None) => return Err(format!("voice pack {} has no actors.res", pack.locale)),
                Err(error) => {
                    return Err(format!(
                        "voice pack {} actors.res lookup failed: {error:#}",
                        pack.locale
                    ));
                }
            };
        build_exclamations_from(
            profiles,
            Some(shipping),
            &mut resources,
            |dat_filename| {
                let path = format!("Data/Sounds/Exclamations/{dat_filename}");
                shipping
                    .locale_raw(&pack.locale, &path)
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| format!("{path}: missing from shipping locale {}", pack.locale))
            },
            &source,
            true,
        )
    } else {
        let mut resources = ResourceManager::with_files(files.clone());
        let path = format!("{}/Data/Sounds/Exclamations/actors.res", pack.data_root);
        resources.attach_resource_file(&path).map_err(|error| {
            format!(
                "voice pack {} actors.res failed to load: {error:#}",
                pack.locale
            )
        })?;
        build_exclamations_from(
            profiles,
            shipping,
            &mut resources,
            |dat_filename| {
                let path = format!("{}/Data/Sounds/Exclamations/{dat_filename}", pack.data_root);
                files
                    .read_shared(&path)
                    .map_err(|status| format!("{path}: file error {status}"))
            },
            &source,
            true,
        )
    }
}

fn build_exclamations_from<B: AsRef<[u8]>>(
    profiles: &ProfileManager,
    shipping: Option<&assets_shipping_datadir::ShippingDatadir>,
    excl_res: &mut ResourceManager,
    mut read_definition: impl FnMut(&str) -> Result<B, String>,
    source: &str,
    strict: bool,
) -> Result<Vec<Vec<(u32, Vec<String>)>>, String> {
    // Collect unique exclamation IDs from all profile types. The id's
    // non-zero LE bytes spell the actor file's name suffix.
    let mut files_needed = std::collections::BTreeSet::<u32>::new();
    let mut add = |excl_id: u32| {
        if excl_id != 0 {
            files_needed.insert(excl_id);
        }
    };
    if let Some(datadir) = shipping {
        for excl_id in datadir.active_exclamation_ids() {
            add(excl_id);
        }
    } else {
        for ch in &profiles.characters {
            add(ch.exclamation_id);
        }
        for s in &profiles.soldiers {
            add(s.exclamation_id);
        }
        for c in &profiles.civilians {
            add(c.exclamation_id);
        }
    }

    let mut result = Vec::new();
    let mut total_exclamations = 0usize;
    for &excl_id in &files_needed {
        let mut dat_filename = String::from("actor");
        dat_filename.extend(
            excl_id
                .to_le_bytes()
                .into_iter()
                .filter(|&b| b != 0)
                .map(char::from),
        );
        dat_filename.push_str(".dat");
        let data = match read_definition(&dat_filename) {
            Ok(d) => d,
            Err(e) => {
                if strict {
                    return Err(format!(
                        "failed to read {source} exclamation file '{dat_filename}': {e}"
                    ));
                }
                tracing::warn!("Failed to read {source} exclamation file '{dat_filename}': {e}");
                continue;
            }
        };

        let prefix_id = excl_id & 0xFFFF_0000;
        let (table_id, exclamations) =
            match robin_engine::sound_cache::parse_exclamation_file(data.as_ref(), prefix_id) {
                Ok(r) => r,
                Err(e) => {
                    if strict {
                        return Err(format!(
                            "failed to parse {source} exclamation file '{dat_filename}': {e}"
                        ));
                    }
                    tracing::warn!("Failed to parse exclamation file '{dat_filename}': {e}");
                    continue;
                }
            };

        // Resolve variant indices to WAV file paths via resource manager
        let mut resolved = Vec::with_capacity(exclamations.len());
        for (action_id, variant_indices) in exclamations {
            let mut paths = Vec::with_capacity(variant_indices.len());
            for variant_index in variant_indices {
                match excl_res.get_sample(table_id as i32, variant_index as usize) {
                    Ok(path) => paths.push(path.to_string()),
                    Err(error) if strict => {
                        return Err(format!(
                            "{source} actors.res cannot resolve table {table_id} variant {variant_index}: {error:#}"
                        ));
                    }
                    Err(error) => tracing::warn!(
                        "{source} actors.res cannot resolve table {table_id} variant {variant_index}: {error:#}"
                    ),
                }
            }
            resolved.push((action_id, paths));
        }

        total_exclamations += resolved.len();
        result.push(resolved);
    }

    tracing::info!(
        source,
        "Loaded exclamation cache: {} profiles, {} exclamations",
        files_needed.len(),
        total_exclamations,
    );
    Ok(result)
}

#[cfg(test)]
mod tests;
