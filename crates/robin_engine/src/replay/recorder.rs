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
        mission_assets
            .validate_spellforge_package(spellforge_package.as_ref())
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
        if !mission_id.eq_ignore_ascii_case(&mission_assets.mission_basename) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "replay mission ID `{mission_id}` does not match asset descriptor mission `{}`",
                    mission_assets.mission_basename
                ),
            ));
        }
        if let Some(package) = &spellforge_package {
            package
                .validate_wire()
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
        }
        let mut writer = std::io::BufWriter::new(writer);
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
        serde_json::to_writer(&mut writer, &header).map_err(std::io::Error::other)?;
        writeln!(writer)?;
        writer.flush()?;
        Ok(Self {
            writer,
            next_expected_ordinal: 0,
            boundary_metadata_pending: false,
            observed_taints: BTreeSet::new(),
        })
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
    /// `to_frame` must be strictly earlier than the current frame.  Flushed
    /// immediately.
    pub fn write_load_back(&mut self, ordinal: u32, to_frame: u32, is_continue: bool) {
        assert_eq!(ordinal, self.next_expected_ordinal);
        assert!(
            to_frame < ordinal,
            "load-back target {to_frame} must precede the current replay ordinal {ordinal}",
        );
        self.boundary_metadata_pending = true;
        self.write_record(&FrameRecord {
            f: ordinal,
            i: None,
            h: None,
            sv: None,
            lb: Some(ReplayLoadBack {
                to_frame,
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
