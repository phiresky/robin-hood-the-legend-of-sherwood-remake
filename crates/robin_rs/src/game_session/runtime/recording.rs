//! Replay attempt ownership. Live writers and playback snapshots never escape
//! through mutable getters: lifecycle transitions retire all related state.

mod seek;

use super::{
    BootstrapSaveBoundary, MissionFrame, RecorderFrameState, ReplayFrameOrdinal, TimelineFrame,
};
use crate::game_session::MissionError;
use crate::save_file::{CompressedGameRuntimeSnapshot, ReplaySaveIdentity};
use robin_engine::engine::Engine;
use robin_engine::player_command::PlayerCommand;
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
            Self::Recording(recorder) => Some(recorder.seal()),
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

/// Borrowed frame boundary that one applied save load rewrites.
pub(super) struct RestoreBoundary<'a> {
    pub(super) recording_index: &'a crate::mission_replays::RecordingIndex,
    pub(super) frame: &'a mut MissionFrame,
    pub(super) engine: &'a Engine,
}

pub(in crate::game_session) struct ReplayLifecycle {
    /// Dense host-record position, separate from the lockstep cursor owned
    /// by `TimelineRuntime`.
    pub(super) ordinal: ReplayFrameOrdinal,
    start_paused: bool,
    pub(in crate::game_session) recompute_marker_hashes: bool,
    pub(in crate::game_session) finished_logged: bool,
    recording: RecordingState,
    bootstrap_save: Option<(ReplaySaveIdentity, ReplaySaveMarker)>,
    // Runtime/session markers (including bootstrap), not the durable archive
    // boundaries owned by SharedReplayRecorder. Keep these separate: a session
    // restart identity cannot be reopened by a later process.
    saved_frames: BTreeMap<ReplaySaveIdentity, (ReplayFrameOrdinal, TimelineFrame)>,
    player: Option<ReplayPlayer>,
    pinned_saves: BTreeMap<u32, CompressedGameRuntimeSnapshot>,
    control: crate::replay_service::ReplayRecordingControl,
    initial_state: Option<(
        robin_engine::engine::CompressedEngineSnapshot,
        CompressedGameRuntimeSnapshot,
        super::super::session_policy::SessionModalScheduler,
    )>,
    seek_cache: seek::ReplaySeekCache,
}

impl ReplayLifecycle {
    pub(super) fn new(
        recorder: Option<crate::replay_recording::SharedReplayRecorder>,
        player: Option<ReplayPlayer>,
        control: crate::replay_service::ReplayRecordingControl,
        start_paused: bool,
    ) -> Self {
        Self {
            ordinal: ReplayFrameOrdinal::ZERO,
            start_paused,
            recompute_marker_hashes: false,
            finished_logged: false,
            recording: recorder.map_or(RecordingState::Inactive, RecordingState::Recording),
            bootstrap_save: None,
            saved_frames: BTreeMap::new(),
            player,
            pinned_saves: BTreeMap::new(),
            control,
            initial_state: None,
            seek_cache: Default::default(),
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
    ) -> Result<u32, MissionError> {
        self.recording
            .recorder()
            .expect("active archive restore")
            .commit_restore_boundary(timeline.number(), hash, recording_index)
            .map_err(|error| MissionError::replay(format!("{error:#}")))
    }

    pub(super) fn restore_archive(
        &mut self,
        snapshot: &[u8],
    ) -> Result<Option<crate::replay_recording::ReplayRestoreBoundary>, MissionError> {
        let recorder = self
            .recording
            .recorder()
            .cloned()
            .or_else(|| self.control.capture_recorder());
        let Some(recorder) = recorder.filter(|recorder| recorder.has_archive()) else {
            return Ok(None);
        };
        let save: crate::save_file::GameSaveFile = serde_json::from_slice(snapshot)
            .map_err(|error| MissionError::replay(error.to_string()))?;
        let boundary = recorder
            .restore(&save, &self.control)
            .map_err(|error| MissionError::replay(format!("{error:#}")))?;
        self.recording = RecordingState::Recording(recorder);
        self.saved_frames.clear();
        Ok(Some(boundary))
    }

    pub(in crate::game_session) fn is_recording(&self) -> bool {
        matches!(self.recording, RecordingState::Recording(_))
    }

    pub(in crate::game_session) fn playback(&self) -> Option<&ReplayPlayer> {
        self.player.as_ref()
    }

    pub(in crate::game_session) fn start_paused(&self) -> bool {
        self.start_paused
    }

    /// Recorder cadence sample for the current host ordinal: only an active
    /// recording hashes, and only at every 25th dense record.
    pub(super) fn recorder_hash(&self, engine: &Engine) -> Option<u64> {
        self.is_recording().then_some(()).and_then(|_| {
            self.ordinal
                .number()
                .is_multiple_of(25)
                .then(|| robin_engine::replay::state_hash(engine))
        })
    }

    /// Save capture may insert host-only records during the open input phase.
    /// Resample cadence at the new ordinal before queued commands are applied.
    pub(in crate::game_session) fn synchronize_save_boundary(
        &mut self,
        frame: &mut MissionFrame,
        engine: &Engine,
    ) {
        if let Some(ordinal) = self.next_ordinal()
            && ordinal != self.ordinal.number()
        {
            self.ordinal = ReplayFrameOrdinal::from_wire(ordinal);
            frame.recorder_hash = ordinal
                .is_multiple_of(25)
                .then(|| robin_engine::replay::state_hash(engine));
        }
    }

    pub(in crate::game_session) fn begin_recording(
        &mut self,
        frame: &mut MissionFrame,
        enabled: bool,
    ) {
        if !self.is_recording() || !enabled {
            return;
        }
        frame.open_recording();
    }

    /// Seal the recorder after the deterministic quit-mission update has
    /// committed. Ranked replay validation requires that terminal command to
    /// be the final replay record; narrative/stat modal dismissals are host UI
    /// and must not extend the canonical simulation artifact.
    pub(in crate::game_session) fn seal_terminal_recording(
        &mut self,
        frame: &MissionFrame,
    ) -> bool {
        let terminal_count = frame
            .commands
            .commands
            .iter()
            .chain(frame.post_commands.commands.iter())
            .filter(|input| {
                matches!(
                    &input.command,
                    PlayerCommand::ApplyQuitMissionUpdates { .. }
                )
            })
            .count();
        if terminal_count == 0 {
            return false;
        }
        assert_eq!(
            terminal_count, 1,
            "one mission frame cannot contain multiple terminal updates"
        );
        assert_eq!(
            frame.recorder_state,
            RecorderFrameState::Finished,
            "terminal replay must be sealed after recorder finalization"
        );
        if self.is_recording() {
            self.seal();
            tracing::debug!(
                replay_ordinal = self.ordinal.number(),
                "sealed canonical replay at terminal mission record"
            );
        }
        true
    }

    /// A completed in-mission save at the lockstep cursor `timeline`.
    ///
    /// A save at a clean pre-command boundary becomes a save-marker record
    /// (state hash + frame) so a later load of it can be expressed as a
    /// load-back.
    pub(super) fn note_save_written(
        &mut self,
        identity: ReplaySaveIdentity,
        timeline: TimelineFrame,
        frame: &mut MissionFrame,
        engine: &Engine,
    ) {
        let replay_ordinal = self.ordinal;
        if !self.is_recording() {
            return;
        }
        if self.saved_frame(identity).is_some() {
            self.synchronize_save_boundary(frame, engine);
            return;
        }
        // Gameplay input remains queued until after save processing.
        // A nonempty command queue is not evidence of a mid-frame capture.
        let hash = robin_engine::replay::state_hash(engine);
        let marker_timeline = timeline;
        self.record_save(identity, replay_ordinal, marker_timeline, hash);
        tracing::info!(
            replay_ordinal = replay_ordinal.number(),
            timeline_frame = marker_timeline.number(),
            hash = format!("{hash:016x}"),
            "replay: save marker recorded"
        );
    }

    /// A completed in-mission load, applied while the lockstep cursor was at
    /// `current`.
    ///
    /// A load drops commands already dispatched this frame (their effects were
    /// overwritten wholesale) and records a load-back to the linked mission
    /// archive marker, including saves made by earlier processes.
    /// `rebase_history` resets every reconstruction consumer (cursor, network
    /// inputs, rewind history) onto its target; it is called exactly once at
    /// the same point of each path that the old single-owner body reset them.
    pub(super) fn note_load_applied(
        &mut self,
        snapshot: Vec<u8>,
        identity: ReplaySaveIdentity,
        is_continue: bool,
        current: TimelineFrame,
        boundary: RestoreBoundary<'_>,
        rebase_history: impl FnOnce(TimelineFrame),
    ) {
        match self.restore_archive(&snapshot) {
            Ok(Some(restored)) => {
                self.adopt_archive_restore(
                    restored,
                    snapshot,
                    is_continue,
                    boundary,
                    rebase_history,
                );
            }
            Ok(None) => self.record_session_load(
                snapshot,
                identity,
                is_continue,
                current,
                boundary,
                rebase_history,
            ),
            Err(error) => {
                self.invalidate(format!("replay history unavailable after load: {error}"));
                boundary.frame.recorder_state = RecorderFrameState::Inactive;
                boundary.frame.recorder_hash = None;
            }
        }
    }

    /// The durable mission archive resolved the loaded save to a restore
    /// boundary: continue its attempt at the archive's ordinal and timeline.
    fn adopt_archive_restore(
        &mut self,
        restored: crate::replay_recording::ReplayRestoreBoundary,
        snapshot: Vec<u8>,
        is_continue: bool,
        boundary: RestoreBoundary<'_>,
        rebase_history: impl FnOnce(TimelineFrame),
    ) {
        let RestoreBoundary {
            recording_index,
            frame,
            engine,
        } = boundary;
        let crate::replay_recording::ReplayRestoreBoundary {
            ordinal,
            timeline_frame: timeline,
            marker_ordinal: target,
        } = restored;
        self.ordinal = ReplayFrameOrdinal::from_wire(ordinal);
        let timeline = TimelineFrame::from_wire(timeline);
        rebase_history(timeline);
        let recorder_state = frame.recorder_state;
        frame.reset_after_terminal_restore(robin_engine::replay::state_hash(engine));
        frame.recorder_state = recorder_state;
        frame.rebind_timeline_after_discontinuity(timeline);
        frame.recorder_hash = ordinal
            .is_multiple_of(25)
            .then(|| robin_engine::replay::state_hash(engine));
        if let Some(target) = target {
            self.record_load_back(
                self.ordinal,
                ReplayFrameOrdinal::from_wire(target),
                is_continue,
            );
        } else {
            self.record_load_snapshot(self.ordinal, snapshot, timeline, is_continue);
        }
        self.record_taints([robin_engine::replay_rankability::InputTaintKind::StateLoad]);
        match self.commit_restore_boundary(
            timeline,
            robin_engine::replay::state_hash(engine),
            recording_index,
        ) {
            Ok(next) => {
                self.ordinal = ReplayFrameOrdinal::from_wire(next);
                frame.recorder_hash = next
                    .is_multiple_of(25)
                    .then(|| robin_engine::replay::state_hash(engine));
            }
            Err(error) => {
                self.invalidate(format!("failed to persist replay restore: {error}"));
                frame.recorder_state = RecorderFrameState::Inactive;
                frame.recorder_hash = None;
            }
        }
    }

    /// No archive boundary: reopen a sealed attempt if needed, then express
    /// the load as a load-back to a session save marker or an embedded
    /// snapshot.
    fn record_session_load(
        &mut self,
        snapshot: Vec<u8>,
        identity: ReplaySaveIdentity,
        is_continue: bool,
        current: TimelineFrame,
        boundary: RestoreBoundary<'_>,
        rebase_history: impl FnOnce(TimelineFrame),
    ) {
        let RestoreBoundary {
            recording_index,
            frame,
            engine,
        } = boundary;
        let reopened = self.reopen_after_restore(identity, recording_index);
        if reopened {
            self.ordinal = ReplayFrameOrdinal::ZERO;
            // The load replaced every effect admitted before it. Keep
            // this host frame's scheduling flags, but give the new
            // attempt a clean recording transaction.
            frame.reset_after_terminal_restore(robin_engine::replay::state_hash(engine));
        }
        let replay_ordinal = self.ordinal;
        self.record_taints([robin_engine::replay_rankability::InputTaintKind::StateLoad]);
        // The engine state jumped; buffered rewind history no longer
        // describes this timeline's future.
        let recorded_save = self.saved_frame(identity);
        let target = recorded_save.map_or(current, |(_, timeline)| timeline);
        rebase_history(target);
        frame.rebind_timeline_after_discontinuity(target);
        if !frame.commands().is_empty() {
            tracing::debug!(
                dropped = frame.commands().len(),
                "replay: dropping commands dispatched before the load; \
                 their effects were overwritten by the loaded state"
            );
            frame.discard_commands();
        }
        if !self.is_recording() {
            frame.recorder_state = RecorderFrameState::Inactive;
            frame.recorder_hash = None;
            return;
        }
        if let Some((to_ordinal, _)) = recorded_save {
            frame.recorder_hash = replay_ordinal
                .number()
                .is_multiple_of(25)
                .then(|| robin_engine::replay::state_hash(engine));
            self.record_load_back(replay_ordinal, to_ordinal, is_continue);
            tracing::info!(
                replay_ordinal = replay_ordinal.number(),
                to_ordinal = to_ordinal.number(),
                "replay: load recorded as linear load-back"
            );
        } else {
            // TODO(replay): deduplicate repeated external-save payloads
            // within an attempt without losing their restore boundaries.
            self.record_load_snapshot(replay_ordinal, snapshot, target, is_continue);
            let recorder_state = frame.recorder_state;
            frame.reset_after_terminal_restore(robin_engine::replay::state_hash(engine));
            frame.recorder_state = recorder_state;
        }
    }

    /// Consume a replay record outside the normal outer-frame lifecycle.
    /// Debugger/manual stepping owns its own transaction, so it advances the
    /// dense replay ordinal immediately instead of deferring it to
    /// `TimelineRuntime::finish_recording`.
    pub(in crate::game_session) fn consume_frame_for_step(
        &mut self,
        timeline: TimelineFrame,
    ) -> Result<super::ReplayStepAdmission, MissionError> {
        let admission = self.consume_step(timeline)?;
        if matches!(admission, super::ReplayStepAdmission::Recorded(_)) {
            self.ordinal.advance();
        }
        Ok(admission)
    }

    pub(super) fn inject_replay_input(&mut self, frame: &mut MissionFrame) {
        if let Some(player) = &mut self.player {
            frame.inject_replay_input(player);
        }
    }

    pub(in crate::game_session) fn resolve_ordinal(
        &self,
        target: TimelineFrame,
    ) -> Result<Option<ReplayFrameOrdinal>, MissionError> {
        // TODO(10/F11): leaf returns String (engine replay player).
        self.player
            .as_ref()
            .map(|player| {
                player
                    .resolve_timeline_frame(target)
                    .map_err(MissionError::replay)
            })
            .transpose()
    }

    pub(super) fn seek_timeline(
        &mut self,
        target: TimelineFrame,
    ) -> Result<Option<ReplayFrameOrdinal>, MissionError> {
        self.player
            .as_mut()
            .map(|player| {
                player
                    .seek_timeline_frame(target)
                    .map_err(MissionError::replay)
            })
            .transpose()
    }

    fn consume_step(
        &mut self,
        timeline: TimelineFrame,
    ) -> Result<super::ReplayStepAdmission, MissionError> {
        let ordinal = self.ordinal;
        let Some(player) = &mut self.player else {
            return Ok(super::ReplayStepAdmission::NoActiveReplay);
        };
        if player.current_frame() != ordinal.number() {
            return Err(MissionError::replay(format!(
                "replay player ordinal {} diverged from timeline runtime ordinal {}",
                player.current_frame(),
                ordinal.number()
            )));
        }
        if player.is_finished() {
            return Ok(super::ReplayStepAdmission::Finished {
                ordinal: player.current_frame(),
                total_frames: player.total_frames(),
            });
        }
        let recorded = player.next_frame().clone();
        if recorded.timeline_before != timeline.number() {
            return Err(MissionError::replay(format!(
                "replay ordinal {} starts at timeline {}, current timeline is {}",
                ordinal.number(),
                recorded.timeline_before,
                timeline.number()
            )));
        }
        Ok(super::ReplayStepAdmission::Recorded(recorded))
    }

    pub(super) fn apply_playback_boundary(
        &mut self,
        timeline: TimelineFrame,
        rewind: &mut crate::rewind::RewindBuffer,
        host: &mut crate::host::Host,
        game: &mut crate::game::Game,
        manager: &mut robin_engine::engine_manager::EngineManager,
        assets: &robin_engine::engine::LevelAssets,
    ) -> Result<Option<TimelineFrame>, MissionError> {
        self.prepare_seek_cache(host, game, manager)?;
        let ordinal = self.ordinal;
        let Some(player) = self.player.as_ref().filter(|player| !player.is_finished()) else {
            return Ok(None);
        };
        if player.current_frame() != ordinal.number() {
            return Err(MissionError::replay(format!(
                "replay player ordinal {} diverged from timeline runtime ordinal {}",
                player.current_frame(),
                ordinal.number()
            )));
        }
        super::apply_replay_timeline_events_with_hash_policy(
            player,
            timeline,
            &mut self.pinned_saves,
            rewind,
            host,
            game,
            manager,
            assets,
            !self.recompute_marker_hashes,
        )
    }

    pub(super) fn prepare_seek_cache(
        &mut self,
        host: &crate::host::Host,
        game: &crate::game::Game,
        manager: &robin_engine::engine_manager::EngineManager,
    ) -> Result<(), MissionError> {
        let ordinal = self.ordinal;
        let Some(player) = self.player.as_ref() else {
            return Ok(());
        };
        if ordinal == ReplayFrameOrdinal::ZERO && self.initial_state.is_none() {
            let mut modals = super::super::session_policy::SessionModalScheduler::default();
            modals.checkpoint(0, &host.effects);
            self.initial_state = Some((
                robin_engine::engine::CompressedEngineSnapshot::capture(&manager.engine).map_err(
                    |error| MissionError::replay(format!("compress replay start: {error}")),
                )?,
                CompressedGameRuntimeSnapshot::capture(&manager.engine, host, game).map_err(
                    |error| MissionError::replay(format!("capture replay start: {error:#}")),
                )?,
                modals,
            ));
            if let Some(sidecar) = crate::replay_seek::take(player.data()) {
                match seek::ReplaySeekCache::import(sidecar, player.data(), host, game) {
                    Ok(cache) => self.seek_cache = cache,
                    Err(error) => {
                        tracing::warn!(%error, "replay seek sidecar rejected; using local checkpoints")
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) fn restore_initial(
        &mut self,
        manager: &mut robin_engine::engine_manager::EngineManager,
        host: &mut crate::host::Host,
        game: &mut crate::game::Game,
        assets: &robin_engine::engine::LevelAssets,
    ) -> Result<(), MissionError> {
        let (engine, snapshot, modals) = self
            .initial_state
            .as_mut()
            .ok_or_else(|| MissionError::replay("replay start has not been captured"))?;
        snapshot
            .clone()
            .apply_to_with_game(&mut manager.engine, host, game, assets)
            .map_err(|error| MissionError::replay(format!("restore replay start: {error}")))?;
        // Seeking is rollback, not a save load: retain the exact pre-frame-zero
        // engine, including runtime queues that persisted-load reconciliation changes.
        manager.engine = engine
            .restore(assets)
            .map_err(|error| MissionError::replay(format!("decode replay start: {error}")))?;
        game.apply_post_load_sync(false);
        game.post_load_resolution_resync();
        modals.restore(0, &mut host.effects);
        self.player
            .as_mut()
            .ok_or_else(|| MissionError::replay("no active replay"))?
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

    /// Attach source evidence observed outside the deterministic command
    /// value (notably HTTP player-command and stepping ingress) to the current
    /// streaming replay ordinal.
    pub(in crate::game_session) fn record_taints(
        &mut self,
        taints: impl IntoIterator<Item = robin_engine::replay_rankability::InputTaintKind>,
    ) {
        let ordinal = self.ordinal;
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

    pub(in crate::game_session) fn seal(&mut self) {
        self.recording = match std::mem::replace(&mut self.recording, RecordingState::Inactive) {
            RecordingState::Recording(recorder) => RecordingState::Sealed(recorder.seal()),
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

    /// Register a successfully completed bootstrap Restart save at frame zero.
    ///
    /// That save is captured during mission setup, immediately before
    /// runtime construction, so its payload is exactly the frame-0
    /// boundary state the replay header reconstructs.  Registering it here
    /// lets a later script-triggered restart record as a load-back to
    /// frame 0 instead of a timeline discontinuity.
    pub(in crate::game_session) fn register_bootstrap(
        &mut self,
        completed: Option<BootstrapSaveBoundary>,
    ) {
        let ordinal = self.ordinal;
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
    pub(in crate::game_session) fn install_test_recorder(&mut self, recorder: ReplayRecorder) {
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
        ReplayLifecycle::new(Some(recorder.into()), None, service.recording(), false)
    }

    #[test]
    fn sealing_and_invalidation_are_idempotent_without_creating_a_writer() {
        let service = Arc::new(crate::replay_service::ReplayService::default());
        let mut inactive = ReplayLifecycle::new(None, None, service.recording(), false);
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
        lifecycle.register_bootstrap(Some(BootstrapSaveBoundary {
            identity: bootstrap,
            marker: ReplaySaveMarker {
                state_hash: 17,
                timeline_frame: 0,
            },
        }));
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
        let playback = ReplayLifecycle::new(None, Some(player), service.recording(), false);
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
