//! Replay recording and playback.
//!
//! A replay is a sequence of complete authoritative simulation-frame inputs,
//! keyed by frame number,
//! plus the metadata needed to reconstruct the initial engine state
//! (mission ID, RNG seed, simulation config, and campaign snapshot). Recording
//! happens transparently during normal gameplay; playback feeds the
//! recorded commands back into the engine in place of live input.
//!
//! ## Storage formats
//!
//! Two formats exist side by side:
//!
//! - **JSONL** (`*.rhrec.jsonl`): the recording format. Line 1 is the
//!   [`ReplayHeader`]; subsequent lines are `FrameRecord` objects —
//!   `{"f":<n>,"i":{…}}` — written for every admitted simulation frame.
//!   Streamed to disk incrementally so a crash
//!   can't truncate the file to an invalid state.
//! - **Compact sharing format** (`rhrec-{versionhash}-{base64}`): a
//!   base64-encoded, zstd-compressed, bitcode-serialized snapshot of a
//!   completed replay. Produced on demand (e.g. when the user wants to
//!   paste a replay into a bug report) and accepted inline by
//!   `--replay` / the JSON API. The encode/decode logic lives in
//!   `robin_rs::replay_format`.

use crate::engine::SimulationFrameInput;
use crate::player_command::{DialogResult, ModalKind};
use crate::replay_rankability::{InputTaint, InputTaintKind, ReplayRankability};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, Read};

mod player;
mod ranked_admission;
mod recorder;
pub use player::ReplayPlayer;
pub use recorder::ReplayRecorder;

/// JSON expands byte arrays substantially. This limit accommodates the full
/// bounded 16-MiB Spellforge package plus ordinary campaign metadata while
/// rejecting an unterminated or hostile header before unbounded allocation.
pub const REPLAY_HEADER_JSON_BYTE_LIMIT: u64 = 96 * 1024 * 1024;

/// Header metadata for a replay file.
#[derive(
    Clone,
    Debug,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(deny_unknown_fields)]
pub struct ReplayHeader {
    /// Mission identifier: the mission's base `.rhm` filename without
    /// extension (e.g. `"Dem_Lei_MP"`, `"Sherwood"`). Used by the
    /// replay loader to find the matching mission in the campaign so a
    /// replay can select its own mission independent of UI flow.
    pub mission_id: String,
    /// Exact immutable mission assets required before engine construction.
    /// This is mandatory in the current schema; old Rust recordings are not
    /// guessed or upgraded.
    pub mission_assets: crate::mission_assets::MissionAssetDescriptor,
    /// RNG seed used at mission start.
    pub rng_seed: u64,
    /// Complete deterministic configuration used at mission construction.
    pub sim_config: crate::engine::SimConfig,
    /// Exact canonical Spellforge package required by this recording. Keeping
    /// the validated bytes in the replay makes Lua reconstruction independent
    /// of a mutable or missing local mod installation. `None` identifies an
    /// ordinary SCB mission.
    pub spellforge_package: Option<crate::spellforge::SpellforgePackage>,
    /// Replay *schema* version, bumped when the on-disk layout or the
    /// deterministic state-hash contract changes. Distinct from the engine git
    /// hash, which lives outside the header (prefix of the compact format).
    pub version: u32,
    /// Total number of simulation frames in the recording.
    /// Set to 0 during recording (unknown until mission ends);
    /// filled in by the player on load from the max frame index.
    pub total_frames: u32,
    /// Cumulative input-entry evidence. Current recorders start rankable and
    /// append each first-observed taint as an independent crash-safe record.
    pub rankability: ReplayRankability,
    /// Required campaign snapshot captured at mission start, stored as
    /// an opaque bitcode-serialized blob. Engine initialization depends
    /// on campaign progression (ARES, prior mission outcomes, relics,
    /// …) so every replay needs this to reproduce bit-exactly.
    pub campaign: Vec<u8>,
}

/// On-disk replay schema version. Version 35 uses one canonical script-global
/// vector in authoritative snapshots and state hashes instead of parallel
/// imported-vector and live ID/value-map storage. Version 34 makes the ordered pending AI
/// detectable-mutation FIFO authoritative snapshot and state-hash data.
/// Version 33 embeds serde JSON saves in load
/// records when a pinned timeline marker cannot reproduce the loaded state.
/// Version 31 adds the opt-in reversible
/// background-patch configuration and remembered activation targets to
/// deterministic state. Version 30 makes vector fog part of the
/// deterministic state-hash contract; the replay JSONL record shape itself is
/// unchanged. The current schema combines the exact bounded
/// canonical Spellforge package and its deterministic journal/digest state with
/// every current-main simulation field documented below.
///
/// Version 15 combined the completed native
/// bitcode campaign/compact encoding with the strict full-frame boundary:
/// every admitted [`SimulationFrameInput`] and host control is explicit,
/// serialized command chains are non-recursive, stored movement actions carry
/// exact routes, and point-Seek state carries explicit route provenance.
/// Version 17 requires full-fidelity campaign history. Version 18 adds
/// achievement tracker state, and version 19 combines that with expanded item
/// rules, cached ale eligibility, and ground-stone state. Version 20 combines
/// that state with authoritative Sherwood trading configuration, commands,
/// and receipts. Version 21 carries resolved Legendary/Custom difficulty
/// rules alongside both feature families. Version 22 adds typed nullable AI
/// entity handles and exact spatial provenance to that complete state: arena
/// slot zero is a live entity and absence is encoded by `Option`, so the
/// preceding raw-zero layout cannot be decoded safely. Version 23 adds
/// deterministic authored-mission timers and runtime ambience scheduling to
/// the simulation config and recorded engine state. Version 24 adds mission
/// diplomacy state, player-coalition policy, faction statistics, and runtime
/// relationship commands. Version 25 adds the authoritative composite-gesture
/// and quality-damage rules plus resolved technique and quality state across
/// commands, quick actions, active sequences, and active sweeps. Version 26
/// adds per-seat planned shield prompts and deterministic tactical queue
/// formations to the completed planned quick-action payloads. Version 27 adds
/// deterministic shared-vision fog state and its host-owned toggle command to
/// the complete engine snapshots and input stream. Version 28 adds the
/// mandatory, bounded cold-start mission-asset descriptor. Version 29 adds the
/// canonical ranked input-provenance contract to that complete mission-assets
/// and Spellforge-capable replay. It remains one replay artifact: participant
/// identity and signatures live in the submission envelope rather than a
/// second replay representation. There is deliberately no Rust-schema
/// compatibility adapter; earlier incompatible layouts are rejected at the
/// header.
pub const REPLAY_SCHEMA_VERSION: u32 = 35;

/// Identity of the next lockstep/history transaction to be admitted.
///
/// This is intentionally neither a replay record ordinal nor an engine
/// simulation tick. Paused graphical host iterations can create replay records
/// without advancing this cursor, and an admitted frame can leave
/// `Engine::simulation_tick()` fixed when the Original hourglass gate freezes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TimelineFrame(u32);

impl TimelineFrame {
    pub const ZERO: Self = Self(0);

    pub const fn from_wire(value: u32) -> Self {
        Self(value)
    }

    pub const fn number(self) -> u32 {
        self.0
    }

    pub fn next(self) -> Self {
        Self(
            self.0
                .checked_add(1)
                .expect("authoritative timeline frame overflow"),
        )
    }

    pub fn previous(self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }
}

/// Dense ordinal of the next host-iteration record in a replay stream.
/// Multiple ordinals may name the same [`TimelineFrame`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReplayFrameOrdinal(u32);

impl ReplayFrameOrdinal {
    pub const fn from_wire(value: u32) -> Self {
        Self(value)
    }

    pub const ZERO: Self = Self(0);

    pub const fn number(self) -> u32 {
        self.0
    }

    pub fn advance(&mut self) {
        self.0 = self
            .0
            .checked_add(1)
            .expect("replay host-frame ordinal overflow");
    }
}

/// A recorded in-mission load and the slot-specific post-load behavior that
/// must be reproduced after restoring its earlier save marker.
#[derive(
    Clone,
    Debug,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ReplayLoadBack {
    /// Complete serde JSON save captured before load fixups, instead of a pinned marker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<ReplaySaveSnapshot>,
    /// Earlier save-marker frame whose captured state must be restored. A
    /// pristine Restart may pin and restore marker 0 at ordinal 0 itself.
    /// Embedded restores set this to their own ordinal and need no marker.
    pub to_frame: u32,
    /// Whether the source slot was the Continue auto-save.
    pub is_continue: bool,
}

/// Exact persisted save payload and the recording's adopted timeline boundary.
#[derive(
    Clone,
    Debug,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ReplaySaveSnapshot {
    pub payload: Vec<u8>,
    pub timeline_frame: u32,
}

/// State pinned by an in-mission save at one replay host ordinal.
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ReplaySaveMarker {
    pub state_hash: u64,
    pub timeline_frame: u32,
}

/// Presentation-side input needed to reproduce host-loop behavior but which
/// must never be smuggled into [`SimulationFrameInput`] as an engine command.
#[derive(
    Clone,
    Debug,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReplayHostControl {
    ModalDismiss {
        modal: ModalKind,
        result: DialogResult,
    },
}

/// One complete recorded host frame: the authoritative engine transaction and
/// its explicitly separate presentation controls.
#[derive(
    Clone,
    Debug,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ReplayFrame {
    /// Lockstep/history frame on entry and after host commit. They are
    /// deliberately distinct from the dense replay ordinal.
    pub timeline_before: u32,
    pub timeline_after: u32,
    pub input: SimulationFrameInput,
    pub host_controls: Vec<ReplayHostControl>,
}

/// One JSONL line. Carries one complete frame input, a periodic engine-state
/// hash used for desync detection on replay, and/or in-mission
/// save-marker / load-back timeline records.
#[derive(
    Clone,
    Debug,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(deny_unknown_fields)]
struct FrameRecord {
    /// Dense replay host-frame ordinal (0-based), distinct from the lockstep
    /// frame carried by [`ReplayFrame::timeline_frame`].
    f: u32,
    /// Complete authoritative input for this admitted simulation frame.
    /// Marker-only records omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    i: Option<ReplayFrame>,
    /// Hash of deterministic engine state, written once per second
    /// (every 25 frames).  Used by the player to detect desyncs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    h: Option<u64>,
    /// Save marker: an in-mission save captured the engine state at this
    /// frame's pre-command boundary.  Carries the state hash at capture so
    /// playback can pin a clone of that state (for later load-backs) and
    /// verify it against the recording.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sv: Option<ReplaySaveMarker>,
    /// Load-back record: at this frame's pre-command boundary the engine
    /// state was replaced with the state captured by the save marker at the
    /// referenced earlier frame (or frame 0 itself for a pristine Restart).
    /// Unknown saves carry an embedded payload instead of a marker reference.
    /// Both forms keep the recording linear across in-mission loads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lb: Option<ReplayLoadBack>,
    /// First observations of ranked-ineligible input paths. These records do
    /// not advance the dense replay ordinal.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    t: Vec<InputTaint>,
}

/// Derive every ranked taint whose origin is unambiguous in deterministic
/// replay data. Explicit recorder markers remain necessary for entry paths
/// such as HTTP stepping and headless automation that otherwise leave no
/// distinct simulation command behind.
fn detected_input_taints(
    input: &SimulationFrameInput,
    host_controls: &[ReplayHostControl],
) -> BTreeSet<InputTaintKind> {
    use crate::engine::ExternalAction;
    use crate::player_command::PlayerCommand;

    let mut taints = BTreeSet::new();
    for action in input
        .external_actions
        .iter()
        .chain(&input.post_external_actions)
    {
        match action {
            ExternalAction::ConsoleCommand { .. } => {
                taints.insert(InputTaintKind::ConsoleCommand);
            }
            ExternalAction::EzekielInstakill { .. } => {
                taints.insert(InputTaintKind::CheatCommand);
            }
            ExternalAction::Native { .. } => {
                taints.insert(InputTaintKind::HttpStateMutation);
            }
            ExternalAction::ReplaceCampaign { .. } => {
                taints.insert(InputTaintKind::CheatCommand);
            }
            ExternalAction::SimpleMessage { .. }
            | ExternalAction::CampaignBuyBlazon { .. }
            | ExternalAction::AcknowledgePseudoMissionDebrief => {}
        }
    }
    for command in input.commands.iter().chain(&input.post_commands) {
        match &command.player_input().command {
            PlayerCommand::SetGoldenEyeMode { .. }
            | PlayerCommand::RevealAllBlips
            | PlayerCommand::TeleportSelectedToPoint { .. } => {
                taints.insert(InputTaintKind::CheatCommand);
            }
            PlayerCommand::ModalDismiss { result, .. } => match result {
                DialogResult::Restart => {
                    taints.insert(InputTaintKind::MissionRestart);
                }
                DialogResult::Load { .. } => {
                    taints.insert(InputTaintKind::StateLoad);
                }
                DialogResult::Completed | DialogResult::Aborted => {}
            },
            _ => {}
        }
    }
    for control in host_controls {
        match control {
            ReplayHostControl::ModalDismiss { result, .. } => match result {
                DialogResult::Restart => {
                    taints.insert(InputTaintKind::MissionRestart);
                }
                DialogResult::Load { .. } => {
                    taints.insert(InputTaintKind::StateLoad);
                }
                DialogResult::Completed | DialogResult::Aborted => {}
            },
        }
    }
    taints
}

/// A complete recorded replay loaded into memory.
/// Validated frame maps are immutable and shared across admission/player clones.
/// The exported ReplayFile retains the exact existing wire representation.
#[derive(Clone, Debug)]
pub struct ReplayData {
    header: ReplayHeader,
    /// Dense logical sequence, keyed explicitly so marker records can remain
    /// independent JSONL lines. Every frame in `0..total_frames` is present.
    frames: std::sync::Arc<BTreeMap<u32, ReplayFrame>>,
    /// Sparse map: expected engine state hash at the start of frame N.
    hashes: std::sync::Arc<BTreeMap<u32, u64>>,
    /// Save markers: frame → state hash at the pre-command boundary of
    /// that frame, where an in-mission save captured the engine state.
    save_markers: std::sync::Arc<BTreeMap<u32, ReplaySaveMarker>>,
    /// Load-back records: frame → earlier save-marker frame whose
    /// captured state replaced the engine at this frame's boundary.
    load_backs: std::sync::Arc<BTreeMap<u32, ReplayLoadBack>>,
}

/// Flat serde-compatible snapshot of a [`ReplayData`], used as the
/// payload for the compact `rhrec-{hash}-{base64}` sharing format.
/// Kept separate from `ReplayData` so the in-memory representation
/// can evolve without breaking binary compatibility.
#[derive(Clone, Debug, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub struct ReplayFile {
    pub header: ReplayHeader,
    pub frames: BTreeMap<u32, ReplayFrame>,
    pub hashes: BTreeMap<u32, u64>,
    pub save_markers: BTreeMap<u32, ReplaySaveMarker>,
    pub load_backs: BTreeMap<u32, ReplayLoadBack>,
}

impl TryFrom<ReplayFile> for ReplayData {
    type Error = String;

    fn try_from(f: ReplayFile) -> Result<Self, Self::Error> {
        let mut data = Self {
            header: f.header,
            frames: f.frames.into(),
            hashes: f.hashes.into(),
            save_markers: f.save_markers.into(),
            load_backs: f.load_backs.into(),
        };
        data.validate_layout()?;
        data.header.rankability = data.rankability().map_err(|error| error.to_string())?;
        Ok(data)
    }
}

impl From<&ReplayData> for ReplayFile {
    fn from(d: &ReplayData) -> Self {
        Self {
            header: d.header.clone(),
            frames: (*d.frames).clone(),
            hashes: (*d.hashes).clone(),
            save_markers: (*d.save_markers).clone(),
            load_backs: (*d.load_backs).clone(),
        }
    }
}

impl ReplayData {
    /// Validated metadata. Modify a raw `ReplayFile` and validate it again to
    /// change the replay's layout or provenance.
    pub fn header(&self) -> &ReplayHeader {
        &self.header
    }

    /// Apply metadata changes transactionally. An invalid edit leaves this
    /// replay unchanged, including its reconstructed provenance evidence.
    pub fn try_edit_header(&mut self, edit: impl FnOnce(&mut ReplayHeader)) -> Result<(), String> {
        let mut candidate = self.clone();
        edit(&mut candidate.header);
        candidate.validate_layout()?;
        candidate.header.rankability =
            candidate.rankability().map_err(|error| error.to_string())?;
        *self = candidate;
        Ok(())
    }
    /// Total number of simulation frames in the replay.
    pub fn frame_count(&self) -> u32 {
        self.header.total_frames
    }

    /// Replace deterministic checkpoints after regenerating the exact
    /// canonical timeline from frame zero.
    pub fn replace_state_hashes(&mut self, hashes: BTreeMap<u32, u64>) -> Result<(), String> {
        self.validate_metadata_ordinals("hash", hashes.keys().copied())?;
        self.hashes = hashes.into();
        Ok(())
    }

    fn validate_metadata_ordinals(
        &self,
        kind: &str,
        ordinals: impl Iterator<Item = u32>,
    ) -> Result<(), String> {
        for ordinal in ordinals {
            if ordinal >= self.header.total_frames {
                return Err(format!(
                    "replay {kind} ordinal {ordinal} is outside total_frames {}",
                    self.header.total_frames
                ));
            }
        }
        Ok(())
    }

    /// Validate invariants shared by JSONL and compact replay containers.
    pub fn validate_layout(&self) -> Result<(), String> {
        if self.header.version != REPLAY_SCHEMA_VERSION {
            return Err(format!(
                "unsupported replay schema version {}; expected {REPLAY_SCHEMA_VERSION}",
                self.header.version
            ));
        }
        // Metadata belongs to an admitted host frame; EOF is not a frame.
        // This is the same strict range enforced at public codec admission.
        self.validate_metadata_ordinals("hash", self.hashes.keys().copied())?;
        self.validate_metadata_ordinals("save marker", self.save_markers.keys().copied())?;
        self.validate_metadata_ordinals("load-back", self.load_backs.keys().copied())?;
        let rankability = self
            .rankability()
            .map_err(|error| format!("invalid replay input provenance: {error}"))?;
        if let Some(taint) = rankability
            .taints()
            .iter()
            .find(|taint| taint.first_frame > self.header.total_frames)
        {
            return Err(format!(
                "replay taint {} starts at {}, after boundary {}",
                taint.kind.stable_code(),
                taint.first_frame,
                self.header.total_frames
            ));
        }
        self.header
            .mission_assets
            .validate_spellforge_package(self.header.spellforge_package.as_ref())
            .map_err(|error| error.to_string())?;
        if !self
            .header
            .mission_id
            .eq_ignore_ascii_case(&self.header.mission_assets.mission_basename)
        {
            return Err(format!(
                "replay mission ID `{}` does not match asset descriptor mission `{}`",
                self.header.mission_id, self.header.mission_assets.mission_basename
            ));
        }
        if let Some(package) = &self.header.spellforge_package {
            package
                .validate_wire()
                .map_err(|error| format!("invalid replay Spellforge package: {error}"))?;
        }
        for frame in 0..self.header.total_frames {
            if !self.frames.contains_key(&frame) {
                return Err(format!(
                    "replay is missing authoritative simulation input for frame {frame}"
                ));
            }
        }
        for (&frame, load_back) in self.load_backs.iter() {
            if let Some(snapshot) = &load_back.snapshot {
                if snapshot.payload.is_empty() || load_back.to_frame != frame {
                    return Err(format!("invalid embedded save at frame {frame}"));
                }
                continue;
            }
            let pristine_restart = frame == 0
                && load_back.to_frame == 0
                && self
                    .save_markers
                    .get(&0)
                    .is_some_and(|marker| marker.timeline_frame == 0);
            if load_back.to_frame >= frame && !pristine_restart {
                return Err(format!(
                    "load-back target {} is not before frame {frame}",
                    load_back.to_frame
                ));
            }
            if !self.save_markers.contains_key(&load_back.to_frame) {
                return Err(format!(
                    "load-back at frame {frame} references frame {}, which has no save marker",
                    load_back.to_frame
                ));
            }
        }
        let mut previous_after = None;
        for (&ordinal, frame) in self.frames.iter() {
            if frame.timeline_after < frame.timeline_before
                || frame.timeline_after > frame.timeline_before.saturating_add(1)
            {
                return Err(format!(
                    "replay frame {ordinal} has invalid timeline transition {} -> {}",
                    frame.timeline_before, frame.timeline_after
                ));
            }
            if let Some(load_back) = self.load_backs.get(&ordinal) {
                if let Some(snapshot) = &load_back.snapshot {
                    if frame.timeline_before != snapshot.timeline_frame {
                        return Err(format!(
                            "embedded save at ordinal {ordinal} has inconsistent timeline"
                        ));
                    }
                    previous_after = Some(frame.timeline_after);
                    continue;
                }
                let marker = self
                    .save_markers
                    .get(&load_back.to_frame)
                    .expect("load-back marker existence checked above");
                if frame.timeline_before != marker.timeline_frame {
                    return Err(format!(
                        "replay load-back frame {ordinal} starts at timeline {}, saved timeline was {}",
                        frame.timeline_before, marker.timeline_frame
                    ));
                }
            } else if let Some(previous_after) = previous_after
                && frame.timeline_before != previous_after
            {
                return Err(format!(
                    "replay frame {ordinal} starts at timeline {}, previous frame ended at {previous_after}",
                    frame.timeline_before
                ));
            }
            previous_after = Some(frame.timeline_after);
        }
        if self
            .frames
            .last_key_value()
            .is_some_and(|(&frame, _)| frame >= self.header.total_frames)
        {
            return Err(format!(
                "replay contains frame input outside total_frames {}",
                self.header.total_frames
            ));
        }
        Ok(())
    }

    /// Complete authoritative input for a recorded frame.
    pub fn frame(&self, frame: u32) -> Option<&ReplayFrame> {
        self.frames.get(&frame)
    }

    /// Expected engine-state hash at the start of the given frame,
    /// or `None` if no hash was recorded for that frame.
    pub fn hash_for_frame(&self, frame: u32) -> Option<u64> {
        self.hashes.get(&frame).copied()
    }

    /// State hash captured by an in-mission save at the pre-command
    /// boundary of `frame`, if a save marker was recorded there.
    pub fn save_marker_for_frame(&self, frame: u32) -> Option<ReplaySaveMarker> {
        self.save_markers.get(&frame).copied()
    }

    /// Earlier save-marker frame whose captured state replaced the engine
    /// at the pre-command boundary of `frame`, if a load-back was recorded.
    pub fn load_back_for_frame(&self, frame: u32) -> Option<&ReplayLoadBack> {
        self.load_backs.get(&frame)
    }

    /// Load a replay from a JSONL reader.
    pub fn from_reader(mut reader: impl BufRead) -> Result<Self, String> {
        let mut header_bytes = Vec::new();
        let read = reader
            .by_ref()
            .take(REPLAY_HEADER_JSON_BYTE_LIMIT.saturating_add(1))
            .read_until(b'\n', &mut header_bytes)
            .map_err(|e| format!("read error: {e}"))?;
        if read == 0 {
            return Err("empty replay file".to_owned());
        }
        if read as u64 > REPLAY_HEADER_JSON_BYTE_LIMIT {
            return Err(format!(
                "replay header exceeds {REPLAY_HEADER_JSON_BYTE_LIMIT} bytes"
            ));
        }
        while matches!(header_bytes.last(), Some(b'\n' | b'\r')) {
            header_bytes.pop();
        }
        let header_line = std::str::from_utf8(&header_bytes)
            .map_err(|error| format!("bad header UTF-8: {error}"))?;
        let header_value: serde_json::Value =
            serde_json::from_str(header_line).map_err(|e| format!("bad header: {e}"))?;
        let version = header_value
            .get("version")
            .and_then(serde_json::Value::as_u64)
            .ok_or("bad header: missing integer version")? as u32;
        if version != REPLAY_SCHEMA_VERSION {
            return Err(format!(
                "unsupported replay schema version {}; expected {REPLAY_SCHEMA_VERSION}",
                version
            ));
        }
        let mut header: ReplayHeader =
            serde_json::from_value(header_value).map_err(|e| format!("bad header: {e}"))?;
        let mut frames = BTreeMap::new();
        let mut hashes = BTreeMap::new();
        let mut save_markers = BTreeMap::new();
        let mut load_backs = BTreeMap::new();
        let mut max_frame: u32 = 0;
        let lines = reader.lines();
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
            if let Some(sv) = rec.sv {
                save_markers.insert(rec.f, sv);
            }
            if let Some(lb) = rec.lb {
                let pristine_restart = rec.f == 0
                    && lb.to_frame == 0
                    && save_markers
                        .get(&0)
                        .is_some_and(|marker| marker.timeline_frame == 0);
                if lb.snapshot.is_none() && lb.to_frame >= rec.f && !pristine_restart {
                    return Err(format!(
                        "bad line {}: load-back target {} is not before frame {}",
                        i + 2,
                        lb.to_frame,
                        rec.f
                    ));
                }
                load_backs.insert(rec.f, lb);
            }
            for taint in rec.t {
                header.rankability.include(taint);
            }
            if let Some(recorded_frame) = rec.i
                && frames.insert(rec.f, recorded_frame).is_some()
            {
                return Err(format!(
                    "bad line {}: duplicate simulation input for frame {}",
                    i + 2,
                    rec.f
                ));
            }
        }
        // Every load-back must reference a save marker recorded earlier in
        // this same file — playback pins engine state at marker frames and
        // has nothing to jump to otherwise.
        for (&frame, load_back) in &load_backs {
            if load_back.snapshot.is_none() && !save_markers.contains_key(&load_back.to_frame) {
                return Err(format!(
                    "load-back at frame {frame} references frame {}, which has no save marker",
                    load_back.to_frame
                ));
            }
        }
        // During streaming the header cannot know the final frame count.
        if header.total_frames == 0 && max_frame > 0 {
            header.total_frames = max_frame;
        }
        let data = ReplayFile {
            header,
            frames,
            hashes,
            save_markers,
            load_backs,
        };
        data.try_into()
    }

    /// Load a replay from a JSONL file on disk.
    pub fn from_file(path: &str) -> Result<Self, String> {
        let file = std::fs::File::open(path).map_err(|e| format!("open {path}: {e}"))?;
        Self::from_reader(std::io::BufReader::new(file))
    }
}

/// Compute a hash of the deterministic engine state. Host/render/input
/// state stays outside the engine snapshot, while explicit snapshot schemas
/// hash only their deterministic fields.
///
/// Walks the `EngineInner` field-by-field via `StateHash` (defined in
/// `robin_util`), feeding bytes into fixed-width little-endian XXH3. Floats hash via
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player_command::{PlayerCommand, PlayerInput};

    #[test]
    fn replay_schema_version_includes_canonical_script_global_vector() {
        assert_eq!(REPLAY_SCHEMA_VERSION, 35);
    }

    #[test]
    fn replay_coordinate_wire_forms_remain_plain_integers() {
        assert_eq!(
            serde_json::to_string(&TimelineFrame::from_wire(42)).unwrap(),
            "42"
        );
        assert_eq!(
            serde_json::to_string(&ReplayFrameOrdinal::from_wire(42)).unwrap(),
            "42"
        );
        assert_eq!(TimelineFrame::ZERO.previous(), None);
        assert_eq!(
            TimelineFrame::from_wire(42).previous().unwrap().number(),
            41
        );
    }

    #[test]
    fn raw_replay_validation_and_header_edits_are_transactional() {
        let file = ReplayFile {
            header: ReplayHeader {
                mission_id: "validation".to_owned(),
                mission_assets: test_mission_assets("validation"),
                rng_seed: 17,
                sim_config: crate::engine::SimConfig::default(),
                spellforge_package: None,
                version: REPLAY_SCHEMA_VERSION,
                total_frames: 0,
                rankability: ReplayRankability::rankable(),
                campaign: bitcode::encode(&crate::campaign::Campaign::default()),
            },
            frames: BTreeMap::new(),
            hashes: BTreeMap::new(),
            save_markers: BTreeMap::new(),
            load_backs: BTreeMap::new(),
        };
        let mut data = ReplayData::try_from(file).unwrap();
        let mut shared = data.clone();
        assert!(std::sync::Arc::ptr_eq(&data.frames, &shared.frames));
        assert!(std::sync::Arc::ptr_eq(&data.hashes, &shared.hashes));
        assert!(std::sync::Arc::ptr_eq(
            &data.save_markers,
            &shared.save_markers
        ));
        assert!(std::sync::Arc::ptr_eq(&data.load_backs, &shared.load_backs));
        assert_eq!(
            bitcode::encode(&ReplayFile::from(&data)),
            bitcode::encode(&ReplayFile::from(&shared))
        );
        shared
            .try_edit_header(|header| header.rng_seed = 42)
            .unwrap();
        assert_eq!(data.header().rng_seed, 17);
        assert!(std::sync::Arc::ptr_eq(&data.frames, &shared.frames));
        shared.replace_state_hashes(BTreeMap::new()).unwrap();
        assert!(!std::sync::Arc::ptr_eq(&data.hashes, &shared.hashes));
        assert!(std::sync::Arc::ptr_eq(&data.frames, &shared.frames));
        assert!(
            data.try_edit_header(|header| {
                header.rng_seed = 99;
                header.total_frames = 1;
            })
            .unwrap_err()
            .contains("missing authoritative")
        );
        assert_eq!(data.header().rng_seed, 17);
        assert_eq!(data.frame_count(), 0);
        for kind in 0..3 {
            let mut outside = ReplayFile::from(&data);
            match kind {
                0 => {
                    outside.hashes.insert(0, 7);
                }
                1 => {
                    outside.save_markers.insert(
                        0,
                        ReplaySaveMarker {
                            state_hash: 7,
                            timeline_frame: 0,
                        },
                    );
                }
                _ => {
                    outside.load_backs.insert(
                        0,
                        ReplayLoadBack {
                            snapshot: None,
                            to_frame: 0,
                            is_continue: false,
                        },
                    );
                }
            }
            assert!(
                ReplayData::try_from(outside)
                    .unwrap_err()
                    .contains("outside total_frames")
            );
        }
        let mut single = ReplayFile::from(&data);
        single.header.total_frames = 1;
        single.frames.insert(
            0,
            ReplayFrame {
                timeline_before: 0,
                timeline_after: 1,
                input: SimulationFrameInput::default(),
                host_controls: Vec::new(),
            },
        );
        single.hashes.insert(0, 7);
        let mut single = ReplayData::try_from(single).unwrap();
        assert!(
            single
                .replace_state_hashes(BTreeMap::from([(1, 9)]))
                .is_err()
        );
        assert_eq!(single.hash_for_frame(0), Some(7));
        assert_eq!(single.hash_for_frame(1), None);
        let mut malformed = ReplayFile::from(&data);
        malformed.header.version = REPLAY_SCHEMA_VERSION - 1;
        assert!(
            ReplayData::try_from(malformed)
                .unwrap_err()
                .contains("unsupported replay schema")
        );
    }

    fn unique_replay_path(label: &str) -> String {
        std::env::temp_dir()
            .join(format!(
                "robin_{label}_{}_{}.rhrec.jsonl",
                std::process::id(),
                std::thread::current().name().unwrap_or("unnamed")
            ))
            .to_string_lossy()
            .into_owned()
    }

    fn test_mission_assets(mission: &str) -> crate::mission_assets::MissionAssetDescriptor {
        crate::mission_assets::MissionAssetDescriptor::built_in(mission, mission, mission)
            .expect("valid built-in test mission descriptor")
    }

    fn record_tick(recorder: &mut ReplayRecorder, ordinal: u32, input: SimulationFrameInput) {
        assert!(recorder.write_frame(ordinal, ordinal, ordinal + 1, input, Vec::new(), None,));
    }

    #[test]
    fn record_and_playback_roundtrip() {
        let dir = std::env::temp_dir().join("replay_test_full_frames.jsonl");
        let path = dir.to_str().unwrap();

        {
            let campaign = crate::campaign::Campaign::default();
            let mut rec = ReplayRecorder::new(
                path,
                "test_mission".into(),
                test_mission_assets("test_mission"),
                42,
                crate::engine::SimConfig::default(),
                &campaign,
            )
            .unwrap();

            // Frame 0: one command.
            record_tick(
                &mut rec,
                0,
                SimulationFrameInput::new(vec![PlayerCommand::SelectAllPcs.into()]),
            );

            // Frames 1-49: explicit empty authoritative frames.
            for ordinal in 1..50 {
                record_tick(&mut rec, ordinal, SimulationFrameInput::default());
            }

            // Frame 50: two commands
            record_tick(
                &mut rec,
                50,
                SimulationFrameInput::new(vec![
                    PlayerCommand::GroupMove {
                        actors: vec![crate::element::EntityId::Pc(crate::entity_id::PcId(1))],
                        destination: crate::coordinates::MapPoint::new(100.0, 200.0),
                        running: false,
                        show_marker: true,
                        goal_override: None,
                        goal_sector_index_override: None,
                        door_route_override: None,
                        recorded_gate_routes: Vec::new(),
                        recorded_failed_gate_routes: Vec::new(),
                    }
                    .into(),
                    PlayerCommand::CrouchDown.into(),
                ]),
            );
        }

        // Every frame is explicit: header + 51 frame lines.
        let contents = std::fs::read_to_string(path).unwrap();
        assert_eq!(contents.lines().count(), 52);

        let data = ReplayData::from_file(path).unwrap();
        assert_eq!(data.frame_count(), 51);
        assert_eq!(data.header.rng_seed, 42);

        let mut player = ReplayPlayer::new(data);
        assert!(!player.is_finished());

        // Frame 0: one command
        let f0 = player.next_frame();
        assert_eq!(f0.input.commands.len(), 1);

        // Frames 1-49: empty
        for _ in 1..50 {
            let empty = player.next_frame();
            assert!(empty.input.commands.is_empty());
        }

        // Frame 50: two commands
        let f50 = player.next_frame();
        assert_eq!(f50.input.commands.len(), 2);

        assert!(player.is_finished());

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn replay_hash_roundtrip_distinguishes_live_ai_slot_zero_from_absence() {
        let mut live = crate::engine::EngineInner::new();
        let mut ai = crate::ai_enemy::EnemyAi::new(0);
        ai.base.primary_target = Some(crate::ai::AiEntityHandle::new(0));
        let owner = live.add_entity(crate::element::Entity::Soldier(
            crate::element::ActorSoldier {
                element: {
                    let mut initial_element = crate::element::ElementData::default();
                    initial_element.kind = crate::element::ElementKind::ActorSoldier;
                    initial_element
                },
                actor: Default::default(),
                human: Default::default(),
                npc: {
                    crate::element::NpcData {
                        ai: crate::element::AiActorData {
                            ai_brain: crate::element::AiBrain::Enemy(Box::new(ai)),
                            ..Default::default()
                        },
                        ..Default::default()
                    }
                },
                soldier: Default::default(),
            },
        ));
        assert_eq!(owner.index(), 0);
        let live_slot_zero_hash = state_hash(&live);
        let mut absent = live;
        absent
            .get_entity_mut(owner)
            .and_then(crate::element::Entity::enemy_ai_mut)
            .unwrap()
            .base
            .primary_target = None;
        assert_ne!(live_slot_zero_hash, state_hash(&absent));

        let path = unique_replay_path("typed_slot_zero_hash");
        let campaign = crate::campaign::Campaign::default();
        let mut recorder = ReplayRecorder::new(
            &path,
            "typed_slot_zero".into(),
            test_mission_assets("typed_slot_zero"),
            42,
            crate::engine::SimConfig::default(),
            &campaign,
        )
        .unwrap();
        recorder.write_hash(0, live_slot_zero_hash);
        record_tick(&mut recorder, 0, SimulationFrameInput::default());
        drop(recorder);

        let data = ReplayData::from_file(&path).expect("load current replay schema");
        assert_eq!(data.hash_for_frame(0), Some(live_slot_zero_hash));
        let player = ReplayPlayer::new(data);
        assert_eq!(player.hash_for_frame(0), Some(live_slot_zero_hash));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn every_obsolete_rust_jsonl_schema_is_rejected() {
        for version in 10..REPLAY_SCHEMA_VERSION {
            let input = format!(
                "{{\"mission_id\":\"old\",\"rng_seed\":7,\"version\":{version},\"total_frames\":0,\"campaign\":null}}\n"
            );
            let error = ReplayData::from_reader(std::io::Cursor::new(input))
                .expect_err("pre-merge replay schemas are not current snapshots");
            assert!(
                error.contains(&format!("unsupported replay schema version {version}")),
                "{error}"
            );
        }
    }

    #[test]
    fn metadata_without_its_forced_frame_is_rejected_as_truncated() {
        let header = serde_json::json!({
            "mission_id": "truncated-marker",
            "mission_assets": test_mission_assets("truncated-marker"),
            "rng_seed": 7,
            "sim_config": crate::engine::SimConfig::default(),
            "spellforge_package": null,
            "version": REPLAY_SCHEMA_VERSION,
            "total_frames": 0,
            "rankability": ReplayRankability::rankable(),
            "campaign": bitcode::encode(&crate::campaign::Campaign::default()),
        });
        let input = format!(
            "{header}\n{}\n",
            serde_json::json!({
                "f": 0,
                "sv": { "state_hash": 123, "timeline_frame": 0 }
            })
        );
        let error = ReplayData::from_reader(std::io::Cursor::new(input))
            .expect_err("marker metadata without its forced transaction is truncated");
        assert!(
            error.contains("missing authoritative simulation input for frame 0"),
            "{error}"
        );
    }

    #[test]
    fn current_schema_rejects_command_only_frame_records() {
        let header = serde_json::json!({
            "mission_id": "old-command-frame",
            "mission_assets": test_mission_assets("old-command-frame"),
            "rng_seed": 7,
            "sim_config": crate::engine::SimConfig::default(),
            "spellforge_package": null,
            "version": REPLAY_SCHEMA_VERSION,
            "total_frames": 1,
            "rankability": ReplayRankability::rankable(),
            "campaign": bitcode::encode(&crate::campaign::Campaign::default()),
        });
        let input = format!("{header}\n{{\"f\":0,\"c\":[]}}\n");
        let error = ReplayData::from_reader(std::io::Cursor::new(input))
            .expect_err("command-only records are not current full frames");
        assert!(error.contains("unknown field `c`"), "{error}");
    }

    #[test]
    fn current_schema_rejects_frame_without_host_controls() {
        let header = serde_json::json!({
            "mission_id": "missing-host-controls",
            "mission_assets": test_mission_assets("missing-host-controls"),
            "rng_seed": 7,
            "sim_config": crate::engine::SimConfig::default(),
            "spellforge_package": null,
            "version": REPLAY_SCHEMA_VERSION,
            "total_frames": 1,
            "rankability": ReplayRankability::rankable(),
            "campaign": bitcode::encode(&crate::campaign::Campaign::default()),
        });
        let frame = serde_json::json!({
            "timeline_before": 0,
            "timeline_after": 1,
            "input": SimulationFrameInput::default(),
        });
        let input = format!("{header}\n{}\n", serde_json::json!({ "f": 0, "i": frame }));

        let error = ReplayData::from_reader(std::io::Cursor::new(input))
            .expect_err("current replay frames must explicitly include host controls");
        assert!(error.contains("missing field `host_controls`"), "{error}");
    }

    #[test]
    fn current_schema_rejects_load_back_without_is_continue() {
        let header = serde_json::json!({
            "mission_id": "missing-is-continue",
            "mission_assets": test_mission_assets("missing-is-continue"),
            "rng_seed": 7,
            "sim_config": crate::engine::SimConfig::default(),
            "spellforge_package": null,
            "version": REPLAY_SCHEMA_VERSION,
            "total_frames": 0,
            "rankability": ReplayRankability::rankable(),
            "campaign": bitcode::encode(&crate::campaign::Campaign::default()),
        });
        let input = format!(
            "{header}\n{}\n",
            serde_json::json!({ "f": 1, "lb": { "to_frame": 0 } })
        );

        let error = ReplayData::from_reader(std::io::Cursor::new(input))
            .expect_err("current load-backs must identify Continue-slot behavior");
        assert!(error.contains("missing field `is_continue`"), "{error}");
    }

    #[test]
    fn recorder_skips_only_pure_paused_noops_and_round_trips_complete_phases() {
        let path = unique_replay_path("complete_frame_phases");
        let campaign = crate::campaign::Campaign::default();
        let mut recorder = ReplayRecorder::new(
            &path,
            "phases".into(),
            test_mission_assets("phases"),
            11,
            crate::engine::SimConfig::default(),
            &campaign,
        )
        .expect("recorder");

        assert!(!recorder.write_frame(
            0,
            7,
            7,
            SimulationFrameInput::no_hourglass(),
            Vec::new(),
            None,
        ));

        let input = SimulationFrameInput::new(vec![PlayerCommand::CrouchDown.into()])
            .with_external_facts(
                crate::engine::ExternalFacts::default()
                    .with_sound_boundary(crate::engine::SoundBoundary::replay(Vec::new())),
            )
            .with_external_actions(vec![crate::engine::ExternalAction::Native {
                name: "Before".into(),
                args: vec![1, 2],
                this_actor: None,
            }])
            .with_post_external_actions(vec![crate::engine::ExternalAction::Native {
                name: "After".into(),
                args: vec![3],
                this_actor: Some(4),
            }])
            .with_post_commands(vec![PlayerCommand::StandUp.into()])
            .with_hourglass(false)
            .with_simulation_body_allowed(false)
            .with_post_initialize(true);
        let host_controls = vec![ReplayHostControl::ModalDismiss {
            modal: ModalKind::SherwoodReport,
            result: DialogResult::Completed,
        }];
        assert!(recorder.write_frame(0, 7, 7, input.clone(), host_controls, Some(0x1234),));
        drop(recorder);

        let data = ReplayData::from_file(&path).expect("load full frame replay");
        assert_eq!(data.frame_count(), 1);
        assert_eq!(data.hash_for_frame(0), Some(0x1234));
        let frame = data.frame(0).expect("recorded frame");
        assert_eq!((frame.timeline_before, frame.timeline_after), (7, 7));
        assert_eq!(
            serde_json::to_value(&frame.input).unwrap(),
            serde_json::to_value(&input).unwrap()
        );
        assert!(matches!(
            frame.host_controls.as_slice(),
            [ReplayHostControl::ModalDismiss {
                modal: ModalKind::SherwoodReport,
                result: DialogResult::Completed,
            }]
        ));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn current_recording_round_trips_player_id() {
        use crate::player_command::PlayerId;
        let path = std::env::temp_dir()
            .join("replay_test_current_seats.jsonl")
            .to_str()
            .unwrap()
            .to_string();
        {
            let campaign = crate::campaign::Campaign::default();
            let mut rec = ReplayRecorder::new(
                &path,
                "mp_seats".into(),
                test_mission_assets("mp_seats"),
                99,
                crate::engine::SimConfig::default(),
                &campaign,
            )
            .unwrap();
            record_tick(
                &mut rec,
                0,
                SimulationFrameInput::from_player_inputs(vec![
                    PlayerInput::new(PlayerId(0), PlayerCommand::CrouchDown),
                    PlayerInput::new(PlayerId(2), PlayerCommand::StandUp),
                ]),
            );
        }
        let data = ReplayData::from_file(&path).unwrap();
        assert_eq!(data.header.version, REPLAY_SCHEMA_VERSION);
        let f0 = &data.frame(0).expect("frame zero").input;
        assert_eq!(f0.commands.len(), 2);
        assert_eq!(f0.commands[0].player_input().player_id, PlayerId(0));
        assert_eq!(f0.commands[1].player_input().player_id, PlayerId(2));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn current_replay_round_trips_resolved_minimap_inputs() {
        let path = unique_replay_path("resolved_minimap_inputs");
        let campaign = crate::campaign::Campaign::default();
        let mut recorder = ReplayRecorder::new(
            &path,
            "minimap".into(),
            test_mission_assets("minimap"),
            99,
            crate::engine::SimConfig::default(),
            &campaign,
        )
        .expect("create replay recorder");
        record_tick(
            &mut recorder,
            0,
            SimulationFrameInput::from_player_inputs(vec![
                PlayerInput::host(PlayerCommand::MinimapMouseMove {
                    mouse_pt: crate::coordinates::ScreenPoint::new(11.0, 22.0),
                    left_mouse_down: true,
                    continuing_drag: true,
                }),
                PlayerInput::host(PlayerCommand::MinimapMouseUp { on_minimap: true }),
                PlayerInput::host(PlayerCommand::CenterCameraOnPoint {
                    point: crate::coordinates::MapPoint::new(333.0, 444.0),
                }),
            ]),
        );
        drop(recorder);

        let replay = ReplayData::from_file(&path).expect("load current replay");
        let commands = &replay.frame(0).expect("frame zero").input.commands;
        assert!(matches!(
            &commands[0].player_input().command,
            PlayerCommand::MinimapMouseMove {
                left_mouse_down: true,
                continuing_drag: true,
                ..
            }
        ));
        assert!(matches!(
            &commands[1].player_input().command,
            PlayerCommand::MinimapMouseUp { on_minimap: true }
        ));
        assert!(matches!(
            &commands[2].player_input().command,
            PlayerCommand::CenterCameraOnPoint { point }
                if *point == crate::coordinates::MapPoint::new(333.0, 444.0)
        ));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn profile_setting_commands_apply_live_and_replay_to_the_same_hash() {
        let path = unique_replay_path("speaking_command");
        let campaign = crate::campaign::Campaign::default();
        let commands = vec![
            PlayerCommand::SetAmountOfSpeaking { amount: 9 },
            PlayerCommand::SetUnbindingEnabled { enabled: false },
            PlayerCommand::SetReusableCloaks { enabled: false },
            PlayerCommand::SetItemGameplayConfig {
                config: crate::gameplay_config::ItemGameplayConfig::classic(),
            },
            PlayerCommand::SetNoiseDistractionFeedback { enabled: false },
        ];
        let mut recorder = ReplayRecorder::new(
            &path,
            "speaking".into(),
            test_mission_assets("speaking"),
            0,
            crate::engine::SimConfig::default(),
            &campaign,
        )
        .unwrap();
        record_tick(
            &mut recorder,
            0,
            SimulationFrameInput::new(commands.iter().cloned().map(Into::into).collect()),
        );
        drop(recorder);

        let mut live = crate::engine::EngineInner::new();
        let mut replayed = live.clone();
        let assets = crate::engine::LevelAssets::new();
        let mut live_display = crate::engine::HostDisplayState::default();
        let mut live_input = crate::engine::InputState::default();
        live.apply_local_commands(&mut live_display, &mut live_input, &assets, &commands);
        assert_eq!(live.control.sim_config.amount_of_speaking, 9);
        assert!(!live.control.sim_config.enable_unbinding);
        assert!(!live.control.sim_config.reusable_cloaks);
        assert_eq!(
            live.control.sim_config.item_gameplay,
            crate::gameplay_config::ItemGameplayConfig::classic()
        );
        assert!(!live.control.sim_config.noise_distraction_feedback);

        let data = ReplayData::from_file(&path).unwrap();
        let replay_commands = ReplayPlayer::new(data)
            .next_frame()
            .input
            .commands
            .iter()
            .map(|input| input.player_input().command.clone())
            .collect::<Vec<_>>();
        let mut replay_display = crate::engine::HostDisplayState::default();
        let mut replay_input = crate::engine::InputState::default();
        replayed.apply_local_commands(
            &mut replay_display,
            &mut replay_input,
            &assets,
            &replay_commands,
        );

        assert_eq!(replayed.control.sim_config.amount_of_speaking, 9);
        assert!(!replayed.control.sim_config.enable_unbinding);
        assert!(!replayed.control.sim_config.reusable_cloaks);
        assert_eq!(
            replayed.control.sim_config.item_gameplay,
            crate::gameplay_config::ItemGameplayConfig::classic()
        );
        assert!(!replayed.control.sim_config.noise_distraction_feedback);
        assert_eq!(state_hash(&live), state_hash(&replayed));
        let _ = std::fs::remove_file(path);
    }

    fn sherwood_fixture() -> (
        crate::campaign::Campaign,
        crate::engine::LevelAssets,
        crate::level_data::LoadedLevel,
    ) {
        let mut profiles = crate::profiles::ProfileManager::new();
        profiles.missions.push(crate::profiles::MissionProfile {
            location: crate::profiles::MissionLocation::Sherwood,
            ..Default::default()
        });
        profiles.characters.push(crate::profiles::CharacterProfile {
            filename: "missing-test-sprite".into(),
            profile_name: "Replay Test Hero".into(),
            ..Default::default()
        });

        let mut campaign = crate::campaign::Campaign::default();
        campaign.missions.push(crate::mission::Mission {
            profile_idx: Some(0),
            ..Default::default()
        });
        campaign.current_mission_idx = Some(0);
        campaign.characters.push(crate::campaign::PcDescription {
            character_profile_idx: Some(crate::profiles::CharacterProfileIdx(0)),
            ..Default::default()
        });
        campaign.gang_indices.push(0);
        campaign.mission_team_indices.push(0);

        let mut assets = crate::engine::LevelAssets::new();
        assets.profile_manager = std::sync::Arc::new(profiles);
        let mut level_grid = crate::fast_find_grid::LevelGrid::default();
        level_grid
            .move_box_half_diagonals
            .push(crate::coordinates::MoveBoxHalfDiagonal::new(1.0, 1.0));
        assets.navigation.level_grid = std::sync::Arc::new(level_grid);
        let mut loaded = crate::level_data::LoadedLevel::empty_for_test();
        let mut graph_bytes = Vec::new();
        graph_bytes.extend_from_slice(&1_u16.to_le_bytes());
        graph_bytes.extend_from_slice(&1.0_f32.to_le_bytes());
        graph_bytes.extend_from_slice(&1.0_f32.to_le_bytes());
        graph_bytes.extend_from_slice(&0_u16.to_le_bytes());
        graph_bytes.extend_from_slice(&0_u16.to_le_bytes());
        graph_bytes.extend_from_slice(&0_u16.to_le_bytes());
        loaded.proto.motion_data = Some(crate::level_data::RawMotionData {
            layers: vec![vec![crate::level_data::RawMotionArea {
                is_lift: false,
                state_id: 0,
                polygon: crate::level_data::SectorPolygon {
                    points: vec![(0, 0), (1_000, 0), (1_000, 1_000), (0, 1_000)],
                },
                skeleton_segments: Vec::new(),
                flags: 0,
                obstacles: Vec::new(),
            }]],
            graph_bytes,
        });
        loaded.proto.grid_chunk_order = vec![crate::level_data::ProtoGridChunk::Motion];
        loaded.mission.beam_mes.push(crate::level_data::BeamMe {
            position: crate::coordinates::MapPoint::new(100.0, 200.0),
            direction: 0,
            action: 0,
            projection_area: u16::MAX,
            sector: 0,
            layer: 0,
            material: 0,
            action_required: Default::default(),
            index: 0,
            script: None,
            required_pc: 0,
            profile_override: None,
            robin_role: false,
        });
        (campaign, assets, loaded)
    }

    fn construct_sherwood_frame_zero(
        campaign: crate::campaign::Campaign,
        mut assets: crate::engine::LevelAssets,
        loaded: crate::level_data::LoadedLevel,
        rng_seed: u64,
        sim_config: crate::engine::SimConfig,
    ) -> (crate::engine::Engine, crate::engine::LevelAssets) {
        let engine = crate::engine::Engine::new(crate::engine::EngineArgs {
            campaign,
            level: crate::engine::LevelLoadArgs {
                assets: &mut assets,
                level_directory: "",
                progress: &mut |_| {},
                loaded,
                bg_pixel_dims: (0.0, 0.0),
            },
            ground_mark_sprite: None,
            titbit_row_frame_counts: Vec::new(),
            rng_seed,
            original_rng_replay: None,
            sim_config,
        })
        .unwrap();
        (engine, assets)
    }

    #[test]
    fn sherwood_recording_reconstructs_frame_zero_team_pcs_and_hash() {
        let path = unique_replay_path("sherwood_frame_zero");
        let rng_seed = 0x5EED_5151;
        let sim_config = crate::engine::SimConfig {
            script_enabled: false,
            ..Default::default()
        };
        let (campaign, assets, loaded) = sherwood_fixture();
        let pre_engine_campaign = campaign.clone();
        let (live, _live_assets) =
            construct_sherwood_frame_zero(campaign, assets, loaded, rng_seed, sim_config);

        assert_eq!(live.pc_ids().len(), 1);
        assert!(live.campaign().mission_team_indices.is_empty());
        let live_hash = state_hash(&live);

        let recorder = ReplayRecorder::new(
            &path,
            "Sherwood".into(),
            test_mission_assets("Sherwood"),
            rng_seed,
            sim_config,
            &pre_engine_campaign,
        )
        .unwrap();
        drop(recorder);

        let replay = ReplayData::from_file(&path).unwrap();
        let replay_campaign: crate::campaign::Campaign =
            bitcode::decode(&replay.header.campaign).unwrap();
        assert_eq!(replay_campaign.mission_team_indices, vec![0]);
        let (_, replay_assets, replay_loaded) = sherwood_fixture();
        let (reconstructed, _replay_assets) = construct_sherwood_frame_zero(
            replay_campaign,
            replay_assets,
            replay_loaded,
            replay.header.rng_seed,
            replay.header.sim_config,
        );

        assert_eq!(reconstructed.pc_ids().len(), 1);
        assert!(reconstructed.campaign().mission_team_indices.is_empty());
        assert_eq!(state_hash(&reconstructed), live_hash);
        let _ = std::fs::remove_file(path);
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
            let mut rec = ReplayRecorder::new_with_campaign(
                &path,
                "campaign".into(),
                test_mission_assets("campaign"),
                11,
                crate::engine::SimConfig::default(),
                &campaign,
            )
            .unwrap();
            record_tick(
                &mut rec,
                0,
                SimulationFrameInput::new(vec![PlayerCommand::CrouchDown.into()]),
            );
        }

        let data = ReplayData::from_file(&path).unwrap();
        let bytes = &data.header.campaign;
        let restored: Campaign = bitcode::decode(bytes).unwrap();
        assert_eq!(restored.values[CampaignValue::Score], 12_345);
        assert_eq!(restored.ares, 4);
        assert_eq!(data.frame(0).expect("frame zero").input.commands.len(), 1);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn replay_header_requires_a_non_null_campaign_field() {
        let base = serde_json::json!({
            "mission_id": "Dem_Lei_MP",
            "mission_assets": test_mission_assets("Dem_Lei_MP"),
            "rng_seed": 7,
            "sim_config": crate::engine::SimConfig::default(),
            "spellforge_package": null,
            "version": REPLAY_SCHEMA_VERSION,
            "total_frames": 0,
            "rankability": ReplayRankability::rankable(),
        });
        assert!(serde_json::from_value::<ReplayHeader>(base.clone()).is_err());

        let mut null_campaign = base;
        null_campaign["campaign"] = serde_json::Value::Null;
        assert!(serde_json::from_value::<ReplayHeader>(null_campaign).is_err());
    }

    #[test]
    fn current_schema_requires_explicit_rankability_evidence() {
        let mut header = serde_json::to_value(ReplayHeader {
            mission_id: "Dem_Lei_MP".into(),
            mission_assets: test_mission_assets("Dem_Lei_MP"),
            rng_seed: 7,
            sim_config: crate::engine::SimConfig::default(),
            spellforge_package: None,
            version: REPLAY_SCHEMA_VERSION,
            total_frames: 0,
            rankability: ReplayRankability::rankable(),
            campaign: bitcode::encode(&crate::campaign::Campaign::default()),
        })
        .unwrap();
        header.as_object_mut().unwrap().remove("rankability");
        let error = serde_json::from_value::<ReplayHeader>(header).unwrap_err();
        assert!(error.to_string().contains("missing field `rankability`"));
    }

    #[test]
    fn group_move_goal_override_serde_roundtrip() {
        let with_override = PlayerCommand::GroupMove {
            actors: vec![crate::element::EntityId::Pc(crate::entity_id::PcId(1))],
            destination: crate::coordinates::MapPoint::new(50.0, 75.0),
            running: true,
            show_marker: false,
            goal_override: Some((crate::sector::SectorNumber::new(42), 3)),
            goal_sector_index_override: crate::fast_find_grid::SectorIndex::new(17),
            door_route_override: None,
            recorded_gate_routes: Vec::new(),
            recorded_failed_gate_routes: vec![crate::element::EntityId::Pc(
                crate::entity_id::PcId(1),
            )],
        };
        let json = serde_json::to_string(&with_override).unwrap();
        let round: PlayerCommand = serde_json::from_str(&json).unwrap();
        match round {
            PlayerCommand::GroupMove {
                goal_override: Some((sector, layer)),
                goal_sector_index_override: Some(index),
                recorded_failed_gate_routes,
                ..
            } => {
                assert_eq!(sector, crate::sector::SectorNumber::new(42));
                assert_eq!(layer, 3);
                assert_eq!(index.get(), 17);
                assert_eq!(
                    recorded_failed_gate_routes,
                    vec![crate::element::EntityId::Pc(crate::entity_id::PcId(1))]
                );
            }
            _ => panic!("round-tripped GroupMove lost its goal_override"),
        }
    }

    #[test]
    fn truncated_group_move_commands_are_rejected() {
        let current = PlayerCommand::GroupMove {
            actors: vec![crate::element::EntityId::Pc(crate::entity_id::PcId(1))],
            destination: crate::coordinates::MapPoint::new(50.0, 75.0),
            running: false,
            show_marker: true,
            goal_override: Some((crate::sector::SectorNumber::new(42), 3)),
            goal_sector_index_override: crate::fast_find_grid::SectorIndex::new(17),
            door_route_override: None,
            recorded_gate_routes: Vec::new(),
            recorded_failed_gate_routes: Vec::new(),
        };
        let current = serde_json::to_value(current).unwrap();
        for missing_field in [
            "show_marker",
            "goal_override",
            "goal_sector_index_override",
            "door_route_override",
            "recorded_gate_routes",
            "recorded_failed_gate_routes",
        ] {
            let mut truncated = current.clone();
            truncated
                .get_mut("GroupMove")
                .and_then(serde_json::Value::as_object_mut)
                .expect("externally tagged GroupMove fields")
                .remove(missing_field);

            assert!(
                serde_json::from_value::<PlayerCommand>(truncated).is_err(),
                "GroupMove without {missing_field} must be rejected"
            );
        }
    }

    #[test]
    fn truncated_drop_ale_commands_are_rejected() {
        let current = PlayerCommand::DropAleAt {
            actor: crate::element::EntityId::Pc(crate::entity_id::PcId(1)),
            target_pos: crate::coordinates::MapPoint::new(50.0, 75.0),
            running: false,
            already_authorized: false,
            goal_override: None,
            goal_sector_index_override: None,
            recorded_gate_path: None,
        };
        let current = serde_json::to_value(current).unwrap();
        for missing_field in [
            "already_authorized",
            "goal_override",
            "goal_sector_index_override",
            "recorded_gate_path",
        ] {
            let mut truncated = current.clone();
            truncated
                .get_mut("DropAleAt")
                .and_then(serde_json::Value::as_object_mut)
                .expect("externally tagged DropAle command fields")
                .remove(missing_field);

            assert!(
                serde_json::from_value::<PlayerCommand>(truncated).is_err(),
                "DropAleAt without {missing_field} must be rejected"
            );
        }
    }

    #[test]
    fn resolved_drop_ale_command_json_preserves_recorded_route_provenance() {
        let command = PlayerCommand::DropAleAt {
            actor: crate::element::EntityId::Pc(crate::entity_id::PcId(1)),
            target_pos: crate::coordinates::MapPoint::new(50.0, 75.0),
            running: false,
            already_authorized: true,
            goal_override: Some((crate::sector::SectorNumber::new(0), 0)),
            goal_sector_index_override: crate::fast_find_grid::SectorIndex::new(0),
            recorded_gate_path: Some(crate::gate::RecordedGatePath {
                source_sector: crate::sector::SectorNumber::new(133),
                source_sector_index: crate::fast_find_grid::SectorIndex::new(57),
                source_layer: 11,
                outcome: crate::gate::RecordedGateOutcome::Success(vec![
                    crate::gate::GatePathStep {
                        door_index: crate::gate::DoorIndex::new(7).expect("valid door index"),
                        direct: false,
                    },
                ]),
            }),
        };
        let json = serde_json::to_value(&command).unwrap();
        let decoded: PlayerCommand = serde_json::from_value(json).unwrap();
        let PlayerCommand::DropAleAt {
            recorded_gate_path: Some(decoded_route),
            ..
        } = decoded
        else {
            panic!("resolved DropAle route must survive JSON round-trip");
        };
        let PlayerCommand::DropAleAt {
            recorded_gate_path: Some(expected_route),
            ..
        } = command
        else {
            unreachable!();
        };
        assert_eq!(decoded_route, expected_route);
    }

    #[test]
    fn pristine_restart_boundary_round_trips_without_an_extra_frame() {
        let path = unique_replay_path("pristine_restart");
        let mut recorder = ReplayRecorder::new(
            &path,
            "restart".into(),
            test_mission_assets("restart"),
            17,
            Default::default(),
            &crate::campaign::Campaign::default(),
        )
        .unwrap();
        recorder.write_save_marker(
            0,
            ReplaySaveMarker {
                state_hash: 123,
                timeline_frame: 0,
            },
        );
        recorder.write_load_back(0, 0, false);
        record_tick(&mut recorder, 0, SimulationFrameInput::default());
        drop(recorder);
        let data = ReplayData::from_file(&path).unwrap();
        data.validate_layout().unwrap();
        assert_eq!(data.frame_count(), 1);
        assert_eq!(data.load_back_for_frame(0).cloned().unwrap().to_frame, 0);

        let jsonl = std::fs::read_to_string(&path).unwrap();
        // The JSONL parser and the shared compact-layout validator must both
        // reject every other self/future load and a nonzero bootstrap timeline.
        for (ordinal, target, marker_timeline) in [(1, 1, 0), (0, 1, 0), (0, 0, 1)] {
            let mut invalid = data.clone();
            std::sync::Arc::make_mut(&mut invalid.load_backs).clear();
            std::sync::Arc::make_mut(&mut invalid.load_backs).insert(
                ordinal,
                ReplayLoadBack {
                    snapshot: None,
                    to_frame: target,
                    is_continue: false,
                },
            );
            std::sync::Arc::make_mut(&mut invalid.save_markers)
                .get_mut(&0)
                .unwrap()
                .timeline_frame = marker_timeline;
            assert!(invalid.validate_layout().is_err());
            let edited = jsonl
                .lines()
                .map(|line| {
                    let mut value: serde_json::Value = serde_json::from_str(line).unwrap();
                    if value.get("lb").is_some() {
                        value["f"] = ordinal.into();
                        value["lb"]["to_frame"] = target.into();
                    }
                    if value.get("sv").is_some() {
                        value["sv"]["timeline_frame"] = marker_timeline.into();
                    }
                    serde_json::to_string(&value).unwrap()
                })
                .collect::<Vec<_>>()
                .join("\n");
            assert!(ReplayData::from_reader(std::io::Cursor::new(edited)).is_err());
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn save_markers_and_load_backs_round_trip_linearly() {
        let path = unique_replay_path("save_load_timeline");
        let campaign = crate::campaign::Campaign::default();
        {
            let mut rec = ReplayRecorder::new(
                &path,
                "timeline".into(),
                test_mission_assets("timeline"),
                7,
                crate::engine::SimConfig::default(),
                &campaign,
            )
            .unwrap();
            // Frame 0-9: play.
            for ordinal in 0..10 {
                record_tick(&mut rec, ordinal, SimulationFrameInput::default());
            }
            // Frame 10: quick save captured state with hash 0xABCD.
            rec.write_save_marker(
                10,
                ReplaySaveMarker {
                    state_hash: 0xABCD,
                    timeline_frame: 10,
                },
            );
            for ordinal in 10..30 {
                record_tick(&mut rec, ordinal, SimulationFrameInput::default());
            }
            // Frame 30: quick load jumped back to the frame-10 state.
            rec.write_load_back(30, 10, true);
            assert!(rec.write_frame(
                30,
                10,
                11,
                SimulationFrameInput::new(vec![PlayerCommand::CrouchDown.into()]),
                Vec::new(),
                None,
            ));
        }

        let data = ReplayData::from_file(&path).unwrap();
        assert_eq!(
            data.save_marker_for_frame(10),
            Some(ReplaySaveMarker {
                state_hash: 0xABCD,
                timeline_frame: 10,
            })
        );
        assert_eq!(data.save_marker_for_frame(9), None);
        assert_eq!(
            data.load_back_for_frame(30).cloned(),
            Some(ReplayLoadBack {
                snapshot: None,
                to_frame: 10,
                is_continue: true,
            })
        );
        assert_eq!(data.load_back_for_frame(29).cloned(), None);
        assert_eq!(data.frame(30).expect("frame 30").input.commands.len(), 1);
        assert_eq!(data.frame_count(), 31);

        // The same timeline boundary exists before and after load-back. Seek
        // must stay on the current linear segment instead of jumping across
        // the discontinuity to an older branch.
        let mut player = ReplayPlayer::new(data.clone());
        player.seek_ordinal(ReplayFrameOrdinal::from_wire(30));
        assert_eq!(
            player
                .seek_timeline_frame(TimelineFrame::from_wire(10))
                .unwrap()
                .number(),
            10
        );
        player.seek_ordinal(ReplayFrameOrdinal::from_wire(31));
        assert_eq!(
            player
                .seek_timeline_frame(TimelineFrame::from_wire(10))
                .unwrap()
                .number(),
            30
        );
        player.seek_ordinal(ReplayFrameOrdinal::from_wire(31));
        assert_eq!(
            player
                .seek_timeline_frame(TimelineFrame::from_wire(11))
                .unwrap()
                .number(),
            31
        );

        // Timeline records survive the compact-format struct conversion.
        let file = ReplayFile::from(&data);
        let back: ReplayData = file.try_into().expect("valid replay file");
        assert_eq!(
            back.save_marker_for_frame(10),
            Some(ReplaySaveMarker {
                state_hash: 0xABCD,
                timeline_frame: 10,
            })
        );
        assert_eq!(
            back.load_back_for_frame(30).cloned(),
            Some(ReplayLoadBack {
                snapshot: None,
                to_frame: 10,
                is_continue: true,
            })
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_back_without_matching_save_marker_is_rejected() {
        let header = serde_json::json!({
            "mission_id": "x",
            "mission_assets": test_mission_assets("x"),
            "rng_seed": 0,
            "sim_config": crate::engine::SimConfig::default(),
            "spellforge_package": null,
            "version": REPLAY_SCHEMA_VERSION,
            "total_frames": 0,
            "rankability": ReplayRankability::rankable(),
            "campaign": bitcode::encode(&crate::campaign::Campaign::default()),
        });
        let input =
            format!("{header}\n{{\"f\":30,\"lb\":{{\"to_frame\":10,\"is_continue\":false}}}}\n");
        let error = ReplayData::from_reader(std::io::Cursor::new(input))
            .expect_err("dangling load-back must not load");
        assert!(error.contains("no save marker"), "{error}");

        let forward =
            format!("{header}\n{{\"f\":5,\"lb\":{{\"to_frame\":10,\"is_continue\":false}}}}\n");
        let error = ReplayData::from_reader(std::io::Cursor::new(forward))
            .expect_err("forward load-back must not load");
        assert!(error.contains("not before frame"), "{error}");
    }

    #[test]
    fn recorder_persists_every_first_observed_input_taint() {
        let path = unique_replay_path("ranked-input-taints");
        let campaign = crate::campaign::Campaign::default();
        {
            let mut recorder = ReplayRecorder::new(
                &path,
                "taint-fixture".into(),
                test_mission_assets("taint-fixture"),
                17,
                crate::engine::SimConfig::default(),
                &campaign,
            )
            .unwrap();
            for kind in InputTaintKind::ALL {
                recorder.record_input_taint(kind, 0);
                recorder.record_input_taint(kind, 5);
            }
            record_tick(&mut recorder, 0, SimulationFrameInput::default());
        }
        let replay = ReplayData::from_file(&path).unwrap();
        assert_eq!(
            replay.rankability().unwrap().taints().len(),
            InputTaintKind::ALL.len()
        );
        assert!(
            replay
                .rankability()
                .unwrap()
                .taints()
                .iter()
                .all(|taint| taint.first_frame == 0)
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn deterministic_cheat_rpc_load_and_restart_paths_cannot_hide_taints() {
        let mut input = SimulationFrameInput::no_hourglass().with_post_external_actions(vec![
            crate::engine::ExternalAction::Native {
                name: "SetMoney".into(),
                args: vec![1000],
                this_actor: None,
            },
        ]);
        input
            .commands
            .push(PlayerInput::host(PlayerCommand::SetGoldenEyeMode { on: true }).into());
        input.post_commands.push(
            PlayerInput::host(PlayerCommand::ModalDismiss {
                kind: ModalKind::FinalDebriefing {
                    text_id: crate::player_command::DebriefingTextId::Win { index: 0 },
                },
                result: DialogResult::Restart,
            })
            .into(),
        );
        let replay: ReplayData = ReplayFile {
            header: ReplayHeader {
                mission_id: "taint-fixture".into(),
                mission_assets: test_mission_assets("taint-fixture"),
                rng_seed: 1,
                sim_config: crate::engine::SimConfig::default(),
                spellforge_package: None,
                version: REPLAY_SCHEMA_VERSION,
                total_frames: 1,
                rankability: ReplayRankability::rankable(),
                campaign: bitcode::encode(&crate::campaign::Campaign::default()),
            },
            frames: BTreeMap::from([(
                0,
                ReplayFrame {
                    timeline_before: 0,
                    timeline_after: 1,
                    input,
                    host_controls: vec![ReplayHostControl::ModalDismiss {
                        modal: ModalKind::FinalDebriefing {
                            text_id: crate::player_command::DebriefingTextId::Win { index: 0 },
                        },
                        result: DialogResult::Load { slot: 1 },
                    }],
                },
            )]),
            hashes: BTreeMap::new(),
            save_markers: BTreeMap::new(),
            load_backs: BTreeMap::new(),
        }
        .try_into()
        .expect("valid replay file");
        let kinds = replay
            .rankability()
            .unwrap()
            .taints()
            .iter()
            .map(|taint| taint.kind)
            .collect::<BTreeSet<_>>();
        assert!(kinds.contains(&InputTaintKind::HttpStateMutation));
        assert!(kinds.contains(&InputTaintKind::CheatCommand));
        assert!(kinds.contains(&InputTaintKind::StateLoad));
        assert!(kinds.contains(&InputTaintKind::MissionRestart));
    }

    #[test]
    #[should_panic(expected = "replay frame 1 is absent")]
    fn player_past_end_is_a_contract_violation() {
        let mut frames = BTreeMap::new();
        frames.insert(
            0,
            ReplayFrame {
                timeline_before: 0,
                timeline_after: 1,
                input: SimulationFrameInput::new(vec![PlayerCommand::CrouchDown.into()]),
                host_controls: Vec::new(),
            },
        );
        let data = ReplayData {
            header: ReplayHeader {
                mission_id: "x".into(),
                mission_assets: test_mission_assets("x"),
                rng_seed: 0,
                sim_config: crate::engine::SimConfig::default(),
                spellforge_package: None,
                version: REPLAY_SCHEMA_VERSION,
                total_frames: 1,
                rankability: ReplayRankability::rankable(),
                campaign: bitcode::encode(&crate::campaign::Campaign::default()),
            },
            frames: frames.into(),
            hashes: std::sync::Arc::default(),
            save_markers: std::sync::Arc::default(),
            load_backs: std::sync::Arc::default(),
        };
        let mut player = ReplayPlayer::new(data);
        let _ = player.next_frame();
        assert!(player.is_finished());
        let _ = player.next_frame();
    }
}
