//! Replay attempt ownership. Live writers and playback snapshots never escape
//! through mutable getters: lifecycle transitions retire all related state.

use super::{BootstrapSaveBoundary, MissionFrame, ReplayFrameOrdinal, TimelineFrame};
use crate::save_file::{GameRuntimeSnapshot, ReplaySaveIdentity};
use robin_engine::replay::{ReplayHeader, ReplayPlayer, ReplayRecorder, ReplaySaveMarker};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum RecordingValidity {
    Linear,
    Invalid { reason: String },
}

pub(super) struct ReplayLifecycle {
    recorder: Option<ReplayRecorder>,
    sealed_header: Option<ReplayHeader>,
    validity: RecordingValidity,
    bootstrap_save: Option<(ReplaySaveIdentity, ReplaySaveMarker)>,
    saved_frames: BTreeMap<ReplaySaveIdentity, (ReplayFrameOrdinal, TimelineFrame)>,
    player: Option<ReplayPlayer>,
    pinned_saves: BTreeMap<u32, GameRuntimeSnapshot>,
    control: crate::replay_service::ReplayRecordingControl,
}

impl ReplayLifecycle {
    pub(super) fn new(
        recorder: Option<ReplayRecorder>,
        player: Option<ReplayPlayer>,
        control: crate::replay_service::ReplayRecordingControl,
    ) -> Self {
        Self {
            recorder,
            sealed_header: None,
            validity: RecordingValidity::Linear,
            bootstrap_save: None,
            saved_frames: BTreeMap::new(),
            player,
            pinned_saves: BTreeMap::new(),
            control,
        }
    }

    pub(super) fn is_recording(&self) -> bool {
        assert!(
            matches!(self.validity, RecordingValidity::Linear) || self.recorder.is_none(),
            "an invalidated recording cannot own a live recorder"
        );
        self.recorder.is_some()
    }

    pub(super) fn playback(&self) -> Option<&ReplayPlayer> {
        self.player.as_ref()
    }

    pub(super) fn inject_replay_input(&mut self, frame: &mut MissionFrame) {
        if let Some(player) = &mut self.player {
            frame.inject_replay_input(player);
        }
    }

    pub(super) fn resolve_ordinal(
        &mut self,
        target: TimelineFrame,
    ) -> Result<Option<ReplayFrameOrdinal>, String> {
        let Some(player) = &mut self.player else {
            return Ok(None);
        };
        let original = ReplayFrameOrdinal::from_wire(player.current_frame());
        let resolved = player.seek_timeline_frame(target);
        player.seek_ordinal(original);
        resolved.map(Some)
    }

    pub(super) fn seek_timeline(
        &mut self,
        target: TimelineFrame,
    ) -> Result<Option<ReplayFrameOrdinal>, String> {
        self.player
            .as_mut()
            .map(|player| player.seek_timeline_frame(target))
            .transpose()
    }

    pub(super) fn consume_step(
        &mut self,
        ordinal: ReplayFrameOrdinal,
        timeline: TimelineFrame,
    ) -> Result<super::ReplayStepAdmission, String> {
        let Some(player) = &mut self.player else {
            return Ok(super::ReplayStepAdmission::NoActiveReplay);
        };
        if player.current_frame() != ordinal.number() {
            return Err(format!(
                "replay player ordinal {} diverged from timeline runtime ordinal {}",
                player.current_frame(),
                ordinal.number()
            ));
        }
        if player.is_finished() {
            return Ok(super::ReplayStepAdmission::Finished {
                ordinal: player.current_frame(),
                total_frames: player.total_frames(),
            });
        }
        let recorded = player.next_frame().clone();
        if recorded.timeline_before != timeline.number() {
            return Err(format!(
                "replay ordinal {} starts at timeline {}, current timeline is {}",
                ordinal.number(),
                recorded.timeline_before,
                timeline.number()
            ));
        }
        Ok(super::ReplayStepAdmission::Recorded(recorded))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn apply_playback_boundary(
        &mut self,
        ordinal: ReplayFrameOrdinal,
        timeline: TimelineFrame,
        rewind: &mut crate::rewind::RewindBuffer,
        host: &mut crate::host::Host,
        game: &mut crate::game::Game,
        manager: &mut robin_engine::engine_manager::EngineManager,
        assets: &robin_engine::engine::LevelAssets,
    ) -> Result<Option<TimelineFrame>, String> {
        let Some(player) = self.player.as_ref().filter(|player| !player.is_finished()) else {
            return Ok(None);
        };
        if player.current_frame() != ordinal.number() {
            return Err(format!(
                "replay player ordinal {} diverged from timeline runtime ordinal {}",
                player.current_frame(),
                ordinal.number()
            ));
        }
        super::apply_replay_timeline_events_at_boundary(
            player,
            timeline,
            &mut self.pinned_saves,
            rewind,
            host,
            game,
            manager,
            assets,
        )
    }

    pub(super) fn saved_frame(
        &self,
        identity: ReplaySaveIdentity,
    ) -> Option<(ReplayFrameOrdinal, TimelineFrame)> {
        self.saved_frames.get(&identity).copied()
    }

    pub(super) fn record_save(
        &mut self,
        identity: ReplaySaveIdentity,
        ordinal: ReplayFrameOrdinal,
        timeline: TimelineFrame,
        hash: u64,
    ) {
        let Some(recorder) = &mut self.recorder else {
            return;
        };
        recorder.write_save_marker(
            ordinal.number(),
            ReplaySaveMarker {
                state_hash: hash,
                timeline_frame: timeline.number(),
            },
        );
        self.saved_frames.insert(identity, (ordinal, timeline));
    }

    pub(super) fn record_load_back(
        &mut self,
        ordinal: ReplayFrameOrdinal,
        target: ReplayFrameOrdinal,
        is_continue: bool,
    ) {
        self.recorder
            .as_mut()
            .expect("load-back requires an active recording")
            .write_load_back(ordinal.number(), target.number(), is_continue);
    }

    pub(super) fn record_taints(
        &mut self,
        ordinal: ReplayFrameOrdinal,
        taints: impl IntoIterator<Item = robin_engine::replay_rankability::InputTaintKind>,
    ) {
        if let Some(recorder) = &mut self.recorder {
            for kind in taints {
                recorder.record_input_taint(kind, ordinal.number());
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn write_frame(
        &mut self,
        ordinal: ReplayFrameOrdinal,
        before: TimelineFrame,
        after: TimelineFrame,
        input: robin_engine::engine::SimulationFrameInput,
        controls: Vec<robin_engine::replay::ReplayHostControl>,
        hash: Option<u64>,
    ) -> bool {
        self.recorder
            .as_mut()
            .expect("open recorder frame lost its recorder owner")
            .write_frame(
                ordinal.number(),
                before.number(),
                after.number(),
                input,
                controls,
                hash,
            )
    }

    pub(super) fn seal(&mut self) {
        if let Some(recorder) = self.recorder.take() {
            self.sealed_header = Some(recorder.into_recording_header());
        }
    }

    pub(super) fn invalidate(&mut self, reason: impl Into<String>) {
        let reason = reason.into();
        self.seal();
        self.saved_frames.clear();
        self.control.invalidate(reason.clone());
        tracing::warn!("{reason}");
        self.validity = RecordingValidity::Invalid { reason };
    }

    /// Only the original, successfully persisted bootstrap boundary can open
    /// a new linear attempt after a terminal record or foreign-save load.
    pub(super) fn reopen_after_restore(&mut self, identity: ReplaySaveIdentity) -> bool {
        let Some(header) = self.sealed_header.as_ref() else {
            return false;
        };
        let Some((_, marker)) = self.bootstrap_save.filter(|(saved, _)| *saved == identity) else {
            // TODO(replay): arbitrary saves need an embedded initial snapshot.
            self.invalidate("replay unavailable after post-terminal load of a non-bootstrap save");
            return false;
        };
        match crate::game_session::replay_init::restart_recording(&self.control, header.clone()) {
            Ok(mut recorder) => {
                recorder.write_save_marker(0, marker);
                self.recorder = Some(recorder);
                self.validity = RecordingValidity::Linear;
                self.sealed_header = None;
                self.saved_frames.clear();
                self.saved_frames
                    .insert(identity, (ReplayFrameOrdinal::ZERO, TimelineFrame::ZERO));
                tracing::info!("Recording restarted mission from its bootstrap save boundary");
                true
            }
            Err(error) => {
                self.invalidate(format!(
                    "replay unavailable: could not start restarted recording: {error}"
                ));
                false
            }
        }
    }

    pub(super) fn register_bootstrap(
        &mut self,
        ordinal: ReplayFrameOrdinal,
        completed: Option<BootstrapSaveBoundary>,
    ) {
        let Some(BootstrapSaveBoundary { identity, marker }) = completed else {
            return;
        };
        if !self.is_recording() {
            return;
        }
        assert_eq!(
            ordinal,
            ReplayFrameOrdinal::ZERO,
            "bootstrap save must be registered before the first recorded frame"
        );
        self.record_save(identity, ordinal, TimelineFrame::ZERO, marker.state_hash);
        self.bootstrap_save = Some((identity, marker));
    }

    #[cfg(test)]
    pub(super) fn install_test_recorder(&mut self, recorder: ReplayRecorder) {
        assert!(self.recorder.is_none());
        assert!(self.sealed_header.is_none());
        assert!(self.saved_frames.is_empty());
        self.recorder = Some(recorder);
    }

    #[cfg(test)]
    pub(super) fn install_test_player(&mut self, player: ReplayPlayer) {
        assert!(self.player.is_none());
        assert!(self.pinned_saves.is_empty());
        self.player = Some(player);
    }

    #[cfg(test)]
    pub(super) fn validity(&self) -> &RecordingValidity {
        &self.validity
    }

    #[cfg(test)]
    pub(super) fn saved_frame_count(&self) -> usize {
        self.saved_frames.len()
    }

    #[cfg(test)]
    pub(super) fn has_pinned_save(&self, frame: u32) -> bool {
        self.pinned_saves.contains_key(&frame)
    }

    #[cfg(test)]
    pub(super) fn has_sealed_header(&self) -> bool {
        self.sealed_header.is_some()
    }
}

impl Serialize for ReplayLifecycle {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("ReplayLifecycle", 5)?;
        state.serialize_field("is_recording", &self.is_recording())?;
        state.serialize_field("validity", &self.validity)?;
        state.serialize_field("is_playing", &self.player.is_some())?;
        state.serialize_field("saved_frames", &self.saved_frames.len())?;
        state.serialize_field("pinned_saves", &self.pinned_saves.len())?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for ReplayLifecycle {
    fn deserialize<D: Deserializer<'de>>(_deserializer: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "replay lifecycle is live mission authority, not saved game state",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn recording(service: &Arc<crate::replay_service::ReplayService>) -> ReplayLifecycle {
        let recorder = ReplayRecorder::with_writer(
            Box::new(service.recording().begin_recording()),
            "lifecycle".into(),
            robin_engine::mission_assets::MissionAssetDescriptor::built_in(
                "lifecycle",
                "lifecycle",
                "lifecycle",
            )
            .unwrap(),
            0,
            Default::default(),
            &Default::default(),
        )
        .unwrap();
        ReplayLifecycle::new(Some(recorder), None, service.recording())
    }

    #[test]
    fn invalidation_retires_save_authority_and_recovery_replaces_it_atomically() {
        let service = Arc::new(crate::replay_service::ReplayService::default());
        let mut lifecycle = recording(&service);
        let bootstrap = ReplaySaveIdentity::SessionRestart(1);
        let later = ReplaySaveIdentity::Payload([2; 32]);
        lifecycle.register_bootstrap(
            ReplayFrameOrdinal::ZERO,
            Some(BootstrapSaveBoundary {
                identity: bootstrap,
                marker: ReplaySaveMarker {
                    state_hash: 17,
                    timeline_frame: 0,
                },
            }),
        );
        // Save markers belong to the next dense record boundary, not an
        // arbitrary future ordinal. Advance a real attempt before saving.
        for frame in 0..3 {
            assert!(lifecycle.write_frame(
                ReplayFrameOrdinal::from_wire(frame),
                TimelineFrame::from_wire(frame),
                TimelineFrame::from_wire(frame + 1),
                robin_engine::engine::SimulationFrameInput::default().with_hourglass(true),
                Vec::new(),
                None,
            ));
        }
        lifecycle.record_save(
            later,
            ReplayFrameOrdinal::from_wire(3),
            TimelineFrame::from_wire(3),
            99,
        );
        assert!(lifecycle.write_frame(
            ReplayFrameOrdinal::from_wire(3),
            TimelineFrame::from_wire(3),
            TimelineFrame::from_wire(4),
            robin_engine::engine::SimulationFrameInput::default().with_hourglass(true),
            Vec::new(),
            None,
        ));
        let valid_attempt = service.exports().snapshot().unwrap().parse_sync().unwrap();
        assert_eq!(valid_attempt.frame_count(), 4);
        assert!(valid_attempt.save_marker_for_frame(3).is_some());
        lifecycle.invalidate("foreign save");
        assert!(!lifecycle.is_recording());
        assert!(lifecycle.saved_frames.is_empty());
        assert!(lifecycle.sealed_header.is_some());
        assert!(service.exports().snapshot().is_err());
        assert!(!lifecycle.reopen_after_restore(later));
        assert!(lifecycle.reopen_after_restore(bootstrap));
        assert!(lifecycle.is_recording());
        assert_eq!(lifecycle.validity, RecordingValidity::Linear);
        assert!(lifecycle.sealed_header.is_none());
        assert_eq!(
            lifecycle.saved_frame(bootstrap),
            Some((ReplayFrameOrdinal::ZERO, TimelineFrame::ZERO))
        );
        assert_eq!(lifecycle.saved_frames.len(), 1);
        assert_eq!(lifecycle.saved_frame(later), None);
    }

    #[test]
    fn replay_seek_probe_preserves_cursor_on_success_and_failure() {
        let service = Arc::new(crate::replay_service::ReplayService::default());
        let mut lifecycle = recording(&service);
        for frame in 0..2 {
            assert!(lifecycle.write_frame(
                ReplayFrameOrdinal::from_wire(frame),
                TimelineFrame::from_wire(frame),
                TimelineFrame::from_wire(frame + 1),
                robin_engine::engine::SimulationFrameInput::default().with_hourglass(true),
                Vec::new(),
                None,
            ));
        }
        lifecycle.seal();
        let mut player =
            ReplayPlayer::new(service.exports().snapshot().unwrap().parse_sync().unwrap());
        player.seek_ordinal(ReplayFrameOrdinal::from_wire(2));
        let mut playback = ReplayLifecycle::new(None, Some(player), service.recording());
        assert_eq!(
            playback.resolve_ordinal(TimelineFrame::ZERO).unwrap(),
            Some(ReplayFrameOrdinal::ZERO)
        );
        assert_eq!(playback.playback().unwrap().current_frame(), 2);
        assert!(
            playback
                .resolve_ordinal(TimelineFrame::from_wire(99))
                .is_err()
        );
        assert_eq!(playback.playback().unwrap().current_frame(), 2);
    }

    #[test]
    fn diagnostic_serde_cannot_recreate_replay_authority() {
        let service = Arc::new(crate::replay_service::ReplayService::default());
        let lifecycle = recording(&service);
        let json = serde_json::to_value(&lifecycle).unwrap();
        assert_eq!(json["is_recording"], true);
        assert!(serde_json::from_value::<ReplayLifecycle>(json).is_err());
    }
}
