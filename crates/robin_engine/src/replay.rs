//! Replay recording and playback.
//!
//! A replay is a sequence of player commands keyed by frame number,
//! plus the metadata needed to reconstruct the initial engine state
//! (mission ID, RNG seed, optional campaign snapshot). Recording
//! happens transparently during normal gameplay; playback feeds the
//! recorded commands back into the engine in place of live input.
//!
//! ## Storage formats
//!
//! Two formats exist side by side:
//!
//! - **JSONL** (`*.rhrec.jsonl`): the recording format. Line 1 is the
//!   [`ReplayHeader`]; subsequent lines are `FrameRecord` objects —
//!   `{"f":<n>,"c":[…]}` — written **only for frames that have at
//!   least one command**. Streamed to disk incrementally so a crash
//!   can't truncate the file to an invalid state.
//! - **Compact sharing format** (`rhrec-{versionhash}-{base64}`): a
//!   base64-encoded, zstd-compressed, bitcode-serialized snapshot of a
//!   completed replay. Produced on demand (e.g. when the user wants to
//!   paste a replay into a bug report) and accepted inline by
//!   `--replay` / the JSON API. The encode/decode logic lives in
//!   `robin_rs::replay_format`.

use crate::player_command::{FrameCommands, PlayerCommand, PlayerInput};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{BufRead, Write};

/// Header metadata for a replay file.
#[derive(Clone, Debug, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct ReplayHeader {
    /// Mission identifier: the mission's base `.rhm` filename without
    /// extension (e.g. `"Dem_Lei_MP"`, `"Sherwood"`). Used by the
    /// replay loader to find the matching mission in the campaign so a
    /// replay can select its own mission independent of UI flow.
    pub mission_id: String,
    /// RNG seed used at mission start.
    pub rng_seed: u64,
    /// Replay *schema* version, bumped when the on-disk layout changes
    /// in a breaking way. Distinct from the engine git hash, which
    /// lives outside the header (prefix of the compact format).
    pub version: u32,
    /// Total number of simulation frames in the recording.
    /// Set to 0 during recording (unknown until mission ends);
    /// filled in by the player on load from the max frame index.
    pub total_frames: u32,
    /// Optional campaign snapshot captured at mission start, stored as
    /// an opaque bitcode-serialized blob. Engine initialization depends
    /// on campaign progression (ARES, prior mission outcomes, relics,
    /// …) so replays started mid-campaign need this to reproduce
    /// bit-exactly. `None` means the caller didn't supply a snapshot.
    pub campaign: Option<Vec<u8>>,
}

/// On-disk replay schema version.
///
/// - `1`: `c` was `Vec<PlayerCommand>` (untagged single-player).  Still
///   accepted on read via the untagged-enum fallback in
///   [`deserialize_inputs`].
/// - `2`: `c` is `Vec<PlayerInput>` (tagged with `player_id`) — the
///   first multiplayer-aware format.
const REPLAY_SCHEMA_VERSION: u32 = 2;

/// One JSONL line.  Carries per-frame commands and/or a periodic
/// engine-state hash used for desync detection on replay.
#[derive(Clone, Debug, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
struct FrameRecord {
    /// Frame number (0-based).
    f: u32,
    /// Inputs issued this frame, tagged with the seat that produced
    /// them.  v1 recordings (untagged `Vec<PlayerCommand>`) are
    /// transparently re-tagged as `PlayerId::HOST` on read.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "deserialize_inputs"
    )]
    c: Vec<PlayerInput>,
    /// Hash of deterministic engine state, written once per second
    /// (every 25 frames).  Used by the player to detect desyncs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    h: Option<u64>,
}

/// Per-element accept of either v2 (`PlayerInput`) or v1 (bare
/// `PlayerCommand`) shapes.  v1 elements get tagged `PlayerId::HOST`
/// on the way in.
fn deserialize_inputs<'de, D>(d: D) -> Result<Vec<PlayerInput>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum InputOrCommand {
        Tagged(PlayerInput),
        Bare(PlayerCommand),
    }
    let raw: Vec<InputOrCommand> = Vec::deserialize(d)?;
    Ok(raw
        .into_iter()
        .map(|item| match item {
            InputOrCommand::Tagged(p) => p,
            InputOrCommand::Bare(c) => PlayerInput::host(c),
        })
        .collect())
}

/// A complete recorded replay loaded into memory.
#[derive(Clone, Debug)]
pub struct ReplayData {
    pub header: ReplayHeader,
    /// Sparse map: only frames with commands are present.  v1 (untagged
    /// `Vec<PlayerCommand>`) recordings have every command tagged
    /// [`crate::player_command::PlayerId::HOST`] on read — there was
    /// only one seat by definition.  v2+ recordings carry the real
    /// per-input seat tag.
    frames: BTreeMap<u32, Vec<PlayerInput>>,
    /// Sparse map: expected engine state hash at the start of frame N.
    hashes: BTreeMap<u32, u64>,
}

/// Flat serde-compatible snapshot of a [`ReplayData`], used as the
/// payload for the compact `rhrec-{hash}-{base64}` sharing format.
/// Kept separate from `ReplayData` so the in-memory representation
/// can evolve without breaking binary compatibility.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplayFile {
    pub header: ReplayHeader,
    pub frames: BTreeMap<u32, Vec<PlayerInput>>,
    pub hashes: BTreeMap<u32, u64>,
}

impl From<ReplayFile> for ReplayData {
    fn from(f: ReplayFile) -> Self {
        Self {
            header: f.header,
            frames: f.frames,
            hashes: f.hashes,
        }
    }
}

impl From<&ReplayData> for ReplayFile {
    fn from(d: &ReplayData) -> Self {
        Self {
            header: d.header.clone(),
            frames: d.frames.clone(),
            hashes: d.hashes.clone(),
        }
    }
}

impl ReplayData {
    /// Total number of simulation frames in the replay.
    pub fn frame_count(&self) -> u32 {
        self.header.total_frames
    }

    /// Commands for a given frame, or empty if none recorded.
    pub fn commands_for_frame(&self, frame: u32) -> &[PlayerInput] {
        self.frames.get(&frame).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Expected engine-state hash at the start of the given frame,
    /// or `None` if no hash was recorded for that frame.
    pub fn hash_for_frame(&self, frame: u32) -> Option<u64> {
        self.hashes.get(&frame).copied()
    }

    /// Load a replay from a JSONL reader.
    pub fn from_reader(reader: impl BufRead) -> Result<Self, String> {
        let mut lines = reader.lines();
        let header_line = lines
            .next()
            .ok_or("empty replay file")?
            .map_err(|e| format!("read error: {e}"))?;
        let mut header: ReplayHeader =
            serde_json::from_str(&header_line).map_err(|e| format!("bad header: {e}"))?;

        let mut frames = BTreeMap::new();
        let mut hashes = BTreeMap::new();
        let mut max_frame: u32 = 0;
        for (i, line) in lines.enumerate() {
            let line = line.map_err(|e| format!("read error at line {}: {e}", i + 2))?;
            if line.is_empty() {
                continue;
            }
            let rec: FrameRecord =
                serde_json::from_str(&line).map_err(|e| format!("bad line {}: {e}", i + 2))?;
            max_frame = max_frame.max(rec.f + 1);
            if let Some(h) = rec.h {
                hashes.insert(rec.f, h);
            }
            if !rec.c.is_empty() {
                frames.insert(rec.f, rec.c);
            }
        }
        // If header didn't have total_frames, infer from max frame index.
        if header.total_frames == 0 && (!frames.is_empty() || !hashes.is_empty()) {
            header.total_frames = max_frame;
        }
        Ok(Self {
            header,
            frames,
            hashes,
        })
    }

    /// Load a replay from a JSONL file on disk.
    pub fn from_file(path: &str) -> Result<Self, String> {
        let file = std::fs::File::open(path).map_err(|e| format!("open {path}: {e}"))?;
        Self::from_reader(std::io::BufReader::new(file))
    }
}

/// Records player commands during live gameplay, streaming each frame
/// to a JSONL file as it completes.
///
/// Line 1 (the header) is written on construction.  Each subsequent
/// `end_frame` appends a line **only if the frame has commands**.
/// No explicit close is needed — the file is always valid up to the
/// last completed frame.
pub struct ReplayRecorder {
    writer: std::io::BufWriter<Box<dyn std::io::Write + Send>>,
    current_frame: FrameCommands,
    frame_number: u32,
}

impl ReplayRecorder {
    /// Create a recorder that streams to `path`.  Writes the header
    /// immediately; returns `Err` if the file can't be created.
    pub fn new(path: &str, mission_id: String, rng_seed: u64) -> std::io::Result<Self> {
        let file = std::fs::File::create(path)?;
        Self::with_writer(Box::new(file), mission_id, rng_seed)
    }

    /// Create a recorder and include a bitcode campaign snapshot in
    /// the replay header.
    pub fn new_with_campaign(
        path: &str,
        mission_id: String,
        rng_seed: u64,
        campaign: &crate::campaign::Campaign,
    ) -> std::io::Result<Self> {
        let file = std::fs::File::create(path)?;
        Self::with_writer_and_campaign(Box::new(file), mission_id, rng_seed, Some(campaign))
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
        rng_seed: u64,
    ) -> std::io::Result<Self> {
        Self::with_writer_and_campaign(writer, mission_id, rng_seed, None)
    }

    /// Create a recorder to an arbitrary sink, optionally stamping a
    /// campaign snapshot into the replay header.  The snapshot is
    /// captured before frame 0 so replay launch can restore campaign
    /// progression before loading the mission.
    pub fn with_writer_and_campaign(
        writer: Box<dyn std::io::Write + Send>,
        mission_id: String,
        rng_seed: u64,
        campaign: Option<&crate::campaign::Campaign>,
    ) -> std::io::Result<Self> {
        let mut writer = std::io::BufWriter::new(writer);
        let campaign = campaign
            .map(bitcode::serialize)
            .transpose()
            .map_err(std::io::Error::other)?;
        let header = ReplayHeader {
            mission_id,
            rng_seed,
            version: REPLAY_SCHEMA_VERSION,
            total_frames: 0, // unknown until mission ends
            campaign,
        };
        serde_json::to_writer(&mut writer, &header).map_err(std::io::Error::other)?;
        writeln!(writer)?;
        writer.flush()?;
        Ok(Self {
            writer,
            current_frame: FrameCommands::new(),
            frame_number: 0,
        })
    }

    /// Record a command for the current frame. Accepts a bare
    /// [`PlayerCommand`] (tagged `PlayerId::HOST`) or a pre-tagged
    /// [`PlayerInput`].
    pub fn push(&mut self, cmd: impl Into<PlayerInput>) {
        self.current_frame.push(cmd);
    }

    /// Finalize the current frame.  Writes a JSONL line only if the
    /// frame contains at least one command.  Advances the frame counter.
    pub fn end_frame(&mut self) {
        let inputs = std::mem::take(&mut self.current_frame.commands);
        if !inputs.is_empty() {
            let rec = FrameRecord {
                f: self.frame_number,
                c: inputs,
                h: None,
            };
            if let Err(e) = serde_json::to_writer(&mut self.writer, &rec) {
                tracing::error!("Replay write error: {e}");
            } else if let Err(e) = writeln!(self.writer) {
                tracing::error!("Replay write error: {e}");
            } else {
                let _ = self.writer.flush();
            }
        }
        self.frame_number += 1;
    }

    /// Number of frames elapsed (including empty ones).
    pub fn frame_number(&self) -> u32 {
        self.frame_number
    }

    /// Write a standalone hash record for `frame` (no commands).
    /// Flushed immediately so partial replays remain crash-safe.
    pub fn write_hash(&mut self, frame: u32, hash: u64) {
        let rec = FrameRecord {
            f: frame,
            c: Vec::new(),
            h: Some(hash),
        };
        if let Err(e) = serde_json::to_writer(&mut self.writer, &rec) {
            tracing::error!("Replay write error: {e}");
        } else if let Err(e) = writeln!(self.writer) {
            tracing::error!("Replay write error: {e}");
        } else {
            let _ = self.writer.flush();
        }
    }
}

/// Compute a hash of the deterministic engine state. Host/render/input
/// state stays outside the engine snapshot, while explicit snapshot schemas
/// hash only their deterministic fields.
///
/// Walks the `EngineInner` field-by-field via `StateHash` (defined in
/// `robin_util`), feeding bytes into `FxHasher`. Floats hash via
/// `to_bits()`, `BTreeMap`/`HashMap` hash in deterministic order, so the
/// hash stays stable across rollback replays without going through JSON
/// serialization.
pub fn state_hash(engine: &crate::engine::EngineInner) -> u64 {
    let start = web_time::Instant::now();
    let out = robin_util::state_hash::compute(engine);
    let elapsed_us = start.elapsed().as_micros();
    STATE_HASH_STATS.with(|s| {
        let mut s = s.borrow_mut();
        s.count += 1;
        s.hash_us += elapsed_us as u64;
        if s.count == 1 || s.count % 50 == 0 {
            tracing::debug!(
                target: "robin_engine::replay::perf",
                "state_hash count={} avg_us={}",
                s.count,
                s.hash_us / s.count,
            );
        }
    });
    out
}

#[derive(Default)]
struct StateHashStats {
    count: u64,
    hash_us: u64,
}

thread_local! {
    static STATE_HASH_STATS: std::cell::RefCell<StateHashStats> =
        std::cell::RefCell::new(StateHashStats::default());
}

/// Plays back a recorded replay, yielding commands frame by frame.
pub struct ReplayPlayer {
    data: ReplayData,
    current_frame: u32,
}

impl ReplayPlayer {
    pub fn new(data: ReplayData) -> Self {
        Self {
            data,
            current_frame: 0,
        }
    }

    /// Header metadata (mission ID, seed, version).
    pub fn header(&self) -> &ReplayHeader {
        &self.data.header
    }

    /// Whether playback has reached the end.
    pub fn is_finished(&self) -> bool {
        self.current_frame >= self.data.frame_count()
    }

    /// Get commands for the current frame and advance.
    pub fn next_frame(&mut self) -> &[PlayerInput] {
        let cmds = self.data.commands_for_frame(self.current_frame);
        self.current_frame += 1;
        cmds
    }

    /// Current frame index (before next_frame advances it).
    pub fn current_frame(&self) -> u32 {
        self.current_frame
    }

    /// Move the playback cursor to `frame`.  Used by the host-side
    /// step-back / rewind paths so replay playback can resume from the
    /// reconstructed past frame instead of racing forward from where
    /// the cursor happened to be.  Clamped to `[0, total_frames]`.
    pub fn seek(&mut self, frame: u32) {
        self.current_frame = frame.min(self.data.frame_count());
    }

    /// Expected engine-state hash at the start of `frame`, if the
    /// recording carries one for that frame.
    pub fn hash_for_frame(&self, frame: u32) -> Option<u64> {
        self.data.hash_for_frame(frame)
    }

    /// Total frames in the replay.
    pub fn total_frames(&self) -> u32 {
        self.data.frame_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_and_playback_roundtrip() {
        let dir = std::env::temp_dir().join("replay_test_sparse.jsonl");
        let path = dir.to_str().unwrap();

        {
            let mut rec = ReplayRecorder::new(path, "test_mission".into(), 42).unwrap();

            // Frame 0: one command
            rec.push(PlayerCommand::SelectAllPcs);
            rec.end_frame();

            // Frames 1-49: empty (no commands → no lines written)
            for _ in 1..50 {
                rec.end_frame();
            }

            // Frame 50: two commands
            rec.push(PlayerCommand::GroupMove {
                actors: vec![crate::element::EntityId::Pc(crate::entity_id::PcId(1))],
                destination: crate::coordinates::MapPoint::new(100.0, 200.0),
                running: false,
                show_marker: true,
                goal_override: None,
            });
            rec.push(PlayerCommand::CrouchDown);
            rec.end_frame();

            assert_eq!(rec.frame_number(), 51);
        }

        // Check file is compact: header + 2 data lines
        let contents = std::fs::read_to_string(path).unwrap();
        assert_eq!(contents.lines().count(), 3); // header + frame 0 + frame 50

        let data = ReplayData::from_file(path).unwrap();
        assert_eq!(data.frame_count(), 51);
        assert_eq!(data.header.rng_seed, 42);

        let mut player = ReplayPlayer::new(data);
        assert!(!player.is_finished());

        // Frame 0: one command
        let f0 = player.next_frame();
        assert_eq!(f0.len(), 1);

        // Frames 1-49: empty
        for _ in 1..50 {
            let empty = player.next_frame();
            assert!(empty.is_empty());
        }

        // Frame 50: two commands
        let f50 = player.next_frame();
        assert_eq!(f50.len(), 2);

        assert!(player.is_finished());

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn legacy_v1_jsonl_loads_as_host_seat() {
        // v1 recordings stored bare `PlayerCommand` values in `c`.
        // Make sure the v2 reader still accepts them, tagging each
        // command as `PlayerId::HOST`.
        let path = std::env::temp_dir()
            .join("replay_test_legacy_v1.jsonl")
            .to_str()
            .unwrap()
            .to_string();
        let mut buf = String::new();
        buf.push_str(
            r#"{"mission_id":"legacy","rng_seed":7,"version":1,"total_frames":0,"campaign":null}
"#,
        );
        // CrouchDown is a unit variant — serializes as a bare string
        // under serde's default external tagging; round-trips through
        // the InputOrCommand::Bare arm.
        buf.push_str(
            r#"{"f":0,"c":["CrouchDown"]}
"#,
        );
        buf.push_str(
            r#"{"f":2,"c":["StandUp"],"h":12345}
"#,
        );
        std::fs::write(&path, &buf).unwrap();

        let data = ReplayData::from_file(&path).unwrap();
        assert_eq!(data.frame_count(), 3);
        assert_eq!(data.header.version, 1);

        let f0 = data.commands_for_frame(0);
        assert_eq!(f0.len(), 1);
        assert_eq!(f0[0].player_id, crate::player_command::PlayerId::HOST);
        assert!(matches!(f0[0].command, PlayerCommand::CrouchDown));

        assert_eq!(data.hash_for_frame(2), Some(12345));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn v2_recording_round_trips_player_id() {
        use crate::player_command::PlayerId;
        let path = std::env::temp_dir()
            .join("replay_test_v2_seats.jsonl")
            .to_str()
            .unwrap()
            .to_string();
        {
            let mut rec = ReplayRecorder::new(&path, "mp_seats".into(), 99).unwrap();
            rec.push(PlayerInput::new(PlayerId(0), PlayerCommand::CrouchDown));
            rec.push(PlayerInput::new(PlayerId(2), PlayerCommand::StandUp));
            rec.end_frame();
        }
        let data = ReplayData::from_file(&path).unwrap();
        assert_eq!(data.header.version, REPLAY_SCHEMA_VERSION);
        let f0 = data.commands_for_frame(0);
        assert_eq!(f0.len(), 2);
        assert_eq!(f0[0].player_id, PlayerId(0));
        assert_eq!(f0[1].player_id, PlayerId(2));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn recorder_header_round_trips_campaign_snapshot() {
        use crate::campaign::{Campaign, CampaignValue};

        let path = std::env::temp_dir()
            .join("replay_test_campaign_header.jsonl")
            .to_str()
            .unwrap()
            .to_string();
        let mut campaign = Campaign::default();
        campaign.values[CampaignValue::Score] = 12_345;
        campaign.ares = 4;
        {
            let mut rec =
                ReplayRecorder::new_with_campaign(&path, "campaign".into(), 11, &campaign).unwrap();
            rec.push(PlayerCommand::CrouchDown);
            rec.end_frame();
        }

        let data = ReplayData::from_file(&path).unwrap();
        let bytes = data.header.campaign.as_ref().expect("campaign snapshot");
        let restored: Campaign = bitcode::deserialize(bytes).unwrap();
        assert_eq!(restored.values[CampaignValue::Score], 12_345);
        assert_eq!(restored.ares, 4);
        assert_eq!(data.commands_for_frame(0).len(), 1);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn group_move_goal_override_serde_roundtrip() {
        let with_override = PlayerCommand::GroupMove {
            actors: vec![crate::element::EntityId::Pc(crate::entity_id::PcId(1))],
            destination: crate::coordinates::MapPoint::new(50.0, 75.0),
            running: true,
            show_marker: false,
            goal_override: Some((crate::sector::SectorNumber::new(42), 3)),
        };
        let json = serde_json::to_string(&with_override).unwrap();
        let round: PlayerCommand = serde_json::from_str(&json).unwrap();
        match round {
            PlayerCommand::GroupMove {
                goal_override: Some((sector, layer)),
                ..
            } => {
                assert_eq!(sector, crate::sector::SectorNumber::new(42));
                assert_eq!(layer, 3);
            }
            _ => panic!("round-tripped GroupMove lost its goal_override"),
        }
    }

    #[test]
    fn player_past_end_returns_empty() {
        let mut frames = BTreeMap::new();
        frames.insert(0, vec![PlayerInput::host(PlayerCommand::CrouchDown)]);
        let data = ReplayData {
            header: ReplayHeader {
                mission_id: "x".into(),
                rng_seed: 0,
                version: 1,
                total_frames: 1,
                campaign: None,
            },
            frames,
            hashes: BTreeMap::new(),
        };
        let mut player = ReplayPlayer::new(data);
        let _ = player.next_frame();
        assert!(player.is_finished());
        let past = player.next_frame();
        assert!(past.is_empty());
    }
}
