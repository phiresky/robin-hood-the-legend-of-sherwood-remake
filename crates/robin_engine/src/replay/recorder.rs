use super::*;
use std::io::Write;

/// Records complete authoritative simulation inputs during live gameplay,
/// streaming each frame
/// to a JSONL file as it completes.
///
/// Line 1 (the header) is written on construction.  Each subsequent
/// `end_frame` appends exactly one input line for every admitted frame.
/// No explicit close is needed — the file is always valid up to the
/// last completed frame.
pub struct ReplayRecorder {
    writer: std::io::BufWriter<Box<dyn std::io::Write + Send>>,
    initial_header: ReplayHeader,
    bootstrap_marker_written: bool,
    next_expected_ordinal: u32,
    boundary_metadata_pending: bool,
    observed_taints: BTreeSet<InputTaintKind>,
}

impl ReplayRecorder {
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

    /// Backward-compatible name for [`Self::new`]. Every recorder requires a
    /// campaign snapshot.
    pub fn new_with_campaign(
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
        })
    }

    /// Close the old writer and retain its original pre-engine campaign, seed,
    /// configuration and mission authority for a pristine Restart recording.
    pub fn into_recording_header(self) -> ReplayHeader {
        self.initial_header
    }

    /// Finalize the current frame with its complete authoritative input and
    /// advance the recorder cursor.
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
        if let Err(e) = serde_json::to_writer(&mut self.writer, rec) {
            tracing::error!("Replay write error: {e}");
        } else if let Err(e) = writeln!(self.writer) {
            tracing::error!("Replay write error: {e}");
        } else {
            let _ = self.writer.flush();
        }
    }
}
