use super::*;
use std::io::Write;

/// Records complete authoritative simulation inputs during live gameplay,
/// streaming each frame
/// to a JSONL file as it completes.
///
/// Line 1 (the header) is written on construction. Each subsequent
/// `write_frame` appends exactly one input line for every admitted frame.
/// Any I/O failure invalidates recording; `flush` retains that failure so
/// callers cannot publish a save link to an incomplete history.
pub struct ReplayRecorder {
    writer: std::io::BufWriter<Box<dyn std::io::Write + Send>>,
    initial_header: ReplayHeader,
    bootstrap_marker_written: bool,
    next_expected_ordinal: u32,
    boundary_metadata_pending: bool,
    observed_taints: BTreeSet<InputTaintKind>,
    // A later successful flush cannot repair a partially written JSONL record.
    failure: Option<std::io::Error>,
}

impl ReplayRecorder {
    /// Dense mission-wide cursor, including host-only save boundaries.
    pub fn next_ordinal(&self) -> u32 {
        self.next_expected_ordinal
    }

    pub fn recording_header(&self) -> &ReplayHeader {
        &self.initial_header
    }

    /// Continue a validated mission prefix in a new physical chunk. The writer
    /// receives a header for that chunk; archive assembly keeps only the root header.
    pub fn continue_recording(
        writer: Box<dyn std::io::Write + Send>,
        header: ReplayHeader,
        next_ordinal: u32,
    ) -> std::io::Result<Self> {
        let mut recorder = Self::from_recording_header(writer, header)?;
        recorder.next_expected_ordinal = next_ordinal;
        Ok(recorder)
    }

    /// Save publication must observe a failed recording write. Frame recording
    /// can log errors, but must never publish a save pointing at missing bytes.
    pub fn flush(&mut self) -> std::io::Result<()> {
        if self.failure.is_none() {
            self.failure = self.writer.flush().err();
        }
        match &self.failure {
            Some(error) => Err(std::io::Error::new(error.kind(), error.to_string())),
            None => Ok(()),
        }
    }

    /// Create a recorder that streams to `path`.  Writes the header
    /// immediately; returns `Err` if the file can't be created.
    pub fn new(
        path: &str,
        mission_id: String,
        mission_assets: crate::mission_assets::MissionAssetDescriptor,
        rng_seed: u64,
        sim_config: crate::engine::SimConfig,
        campaign: &crate::campaign::Campaign,
    ) -> std::io::Result<Self> {
        let file = std::fs::File::create(path)?;
        Self::with_writer(
            Box::new(file),
            mission_id,
            mission_assets,
            rng_seed,
            sim_config,
            campaign,
        )
    }

    /// Create a recorder that streams to an arbitrary `Write` sink.
    /// Lets the caller tee the recording through a shared in-memory
    /// buffer (so the script-RPC `get-replay` can serialize the bytes
    /// directly without going through the filesystem), or to pipe the
    /// recording over a network connection, etc.  Writes the header
    /// immediately.
    pub fn with_writer(
        writer: Box<dyn std::io::Write + Send>,
        mission_id: String,
        mission_assets: crate::mission_assets::MissionAssetDescriptor,
        rng_seed: u64,
        sim_config: crate::engine::SimConfig,
        campaign: &crate::campaign::Campaign,
    ) -> std::io::Result<Self> {
        Self::with_writer_and_spellforge_package(
            writer,
            mission_id,
            mission_assets,
            rng_seed,
            sim_config,
            campaign,
            None,
        )
    }

    pub fn with_writer_and_spellforge_package(
        writer: Box<dyn std::io::Write + Send>,
        mission_id: String,
        mission_assets: crate::mission_assets::MissionAssetDescriptor,
        rng_seed: u64,
        sim_config: crate::engine::SimConfig,
        campaign: &crate::campaign::Campaign,
        spellforge_package: Option<crate::spellforge::SpellforgePackage>,
    ) -> std::io::Result<Self> {
        let campaign = bitcode::encode(campaign);
        let header = ReplayHeader {
            mission_id,
            mission_assets,
            rng_seed,
            sim_config,
            spellforge_package,
            version: REPLAY_SCHEMA_VERSION,
            total_frames: 0, // unknown until mission ends
            rankability: ReplayRankability::rankable(),
            campaign,
        };
        Self::from_recording_header(writer, header)
    }

    /// Reopen the exact construction-time authority of a completed recording.
    /// The caller must also record any snapshot restoration at the new boundary.
    pub fn from_recording_header(
        writer: Box<dyn std::io::Write + Send>,
        header: ReplayHeader,
    ) -> std::io::Result<Self> {
        let invalid = |error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error);
        if header.version != REPLAY_SCHEMA_VERSION || header.total_frames != 0 {
            return Err(invalid(
                "recorder requires a current construction-time header".to_string(),
            ));
        }
        header
            .mission_assets
            .validate_spellforge_package(header.spellforge_package.as_ref())
            .map_err(|error| invalid(error.to_string()))?;
        if !header
            .mission_id
            .eq_ignore_ascii_case(&header.mission_assets.mission_basename)
        {
            return Err(invalid(
                "replay mission ID does not match asset descriptor".to_string(),
            ));
        }
        if let Some(package) = &header.spellforge_package {
            package
                .validate_wire()
                .map_err(|error| invalid(error.to_string()))?;
        }
        let mut writer = std::io::BufWriter::new(writer);
        serde_json::to_writer(&mut writer, &header).map_err(std::io::Error::other)?;
        writeln!(writer)?;
        writer.flush()?;
        Ok(Self {
            writer,
            initial_header: header,
            bootstrap_marker_written: false,
            next_expected_ordinal: 0,
            boundary_metadata_pending: false,
            observed_taints: BTreeSet::new(),
            failure: None,
        })
    }

    /// Close the old writer and retain its original pre-engine campaign, seed,
    /// configuration and mission authority for a pristine Restart recording.
    pub fn into_recording_header(self) -> ReplayHeader {
        self.initial_header
    }

    /// Finalize the current frame with its complete authoritative input and
    /// advance the recorder cursor.
    /// The return value reports timeline admission, not persistence. Call
    /// `flush` before publishing references to recorded frames.
    pub fn write_frame(
        &mut self,
        ordinal: u32,
        timeline_before: u32,
        timeline_after: u32,
        input: SimulationFrameInput,
        host_controls: Vec<ReplayHostControl>,
        hash: Option<u64>,
    ) -> bool {
        assert_eq!(
            ordinal, self.next_expected_ordinal,
            "replay frame ordinal must be dense"
        );
        let meaningful = input.run_hourglass
            || !input.external_facts.is_empty()
            || !input.external_actions.is_empty()
            || !input.commands.is_empty()
            || !input.post_external_actions.is_empty()
            || !input.post_commands.is_empty()
            || input.run_post_initialize
            || !host_controls.is_empty()
            || self.boundary_metadata_pending;
        if !meaningful {
            return false;
        }
        assert!(
            timeline_after == timeline_before || timeline_after == timeline_before + 1,
            "replay timeline transition must stay or advance exactly once"
        );
        let rec = FrameRecord {
            f: ordinal,
            i: Some(ReplayFrame {
                timeline_before,
                timeline_after,
                input,
                host_controls,
            }),
            h: hash,
            sv: None,
            lb: None,
            t: Vec::new(),
        };
        self.write_record(&rec);
        self.next_expected_ordinal += 1;
        self.boundary_metadata_pending = false;
        true
    }

    /// Write a standalone hash record for `frame` (no commands).
    /// Flushed immediately so partial replays remain crash-safe.
    pub fn write_hash(&mut self, frame: u32, hash: u64) {
        self.write_record(&FrameRecord {
            f: frame,
            i: None,
            h: Some(hash),
            sv: None,
            lb: None,
            t: Vec::new(),
        });
    }

    /// Write a save-marker record for the current frame: an in-mission save
    /// captured the engine state (with the given state hash) at this frame's
    /// pre-command boundary.  Flushed immediately.
    pub fn write_save_marker(&mut self, ordinal: u32, marker: ReplaySaveMarker) {
        assert_eq!(ordinal, self.next_expected_ordinal);
        if ordinal == 0 && marker.timeline_frame == 0 {
            self.bootstrap_marker_written = true;
        }
        self.boundary_metadata_pending = true;
        self.write_record(&FrameRecord {
            f: ordinal,
            i: None,
            h: None,
            sv: Some(marker),
            lb: None,
            t: Vec::new(),
        });
    }

    /// Write a load-back record for the current frame: the engine state was
    /// replaced with the state captured by the save marker at `to_frame`.
    /// The sole same-ordinal case is a pristine Restart: marker 0 is pinned
    /// before load-back 0, reproducing post-load fixups before the first input.
    /// All other targets must be strictly earlier. Flushed
    /// immediately.
    pub fn write_load_back(&mut self, ordinal: u32, to_frame: u32, is_continue: bool) {
        assert_eq!(ordinal, self.next_expected_ordinal);
        assert!(
            to_frame < ordinal || (ordinal == 0 && to_frame == 0 && self.bootstrap_marker_written),
            "load-back target {to_frame} must precede the current replay ordinal {ordinal}",
        );
        self.boundary_metadata_pending = true;
        self.write_record(&FrameRecord {
            f: ordinal,
            i: None,
            h: None,
            sv: None,
            lb: Some(ReplayLoadBack {
                snapshot: None,
                to_frame,
                is_continue,
            }),
            t: Vec::new(),
        });
    }

    /// Restore an external save at this boundary without a timeline marker.
    pub fn write_load_snapshot(
        &mut self,
        ordinal: u32,
        snapshot: Vec<u8>,
        timeline_frame: u32,
        is_continue: bool,
    ) {
        assert_eq!(ordinal, self.next_expected_ordinal);
        assert!(!snapshot.is_empty(), "embedded save must not be empty");
        self.boundary_metadata_pending = true;
        self.write_record(&FrameRecord {
            f: ordinal,
            i: None,
            h: None,
            sv: None,
            lb: Some(ReplayLoadBack {
                snapshot: Some(ReplaySaveSnapshot {
                    payload: snapshot,
                    timeline_frame,
                }),
                to_frame: ordinal,
                is_continue,
            }),
            t: Vec::new(),
        });
    }

    /// Append the first observation of one ranked-ineligible input path.
    /// Repeated observations are deliberately omitted to keep JSONL bounded.
    pub fn record_input_taint(&mut self, kind: InputTaintKind, first_frame: u32) {
        if !self.observed_taints.insert(kind) {
            return;
        }
        self.write_record(&FrameRecord {
            f: first_frame,
            i: None,
            h: None,
            sv: None,
            lb: None,
            t: vec![InputTaint { kind, first_frame }],
        });
    }

    fn write_record(&mut self, rec: &FrameRecord) {
        if self.failure.is_some() {
            return;
        }
        let result = (|| {
            serde_json::to_writer(&mut self.writer, rec).map_err(std::io::Error::other)?;
            writeln!(self.writer)?;
            self.writer.flush()
        })();
        if let Err(error) = result {
            tracing::error!("Replay recording invalidated: {error}");
            self.failure = Some(error);
        }
    }
}

#[cfg(test)]
mod failure_tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Default, serde::Serialize, serde::Deserialize)]
    struct FaultState {
        bytes: Vec<u8>,
        remaining: Option<usize>,
        fail_flush: bool,
    }

    #[derive(serde::Serialize, serde::Deserialize)]
    struct IntermittentWriter {
        #[serde(skip)]
        state: Arc<Mutex<FaultState>>,
    }
    impl Write for IntermittentWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let mut state = self.state.lock().unwrap();
            if state.remaining == Some(0) {
                state.remaining = None; // Only the first attempt fails.
                return Err(std::io::Error::other("injected partial write"));
            }
            let count = state.remaining.map_or(bytes.len(), |n| n.min(bytes.len()));
            state.bytes.extend_from_slice(&bytes[..count]);
            if let Some(remaining) = &mut state.remaining {
                *remaining -= count;
            }
            Ok(count)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            let mut state = self.state.lock().unwrap();
            if std::mem::take(&mut state.fail_flush) {
                return Err(std::io::Error::other("injected flush failure"));
            }
            Ok(())
        }
    }

    #[test]
    fn recording_failures_survive_recovery_and_stop_subsequent_records() {
        // Partial buffered writes, underlying flush failures, and a large
        // record failing during serialization must all poison publication.
        for (partial, large) in [(true, false), (false, false), (true, true)] {
            let state = Arc::new(Mutex::new(FaultState::default()));
            let mut recorder = ReplayRecorder::with_writer(
                Box::new(IntermittentWriter {
                    state: state.clone(),
                }),
                "fault".into(),
                crate::mission_assets::MissionAssetDescriptor::built_in("fault", "fault", "fault")
                    .unwrap(),
                0,
                Default::default(),
                &Default::default(),
            )
            .unwrap();
            {
                let mut fault = state.lock().unwrap();
                if partial {
                    fault.remaining = Some(5);
                } else {
                    fault.fail_flush = true;
                }
            }
            if large {
                recorder.write_load_snapshot(0, vec![42; 32 * 1024], 0, false);
            } else {
                recorder.write_save_marker(
                    0,
                    ReplaySaveMarker {
                        state_hash: 1,
                        timeline_frame: 0,
                    },
                );
            }
            let first = recorder.flush().unwrap_err().to_string();
            let written = state.lock().unwrap().bytes.clone();
            // The underlying writer has recovered, but missing bytes cannot
            // be reconstructed by a later successful flush or frame.
            assert_eq!(recorder.flush().unwrap_err().to_string(), first);
            recorder.write_frame(0, 0, 1, Default::default(), Vec::new(), None);
            assert_eq!(recorder.flush().unwrap_err().to_string(), first);
            assert_eq!(state.lock().unwrap().bytes, written);
        }
    }

    #[test]
    fn explicit_flush_failure_is_also_sticky() {
        let state = Arc::new(Mutex::new(FaultState::default()));
        let mut recorder = ReplayRecorder::with_writer(
            Box::new(IntermittentWriter {
                state: state.clone(),
            }),
            "fault".into(),
            crate::mission_assets::MissionAssetDescriptor::built_in("fault", "fault", "fault")
                .unwrap(),
            0,
            Default::default(),
            &Default::default(),
        )
        .unwrap();
        state.lock().unwrap().fail_flush = true;
        assert!(recorder.flush().is_err());
        assert!(!state.lock().unwrap().fail_flush);
        assert!(recorder.flush().is_err());
    }
}
