//! Replay attempt ownership. Live writers and playback snapshots never escape
//! through mutable getters: lifecycle transitions retire all related state.

use super::{BootstrapSaveBoundary, MissionFrame, ReplayFrameOrdinal, TimelineFrame};
use crate::save_file::{GameRuntimeSnapshot, ReplaySaveIdentity};
#[cfg(test)]
use robin_engine::replay::ReplayRecorder;
use robin_engine::replay::{ReplayHeader, ReplayPlayer, ReplaySaveMarker};
use serde::{Deserialize, Serialize, Serializer};
use std::collections::BTreeMap;

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum RecordingValidity {
    Linear,
    Invalid { reason: String },
}

/// A live writer cannot coexist with a sealed header or invalidation reason.
/// Playback remains independent: recording a playback is a supported mode.
enum RecordingState {
    Inactive,
    Recording(crate::replay_recording::SharedReplayRecorder),
    Sealed(ReplayHeader),
    Invalid {
        header: Option<ReplayHeader>,
        reason: String,
    },
}

impl RecordingState {
    fn recorder(&self) -> Option<&crate::replay_recording::SharedReplayRecorder> {
        match self {
            Self::Recording(recorder) => Some(recorder),
            _ => None,
        }
    }

    fn recorder_mut(&mut self) -> Option<&mut crate::replay_recording::SharedReplayRecorder> {
        match self {
            Self::Recording(recorder) => Some(recorder),
            _ => None,
        }
    }

    fn sealed_header(&self) -> Option<&ReplayHeader> {
        match self {
            Self::Sealed(header)
            | Self::Invalid {
                header: Some(header),
                ..
            } => Some(header),
            _ => None,
        }
    }

    fn into_header(self) -> Option<ReplayHeader> {
        match self {
            Self::Recording(recorder) => Some(recorder.into_recording_header()),
            Self::Sealed(header) => Some(header),
            Self::Invalid { header, .. } => header,
            Self::Inactive => None,
        }
    }

    fn validity(&self) -> RecordingValidity {
        match self {
            Self::Invalid { reason, .. } => RecordingValidity::Invalid {
                reason: reason.clone(),
            },
            _ => RecordingValidity::Linear,
        }
    }
}

pub(super) struct ReplayLifecycle {
    recording: RecordingState,
    bootstrap_save: Option<(ReplaySaveIdentity, ReplaySaveMarker)>,
    // Runtime/session markers (including bootstrap), not the durable archive
    // boundaries owned by SharedReplayRecorder. Keep these separate: a session
    // restart identity cannot be reopened by a later process.
    saved_frames: BTreeMap<ReplaySaveIdentity, (ReplayFrameOrdinal, TimelineFrame)>,
    player: Option<ReplayPlayer>,
    pinned_saves: BTreeMap<u32, GameRuntimeSnapshot>,
    control: crate::replay_service::ReplayRecordingControl,
    initial_state: Option<(
        robin_engine::engine::Engine,
        GameRuntimeSnapshot,
        super::super::session_policy::SessionModalScheduler,
    )>,
}

impl ReplayLifecycle {
    pub(super) fn new(
        recorder: Option<crate::replay_recording::SharedReplayRecorder>,
        player: Option<ReplayPlayer>,
        control: crate::replay_service::ReplayRecordingControl,
    ) -> Self {
        Self {
            recording: recorder.map_or(RecordingState::Inactive, RecordingState::Recording),
            bootstrap_save: None,
            saved_frames: BTreeMap::new(),
            player,
            pinned_saves: BTreeMap::new(),
            control,
            initial_state: None,
        }
    }

    pub(super) fn next_ordinal(&self) -> Option<u32> {
        self.recording
            .recorder()
            .map(|recorder| recorder.next_ordinal())
    }

    pub(super) fn commit_restore_boundary(
        &self,
        timeline: TimelineFrame,
        hash: u64,
        recording_index: &crate::mission_replays::RecordingIndex,
    ) -> Result<u32, String> {
        self.recording
            .recorder()
            .expect("active archive restore")
            .commit_restore_boundary(timeline.number(), hash, recording_index)
            .map_err(|error| format!("{error:#}"))
    }

    pub(super) fn restore_archive(
        &mut self,
        snapshot: &[u8],
    ) -> Result<Option<crate::replay_recording::ReplayRestoreBoundary>, String> {
        let recorder = self
            .recording
            .recorder()
            .cloned()
            .or_else(|| self.control.capture_recorder());
        let Some(recorder) = recorder.filter(|recorder| recorder.has_archive()) else {
            return Ok(None);
        };
        let save: crate::save_file::GameSaveFile =
            serde_json::from_slice(snapshot).map_err(|error| error.to_string())?;
        let boundary = recorder
            .restore(&save, &self.control)
            .map_err(|error| format!("{error:#}"))?;
        self.recording = RecordingState::Recording(recorder);
        self.saved_frames.clear();
        Ok(Some(boundary))
    }

    pub(super) fn is_recording(&self) -> bool {
        matches!(self.recording, RecordingState::Recording(_))
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
        &self,
        target: TimelineFrame,
    ) -> Result<Option<ReplayFrameOrdinal>, String> {
        self.player
            .as_ref()
            .map(|player| player.resolve_timeline_frame(target))
            .transpose()
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
        if ordinal == ReplayFrameOrdinal::ZERO && self.initial_state.is_none() {
            let mut modals = super::super::session_policy::SessionModalScheduler::default();
            modals.checkpoint(0, &host.effects);
            self.initial_state = Some((
                manager.engine.clone(),
                GameRuntimeSnapshot::capture(&manager.engine, host, game)
                    .map_err(|error| format!("capture replay start: {error:#}"))?,
                modals,
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

    pub(super) fn restore_initial(
        &mut self,
        manager: &mut robin_engine::engine_manager::EngineManager,
        host: &mut crate::host::Host,
        game: &mut crate::game::Game,
        assets: &robin_engine::engine::LevelAssets,
    ) -> Result<(), String> {
        let (engine, snapshot, modals) = self
            .initial_state
            .as_mut()
            .ok_or("replay start has not been captured")?;
        snapshot
            .clone()
            .apply_to_with_game(&mut manager.engine, host, game, assets)
            .map_err(|error| format!("restore replay start: {error}"))?;
        // Seeking is rollback, not a save load: retain the exact pre-frame-zero
        // engine, including runtime queues that persisted-load reconciliation changes.
        manager.engine = engine.clone();
        game.apply_post_load_sync(false);
        game.post_load_resolution_resync();
        modals.restore(0, &mut host.effects);
        self.player
            .as_mut()
            .ok_or("no active replay")?
            .seek_ordinal(ReplayFrameOrdinal::ZERO);
        self.pinned_saves.clear();
        Ok(())
    }

    pub(super) fn saved_frame(
        &self,
        identity: ReplaySaveIdentity,
    ) -> Option<(ReplayFrameOrdinal, TimelineFrame)> {
        self.recording
            .recorder()
            .and_then(|recorder| recorder.captured_frame(identity))
            .map(|(ordinal, timeline)| {
                (
                    ReplayFrameOrdinal::from_wire(ordinal),
                    TimelineFrame::from_wire(timeline),
                )
            })
            .or_else(|| self.saved_frames.get(&identity).copied())
    }

    pub(super) fn record_save(
        &mut self,
        identity: ReplaySaveIdentity,
        ordinal: ReplayFrameOrdinal,
        timeline: TimelineFrame,
        hash: u64,
    ) {
        let Some(recorder) = self.recording.recorder_mut() else {
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
        self.recording
            .recorder_mut()
            .expect("load-back requires an active recording")
            .write_load_back(ordinal.number(), target.number(), is_continue);
    }

    pub(super) fn record_load_snapshot(
        &mut self,
        ordinal: ReplayFrameOrdinal,
        snapshot: Vec<u8>,
        timeline: TimelineFrame,
        is_continue: bool,
    ) {
        self.recording
            .recorder_mut()
            .expect("snapshot load requires an active recording")
            .write_load_snapshot(ordinal.number(), snapshot, timeline.number(), is_continue);
    }

    pub(super) fn record_taints(
        &mut self,
        ordinal: ReplayFrameOrdinal,
        taints: impl IntoIterator<Item = robin_engine::replay_rankability::InputTaintKind>,
    ) {
        if let Some(recorder) = self.recording.recorder_mut() {
            for kind in taints {
                recorder.record_input_taint(kind, ordinal.number());
            }
        }
    }

    pub(super) fn write_frame(
        &mut self,
        ordinal: ReplayFrameOrdinal,
        before: TimelineFrame,
        after: TimelineFrame,
        input: robin_engine::engine::SimulationFrameInput,
        controls: Vec<robin_engine::replay::ReplayHostControl>,
        hash: Option<u64>,
    ) -> bool {
        self.recording
            .recorder_mut()
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
        self.control.checkpoint_ranked_input();
        self.recording = match std::mem::replace(&mut self.recording, RecordingState::Inactive) {
            RecordingState::Recording(recorder) => {
                RecordingState::Sealed(recorder.into_recording_header())
            }
            state => state,
        };
    }

    pub(super) fn invalidate(&mut self, reason: impl Into<String>) {
        let reason = reason.into();
        let recording = std::mem::replace(&mut self.recording, RecordingState::Inactive);
        self.recording = RecordingState::Invalid {
            header: recording.into_header(),
            reason: reason.clone(),
        };
        self.saved_frames.clear();
        self.control.invalidate(reason.clone());
        tracing::warn!("{reason}");
    }

    /// Start a new attempt after a terminal record using a bootstrap marker
    /// or an embedded save captured before restoration.
    /// The caller owns the required applied-load snapshot; a matching bootstrap
    /// identity still lets the recorder use its marker instead of embedding bytes.
    pub(super) fn reopen_after_restore(
        &mut self,
        identity: ReplaySaveIdentity,
        recording_index: &crate::mission_replays::RecordingIndex,
    ) -> bool {
        let Some(header) = self.recording.sealed_header() else {
            return false;
        };
        let bootstrap = self.bootstrap_save.filter(|(saved, _)| *saved == identity);
        match crate::game_session::replay_init::restart_recording(
            &self.control,
            recording_index,
            header.clone(),
        ) {
            Ok(mut recorder) => {
                if let Some((_, marker)) = bootstrap {
                    recorder.write_save_marker(0, marker);
                }
                self.recording = RecordingState::Recording(recorder.into());
                self.saved_frames.clear();
                if bootstrap.is_some() {
                    self.saved_frames
                        .insert(identity, (ReplayFrameOrdinal::ZERO, TimelineFrame::ZERO));
                }
                tracing::info!("Recording restarted at save restore boundary");
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
        if let Some(recorder) = self.recording.recorder() {
            if recorder.has_archive() {
                // The central capture hook already wrote the durable frame-zero marker.
                self.bootstrap_save = Some((identity, marker));
                return;
            }
        }
        self.record_save(identity, ordinal, TimelineFrame::ZERO, marker.state_hash);
        self.bootstrap_save = Some((identity, marker));
    }

    #[cfg(test)]
    pub(super) fn install_test_recorder(&mut self, recorder: ReplayRecorder) {
        assert!(matches!(self.recording, RecordingState::Inactive));
        assert!(self.saved_frames.is_empty());
        self.recording = RecordingState::Recording(recorder.into());
    }

    #[cfg(test)]
    pub(super) fn install_test_player(&mut self, player: ReplayPlayer) {
        assert!(self.player.is_none());
        assert!(self.pinned_saves.is_empty());
        self.player = Some(player);
    }

    #[cfg(test)]
    pub(super) fn validity(&self) -> RecordingValidity {
        self.recording.validity()
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
        self.recording.sealed_header().is_some()
    }
}

impl Serialize for ReplayLifecycle {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("ReplayLifecycle", 5)?;
        state.serialize_field("is_recording", &self.is_recording())?;
        state.serialize_field("validity", &self.recording.validity())?;
        state.serialize_field("is_playing", &self.player.is_some())?;
        state.serialize_field("saved_frames", &self.saved_frames.len())?;
        state.serialize_field("pinned_saves", &self.pinned_saves.len())?;
        state.end()
    }
}

robin_util::deny_deserialize!(
    ReplayLifecycle,
    "replay lifecycle is live mission authority, not saved game state"
);

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
        ReplayLifecycle::new(Some(recorder.into()), None, service.recording())
    }

    #[test]
    fn sealing_and_invalidation_are_idempotent_without_creating_a_writer() {
        let service = Arc::new(crate::replay_service::ReplayService::default());
        let mut inactive = ReplayLifecycle::new(None, None, service.recording());
        inactive.seal();
        inactive.invalidate("disabled");
        inactive.seal();
        assert!(!inactive.is_recording());
        assert!(!inactive.has_sealed_header());
        assert_eq!(
            inactive.validity(),
            RecordingValidity::Invalid {
                reason: "disabled".into()
            }
        );
        assert!(!inactive.reopen_after_restore(
            ReplaySaveIdentity::SessionRestart(1),
            &crate::mission_replays::RecordingIndex::disabled()
        ));

        let mut lifecycle = recording(&service);
        lifecycle.seal();
        lifecycle.seal();
        assert!(!lifecycle.is_recording());
        assert!(lifecycle.has_sealed_header());
        lifecycle.invalidate("first");
        lifecycle.invalidate("second");
        lifecycle.seal();
        assert!(lifecycle.has_sealed_header());
        assert!(!lifecycle.is_recording());
        assert_eq!(
            lifecycle.validity(),
            RecordingValidity::Invalid {
                reason: "second".into()
            }
        );
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
        assert!(lifecycle.has_sealed_header());
        assert!(service.exports().snapshot().is_err());
        assert!(
            lifecycle
                .reopen_after_restore(later, &crate::mission_replays::RecordingIndex::disabled())
        );
        assert!(
            lifecycle.saved_frames.is_empty(),
            "external restore cannot recreate retired marker authority"
        );
        lifecycle.invalidate("injected subsequent recording failure");
        assert!(lifecycle.reopen_after_restore(
            bootstrap,
            &crate::mission_replays::RecordingIndex::disabled()
        ));
        assert!(lifecycle.is_recording());
        assert_eq!(lifecycle.validity(), RecordingValidity::Linear);
        assert!(!lifecycle.has_sealed_header());
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
        let playback = ReplayLifecycle::new(None, Some(player), service.recording());
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
