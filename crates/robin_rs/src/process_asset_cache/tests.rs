use super::*;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

#[test]
fn voice_pack_backends_report_their_required_resource_failures() {
    let mut pack = crate::localization::LanguagePack {
        locale: "en".into(),
        native_name: "English".into(),
        data_root: String::new(),
        has_voice: true,
        has_cinematics: false,
        voice_uses_english_fallback: false,
        cinematics_use_english_fallback: false,
        mission_names: Default::default(),
    };
    let files = Arc::new(SbFileSystem::new(Arc::new(
        robin_util::asset_fs::AssetVfs::new(),
    )));
    let profiles = ProfileManager::new();
    assert_eq!(
        build_exclamations_for_language(&pack, None, &profiles, files.clone()).unwrap_err(),
        "shipping voice pack en has no shipping datadir"
    );
    pack.data_root = "missing-voice-fixture".into();
    let error = build_exclamations_for_language(&pack, None, &profiles, files).unwrap_err();
    assert!(
        error.starts_with("voice pack en actors.res failed to load:"),
        "{error}"
    );
}

#[test]
fn speech_definition_reads_are_unique_and_sorted_by_actor_id() {
    let mut profiles = ProfileManager::new();
    for id in [0x0042_0041, 0, 0x0042_0041, 0x41, 0x41, 0xff] {
        profiles
            .civilians
            .push(robin_engine::profiles::CivilianProfile {
                exclamation_id: id,
                ..Default::default()
            });
    }
    let mut resources = ResourceManager::new();
    let mut reads = Vec::new();
    let result = build_exclamations_from::<&[u8]>(
        &profiles,
        None,
        &mut resources,
        |name| {
            reads.push(name.to_owned());
            Err("fixture has no definition files".into())
        },
        "fixture",
        false,
    )
    .unwrap();
    assert!(result.is_empty());
    assert_eq!(reads, ["actorA.dat", "actorÿ.dat", "actorAB.dat"]);
}

#[test]
fn speech_definitions_accept_borrowed_and_shared_bytes_with_the_same_errors() {
    let id = u32::from_le_bytes(*b"ROBN");
    let mut profiles = ProfileManager::new();
    profiles
        .civilians
        .push(robin_engine::profiles::CivilianProfile {
            exclamation_id: id,
            ..Default::default()
        });
    let mut definition = b"NEUF".to_vec();
    for value in [1u32, 42, 1, 0] {
        definition.extend_from_slice(&value.to_le_bytes());
    }
    let mut resources = ResourceManager::new();
    let borrowed = build_exclamations_from(
        &profiles,
        None,
        &mut resources,
        |name| {
            assert_eq!(name, "actorROBN.dat");
            Ok(definition.as_slice())
        },
        "fixture",
        true,
    )
    .unwrap();
    let shared = robin_util::asset_fs::AssetBytes::from(definition.clone());
    let retained = build_exclamations_from(
        &profiles,
        None,
        &mut resources,
        |_| Ok(shared.clone()),
        "fixture",
        true,
    )
    .unwrap();
    assert_eq!(
        borrowed,
        vec![vec![(id & 0xffff_0000, Vec::<String>::new())]]
    );
    assert_eq!(retained, borrowed);
    let read_error = build_exclamations_from::<&[u8]>(
        &profiles,
        None,
        &mut resources,
        |_| Err("unreadable".to_owned()),
        "fixture",
        true,
    )
    .unwrap_err();
    assert!(read_error.contains("failed to read fixture"));
    let parse_error = build_exclamations_from(
        &profiles,
        None,
        &mut resources,
        |_| Ok(b"bad".as_slice()),
        "fixture",
        true,
    )
    .unwrap_err();
    assert!(parse_error.contains("failed to parse fixture"));
    assert!(
        build_exclamations_from(
            &profiles,
            None,
            &mut resources,
            |_| Ok(b"bad".as_slice()),
            "fixture",
            false,
        )
        .unwrap()
        .is_empty()
    );
}

fn key(generation: u64) -> CacheKey {
    CacheKey {
        reader: 0,
        mounts: SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new())).mount_snapshot(),
        presentation_locale: None,
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
fn retiring_jobs_preserves_only_completed_products_and_always_cancels() {
    for terminal in [None, Some(false), Some(true)] {
        let previous = build_test(key(1), None);
        let completed = build_test(key(2), None);
        let job = LoadingJob::new(key(2));
        match terminal {
            Some(true) => job.finish(JobResult::Complete(completed.clone())),
            Some(false) => job.finish(JobResult::Failed),
            None => {}
        }
        let mut state = State {
            ready: Some(previous.clone()),
            loading: Some(job.clone()),
            localized_epoch: 7,
        };
        state.retire_loading();
        assert!(state.loading.is_none());
        assert!(job.is_cancelled());
        assert!(job.wait().is_none());
        assert_eq!(state.localized_epoch, 7);
        let expected = if terminal == Some(true) {
            &completed
        } else {
            &previous
        };
        assert!(Arc::ptr_eq(state.ready.as_ref().unwrap(), expected));
        state.retire_loading();
        assert!(Arc::ptr_eq(state.ready.as_ref().unwrap(), expected));
    }
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
fn synchronous_builder_returns_replacement_after_invalidation() {
    let owner = Arc::new(ApplicationAssetCache::default());
    let (release, blocked) = std::sync::mpsc::channel();
    let (started, running) = std::sync::mpsc::channel();
    let builder_owner = owner.clone();
    let builder = std::thread::spawn(move || {
        resolve(
            &builder_owner,
            || key(1),
            |key, stable| {
                started.send(()).unwrap();
                blocked.recv().unwrap();
                build_test(key, stable)
            },
        )
    });
    running
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    let invalidating_owner = owner.clone();
    let (done, invalidated) = std::sync::mpsc::channel();
    let invalidator = std::thread::spawn(move || {
        invalidating_owner.invalidate_localized();
        done.send(()).unwrap();
    });
    invalidated
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    let fresh = resolve(&owner, || key(1), build_test);
    release.send(()).unwrap();
    assert!(Arc::ptr_eq(&fresh, &builder.join().unwrap()));
    invalidator.join().unwrap();
}

#[test]
fn retired_job_does_not_start_new_work() {
    let job = LoadingJob::new(key(1));
    job.cancel();
    job.run(|| panic!("retired work must not start"));
    assert!(job.wait().is_none());
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
    // Inject failure without relying on unwinding in abort profiles.
    job.finish(JobResult::Failed);
    assert!(job.wait().is_none());
    let result = resolve(&owner, || key(1), build_test);
    assert_eq!(result.key.generation, 1);
}

#[test]
#[cfg(panic = "unwind")]
#[ignore = "requires LLVM codegen to exercise unwind cleanup; see docs/TESTING.md"]
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
    files
        .set_primary_path(root.to_str().unwrap())
        .expect("mount fixture asset directory");
    let before = CacheKey::capture(None, 0, &files);
    let state = ApplicationAssetCache::default();
    let unconfined = resolve(&state, || before.clone(), build_test);
    files
        .lock_ranked_verifier_primary_path(&root)
        .expect("confine fixture asset lookup");
    let after = CacheKey::capture(None, 0, &files);
    assert_eq!(before.mounts.primary_path, after.mounts.primary_path);
    assert_ne!(before, after);
    assert!(!before.same_stable_assets(&after));
    let confined = resolve(&state, || after.clone(), build_test);
    assert!(!Arc::ptr_eq(&unconfined.stable, &confined.stable));
}

#[test]
fn language_only_changes_retire_localized_banks_but_reuse_stable_assets() {
    let files = Arc::new(SbFileSystem::new(Arc::new(
        robin_util::asset_fs::AssetVfs::new(),
    )));
    files
        .set_presentation_locale(None, None, Some("en-US"))
        .expect("configure initial fixture locale");
    let before = CacheKey::capture(None, 0, &files);
    let prepared = Arc::new(files.snapshot());
    files
        .set_presentation_locale(None, None, Some("ja-JP"))
        .expect("change fixture locale");
    let after = CacheKey::capture(None, 0, &files);
    assert_ne!(before, after);
    assert!(before.same_stable_assets(&after));
    assert_eq!(before, CacheKey::capture(None, 0, &prepared));
    let owner = ApplicationAssetCache::default();
    let previous = resolve(&owner, || before.clone(), build_test);
    let current = resolve(&owner, || after.clone(), build_test);
    assert!(!Arc::ptr_eq(&previous, &current));
    assert!(Arc::ptr_eq(&previous.stable, &current.stable));
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
    first
        .set_locale_paths(Some("1036"), Some("1033"))
        .expect("configure fixture locale");
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
    live.add_alternate_path("changed-root")
        .expect("mount fixture asset directory");
    assert_ne!(
        CacheKey::capture(None, 0, &live),
        CacheKey::capture(None, 0, &prepared)
    );
    let changed = resolve(&state, || CacheKey::capture(None, 0, &live), build_test);
    assert!(!Arc::ptr_eq(&warmup.stable, &changed.stable));
}
