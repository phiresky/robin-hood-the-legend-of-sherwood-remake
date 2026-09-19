//! Local and validator-generated ordinal checkpoints share one storage path.
use super::*;
use crate::game_session::session_policy::{
    ModalCheckpoint, ModalDecisionSource, ReplayModalDismissals, SessionModalScheduler,
};
use crate::{
    game::Game,
    host::{Host, HostEffectBatches},
    save_file::ReplayPresentationSnapshot,
};
use robin_engine::{
    engine::{CompressedEngineSnapshot, LevelAssets},
    engine_manager::EngineManager,
    replay::{ReplayData, ReplayHostControl, state_hash},
};
use robin_replay_format::seek::{INTERVAL, ReplaySeekSidecar, SeekSnapshot};

struct Checkpoint {
    timeline: TimelineFrame,
    engine: CompressedEngineSnapshot,
    presentation: ReplayPresentationSnapshot,
    modals: ModalCheckpoint,
}

#[derive(Default)]
pub(super) struct ReplaySeekCache {
    checkpoints: BTreeMap<u32, Checkpoint>,
    saves: BTreeMap<u32, CompressedGameRuntimeSnapshot>,
}

fn decode(snapshot: &SeekSnapshot) -> Result<Engine, String> {
    let engine = Engine::decode_native_snapshot(&snapshot.engine)?;
    if state_hash(&engine) != snapshot.state_hash {
        return Err("seek snapshot state hash mismatch".into());
    }
    Ok(engine)
}

impl ReplaySeekCache {
    #[cfg(test)]
    pub(super) fn import(
        sidecar: ReplaySeekSidecar,
        replay: &ReplayData,
        host: &Host,
        game: &Game,
    ) -> Result<Self, String> {
        Self::import_from_start(
            sidecar,
            replay,
            ReplayPresentationSnapshot::capture(host, game),
            (*host.effects).clone(),
        )
    }

    pub(super) fn import_from_start(
        sidecar: ReplaySeekSidecar,
        replay: &ReplayData,
        mut presentation: ReplayPresentationSnapshot,
        initial_effects: robin_engine::engine::HostEffects,
    ) -> Result<Self, String> {
        let mut cache = Self::default();
        let mut checkpoints = sidecar.checkpoints.into_iter().peekable();
        let mut saves = sidecar.saves.into_iter().peekable();
        let mut events = sidecar.effects.into_iter().peekable();
        let mut effects = HostEffectBatches::default();
        effects.append(initial_effects);
        let mut modals = SessionModalScheduler::default();
        let mut saved_presentation = BTreeMap::new();
        for ordinal in 0..replay.frame_count() {
            if checkpoints.peek().is_some_and(|s| s.ordinal == ordinal) {
                let snapshot = checkpoints.next().unwrap();
                let engine = decode(&snapshot)?;
                cache.checkpoints.insert(
                    ordinal,
                    Checkpoint {
                        timeline: TimelineFrame::from_wire(snapshot.timeline),
                        engine: CompressedEngineSnapshot::capture(&engine)?,
                        presentation: presentation.clone(),
                        modals: modals.capture_seek(&effects),
                    },
                );
            }
            if saves.peek().is_some_and(|s| s.ordinal == ordinal) {
                let snapshot = saves.next().unwrap();
                let engine = decode(&snapshot)?;
                cache.saves.insert(
                    ordinal,
                    presentation
                        .saved_engine(&engine)
                        .map_err(|e| e.to_string())?,
                );
                saved_presentation.insert(ordinal, presentation.clone());
            }
            if let Some(load) = replay.load_back_for_frame(ordinal) {
                presentation = saved_presentation
                    .get(&load.to_frame)
                    .ok_or("missing seek save presentation")?
                    .clone();
                effects.clear();
                modals.after_load_back();
            }
            if events.peek().is_some_and(|e| e.ordinal == ordinal) {
                let event = events.next().unwrap();
                effects.append(event.effects);
                if let Some(value) = event.draw_hidden {
                    presentation.set_draw_hidden(value);
                }
            }
            let mut controls = ReplayModalDismissals::default();
            controls.begin_replay_frame();
            for control in &replay.frame(ordinal).unwrap().host_controls {
                let ReplayHostControl::ModalDismiss { modal, result } = control;
                if matches!(
                    modal,
                    robin_engine::player_command::ModalKind::FinalDebriefing { .. }
                        | robin_engine::player_command::ModalKind::MissionState {
                            kind: robin_engine::player_command::MissionStateModalKind::EndState { .. }
                        }
                ) {
                    if checkpoints.peek().is_some() {
                        return Err("terminal transition precedes a seek checkpoint".into());
                    }
                    continue;
                }
                controls.push_back(PlayerCommand::ModalDismiss {
                    kind: modal.clone(),
                    result: *result,
                });
            }
            while modals
                .advance(&mut effects, &mut controls, ModalDecisionSource::Recorded)
                .is_some()
            {
                if controls.is_empty() {
                    break;
                }
            }
            // Terminal UI controls have their own owner and never anchor a
            // later periodic checkpoint in a successful ranked recording.
            if !controls.is_empty()
                && ordinal + 1 < replay.frame_count()
                && checkpoints.peek().is_some()
            {
                return Err(format!(
                    "unmatched modal controls before seek checkpoint at {ordinal}"
                ));
            }
        }
        Ok(cache)
    }
}

impl ReplayLifecycle {
    pub(in crate::game_session) fn capture_seek_checkpoint(
        &mut self,
        timeline: TimelineFrame,
        engine: &Engine,
        host: &Host,
        game: &Game,
        modals: &SessionModalScheduler,
    ) -> Result<(), MissionError> {
        let ordinal = self.ordinal.number();
        if ordinal.is_multiple_of(INTERVAL) && !self.seek_cache.checkpoints.contains_key(&ordinal) {
            self.seek_cache.checkpoints.insert(
                ordinal,
                Checkpoint {
                    timeline,
                    engine: CompressedEngineSnapshot::capture(engine)
                        .map_err(MissionError::replay)?,
                    presentation: ReplayPresentationSnapshot::capture(host, game),
                    modals: modals.capture_seek(&host.effects),
                },
            );
        }
        self.seek_cache
            .saves
            .extend(self.pinned_saves.iter().map(|(&k, v)| (k, v.clone())));
        Ok(())
    }

    pub(in crate::game_session) fn restore_seek_checkpoint(
        &mut self,
        target: u32,
        manager: &mut EngineManager,
        host: &mut Host,
        game: &mut Game,
        assets: &LevelAssets,
        modals: Option<&mut SessionModalScheduler>,
    ) -> Result<Option<TimelineFrame>, MissionError> {
        let Some((&ordinal, checkpoint)) = self
            .seek_cache
            .checkpoints
            .range(..=target)
            .rev()
            .find(|(_, checkpoint)| modals.is_some() || checkpoint.modals.is_settled())
        else {
            return Ok(None);
        };
        let current = self
            .player
            .as_ref()
            .expect("active playback")
            .current_frame();
        if current <= target && current >= ordinal {
            return Ok(None);
        }
        let engine = checkpoint
            .engine
            .restore(assets)
            .map_err(MissionError::replay)?;
        checkpoint.presentation.restore(&engine, host, game);
        let mut interactive_modals = SessionModalScheduler::default();
        modals.unwrap_or(&mut interactive_modals).restore_seek(
            ordinal,
            checkpoint.modals.clone(),
            &mut host.effects,
        );
        tracing::info!(target, ordinal, "restored replay seek checkpoint");
        manager.engine = engine;
        self.ordinal = ReplayFrameOrdinal::from_wire(ordinal);
        self.player.as_mut().unwrap().seek_ordinal(self.ordinal);
        self.pinned_saves = self
            .seek_cache
            .saves
            .range(..ordinal)
            .map(|(&k, v)| (k, v.clone()))
            .collect();
        self.finished_logged = false;
        Ok(Some(checkpoint.timeline))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires ROBIN_SEEK_REPLAY and ROBIN_SEEK_SIDECAR from the size probe"]
    fn real_sidecar_import_reconstructs_host_checkpoints() {
        use sha2::{Digest, Sha256};
        let replay_bytes = std::fs::read(std::env::var("ROBIN_SEEK_REPLAY").unwrap()).unwrap();
        let sidecar_bytes = std::fs::read(std::env::var("ROBIN_SEEK_SIDECAR").unwrap()).unwrap();
        let (_, replay) = robin_replay_format::decode_compact(&replay_bytes).unwrap();
        let sidecar = ReplaySeekSidecar::decode(
            &sidecar_bytes,
            Sha256::digest(&replay_bytes).into(),
            &replay,
        )
        .unwrap();
        let snapshots = sidecar.checkpoints.len();
        let saves = sidecar.saves.len();
        let cache = ReplaySeekCache::import(
            sidecar,
            &replay,
            &Host::scratch(1024.0, 768.0),
            &Game::default(),
        )
        .unwrap();
        assert_eq!(cache.checkpoints.len(), snapshots);
        assert_eq!(cache.saves.len(), saves);
    }
    use robin_engine::replay::{
        REPLAY_SCHEMA_VERSION, ReplayFile, ReplayFrame, ReplayHeader, ReplayLoadBack,
        ReplaySaveMarker,
    };

    #[test]
    fn imported_checkpoints_seek_both_directions_and_retain_load_dependencies() {
        let (initial, assets) = robin_engine::test_support::fresh_engine_sized(640.0, 480.0);
        let replay: ReplayData = ReplayFile {
            header: ReplayHeader {
                mission_id: "fixture".into(),
                mission_assets: robin_engine::mission_assets::MissionAssetDescriptor::built_in(
                    "fixture", "fixture", "fixture",
                )
                .unwrap(),
                rng_seed: 0,
                sim_config: initial.sim_config(),
                spellforge_package: None,
                version: REPLAY_SCHEMA_VERSION,
                total_frames: 502,
                rankability: robin_engine::replay_rankability::ReplayRankability::rankable(),
                campaign: bitcode::encode(initial.campaign()),
            },
            frames: (0..502)
                .map(|ordinal| {
                    let timeline = if ordinal < 251 {
                        ordinal
                    } else {
                        ordinal - 251
                    };
                    (
                        ordinal,
                        ReplayFrame {
                            timeline_before: timeline,
                            timeline_after: timeline + 1,
                            input: Default::default(),
                            host_controls: if ordinal == 501 {
                                vec![ReplayHostControl::ModalDismiss {
                                    modal: robin_engine::player_command::ModalKind::MissionState {
                                        kind: robin_engine::player_command::MissionStateModalKind::EndState { won: true },
                                    },
                                    result: robin_engine::player_command::DialogResult::Completed,
                                }]
                            } else {
                                Vec::new()
                            },
                        },
                    )
                })
                .collect(),
            hashes: Default::default(),
            save_markers: BTreeMap::from([(
                0,
                ReplaySaveMarker {
                    state_hash: state_hash(&initial),
                    timeline_frame: 0,
                },
            )]),
            load_backs: BTreeMap::from([(
                251,
                ReplayLoadBack {
                    to_frame: 0,
                    is_continue: false,
                    snapshot: None,
                },
            )]),
        }
        .try_into()
        .unwrap();
        let mut sidecar = ReplaySeekSidecar::new([7; 32], &replay);
        let saved = initial.capture_persisted_state().unwrap();
        let mut engine = initial.clone();
        let mut expected = BTreeMap::new();
        for ordinal in 0..502 {
            expected.insert(ordinal, state_hash(&engine));
            sidecar.observe(&replay, ordinal, &engine, None);
            if ordinal == 251 {
                engine = Engine::restore_from_snapshot(
                    &mut Default::default(),
                    Engine::from_persisted_state(saved.clone()),
                    &assets,
                )
                .unwrap();
            }
            let output = engine
                .advance_frame(&assets, replay.frame(ordinal).unwrap().input.clone())
                .unwrap();
            sidecar.observe(&replay, ordinal, &engine, Some(&output));
        }
        let encoded = sidecar.encode().unwrap();
        assert!(ReplaySeekSidecar::decode(&encoded, [8; 32], &replay).is_err());
        let decoded = ReplaySeekSidecar::decode(&encoded, [7; 32], &replay).unwrap();
        decoded.validate_engines().unwrap();
        let mut damaged = decoded.clone();
        damaged.checkpoints[1].ordinal = 249;
        assert!(ReplaySeekSidecar::decode(&damaged.encode().unwrap(), [7; 32], &replay).is_err());
        let mut host = Host::scratch(640.0, 480.0);
        let mut game = Game::default();
        let cache = ReplaySeekCache::import(decoded, &replay, &host, &game).unwrap();
        assert_eq!(cache.checkpoints.len(), 3);
        let service = std::sync::Arc::new(crate::replay_service::ReplayService::default());
        let mut lifecycle = ReplayLifecycle::new(
            None,
            Some(ReplayPlayer::new(replay)),
            service.recording(),
            false,
        );
        lifecycle.seek_cache = cache;
        let mut manager = EngineManager::new(initial);
        let mut modals = SessionModalScheduler::default();
        for (target, headless) in [
            (501, true),
            (250, true),
            (0, true),
            (500, true),
            (250, false),
            (0, false),
            (501, false),
        ] {
            let timeline = lifecycle
                .restore_seek_checkpoint(
                    target,
                    &mut manager,
                    &mut host,
                    &mut game,
                    &assets,
                    headless.then_some(&mut modals),
                )
                .unwrap()
                .unwrap();
            let ordinal = target / INTERVAL * INTERVAL;
            assert_eq!(lifecycle.ordinal.number(), ordinal);
            assert_eq!(state_hash(&manager.engine), expected[&ordinal]);
            assert_eq!(
                timeline.number(),
                if ordinal < 251 {
                    ordinal
                } else {
                    ordinal - 251
                }
            );
            if ordinal > 0 {
                assert!(lifecycle.pinned_saves.contains_key(&0));
            }
        }
        lifecycle.pinned_saves[&0]
            .apply_to_with_game(&mut manager.engine, &mut host, &mut game, &assets)
            .unwrap();
        let restored = Engine::restore_from_snapshot(
            &mut Default::default(),
            Engine::from_persisted_state(saved),
            &assets,
        )
        .unwrap();
        assert_eq!(state_hash(&manager.engine), state_hash(&restored));
    }
}
