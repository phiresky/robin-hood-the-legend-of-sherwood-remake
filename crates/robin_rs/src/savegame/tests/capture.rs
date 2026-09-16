use super::*;

#[test]
fn replay_store_rejects_save_io_before_capture_or_publication() {
    let (engine, _, profiles, mut host) = fresh_save_session("Replay viewer");
    let game = game_for_save(&profiles, 17);
    let mut manager = SaveGameManager::disabled();
    for error in [
        manager.save_index().unwrap_err(),
        manager.load_autosaves().unwrap_err().to_string(),
        manager
            .create_draft("Replay".into(), 17)
            .unwrap_err()
            .to_string(),
        manager.preflight_exact_slot(0).unwrap_err().to_string(),
        manager
            .write_restart_save(&mut host, &game, &engine, 17, Some(&profiles), None)
            .unwrap_err()
            .to_string(),
    ] {
        assert!(error.contains("save storage is disabled"), "{error}");
    }
    assert_eq!(manager.count(), 0);
    assert!(!manager.poll_background().unwrap());
    manager.finish_background().unwrap();
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn snapshots_are_pure_and_recording_boundaries_are_explicit() {
    use crate::replay_archive::MissionArchive;
    use crate::replay_recording::SharedReplayRecorder;
    use robin_engine::replay::ReplayRecorder;

    let root = tempfile::tempdir().unwrap();
    let (engine, _, profiles, host) = fresh_save_session("Explicit boundary");
    let game = game_for_save(&profiles, 17);
    let archive = MissionArchive::create(&root.path().join("replay")).unwrap();
    let chunk_path = archive.directory().join(archive.current_chunk());
    let recorder = ReplayRecorder::with_writer(
        archive.writer().unwrap(),
        "Mission_17".into(),
        game.mission_assets().unwrap().clone(),
        0,
        Default::default(),
        engine.campaign(),
    )
    .unwrap();
    let recorder = SharedReplayRecorder::archived(recorder, archive);
    let recording = host.application_context().replay_recording();
    recording.install_capture_recorder(Some(recorder.clone()));

    let mut save = GameSaveFile::capture_with_game(
        &engine,
        &host,
        &game,
        17,
        game.mission_assets().unwrap().clone(),
        "snapshot".into(),
        required_save_provenance(&host, &engine, 17, Some(&profiles)).unwrap(),
    )
    .unwrap();
    let mut restart =
        PreparedGameSave::capture_session_restart(&engine, &host, &game, save.header.clone())
            .unwrap();
    let restart_identity = restart.replay_identity().unwrap();
    let payload_identity = save.replay_identity().unwrap();
    assert_eq!(recorder.next_ordinal(), 0, "snapshot construction is pure");
    assert!(save.header.replay.is_none());
    assert!(restart.header.replay.is_none());

    recording.attach_save_boundary(&mut save).unwrap();
    assert_eq!(recorder.next_ordinal(), 1);
    assert_eq!(save.replay_identity().unwrap(), payload_identity);
    assert_eq!(recorder.captured_frame(payload_identity), Some((0, 0)));
    let link = save.header.replay.clone().unwrap();
    assert!(recording.attach_save_boundary(&mut save).is_err());
    assert_eq!(
        recorder.next_ordinal(),
        1,
        "relinking cannot append an event"
    );
    assert_eq!(save.header.replay, Some(link));

    // A publication failure cannot undo an already durable replay event.
    // Retrying publication of this exact payload needs no new event.
    assert!(save.write_to(root.path()).is_err());
    save.write_to(&root.path().join("save.json")).unwrap();
    assert_eq!(recorder.next_ordinal(), 1);

    restart.record_replay_boundary(&recording).unwrap();
    assert_eq!(recorder.next_ordinal(), 2);
    assert_eq!(restart.replay_identity().unwrap(), restart_identity);
    assert!(restart.header.replay.is_some());
    assert_ne!(restart_identity, payload_identity);
    assert!(restart.record_replay_boundary(&recording).is_err());
    assert_eq!(recorder.next_ordinal(), 2);

    // Background and transition saves can capture the completed campaign after
    // gameplay has sealed its recorder. Neither may extend the mission replay.
    recorder.clone().seal();
    let before = std::fs::read(&chunk_path).unwrap();
    for name in ["background", "transition"] {
        let mut completed = GameSaveFile::capture_with_game(
            &engine,
            &host,
            &game,
            17,
            game.mission_assets().unwrap().clone(),
            name.into(),
            required_save_provenance(&host, &engine, 17, Some(&profiles)).unwrap(),
        )
        .unwrap();
        recording.attach_save_boundary(&mut completed).unwrap();
        assert!(completed.header.replay.is_none());
        let path = root.path().join(format!("{name}.json"));
        completed.write_to(&path).unwrap();
        assert!(
            GameSaveFile::read_from(&path)
                .unwrap()
                .header
                .replay
                .is_none()
        );
        assert_eq!(recorder.next_ordinal(), 2);
    }
    let after = std::fs::read(&chunk_path).unwrap();
    assert_eq!(before, after, "post-seal saves must preserve replay bytes");
    let persisted = GameSaveFile::read_from(&root.path().join("save.json")).unwrap();

    // Loading a pre-completion save explicitly opens a continuation and enables
    // recording and save boundaries again.
    let boundary = recorder.restore(&persisted, &recording).unwrap();
    recorder.write_load_back(boundary.ordinal, boundary.marker_ordinal.unwrap(), false);
    recorder
        .commit_restore_boundary(
            boundary.timeline_frame,
            robin_engine::replay::state_hash(&persisted.engine),
            &crate::mission_replays::RecordingIndex::disabled(),
        )
        .unwrap();
    let mut resumed = persisted;
    resumed.header.replay = None;
    recording.attach_save_boundary(&mut resumed).unwrap();
    assert!(resumed.header.replay.is_some());
    assert_eq!(recorder.next_ordinal(), 4);
    recording.install_capture_recorder(None);
    drop(recorder);
    let reopened = MissionArchive::open(&root.path().join("replay")).unwrap();
    let (_, replay, _) = reopened.assembled_replay().unwrap();
    assert_eq!(replay.frame_count(), 4);
    let save_file::ReplaySaveIdentity::Payload(digest) = resumed.replay_identity().unwrap() else {
        panic!("published saves require a payload identity");
    };
    reopened
        .validate_link(resumed.header.replay.as_ref().unwrap(), &replay, digest)
        .unwrap();
}

#[test]
fn unarchived_save_boundary_keeps_the_runtime_marker_lane() {
    use crate::replay_recording::SharedReplayRecorder;
    use robin_engine::replay::ReplayRecorder;

    let (engine, _, profiles, host) = fresh_save_session("Unarchived boundary");
    let game = game_for_save(&profiles, 17);
    let recording = host.application_context().replay_recording();
    let mut save = GameSaveFile::capture_with_game(
        &engine,
        &host,
        &game,
        17,
        game.mission_assets().unwrap().clone(),
        "snapshot".into(),
        required_save_provenance(&host, &engine, 17, Some(&profiles)).unwrap(),
    )
    .unwrap();
    recording.attach_save_boundary(&mut save).unwrap();
    assert!(save.header.replay.is_none(), "recording is optional");

    let recorder = SharedReplayRecorder::from(
        ReplayRecorder::with_writer(
            Box::new(Vec::<u8>::new()),
            "Mission_17".into(),
            game.mission_assets().unwrap().clone(),
            0,
            Default::default(),
            engine.campaign(),
        )
        .unwrap(),
    );
    recording.install_capture_recorder(Some(recorder.clone()));
    recording.attach_save_boundary(&mut save).unwrap();
    assert!(save.header.replay.is_none());
    assert_eq!(recorder.next_ordinal(), 0);
    assert_eq!(
        recorder.captured_frame(save.replay_identity().unwrap()),
        None
    );

    // Invalid metadata is rejected before touching either recording lane.
    save.header.mission_id = 0;
    assert!(recording.attach_save_boundary(&mut save).is_err());
    assert_eq!(recorder.next_ordinal(), 0);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn loaded_continue_mirror_preserves_the_saved_replay_boundary() {
    use crate::replay_archive::MissionArchive;
    use crate::replay_recording::SharedReplayRecorder;
    use robin_engine::replay::ReplayRecorder;

    let root = tempfile::tempdir().unwrap();
    let (mut engine, _, profiles, mut host) = fresh_save_session("Loaded mirror");
    let game = game_for_save(&profiles, 17);
    let archive = MissionArchive::create(&root.path().join("replay")).unwrap();
    let recorder = ReplayRecorder::with_writer(
        archive.writer().unwrap(),
        "Mission_17".into(),
        game.mission_assets().unwrap().clone(),
        0,
        Default::default(),
        engine.campaign(),
    )
    .unwrap();
    let recorder = SharedReplayRecorder::archived(recorder, archive);
    host.application_context()
        .replay_recording()
        .install_capture_recorder(Some(recorder.clone()));
    let mut manager = SaveGameManager::new(root.path().to_str().unwrap().into());
    manager
        .write_quick_save(&mut host, &game, &engine, 17, Some(&profiles), None)
        .unwrap();
    let quick = manager
        .find_by_filename(save_file::special_slots::QUICK)
        .unwrap();
    let saved = GameSaveFile::read_from(&manager.save_path(quick)).unwrap();
    let identity = saved.replay_identity().unwrap();
    let link = saved.header.replay.clone().expect("saved replay link");
    let ordinal = recorder.next_ordinal();
    engine.test_set_frame_counter(117);
    manager
        .write_loaded_continue_background(saved, &profiles, None)
        .unwrap();
    manager.finish_background().unwrap();
    let target = manager
        .find_by_filename(save_file::special_slots::CONTINUE)
        .unwrap();
    let mirrored = GameSaveFile::read_from(&manager.save_path(target)).unwrap();
    assert_eq!(mirrored.replay_identity().unwrap(), identity);
    assert_eq!(mirrored.header.replay, Some(link));
    assert_eq!(
        recorder.next_ordinal(),
        ordinal,
        "a load mirror must not record the restored state on the old timeline"
    );
    assert_ne!(mirrored.engine.frame_counter(), engine.frame_counter());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn mirrored_saves_share_one_snapshot_and_one_replay_marker() {
    use crate::replay_archive::MissionArchive;
    use crate::replay_recording::SharedReplayRecorder;
    use robin_engine::replay::ReplayRecorder;
    for quick in [false, true] {
        for fail_mirror in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let (engine, _, profiles, mut host) = fresh_save_session("Single capture");
            let game = game_for_save(&profiles, 17);
            let archive = MissionArchive::create(&root.path().join("replay")).unwrap();
            let recorder = ReplayRecorder::with_writer(
                archive.writer().unwrap(),
                "Mission_17".into(),
                game.mission_assets().unwrap().clone(),
                0,
                Default::default(),
                engine.campaign(),
            )
            .unwrap();
            let recorder = SharedReplayRecorder::archived(recorder, archive);
            host.application_context()
                .replay_recording()
                .install_capture_recorder(Some(recorder.clone()));
            let mut manager = SaveGameManager::new(root.path().to_str().unwrap().into());
            let target = manager
                .ensure_special_slot(save_file::special_slots::CONTINUE, "Resume here")
                .unwrap();
            if fail_mirror {
                std::fs::create_dir(manager.save_path(target)).unwrap();
            }
            let source = if quick {
                assert_eq!(
                    manager
                        .write_quick_save_and_continue(
                            &mut host,
                            &game,
                            &engine,
                            17,
                            Some(&profiles),
                            None,
                        )
                        .unwrap()
                        .is_some(),
                    fail_mirror
                );
                manager
                    .find_by_filename(save_file::special_slots::QUICK)
                    .unwrap()
            } else {
                let source = manager.create("Checkpoint 雪".into(), 17);
                assert_eq!(
                    manager
                        .write_save_and_continue(
                            &mut host,
                            &game,
                            source,
                            &engine,
                            17,
                            Some(&profiles),
                            None,
                        )
                        .unwrap()
                        .is_some(),
                    fail_mirror
                );
                source
            };
            assert_eq!(
                recorder.next_ordinal(),
                1,
                "mirror must not capture another replay boundary"
            );
            let primary = GameSaveFile::read_from(&manager.save_path(source)).unwrap();
            assert!(primary.header.replay.is_some());
            if !fail_mirror {
                let mirror = GameSaveFile::read_from(&manager.save_path(target)).unwrap();
                assert_eq!(mirror.header.display_text, "Resume here");
                assert_eq!(primary.header.timestamp_unix, mirror.header.timestamp_unix);
                assert_eq!(primary.header.replay, mirror.header.replay);
                assert_eq!(
                    primary.replay_identity().unwrap(),
                    mirror.replay_identity().unwrap()
                );
                let mut primary_json = serde_json::to_value(primary).unwrap();
                let mut mirror_json = serde_json::to_value(mirror).unwrap();
                primary_json["header"]["display_text"] = serde_json::Value::Null;
                mirror_json["header"]["display_text"] = serde_json::Value::Null;
                assert_eq!(primary_json, mirror_json);
                let reopened = SaveGameManager::load_index(root.path().to_str().unwrap()).unwrap();
                for index in [source, target] {
                    let payload = reopened.preflight_exact_slot(index).unwrap();
                    reopened.validate_slot_identity(index, &payload).unwrap();
                }
            } else {
                assert!(
                    manager.save_path(source).is_file(),
                    "mirror failure must preserve primary"
                );
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn failed_primary_does_not_publish_continue_and_skipped_slots_do_not_mirror() {
    let root = tempfile::tempdir().unwrap();
    let (engine, _, profiles, mut host) = fresh_save_session("Mirror policy");
    let game = game_for_save(&profiles, 17);
    let mut manager = SaveGameManager::new(root.path().to_str().unwrap().into());
    let source = manager.create("Rejected".into(), 17);
    persistence::inject_failure(persistence::FailurePoint::BeforePayload);
    assert!(
        manager
            .write_save_and_continue(&mut host, &game, source, &engine, 17, Some(&profiles), None)
            .is_err()
    );
    assert!(
        manager
            .find_by_filename(save_file::special_slots::CONTINUE)
            .is_none()
    );
    assert!(!manager.save_path(source).exists());
    let restart = manager
        .ensure_special_slot(save_file::special_slots::RESTART, "Restart")
        .unwrap();
    assert!(
        manager
            .write_save_and_continue(
                &mut host,
                &game,
                restart,
                &engine,
                17,
                Some(&profiles),
                None
            )
            .unwrap()
            .is_none()
    );
    assert!(
        manager
            .find_by_filename(save_file::special_slots::CONTINUE)
            .is_none()
    );
}

#[test]
fn session_restart_restores_persisted_state_without_filesystem_or_json_identity() {
    let directory = tempfile::tempdir().unwrap();
    let blocked_root = directory.path().join("not-a-directory");
    std::fs::write(&blocked_root, b"filesystem writes must fail here").unwrap();
    let mut manager = SaveGameManager::new(blocked_root.to_string_lossy().into_owned());
    let (mut engine, assets, profiles, mut host) = fresh_save_session("Session Restart");
    let mut game = game_for_save(&profiles, 17);
    host.frontend.input.feedback.draw_hidden = true;
    game.persistent.campaign_map_displayed = true;
    engine.test_set_frame_counter(42);
    manager
        .write_session_restart(&host, &game, &engine, 17, Some(&profiles))
        .unwrap();
    assert!(manager.has_restart_save());
    let (index, prepared) = manager.preflight_restart_save().unwrap().unwrap();
    assert_eq!(manager.get(index).unwrap().player_name, "Session Restart");
    assert!(matches!(
        prepared.replay_identity().unwrap(),
        ReplaySaveIdentity::SessionRestart(_)
    ));
    assert_eq!(
        prepared.session_identity(),
        manager.restart_session_identity()
    );
    assert_eq!(
        manager
            .preflight_load(Some(index))
            .unwrap()
            .unwrap()
            .1
            .replay_identity()
            .unwrap(),
        prepared.replay_identity().unwrap()
    );
    assert!(!manager.save_path(index).exists());

    // A real disk round-trip provides the reference persisted projection.
    let disk_path = directory.path().join("reference.json");
    prepared.write_to(&disk_path).unwrap();
    let disk = GameSaveFile::read_from(&disk_path).unwrap();
    assert_eq!(
        GameSaveFile::replay_identity(&prepared).unwrap(),
        disk.replay_identity().unwrap()
    );
    let decoded: PreparedGameSave =
        serde_json::from_str(&serde_json::to_string(&prepared).unwrap()).unwrap();
    assert_eq!(decoded.session_identity(), None);
    assert_eq!(
        decoded.replay_identity().unwrap(),
        disk.replay_identity().unwrap()
    );

    engine.test_set_frame_counter(99);
    host.frontend.input.feedback.draw_hidden = false;
    game.persistent.campaign_map_displayed = false;
    let mut disk_engine = engine.clone();
    let mut disk_host = Host::scratch(800.0, 600.0);
    let mut disk_game = Game::default();
    disk.apply_to_with_game(&mut disk_engine, &mut disk_host, &mut disk_game, &assets)
        .unwrap();
    prepared
        .clone()
        .apply_to_with_game(&mut engine, &mut host, &mut game, &assets)
        .unwrap();
    assert!(host.frontend.input.feedback.draw_hidden);
    assert!(game.persistent.campaign_map_displayed);
    assert_eq!(
        crate::save_file::GameRuntimeSnapshot::identity_of_live(&engine, &host, &game).unwrap(),
        crate::save_file::GameRuntimeSnapshot::identity_of_live(
            &disk_engine,
            &disk_host,
            &disk_game
        )
        .unwrap()
    );

    // Index serialization and new profile managers cannot resurrect memory
    // checkpoints or their process-local identity authority.
    let index_data = SaveIndex {
        saves: manager.catalog.iter().cloned().collect(),
        next_id: manager.next_id,
        save_directory: manager.save_directory.clone(),
    };
    let index_data: SaveIndex =
        serde_json::from_str(&serde_json::to_string(&index_data).unwrap()).unwrap();
    let mut reopened = SaveGameManager::new(blocked_root.to_str().unwrap().into());
    reopened.next_id = index_data.next_id;
    for slot in index_data.saves {
        reopened.insert_test_slot(slot, SlotState::Published);
    }
    assert!(!reopened.has_restart_save());
    assert_eq!(reopened.restart_session_identity(), None);
    let other_profile = SaveGameManager::new(
        directory
            .path()
            .join("other-profile")
            .to_string_lossy()
            .into_owned(),
    );
    assert!(!other_profile.has_restart_save());

    let old_identity = prepared.replay_identity().unwrap();
    manager
        .write_session_restart(&host, &game, &engine, 17, Some(&profiles))
        .unwrap();
    assert_ne!(manager.restart_session_identity(), Some(old_identity));
    assert_eq!(prepared.replay_identity().unwrap(), old_identity);
    manager.remove(index).unwrap();
    assert!(!manager.has_restart_save());
}

#[test]
fn failed_session_restart_capture_invalidates_previous_checkpoint() {
    let directory = tempfile::tempdir().unwrap();
    let mut manager = SaveGameManager::new(directory.path().to_string_lossy().into_owned());
    let (engine, _, profiles, host) = fresh_save_session("Failed Session Restart");
    let game = game_for_save(&profiles, 17);
    manager
        .write_session_restart(&host, &game, &engine, 17, Some(&profiles))
        .unwrap();
    let missing_profile_host = Host::scratch(800.0, 600.0);
    assert!(
        manager
            .write_session_restart(&missing_profile_host, &game, &engine, 17, Some(&profiles))
            .is_err()
    );
    assert!(!manager.has_restart_save());
    assert!(manager.preflight_restart_save().unwrap().is_none());
    assert_eq!(manager.restart_session_identity(), None);
}

#[test]
fn save_provenance_owns_identity_and_requires_an_active_profile() {
    let (engine, _, profiles, host) = fresh_save_session("Robin 雪");
    let captured = required_save_provenance(&host, &engine, 17, Some(&profiles)).unwrap();
    assert_eq!(
        captured,
        SaveProvenance::new("Mission 17".into(), 0, "Robin 雪".into()).unwrap()
    );
    host.application_context()
        .with_player_profiles_mut(|players| {
            players.get_active_mut().unwrap().name = "Renamed Robin".into();
        })
        .unwrap();
    assert_eq!(captured.player_name, "Robin 雪");
    assert_eq!(
        required_save_provenance(&host, &engine, 17, Some(&profiles))
            .unwrap()
            .player_name,
        "Renamed Robin"
    );
    let scratch = Host::scratch(800.0, 600.0);
    let expected = scratch
        .application_context()
        .active_profile_snapshot()
        .unwrap_err();
    let error = required_save_provenance(&scratch, &engine, 17, Some(&profiles)).unwrap_err();
    assert_eq!(error.to_string(), "save requires an active player profile");
    assert!(format!("{error:#}").contains(&expected));
}

#[test]
fn engine_round_trip_via_manager() {
    use tempfile::tempdir;

    let tmp = tempdir().unwrap();
    let mut mgr = SaveGameManager::new(tmp.path().to_string_lossy().into_owned());

    // Build a live engine with some distinctive state.
    let (mut engine, assets, mut profiles, mut host) = fresh_save_session("Alice");
    let game = game_for_save(&profiles, 17);
    engine.test_set_frame_counter(42);
    engine.test_set_engine_scalars(0xAA55_AA55, 2.0, 0, false, false, Vec::new());

    // Write to a manual slot.
    let idx = mgr.create("Slot A".into(), 17);
    mgr.write_save_from_engine(&mut host, &game, idx, &engine, 17, Some(&profiles), None)
        .unwrap();
    assert!(mgr.slot_file_exists(idx));
    assert_eq!(mgr.slot_mission_id(idx), Some(17));
    let decoded = mgr.preflight_exact_slot(idx).unwrap();
    mgr.validate_slot_identity(idx, &decoded).unwrap();
    assert_eq!(
        decoded.header.provenance,
        SaveProvenance::new("Mission 17".into(), 0, "Alice".into()).unwrap()
    );
    assert_eq!(mgr.catalog[idx].mission_name, "Mission 17");
    assert_eq!(mgr.catalog[idx].player_profile_id, Some(0));
    assert_eq!(mgr.catalog[idx].player_name, "Alice");
    profiles.missions[2].mission_name = "Mission 17 (renamed)".into();
    assert_eq!(mgr.catalog[idx].mission_name, "Mission 17");
    assert_eq!(decoded.header.provenance.mission_name, "Mission 17");
    mgr.catalog[idx].mission_id = 99;
    assert!(
        mgr.validate_slot_identity(idx, &decoded)
            .unwrap_err()
            .to_string()
            .contains("metadata does not match decoded payload")
    );
    mgr.catalog[idx].mission_id = 17;
    mgr.catalog[idx].player_name = "Mallory".into();
    assert!(
        mgr.validate_slot_identity(idx, &decoded)
            .unwrap_err()
            .to_string()
            .contains("provenance does not match")
    );
    mgr.catalog[idx].player_name = "Alice".into();

    host.application_context()
        .with_player_profiles_mut(|players| {
            players.get_active_mut().unwrap().name = "Renamed Alice".into();
        })
        .unwrap();

    // Write a Continue auto-save.
    mgr.write_continue_save(&mut host, &game, &engine, 17, Some(&profiles), None)
        .unwrap();
    let continue_idx = mgr
        .find_by_filename(special_slots::CONTINUE)
        .expect("continue slot should exist");
    assert!(mgr.slot_file_exists(continue_idx));
    assert_eq!(mgr.catalog[idx].player_name, "Alice");
    assert_eq!(mgr.catalog[continue_idx].player_name, "Renamed Alice");
    assert_eq!(
        mgr.catalog[continue_idx].mission_name,
        "Mission 17 (renamed)"
    );

    // find_load_target should prefer the explicit slot when supplied,
    // otherwise fall back to Continue.
    assert_eq!(mgr.find_load_target(Some(idx)), Some(idx));
    assert_eq!(mgr.find_load_target(None), Some(continue_idx));

    // Load into a fresh engine.
    let mut engine2 = fresh_engine().0;
    let mut host2 = Host::scratch(800.0, 600.0);
    let mut game2 = Game::default();
    mgr.load_save_into_engine(idx, &mut engine2, &mut host2, &mut game2, &assets)
        .unwrap();
    assert_eq!(engine2.frame_counter(), 42);
}

#[test]
fn multiplayer_diagnostic_tag_is_written_to_payload_and_index() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = SaveGameManager::new(tmp.path().to_string_lossy().into_owned());
    let (engine, _assets, profiles, mut host) = fresh_save_session("Alice");
    let game = game_for_save(&profiles, 17);
    let slot = manager.create("Network diagnostic".into(), 17);

    manager
        .write_multiplayer_diagnostic_from_engine(
            &mut host,
            &game,
            slot,
            &engine,
            17,
            Some(&profiles),
            None,
        )
        .unwrap();
    assert!(manager.get(slot).unwrap().multiplayer_diagnostic);
    assert!(
        manager
            .preflight_exact_slot(slot)
            .unwrap()
            .header
            .multiplayer_diagnostic
    );

    manager
        .write_save_from_engine(&mut host, &game, slot, &engine, 17, Some(&profiles), None)
        .unwrap();
    assert!(!manager.get(slot).unwrap().multiplayer_diagnostic);
    assert!(
        !manager
            .preflight_exact_slot(slot)
            .unwrap()
            .header
            .multiplayer_diagnostic
    );
}

#[test]
fn missing_explicit_slot_never_falls_back_to_continue() {
    let tmp = tempfile::tempdir().unwrap();
    let mut mgr = SaveGameManager::new(tmp.path().to_string_lossy().into_owned());
    let (engine, _assets, profiles, mut host) = fresh_save_session("Alice");
    let game = game_for_save(&profiles, 1);
    mgr.write_continue_save(&mut host, &game, &engine, 1, Some(&profiles), None)
        .unwrap();
    let missing = mgr.create("Missing explicit slot".into(), 1);

    assert_eq!(mgr.find_load_target(Some(missing)), None);
    assert!(mgr.preflight_load(Some(missing)).unwrap().is_none());
    assert_eq!(
        mgr.find_load_target(None),
        mgr.find_by_filename(special_slots::CONTINUE)
    );
}

#[test]
fn quick_save_rotates_previous() {
    use tempfile::tempdir;

    let tmp = tempdir().unwrap();
    let mut mgr = SaveGameManager::new(tmp.path().to_string_lossy().into_owned());

    let (mut engine, assets, profiles, mut host) = fresh_save_session("Alice");
    let game = game_for_save(&profiles, 3);

    engine.test_set_frame_counter(1);
    mgr.write_quick_save(&mut host, &game, &engine, 3, Some(&profiles), None)
        .unwrap();
    engine.test_set_frame_counter(2);
    mgr.write_quick_save(&mut host, &game, &engine, 3, Some(&profiles), None)
        .unwrap();

    let quick_idx = mgr.find_by_filename(special_slots::QUICK).unwrap();
    let ex_idx = mgr.find_by_filename(special_slots::EX_QUICK).unwrap();
    assert!(mgr.slot_file_exists(quick_idx));
    assert!(mgr.slot_file_exists(ex_idx));

    let mut engine_q = fresh_engine().0;
    let mut host_q = Host::scratch(800.0, 600.0);
    let mut game_q = Game::default();
    mgr.load_save_into_engine(quick_idx, &mut engine_q, &mut host_q, &mut game_q, &assets)
        .unwrap();
    assert_eq!(engine_q.frame_counter(), 2);

    let mut engine_e = fresh_engine().0;
    let mut host_e = Host::scratch(800.0, 600.0);
    let mut game_e = Game::default();
    mgr.load_save_into_engine(ex_idx, &mut engine_e, &mut host_e, &mut game_e, &assets)
        .unwrap();
    assert_eq!(engine_e.frame_counter(), 1);
    assert_eq!(mgr.catalog[quick_idx].player_name, "Alice");
    assert_eq!(mgr.catalog[ex_idx].player_name, "Alice");
}

#[test]
fn quick_save_recovers_payloads_after_index_publication_failure() {
    let tmp = tempfile::tempdir().unwrap();
    let directory = tmp.path().to_string_lossy().into_owned();
    let mut manager = SaveGameManager::new(directory.clone());
    let (mut engine, _, profiles, mut host) = fresh_save_session("Recovery");
    let game = game_for_save(&profiles, 3);
    engine.test_set_frame_counter(1);
    manager
        .write_quick_save(&mut host, &game, &engine, 3, Some(&profiles), None)
        .unwrap();
    let index_path = tmp.path().join("saves.json");
    std::fs::rename(&index_path, tmp.path().join("old-index.json")).unwrap();
    std::fs::create_dir(&index_path).unwrap();
    engine.test_set_frame_counter(2);
    assert!(
        manager
            .write_quick_save(&mut host, &game, &engine, 3, Some(&profiles), None)
            .is_err()
    );
    std::fs::remove_dir(&index_path).unwrap();
    let mut recovered = SaveGameManager::load_index(&directory).unwrap();
    for (name, frame) in [(special_slots::QUICK, 2), (special_slots::EX_QUICK, 1)] {
        let index = recovered.find_by_filename(name).expect("recovered slot");
        let save = GameSaveFile::read_from(&recovered.save_path(index)).unwrap();
        assert_eq!(save.engine.frame_counter(), frame);
        assert_eq!(recovered.catalog[index].player_name, "Recovery");
    }
    assert!(!recovered.quick_recovery_path().exists());
    let quick = recovered.find_by_filename(special_slots::QUICK).unwrap();
    recovered.get_mut(quick).unwrap().text = "Edited after recovery".to_owned();
    recovered.save_index().unwrap();
    let reopened = SaveGameManager::load_index(&directory).unwrap();
    let quick = reopened.find_by_filename(special_slots::QUICK).unwrap();
    assert_eq!(reopened.get(quick).unwrap().text, "Edited after recovery");
}

#[test]
fn quick_save_preparation_failure_does_not_rotate_existing_payloads() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = SaveGameManager::new(tmp.path().to_string_lossy().into_owned());
    let (engine, _, profiles, mut host) = fresh_save_session("Preparation");
    let game = game_for_save(&profiles, 3);
    manager
        .write_quick_save(&mut host, &game, &engine, 3, Some(&profiles), None)
        .unwrap();
    let quick = manager.save_path(manager.find_by_filename(special_slots::QUICK).unwrap());
    let before = std::fs::read(&quick).unwrap();
    assert!(
        manager
            .write_quick_save(&mut host, &game, &engine, 3, None, None)
            .is_err()
    );
    assert_eq!(std::fs::read(&quick).unwrap(), before);
    assert!(!tmp.path().join("ExQuickSave.json").exists());
}

#[test]
fn missing_rotation_source_preserves_previous_payload() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = SaveGameManager::new(tmp.path().to_string_lossy().into_owned());
    let quick = manager
        .ensure_special_slot(special_slots::QUICK, "Quick Save")
        .unwrap();
    let previous = manager
        .ensure_special_slot(special_slots::EX_QUICK, "Previous Quick Save")
        .unwrap();
    let previous_path = manager.save_path(previous);
    std::fs::write(&previous_path, b"previous save payload").unwrap();
    assert!(manager.copy_files(quick, previous).is_err());
    assert_eq!(
        std::fs::read(previous_path).unwrap(),
        b"previous save payload"
    );
}

#[test]
fn save_write_rejects_missing_profiles_or_active_player() {
    let tmp = tempfile::tempdir().unwrap();
    let mut mgr = SaveGameManager::new(tmp.path().to_string_lossy().into_owned());
    let (engine, _assets, profiles, _host) = fresh_save_session("Alice");
    let mut scratch_host = Host::scratch(800.0, 600.0);
    let game = game_for_save(&profiles, 1);
    let slot = mgr.create("Strict metadata".into(), 1);

    let missing_profiles = mgr
        .write_save_from_engine(&mut scratch_host, &game, slot, &engine, 1, None, None)
        .unwrap_err();
    assert!(
        format!("{missing_profiles:#}").contains("active mission profile table"),
        "{missing_profiles:#}"
    );

    let missing_player = mgr
        .write_save_from_engine(
            &mut scratch_host,
            &game,
            slot,
            &engine,
            1,
            Some(&profiles),
            None,
        )
        .unwrap_err();
    assert!(
        format!("{missing_player:#}").contains("active player profile"),
        "{missing_player:#}"
    );

    let (_engine, _assets, profiles, mut host) = fresh_save_session("Alice");
    let missing_slot = mgr
        .write_save_from_engine(
            &mut host,
            &game,
            usize::MAX,
            &engine,
            1,
            Some(&profiles),
            None,
        )
        .unwrap_err();
    assert!(
        format!("{missing_slot:#}").contains("missing save slot"),
        "{missing_slot:#}"
    );
}

#[test]
fn timestamp_sort_is_numeric_and_puts_invalid_legacy_values_last() {
    let mut mgr = SaveGameManager::new("/tmp/test_saves".into());
    for (name, timestamp) in [("Ten", "10"), ("Two", "2"), ("Legacy", "")] {
        let slot = mgr.create(name.into(), 1);
        mgr.catalog[slot].timestamp = timestamp.into();
    }
    mgr.sort_by_time();
    assert_eq!(
        mgr.catalog
            .iter()
            .map(|save| save.text.as_str())
            .collect::<Vec<_>>(),
        ["Two", "Ten", "Legacy"]
    );
}

#[test]
fn native_index_without_player_metadata_is_rejected() {
    let json = serde_json::json!({
        "saves": [{
            "text": "Legacy",
            "filename": "Savegame_000",
            "mission_id": 1,
            "version": save_file::SAVE_FORMAT_VERSION,
            "timestamp": "123",
            "special": null,
            "mission_name": "Mission 1"
        }],
        "save_directory": "/tmp/test_saves",
        "next_id": 1
    });
    let error = serde_json::from_value::<SaveIndex>(json).unwrap_err();
    assert!(error.to_string().contains("missing field"));
}

#[test]
fn per_profile_save_managers_are_isolated() {
    // Gap 1 test: two profiles using the same root save dir should
    // each get their own `Profile_NNN/` subdirectory so their slot
    // lists never collide.
    use crate::save_file::{save_directory_for_profile, special_slots};
    use tempfile::tempdir;

    let root = tempdir().unwrap();

    // Build two per-profile managers rooted at Profile_000 / Profile_001
    // (independent of the global PlayerProfileManager to keep the test
    // hermetic).
    let p0_dir = root.path().join("Profile_000");
    let p1_dir = root.path().join("Profile_001");
    // Matches the `Profile_NNN` layout `save_directory_for_profile` uses.
    assert!(save_directory_for_profile(0).ends_with("Profile_000"));
    assert!(save_directory_for_profile(42).ends_with("Profile_042"));
    let mut mgr0 = SaveGameManager::new(p0_dir.to_string_lossy().into_owned());
    let mut mgr1 = SaveGameManager::new(p1_dir.to_string_lossy().into_owned());

    let (mut engine, assets, profiles, mut host) = fresh_save_session("Alice");
    let game = game_for_save(&profiles, 1);

    // Profile 0 saves frame=100 into QuickSave.
    engine.test_set_frame_counter(100);
    mgr0.write_quick_save(&mut host, &game, &engine, 1, Some(&profiles), None)
        .unwrap();
    let q0 = mgr0.find_by_filename(special_slots::QUICK).unwrap();
    let path0 = mgr0.save_path(q0);
    assert!(
        path0.starts_with(&p0_dir),
        "p0 save must be under Profile_000"
    );

    // Profile 1 saves frame=200 into its own QuickSave.
    engine.test_set_frame_counter(200);
    mgr1.write_quick_save(&mut host, &game, &engine, 1, Some(&profiles), None)
        .unwrap();
    let q1 = mgr1.find_by_filename(special_slots::QUICK).unwrap();
    let path1 = mgr1.save_path(q1);
    assert!(
        path1.starts_with(&p1_dir),
        "p1 save must be under Profile_001"
    );
    assert_ne!(path0, path1, "profiles must use distinct save files");

    // Each profile loads its own snapshot back independently.
    let mut engine_a = fresh_engine().0;
    let mut host_a = Host::scratch(800.0, 600.0);
    let mut game_a = Game::default();
    mgr0.load_save_into_engine(q0, &mut engine_a, &mut host_a, &mut game_a, &assets)
        .unwrap();
    assert_eq!(engine_a.frame_counter(), 100);

    let mut engine_b = fresh_engine().0;
    let mut host_b = Host::scratch(800.0, 600.0);
    let mut game_b = Game::default();
    mgr1.load_save_into_engine(q1, &mut engine_b, &mut host_b, &mut game_b, &assets)
        .unwrap();
    assert_eq!(engine_b.frame_counter(), 200);
}

#[test]
fn remove_by_filename() {
    let root = tempfile::tempdir().unwrap();
    let mut mgr = SaveGameManager::new(root.path().to_str().unwrap().into());
    mgr.create("A".into(), 1);
    mgr.create_with_filename("Continue".into(), "Continue".into(), 0);
    assert_eq!(mgr.count(), 2);
    mgr.remove_by_filename("Continue").unwrap();
    assert_eq!(mgr.count(), 1);
    assert_eq!(mgr.catalog[0].filename, "Savegame_000");
}

#[test]
fn manual_remove_apis_refuse_autosaves() {
    let mut mgr = SaveGameManager::new("/tmp/test_saves".into());
    mgr.insert_test_slot(
        SaveGame::new("Autosave_1_0000".into(), "Mission".into(), 1),
        SlotState::Published,
    );
    assert!(mgr.remove(0).is_err());
    assert!(mgr.remove_by_filename("Autosave_1_0000").is_err());
    assert_eq!(mgr.count(), 1);
}
