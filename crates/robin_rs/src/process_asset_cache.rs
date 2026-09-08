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

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    fn key(generation: u64) -> CacheKey {
        CacheKey {
            reader: 0,
            mounts: SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()))
                .mount_snapshot(),
            installation: 7,
            generation,
            mission_generation: 0,
            content_generation: 0,
            exclamations: vec![],
            localized_epoch: 0,
        }
    }
    fn build_test(key: CacheKey, stable: Option<Arc<StableAssetCache>>) -> Arc<ProcessAssetCache> {
        Arc::new(ProcessAssetCache {
            _files: None,
            key,
            stable: stable.unwrap_or_else(|| {
                Arc::new(StableAssetCache {
                    sprite_bank: None,
                    fx_bank: None,
                })
            }),
            menu_bank: None,
            exclamations: vec![],
        })
    }

    fn resolve(
        owner: &ApplicationAssetCache,
        capture: impl Fn() -> CacheKey,
        mut build: impl FnMut(CacheKey, Option<Arc<StableAssetCache>>) -> Arc<ProcessAssetCache>,
    ) -> Arc<ProcessAssetCache> {
        owner.resolve(
            |epoch| CacheKey {
                localized_epoch: epoch,
                ..capture()
            },
            |key, stable, _| Some(build(key, stable)),
        )
    }

    #[test]
    fn application_owners_do_not_share_entries_or_invalidation() {
        let first = ApplicationAssetCache::default();
        let second = ApplicationAssetCache::default();
        let first_entry = resolve(&first, || key(1), build_test);
        let second_entry = resolve(&second, || key(1), build_test);
        assert!(!Arc::ptr_eq(&first_entry.stable, &second_entry.stable));
        first.invalidate_localized();
        let localized = resolve(&first, || key(1), build_test);
        assert_eq!(localized.key.localized_epoch, 1);
        assert!(Arc::ptr_eq(&first_entry.stable, &localized.stable));
        assert!(Arc::ptr_eq(
            &second_entry,
            &resolve(&second, || key(1), build_test)
        ));
    }

    #[test]
    fn locale_reuses_banks_but_mission_replaces_them() {
        let owner = ApplicationAssetCache::default();
        let first = resolve(&owner, || key(1), build_test);
        let localized = resolve(&owner, || key(2), build_test);
        assert!(Arc::ptr_eq(&first.stable, &localized.stable));
        let mission = resolve(
            &owner,
            || CacheKey {
                mission_generation: 3,
                ..key(3)
            },
            build_test,
        );
        assert!(!Arc::ptr_eq(&localized.stable, &mission.stable));
    }

    #[test]
    fn changed_generation_never_publishes_old_result() {
        let owner = ApplicationAssetCache::default();
        let generation = AtomicU64::new(1);
        let calls = AtomicUsize::new(0);
        let result = resolve(
            &owner,
            || key(generation.load(Ordering::SeqCst)),
            |key, stable| {
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    generation.store(2, Ordering::SeqCst);
                }
                build_test(key, stable)
            },
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(result.key.generation, 2);
        assert_eq!(
            owner
                .state
                .lock()
                .unwrap()
                .ready
                .as_ref()
                .unwrap()
                .key
                .generation,
            2
        );
    }

    #[test]
    fn concurrent_callers_share_one_result_without_holding_owner_lock() {
        let owner = Arc::new(ApplicationAssetCache::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(std::sync::Barrier::new(4));
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let owner = owner.clone();
                let calls = calls.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    resolve(
                        &owner,
                        || key(1),
                        |key, stable| {
                            // A builder can access owner state: it is not running under its lock.
                            assert_eq!(owner.state.lock().unwrap().localized_epoch, 0);
                            calls.fetch_add(1, Ordering::SeqCst);
                            build_test(key, stable)
                        },
                    )
                })
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(
            results
                .iter()
                .all(|result| Arc::ptr_eq(&results[0], result))
        );
    }

    #[test]
    fn invalidation_releases_waiters_and_rejects_blocked_worker_completion() {
        let owner = Arc::new(ApplicationAssetCache::default());
        let job = LoadingJob::new(key(1));
        owner.state.lock().unwrap().loading = Some(job.clone());
        let (release, blocked) = std::sync::mpsc::channel();
        let (started, running) = std::sync::mpsc::channel();
        let worker_job = job.clone();
        let worker = std::thread::spawn(move || {
            worker_job.run(|| {
                started.send(()).unwrap();
                blocked.recv().unwrap();
                Some(build_test(key(1), None))
            })
        });
        running
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let waiter_job = job.clone();
        let (done, result) = std::sync::mpsc::channel();
        let waiter = std::thread::spawn(move || done.send(waiter_job.wait().is_none()).unwrap());
        // Invalidation must complete while the old worker is still blocked.
        owner.invalidate_localized();
        assert!(
            result
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap()
        );
        let fresh = resolve(&owner, || key(1), build_test);
        assert_eq!(fresh.key.localized_epoch, 1);
        release.send(()).unwrap();
        worker.join().unwrap();
        waiter.join().unwrap();
        assert!(job.wait().is_none());
        assert!(Arc::ptr_eq(&fresh, &resolve(&owner, || key(1), build_test)));
    }

    #[test]
    fn stale_warmup_key_does_not_block_current_generation() {
        let owner = ApplicationAssetCache::default();
        let stale = LoadingJob::new(key(0));
        owner.state.lock().unwrap().loading = Some(stale.clone());
        let result = resolve(&owner, || key(1), build_test);
        assert_eq!(result.key.generation, 1);
        assert!(stale.is_cancelled());
    }

    #[test]
    fn completed_warmup_keeps_stable_banks_across_locale_invalidation() {
        let owner = ApplicationAssetCache::default();
        let job = LoadingJob::new(key(1));
        let warmed = build_test(key(1), None);
        job.finish(JobResult::Complete(warmed.clone()));
        owner.state.lock().unwrap().loading = Some(job);
        owner.invalidate_localized();
        let fresh = resolve(&owner, || key(1), build_test);
        assert_eq!(fresh.key.localized_epoch, 1);
        assert!(Arc::ptr_eq(&warmed.stable, &fresh.stable));
    }

    #[test]
    fn dropping_owner_cancels_detached_job_without_joining() {
        let owner = ApplicationAssetCache::default();
        let job = LoadingJob::new(key(1));
        owner.state.lock().unwrap().loading = Some(job.clone());
        drop(owner);
        job.finish(JobResult::Complete(build_test(key(1), None)));
        assert!(job.is_cancelled());
        assert!(job.wait().is_none());
    }

    #[test]
    fn abandoned_worker_releases_waiters_and_allows_retry() {
        let owner = ApplicationAssetCache::default();
        let job = LoadingJob::new(key(1));
        owner.state.lock().unwrap().loading = Some(job.clone());
        // Exercise the exact unwind cleanup without panicking in abort profiles.
        drop(JobCompletionGuard { job: Some(&job) });
        assert!(job.wait().is_none());
        let result = resolve(&owner, || key(1), build_test);
        assert_eq!(result.key.generation, 1);
    }

    #[test]
    #[cfg(panic = "unwind")]
    fn panicking_worker_does_not_poison_owner_and_next_caller_retries() {
        let owner = Arc::new(ApplicationAssetCache::default());
        let worker_owner = owner.clone();
        let failed = std::thread::spawn(move || {
            resolve(
                &worker_owner,
                || key(1),
                |_, _| {
                    panic!("injected cache worker panic");
                },
            )
        });
        assert!(failed.join().is_err());
        assert_eq!(resolve(&owner, || key(1), build_test).key.generation, 1);
    }

    #[test]
    fn confining_an_existing_primary_reader_invalidates_all_cached_banks() {
        let files = Arc::new(SbFileSystem::new(Arc::new(
            robin_util::asset_fs::AssetVfs::new(),
        )));
        let root = std::fs::canonicalize(env!("CARGO_MANIFEST_DIR")).unwrap();
        assert_eq!(files.set_primary_path(root.to_str().unwrap()), 0);
        let before = CacheKey::capture(None, 0, &files);
        let state = ApplicationAssetCache::default();
        let unconfined = resolve(&state, || before.clone(), build_test);
        assert_eq!(files.lock_ranked_verifier_primary_path(&root), 0);
        let after = CacheKey::capture(None, 0, &files);
        assert_eq!(before.mounts.primary_path, after.mounts.primary_path);
        assert_ne!(before, after);
        assert!(!before.same_stable_assets(&after));
        let confined = resolve(&state, || after.clone(), build_test);
        assert!(!Arc::ptr_eq(&unconfined.stable, &confined.stable));
    }

    #[test]
    fn explicit_readers_and_mount_changes_never_share_cache_entries() {
        let first = Arc::new(SbFileSystem::new(Arc::new(
            robin_util::asset_fs::AssetVfs::new(),
        )));
        let second = Arc::new(SbFileSystem::new(Arc::new(
            robin_util::asset_fs::AssetVfs::new(),
        )));
        let before = CacheKey::capture(None, 0, &first);
        let other = CacheKey::capture(None, 0, &second);
        assert_ne!(before, other);
        assert!(!before.same_stable_assets(&other));
        assert_eq!(first.set_locale_paths(Some("1036"), Some("1033")), 0);
        let localized = CacheKey::capture(None, 0, &first);
        assert_ne!(before, localized);
        assert!(before.same_stable_assets(&localized));
        assert_eq!(other, CacheKey::capture(None, 0, &second));
    }

    #[test]
    fn warmup_and_prepared_snapshot_reuse_banks_without_aliasing_other_readers() {
        let live = Arc::new(SbFileSystem::new(Arc::new(
            robin_util::asset_fs::AssetVfs::new(),
        )));
        let prepared = Arc::new(live.snapshot());
        let independent = Arc::new(SbFileSystem::new(Arc::new(
            robin_util::asset_fs::AssetVfs::new(),
        )));
        assert_eq!(
            CacheKey::capture(None, 0, &live),
            CacheKey::capture(None, 0, &prepared)
        );
        let state = ApplicationAssetCache::default();
        let warmup = resolve(&state, || CacheKey::capture(None, 0, &live), build_test);
        let mission = resolve(&state, || CacheKey::capture(None, 0, &prepared), build_test);
        assert!(Arc::ptr_eq(&warmup, &mission));
        let other = resolve(
            &state,
            || CacheKey::capture(None, 0, &independent),
            build_test,
        );
        assert!(!Arc::ptr_eq(&warmup.stable, &other.stable));
        assert_eq!(live.add_alternate_path("changed-root"), 0);
        assert_ne!(
            CacheKey::capture(None, 0, &live),
            CacheKey::capture(None, 0, &prepared)
        );
        let changed = resolve(&state, || CacheKey::capture(None, 0, &live), build_test);
        assert!(!Arc::ptr_eq(&warmup.stable, &changed.stable));
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct CacheKey {
    reader: u64,
    mounts: robin_engine::sbfile::SbFileMountSnapshot,
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
        let mut mounts = files.mount_snapshot();
        #[cfg(not(target_arch = "wasm32"))]
        if mounts.working_directory.is_none() {
            mounts.working_directory = Some(
                std::env::current_dir().expect("cannot capture asset-cache working directory"),
            );
        }
        Self {
            reader: files.origin_identity(),
            mounts,
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
        let guard = JobCompletionGuard { job: Some(self) };
        if self.is_cancelled() {
            return;
        }
        self.finish(match build() {
            Some(cache) => JobResult::Complete(cache),
            None => JobResult::Cancelled,
        });
        drop(guard);
    }
}

/// Unwinding must release single-flight waiters without poisoning the owner's
/// lock. Abort-on-panic builds still terminate the process, as before.
#[derive(serde::Serialize, serde::Deserialize)]
struct JobCompletionGuard<'a> {
    #[serde(skip)]
    job: Option<&'a LoadingJob>,
}

impl Drop for JobCompletionGuard<'_> {
    fn drop(&mut self) {
        if let Some(job) = self.job {
            if std::thread::panicking() {
                tracing::warn!("asset loading worker panicked; a subsequent caller can retry");
            }
            job.finish(JobResult::Failed);
        }
    }
}

/// Runtime cache ownership is never restored from serialized parsed products.
/// ApplicationServices skips this owner entirely when decoding a context.
#[derive(Default, serde::Serialize, serde::Deserialize)]
pub struct ApplicationAssetCache {
    #[serde(skip)]
    state: Mutex<State>,
}

impl std::fmt::Debug for ApplicationAssetCache {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ApplicationAssetCache")
            .finish_non_exhaustive()
    }
}

impl ApplicationAssetCache {
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
        if let Some(job) = state.loading.take() {
            if let Some(cache) = job.completed() {
                state.ready = Some(cache);
            }
            job.cancel();
        }
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
            if state.loading.as_ref().is_some_and(|job| job.key != key) {
                let stale = state.loading.take().expect("checked active cache job");
                if let Some(cache) = stale.completed() {
                    state.ready = Some(cache);
                }
                stale.cancel();
            }
            if let Some(cache) = &state.ready {
                if cache.key == key {
                    return cache.clone();
                }
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
            if capture(state.localized_epoch) == key {
                if let Some(cache) = result {
                    state.ready = Some(cache.clone());
                    return cache;
                }
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
        let fx_bank = match files.read_all(fx_bank_path) {
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
    let menu_bank = match files.read_all(menu_bank_path) {
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
                .read_all(&path)
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
    let mut resources = if pack.data_root.is_empty() {
        let shipping = shipping.ok_or_else(|| {
            format!(
                "shipping voice pack {} has no shipping datadir",
                pack.locale
            )
        })?;
        match shipping.locale_resource(&pack.locale, "Data/Sounds/Exclamations/actors.res") {
            Ok(Some(resources)) => resources.clone(),
            Ok(None) => return Err(format!("voice pack {} has no actors.res", pack.locale)),
            Err(error) => {
                return Err(format!(
                    "voice pack {} actors.res lookup failed: {error:#}",
                    pack.locale
                ));
            }
        }
    } else {
        let mut resources = ResourceManager::with_files(files.clone());
        let path = format!("{}/Data/Sounds/Exclamations/actors.res", pack.data_root);
        resources.attach_resource_file(&path).map_err(|error| {
            format!(
                "voice pack {} actors.res failed to load: {error:#}",
                pack.locale
            )
        })?;
        resources
    };

    let data_root = pack.data_root.clone();
    let locale = pack.locale.clone();
    build_exclamations_from(
        profiles,
        shipping,
        &mut resources,
        |dat_filename| {
            if data_root.is_empty() {
                let shipping = shipping.ok_or_else(|| "shipping datadir is absent".to_owned())?;
                let path = format!("Data/Sounds/Exclamations/{dat_filename}");
                shipping
                    .locale_raw(&locale, &path)
                    .map_err(|error| error.to_string())?
                    .map(<[u8]>::to_vec)
                    .ok_or_else(|| format!("{path}: missing from shipping locale {locale}"))
            } else {
                let path = format!("{data_root}/Data/Sounds/Exclamations/{dat_filename}");
                files
                    .read_all(&path)
                    .map_err(|status| format!("{path}: file error {status}"))
            }
        },
        &format!("canonical locale {}", pack.locale),
        true,
    )
}

fn build_exclamations_from(
    profiles: &ProfileManager,
    shipping: Option<&assets_shipping_datadir::ShippingDatadir>,
    excl_res: &mut ResourceManager,
    mut read_definition: impl FnMut(&str) -> Result<Vec<u8>, String>,
    source: &str,
    strict: bool,
) -> Result<Vec<Vec<(u32, Vec<String>)>>, String> {
    // Collect unique exclamation IDs from all profile types. The id's
    // non-zero LE bytes spell the actor file's name suffix.
    let mut files_needed = std::collections::BTreeMap::<u32, String>::new();
    let mut add = |excl_id: u32| {
        if excl_id != 0 {
            let name: String = excl_id
                .to_le_bytes()
                .iter()
                .filter(|&&b| b != 0)
                .map(|&b| b as char)
                .collect();
            files_needed.insert(excl_id, format!("actor{name}.dat"));
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
    for (&excl_id, dat_filename) in &files_needed {
        let data = match read_definition(dat_filename) {
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
            match robin_engine::sound_cache::parse_exclamation_file(&data, prefix_id) {
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
