//! Exact local preflight ownership and committed remote admission. No API accepts
//! an independently supplied local handle and decoded payload.

use crate::save_file::{GameSaveFile, PreparedGameSave};
use crate::savegame::{SaveGameManager, SlotHandle};

/// The payload cannot be swapped independently of its admitted selection.
/// ```compile_fail
/// fn substitute(load: &mut robin_rs::main_entry::PreparedLoad, other: robin_rs::save_file::GameSaveFile) {
///     load.save = other.into();
/// }
/// ```
#[derive(Clone, serde::Serialize)]
pub struct PreparedLoad {
    source: LoadSource,
    #[serde(skip)]
    save: PreparedGameSave,
}

#[derive(Clone, serde::Serialize)]
enum LoadSource {
    Local(SlotHandle),
    CommittedLocal(SlotHandle),
    CommittedRemote,
}

impl<'de> serde::Deserialize<'de> for PreparedLoad {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "prepared load ownership cannot be deserialized",
        ))
    }
}

impl PreparedLoad {
    pub(crate) fn preflight(
        manager: &SaveGameManager,
        slot: Option<SlotHandle>,
    ) -> anyhow::Result<Option<Self>> {
        let index = slot
            .as_ref()
            .map(|slot| manager.resolve_handle(slot))
            .transpose()?;
        manager
            .preflight_load(index)?
            .map(|(index, save)| {
                Ok(Self {
                    source: LoadSource::Local(manager.slot_handle(index)?),
                    save,
                })
            })
            .transpose()
    }

    pub(in crate::main_entry::callbacks) fn restart(
        manager: &SaveGameManager,
    ) -> anyhow::Result<Option<Self>> {
        manager
            .preflight_restart_save()?
            .map(|(index, save)| {
                Ok(Self {
                    source: LoadSource::Local(manager.slot_handle(index)?),
                    save,
                })
            })
            .transpose()
    }

    pub(crate) fn from_committed_snapshot(
        transition: crate::host::CommittedSnapshotTransition,
    ) -> anyhow::Result<Self> {
        let crate::host::PendingSnapshotTransitionPayload::Save { load } =
            transition.into_payload()
        else {
            anyhow::bail!("committed campaign transition is not a save load")
        };
        match load {
            crate::host::SnapshotSave::Local(mut load) => {
                let LoadSource::Local(slot) = load.source else {
                    anyhow::bail!("snapshot proposal was already committed")
                };
                load.source = LoadSource::CommittedLocal(slot);
                Ok(load)
            }
            crate::host::SnapshotSave::Remote(save) => {
                save.validate_current_schema()?;
                Ok(Self {
                    source: LoadSource::CommittedRemote,
                    save: (*save).into(),
                })
            }
        }
    }

    pub(crate) fn save(&self) -> &GameSaveFile {
        &self.save
    }
    pub(crate) fn mission_id(&self) -> u32 {
        self.save.header.mission_id
    }
    pub(crate) fn validate_slot(&self, manager: &SaveGameManager) -> anyhow::Result<()> {
        if let Some(slot) = self.slot() {
            manager.resolve_handle(slot)?;
        }
        Ok(())
    }
    pub(in crate::main_entry::callbacks) fn slot(&self) -> Option<&SlotHandle> {
        match &self.source {
            LoadSource::Local(slot) | LoadSource::CommittedLocal(slot) => Some(slot),
            LoadSource::CommittedRemote => None,
        }
    }
    pub(in crate::main_entry::callbacks) fn is_committed(&self) -> bool {
        !matches!(self.source, LoadSource::Local(_))
    }
    pub(in crate::main_entry::callbacks) fn into_payload(self) -> PreparedGameSave {
        self.save
    }
}

impl std::fmt::Debug for PreparedLoad {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedLoad")
            .field("slot", &self.slot())
            .field("mission_id", &self.mission_id())
            .field("committed", &self.is_committed())
            .finish_non_exhaustive()
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::PreparedLoad;
    use crate::host::{PendingSnapshotTransition, PendingSnapshotTransitionPayload, SnapshotSave};
    use crate::main_entry::callbacks::{
        PendingLevelLoad, SaveLoadRequest, operation_outcome_tests::diagnostic_callback_fixture,
    };
    use robin_engine::multiplayer::{MultiplayerSessionId, SnapshotTransitionId};

    #[test]
    fn prepared_local_rejects_stale_foreign_and_decoded_authority_without_rebinding() {
        let directory = tempfile::tempdir().unwrap();
        let (mut callbacks, mut host, engine, _assets, game, profiles) =
            diagnostic_callback_fixture(directory.path());
        let other = callbacks
            .save_manager
            .create_draft("other".into(), 17)
            .unwrap();
        let handle = callbacks
            .save_manager
            .create_draft("selected".into(), 17)
            .unwrap();
        let index = callbacks.save_manager.resolve_handle(&handle).unwrap();
        callbacks
            .save_manager
            .write_save_from_engine(&mut host, &game, index, &engine, 17, Some(&profiles), None)
            .unwrap();
        let prepared = PreparedLoad::preflight(&callbacks.save_manager, Some(handle.clone()))
            .unwrap()
            .unwrap();
        assert_eq!(prepared.save().header.display_text, "selected");
        let request = SaveLoadRequest::ApplyLoad(prepared.clone());
        assert!(
            serde_json::from_slice::<SaveLoadRequest>(&serde_json::to_vec(&request).unwrap())
                .is_err()
        );
        let decoded = serde_json::from_slice(&serde_json::to_vec(&handle).unwrap()).unwrap();
        assert!(PreparedLoad::preflight(&callbacks.save_manager, Some(decoded)).is_err());
        let foreign =
            crate::savegame::SaveGameManager::new(directory.path().to_string_lossy().into_owned());
        assert!(prepared.validate_slot(&foreign).is_err());
        let other_index = callbacks.save_manager.resolve_handle(&other).unwrap();
        callbacks.save_manager.remove(other_index).unwrap();
        prepared.validate_slot(&callbacks.save_manager).unwrap();
        let shifted = callbacks.save_manager.resolve_handle(&handle).unwrap();
        assert_ne!(shifted, index);
        callbacks.save_manager.remove(shifted).unwrap();
        assert!(prepared.validate_slot(&callbacks.save_manager).is_err());
        callbacks.save_manager.create_with_filename(
            handle.name().as_str().into(),
            "recreated".into(),
            17,
        );
        assert!(prepared.validate_slot(&callbacks.save_manager).is_err());
    }

    #[test]
    fn handoff_retains_exact_payload_and_derives_mission_after_file_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let (mut callbacks, mut host, mut engine, assets, mut game, profiles) =
            diagnostic_callback_fixture(directory.path());
        let handle = callbacks
            .save_manager
            .create_draft("first".into(), 17)
            .unwrap();
        let index = callbacks.save_manager.resolve_handle(&handle).unwrap();
        engine.test_set_frame_counter(41);
        callbacks
            .save_manager
            .write_save_from_engine(&mut host, &game, index, &engine, 17, Some(&profiles), None)
            .unwrap();
        let load = PreparedLoad::preflight(&callbacks.save_manager, Some(handle))
            .unwrap()
            .unwrap();
        let id = SnapshotTransitionId {
            session_id: MultiplayerSessionId([8; 32]),
            sequence: 1,
        };
        let mut pending = PendingSnapshotTransition::new(
            id,
            PendingSnapshotTransitionPayload::Save {
                load: SnapshotSave::Local(load),
            },
        );
        pending.commit_authenticated(id).unwrap();
        host.transport.prepare_snapshot_transition(pending);
        let committed = PreparedLoad::from_committed_snapshot(
            host.transport.take_committed_snapshot_transition().unwrap(),
        )
        .unwrap();
        assert!(committed.is_committed() && committed.slot().is_some());
        callbacks.queue_operation(SaveLoadRequest::ApplyLoad(committed));
        game.set_mission_assets(
            robin_engine::mission_assets::MissionAssetDescriptor::built_in(
                "OtherMission",
                "OtherMap",
                "OtherMap",
            )
            .unwrap(),
        )
        .unwrap();
        let mut outcome = pollster::block_on(crate::main_entry::perform_pending_save_load(
            &mut host,
            &mut game,
            &mut callbacks,
            &mut engine,
            &assets,
            &profiles,
            None,
        ));
        assert!(outcome.event.is_none() && outcome.restore().is_none());
        let transition = outcome
            .take_transition()
            .expect("different descriptor requires a mission handoff");
        engine.test_set_frame_counter(99);
        callbacks
            .save_manager
            .write_save_from_engine(&mut host, &game, index, &engine, 17, Some(&profiles), None)
            .unwrap();
        transition.validate_slot(&callbacks.save_manager).unwrap();
        assert_eq!(transition.mission_id(), 17);
        assert_eq!(transition.save().engine.frame_counter(), 41);
        let successful = crate::game_session::MissionOutcome::from_engine(
            robin_engine::campaign::Campaign::default(),
            4,
            robin_engine::engine::SimConfig::default(),
            Ok(robin_engine::game_operation::GameCode::LevelLoad),
        )
        .with_transition(Some(transition.clone()));
        assert_eq!(
            successful.transition.unwrap().save().engine.frame_counter(),
            41
        );
        let failed = crate::game_session::MissionOutcome::from_engine(
            robin_engine::campaign::Campaign::default(),
            4,
            robin_engine::engine::SimConfig::default(),
            Err("exit failed".into()),
        )
        .with_transition(Some(transition.clone()));
        assert!(failed.transition.is_none());
        assert_eq!(failed.result, Err("exit failed".into()));
        let request = SaveLoadRequest::ApplyLoad(transition.into_load());
        callbacks.queue_operation(request);
        let Some(SaveLoadRequest::ApplyLoad(load)) = callbacks.pending_request() else {
            panic!("prepared handoff lost")
        };
        assert_eq!(load.save().engine.frame_counter(), 41);
    }

    #[test]
    fn remote_admission_requires_exact_committed_transport_instance_and_cannot_deserialize() {
        let directory = tempfile::tempdir().unwrap();
        let (mut callbacks, mut host, engine, _assets, _game, _profiles) =
            diagnostic_callback_fixture(directory.path());
        let save = crate::save_file::GameSaveFile::capture(&engine, &host, 17, "remote".into());
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
        assert!(
            host.transport
                .take_committed_snapshot_transition()
                .is_none()
        );
        assert!(
            host.transport
                .commit_snapshot_transition(SnapshotTransitionId { sequence: 2, ..id })
                .is_err()
        );
        host.transport.commit_snapshot_transition(id).unwrap();
        assert!(host.transport.commit_snapshot_transition(id).is_err());
        let token = host.transport.take_committed_snapshot_transition().unwrap();
        assert!(
            serde_json::from_slice::<crate::host::CommittedSnapshotTransition>(
                &serde_json::to_vec(&token).unwrap()
            )
            .is_err()
        );
        let load = PreparedLoad::from_committed_snapshot(token).unwrap();
        assert!(load.is_committed());
        assert!(load.slot().is_none());
        load.validate_slot(&callbacks.save_manager).unwrap();
        let transition = PendingLevelLoad::new(load.clone());
        assert!(
            serde_json::from_slice::<PendingLevelLoad>(&serde_json::to_vec(&transition).unwrap())
                .is_err()
        );
        callbacks.queue_operation(SaveLoadRequest::ApplyLoad(load));
        callbacks.queue_operation(SaveLoadRequest::QuickLoad { use_backup: false });
        assert!(matches!(
            callbacks.pending_request(),
            Some(SaveLoadRequest::QuickLoad { .. })
        ));
        assert!(
            host.transport
                .take_committed_snapshot_transition()
                .is_none()
        );
    }
}
