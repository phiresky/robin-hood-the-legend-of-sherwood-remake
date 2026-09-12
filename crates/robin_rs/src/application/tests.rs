use super::*;
use crate::host::{FrontendPreferenceEffects, FrontendPreferences, Host, HostFrontend};
use robin_engine::player_profile::DifficultyLevel;
use winit::keyboard::KeyCode;

#[cfg(all(feature = "projection-export", not(target_arch = "wasm32")))]
#[test]
fn nondefault_projection_rules_survive_host_only_profile_updates() {
    let mut exact = engine_api::SimConfig::original_parity_ranked(DifficultyLevel::Hard);
    exact.synchronous_pathfinding = true;
    let application = ApplicationContext::complete_official_projection(
        engine_api::GlobalOptions::default(),
        exact,
        None,
    )
    .unwrap();
    application
        .with_player_profiles_mut(|profiles| {
            profiles.get_active_mut().unwrap().minimap_x = 123.0;
        })
        .unwrap();
    assert_eq!(application.sim_config(), exact);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn save_recovery_conflict_does_not_regenerate_profiles() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().to_str().unwrap();
    let store = crate::player_profile_store::PlayerProfileStore::for_directory(directory);
    let mut profiles = store.load().unwrap();
    profiles.get_active_mut().unwrap().name = "Retained identity".into();
    store.save(&profiles).unwrap();
    let archive = std::fs::read(root.path().join("profiles.json")).unwrap();
    std::fs::create_dir(root.path().join("Profile_000")).unwrap();
    store.quarantine_profile_saves(0).unwrap();
    std::fs::create_dir(root.path().join("Profile_000")).unwrap();
    // Metadata loading must still succeed: the launcher regenerates defaults
    // for corrupt archives, but a save-directory conflict is not corruption.
    let loaded = store.load().unwrap();
    let error = ApplicationContext::complete_with_localization(
        store,
        Default::default(),
        loaded,
        KeyConfigStore::new(directory.into()),
        None,
        LocalizationService::disabled(),
    )
    .unwrap_err();
    assert!(error.contains("recover interrupted player deletion"));
    assert_eq!(
        std::fs::read(root.path().join("profiles.json")).unwrap(),
        archive
    );
    assert!(root.path().join(".deleted-Profile_000").is_dir());
    assert!(root.path().join("Profile_000").is_dir());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn deletion_keeps_quarantined_saves_when_key_cleanup_fails() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().to_str().unwrap();
    let store = crate::player_profile_store::PlayerProfileStore::for_directory(directory);
    let mut profiles = store.load().unwrap();
    profiles.create_profile("Marian".into(), DifficultyLevel::Hard);
    store.save(&profiles).unwrap();
    let application = ApplicationContext::complete(
        store,
        Default::default(),
        profiles,
        KeyConfigStore::new(directory.into()),
        None,
    )
    .unwrap();
    std::fs::create_dir(root.path().join("Profile_000")).unwrap();
    std::fs::write(root.path().join("Profile_000/save.json"), b"retained").unwrap();
    std::fs::create_dir(root.path().join("keyconfigs.json")).unwrap();
    assert!(application.delete_player_profile(0).unwrap());
    assert_eq!(
        application.active_profile_snapshot().unwrap().name,
        "Marian"
    );
    assert_eq!(
        std::fs::read(root.path().join(".deleted-Profile_000/save.json")).unwrap(),
        b"retained"
    );
    let reloaded = crate::player_profile_store::PlayerProfileStore::for_directory(directory)
        .load()
        .unwrap();
    assert_eq!(reloaded.profiles.len(), 1);
    assert_eq!(reloaded.profiles[0].name, "Marian");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn recording_index_is_shared_only_within_application_and_retired_on_exit() {
    let directory = tempfile::tempdir().unwrap();
    let mut application = context(0, DifficultyLevel::Medium, KeyCode::F2, "index");
    let index = Arc::new(crate::mission_replays::RecordingIndex::native(
        directory.path().join("attempts"),
    ));
    Arc::get_mut(application.services.as_mut().unwrap())
        .unwrap()
        .recording_index = index.clone();
    let sibling = application.clone();
    let independent = context(0, DifficultyLevel::Medium, KeyCode::F2, "other-index");
    assert!(Arc::ptr_eq(
        application.recording_index(),
        sibling.recording_index()
    ));
    assert!(!Arc::ptr_eq(
        application.recording_index(),
        independent.recording_index()
    ));
    let diagnostic = serde_json::to_value(&application).unwrap();
    assert!(diagnostic["services"].get("recording_index").is_none());
    assert!(serde_json::from_value::<ApplicationContext>(diagnostic).is_err());
    index.refresh_index().unwrap();
    pollster::block_on(application.shutdown()).unwrap();
    assert!(
        sibling
            .recording_index()
            .refresh_index()
            .unwrap_err()
            .contains("shut down")
    );
    drop(application);
    drop(sibling);
    assert!(index.refresh_index().is_err());

    // Unstructured exits must retire the service even when a view retains
    // the narrow index capability beyond the last application context.
    let application = context(0, DifficultyLevel::Medium, KeyCode::F2, "drop-index");
    let index = application.recording_index().clone();
    drop(application);
    assert!(index.refresh_index().is_err());
}

#[test]
fn failed_profile_transactions_do_not_publish_profiles_or_settings() {
    let context = context(0, DifficultyLevel::Medium, KeyCode::F2, "transaction");
    let before = serde_json::to_value(context.player_profiles_snapshot().unwrap()).unwrap();
    let config = context.sim_config();
    let error = context
        .try_update_player_profiles(|profiles| {
            profiles.get_active_mut().unwrap().difficulty = DifficultyLevel::Hard;
            Err::<(), _>("rejected mutation".to_string())
        })
        .unwrap_err();
    assert_eq!(error, "rejected mutation");
    assert_eq!(
        serde_json::to_value(context.player_profiles_snapshot().unwrap()).unwrap(),
        before
    );
    assert_eq!(context.sim_config(), config);
    let error = context
        .with_player_profiles_mut(|profiles| {
            profiles.get_active_mut().unwrap().difficulty = DifficultyLevel::Hard;
            profiles.active_index = None;
        })
        .unwrap_err();
    assert!(error.contains("leave an active profile"));
    assert_eq!(
        serde_json::to_value(context.player_profiles_snapshot().unwrap()).unwrap(),
        before
    );
    assert_eq!(context.sim_config(), config);
    assert!(
        context
            .with_player_profiles_mut(|profiles| {
                profiles.active_index = Some(profiles.profiles.len());
            })
            .unwrap_err()
            .contains("leave an active profile")
    );
    assert!(
        context
            .with_player_profiles_mut(|profiles| {
                let mut rules = DifficultyLevel::Medium.rules();
                rules.enemy_fighting_percent = u16::MAX;
                profiles.get_active_mut().unwrap().difficulty = DifficultyLevel::Custom(rules);
            })
            .unwrap_err()
            .contains("invalid staged player profiles")
    );
    assert_eq!(
        serde_json::to_value(context.player_profiles_snapshot().unwrap()).unwrap(),
        before
    );
    assert_eq!(context.sim_config(), config);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn profile_persistence_validates_before_writing_and_has_explicit_failure_policy() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().to_str().unwrap();
    let store = crate::player_profile_store::PlayerProfileStore::for_directory(directory);
    let profiles = store.load().unwrap();
    let context = ApplicationContext::complete(
        store,
        Default::default(),
        profiles,
        KeyConfigStore::new(directory.into()),
        None,
    )
    .unwrap();
    let archive = root.path().join("profiles.json");
    let original = std::fs::read(&archive).unwrap();
    let original_config = context.sim_config();
    for retain in [false, true] {
        let invalidate = |profiles: &mut PlayerProfileManager| {
            profiles.active_index = None;
        };
        let result = if retain {
            context.update_and_retain_player_profiles(invalidate)
        } else {
            context.try_persist_player_profiles(|profiles| {
                invalidate(profiles);
                Ok(())
            })
        };
        assert!(result.unwrap_err().contains("leave an active profile"));
        assert_eq!(std::fs::read(&archive).unwrap(), original);
        assert_eq!(context.sim_config(), original_config);
    }
    assert!(
        context
            .try_persist_player_profiles(|profiles| {
                profiles.get_active_mut().unwrap().name = "Must not publish".into();
                Err::<(), _>("rejected".into())
            })
            .is_err()
    );
    assert_eq!(std::fs::read(&archive).unwrap(), original);

    // Block replacement, without permissions/root assumptions or fault globals.
    std::fs::rename(&archive, root.path().join("original.json")).unwrap();
    std::fs::create_dir(&archive).unwrap();
    let change = |profiles: &mut PlayerProfileManager| {
        profiles.get_active_mut().unwrap().difficulty = DifficultyLevel::Hard;
        17
    };
    assert!(
        context
            .try_persist_player_profiles(|profiles| Ok(change(profiles)))
            .is_err()
    );
    assert_eq!(context.sim_config(), original_config);
    let retained = context.update_and_retain_player_profiles(change).unwrap();
    assert_eq!(retained.value, 17);
    assert!(retained.persistence.is_err());
    assert_eq!(
        context.active_profile_snapshot().unwrap().difficulty,
        DifficultyLevel::Hard
    );
    assert_eq!(context.sim_config().difficulty, DifficultyLevel::Hard);

    std::fs::remove_dir(&archive).unwrap();
    std::fs::rename(root.path().join("original.json"), &archive).unwrap();
    context.save_player_profiles().unwrap().persistence.unwrap();
    let reloaded = crate::player_profile_store::PlayerProfileStore::for_directory(directory)
        .load()
        .unwrap();
    assert_eq!(
        reloaded.get_active().unwrap().difficulty,
        DifficultyLevel::Hard
    );
    let published = context
        .try_persist_player_profiles(|profiles| {
            profiles.get_active_mut().unwrap().name = "Published".into();
            Ok(23)
        })
        .unwrap();
    assert_eq!(published.value, 23);
    published.persistence.unwrap();
    assert_eq!(
        crate::player_profile_store::PlayerProfileStore::for_directory(directory)
            .load()
            .unwrap()
            .get_active()
            .unwrap()
            .name,
        "Published"
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn profile_publication_visibility_distinguishes_durability_from_replacement_failure() {
    use crate::desktop_persistence::{PublicationFailure, PublicationStage};
    for stage in [
        PublicationStage::Prepare,
        PublicationStage::Write,
        PublicationStage::SyncFile,
        PublicationStage::Replace,
        PublicationStage::SyncDirectory,
    ] {
        let error = std::io::Error::other(PublicationFailure {
            stage,
            detail: "injected".into(),
        });
        assert_eq!(
            profile_publication_visible(&error),
            stage == PublicationStage::SyncDirectory
        );
    }
    assert!(!profile_publication_visible(&std::io::Error::other(
        "unclassified"
    )));
}

#[test]
#[cfg(all(panic = "unwind", not(target_arch = "wasm32")))]
#[ignore = "requires LLVM unwinding; run explicitly with robin_rs test codegen-backend=llvm"]
fn poisoned_simulation_lock_prevents_profile_callback_and_first_launch() {
    let context = context(
        0,
        DifficultyLevel::Medium,
        KeyCode::F2,
        "poisoned-transaction",
    );
    context
        .required_services()
        .unwrap()
        .player_profiles
        .lock()
        .unwrap()
        .default_profiles = true;
    let before = serde_json::to_value(context.player_profiles_snapshot().unwrap()).unwrap();
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = context.sim_config.lock().unwrap();
        panic!("poison config for regression test");
    }));
    let mut called = false;
    assert!(
        context
            .with_player_profiles_mut(|_| {
                called = true;
            })
            .unwrap_err()
            .contains("sim-config lock poisoned")
    );
    assert!(!called);
    assert!(
        context
            .try_persist_player_profiles(|_| {
                called = true;
                Ok(())
            })
            .unwrap_err()
            .contains("sim-config lock poisoned")
    );
    assert!(
        context
            .update_and_retain_player_profiles(|_| {
                called = true;
            })
            .unwrap_err()
            .contains("sim-config lock poisoned")
    );
    assert!(!called);
    assert!(
        context
            .complete_first_launch_profile(None, (800, 600))
            .unwrap_err()
            .contains("sim-config lock poisoned")
    );
    assert_eq!(
        serde_json::to_value(context.player_profiles_snapshot().unwrap()).unwrap(),
        before
    );
}

#[test]
#[cfg(all(panic = "unwind", not(target_arch = "wasm32")))]
#[ignore = "requires LLVM unwinding; run explicitly with robin_rs test codegen-backend=llvm"]
fn poisoned_profile_lock_prevents_callback_and_configuration_changes() {
    let context = context(0, DifficultyLevel::Medium, KeyCode::F2, "poisoned-profile");
    let before = context.sim_config();
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = context
            .required_services()
            .unwrap()
            .player_profiles
            .lock()
            .unwrap();
        panic!("poison profiles for regression test");
    }));
    let mut called = false;
    assert!(
        context
            .with_player_profiles_mut(|_| {
                called = true;
            })
            .unwrap_err()
            .contains("player-profile lock poisoned")
    );
    assert!(!called);
    assert_eq!(context.sim_config(), before);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn concurrent_profile_transactions_publish_matching_configuration() {
    let context = context(
        0,
        DifficultyLevel::Medium,
        KeyCode::F2,
        "concurrent-transaction",
    );
    std::thread::scope(|scope| {
        for speech in [2, 9] {
            let context = &context;
            scope.spawn(move || {
                for _ in 0..100 {
                    context
                        .with_player_profiles_mut(|profiles| {
                            profiles
                                .get_active_mut()
                                .unwrap()
                                .sound_config
                                .amount_of_speaking = speech;
                        })
                        .unwrap();
                    // Inspect the joint snapshot in the transaction's lock order.
                    let profiles = context
                        .required_services()
                        .unwrap()
                        .player_profiles
                        .lock()
                        .unwrap();
                    let config = context.sim_config.lock().unwrap();
                    assert_eq!(
                        profiles
                            .get_active()
                            .unwrap()
                            .sound_config
                            .amount_of_speaking,
                        config.amount_of_speaking
                    );
                }
            });
        }
    });
}

#[test]
fn cache_maintenance_is_application_owned_and_not_deserialized() {
    let first = context(42, DifficultyLevel::Medium, KeyCode::KeyA, "maintenance");
    let cloned = first.clone();
    let independent = context(42, DifficultyLevel::Medium, KeyCode::KeyA, "maintenance");
    assert!(std::ptr::eq(
        &first.required_services().unwrap().cache_maintenance,
        &cloned.required_services().unwrap().cache_maintenance,
    ));
    assert!(!std::ptr::eq(
        &first.required_services().unwrap().cache_maintenance,
        &independent.required_services().unwrap().cache_maintenance,
    ));
    assert_eq!(
        first.cache_clear_status().unwrap(),
        crate::cache_maintenance::CacheClearStatus::Idle
    );
    let encoded = serde_json::to_value(&first).unwrap();
    assert!(encoded["services"].get("cache_maintenance").is_none());
    let _: ApplicationContextDiagnostic = serde_json::from_value(encoded.clone()).unwrap();
    assert!(serde_json::from_value::<ApplicationContext>(encoded).is_err());
    assert!(ApplicationContext::default().cache_clear_status().is_err());
}

#[test]
fn application_asset_cache_is_shared_by_clones_only_and_not_decoded() {
    let make_context = || {
        let mut application = context(42, DifficultyLevel::Medium, KeyCode::KeyA, "cache");
        let files = Arc::new(robin_engine::sbfile::SbFileSystem::new(Arc::new(
            robin_util::asset_fs::AssetVfs::new(),
        )));
        Arc::get_mut(application.services.as_mut().unwrap())
            .unwrap()
            .preparation_files = Some(files);
        application
    };
    let first = make_context();
    let cloned = first.clone();
    let independent = make_context();
    assert!(std::ptr::eq(
        first.asset_cache().unwrap(),
        cloned.asset_cache().unwrap()
    ));
    assert!(!std::ptr::eq(
        first.asset_cache().unwrap(),
        independent.asset_cache().unwrap()
    ));
    let encoded = serde_json::to_value(&first).unwrap();
    assert!(encoded["services"].get("asset_cache").is_none());
    let _: ApplicationContextDiagnostic = serde_json::from_value(encoded.clone()).unwrap();
    assert!(serde_json::from_value::<ApplicationContext>(encoded).is_err());
    assert!(ApplicationContext::default().asset_cache().is_err());
}

#[test]
fn explicit_context_store_ignores_archive_directory_metadata() {
    let selected = tempfile::tempdir().unwrap();
    let redirected = tempfile::tempdir().unwrap();
    let store = crate::player_profile_store::PlayerProfileStore::for_directory(
        selected.path().to_str().unwrap(),
    );
    let mut profiles = store.load().unwrap();
    profiles.save_directory = redirected.path().to_str().unwrap().into();
    let archive = serde_json::to_string(&profiles).unwrap();
    let decoded = serde_json::from_str(&archive).unwrap();
    let context = ApplicationContext::complete(
        store,
        engine_api::GlobalOptions::default(),
        decoded,
        KeyConfigStore::new(selected.path().to_str().unwrap().into()),
        None,
    )
    .unwrap();
    context.save_player_profiles().unwrap().persistence.unwrap();
    assert!(
        context
            .active_profile_save_directory()
            .unwrap()
            .starts_with(selected.path())
    );
    assert!(!redirected.path().join("profiles.json").exists());
}

#[test]
fn deserialized_context_cannot_recover_profile_storage_authority() {
    let original = context(
        42,
        DifficultyLevel::Medium,
        KeyCode::KeyA,
        "store-authority",
    );
    let encoded = serde_json::to_value(&original).unwrap();
    assert!(encoded["services"].get("profile_store").is_none());
    let diagnostic: ApplicationContextDiagnostic = serde_json::from_value(encoded.clone()).unwrap();
    assert!(
        diagnostic
            .services
            .unwrap()
            .player_profiles
            .get_active()
            .is_some()
    );
    assert!(serde_json::from_value::<ApplicationContext>(encoded.clone()).is_err());
    assert!(serde_json::from_value::<ReadyApplicationContext>(encoded).is_err());
}

fn spellforge_trust_key(value: u8) -> SpellforgeTrustKey {
    SpellforgeTrustKey {
        full_mod_sha256: [value; 32],
        package_sha256: Some([value.wrapping_add(1); 32]),
    }
}

fn spellforge_trust_metadata() -> SpellforgeTrustMetadata {
    SpellforgeTrustMetadata {
        mission: "Mission".into(),
        title: "Mission".into(),
        claimed_author: "Author".into(),
        version: "1".into(),
        source_url: "https://example.invalid/mod".into(),
        license: "CC0".into(),
        host_endpoint_id: "endpoint-public-key".into(),
        package_vm_abi: Some("spellforge-v1-sha256:00".into()),
        compressed_bytes: 123,
    }
}

fn context(
    profile_id: u32,
    difficulty: DifficultyLevel,
    key: KeyCode,
    shipping_marker: &str,
) -> ApplicationContext {
    let mut profiles = PlayerProfileManager::new(format!("/tmp/context-{profile_id}"));
    let profile_idx = profiles.create_profile(format!("Profile {profile_id}"), difficulty);
    profiles.set_active(profile_idx);

    let mut keys = KeyConfigStore::new(format!("/tmp/context-{profile_id}"));
    keys.entry_or_default(profile_id)
        .active
        .set_binding("ZoomIn", Some(key), None);

    let mut shipping = ShippingDatadir::default();
    shipping
        .raw
        .insert(shipping_marker.to_string(), vec![profile_id as u8]);

    ApplicationContext::complete(
        crate::player_profile_store::PlayerProfileStore::for_directory(&format!(
            "/tmp/context-{profile_id}"
        )),
        engine_api::GlobalOptions::default(),
        profiles,
        keys,
        Some(Arc::new(shipping)),
    )
    .unwrap()
}

#[test]
fn independent_contexts_do_not_cross_talk() {
    let easy = context(0, DifficultyLevel::Easy, KeyCode::F2, "easy.marker");
    let hard = context(0, DifficultyLevel::Hard, KeyCode::F3, "hard.marker");

    let easy_host = Host::new(easy.clone().try_into().unwrap(), 1024.0, 768.0).unwrap();
    let hard_host = Host::new(hard.clone().try_into().unwrap(), 1024.0, 768.0).unwrap();

    assert_eq!(easy.sim_config().difficulty, DifficultyLevel::Easy);
    assert_eq!(hard.sim_config().difficulty, DifficultyLevel::Hard);
    assert_eq!(
        easy_host
            .frontend
            .preferences()
            .key_config()
            .get_binding("ZoomIn")
            .unwrap()
            .primary_key,
        Some(KeyCode::F2)
    );
    assert_eq!(
        hard_host
            .frontend
            .preferences()
            .key_config()
            .get_binding("ZoomIn")
            .unwrap()
            .primary_key,
        Some(KeyCode::F3)
    );
    assert!(
        easy_host
            .frontend
            .resources
            .shipping
            .as_ref()
            .unwrap()
            .raw
            .contains_key("easy.marker")
    );
    assert!(
        !easy_host
            .frontend
            .resources
            .shipping
            .as_ref()
            .unwrap()
            .raw
            .contains_key("hard.marker")
    );
    assert!(
        hard_host
            .frontend
            .resources
            .shipping
            .as_ref()
            .unwrap()
            .raw
            .contains_key("hard.marker")
    );

    easy.with_player_profiles_mut(|profiles| {
        profiles.get_active_mut().unwrap().minimap_x = 123.0;
    })
    .unwrap();
    let hard_x = hard
        .with_player_profiles_mut(|profiles| profiles.get_active().unwrap().minimap_x)
        .unwrap();
    assert_eq!(hard_x, 65536.0);

    easy.with_player_profiles_mut(|profiles| {
        profiles.get_active_mut().unwrap().difficulty = DifficultyLevel::Medium;
    })
    .unwrap();
    assert_eq!(easy.sim_config().difficulty, DifficultyLevel::Medium);
    assert_eq!(hard.sim_config().difficulty, DifficultyLevel::Hard);
}

#[test]
fn production_host_reports_service_failure_after_readiness_validation() {
    let context = context(0, DifficultyLevel::Medium, KeyCode::F2, "ready.marker");
    let ready = ReadyApplicationContext::try_from(context.clone()).unwrap();
    let services = context.required_services().unwrap();
    services.player_profiles.lock().unwrap().active_index = None;
    let error = Host::new(ready, 800.0, 600.0).err().unwrap();
    assert!(error.contains("no active player profile"), "{error}");
}

#[test]
fn replay_composition_precedes_sharing_and_diagnostics_do_not_restore_authority() {
    use std::io::Write;
    let early = Arc::new(crate::replay_service::ReplayService::default());
    let mut writer = early.recording().begin_recording();
    writer.write_all(b"early browser recording\n").unwrap();
    writer.flush().unwrap();
    let application = context(0, DifficultyLevel::Medium, KeyCode::F2, "replay.marker")
        .with_replay_service(early)
        .unwrap();
    let sibling = application.clone();
    assert_eq!(
        sibling.replay_exports().snapshot_bytes().unwrap(),
        b"early browser recording\n"
    );
    let independent = context(
        0,
        DifficultyLevel::Medium,
        KeyCode::F2,
        "independent-replay.marker",
    );
    assert!(
        independent
            .replay_exports()
            .snapshot_bytes()
            .unwrap()
            .is_empty()
    );
    assert!(
        application
            .with_replay_service(Arc::new(Default::default()))
            .unwrap_err()
            .contains("precede sharing")
    );
    let diagnostic = serde_json::to_value(&sibling).unwrap();
    assert!(diagnostic["services"].get("replay").is_none());
    let _: ApplicationContextDiagnostic = serde_json::from_value(diagnostic.clone()).unwrap();
    assert!(serde_json::from_value::<ApplicationContext>(diagnostic).is_err());
    assert_eq!(
        sibling.replay_exports().snapshot_bytes().unwrap(),
        b"early browser recording\n"
    );
}

#[test]
fn ready_diagnostics_cannot_reconstruct_live_authority() {
    let ready = ReadyApplicationContext::try_from(context(
        0,
        DifficultyLevel::Medium,
        KeyCode::F2,
        "ready.marker",
    ))
    .unwrap();
    let bytes = serde_json::to_vec(&ready).unwrap();
    let diagnostic: ApplicationContextDiagnostic = serde_json::from_slice(&bytes).unwrap();
    assert!(diagnostic.services.is_some());
    assert!(serde_json::from_slice::<ReadyApplicationContext>(&bytes).is_err());
    assert!(serde_json::from_slice::<ApplicationContext>(&bytes).is_err());
    assert!(Host::new(ready, 800.0, 600.0).is_ok());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn replay_authority_cannot_change_after_transport_configuration() {
    let application = context(0, DifficultyLevel::Medium, KeyCode::F2, "transport.marker");
    // Port zero configures the disabled native endpoint without opening a
    // socket. Even that endpoint owns the original ingress capabilities.
    application.start_http_transport(0).unwrap();
    let error = application
        .with_replay_service(Arc::new(Default::default()))
        .unwrap_err();
    assert!(error.contains("precede starting"), "{error}");
}

#[test]
fn bootstrap_diagnostics_preserve_options_without_granting_services() {
    let options = engine_api::GlobalOptions {
        highlander: true,
        script_enabled: false,
        ..Default::default()
    };
    let bootstrap = ApplicationContext::bootstrap(options);
    let encoded = serde_json::to_value(&bootstrap).unwrap();
    let diagnostic: ApplicationContextDiagnostic = serde_json::from_value(encoded.clone()).unwrap();
    assert!(diagnostic.services.is_none());
    assert_eq!(diagnostic.sim_config(), bootstrap.sim_config());
    assert_eq!(serde_json::to_value(&diagnostic).unwrap(), encoded);
    assert!(serde_json::from_value::<ApplicationContext>(encoded).is_err());

    // Retaining launcher options is an explicit construction operation,
    // not a restore of the diagnostic's recorded application authority.
    let launch = ApplicationContext::bootstrap(diagnostic.options);
    assert!(launch.options().highlander);
    assert!(ReadyApplicationContext::try_from(launch).is_err());
}

#[test]
fn scratch_hosts_do_not_gain_application_service_authority() {
    for host in [Host::default(), Host::scratch(800.0, 600.0)] {
        assert!(
            host.application_context()
                .active_profile_snapshot()
                .is_err()
        );
        assert!(ReadyApplicationContext::try_from(host.application_context().clone()).is_err());
    }
}

#[test]
fn production_host_rejects_decoded_services_without_an_active_profile() {
    let context = context(0, DifficultyLevel::Medium, KeyCode::F2, "ready.marker");
    let mut encoded = serde_json::to_value(context).unwrap();
    encoded["services"]["player_profiles"]["profiles"] = serde_json::json!([]);
    assert!(serde_json::from_value::<ReadyApplicationContext>(encoded).is_err());
}

#[cfg(all(feature = "projection-export", not(target_arch = "wasm32")))]
#[test]
fn official_projection_context_preserves_the_exact_current_sim_config() {
    let sim_config = engine_api::SimConfig {
        script_enabled: false,
        highlander: true,
        amount_of_speaking: 2,
        synchronous_pathfinding: true,
        item_gameplay: robin_engine::gameplay_config::ItemGameplayConfig::classic(),
        ..engine_api::SimConfig::default()
    };
    let options = engine_api::GlobalOptions {
        script_enabled: false,
        highlander: true,
        ..Default::default()
    };
    let context =
        ApplicationContext::complete_official_projection(options, sim_config, None).unwrap();

    assert_eq!(context.sim_config(), sim_config);
    assert!(context.save_player_profiles().unwrap().persistence.is_err());
    assert!(context.cache_clear_status().is_err());
    assert_eq!(
        serde_json::to_value(context.recording_index()).unwrap()["directory"],
        serde_json::Value::Null
    );
    assert_eq!(
        context
            .clone()
            .with_options(context.options().clone())
            .sim_config(),
        sim_config,
        "ordinary headless handoff and exporter must seal byte-identical rules"
    );
    let profiles = context.player_profiles_snapshot().unwrap();
    assert_eq!(profiles.save_directory, "official-projection-memory-only");
    assert_eq!(profiles.profiles.len(), 1);
    assert_eq!(
        profiles
            .get_active()
            .unwrap()
            .sound_config
            .amount_of_speaking,
        2
    );
    assert!(
        context
            .active_spellforge_trust_grants()
            .unwrap_err()
            .contains("disabled in the closed official projection context")
    );
    assert!(
        context
            .clear_distributed_mod_cache()
            .unwrap_err()
            .contains("disabled in the closed official projection context")
    );

    let mut mismatched = context.options().clone();
    mismatched.script_enabled = true;
    assert!(
        ApplicationContext::complete_official_projection(mismatched, sim_config, None).is_err()
    );
}

#[test]
fn replacing_launcher_options_preserves_profile_speech_amount() {
    let context = context(0, DifficultyLevel::Medium, KeyCode::F2, "speech.marker");
    context
        .with_player_profiles_mut(|profiles| {
            profiles
                .get_active_mut()
                .unwrap()
                .sound_config
                .amount_of_speaking = 9;
        })
        .unwrap();

    let options = engine_api::GlobalOptions {
        highlander2: true,
        ..Default::default()
    };
    let replaced = context.with_options(options);

    assert_eq!(replaced.sim_config().amount_of_speaking, 9);
    assert!(replaced.sim_config().highlander2);
}

#[test]
fn cloned_launch_options_stay_local_while_profile_updates_are_shared() {
    let original = context(
        0,
        DifficultyLevel::Easy,
        KeyCode::F2,
        "clone-options.marker",
    );
    let changed = original.clone().with_options(engine_api::GlobalOptions {
        script_enabled: false,
        highlander: true,
        highlander2: true,
        golden_eye: true,
        ignore_default_loose: true,
        bypass_fog_sprites_crash: true,
        ..Default::default()
    });
    let original_options = original.options().clone();
    let changed_options = changed.options().clone();
    let sealed = changed.sim_config();

    for (updater, difficulty, speech) in [
        (&original, DifficultyLevel::Hard, 9),
        (&changed, DifficultyLevel::Medium, 2),
    ] {
        updater
            .with_player_profiles_mut(|profiles| {
                let active = profiles.get_active_mut().unwrap();
                active.difficulty = difficulty;
                active.sound_config.amount_of_speaking = speech;
                active.gameplay_config.enable_unbinding = false;
            })
            .unwrap();
        for (context, options) in [(&original, &original_options), (&changed, &changed_options)] {
            let gameplay = context.active_profile_snapshot().unwrap().gameplay_config;
            assert_eq!(
                context.sim_config(),
                profile_sim_config(options, difficulty, speech, gameplay),
            );
            assert_eq!(
                serde_json::to_value(context.options()).unwrap(),
                serde_json::to_value(options).unwrap(),
            );
        }
    }
    assert_eq!(sealed.difficulty, DifficultyLevel::Easy);
    assert!(
        sealed.highlander,
        "already sealed simulation values are independent copies"
    );
    // The diagnostic wire snapshot must retain the same effective contract.
    let decoded: ApplicationContextDiagnostic =
        serde_json::from_value(serde_json::to_value(&changed).unwrap()).unwrap();
    assert_eq!(decoded.sim_config(), changed.sim_config());
}

#[test]
fn startup_and_options_use_the_same_frontend_projection() {
    let context = context(0, DifficultyLevel::Hard, KeyCode::F4, "projection.marker");
    context
        .with_player_profiles_mut(|profiles| {
            let profile = profiles.get_active_mut().unwrap();
            profile.gameplay_config.control_tactical_units = false;
            profile.gameplay_config.plan_quick_actions = false;
            profile.gameplay_config.touch_camera_gestures = false;
            profile.graphic_config.native_refresh_presentation = true;
            profile.graphic_config.quick_action_cursor_pulse = false;
            profile.graphic_config.diplomacy_visuals = true;
            profile.sound_config.amount_of_speaking = 7;
        })
        .unwrap();
    let startup = Host::new(context.clone().try_into().unwrap(), 800.0, 600.0).unwrap();
    let profile = context.active_profile_snapshot().unwrap();
    let (keys, custom_keys) = context.active_key_configs().unwrap();
    let mut live = Host::scratch(800.0, 600.0);
    FrontendPreferences::new(
        KeyConfig::default(),
        KeyConfig::default(),
        robin_engine::gameplay_config::GameplayConfig {
            plan_quick_actions: true,
            ..Default::default()
        },
        &Default::default(),
    )
    .apply(&mut live.frontend);
    live.frontend.route_touch_plan_event(
        &crate::gfx_types::GameEvent::MouseDown(0, 0, 1, 1),
        true,
        |_, _| true,
    );
    assert!(live.frontend.planning().touch_latched());
    let sealed = engine_api::SimConfig {
        difficulty: DifficultyLevel::Easy,
        amount_of_speaking: 1,
        ..Default::default()
    };
    let (channels, _incoming, _outgoing, _, _) = crate::multiplayer::NetChannels::new();
    live.transport.install_session(
        channels,
        robin_engine::player_command::PlayerId::HOST,
        "leicester".into(),
        42,
        sealed,
        None,
    );
    let effects = FrontendPreferences::new(
        keys,
        custom_keys,
        profile.gameplay_config,
        &profile.graphic_config,
    )
    .apply(&mut live.frontend);
    assert_eq!(
        effects,
        FrontendPreferenceEffects {
            cancel_planned_action: true,
            native_refresh_presentation: true,
            release_tactical_control: true,
        }
    );
    assert!(!live.frontend.planning().touch_latched());
    assert_eq!(live.transport.mission_sim_config(), Some(sealed));
    assert_eq!(context.sim_config().amount_of_speaking, 7);
    for frontend in [&startup.frontend, &live.frontend] {
        assert_eq!(
            serde_json::to_value(FrontendPreferences::new(
                frontend.preferences().key_config().clone(),
                frontend.preferences().custom_key_config().clone(),
                frontend.preferences().gameplay_config(),
                &profile.graphic_config,
            ))
            .unwrap(),
            serde_json::to_value(context.host_snapshot().unwrap().preferences).unwrap(),
        );
        assert!(!frontend.preferences().control_tactical_units());
        assert!(!frontend.planning().enabled());
        assert!(!frontend.preferences().touch_camera_gestures());
        assert!(frontend.preferences().native_refresh_presentation());
        assert!(!frontend.preferences().quick_action_cursor_pulse());
        assert!(frontend.preferences().diplomacy_visuals());
    }
}

#[test]
fn profile_updates_preserve_explicit_pathfinding_construction_policy() {
    let context = context(
        0,
        DifficultyLevel::Medium,
        KeyCode::F2,
        "pathfinding.marker",
    );
    context.sim_config.lock().unwrap().synchronous_pathfinding = true;
    let sibling = context.clone().with_options(engine_api::GlobalOptions {
        highlander: true,
        ..Default::default()
    });
    sibling
        .with_player_profiles_mut(|profiles| {
            profiles
                .get_active_mut()
                .unwrap()
                .sound_config
                .amount_of_speaking = 9;
        })
        .unwrap();
    for snapshot in [context.sim_config(), sibling.sim_config()] {
        assert!(snapshot.synchronous_pathfinding);
        assert_eq!(snapshot.amount_of_speaking, 9);
        assert_eq!(snapshot.difficulty, DifficultyLevel::Medium);
    }
    assert!(!context.sim_config().highlander);
    assert!(sibling.sim_config().highlander);
}

#[test]
fn frontend_projection_preserves_session_planning_policy() {
    let mut frontend = HostFrontend::default();
    frontend.force_planning_off_for_session();
    let gameplay = robin_engine::gameplay_config::GameplayConfig {
        plan_quick_actions: true,
        control_tactical_units: true,
        ..Default::default()
    };
    let effects = FrontendPreferences::new(
        KeyConfig::default(),
        KeyConfig::default(),
        gameplay,
        &Default::default(),
    )
    .apply(&mut frontend);
    assert!(effects.cancel_planned_action);
    assert!(!effects.release_tactical_control);
    assert!(!frontend.planning().enabled());
}

#[test]
fn active_profile_queries_return_owned_values_and_release_the_profile_lock() {
    let context = context(
        0,
        DifficultyLevel::Medium,
        KeyCode::F4,
        "profile-query.marker",
    );
    let expected = context.active_profile_snapshot().unwrap();
    let values = context
        .with_active_profile(|profile| (profile.id, profile.name.clone(), profile.difficulty))
        .unwrap();
    assert_eq!(values, (expected.id, expected.name, expected.difficulty));
    assert!(
        context
            .required_services()
            .unwrap()
            .player_profiles
            .try_lock()
            .is_ok()
    );
}

#[test]
fn unavailable_or_missing_active_profiles_never_invoke_the_reader() {
    let called = std::cell::Cell::new(false);
    assert!(
        ApplicationContext::default()
            .with_active_profile(|_| called.set(true))
            .is_err()
    );
    assert!(!called.get());

    let context = context(
        0,
        DifficultyLevel::Medium,
        KeyCode::F4,
        "missing-profile.marker",
    );
    let services = context.required_services().unwrap();
    for index in [None, Some(usize::MAX)] {
        services.player_profiles.lock().unwrap().active_index = index;
        let error = context
            .with_active_profile(|_| called.set(true))
            .unwrap_err();
        assert_eq!(error, "ApplicationContext has no active player profile");
        assert!(!called.get());
        assert_eq!(context.active_profile_snapshot().unwrap_err(), error);
        assert!(services.player_profiles.try_lock().is_ok());
    }
}

#[test]
fn profile_and_key_projection_holds_both_locks_until_values_are_copied() {
    let context = context(0, DifficultyLevel::Medium, KeyCode::F4, "coherent.marker");
    let services = context.required_services().unwrap();
    context
        .with_active_profile_and_keys(|profile, keys| {
            // Deterministic contention checks: neither a profile switch nor a
            // key edit can publish while the combined reader is projecting.
            assert!(matches!(
                services.player_profiles.try_lock(),
                Err(std::sync::TryLockError::WouldBlock)
            ));
            assert!(matches!(
                services.key_configs.try_lock(),
                Err(std::sync::TryLockError::WouldBlock)
            ));
            assert_eq!(profile.id, 0);
            assert_eq!(
                keys.active.get_binding("ZoomIn").unwrap().primary_key,
                Some(KeyCode::F4)
            );
        })
        .unwrap();

    let before = context.host_snapshot().unwrap();
    let next = context
        .with_player_profiles_mut(|profiles| {
            let next = profiles.create_profile("Second".into(), DifficultyLevel::Hard);
            let profile = &mut profiles.profiles[next];
            profile.gameplay_config.control_tactical_units =
                !before.preferences.control_tactical_units();
            profile.graphic_config.native_refresh_presentation =
                !before.preferences.native_refresh_presentation();
            (next, profile.id)
        })
        .unwrap();
    context
        .with_key_configs_mut(|keys| {
            let keys = keys.entry_or_default(next.1);
            keys.active.set_binding("ZoomIn", Some(KeyCode::F6), None);
            keys.custom.set_binding("ZoomIn", Some(KeyCode::F7), None);
        })
        .unwrap();
    context
        .with_player_profiles_mut(|profiles| profiles.set_active(next.0))
        .unwrap();
    let after = context.host_snapshot().unwrap();
    assert_eq!(
        after
            .preferences
            .key_config()
            .get_binding("ZoomIn")
            .unwrap()
            .primary_key,
        Some(KeyCode::F6)
    );
    assert_eq!(
        after
            .preferences
            .custom_key_config()
            .get_binding("ZoomIn")
            .unwrap()
            .primary_key,
        Some(KeyCode::F7)
    );
    assert_ne!(
        after.preferences.control_tactical_units(),
        before.preferences.control_tactical_units()
    );
    assert_ne!(
        after.preferences.native_refresh_presentation(),
        before.preferences.native_refresh_presentation()
    );
    assert_eq!(
        before
            .preferences
            .key_config()
            .get_binding("ZoomIn")
            .unwrap()
            .primary_key,
        Some(KeyCode::F4)
    );
}

#[test]
fn combined_profile_reader_preserves_missing_profile_and_keys_errors() {
    assert!(
        ApplicationContext::default()
            .with_active_profile_and_keys(|_, _| panic!(
                "unavailable services must not invoke reader"
            ))
            .is_err()
    );
    let context = context(
        0,
        DifficultyLevel::Medium,
        KeyCode::F4,
        "missing-keys.marker",
    );
    let services = context.required_services().unwrap();
    services.key_configs.lock().unwrap().configs.clear();
    assert_eq!(
        context.host_snapshot().unwrap_err(),
        "ApplicationContext has no key config for active profile 0"
    );
    for index in [None, Some(usize::MAX)] {
        services.player_profiles.lock().unwrap().active_index = index;
        let error = context
            .with_active_profile_and_keys(|_, _| panic!("missing profile must not invoke reader"))
            .unwrap_err();
        assert_eq!(error, "ApplicationContext has no active player profile");
        assert_eq!(context.host_snapshot().unwrap_err(), error);
    }
    assert!(services.player_profiles.try_lock().is_ok());
    assert!(services.key_configs.try_lock().is_ok());
}

#[test]
#[cfg(all(panic = "unwind", not(target_arch = "wasm32")))]
#[ignore = "requires LLVM unwinding; run explicitly with robin_rs test codegen-backend=llvm"]
fn combined_profile_reader_preserves_poison_errors_and_releases_other_lock() {
    let context = context(
        0,
        DifficultyLevel::Medium,
        KeyCode::F4,
        "poison-keys.marker",
    );
    let services = context.required_services().unwrap();
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _keys = services.key_configs.lock().unwrap();
        panic!("poison key store for regression");
    }));
    assert_eq!(
        context.host_snapshot().unwrap_err(),
        "ApplicationContext key-config lock poisoned"
    );
    assert!(services.player_profiles.try_lock().is_ok());
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _profiles = services.player_profiles.lock().unwrap();
        panic!("poison profiles for regression");
    }));
    assert_eq!(
        context.host_snapshot().unwrap_err(),
        "ApplicationContext player-profile lock poisoned"
    );
}

#[test]
fn context_snapshots_release_locks_before_await() {
    let context = context(0, DifficultyLevel::Medium, KeyCode::F4, "lock.marker");

    pollster::block_on(async {
        let snapshot = context.host_snapshot().unwrap();
        std::future::ready(()).await;

        let services = context.required_services().unwrap();
        assert!(services.player_profiles.try_lock().is_ok());
        assert!(services.key_configs.try_lock().is_ok());
        assert_eq!(
            snapshot
                .preferences
                .key_config()
                .get_binding("ZoomIn")
                .unwrap()
                .primary_key,
            Some(KeyCode::F4)
        );
    });
}

#[test]
fn first_launch_replacement_installs_keys_host_and_save_target_for_new_id() {
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().to_string_lossy().into_owned();
    let mut profiles = PlayerProfileManager::new(root_path.clone());
    let placeholder = profiles.create_profile("Robin".into(), DifficultyLevel::Medium);
    profiles.set_active(placeholder);
    profiles.default_profiles = true;
    crate::player_profile_store::PlayerProfileStore::for_directory(&profiles.save_directory)
        .save(&profiles)
        .unwrap();

    let mut keys = KeyConfigStore::new(root_path.clone());
    keys.entry_or_default(0);
    keys.save().unwrap();
    let context = ApplicationContext::complete(
        crate::player_profile_store::PlayerProfileStore::for_directory(&root_path),
        engine_api::GlobalOptions::default(),
        profiles,
        keys,
        None,
    )
    .unwrap();
    context
        .grant_spellforge_content_trust(spellforge_trust_key(9), spellforge_trust_metadata(), 10)
        .unwrap();

    let new_id = context
        .complete_first_launch_profile(Some(("Marian".into(), DifficultyLevel::Hard)), (1280, 720))
        .unwrap();
    assert_eq!(new_id, 1);
    assert_eq!(context.active_profile_snapshot().unwrap().id, new_id);
    context
        .with_spellforge_trust(|trust| {
            assert!(trust.grants_for_profile(0).is_empty());
        })
        .unwrap();
    assert!(
        !SpellforgeTrustStore::load(&root_path)
            .unwrap()
            .is_trusted(0, spellforge_trust_key(9))
            .unwrap()
    );

    let (active_keys, custom_keys) = context.active_key_configs().unwrap();
    assert!(!active_keys.bindings.is_empty());
    assert!(!custom_keys.bindings.is_empty());
    context
        .with_key_configs(|store| {
            assert!(store.get(0).is_none());
            assert!(store.get(new_id).is_some());
        })
        .unwrap();

    let host = Host::new(context.clone().try_into().unwrap(), 1280.0, 720.0).unwrap();
    assert_eq!(
        host.frontend.preferences().key_config().key_type,
        active_keys.key_type
    );
    assert_eq!(
        host.frontend.preferences().custom_key_config().key_type,
        custom_keys.key_type
    );

    let mut saves = crate::savegame::SaveGameManager::open_for_context(&context).unwrap();
    let slot = saves.create("First save".into(), 7);
    let expected_save_root = root.path().join("Profile_001");
    assert_eq!(
        std::path::Path::new(saves.save_directory()),
        expected_save_root
    );
    assert!(saves.save_path(slot).starts_with(&expected_save_root));
    assert!(!std::path::Path::new(saves.save_directory()).ends_with("Profile_000"));
}

#[test]
fn unavailable_trust_persistence_blocks_first_launch_profile_replacement() {
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().to_string_lossy().into_owned();
    let mut profiles = PlayerProfileManager::new(root_path.clone());
    let placeholder = profiles.create_profile("Robin".into(), DifficultyLevel::Medium);
    profiles.set_active(placeholder);
    profiles.default_profiles = true;

    let mut keys = KeyConfigStore::new(root_path.clone());
    keys.entry_or_default(0);
    let context = ApplicationContext::complete(
        crate::player_profile_store::PlayerProfileStore::for_directory(&root_path),
        engine_api::GlobalOptions::default(),
        profiles,
        keys,
        None,
    )
    .unwrap();
    context
        .with_spellforge_trust_mut(|store| {
            *store =
                SpellforgeTrustStore::unavailable(root_path, "corrupt first-launch trust store");
        })
        .unwrap();

    let error = context
        .complete_first_launch_profile(Some(("Marian".into(), DifficultyLevel::Hard)), (1280, 720))
        .unwrap_err();
    assert!(error.contains("persistence is unavailable"), "{error}");
    context
        .with_player_profiles(|profiles| {
            assert_eq!(profiles.profile_count(), 1);
            assert_eq!(profiles.get_active().unwrap().id, 0);
            assert!(profiles.default_profiles);
        })
        .unwrap();
    context
        .with_key_configs(|keys| {
            assert!(keys.get(0).is_some());
            assert!(keys.get(1).is_none());
        })
        .unwrap();
}

#[test]
fn failed_profile_recovery_revocation_keeps_remote_trust_unavailable() {
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().to_string_lossy().into_owned();
    let mut profiles = PlayerProfileManager::new(root_path.clone());
    let profile = profiles.create_profile("Robin".into(), DifficultyLevel::Medium);
    profiles.set_active(profile);
    let mut keys = KeyConfigStore::new(root_path.clone());
    keys.entry_or_default(0);
    let context = ApplicationContext::complete(
        crate::player_profile_store::PlayerProfileStore::for_directory(&root_path),
        engine_api::GlobalOptions::default(),
        profiles,
        keys,
        None,
    )
    .unwrap();
    context
        .grant_spellforge_content_trust(spellforge_trust_key(5), spellforge_trust_metadata(), 10)
        .unwrap();

    let moved = root.path().with_extension("moved");
    std::fs::rename(root.path(), &moved).unwrap();
    std::fs::write(root.path(), b"blocks trust directory recreation").unwrap();
    let error = context
        .reset_spellforge_trust_after_profile_recovery()
        .unwrap_err();
    assert!(
        error.contains("create Spellforge trust directory"),
        "{error}"
    );
    let unavailable = context.active_spellforge_trust_grants().unwrap_err();
    assert!(
        unavailable.contains("persistence is unavailable"),
        "{unavailable}"
    );

    std::fs::remove_file(root.path()).unwrap();
    std::fs::rename(moved, root.path()).unwrap();
}
