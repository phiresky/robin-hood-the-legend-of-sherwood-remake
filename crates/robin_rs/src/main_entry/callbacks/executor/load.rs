//! Immutable routing and consuming application stages. Neither stage can
//! publish an index, enqueue another callback, or change a presentation banner.

use super::super::{
    OperationOutcome, PostLoadSync, PreparedLoad, SaveBannerKind, SaveLoadEvent,
    current_mission_id, validated_save_reload_target,
};
use crate::savegame::SpecialSlot;
use anyhow::Context;
use robin_engine::{engine as engine_api, profiles::ProfileManager};

pub(super) enum LoadRoute {
    Current(CurrentMissionLoad),
    OtherMission {
        save: PreparedLoad,
        target_mission_id: u32,
        active_mission_id: u32,
    },
}

/// Only routing can grant permission to apply to this mission. The private
/// payload cannot be swapped by the dispatcher after descriptor validation.
#[derive(serde::Serialize)]
pub(super) struct CurrentMissionLoad {
    #[serde(skip)]
    save: PreparedLoad,
}

impl CurrentMissionLoad {
    pub(super) fn save(&self) -> &crate::save_file::GameSaveFile {
        self.save.save()
    }
}

#[cfg(target_arch = "wasm32")]
impl CurrentMissionLoad {
    pub(super) fn replay_directory(&self) -> Option<&std::path::Path> {
        self.save
            .save()
            .header
            .replay
            .as_ref()
            .map(|link| std::path::Path::new(&link.mission_directory))
    }
}

robin_util::deny_deserialize!(
    CurrentMissionLoad,
    "current-mission load permission is process-local"
);

pub(super) enum LoadCompletion {
    Restart,
    Selected(Option<SpecialSlot>),
    Quick,
}

impl LoadCompletion {
    pub(super) fn mirrors_continue(&self) -> bool {
        !matches!(
            self,
            Self::Restart | Self::Selected(Some(SpecialSlot::Continue | SpecialSlot::Restart))
        )
    }
}

pub(super) fn route(
    save: PreparedLoad,
    engine: &engine_api::Engine,
    game: &crate::game::Game,
    profiles: &ProfileManager,
) -> Result<LoadRoute, String> {
    let active_mission_id = current_mission_id(engine.campaign(), profiles);
    let active_spellforge_package = engine.spellforge_package();
    match validated_save_reload_target(
        save.save(),
        profiles,
        active_mission_id,
        game.mission_assets()?,
        active_spellforge_package.as_deref(),
    )? {
        Some(target_mission_id) => Ok(LoadRoute::OtherMission {
            save,
            target_mission_id,
            active_mission_id,
        }),
        None => Ok(LoadRoute::Current(CurrentMissionLoad { save })),
    }
}

/// Only successful application produces a receipt. Replay identity is captured
/// from the decoded payload, before any subsequent live-engine fixups.
#[derive(serde::Serialize)]
pub(super) struct AppliedLoad {
    snapshot: Vec<u8>,
    identity: crate::save_file::ReplaySaveIdentity,
}

robin_util::deny_deserialize!(AppliedLoad, "load application receipts are process-local");

impl AppliedLoad {
    pub(super) fn outcome(self, completion: LoadCompletion) -> OperationOutcome {
        let (is_continue, reset_input, banner) = match completion {
            LoadCompletion::Restart => (false, false, None),
            LoadCompletion::Quick => (false, true, Some(SaveBannerKind::Loaded)),
            LoadCompletion::Selected(special) => (
                special == Some(SpecialSlot::Continue),
                true,
                (!matches!(special, Some(SpecialSlot::Restart | SpecialSlot::Sherwood)))
                    .then_some(SaveBannerKind::Loaded),
            ),
        };
        OperationOutcome {
            event: Some(SaveLoadEvent::LoadApplied {
                snapshot: self.snapshot,
                identity: self.identity,
                is_continue,
            }),
            completion: super::super::OperationCompletion::Restored {
                sync: PostLoadSync { is_continue },
                reset_input,
            },
            banner,
        }
    }
}

pub(super) fn apply(
    prepared: CurrentMissionLoad,
    engine: &mut engine_api::Engine,
    host: &mut crate::host::Host,
    game: &mut crate::game::Game,
    assets: &engine_api::LevelAssets,
) -> anyhow::Result<AppliedLoad> {
    let CurrentMissionLoad { save } = prepared;
    let save = save.into_payload();
    // Both pieces of recording evidence are required before changing the live
    // engine. A successful restore must never disappear from the timeline just
    // because computing its identity failed.
    let identity = save
        .replay_identity()
        .context("prepare loaded save replay identity")?;
    // Dereference the process-local prepared-save wrapper: the replay carries
    // the public save envelope, never the checkpoint's local authority.
    let snapshot = serde_json::to_vec(&*save).context("encode loaded save replay snapshot")?;
    save.apply_to_with_game(engine, host, game, assets)?;
    Ok(AppliedLoad { snapshot, identity })
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::{AppliedLoad, LoadCompletion, LoadRoute, apply, route};
    use crate::main_entry::callbacks::{
        SaveBannerKind, operation_outcome_tests::diagnostic_callback_fixture,
    };
    use crate::savegame::SpecialSlot;

    #[test]
    fn continue_mirror_policy_matrix() {
        for (completion, expected) in [
            (LoadCompletion::Restart, false),
            (LoadCompletion::Quick, true),
            (LoadCompletion::Selected(None), true),
            (LoadCompletion::Selected(Some(SpecialSlot::Continue)), false),
            (LoadCompletion::Selected(Some(SpecialSlot::Restart)), false),
            (LoadCompletion::Selected(Some(SpecialSlot::Sherwood)), true),
        ] {
            assert_eq!(completion.mirrors_continue(), expected);
        }
    }

    #[test]
    fn routing_owns_prepared_payload_and_only_application_emits_restore() {
        let directory = tempfile::tempdir().unwrap();
        let (mut callbacks, mut host, mut engine, assets, mut game, profiles) =
            diagnostic_callback_fixture(directory.path());
        let handle = callbacks
            .save_manager
            .create_draft("staged".into(), 17)
            .unwrap();
        let index = callbacks.save_manager.resolve_handle(&handle).unwrap();
        engine.test_set_frame_counter(41);
        callbacks
            .save_manager
            .write_save_from_engine(&mut host, &game, index, &engine, 17, Some(&profiles), None)
            .unwrap();
        let prepared =
            crate::main_entry::PreparedLoad::preflight(&callbacks.save_manager, Some(handle))
                .unwrap()
                .unwrap();
        engine.test_set_frame_counter(99);
        let routed = route(prepared, &engine, &game, &profiles).unwrap();
        assert_eq!(engine.frame_counter(), 99);
        let LoadRoute::Current(prepared) = routed else {
            panic!("same mission must not require reload")
        };
        assert_eq!(prepared.save.save().engine.frame_counter(), 41);
        let identity = prepared.save().replay_identity().unwrap();
        let applied = apply(prepared, &mut engine, &mut host, &mut game, &assets).unwrap();
        assert_eq!(engine.frame_counter(), 41);
        let outcome = applied.outcome(LoadCompletion::Quick);
        let super::SaveLoadEvent::LoadApplied {
            snapshot,
            identity: recorded_identity,
            ..
        } = outcome.event.as_ref().expect("successful load receipt")
        else {
            panic!("successful load must carry its replay save payload");
        };
        let embedded: crate::save_file::GameSaveFile = serde_json::from_slice(snapshot).unwrap();
        embedded.validate_current_schema().unwrap();
        let event = outcome.event.as_ref().unwrap();
        let encoded = serde_json::to_value(event).unwrap();
        assert_eq!(
            serde_json::from_value::<super::SaveLoadEvent>(encoded.clone()).unwrap(),
            *event
        );
        for omit in [false, true] {
            let mut invalid = encoded.clone();
            let fields = invalid["LoadApplied"].as_object_mut().unwrap();
            if omit {
                fields.remove("snapshot");
            } else {
                fields.insert("snapshot".into(), serde_json::Value::Null);
            }
            assert!(serde_json::from_value::<super::SaveLoadEvent>(invalid).is_err());
        }
        assert_eq!(embedded.header.mission_id, 17);
        assert_eq!(embedded.engine.frame_counter(), 41);
        assert_eq!(*recorded_identity, identity);
        assert!(outcome.processed() && outcome.reset_input());
        assert_eq!(outcome.banner, Some(SaveBannerKind::Loaded));
        assert!(!outcome.restore().unwrap().is_continue);
        assert!(outcome.transition().is_none() && !outcome.restart_requested());
    }

    #[test]
    fn replay_identity_failure_rejects_load_before_live_state_changes() {
        use crate::host::{
            PendingSnapshotTransition, PendingSnapshotTransitionPayload, SnapshotSave,
        };
        use robin_engine::engine::{Engine, EngineArgs, LevelLoadArgs, SimConfig};
        use robin_engine::multiplayer::{MultiplayerSessionId, SnapshotTransitionId};

        let directory = tempfile::tempdir().unwrap();
        let (_, mut host, mut engine, mut assets, mut game, profiles) =
            diagnostic_callback_fixture(directory.path());
        // Original parity RNG ownership is deliberately non-serializable. Use
        // that real failure, not an injected codec or a permissive dummy save.
        let source = Engine::new(EngineArgs {
            campaign: engine.campaign().clone(),
            level: LevelLoadArgs {
                assets: &mut assets,
                level_directory: "",
                progress: &mut |_| {},
                loaded: robin_engine::level_data::LoadedLevel::empty_for_test(),
                bg_pixel_dims: (0.0, 0.0),
            },
            ground_mark_sprite: None,
            titbit_row_frame_counts: Vec::new(),
            rng_seed: 0,
            original_rng_replay: Some(vec![1; 10_000]),
            sim_config: SimConfig {
                script_enabled: false,
                ..SimConfig::default()
            },
        })
        .unwrap();
        let mut save =
            crate::save_file::GameSaveFile::capture(&source, &host, 17, "invalid".into());
        save.header.mission_assets = game.mission_assets().unwrap().clone();
        assert!(save.replay_identity().is_err());

        // Follow the actual in-memory committed-load admission path. This
        // fixture cannot enter through disk JSON because its RNG cannot encode.
        let id = SnapshotTransitionId {
            session_id: MultiplayerSessionId([7; 32]),
            sequence: 1,
        };
        host.transport
            .prepare_snapshot_transition(PendingSnapshotTransition::new(
                id,
                PendingSnapshotTransitionPayload::Save {
                    load: SnapshotSave::Remote(Box::new(save)),
                },
            ));
        host.transport.commit_snapshot_transition(id).unwrap();
        let prepared = crate::main_entry::PreparedLoad::from_committed_snapshot(
            host.transport.take_committed_snapshot_transition().unwrap(),
        )
        .unwrap();
        let LoadRoute::Current(prepared) = route(prepared, &engine, &game, &profiles).unwrap()
        else {
            panic!("same-mission load expected");
        };
        engine.test_set_frame_counter(99);
        let before = serde_json::to_value((&engine, &host.audio.sound, &game.persistent)).unwrap();
        let error = match apply(prepared, &mut engine, &mut host, &mut game, &assets) {
            Ok(_) => panic!("a restore without replay identity must not succeed"),
            Err(error) => error,
        };
        assert!(format!("{error:#}").contains("prepare loaded save replay identity"));
        assert!(format!("{error:#}").contains("original RNG parity replay cannot be serialized"));
        assert_eq!(
            serde_json::to_value((&engine, &host.audio.sound, &game.persistent)).unwrap(),
            before
        );
    }

    #[test]
    fn invalid_route_produces_no_application_and_completion_policies_stay_distinct() {
        let directory = tempfile::tempdir().unwrap();
        let (mut callbacks, mut host, engine, _assets, game, profiles) =
            diagnostic_callback_fixture(directory.path());
        let handle = callbacks
            .save_manager
            .create_draft("invalid preflight".into(), 17)
            .unwrap();
        let index = callbacks.save_manager.resolve_handle(&handle).unwrap();
        callbacks
            .save_manager
            .write_save_from_engine(&mut host, &game, index, &engine, 17, Some(&profiles), None)
            .unwrap();
        let prepared = crate::main_entry::PreparedLoad::preflight(
            &callbacks.save_manager,
            Some(handle.clone()),
        )
        .unwrap()
        .unwrap();
        let mut invalid = prepared.save().clone();
        assert!(matches!(
            route(prepared, &engine, &game, &profiles).unwrap(),
            LoadRoute::Current(_)
        ));
        // Another valid mission/descriptor means OtherMission, not rejection.
        // Mutate one actual descriptor invariant instead.
        invalid.header.mission_assets.mission_basename.clear();
        std::fs::write(
            callbacks.save_manager.save_path(index),
            serde_json::to_vec(&invalid).unwrap(),
        )
        .unwrap();
        // Corrupt input now fails before an owned prepared value can exist.
        assert!(
            crate::main_entry::PreparedLoad::preflight(&callbacks.save_manager, Some(handle))
                .is_err()
        );
        for (completion, reset, banner, continued) in [
            (LoadCompletion::Restart, false, None, false),
            (
                LoadCompletion::Selected(Some(SpecialSlot::Restart)),
                true,
                None,
                false,
            ),
            (
                LoadCompletion::Selected(Some(SpecialSlot::Sherwood)),
                true,
                None,
                false,
            ),
            (
                LoadCompletion::Selected(Some(SpecialSlot::Continue)),
                true,
                Some(SaveBannerKind::Loaded),
                true,
            ),
            (
                LoadCompletion::Selected(None),
                true,
                Some(SaveBannerKind::Loaded),
                false,
            ),
        ] {
            // Reducer-only fixture: no claim that an engine was applied here.
            let receipt = AppliedLoad {
                snapshot: b"policy-only fixture".to_vec(),
                identity: crate::save_file::ReplaySaveIdentity::Payload([7; 32]),
            };
            let bytes = serde_json::to_vec(&receipt).unwrap();
            assert!(serde_json::from_slice::<AppliedLoad>(&bytes).is_err());
            let outcome = receipt.outcome(completion);
            assert_eq!(outcome.reset_input(), reset);
            assert_eq!(outcome.banner, banner);
            assert_eq!(outcome.restore().unwrap().is_continue, continued);
            assert!(matches!(
                &outcome.event,
                Some(super::SaveLoadEvent::LoadApplied { snapshot: _, .. })
            ));
            assert!(outcome.transition().is_none());
        }
    }
}
