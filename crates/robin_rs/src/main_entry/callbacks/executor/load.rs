//! Immutable routing and consuming application stages. Neither stage can
//! publish an index, enqueue another callback, or change a presentation banner.

use super::super::{
    OperationOutcome, PostLoadSync, PreparedLoad, SaveBannerKind, SaveLoadEvent,
    current_mission_id, replay_loaded_identity, validated_save_reload_target,
};
use crate::savegame::SpecialSlot;
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

impl<'de> serde::Deserialize<'de> for CurrentMissionLoad {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "current-mission load permission is process-local",
        ))
    }
}

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
    mission_id: u32,
    snapshot: Option<Vec<u8>>,
    identity: Option<crate::save_file::ReplaySaveIdentity>,
}

impl<'de> serde::Deserialize<'de> for AppliedLoad {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "load application receipts are process-local",
        ))
    }
}

impl AppliedLoad {
    pub(super) fn mission_id(&self) -> u32 {
        self.mission_id
    }

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
            event: self.identity.map(|identity| SaveLoadEvent::LoadApplied {
                snapshot: self.snapshot,
                identity,
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
    let mission_id = save.header.mission_id;
    let identity = replay_loaded_identity(&save);
    // Dereference the process-local prepared-save wrapper: the replay carries
    // the public save envelope, never the checkpoint's local authority.
    let snapshot = Some(serde_json::to_vec(&*save)?);
    save.apply_to_with_game(engine, host, game, assets)?;
    Ok(AppliedLoad {
        snapshot,
        mission_id,
        identity,
    })
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
        let applied = apply(prepared, &mut engine, &mut host, &mut game, &assets).unwrap();
        assert_eq!(engine.frame_counter(), 41);
        assert_eq!(applied.mission_id(), 17);
        let outcome = applied.outcome(LoadCompletion::Quick);
        let super::SaveLoadEvent::LoadApplied {
            snapshot: Some(snapshot),
            ..
        } = outcome.event.as_ref().expect("successful load receipt")
        else {
            panic!("successful load must carry its replay save payload");
        };
        let embedded: crate::save_file::GameSaveFile = serde_json::from_slice(snapshot).unwrap();
        embedded.validate_current_schema().unwrap();
        assert_eq!(embedded.engine.frame_counter(), 41);
        assert!(outcome.processed() && outcome.reset_input());
        assert_eq!(outcome.banner, Some(SaveBannerKind::Loaded));
        assert!(!outcome.restore().unwrap().is_continue);
        assert!(outcome.transition().is_none() && !outcome.restart_requested());
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
                snapshot: None,
                mission_id: 17,
                identity: None,
            };
            let bytes = serde_json::to_vec(&receipt).unwrap();
            assert!(serde_json::from_slice::<AppliedLoad>(&bytes).is_err());
            let outcome = receipt.outcome(completion);
            assert_eq!(outcome.reset_input(), reset);
            assert_eq!(outcome.banner, banner);
            assert_eq!(outcome.restore().unwrap().is_continue, continued);
            assert!(outcome.event.is_none() && outcome.transition().is_none());
        }
    }
}
