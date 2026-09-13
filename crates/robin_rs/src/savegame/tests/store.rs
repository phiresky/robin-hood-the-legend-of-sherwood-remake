use super::*;

#[test]
fn play_resumes_newer_mission_autosave_instead_of_stale_continue() {
    let mut manager = SaveGameManager::new(String::new());
    assert_eq!(manager.find_resume_target(), None);
    let mut first = published_slot("Continue");
    first.timestamp = "100".into();
    let mut second = published_slot("Autosave_200_0003");
    second.timestamp = "200".into();
    second.mission_id = 2;
    let mut restart = published_slot("Restart");
    restart.timestamp = "300".into();
    for save in [first, second, restart] {
        manager.insert_test_slot(save, SlotState::Published);
    }
    assert_eq!(manager.find_resume_target(), Some(1));
    assert_eq!(manager.slot_mission_id(1), Some(2));
    manager.catalog[0].timestamp = "400".into();
    assert_eq!(manager.find_resume_target(), Some(0));
    manager.catalog[1].timestamp = "400".into();
    assert_eq!(manager.find_resume_target(), Some(0));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn synchronous_publication_failure_matrix_recovers_only_completed_payloads() {
    use persistence::FailurePoint::*;
    let (engine, _, profiles, mut host) = fresh_save_session("Transaction player");
    let game = game_for_save(&profiles, 17);
    for overwrite in [false, true] {
        for diagnostic in [false, true] {
            for stage in [
                BeforeReceipt,
                BeforePayload,
                AfterPayload,
                BeforeIndex,
                AfterIndex,
            ] {
                let root = tempfile::tempdir().unwrap();
                let mut manager = indexed_store(root.path(), &["Unrelated"]);
                let untouched = manager.get(0).unwrap().clone();
                std::fs::write(manager.save_path(0), b"unrelated payload").unwrap();
                let handle = manager.create_draft("Transaction".into(), 17).unwrap();
                let index = manager.resolve_handle(&handle).unwrap();
                if overwrite {
                    manager
                        .write_save_from_engine(
                            &mut host,
                            &game,
                            index,
                            &engine,
                            17,
                            Some(&profiles),
                            None,
                        )
                        .unwrap();
                }
                let old = manager.get(index).unwrap().clone();
                let old_payload = std::fs::read(manager.save_path(index)).ok();
                // Distinguish the replacement from the previous successful
                // write even when wall-clock timestamps have not advanced.
                manager
                    .rename_slot(&handle, format!("Replacement {stage:?}"))
                    .unwrap();
                persistence::inject_failure(stage);
                let error = manager
                    .write_save_from_engine_with_diagnostic(
                        &mut host,
                        &game,
                        index,
                        &engine,
                        17,
                        Some(&profiles),
                        None,
                        diagnostic,
                    )
                    .unwrap_err();
                assert!(
                    format!("{error:#}").contains("injected"),
                    "{stage:?}: {error:#}"
                );
                let landed = matches!(stage, AfterPayload | BeforeIndex | AfterIndex);
                if landed {
                    assert!(manager.owned_recovery_path().exists());
                    assert!(manager.create_draft("Blocked".into(), 17).is_err());
                } else {
                    assert!(!manager.owned_recovery_path().exists());
                    assert_eq!(std::fs::read(manager.save_path(index)).ok(), old_payload);
                    assert_eq!(
                        manager.slot_state(handle.name()).unwrap(),
                        if overwrite {
                            SlotState::Published
                        } else {
                            SlotState::Draft
                        }
                    );
                }
                let recovered = SaveGameManager::load_index(root.path().to_str().unwrap()).unwrap();
                assert_eq!(
                    recovered
                        .get(recovered.find_by_filename("Unrelated").unwrap())
                        .unwrap(),
                    &untouched
                );
                assert_eq!(
                    std::fs::read(root.path().join("Unrelated.json")).unwrap(),
                    b"unrelated payload"
                );
                let recovered_index = recovered.find_by_filename(handle.name().as_str());
                if landed {
                    let recovered_index = recovered_index.unwrap();
                    let payload =
                        GameSaveFile::read_from(&recovered.save_path(recovered_index)).unwrap();
                    recovered
                        .validate_slot_identity(recovered_index, &payload)
                        .unwrap();
                    assert_eq!(payload.header.multiplayer_diagnostic, diagnostic);
                    assert_eq!(
                        recovered
                            .get(recovered_index)
                            .unwrap()
                            .multiplayer_diagnostic,
                        diagnostic
                    );
                    assert_eq!(
                        recovered.get(recovered_index).unwrap().text,
                        format!("Replacement {stage:?}")
                    );
                } else if overwrite {
                    assert_eq!(recovered.get(recovered_index.unwrap()).unwrap(), &old);
                } else {
                    assert!(recovered_index.is_none());
                }
                assert!(!recovered.owned_recovery_path().exists());
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn synchronous_commit_evidence_requires_durable_index_and_preserves_identity() {
    let root = tempfile::tempdir().unwrap();
    let mut manager = indexed_store(root.path(), &["Unrelated"]);
    let (engine, _, profiles, mut host) = fresh_save_session("Committed player");
    let game = game_for_save(&profiles, 17);
    let handle = manager.create_draft("New".into(), 17).unwrap();
    let index = manager.resolve_handle(&handle).unwrap();
    for diagnostic in [false, true] {
        let committed = if diagnostic {
            manager.write_multiplayer_diagnostic_from_engine(
                &mut host,
                &game,
                index,
                &engine,
                17,
                Some(&profiles),
                None,
            )
        } else {
            manager.write_save_from_engine(
                &mut host,
                &game,
                index,
                &engine,
                17,
                Some(&profiles),
                None,
            )
        }
        .unwrap();
        assert_eq!(committed.slot(), &handle);
        let encoded = serde_json::to_string(&committed).unwrap();
        assert!(
            serde_json::from_str::<CommittedSave>(&encoded)
                .unwrap_err()
                .to_string()
                .contains("save commit evidence is process-local")
        );
        assert_eq!(
            *committed.digest(),
            <[u8; 32]>::from(Sha256::digest(
                std::fs::read(manager.save_path(index)).unwrap()
            ))
        );
        assert_eq!(
            manager.slot_state(handle.name()).unwrap(),
            SlotState::Published
        );
        assert!(!manager.owned_recovery_path().exists());
        let reopened = SaveGameManager::load_index(root.path().to_str().unwrap()).unwrap();
        assert_eq!(reopened.count(), 2);
        assert_eq!(
            reopened
                .get(reopened.find_by_filename(handle.name().as_str()).unwrap())
                .unwrap()
                .multiplayer_diagnostic,
            diagnostic
        );
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn synchronous_new_target_collision_never_promotes_identical_orphan_bytes() {
    let root = tempfile::tempdir().unwrap();
    let mut manager = indexed_store(root.path(), &["Unrelated"]);
    let handle = manager.create_draft("New".into(), 1).unwrap();
    let index = manager.resolve_handle(&handle).unwrap();
    std::fs::write(manager.save_path(index), b"identical payload").unwrap();
    let metadata = published_slot(handle.name().as_str());
    assert!(
        manager
            .commit_synchronous(index, metadata, b"identical payload", None)
            .is_err()
    );
    assert_eq!(manager.slot_state(handle.name()).unwrap(), SlotState::Draft);
    assert!(!manager.owned_recovery_path().exists());
    assert!(manager.operation_error.is_none());
    manager.remove(index).unwrap();
    assert_eq!(
        std::fs::read(root.path().join(format!("{}.json", handle.name().as_str()))).unwrap(),
        b"identical payload"
    );
    let reopened = SaveGameManager::load_index(root.path().to_str().unwrap()).unwrap();
    assert_eq!(reopened.count(), 1);
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen_test::wasm_bindgen_test]
fn browser_sync_publication_rejects_without_changing_session_or_autosave_catalog() {
    let mut manager = SaveGameManager::new("browser-memory".into());
    manager.insert_test_slot(published_slot("Restart"), SlotState::Session);
    manager.insert_test_slot(published_slot("Autosave_1_0000"), SlotState::Published);
    let handle = manager.create_draft("Manual".into(), 17).unwrap();
    let before = manager.saves().cloned().collect::<Vec<_>>();
    let index = manager.resolve_handle(&handle).unwrap();
    let error = manager
        .commit_synchronous(
            index,
            published_slot(handle.name().as_str()),
            b"payload",
            None,
        )
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("browser manual-save persistence is unavailable")
    );
    assert!(manager.saves().eq(before.iter()));
    assert_eq!(
        manager
            .slot_state(&SlotName::new("Restart").unwrap())
            .unwrap(),
        SlotState::Session
    );
    assert!(manager.operation_error.is_none());
}

#[test]
fn stable_handles_reject_retired_generations_other_owners_and_serde() {
    let root = tempfile::tempdir().unwrap();
    let mut manager = SaveGameManager::new(root.path().to_str().unwrap().into());
    let first = manager.create_draft("Draft".into(), 17).unwrap();
    assert_eq!(manager.slot_state(first.name()).unwrap(), SlotState::Draft);
    let decoded: SlotHandle =
        serde_json::from_str(&serde_json::to_string(&first).unwrap()).unwrap();
    assert!(manager.resolve_handle(&decoded).is_err());
    let mut other = SaveGameManager::new(root.path().to_str().unwrap().into());
    other.create_draft("Other owner".into(), 17).unwrap();
    assert!(other.resolve_handle(&first).is_err());
    let index = manager.resolve_handle(&first).unwrap();
    manager.remove(index).unwrap();
    let replacement = manager
        .allocate_named_draft(first.name().as_str().into(), "Replacement".into(), 17)
        .unwrap();
    assert!(manager.resolve_handle(&first).is_err());
    assert_ne!(manager.slot_handle(replacement).unwrap(), first);
}

#[test]
fn explicit_draft_state_cannot_be_promoted_by_filling_timestamp() {
    let root = tempfile::tempdir().unwrap();
    let mut manager = SaveGameManager::new(root.path().to_str().unwrap().into());
    let handle = manager.create_draft("Draft".into(), 17).unwrap();
    let index = manager.resolve_handle(&handle).unwrap();
    manager.catalog[index] = published_slot(handle.name().as_str());
    assert!(!manager.slot_file_exists(index));
    manager.save_index().unwrap();
    assert!(
        SaveGameManager::load_index(root.path().to_str().unwrap())
            .unwrap()
            .saves()
            .len()
            == 0
    );
    std::fs::write(manager.save_path(index), b"unrelated writer").unwrap();
    assert!(!manager.slot_file_exists(index));
    manager.remove(index).unwrap();
    assert_eq!(
        std::fs::read(root.path().join(format!("{}.json", handle.name().as_str()))).unwrap(),
        b"unrelated writer"
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn bootstrap_completion_fence_is_nonblocking_and_failure_stays_failed() {
    let root = tempfile::tempdir().unwrap();
    let mut manager = SaveGameManager::new(root.path().to_str().unwrap().into());
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    manager
        .operations
        .start(SlotName::new("Restart").unwrap(), move || {
            release_rx.recv_timeout(std::time::Duration::from_secs(10))?;
            anyhow::bail!("injected Restart publication failure")
        })
        .unwrap();
    assert_eq!(
        manager.try_finish_background().unwrap(),
        SaveWriteStatus::Queued
    );
    release_tx.send(()).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !manager.operations.is_finished() {
        assert!(
            std::time::Instant::now() < deadline,
            "save worker did not finish"
        );
        std::thread::yield_now();
    }
    assert!(
        manager
            .try_finish_background()
            .unwrap_err()
            .to_string()
            .contains("injected Restart")
    );
    assert!(manager.poll_background().is_err());
    assert!(!manager.poll_background().unwrap());
    assert!(manager.try_finish_background().is_err());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn queued_special_save_publishes_payload_then_owned_metadata_and_index() {
    let root = tempfile::tempdir().unwrap();
    let mut manager = SaveGameManager::new(root.path().to_str().unwrap().into());
    let (engine, _, profiles, mut host) = fresh_save_session("Owned completion");
    let game = game_for_save(&profiles, 17);
    assert_eq!(
        manager
            .write_continue_save_background(&mut host, &game, &engine, 17, Some(&profiles), None)
            .unwrap(),
        SaveWriteStatus::Queued
    );
    let slot = manager.find_by_filename("Continue").unwrap();
    assert_eq!(
        manager
            .slot_state(&SlotName::new("Continue").unwrap())
            .unwrap(),
        SlotState::Draft
    );
    assert!(manager.preflight_exact_slot(slot).is_err());
    assert!(manager.save_index().is_err());
    manager.finish_background().unwrap();
    assert_eq!(
        manager.try_finish_background().unwrap(),
        SaveWriteStatus::Completed
    );
    assert_eq!(
        manager
            .slot_state(&SlotName::new("Continue").unwrap())
            .unwrap(),
        SlotState::Published
    );
    let reopened = SaveGameManager::load_index(root.path().to_str().unwrap()).unwrap();
    let decoded = reopened
        .preflight_exact_slot(reopened.find_by_filename("Continue").unwrap())
        .unwrap();
    assert_eq!(decoded.header.provenance.player_name, "Owned completion");
    assert!(!manager.owned_recovery_path().exists());
    assert!(!manager.poll_background().unwrap());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn payload_failure_preserves_old_metadata_and_error_is_sticky_but_notice_once() {
    let root = tempfile::tempdir().unwrap();
    let mut manager = indexed_store(root.path(), &["Continue"]);
    let before = manager.get(0).unwrap().clone();
    std::fs::create_dir(manager.save_path(0)).unwrap();
    let (engine, _, profiles, mut host) = fresh_save_session("Failed owner");
    let game = game_for_save(&profiles, 17);
    manager
        .write_continue_save_background(&mut host, &game, &engine, 17, Some(&profiles), None)
        .unwrap();
    assert!(manager.finish_background().is_err());
    assert_eq!(manager.get(0).unwrap(), &before);
    assert!(manager.poll_background().is_err());
    assert!(!manager.poll_background().unwrap());
    assert!(
        manager
            .create_draft("Must remain blocked".into(), 17)
            .is_err()
    );
    assert!(manager.finish_background().is_err());
    std::fs::remove_dir(manager.save_path(0)).unwrap();
    let recovered = SaveGameManager::load_index(root.path().to_str().unwrap()).unwrap();
    assert_eq!(recovered.get(0).unwrap(), &before);
    assert!(!manager.owned_recovery_path().exists());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn completed_payload_index_failure_is_recoverable_after_store_reopen() {
    let root = tempfile::tempdir().unwrap();
    let mut manager = indexed_store(root.path(), &["Continue"]);
    let index_path = root.path().join("saves.json");
    let old_index = std::fs::read(&index_path).unwrap();
    std::fs::remove_file(&index_path).unwrap();
    std::fs::create_dir(&index_path).unwrap();
    let (engine, _, profiles, mut host) = fresh_save_session("Recover owned metadata");
    let game = game_for_save(&profiles, 17);
    manager
        .write_continue_save_background(&mut host, &game, &engine, 17, Some(&profiles), None)
        .unwrap();
    assert!(manager.finish_background().is_err());
    assert!(manager.owned_recovery_path().exists());
    assert_eq!(
        GameSaveFile::read_from(&manager.save_path(0))
            .unwrap()
            .header
            .provenance
            .player_name,
        "Recover owned metadata"
    );
    std::fs::remove_dir(&index_path).unwrap();
    std::fs::write(&index_path, old_index).unwrap();
    let recovered = SaveGameManager::load_index(root.path().to_str().unwrap()).unwrap();
    assert_eq!(
        recovered.get(0).unwrap().player_name,
        "Recover owned metadata"
    );
    assert_eq!(recovered.get(0).unwrap().mission_id, 17);
    assert!(!manager.owned_recovery_path().exists());
}

#[test]
fn owned_receipt_cannot_publish_autosave_or_control_slots() {
    let root = tempfile::tempdir().unwrap();
    let manager = indexed_store(root.path(), &["Continue"]);
    let before = std::fs::read(root.path().join("saves.json")).unwrap();
    for name in [
        "Autosave_1_0000",
        "autosaves",
        "../escape",
        "owned-save-recovery",
    ] {
        let mut slot = published_slot("Continue");
        slot.filename = name.into();
        slot.special = SpecialSlot::from_filename(name);
        let receipt = SpecialSaveRecovery {
            slot,
            digest: Sha256::digest(b"payload").into(),
        };
        std::fs::write(
            manager.owned_recovery_path(),
            serde_json::to_vec(&receipt).unwrap(),
        )
        .unwrap();
        assert!(SaveGameManager::load_index(root.path().to_str().unwrap()).is_err());
        assert_eq!(
            std::fs::read(root.path().join("saves.json")).unwrap(),
            before
        );
        assert!(manager.owned_recovery_path().exists());
    }
}

#[test]
fn quick_owned_and_delete_recovery_preserve_each_others_metadata() {
    let root = tempfile::tempdir().unwrap();
    let manager = indexed_store(root.path(), &["Continue", "Savegame_000"]);
    let quick_bytes = b"quick recovery payload";
    let owned_bytes = b"owned recovery payload";
    std::fs::write(root.path().join("QuickSave.json"), quick_bytes).unwrap();
    std::fs::write(root.path().join("Continue.json"), owned_bytes).unwrap();
    let quick = QuickSaveRecovery {
        slots: vec![(
            published_slot("QuickSave"),
            Sha256::digest(quick_bytes).into(),
        )],
    };
    let mut owned_slot = published_slot("Continue");
    owned_slot.player_name = "Recovered owned player".into();
    let owned = SpecialSaveRecovery {
        slot: owned_slot,
        digest: Sha256::digest(owned_bytes).into(),
    };
    let delete = DeleteRecovery {
        filename: SlotName::new("Savegame_000").unwrap(),
    };
    std::fs::write(
        manager.quick_recovery_path(),
        serde_json::to_vec(&quick).unwrap(),
    )
    .unwrap();
    std::fs::write(
        manager.owned_recovery_path(),
        serde_json::to_vec(&owned).unwrap(),
    )
    .unwrap();
    std::fs::write(
        manager.delete_recovery_path(),
        serde_json::to_vec(&delete).unwrap(),
    )
    .unwrap();
    let reopened = SaveGameManager::load_index(root.path().to_str().unwrap()).unwrap();
    assert_eq!(reopened.count(), 2);
    assert!(reopened.find_by_filename("QuickSave").is_some());
    assert_eq!(
        reopened
            .get(reopened.find_by_filename("Continue").unwrap())
            .unwrap()
            .player_name,
        "Recovered owned player"
    );
    assert!(!manager.quick_recovery_path().exists());
    assert!(!manager.owned_recovery_path().exists());
    assert!(!manager.delete_recovery_path().exists());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn delayed_old_write_is_drained_before_delete_and_cannot_resurrect_slot() {
    use std::sync::mpsc;
    let root = tempfile::tempdir().unwrap();
    let mut manager = indexed_store(root.path(), &["Continue"]);
    let path = manager.save_path(0);
    std::fs::write(&path, b"old payload").unwrap();
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    manager
        .operations
        .start(SlotName::new("Continue").unwrap(), move || {
            started_tx.send(()).unwrap();
            release_rx
                .recv_timeout(std::time::Duration::from_secs(10))
                .context("release latch timed out")?;
            std::fs::write(path, b"late old write")?;
            Ok(published_slot("Continue"))
        })
        .unwrap();
    started_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let deletion = std::thread::spawn(move || {
        entered_tx.send(()).unwrap();
        let result = manager.remove(0);
        done_tx.send(result).unwrap();
    });
    entered_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    let pending = done_rx.recv_timeout(std::time::Duration::from_millis(50));
    release_tx.send(()).unwrap();
    assert!(matches!(pending, Err(mpsc::RecvTimeoutError::Timeout)));
    done_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap()
        .unwrap();
    deletion.join().unwrap();
    assert!(!root.path().join("Continue.json").exists());
    assert!(
        SaveGameManager::load_index(root.path().to_str().unwrap())
            .unwrap()
            .saves()
            .len()
            == 0
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn queued_writes_complete_in_order_and_retirement_cannot_touch_next_profile() {
    let first_root = tempfile::tempdir().unwrap();
    let second_root = tempfile::tempdir().unwrap();
    let mut manager = SaveGameManager::new(first_root.path().to_str().unwrap().into());
    let (mut engine, _, profiles, mut host) = fresh_save_session("Ordered save");
    let game = game_for_save(&profiles, 17);
    engine.test_set_frame_counter(10);
    manager
        .write_continue_save_background(&mut host, &game, &engine, 17, Some(&profiles), None)
        .unwrap();
    engine.test_set_frame_counter(20);
    manager
        .write_continue_save_background(&mut host, &game, &engine, 17, Some(&profiles), None)
        .unwrap();
    manager.finish_background().unwrap();
    drop(manager);
    let successor = indexed_store(second_root.path(), &["Continue"]);
    std::fs::write(successor.save_path(0), b"successor profile").unwrap();
    let recovered = SaveGameManager::load_index(first_root.path().to_str().unwrap()).unwrap();
    assert_eq!(
        recovered
            .preflight_exact_slot(0)
            .unwrap()
            .engine
            .frame_counter(),
        20
    );
    assert_eq!(
        std::fs::read(successor.save_path(0)).unwrap(),
        b"successor profile"
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "requires LLVM unwinding; run explicitly with robin_rs test codegen-backend=llvm"]
fn llvm_owned_worker_panic_is_joined_and_reported_once() {
    let root = tempfile::tempdir().unwrap();
    let mut manager = indexed_store(root.path(), &["Continue"]);
    let before = manager.get(0).unwrap().clone();
    let path = manager.save_path(0);
    manager
        .operations
        .start(SlotName::new("Continue").unwrap(), move || {
            struct TerminalMarker(PathBuf);
            impl Drop for TerminalMarker {
                fn drop(&mut self) {
                    std::fs::write(&self.0, b"unwind completed").unwrap();
                }
            }
            let _terminal = TerminalMarker(path);
            panic!("injected owned worker panic");
        })
        .unwrap();
    assert!(
        manager
            .finish_background()
            .unwrap_err()
            .to_string()
            .contains("panicked")
    );
    assert!(manager.operations.pending_name().is_none());
    assert_eq!(
        std::fs::read(manager.save_path(0)).unwrap(),
        b"unwind completed"
    );
    assert_eq!(manager.get(0).unwrap(), &before);
    assert!(manager.poll_background().is_err());
    assert!(!manager.poll_background().unwrap());
    assert!(manager.create_draft("blocked".into(), 1).is_err());
}

#[test]
fn emitted_index_preserves_legacy_directory_without_trusting_it() {
    let source = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    indexed_store(source.path(), &["Savegame_000"]);
    let bytes = std::fs::read(source.path().join("saves.json")).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["save_directory"], source.path().to_str().unwrap());
    std::fs::write(destination.path().join("saves.json"), bytes).unwrap();
    let manager = SaveGameManager::load_index(destination.path().to_str().unwrap()).unwrap();
    assert_eq!(
        manager.save_directory(),
        destination.path().to_str().unwrap()
    );
    manager.save_index().unwrap();
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(destination.path().join("saves.json")).unwrap())
            .unwrap();
    assert_eq!(json["save_directory"], destination.path().to_str().unwrap());
}

#[test]
fn autosave_manifest_is_not_a_manual_slot() {
    let root = tempfile::tempdir().unwrap();
    let manifest = root.path().join("autosaves.json");
    std::fs::write(&manifest, b"manifest must survive").unwrap();
    for name in ["autosaves", "AUTOSAVES"] {
        assert!(SlotName::new(name).is_err());
        let mut manager = indexed_store(root.path(), &["Savegame_000"]);
        manager.catalog[0].filename = name.into();
        assert!(manager.remove(0).is_err());
        assert!(manager.save_index().is_err());
        assert_eq!(std::fs::read(&manifest).unwrap(), b"manifest must survive");
    }
}

#[test]
fn failed_new_save_then_delete_other_slot_keeps_index_readable() {
    let root = tempfile::tempdir().unwrap();
    let mut manager = indexed_store(root.path(), &["Savegame_000"]);
    std::fs::write(manager.save_path(0), b"old save").unwrap();
    let draft = manager.create("Failed save".into(), 17);
    let (engine, _assets, profiles, mut host) = fresh_save_session("Failed new draft");
    let game = game_for_save(&profiles, 17);
    // A concurrent target causes the actual no-clobber save path to fail.
    let draft_path = manager.save_path(draft);
    std::fs::write(&draft_path, b"other writer").unwrap();
    assert!(
        manager
            .write_save_from_engine(&mut host, &game, draft, &engine, 17, Some(&profiles), None)
            .is_err()
    );
    manager.remove(0).unwrap();
    let reopened = SaveGameManager::load_index(root.path().to_str().unwrap()).unwrap();
    assert!(reopened.catalog.is_empty());
    assert_eq!(
        manager.count(),
        1,
        "draft remains available for retry in memory"
    );
    manager.remove(0).unwrap();
    assert_eq!(std::fs::read(draft_path).unwrap(), b"other writer");
}

#[test]
fn live_delete_recovers_pending_quick_metadata_before_retiring_receipt() {
    let root = tempfile::tempdir().unwrap();
    let mut manager = indexed_store(root.path(), &["Savegame_000"]);
    let bytes = b"published quick payload";
    std::fs::write(root.path().join("QuickSave.json"), bytes).unwrap();
    let receipt = QuickSaveRecovery {
        slots: vec![(published_slot("QuickSave"), Sha256::digest(bytes).into())],
    };
    std::fs::write(
        manager.quick_recovery_path(),
        serde_json::to_vec(&receipt).unwrap(),
    )
    .unwrap();
    assert!(
        manager.save_index().is_err(),
        "ordinary stale publication must not retire receipt"
    );
    manager.remove(0).unwrap();
    let reopened = SaveGameManager::load_index(root.path().to_str().unwrap()).unwrap();
    assert_eq!(reopened.count(), 1);
    assert_eq!(reopened.catalog[0].filename, "QuickSave");
    assert!(!manager.quick_recovery_path().exists());
}

#[test]
fn slot_names_reject_paths_devices_and_store_control_files() {
    for name in [
        "",
        "..",
        "../Savegame_000",
        "/tmp/save",
        "C:\\save",
        "folder\\save",
        "save.json",
        "saves",
        "CON",
        "aux",
        "Lpt9",
        "COM1",
        "quick-save-recovery",
        "save-delete-recovery",
        "OwNeD-SaVe-ReCoVeRy",
        "AuToSaVeS",
        "NUL",
        "PRN",
        "雪a",
        "a雪",
    ] {
        assert!(SlotName::new(name).is_err(), "accepted {name:?}");
        assert!(
            SlotName::validate(name).is_err(),
            "accepted borrowed {name:?}"
        );
        assert!(serde_json::from_value::<SlotName>(serde_json::json!(name)).is_err());
    }
    for name in [
        "Savegame_000",
        "QuickSave",
        "Autosave_123_0000",
        "custom-save",
        "com0",
        "COM10",
        "lpt0",
        "saves-backup",
    ] {
        assert_eq!(SlotName::new(name).unwrap().as_str(), name);
        assert!(SlotName::validate(name).is_ok());
    }
}

#[test]
fn legacy_index_is_rebound_before_recovery_and_deletion() {
    let old = tempfile::tempdir().unwrap();
    let current = tempfile::tempdir().unwrap();
    let original = indexed_store(old.path(), &["Savegame_000"]);
    std::fs::write(original.save_path(0), b"old payload").unwrap();
    let mut json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(old.path().join("saves.json")).unwrap()).unwrap();
    json["save_directory"] = serde_json::json!(old.path().to_str().unwrap());
    std::fs::write(
        current.path().join("saves.json"),
        serde_json::to_vec(&json).unwrap(),
    )
    .unwrap();
    // A bad receipt in the old root must never be consulted.
    std::fs::write(old.path().join("quick-save-recovery.json"), b"invalid").unwrap();
    std::fs::write(current.path().join("Savegame_000.json"), b"copied payload").unwrap();
    let mut reopened = SaveGameManager::load_index(current.path().to_str().unwrap()).unwrap();
    assert_eq!(reopened.save_directory(), current.path().to_str().unwrap());
    reopened.remove(0).unwrap();
    assert_eq!(
        std::fs::read(original.save_path(0)).unwrap(),
        b"old payload"
    );
    assert!(
        SaveGameManager::load_index(current.path().to_str().unwrap())
            .unwrap()
            .saves()
            .next()
            .is_none()
    );
}

#[test]
fn invalid_and_duplicate_index_names_are_rejected_before_recovery() {
    let root = tempfile::tempdir().unwrap();
    for names in [
        vec!["../outside"],
        vec!["Savegame_000", "Savegame_000"],
        vec!["Savegame_000", "savegame_000"],
    ] {
        let slots: Vec<_> = names
            .iter()
            .map(|name| {
                let mut slot = published_slot("Savegame_000");
                slot.filename = (*name).into();
                slot
            })
            .collect();
        let bytes = serde_json::to_vec(&SaveIndex {
            saves: slots,
            next_id: 0,
            save_directory: root.path().to_str().unwrap().into(),
        })
        .unwrap();
        std::fs::write(root.path().join("saves.json"), &bytes).unwrap();
        std::fs::write(root.path().join("quick-save-recovery.json"), b"invalid").unwrap();
        let error = SaveGameManager::load_index(root.path().to_str().unwrap()).unwrap_err();
        // Basenames are now validated while binding explicit runtime slot
        // state; duplicate checks follow before recovery. Assert the
        // actual rejected invariant rather than one former phase prefix.
        let expected = if names.len() == 1 {
            "invalid save slot basename"
        } else {
            "duplicate save slot name"
        };
        assert!(error.contains(expected), "{error}");
        assert!(!error.contains("recover quick saves"), "{error}");
        assert_eq!(
            std::fs::read(root.path().join("saves.json")).unwrap(),
            bytes
        );
        assert_eq!(
            std::fs::read(root.path().join("quick-save-recovery.json")).unwrap(),
            b"invalid"
        );
    }
}

#[test]
fn corrupt_obsolete_and_unreadable_indexes_do_not_reset_existing_saves() {
    let root = tempfile::tempdir().unwrap();
    let payload = root.path().join("Savegame_000.json");
    let index = root.path().join("saves.json");
    std::fs::write(&payload, b"precious payload").unwrap();
    std::fs::write(&index, b"broken JSON").unwrap();
    assert!(SaveGameManager::load_index(root.path().to_str().unwrap()).is_err());
    let mut slot = published_slot("Savegame_000");
    slot.version = 0;
    std::fs::write(
        &index,
        serde_json::to_vec(&SaveIndex {
            saves: vec![slot],
            next_id: 0,
            save_directory: root.path().to_str().unwrap().into(),
        })
        .unwrap(),
    )
    .unwrap();
    assert!(SaveGameManager::load_index(root.path().to_str().unwrap()).is_err());
    std::fs::remove_file(&index).unwrap();
    // A directory at the index path is a deterministic read error even
    // when tests run as a privileged user (unlike chmod-based fixtures).
    std::fs::create_dir(&index).unwrap();
    assert!(SaveGameManager::load_index(root.path().to_str().unwrap()).is_err());
    assert_eq!(std::fs::read(payload).unwrap(), b"precious payload");
}

#[test]
fn missing_and_stale_indexes_allocate_past_orphans_and_indexed_slots() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("Savegame_000.json"), b"orphan").unwrap();
    std::fs::write(root.path().join("Savegame_001_thumb.png"), b"preview").unwrap();
    let mut missing = SaveGameManager::load_index(root.path().to_str().unwrap()).unwrap();
    assert!(missing.slot_name(missing.count()).is_err());
    let slot = missing.create("New".into(), 1);
    assert_eq!(missing.slot_name(slot).unwrap().as_str(), "Savegame_002");
    let mut stale = indexed_store(root.path(), &["Savegame_002"]);
    let slot = stale.create("Next".into(), 1);
    assert_eq!(stale.slot_name(slot).unwrap().as_str(), "Savegame_003");
    assert_eq!(
        std::fs::read(root.path().join("Savegame_000.json")).unwrap(),
        b"orphan"
    );
}

#[test]
fn delete_is_durable_and_selection_survives_reopen() {
    let root = tempfile::tempdir().unwrap();
    let mut manager = indexed_store(root.path(), &["Savegame_000", "Savegame_001"]);
    std::fs::write(manager.save_path(0), b"first").unwrap();
    std::fs::write(manager.save_path(1), b"selected").unwrap();
    let selection = manager.slot_name(1).unwrap();
    manager.remove(0).unwrap();
    let reopened = SaveGameManager::load_index(root.path().to_str().unwrap()).unwrap();
    let selected = reopened.find_by_filename(selection.as_str()).unwrap();
    assert_eq!(selected, 0);
    assert_eq!(
        std::fs::read(reopened.save_path(selected)).unwrap(),
        b"selected"
    );
    assert!(reopened.find_by_filename("Savegame_000").is_none());
    assert!(!root.path().join("Savegame_000.json").exists());
    assert!(!manager.delete_recovery_path().exists());
}

#[test]
fn delete_cleanup_failure_is_visible_and_recoverable() {
    let root = tempfile::tempdir().unwrap();
    let mut manager = indexed_store(root.path(), &["Savegame_000"]);
    let payload = manager.save_path(0);
    std::fs::create_dir(&payload).unwrap();
    let error = manager.remove(0).unwrap_err();
    assert!(format!("{error:#}").contains("cleanup"));
    assert!(manager.catalog.is_empty());
    assert!(
        manager
            .save_index()
            .unwrap_err()
            .contains("recovery is pending")
    );
    assert!(manager.delete_recovery_path().exists());
    assert!(SaveGameManager::load_index(root.path().to_str().unwrap()).is_err());
    std::fs::remove_dir(payload).unwrap();
    assert!(
        SaveGameManager::load_index(root.path().to_str().unwrap())
            .unwrap()
            .saves()
            .next()
            .is_none()
    );
    assert!(!manager.delete_recovery_path().exists());
}

#[test]
fn delete_index_failure_keeps_payload_and_intent_for_retry() {
    let root = tempfile::tempdir().unwrap();
    let mut manager = indexed_store(root.path(), &["Savegame_000"]);
    let payload = manager.save_path(0);
    std::fs::write(&payload, b"preserved until indexed").unwrap();
    let index = root.path().join("saves.json");
    std::fs::remove_file(&index).unwrap();
    std::fs::create_dir(&index).unwrap();
    assert!(manager.remove(0).is_err());
    assert!(manager.catalog.is_empty());
    assert!(manager.delete_recovery_path().exists());
    assert_eq!(std::fs::read(&payload).unwrap(), b"preserved until indexed");
    std::fs::remove_dir(&index).unwrap();
    let reopened = SaveGameManager::load_index(root.path().to_str().unwrap()).unwrap();
    assert!(reopened.catalog.is_empty());
    assert!(!payload.exists());
}

#[test]
fn failed_delete_intent_does_not_remove_slot_or_payload() {
    let root = tempfile::tempdir().unwrap();
    let mut manager = indexed_store(root.path(), &["Savegame_000"]);
    std::fs::write(manager.save_path(0), b"payload").unwrap();
    std::fs::create_dir(manager.delete_recovery_path()).unwrap();
    assert!(manager.remove(0).is_err());
    assert_eq!(manager.count(), 1);
    assert_eq!(std::fs::read(manager.save_path(0)).unwrap(), b"payload");
}

#[test]
fn quick_and_delete_receipts_recover_together_without_losing_quick_metadata() {
    let root = tempfile::tempdir().unwrap();
    let manager = indexed_store(root.path(), &["Savegame_000"]);
    std::fs::write(manager.save_path(0), b"delete me").unwrap();
    let quick_bytes = b"digest bound quick payload";
    std::fs::write(root.path().join("QuickSave.json"), quick_bytes).unwrap();
    let quick = QuickSaveRecovery {
        slots: vec![(
            published_slot("QuickSave"),
            Sha256::digest(quick_bytes).into(),
        )],
    };
    std::fs::write(
        manager.quick_recovery_path(),
        serde_json::to_vec(&quick).unwrap(),
    )
    .unwrap();
    let delete = DeleteRecovery {
        filename: SlotName::new("Savegame_000").unwrap(),
    };
    std::fs::write(
        manager.delete_recovery_path(),
        serde_json::to_vec(&delete).unwrap(),
    )
    .unwrap();
    let recovered = SaveGameManager::load_index(root.path().to_str().unwrap()).unwrap();
    assert_eq!(recovered.count(), 1);
    assert_eq!(recovered.catalog[0].filename, "QuickSave");
    assert!(!manager.quick_recovery_path().exists());
    assert!(!manager.delete_recovery_path().exists());
    assert_eq!(
        std::fs::read(root.path().join("QuickSave.json")).unwrap(),
        quick_bytes
    );
}

#[test]
fn corrupt_quick_receipt_blocks_open_and_preserves_index_and_payload() {
    let root = tempfile::tempdir().unwrap();
    let manager = indexed_store(root.path(), &["Savegame_000"]);
    std::fs::write(manager.save_path(0), b"payload").unwrap();
    let before = std::fs::read(root.path().join("saves.json")).unwrap();
    std::fs::write(manager.quick_recovery_path(), b"broken").unwrap();
    assert!(
        SaveGameManager::load_index(root.path().to_str().unwrap())
            .unwrap_err()
            .contains("recover quick saves")
    );
    assert_eq!(
        std::fs::read(root.path().join("saves.json")).unwrap(),
        before
    );
    assert_eq!(std::fs::read(manager.save_path(0)).unwrap(), b"payload");
}

#[test]
fn new_manual_save_cannot_clobber_a_payload_created_after_selection() {
    let root = tempfile::tempdir().unwrap();
    let mut manager = SaveGameManager::new(root.path().to_str().unwrap().into());
    let slot = manager.create("New".into(), 17);
    let (engine, _assets, profiles, mut host) = fresh_save_session("Concurrent writer test");
    let game = game_for_save(&profiles, 17);
    std::fs::write(manager.save_path(slot), b"concurrent writer").unwrap();
    assert!(
        manager
            .write_save_from_engine(&mut host, &game, slot, &engine, 17, Some(&profiles), None)
            .is_err()
    );
    assert_eq!(
        std::fs::read(manager.save_path(slot)).unwrap(),
        b"concurrent writer"
    );
    assert!(manager.catalog[slot].timestamp.is_empty());
}
