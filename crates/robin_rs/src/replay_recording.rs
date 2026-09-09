//! Shared recording boundary used by simulation and synchronous save capture.
//! Save workers receive already linked payloads and never mutate this owner.

use crate::replay_archive::{MissionArchive, SaveReplayLink};
use crate::save_file::{GameSaveFile, ReplaySaveIdentity};
use anyhow::{Context, Result, ensure};
use robin_engine::replay::{ReplayHeader, ReplayRecorder, ReplaySaveMarker};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::sync::{Arc, Mutex};

pub(crate) struct Recording {
    recorder: ReplayRecorder,
    archive: Option<MissionArchive>,
    timeline: u32,
    captured: std::collections::BTreeMap<ReplaySaveIdentity, (u32, u32)>,
}

#[derive(Clone)]
pub(crate) struct SharedReplayRecorder(Arc<Mutex<Recording>>);

impl Serialize for SharedReplayRecorder {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str("live mission recording")
    }
}
impl<'de> Deserialize<'de> for SharedReplayRecorder {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "recording requires live process ownership",
        ))
    }
}

impl From<ReplayRecorder> for SharedReplayRecorder {
    fn from(recorder: ReplayRecorder) -> Self {
        Self(Arc::new(Mutex::new(Recording {
            recorder,
            archive: None,
            timeline: 0,
            captured: Default::default(),
        })))
    }
}

impl SharedReplayRecorder {
    pub(crate) fn persist_ranked_input(
        &self,
        input: &crate::leaderboard_mission_end::MissionEndSubmissionInput,
    ) -> Result<()> {
        self.0
            .lock()
            .expect("recording poisoned")
            .archive
            .as_ref()
            .context("ranked archive requires mission storage")?
            .write_ranked_input(input)
    }
    pub(crate) fn archived(recorder: ReplayRecorder, archive: MissionArchive) -> Self {
        Self(Arc::new(Mutex::new(Recording {
            recorder,
            archive: Some(archive),
            timeline: 0,
            captured: Default::default(),
        })))
    }

    pub(crate) fn next_ordinal(&self) -> u32 {
        self.0
            .lock()
            .expect("recording poisoned")
            .recorder
            .next_ordinal()
    }
    pub(crate) fn has_archive(&self) -> bool {
        self.0.lock().expect("recording poisoned").archive.is_some()
    }
    pub(crate) fn captured_frame(&self, identity: ReplaySaveIdentity) -> Option<(u32, u32)> {
        self.0
            .lock()
            .expect("recording poisoned")
            .captured
            .get(&identity)
            .copied()
    }
    pub(crate) fn into_recording_header(self) -> ReplayHeader {
        self.0
            .lock()
            .expect("recording poisoned")
            .recorder
            .recording_header()
            .clone()
    }

    /// A save is an actual host event: record its boundary without advancing
    /// the engine or consuming commands still queued for the next transaction.
    pub(crate) fn capture_save(&self, save: &GameSaveFile) -> Result<Option<SaveReplayLink>> {
        let mut recording = self.0.lock().expect("recording poisoned");
        if recording.archive.is_none() {
            return Ok(None);
        }
        ensure!(
            save.header.mission_assets == recording.recorder.recording_header().mission_assets,
            "save assets differ from active recording"
        );
        let identity = save.replay_identity()?;
        let ReplaySaveIdentity::Payload(digest) = identity else {
            unreachable!("serialized save has a payload identity")
        };
        let ordinal = recording.recorder.next_ordinal();
        let timeline = recording.timeline;
        let hash = robin_engine::replay::state_hash(&save.engine);
        let marker = ReplaySaveMarker {
            state_hash: hash,
            timeline_frame: timeline,
        };
        let link = recording
            .archive
            .as_ref()
            .expect("checked archive")
            .marker_link(ordinal, marker, digest);
        recording.recorder.write_save_marker(ordinal, marker);
        assert!(
            recording.recorder.write_frame(
                ordinal,
                timeline,
                timeline,
                robin_engine::engine::SimulationFrameInput::default()
                    .with_hourglass(false)
                    .with_simulation_body_allowed(false),
                Vec::new(),
                ordinal.is_multiple_of(25).then_some(hash)
            )
        );
        recording
            .recorder
            .flush()
            .context("persist replay marker before saving")?;
        recording
            .archive
            .as_ref()
            .expect("checked archive")
            .sync_current()?;
        recording.captured.insert(identity, (ordinal, timeline));
        Ok(Some(link))
    }

    /// Preserve the entire prior history, then start a new physical file at
    /// this restore boundary. Returns the adopted mission ordinal and timeline.
    pub(crate) fn restore(
        &self,
        save: &GameSaveFile,
        control: &crate::replay_service::ReplayRecordingControl,
    ) -> Result<(u32, u32, Option<u32>)> {
        // Include signed participant events from abandoned gameplay before
        // adopting the original archive's authority.
        control.checkpoint_ranked_input();
        let mut recording = self.0.lock().expect("recording poisoned");
        recording.recorder.flush()?;
        let link = save.header.replay.as_ref();
        let current = recording
            .archive
            .as_ref()
            .context("recording has no mission archive")?;
        current.sync_current()?;
        let other = link
            .filter(|link| std::path::Path::new(&link.mission_directory) != current.directory());
        let opened = other
            .map(|link| MissionArchive::open(std::path::Path::new(&link.mission_directory)))
            .transpose()?;
        let archive = opened.as_ref().unwrap_or(current);
        let prefix = archive.assembled_bytes()?;
        let data = robin_engine::replay::ReplayData::from_reader(std::io::Cursor::new(&prefix))
            .map_err(|error| anyhow::anyhow!("parse mission history: {error}"))?;
        ensure!(
            data.header().mission_assets == save.header.mission_assets,
            "loaded replay requires different mission assets"
        );
        let (timeline, target) = if let Some(link) = link {
            let ReplaySaveIdentity::Payload(digest) = save.replay_identity()? else {
                unreachable!()
            };
            archive.validate_link(link, &data, digest)?;
            (link.timeline_frame, Some(link.marker))
        } else {
            // A foreign payload is replayable, but cannot supply missing input
            // history. The ordinary StateLoad evidence keeps it unranked.
            tracing::warn!(
                "loaded save has no replay history reference; recording an embedded restore"
            );
            (recording.timeline, None)
        };
        let ordinal = data.frame_count();
        let mut header = data.header().clone();
        header.total_frames = 0;
        // Keep the exact root header. Derived taints are already in the prefix.
        let newline = prefix
            .iter()
            .position(|byte| *byte == b'\n')
            .context("replay root has no header")?;
        let root: ReplayHeader = serde_json::from_slice(&prefix[..newline])?;
        ensure!(
            root.mission_assets == header.mission_assets,
            "replay root differs from parsed mission"
        );
        if let Some(opened) = opened {
            recording.archive = Some(opened);
        }
        let archive = recording.archive.as_mut().expect("checked archive");
        archive.append_chunk(ordinal, link.cloned())?;
        let primary = archive.writer()?;
        let ranked_input = archive
            .read_ranked_input()
            .map_err(|error| format!("original ranked admission unavailable: {error:#}"));
        let mut mirror = control.begin_recording();
        mirror.write_all(&prefix)?;
        mirror.flush()?;
        let writer = crate::game_session::replay_init::continuation_writer(primary, mirror);
        recording.recorder = ReplayRecorder::continue_recording(writer, root, ordinal)?;
        recording.timeline = timeline;
        recording.captured.clear();
        control.restore_ranked_input(ranked_input);
        #[cfg(not(target_arch = "wasm32"))]
        crate::mission_replays::recording_started(
            &recording
                .archive
                .as_ref()
                .expect("archive")
                .directory()
                .join(recording.archive.as_ref().expect("archive").current_chunk()),
        );
        Ok((ordinal, timeline, target))
    }

    pub(crate) fn write_save_marker(&self, ordinal: u32, marker: ReplaySaveMarker) {
        self.0
            .lock()
            .expect("recording poisoned")
            .recorder
            .write_save_marker(ordinal, marker);
    }

    /// Persist the restore even when the caller exits or remains paused before
    /// admitting another gameplay frame. No chunk can be left with an orphan lb.
    pub(crate) fn commit_restore_boundary(&self, timeline: u32, hash: u64) -> Result<u32> {
        let mut recording = self.0.lock().expect("recording poisoned");
        let ordinal = recording.recorder.next_ordinal();
        ensure!(
            recording.recorder.write_frame(
                ordinal,
                timeline,
                timeline,
                robin_engine::engine::SimulationFrameInput::default()
                    .with_hourglass(false)
                    .with_simulation_body_allowed(false),
                Vec::new(),
                ordinal.is_multiple_of(25).then_some(hash),
            ),
            "restore boundary is missing its load event"
        );
        recording.recorder.flush()?;
        recording
            .archive
            .as_ref()
            .context("restore boundary requires a mission archive")?
            .sync_current()?;
        Ok(recording.recorder.next_ordinal())
    }
    pub(crate) fn write_load_back(&self, ordinal: u32, target: u32, is_continue: bool) {
        self.0
            .lock()
            .expect("recording poisoned")
            .recorder
            .write_load_back(ordinal, target, is_continue);
    }
    pub(crate) fn write_load_snapshot(
        &self,
        ordinal: u32,
        bytes: Vec<u8>,
        timeline: u32,
        is_continue: bool,
    ) {
        self.0
            .lock()
            .expect("recording poisoned")
            .recorder
            .write_load_snapshot(ordinal, bytes, timeline, is_continue);
    }
    pub(crate) fn record_input_taint(
        &self,
        kind: robin_engine::replay_rankability::InputTaintKind,
        ordinal: u32,
    ) {
        self.0
            .lock()
            .expect("recording poisoned")
            .recorder
            .record_input_taint(kind, ordinal);
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn write_frame(
        &self,
        ordinal: u32,
        before: u32,
        after: u32,
        input: robin_engine::engine::SimulationFrameInput,
        controls: Vec<robin_engine::replay::ReplayHostControl>,
        hash: Option<u64>,
    ) -> bool {
        let mut recording = self.0.lock().expect("recording poisoned");
        let written = recording
            .recorder
            .write_frame(ordinal, before, after, input, controls, hash);
        if written {
            recording.timeline = after;
        }
        written
    }
}
