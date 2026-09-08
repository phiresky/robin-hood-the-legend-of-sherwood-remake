//! Replay a plain or zstd-compressed JSONL parity trace produced by the
//! original game.
//!
//! This intentionally accepts the original game's neutral, resolved-command
//! schema rather than the Rust-native replay schema.
//!
//! Usage:
//!   ROBINHOOD_DATA_DIR=datadirs/demo_leicester_linux \
//!     cargo run -p robin_parity --bin original_parity_replay -- \
//!       parity-traces/original-demo-baseline.jsonl

mod native_storage;
mod runner;
use native_storage::*;
mod trace_codec;
use trace_codec::*;
mod comparison;
use comparison::*;
mod projection;
use projection::*;
pub use runner::main;

use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt as _;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{collections::BTreeMap, collections::BTreeSet, collections::VecDeque};

use base64::Engine as _;
// Version 67 native parity traces are authoritative artifacts. Keep their
// codec pinned independently of the game's intentionally evolving formats.
use bitcode_parity as bitcode;
use fs2::FileExt as _;
use robin_engine::coordinates::MapPoint;
use robin_engine::coordinates::WorldPoint3D;
use robin_engine::element::{Command, Entity, EntityId, EntityIdKind};
use robin_engine::engine::{Engine, LegacyGridSectorAsset, LevelAssets};
#[cfg(feature = "client")]
use robin_engine::engine::{HostDisplayState, InputState};
use robin_engine::fast_find_grid::LineIndex;
use robin_engine::game_operation::GameCode;
#[cfg(feature = "client")]
use robin_engine::graphic_config::TextureScaleMode;
use robin_engine::player_command::{GestureQuality, PlayerCommand};
use robin_engine::profiles::Action;
use robin_engine::sector::SectorNumber;
#[cfg(feature = "client")]
use robin_engine::sprite::BBox;
#[cfg(feature = "client")]
use robin_rs::Host;
#[cfg(feature = "client")]
use robin_rs::gfx_types::BlendMode;
#[cfg(feature = "client")]
use robin_rs::level_loading_host::EngineLevelLoadExt;
#[cfg(feature = "client")]
use robin_rs::renderer::{GpuImage, Renderer, rgb565_to_rgb8};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut hex, "{byte:02x}").expect("writing to String cannot fail");
    }
    hex
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(tag = "command", rename_all = "snake_case")]
enum TraceDirectorCompletion {
    CameraGoto,
    ZoomLevel,
}

impl From<TraceDirectorCompletion> for robin_engine::engine::DirectorCompletion {
    fn from(value: TraceDirectorCompletion) -> Self {
        match value {
            TraceDirectorCompletion::CameraGoto => Self::CameraGoto,
            TraceDirectorCompletion::ZoomLevel => Self::ZoomLevel,
        }
    }
}

/// JSON trace header and the embedded header layout of
/// [`BinaryTraceHeaderV68`].
///
/// ON-DISK FORMAT INVARIANT: changing any field, field order, or field type
/// changes bitcode's native trace layout. Such a change must bump
/// `TRACE_NATIVE_VERSION`, freeze the old layout in a version-named struct,
/// and add an explicit decoder branch for it. Never silently edit this type
/// while retaining native trace version 68.
#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
struct TraceHeader {
    #[serde(rename = "type")]
    record_type: String,
    mission: String,
    proto_level: String,
    rng_seed: u64,
    schema: u32,
    session_index: u32,
    start_state: TraceStartState,
    initial_frame: u64,
    simulation_hz: u32,
    synchronous_pathfinding: bool,
    rng_stream: String,
    visibility_queries: String,
    #[serde(default)]
    random_input_seed: Option<u32>,
    sim_config: TraceSimConfig,
    campaign: TraceCampaign,
    motion_grid: TraceMotionGrid,
    /// Current session-boundary state omitted by the original game's save payload.
    /// Early schema-16 interactive recordings predate this additive overlay;
    /// `None` selects the narrowly-scoped legacy reconstruction below while a
    /// present (including empty) list remains authoritative.
    #[serde(default)]
    initial_npc_transients: Option<Vec<TraceInitialNpcTransient>>,
    #[serde(default)]
    initial_save: Option<TraceInitialSave>,
}

/// Header layout written by native trace version 67. Changing
/// `initial_npc_transients` from `Vec<_>` to `Option<Vec<_>>` changes
/// bitcode's struct layout and therefore requires native trace version 68.
#[derive(Debug, bitcode::Encode, bitcode::Decode)]
struct TraceHeaderV67 {
    record_type: String,
    mission: String,
    proto_level: String,
    rng_seed: u64,
    schema: u32,
    session_index: u32,
    start_state: TraceStartState,
    initial_frame: u64,
    simulation_hz: u32,
    synchronous_pathfinding: bool,
    rng_stream: String,
    visibility_queries: String,
    random_input_seed: Option<u32>,
    sim_config: TraceSimConfig,
    campaign: TraceCampaign,
    motion_grid: TraceMotionGrid,
    initial_npc_transients: Vec<TraceInitialNpcTransient>,
    initial_save: Option<TraceInitialSave>,
}

/// Accidental late-v67 layout written between `1a932c148` and the v68 bump.
/// Unlike the original v67 layout, absence and an explicitly empty transient
/// overlay remain distinguishable here.
#[derive(Debug, bitcode::Encode, bitcode::Decode)]
struct TraceHeaderV67Late {
    record_type: String,
    mission: String,
    proto_level: String,
    rng_seed: u64,
    schema: u32,
    session_index: u32,
    start_state: TraceStartState,
    initial_frame: u64,
    simulation_hz: u32,
    synchronous_pathfinding: bool,
    rng_stream: String,
    visibility_queries: String,
    random_input_seed: Option<u32>,
    sim_config: TraceSimConfig,
    campaign: TraceCampaign,
    motion_grid: TraceMotionGrid,
    initial_npc_transients: Option<Vec<TraceInitialNpcTransient>>,
    initial_save: Option<TraceInitialSave>,
}

/// Header layout written by native trace version 66. This is the oldest
/// authoritative native format in the retained corpus and must remain frozen.
#[derive(Debug, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceHeaderV66 {
    record_type: String,
    mission: String,
    proto_level: String,
    rng_seed: u64,
    schema: u32,
    session_index: u32,
    start_state: TraceStartState,
    initial_frame: u64,
    simulation_hz: u32,
    synchronous_pathfinding: bool,
    rng_stream: String,
    visibility_queries: String,
    authoritative_state: Option<String>,
    random_input_seed: Option<u32>,
    sim_config: TraceSimConfig,
    campaign: TraceCampaign,
    motion_grid: TraceMotionGrid,
    initial_npc_transients: Option<Vec<TraceInitialNpcTransient>>,
    initial_save: Option<TraceInitialSave>,
}

impl From<TraceHeaderV66> for TraceHeader {
    fn from(header: TraceHeaderV66) -> Self {
        Self {
            record_type: header.record_type,
            mission: header.mission,
            proto_level: header.proto_level,
            rng_seed: header.rng_seed,
            schema: header.schema,
            session_index: header.session_index,
            start_state: header.start_state,
            initial_frame: header.initial_frame,
            simulation_hz: header.simulation_hz,
            synchronous_pathfinding: header.synchronous_pathfinding,
            rng_stream: header.rng_stream,
            visibility_queries: header.visibility_queries,
            random_input_seed: header.random_input_seed,
            sim_config: header.sim_config,
            campaign: header.campaign,
            motion_grid: header.motion_grid,
            initial_npc_transients: header.initial_npc_transients,
            initial_save: header.initial_save,
        }
    }
}

#[cfg(test)]
impl From<TraceHeader> for TraceHeaderV66 {
    fn from(header: TraceHeader) -> Self {
        Self {
            record_type: header.record_type,
            mission: header.mission,
            proto_level: header.proto_level,
            rng_seed: header.rng_seed,
            schema: header.schema,
            session_index: header.session_index,
            start_state: header.start_state,
            initial_frame: header.initial_frame,
            simulation_hz: header.simulation_hz,
            synchronous_pathfinding: header.synchronous_pathfinding,
            rng_stream: header.rng_stream,
            visibility_queries: header.visibility_queries,
            authoritative_state: None,
            random_input_seed: header.random_input_seed,
            sim_config: header.sim_config,
            campaign: header.campaign,
            motion_grid: header.motion_grid,
            initial_npc_transients: header.initial_npc_transients,
            initial_save: header.initial_save,
        }
    }
}

impl From<TraceHeaderV67> for TraceHeader {
    fn from(header: TraceHeaderV67) -> Self {
        Self {
            record_type: header.record_type,
            mission: header.mission,
            proto_level: header.proto_level,
            rng_seed: header.rng_seed,
            schema: header.schema,
            session_index: header.session_index,
            start_state: header.start_state,
            initial_frame: header.initial_frame,
            simulation_hz: header.simulation_hz,
            synchronous_pathfinding: header.synchronous_pathfinding,
            rng_stream: header.rng_stream,
            visibility_queries: header.visibility_queries,
            random_input_seed: header.random_input_seed,
            sim_config: header.sim_config,
            campaign: header.campaign,
            motion_grid: header.motion_grid,
            // V67 could not distinguish an omitted JSON field from a present
            // empty array. Its replay semantics treated both as legacy state.
            initial_npc_transients: (!header.initial_npc_transients.is_empty())
                .then_some(header.initial_npc_transients),
            initial_save: header.initial_save,
        }
    }
}

impl From<TraceHeaderV67Late> for TraceHeader {
    fn from(header: TraceHeaderV67Late) -> Self {
        Self {
            record_type: header.record_type,
            mission: header.mission,
            proto_level: header.proto_level,
            rng_seed: header.rng_seed,
            schema: header.schema,
            session_index: header.session_index,
            start_state: header.start_state,
            initial_frame: header.initial_frame,
            simulation_hz: header.simulation_hz,
            synchronous_pathfinding: header.synchronous_pathfinding,
            rng_stream: header.rng_stream,
            visibility_queries: header.visibility_queries,
            random_input_seed: header.random_input_seed,
            sim_config: header.sim_config,
            campaign: header.campaign,
            motion_grid: header.motion_grid,
            initial_npc_transients: header.initial_npc_transients,
            initial_save: header.initial_save,
        }
    }
}

#[cfg(test)]
impl From<TraceHeader> for TraceHeaderV67Late {
    fn from(header: TraceHeader) -> Self {
        Self {
            record_type: header.record_type,
            mission: header.mission,
            proto_level: header.proto_level,
            rng_seed: header.rng_seed,
            schema: header.schema,
            session_index: header.session_index,
            start_state: header.start_state,
            initial_frame: header.initial_frame,
            simulation_hz: header.simulation_hz,
            synchronous_pathfinding: header.synchronous_pathfinding,
            rng_stream: header.rng_stream,
            visibility_queries: header.visibility_queries,
            random_input_seed: header.random_input_seed,
            sim_config: header.sim_config,
            campaign: header.campaign,
            motion_grid: header.motion_grid,
            initial_npc_transients: header.initial_npc_transients,
            initial_save: header.initial_save,
        }
    }
}

#[cfg(test)]
impl From<TraceHeader> for TraceHeaderV67 {
    fn from(header: TraceHeader) -> Self {
        Self {
            record_type: header.record_type,
            mission: header.mission,
            proto_level: header.proto_level,
            rng_seed: header.rng_seed,
            schema: header.schema,
            session_index: header.session_index,
            start_state: header.start_state,
            initial_frame: header.initial_frame,
            simulation_hz: header.simulation_hz,
            synchronous_pathfinding: header.synchronous_pathfinding,
            rng_stream: header.rng_stream,
            visibility_queries: header.visibility_queries,
            random_input_seed: header.random_input_seed,
            sim_config: header.sim_config,
            campaign: header.campaign,
            motion_grid: header.motion_grid,
            initial_npc_transients: header.initial_npc_transients.unwrap_or_default(),
            initial_save: header.initial_save,
        }
    }
}

#[derive(
    Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, bitcode::Encode, bitcode::Decode,
)]
#[serde(deny_unknown_fields)]
struct TraceInitialNpcTransient {
    creation_order: u32,
    maximal_visibility: u16,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceInitialSave {
    format: String,
    source_profile: TraceSaveSourceProfile,
    encoding: String,
    byte_length: u64,
    sha256: String,
    slot: String,
    header_version: u32,
    mission_id: u32,
    stream_version: u32,
    data: String,
}

#[derive(
    Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, bitcode::Encode, bitcode::Decode,
)]
#[serde(rename_all = "snake_case")]
enum TraceSaveSourceProfile {
    LinuxI386RhsgV48,
    WindowsI386GshrV48,
}

impl TraceSaveSourceProfile {
    fn expected_magic(self) -> &'static [u8; 4] {
        match self {
            Self::LinuxI386RhsgV48 => b"RHSG",
            Self::WindowsI386GshrV48 => b"GSHR",
        }
    }
}

impl TraceInitialSave {
    fn decode_and_validate(&self, expected_mission_id: u32) -> Result<Vec<u8>, String> {
        if self.format != "rhsg" {
            return Err(format!(
                "unsupported initial_save format {:?}; expected \"rhsg\"",
                self.format
            ));
        }
        if self.encoding != "base64" {
            return Err(format!(
                "unsupported initial_save encoding {:?}; expected \"base64\"",
                self.encoding
            ));
        }
        if self.slot.is_empty() || self.slot.contains(['/', '\\']) {
            return Err(format!(
                "initial_save slot {:?} must be a non-empty basename",
                self.slot
            ));
        }
        if self.header_version != 48 || self.stream_version != 48 {
            return Err(format!(
                "unsupported initial_save RHSG versions header={} stream={}; expected v48/v48",
                self.header_version, self.stream_version
            ));
        }
        if self.mission_id != expected_mission_id {
            return Err(format!(
                "initial_save mission {} does not match campaign mission {}",
                self.mission_id, expected_mission_id
            ));
        }
        if self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err("initial_save sha256 must be 64 lowercase hexadecimal digits".to_owned());
        }

        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&self.data)
            .map_err(|error| format!("decode initial_save base64: {error}"))?;
        let declared_length = usize::try_from(self.byte_length)
            .map_err(|_| format!("initial_save byte_length {} is too large", self.byte_length))?;
        if bytes.len() != declared_length {
            return Err(format!(
                "initial_save byte_length says {} but decoded {} bytes",
                self.byte_length,
                bytes.len()
            ));
        }

        let actual_sha256 = sha256_hex(&bytes);
        if actual_sha256 != self.sha256 {
            return Err(format!(
                "initial_save sha256 mismatch: header={} decoded={actual_sha256}",
                self.sha256
            ));
        }
        if bytes.len() < 16 {
            return Err(format!(
                "initial_save is only {} bytes; RHSG header needs 16",
                bytes.len()
            ));
        }
        let expected_magic = self.source_profile.expected_magic();
        if &bytes[0..4] != expected_magic {
            return Err(format!(
                "initial_save source profile {:?} requires magic {:?}, found {:?}",
                self.source_profile,
                std::str::from_utf8(expected_magic).unwrap(),
                std::str::from_utf8(&bytes[0..4]).unwrap_or("<non-ASCII>")
            ));
        }

        let payload_header_version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let payload_mission_id = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        let payload_stream_version = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
        if payload_header_version != self.header_version
            || payload_mission_id != self.mission_id
            || payload_stream_version != self.stream_version
        {
            return Err(format!(
                "initial_save RHSG header ({payload_header_version}, {payload_mission_id}, \
                 {payload_stream_version}) disagrees with metadata ({}, {}, {})",
                self.header_version, self.mission_id, self.stream_version
            ));
        }

        Ok(bytes)
    }
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceSimConfig {
    difficulty: TraceDifficulty,
    script_enabled: bool,
    highlander: bool,
    highlander2: bool,
    golden_eye: bool,
    ignore_default_loose: bool,
    bypass_fog_sprites_crash: bool,
    amount_of_speaking: u16,
}

impl TraceSimConfig {
    fn to_sim_config(&self, synchronous_pathfinding: bool) -> robin_engine::engine::SimConfig {
        robin_engine::engine::SimConfig {
            difficulty: self.difficulty.into(),
            // Original-parity traces deliberately retain the shipped bug.
            fix_hard_reaction_times: false,
            // Untying is a post-port extension and stays off in Original traces.
            enable_unbinding: false,
            clean_hands_npc_kills_invalidate: false,
            // Reusable cloaks are an opt-in extension and must never alter
            // original-parity traces.
            reusable_cloaks: false,
            // Item rebalances and their extra cue are post-port extensions.
            item_gameplay: robin_engine::gameplay_config::ItemGameplayConfig::classic(),
            noise_distraction_feedback: false,
            // Original traces use the two-camp distinct-ID rules, including
            // Royalist/Lacklandist NPC combat.
            diplomacy: false,
            npc_faction_wars: true,
            // Advanced gesture recognition and quality-scaled damage are
            // post-port extensions. Original parity traces must retain the
            // shipped combat input and damage rules.
            more_combat_gestures: false,
            gesture_quality_damage: false,
            // Shared-vision fog is also a post-port gameplay rule.
            fog_of_war: false,
            script_enabled: self.script_enabled,
            highlander: self.highlander,
            highlander2: self.highlander2,
            golden_eye: self.golden_eye,
            ignore_default_loose: self.ignore_default_loose,
            bypass_fog_sprites_crash: self.bypass_fog_sprites_crash,
            amount_of_speaking: self.amount_of_speaking,
            synchronous_pathfinding,
            sherwood_trading: false,
            // Original traces contain no Rust-authored timer or ambience
            // schedule, so leaving the authoring gates enabled is inert.
            enable_timed_missions: true,
            enable_dynamic_ambience: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(rename_all = "snake_case")]
enum TraceDifficulty {
    Easy,
    Medium,
    Hard,
}

impl From<TraceDifficulty> for robin_engine::player_profile::DifficultyLevel {
    fn from(value: TraceDifficulty) -> Self {
        match value {
            TraceDifficulty::Easy => Self::Easy,
            TraceDifficulty::Medium => Self::Medium,
            TraceDifficulty::Hard => Self::Hard,
        }
    }
}

#[derive(
    Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, bitcode::Encode, bitcode::Decode,
)]
#[serde(rename_all = "snake_case")]
enum TraceStartState {
    MissionStart,
    LoadedSave,
}

const TRACE_SCHEMA_VERSION: u32 = 16;
const LAST_TRACE_SCHEMA_WITHOUT_DRAW_VIEW: u32 = 16;
const OLDEST_SUPPORTED_TRACE_SCHEMA: u32 = 12;

fn validate_trace_schema(schema: u32) {
    assert!(
        trace_schema_is_supported(schema),
        "unsupported parity trace schema {schema}; schemas {OLDEST_SUPPORTED_TRACE_SCHEMA} through {TRACE_SCHEMA_VERSION} are supported"
    );
}

fn trace_schema_is_supported(schema: u32) -> bool {
    (OLDEST_SUPPORTED_TRACE_SCHEMA..=TRACE_SCHEMA_VERSION).contains(&schema)
}

fn validate_trace_header(header: &TraceHeader) {
    validate_trace_schema(header.schema);
    assert_eq!(
        header.record_type, "header",
        "invalid parity header record type"
    );
    assert_eq!(
        header.simulation_hz, 25,
        "parity replay requires the Original's 25 Hz simulation"
    );
    assert_eq!(
        header.rng_stream, "libc_rand_raw_global_draw_order",
        "unsupported parity RNG stream"
    );
    assert_eq!(
        header.visibility_queries, "opaque_is_reachable",
        "unsupported parity visibility-query contract"
    );
}

fn decode_and_validate_initial_save(header: &TraceHeader) -> Option<Vec<u8>> {
    match (header.start_state, header.initial_save.as_ref()) {
        (TraceStartState::MissionStart, None) => None,
        (TraceStartState::MissionStart, Some(_)) => {
            panic!("mission_start traces must not contain initial_save")
        }
        (TraceStartState::LoadedSave, None) => {
            panic!("loaded_save traces require initial_save")
        }
        (TraceStartState::LoadedSave, Some(initial_save)) => {
            let mission_index = header
                .campaign
                .current_mission_index
                .expect("loaded_save campaign has no current mission");
            let mission = header
                .campaign
                .missions
                .get(mission_index)
                .unwrap_or_else(|| {
                    panic!("loaded_save current mission index {mission_index} is out of range")
                });
            Some(
                initial_save
                    .decode_and_validate(mission.profile_id)
                    .unwrap_or_else(|error| panic!("invalid initial_save: {error}")),
            )
        }
    }
}

fn apply_initial_npc_transients(engine: &mut Engine, transients: &[TraceInitialNpcTransient]) {
    let mut runtime_by_creation_order = BTreeMap::new();
    for id in engine.npc_ids() {
        let creation_order = engine.original_creation_order(id);
        assert!(
            runtime_by_creation_order
                .insert(creation_order, id)
                .is_none(),
            "two Rust NPCs share Original creation order {creation_order}"
        );
    }
    assert_eq!(
        transients.len(),
        runtime_by_creation_order.len(),
        "schema-{TRACE_SCHEMA_VERSION} initial_npc_transients must cover every NPC exactly once"
    );

    let mut seen = BTreeSet::new();
    for transient in transients {
        assert!(
            seen.insert(transient.creation_order),
            "schema-{TRACE_SCHEMA_VERSION} initial_npc_transients repeats creation order {}",
            transient.creation_order
        );
        let id = runtime_by_creation_order
            .get(&transient.creation_order)
            .copied()
            .unwrap_or_else(|| {
                panic!(
                    "schema-{TRACE_SCHEMA_VERSION} NPC transient creation order {} is absent from the Rust engine",
                    transient.creation_order
                )
            });
        engine
            .parity_replay_setup()
            .restore_npc_maximal_visibility(id, transient.maximal_visibility);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LegacyBlockedBoxTuple {
    min_x: u32,
    min_y: u32,
    max_x: u32,
    max_y: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LegacyBlockedBoxValidity {
    Unknown,
    Unset,
    Set,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LegacyStoppableMotionOrder {
    id: u32,
    stop_animation: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LegacyBlockedBoxShadow {
    tuple: LegacyBlockedBoxTuple,
    validity: LegacyBlockedBoxValidity,
    last_processed_order_id: u32,
    pending_motion_order_id: Option<u32>,
    stoppable_motion_order: Option<LegacyStoppableMotionOrder>,
    deviated: Option<bool>,
    direct_validity_observed: bool,
}

fn initial_legacy_blocked_box_shadows(
    save: &robin_engine::legacy_save::body::LegacySaveBody,
) -> BTreeMap<u32, LegacyBlockedBoxShadow> {
    save.element_payloads
        .records
        .iter()
        .filter_map(|record| {
            let sprite = record.payload.actor_sprite()?;
            let blocked = sprite.position.blocked_box;
            Some((
                record.header.creation_order,
                LegacyBlockedBoxShadow {
                    tuple: LegacyBlockedBoxTuple {
                        min_x: blocked.top_left.x.to_bits(),
                        min_y: blocked.top_left.y.to_bits(),
                        max_x: blocked.bottom_right.x.to_bits(),
                        max_y: blocked.bottom_right.y.to_bits(),
                    },
                    validity: if blocked.bounds_are_set {
                        LegacyBlockedBoxValidity::Set
                    } else {
                        LegacyBlockedBoxValidity::Unset
                    },
                    last_processed_order_id: sprite.last_processed_order_id,
                    pending_motion_order_id: None,
                    stoppable_motion_order: None,
                    deviated: Some(sprite.position.deviated),
                    direct_validity_observed: false,
                },
            ))
        })
        .collect()
}

fn legacy_blocked_box_tuple(runtime: &serde_json::Value) -> Option<LegacyBlockedBoxTuple> {
    let blocked = runtime.pointer("/position/blocked_box")?;
    if blocked.is_null() {
        return None;
    }
    let bits = |pointer: &str| {
        blocked
            .pointer(pointer)
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
    };
    Some(LegacyBlockedBoxTuple {
        min_x: bits("/min/x/bits")?,
        min_y: bits("/min/y/bits")?,
        max_x: bits("/max/x/bits")?,
        max_y: bits("/max/y/bits")?,
    })
}

/// Old schema-16 runtime capture omitted the bounding-box bounds-set marker
/// and printed its stale coordinate words unconditionally. Track Original's
/// validity bit from transitions which prove it without touching Rust state.
/// Sprite motion changes the processed movement-order id and then
/// clears the blocked box; only blocked-box updates mutate its bounds and
/// make the box valid again.
///
/// TODO(parity-recorder): have new original-game captures emit no value when
/// the blocked area is absent. Exact tuple reuse remains ambiguous
/// unless the captured anti-collision state also proves the unset-to-set
/// update path.
fn canonicalize_legacy_blocked_box(
    runtime: &mut serde_json::Value,
    creation_order: u32,
    reset_by_new_movement_order: bool,
    current_motion_order_id: Option<u32>,
    current_stoppable_motion_order: Option<LegacyStoppableMotionOrder>,
    shadows: &mut BTreeMap<u32, LegacyBlockedBoxShadow>,
) -> bool {
    let blocked_is_null = runtime
        .pointer("/position/blocked_box")
        .is_some_and(serde_json::Value::is_null);
    let tuple = legacy_blocked_box_tuple(runtime);
    let deviated = runtime
        .pointer("/position/deviated")
        .and_then(serde_json::Value::as_bool);
    if tuple.is_none() && !blocked_is_null {
        return false;
    }
    let Some(last_processed_order_id) = runtime
        .pointer("/sprite/last_processed_order_id")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
    else {
        return false;
    };

    let Some(shadow) = shadows.get_mut(&creation_order) else {
        // Runtime-created actors have no saved validity bit. Their first old
        // capture is observationally ambiguous, so retain it until a later
        // transition proves whether the cache is valid.
        if let Some(tuple) = tuple {
            shadows.insert(
                creation_order,
                LegacyBlockedBoxShadow {
                    tuple,
                    validity: LegacyBlockedBoxValidity::Unknown,
                    last_processed_order_id,
                    pending_motion_order_id: current_motion_order_id,
                    stoppable_motion_order: current_stoppable_motion_order,
                    deviated,
                    direct_validity_observed: false,
                },
            );
        }
        return blocked_is_null;
    };

    let tuple_changed = tuple.is_some_and(|tuple| tuple != shadow.tuple);
    let same_tuple_revalidated = tuple.is_some_and(|tuple| {
        tuple == shadow.tuple
            && shadow.validity == LegacyBlockedBoxValidity::Unset
            && legacy_blocked_box_revalidated_by_deviation(runtime, tuple, shadow.deviated)
    });
    let order_changed = last_processed_order_id != shadow.last_processed_order_id;
    if blocked_is_null {
        // New recordings carry the validity bit directly by emitting null.
        shadow.validity = LegacyBlockedBoxValidity::Unset;
        shadow.direct_validity_observed = true;
    } else if shadow.direct_validity_observed {
        shadow.validity = LegacyBlockedBoxValidity::Set;
    } else if reset_by_new_movement_order && order_changed {
        shadow.validity = LegacyBlockedBoxValidity::Unset;
    }
    if let Some(tuple) = tuple {
        // Motion processing resets before any blocked-box update in the same step,
        // so a changed tuple proves that the later update revalidated it.
        if tuple_changed || same_tuple_revalidated {
            shadow.validity = LegacyBlockedBoxValidity::Set;
        }
        shadow.tuple = tuple;
    }
    shadow.last_processed_order_id = last_processed_order_id;
    shadow.pending_motion_order_id = current_motion_order_id;
    shadow.stoppable_motion_order = current_stoppable_motion_order;
    shadow.deviated = deviated;

    if shadow.validity == LegacyBlockedBoxValidity::Unset {
        if let Some(blocked) = runtime.pointer_mut("/position/blocked_box") {
            *blocked = serde_json::Value::Null;
        }
        true
    } else {
        false
    }
}

fn legacy_blocked_box_revalidated_by_deviation(
    runtime: &serde_json::Value,
    tuple: LegacyBlockedBoxTuple,
    prior_deviated: Option<bool>,
) -> bool {
    let Some(position) = runtime.pointer("/position") else {
        return false;
    };
    if prior_deviated != Some(false)
        || position
            .pointer("/deviated")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        || position
            .pointer("/anti_collision_on")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        || position
            .pointer("/blocked_count")
            .and_then(serde_json::Value::as_u64)
            != Some(0)
    {
        return false;
    }
    let bits = |pointer: &str| {
        position
            .pointer(pointer)
            .and_then(serde_json::Value::as_u64)
            .and_then(|bits| u32::try_from(bits).ok())
    };
    let (Some(map_x), Some(map_y), Some(old_x), Some(old_y)) = (
        bits("/map/x/bits"),
        bits("/map/y/bits"),
        bits("/old_map/x/bits"),
        bits("/old_map/y/bits"),
    ) else {
        return false;
    };
    let map_x = f32::from_bits(map_x);
    let map_y = f32::from_bits(map_y);
    if old_x == map_x.to_bits() && old_y == map_y.to_bits() {
        return false;
    }
    let half = 0.49_f32;
    tuple.min_x == (map_x - half).to_bits()
        && tuple.min_y == (map_y - half).to_bits()
        && tuple.max_x == (map_x + half).to_bits()
        && tuple.max_y == (map_y + half).to_bits()
}

/// Whether the captured actor has just entered the original game's matching execution state
/// which performs sprite motion and therefore resets box-blocked state.
///
/// A movement *sequence* may currently be playing an in-place transition
/// through action processing, and a non-movement sequence may use a locomotion
/// transition action. Both helpers return the same movement-start value, so
/// sequence shape plus motion state is not enough to prove a reset.
fn original_motion_executor_order_id(actor: &TraceActor, entity_id: EntityId) -> Option<u32> {
    let sequence = actor.sequence_element.as_ref()?;
    // Several locomotion transition actions are also installed in generic
    // turn sequences. The original game explicitly routes those through action processing
    // when the sequence is not movement.
    sequence.movement.as_ref()?;
    let order = sequence.current_order.as_ref()?.to_json();
    let action = order
        .get("action")
        .and_then(serde_json::Value::as_u64)
        .and_then(|action| u32::try_from(action).ok())
        .and_then(|action| robin_engine::order::OrderType::try_from(action).ok())?;
    if !robin_engine::engine::original_actor_order_uses_motion_executor(entity_id, action) {
        return None;
    }
    order
        .get("id")
        .and_then(serde_json::Value::as_u64)
        .and_then(|order_id| u32::try_from(order_id).ok())
}

fn original_stoppable_current_motion_order(
    actor: &TraceActor,
) -> Option<LegacyStoppableMotionOrder> {
    let sequence = actor.sequence_element.as_ref()?;
    sequence.movement.as_ref()?;
    let order = sequence.current_order.as_ref()?.to_json();
    let action = order
        .get("action")
        .and_then(serde_json::Value::as_u64)
        .and_then(|action| u32::try_from(action).ok())
        .and_then(|action| robin_engine::order::OrderType::try_from(action).ok())?;
    let stop_animation = match action {
        robin_engine::order::OrderType::WalkingUpright => {
            robin_engine::order::OrderType::TransitionWalkingUprightWaitingUpright
        }
        robin_engine::order::OrderType::RunningUpright => {
            robin_engine::order::OrderType::TransitionRunningUprightWaitingUpright
        }
        robin_engine::order::OrderType::WalkingCrouched => {
            robin_engine::order::OrderType::TransitionWalkingCrouchedWaitingCrouched
        }
        _ => return None,
    };
    let id = order
        .get("id")
        .and_then(serde_json::Value::as_u64)
        .and_then(|order_id| u32::try_from(order_id).ok())?;
    Some(LegacyStoppableMotionOrder {
        id,
        stop_animation: stop_animation as u32,
    })
}

fn original_reset_blocked_box_this_frame(
    actor: &TraceActor,
    entity_id: EntityId,
    last_processed_order_id: u32,
    moved_this_frame: bool,
    prior_pending_motion_order_id: Option<u32>,
    prior_stoppable_motion_order: Option<LegacyStoppableMotionOrder>,
    prior_last_processed_order_id: Option<u32>,
) -> bool {
    let Some(sequence) = actor.sequence_element.as_ref() else {
        return false;
    };
    let current_order_id = sequence
        .current_order
        .as_ref()
        .map(TraceJsonValue::to_json)
        .and_then(|order| {
            order
                .get("id")
                .and_then(serde_json::Value::as_u64)
                .and_then(|order_id| u32::try_from(order_id).ok())
        });
    // Stopping movement rewrites the sole live walking/running order in place and
    // assigns a new ID. The rewritten transition may execute and then be replaced
    // before frame recording, leaving its new ID only in last_processed. The
    // preceding capture proves the exact stoppable movement source; the
    // current non-movement order and lack of displacement prove this is the
    // hidden handoff rather than ordinary progress. This is schema-16
    // session-003-0001 frame-1176.
    let hidden_stop_movement_rewrite = prior_stoppable_motion_order
        .zip(prior_last_processed_order_id)
        .is_some_and(|(stoppable_order, prior_last_processed_order_id)| {
            stoppable_order.id == prior_last_processed_order_id
                && actor.animation == stoppable_order.stop_animation
        })
        && sequence.movement.is_none()
        && current_order_id
            .is_some_and(|current_order_id| current_order_id != last_processed_order_id)
        && !moved_this_frame;
    if hidden_stop_movement_rewrite {
        return true;
    }

    let Some(current_motion_order_id) = original_motion_executor_order_id(actor, entity_id) else {
        return false;
    };

    let starts_visible_order =
        actor.motion_state == robin_engine::sprite::MotionState::Start as u32;
    let advanced_to_next_order =
        current_motion_order_id != last_processed_order_id && moved_this_frame;
    let began_previously_observed_order = prior_pending_motion_order_id
        == Some(last_processed_order_id)
        && current_motion_order_id == last_processed_order_id;
    // A distance-producing order may reach its waypoint and advance before
    // frame recording. Its final motion latch is then IN_PROGRESS and the current
    // order is the successor, but `last_processed_order_id` still proves that
    // Motion processing initialized the just-executed predecessor. This is the
    // schema-16 session-003-0006 frame-3769 shape.
    // A new movement order can also remain current after its first update. In
    // that case the preceding capture proves it was pending, and the changed
    // raw `last_processed_order_id` (checked by the shadow) proves it began.
    // This is the schema-16 session-003-0001 frame-600 shape.
    // Motion processing initializes/resets before deciding whether an order's
    // tolerance permits displacement. A preceding capture of the pending
    // exact movement ID, followed by both current and last_processed changing
    // to that ID, proves the reset even for a stationary order. This is
    // schema-16 session-003-0006 frame-4394 (RunningWithSword, tolerance 65).
    starts_visible_order || advanced_to_next_order || began_previously_observed_order
}

fn reconstruct_unrecorded_maximal_visibility(
    leaning_out: bool,
    visibilities: impl IntoIterator<Item = f32>,
) -> u16 {
    let view_speed = if leaning_out {
        robin_engine::ai_vision::LOOK_DOWN_BASE_VIEW_SPEED
    } else {
        robin_engine::ai_vision::BASE_VIEW_SPEED
    };
    visibilities
        .into_iter()
        .map(|visibility| (view_speed as f32 * visibility) as u16)
        .max()
        .unwrap_or(0)
}

/// Reconstruct the process-local maximum omitted by old interactive segments.
///
/// Original clears this value before an ordinary vision pass, but dead and
/// unconscious actors return first and retain the preceding segment's value.
/// Their serialized detectable buckets retain the visibility which supplied
/// that maximum, making this exact reconstruction possible.
fn apply_legacy_segment_visibility_fallback(engine: &mut Engine) -> usize {
    let restorations = engine
        .npc_ids()
        .into_iter()
        .filter_map(|id| {
            let entity = engine
                .get_entity(id)
                .unwrap_or_else(|| panic!("legacy parity fallback lost NPC {id:?}"));
            let npc = entity
                .npc_data()
                .unwrap_or_else(|| panic!("legacy parity fallback found non-NPC {id:?}"));
            let retains_maximum =
                entity.is_dead() || entity.human_data().is_some_and(|human| human.unconscious);
            retains_maximum.then(|| {
                let value = reconstruct_unrecorded_maximal_visibility(
                    npc.view_lean_out,
                    npc.detectable_lists
                        .iter()
                        .flatten()
                        .map(|detectable| detectable.last_visibility),
                );
                (id, value)
            })
        })
        .collect::<Vec<_>>();
    for &(id, value) in &restorations {
        engine
            .parity_replay_setup()
            .restore_npc_maximal_visibility(id, value);
    }
    restorations.len()
}

/// An in-process load starts recording after save adoption and consequently
/// has no setup RNG prefix. A nonempty prefix proves a fresh engine, whose
/// constructor-zero process-local state is authoritative.
fn legacy_loaded_save_retains_process_transients(prefix_draw_count: usize) -> bool {
    prefix_draw_count == 0
}

fn preceding_interactive_session_path(path: &Path, session_index: u32) -> Option<PathBuf> {
    if session_index <= 1 {
        return None;
    }
    let previous = session_index.checked_sub(1)?;
    let name = path.file_name()?.to_str()?;
    let (name, native) = match name.strip_suffix(TRACE_NATIVE_SUFFIX) {
        Some(logical) => (logical, true),
        None => (name, false),
    };
    let suffix = format!("-session-{session_index:04}.jsonl.zst");
    let stem = name.strip_suffix(&suffix)?;
    let previous = format!("{stem}-session-{previous:04}.jsonl.zst");
    Some(path.with_file_name(if native {
        format!("{previous}{TRACE_NATIVE_SUFFIX}")
    } else {
        previous
    }))
}

fn terminal_macro_waypoint(
    element: &TraceElement,
    paths: &[robin_engine::level_data::RawHikingPath],
) -> Option<(robin_engine::ai::PathId, u8, usize)> {
    let ai = element.ai.as_ref()?;
    terminal_macro_waypoint_at(
        (element.position_map.x.bits, element.position_map.y.bits),
        ai.macro_cursor,
        ai.macro_in_progress,
        paths,
    )
}

fn terminal_macro_waypoint_at(
    position_bits: (u32, u32),
    cursor: Option<u16>,
    macro_in_progress: bool,
    paths: &[robin_engine::level_data::RawHikingPath],
) -> Option<(robin_engine::ai::PathId, u8, usize)> {
    if !macro_in_progress {
        return None;
    }
    let offset = usize::from(cursor?);
    let mut matches = paths.iter().enumerate().flat_map(|(path_index, path)| {
        path.waypoints
            .iter()
            .enumerate()
            .filter_map(move |(waypoint_index, waypoint)| {
                let robin_engine::level_data::WaypointCommand::Macro(command) = &waypoint.command
                else {
                    return None;
                };
                (offset <= command.len()
                    && position_bits.0 == f32::from(waypoint.x).to_bits()
                    && position_bits.1 == f32::from(waypoint.y).to_bits())
                .then_some((path_index, waypoint_index))
            })
    });
    let (path_index, waypoint_index) = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some((
        robin_engine::ai::PathId::new(u16::try_from(path_index).ok()?)?,
        u8::try_from(waypoint_index).ok()?,
        offset,
    ))
}

fn apply_legacy_interactive_chain_macro_fallback(
    trace_path: &Path,
    header: &TraceHeader,
    prefix_draw_count: usize,
    engine: &mut Engine,
    assets: &LevelAssets,
) -> usize {
    if header.schema != TRACE_SCHEMA_VERSION
        || header.start_state != TraceStartState::LoadedSave
        || header.initial_npc_transients.is_some()
        || !legacy_loaded_save_retains_process_transients(prefix_draw_count)
    {
        return 0;
    }
    let Some(previous_path) = preceding_interactive_session_path(trace_path, header.session_index)
        .filter(|path| path.is_file() || native_binary_trace_path(path).is_file())
    else {
        return 0;
    };
    let previous_native = ensure_native_binary_trace(&previous_path);
    let mut reader = BinaryTraceReader::open(&previous_native);
    let previous_header = reader.read_header().trace;
    if previous_header.schema != TRACE_SCHEMA_VERSION
        || previous_header.session_index.checked_add(1) != Some(header.session_index)
        || previous_header.mission != header.mission
        || previous_header.proto_level != header.proto_level
        || previous_header.rng_seed != header.rng_seed
    {
        return 0;
    }
    let mut final_frame = None;
    loop {
        match reader.read_record() {
            BinaryTraceRecord::Frame(frame) => final_frame = Some(frame),
            BinaryTraceRecord::End {
                final_frame: end,
                frame_count,
                ..
            } => {
                reader
                    .validate_terminator(frame_count.unwrap(), end.unwrap())
                    .unwrap_or_else(|error| panic!("invalid preceding interactive trace: {error}"));
                break;
            }
        }
    }
    let mut runtime = engine
        .npc_ids()
        .into_iter()
        .map(|id| (engine.original_creation_order(id), id))
        .collect::<BTreeMap<_, _>>();
    let mut restored = 0;
    for element in &final_frame
        .expect("preceding interactive trace has no frames")
        .elements
    {
        let Some((path_id, waypoint, offset)) =
            terminal_macro_waypoint(element, &assets.hiking_paths)
        else {
            continue;
        };
        let Some(id) = runtime.remove(&element.creation_order) else {
            continue;
        };
        restored += usize::from(
            engine
                .parity_replay_setup()
                .restore_npc_dormant_macro_cursor(id, path_id, waypoint, offset, assets),
        );
    }
    restored
}

fn validate_trace_start(start_state: TraceStartState, session_index: u32, initial_frame: u64) {
    match start_state {
        TraceStartState::MissionStart => assert_eq!(
            initial_frame, 0,
            "parity session {session_index} is marked mission_start but begins at frame {initial_frame}"
        ),
        // A loaded automatic mission-start save is reconstructible from the
        // recorded campaign/config/RNG prefix and the ordinary mission
        // loader. Do not reject loaded sessions solely because of their
        // provenance: the normal setup-draw and first-frame isomorphic state
        // comparisons below remain authoritative and fail loudly for a
        // genuinely mid-mission save whose live state is not represented by
        // the header.
        TraceStartState::LoadedSave => {}
    }
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceCampaign {
    version: u32,
    values: Vec<i32>,
    ares: i8,
    missions: Vec<TraceCampaignMission>,
    accessible_mission_indices: Vec<usize>,
    pending_accessible_mission_indices: Vec<usize>,
    last_mission_index: Option<usize>,
    current_mission_index: Option<usize>,
    next_mission_index: Option<usize>,
    blazon_mission_index: Option<usize>,
    last_played_mission_indices: Vec<usize>,
    last_pseudo_mission_status: u32,
    last_pseudo_mission_id: u32,
    characters: Vec<TraceCampaignCharacter>,
    gang_indices: Vec<usize>,
    reservist_indices: Vec<usize>,
    mission_team_indices: Vec<usize>,
    peasant_names: Vec<String>,
    reservists_are_back: bool,
    collected_relics: Vec<u32>,
    production_sectors: Vec<TraceProductionSector>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceCampaignMission {
    profile_index: u32,
    profile_id: u32,
    mission: String,
    proto_level: String,
    age: u16,
    blazon_price: u16,
    status: u32,
    ares_state_succeeded: i8,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceCampaignCharacter {
    profile_index: u32,
    profile_name: String,
    instanced: bool,
    status: TracePcStatus,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TracePcStatus {
    hand_to_hand: TraceSkill,
    bow: TraceSkill,
    life_points: i16,
    in_coma: bool,
    ales: u16,
    arrows: u16,
    apples: u16,
    rations: u16,
    stones: u16,
    wasp_nests: u16,
    nets: u16,
    plants: u16,
    purses: u16,
    name: String,
    beam_me_index_in_sherwood: i16,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceSkill {
    capacity: u32,
    experience: u32,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceProductionSector {
    r#type: u32,
    speed: u16,
    amount: u16,
    produced_amount: u16,
    max_amount_reached: bool,
    occupants: Vec<TraceProductionOccupant>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceProductionOccupant {
    character_index: usize,
    x: TraceFloat,
    y: TraceFloat,
    obstacle: u16,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceMotionGrid {
    layers: Vec<TraceMotionLayer>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceMotionLayer {
    layer: u16,
    lines: Vec<TraceMotionLine>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceMotionLine {
    index: u16,
    a: TracePoint,
    b: TracePoint,
    type_mask: i32,
    associated_sector: i16,
    active: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceMotionLineChange {
    layer: u16,
    index: u16,
    active: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(tag = "phase", rename_all = "snake_case")]
enum TracePathEvent {
    Queued {
        actor: TraceEntityId,
        antagonist: Option<TraceEntityId>,
        layer: u16,
        area: u16,
        source: TracePoint,
        goal: TracePoint,
        half_diagonal_index: u16,
        half_diagonal: TracePoint,
        animation: u32,
        reverse: bool,
        speed: u8,
        tolerance: TraceFloat,
        use_first_point: bool,
    },
    Completed {
        actor: TraceEntityId,
        antagonist: Option<TraceEntityId>,
        layer: u16,
        area: u16,
        source: TracePoint,
        goal: TracePoint,
        half_diagonal_index: u16,
        half_diagonal: TracePoint,
        animation: u32,
        reverse: bool,
        speed: u8,
        tolerance: TraceFloat,
        use_first_point: bool,
        valid: bool,
        waypoints: Vec<TracePoint>,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
struct TraceRngBatch {
    first_index: usize,
    values: Vec<u32>,
    callsite_offsets: Vec<u32>,
    main_thread: Vec<bool>,
    domains: Vec<TraceRngDomain>,
}

impl TraceRngBatch {
    fn validate(&self) {
        let draw_count = self.values.len();
        assert_eq!(
            draw_count,
            self.callsite_offsets.len(),
            "RNG callsite stream has a different length than its values"
        );
        assert_eq!(
            draw_count,
            self.main_thread.len(),
            "RNG thread-origin stream has a different length than its values"
        );
        assert_eq!(
            draw_count,
            self.domains.len(),
            "RNG domain stream has a different length than its values"
        );
        for (index, (domain, main_thread)) in self
            .domains
            .iter()
            .copied()
            .zip(self.main_thread.iter().copied())
            .enumerate()
        {
            assert!(
                domain != TraceRngDomain::Simulation || main_thread,
                "simulation RNG draw {} (global index {}) occurred off the main thread; its global order is not deterministically replayable",
                index,
                self.first_index + index,
            );
        }
    }

    fn gameplay_draw_count(&self) -> usize {
        self.validate();
        self.domains
            .iter()
            .filter(|domain| **domain == TraceRngDomain::Simulation)
            .count()
    }

    fn gameplay_callsite_offsets(&self) -> Vec<u32> {
        self.validate();
        self.callsite_offsets
            .iter()
            .copied()
            .zip(self.domains.iter().copied())
            .filter_map(|(offset, domain)| (domain == TraceRngDomain::Simulation).then_some(offset))
            .collect()
    }

    fn gameplay_values(&self) -> Vec<u32> {
        self.validate();
        self.values
            .iter()
            .copied()
            .zip(self.domains.iter().copied())
            .filter_map(|(value, domain)| (domain == TraceRngDomain::Simulation).then_some(value))
            .collect()
    }
}

#[derive(
    Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, bitcode::Encode, bitcode::Decode,
)]
#[serde(rename_all = "snake_case")]
enum TraceRngDomain {
    Simulation,
    Audio,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
struct TraceRngPrefix {
    #[allow(dead_code)]
    r#type: String,
    draws: TraceRngBatch,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TraceRngOnly {
    #[serde(rename = "type")]
    record_type: String,
    draws: TraceRngBatch,
    final_frame: u64,
    frame_count: u64,
}

#[derive(Debug, Deserialize)]
struct TraceRecordMarker {
    #[serde(rename = "type")]
    record_type: Option<String>,
}

#[derive(
    Debug,
    Clone,
    Copy,
    Deserialize,
    Serialize,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    bitcode::Encode,
    bitcode::Decode,
)]
struct TraceEntityId {
    kind: TraceEntityKind,
    index: u32,
}

#[derive(
    Debug,
    Clone,
    Copy,
    Deserialize,
    Serialize,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(rename_all = "snake_case")]
enum TraceEntityKind {
    Pc,
    Soldier,
    Civilian,
    Fx,
    Target,
    Bonus,
    Scroll,
    Projectile,
    Net,
}

impl From<TraceEntityId> for EntityId {
    fn from(value: TraceEntityId) -> Self {
        let kind = match value.kind {
            TraceEntityKind::Pc => EntityIdKind::Pc,
            TraceEntityKind::Soldier => EntityIdKind::Soldier,
            TraceEntityKind::Civilian => EntityIdKind::Civilian,
            TraceEntityKind::Fx => EntityIdKind::Fx,
            TraceEntityKind::Target => EntityIdKind::Target,
            TraceEntityKind::Bonus => EntityIdKind::Bonus,
            TraceEntityKind::Scroll => EntityIdKind::Scroll,
            TraceEntityKind::Projectile => EntityIdKind::Projectile,
            TraceEntityKind::Net => EntityIdKind::Net,
        };
        EntityId::new(value.index, kind)
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceFloat {
    bits: u32,
}

impl TraceFloat {
    fn value(self) -> f32 {
        f32::from_bits(self.bits)
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TracePoint {
    x: TraceFloat,
    y: TraceFloat,
}

impl From<TracePoint> for MapPoint {
    fn from(value: TracePoint) -> Self {
        Self::new(value.x.value(), value.y.value())
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TracePoint3 {
    x: TraceFloat,
    y: TraceFloat,
    z: TraceFloat,
}

impl From<TracePoint3> for WorldPoint3D {
    fn from(value: TracePoint3) -> Self {
        Self::new(value.x.value(), value.y.value(), value.z.value())
    }
}

#[derive(Debug, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TraceCommand {
    BoxSelect {
        first: TracePoint,
        second: TracePoint,
        append: bool,
    },
    GroupMove {
        actors: Vec<TraceEntityId>,
        destination: TracePoint,
        running: bool,
        show_marker: bool,
        goal_sector: i16,
        goal_layer: u16,
    },
    LaunchInteraction {
        actor: TraceEntityId,
        target: TraceEntityId,
        /// Original's raw numeric command; `original_command_name` is its
        /// stable name and drives replay. Retained for lossless caching.
        original_command: u32,
        original_command_name: String,
        running: bool,
    },
    LaunchSelfAbility {
        actor: TraceEntityId,
        original_command: u32,
        original_command_name: String,
    },
    LaunchGroundTarget {
        actor: TraceEntityId,
        target: TracePoint3,
        original_command: u32,
        original_command_name: String,
        original_target_field: u32,
        titbit_layer: u16,
    },
    LaunchScrollRead {
        actor: TraceEntityId,
        target: TraceEntityId,
        running: bool,
    },
    SwordStrike {
        actor: TraceEntityId,
        target: TraceEntityId,
        original_command: u32,
        original_command_name: String,
        with_seek: bool,
        #[serde(default = "missing_legacy_seek_distance")]
        seek_distance: f32,
    },
    SelectPc {
        pc: TraceEntityId,
        append: bool,
    },
    UnselectAllPcs,
    StopPc {
        pc: TraceEntityId,
    },
    SelectAction {
        pc: TraceEntityId,
        action: TraceAction,
        /// Original's raw numeric action, retained so the cache round trip is
        /// lossless. `action` is its resolved name; replay uses the name.
        original_action: u32,
    },
    CancelAction {
        #[serde(default)]
        pc: Option<TraceEntityId>,
        /// Always `no_action`: selecting no action is
        /// recorded as cancel_action, but the recorder still emits the pair.
        action: TraceAction,
        original_action: u32,
    },
    OrientActionAt {
        action: TraceAction,
        original_action: u32,
        actor: TraceEntityId,
        mouse_map: TracePoint,
        target: TracePoint3,
    },
    MakePcFast {
        entity: TraceEntityId,
    },
    CrouchDown,
    StandUp,
    // NOTE: with the bitcode-encoded native format, ANY change to this enum
    // (adding, removing, or editing a variant, anywhere) changes the on-disk
    // shape. Bump TRACE_NATIVE_VERSION and migrate existing native traces —
    // converted recordings may no longer have a JSONL source to rebuild from.
    DropAleAt {
        actor: TraceEntityId,
        target: TracePoint,
        running: bool,
    },
    ShieldSelectProtected {
        actor: TraceEntityId,
        protected_pc: TraceEntityId,
    },
    BoxUnselect {
        first: TracePoint,
        second: TracePoint,
        append: bool,
    },
    RaiseShieldWithDanger {
        actor: TraceEntityId,
        protected_pc: TraceEntityId,
        danger_point: TracePoint3,
        danger_point_layer: u16,
    },
    TeleportSelected {
        destination: TracePoint,
        goal_sector: i16,
        goal_layer: u16,
    },
    SelectAllPcs,
    UnselectPc {
        pc: TraceEntityId,
    },
    SelectActionIndex {
        index: u32,
    },
    SetLockAlt {
        on: bool,
    },
    KeyControl,
    KeyReleaseControl,
    StartMacro {
        #[serde(default)]
        pc: Option<TraceEntityId>,
        slot: u8,
    },
    DeleteMacro {
        #[serde(default)]
        pc: Option<TraceEntityId>,
        slot: u8,
    },
    StartRecordingMacro {
        #[serde(default)]
        pc: Option<TraceEntityId>,
        slot: u8,
    },
    ChangeQaMemory {
        slot: u8,
    },
    /// A click the Original refused after it had already barked at the
    /// player. The click itself is a raw mouse message and is never
    /// recorded, so without this the bark's speech resolution arrives with
    /// nothing that caused it.
    HeroRefusedAction {
        actor: TraceEntityId,
        action: TraceAction,
        original_action: u32,
        #[serde(default)]
        target: Option<TraceEntityId>,
        reason: String,
    },
    BeggarDontTalkStamp {
        entity: TraceEntityId,
    },
}

/// Command layout embedded in native trace version 66. In particular,
/// `SwordStrike::seek_distance` was optional on disk.
#[derive(Debug, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TraceCommandV66 {
    BoxSelect {
        first: TracePoint,
        second: TracePoint,
        append: bool,
    },
    GroupMove {
        actors: Vec<TraceEntityId>,
        destination: TracePoint,
        running: bool,
        show_marker: bool,
        goal_sector: i16,
        goal_layer: u16,
    },
    LaunchInteraction {
        actor: TraceEntityId,
        target: TraceEntityId,
        original_command: u32,
        original_command_name: String,
        running: bool,
    },
    LaunchSelfAbility {
        actor: TraceEntityId,
        original_command: u32,
        original_command_name: String,
    },
    LaunchGroundTarget {
        actor: TraceEntityId,
        target: TracePoint3,
        original_command: u32,
        original_command_name: String,
        original_target_field: u32,
        titbit_layer: u16,
    },
    LaunchScrollRead {
        actor: TraceEntityId,
        target: TraceEntityId,
        running: bool,
    },
    SwordStrike {
        actor: TraceEntityId,
        target: TraceEntityId,
        original_command: u32,
        original_command_name: String,
        with_seek: bool,
        seek_distance: Option<f32>,
    },
    SelectPc {
        pc: TraceEntityId,
        append: bool,
    },
    UnselectAllPcs,
    StopPc {
        pc: TraceEntityId,
    },
    SelectAction {
        pc: TraceEntityId,
        action: TraceAction,
        original_action: u32,
    },
    CancelAction {
        pc: Option<TraceEntityId>,
        action: TraceAction,
        original_action: u32,
    },
    OrientActionAt {
        action: TraceAction,
        original_action: u32,
        actor: TraceEntityId,
        mouse_map: TracePoint,
        target: TracePoint3,
    },
    MakePcFast {
        entity: TraceEntityId,
    },
    CrouchDown,
    StandUp,
    DropAleAt {
        actor: TraceEntityId,
        target: TracePoint,
        running: bool,
    },
    ShieldSelectProtected {
        actor: TraceEntityId,
        protected_pc: TraceEntityId,
    },
    BoxUnselect {
        first: TracePoint,
        second: TracePoint,
        append: bool,
    },
    RaiseShieldWithDanger {
        actor: TraceEntityId,
        protected_pc: TraceEntityId,
        danger_point: TracePoint3,
        danger_point_layer: u16,
    },
    TeleportSelected {
        destination: TracePoint,
        goal_sector: i16,
        goal_layer: u16,
    },
    SelectAllPcs,
    UnselectPc {
        pc: TraceEntityId,
    },
    SelectActionIndex {
        index: u32,
    },
    SetLockAlt {
        on: bool,
    },
    KeyControl,
    KeyReleaseControl,
    StartMacro {
        pc: Option<TraceEntityId>,
        slot: u8,
    },
    DeleteMacro {
        pc: Option<TraceEntityId>,
        slot: u8,
    },
    StartRecordingMacro {
        pc: Option<TraceEntityId>,
        slot: u8,
    },
    ChangeQaMemory {
        slot: u8,
    },
    HeroRefusedAction {
        actor: TraceEntityId,
        action: TraceAction,
        original_action: u32,
        target: Option<TraceEntityId>,
        reason: String,
    },
    BeggarDontTalkStamp {
        entity: TraceEntityId,
    },
}

impl TraceCommandV66 {
    fn into_current(self) -> TraceCommand {
        match self {
            Self::BoxSelect {
                first,
                second,
                append,
            } => TraceCommand::BoxSelect {
                first,
                second,
                append,
            },
            Self::GroupMove {
                actors,
                destination,
                running,
                show_marker,
                goal_sector,
                goal_layer,
            } => TraceCommand::GroupMove {
                actors,
                destination,
                running,
                show_marker,
                goal_sector,
                goal_layer,
            },
            Self::LaunchInteraction {
                actor,
                target,
                original_command,
                original_command_name,
                running,
            } => TraceCommand::LaunchInteraction {
                actor,
                target,
                original_command,
                original_command_name,
                running,
            },
            Self::LaunchSelfAbility {
                actor,
                original_command,
                original_command_name,
            } => TraceCommand::LaunchSelfAbility {
                actor,
                original_command,
                original_command_name,
            },
            Self::LaunchGroundTarget {
                actor,
                target,
                original_command,
                original_command_name,
                original_target_field,
                titbit_layer,
            } => TraceCommand::LaunchGroundTarget {
                actor,
                target,
                original_command,
                original_command_name,
                original_target_field,
                titbit_layer,
            },
            Self::LaunchScrollRead {
                actor,
                target,
                running,
            } => TraceCommand::LaunchScrollRead {
                actor,
                target,
                running,
            },
            Self::SwordStrike {
                actor,
                target,
                original_command,
                original_command_name,
                with_seek,
                seek_distance,
            } => TraceCommand::SwordStrike {
                actor,
                target,
                original_command,
                original_command_name,
                with_seek,
                seek_distance: seek_distance.unwrap_or_else(missing_legacy_seek_distance),
            },
            Self::SelectPc { pc, append } => TraceCommand::SelectPc { pc, append },
            Self::UnselectAllPcs => TraceCommand::UnselectAllPcs,
            Self::StopPc { pc } => TraceCommand::StopPc { pc },
            Self::SelectAction {
                pc,
                action,
                original_action,
            } => TraceCommand::SelectAction {
                pc,
                action,
                original_action,
            },
            Self::CancelAction {
                pc,
                action,
                original_action,
            } => TraceCommand::CancelAction {
                pc,
                action,
                original_action,
            },
            Self::OrientActionAt {
                action,
                original_action,
                actor,
                mouse_map,
                target,
            } => TraceCommand::OrientActionAt {
                action,
                original_action,
                actor,
                mouse_map,
                target,
            },
            Self::MakePcFast { entity } => TraceCommand::MakePcFast { entity },
            Self::CrouchDown => TraceCommand::CrouchDown,
            Self::StandUp => TraceCommand::StandUp,
            Self::DropAleAt {
                actor,
                target,
                running,
            } => TraceCommand::DropAleAt {
                actor,
                target,
                running,
            },
            Self::ShieldSelectProtected {
                actor,
                protected_pc,
            } => TraceCommand::ShieldSelectProtected {
                actor,
                protected_pc,
            },
            Self::BoxUnselect {
                first,
                second,
                append,
            } => TraceCommand::BoxUnselect {
                first,
                second,
                append,
            },
            Self::RaiseShieldWithDanger {
                actor,
                protected_pc,
                danger_point,
                danger_point_layer,
            } => TraceCommand::RaiseShieldWithDanger {
                actor,
                protected_pc,
                danger_point,
                danger_point_layer,
            },
            Self::TeleportSelected {
                destination,
                goal_sector,
                goal_layer,
            } => TraceCommand::TeleportSelected {
                destination,
                goal_sector,
                goal_layer,
            },
            Self::SelectAllPcs => TraceCommand::SelectAllPcs,
            Self::UnselectPc { pc } => TraceCommand::UnselectPc { pc },
            Self::SelectActionIndex { index } => TraceCommand::SelectActionIndex { index },
            Self::SetLockAlt { on } => TraceCommand::SetLockAlt { on },
            Self::KeyControl => TraceCommand::KeyControl,
            Self::KeyReleaseControl => TraceCommand::KeyReleaseControl,
            Self::StartMacro { pc, slot } => TraceCommand::StartMacro { pc, slot },
            Self::DeleteMacro { pc, slot } => TraceCommand::DeleteMacro { pc, slot },
            Self::StartRecordingMacro { pc, slot } => {
                TraceCommand::StartRecordingMacro { pc, slot }
            }
            Self::ChangeQaMemory { slot } => TraceCommand::ChangeQaMemory { slot },
            Self::HeroRefusedAction {
                actor,
                action,
                original_action,
                target,
                reason,
            } => TraceCommand::HeroRefusedAction {
                actor,
                action,
                original_action,
                target,
                reason,
            },
            Self::BeggarDontTalkStamp { entity } => TraceCommand::BeggarDontTalkStamp { entity },
        }
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, bitcode::Encode, bitcode::Decode,
)]
#[serde(rename_all = "snake_case")]
enum TraceAction {
    NoAction,
    Bow,
    Hit,
    HitHard,
    Purse,
    Stone,
    Shield,
    BigShield,
    Strangle,
    Lever,
    HelpToClimb,
    Apple,
    Ale,
    Eat,
    Guzzle,
    Listen,
    Heal,
    Net,
    Beggar,
    WaspNest,
    Whistle,
    Climb,
    Jump,
    Search,
    Resuscitate,
    LittleJohnCarry,
    FarmerCarry,
    Tie,
    Lockpick,
    Execute,
    Test,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RefreshOrientationSignature {
    actor: TraceEntityId,
    action: TraceAction,
    mouse_map_bits: [u32; 2],
    target_bits: [u32; 3],
}

impl RefreshOrientationSignature {
    fn from_command(command: &TraceCommand) -> Option<Self> {
        let TraceCommand::OrientActionAt {
            actor,
            action,
            mouse_map,
            target,
            ..
        } = command
        else {
            return None;
        };
        Some(Self {
            actor: *actor,
            action: *action,
            mouse_map_bits: [mouse_map.x.bits, mouse_map.y.bits],
            target_bits: [target.x.bits, target.y.bits, target.z.bits],
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum LegacyRefreshOrientationProvenance {
    #[default]
    None,
    ActionSelected {
        actor: TraceEntityId,
        action: TraceAction,
    },
    FirstOrdinaryOrientation(RefreshOrientationSignature),
}

impl LegacyRefreshOrientationProvenance {
    fn advance(
        self,
        commands: &[TraceCommand],
        popup_nested_refresh: bool,
    ) -> LegacyRefreshOrientationProvenance {
        if popup_nested_refresh {
            return Self::None;
        }

        if let Self::ActionSelected { actor, action } = self
            && let [command] = commands
            && let Some(signature) = RefreshOrientationSignature::from_command(command)
            && (signature.actor, signature.action) == (actor, action)
        {
            return Self::FirstOrdinaryOrientation(signature);
        }

        match commands {
            [TraceCommand::SelectAction { pc, action, .. }] => Self::ActionSelected {
                actor: *pc,
                action: *action,
            },
            [
                TraceCommand::SelectPc { pc: selected, .. },
                TraceCommand::SelectAction { pc, action, .. },
            ] if selected == pc => Self::ActionSelected {
                actor: *pc,
                action: *action,
            },
            _ => Self::None,
        }
    }

    fn proves_single_popup_orientation_is_late(
        self,
        commands: &[TraceCommand],
        popup_nested_refresh: bool,
    ) -> bool {
        let Self::FirstOrdinaryOrientation(previous) = self else {
            return false;
        };
        popup_nested_refresh
            && matches!(commands, [command] if RefreshOrientationSignature::from_command(command) == Some(previous))
    }
}

/// Split refresh-owned orientation records from commands that entered through
/// the input phase of this simulation boundary.
///
/// The original game processes input before its simulation update, whereas
/// Orientation processing is called from the element-refresh pass
/// during the refresh pass. Ordinarily a refresh
/// orientation left over from the preceding host pass is already at the front
/// of the next frame's command queue. When an orientation follows this
/// boundary's matching `MSG_SELECT_ACTION`, it came from a refresh reached
/// later in the same host pass and therefore must not affect the actor's
/// earlier execution/action-processing step.
fn split_refresh_owned_orientations(
    commands: Vec<TraceCommand>,
    popup_nested_refresh: bool,
    ordinary_refresh_eligible: &[(TraceEntityId, TraceAction)],
    force_single_popup_orientation_late: bool,
) -> (Vec<TraceCommand>, Vec<TraceCommand>) {
    let mut actions_selected_this_boundary = BTreeMap::new();
    let mut before_hourglass = Vec::with_capacity(commands.len());
    let mut after_hourglass = Vec::new();

    // A frame can contain both the ordinary refresh orientation left by the
    // preceding host pass and a synchronous popup refresh reached during this
    // Hourglass. Original records both into the same flat command stream. The
    // popup refresh is later, so for each actor/action pair only its final
    // resolved orientation belongs after Hourglass.
    let mut final_popup_orientations = Vec::new();
    if popup_nested_refresh {
        for (index, command) in commands.iter().enumerate() {
            if let TraceCommand::OrientActionAt { actor, action, .. } = command {
                if let Some(entry) =
                    final_popup_orientations
                        .iter_mut()
                        .find(|(known_actor, known_action, _, _)| {
                            known_actor == actor && known_action == action
                        })
                {
                    entry.2 = index;
                    entry.3 += 1;
                } else {
                    final_popup_orientations.push((*actor, *action, index, 1_u32));
                }
            }
        }
    }

    for (index, command) in commands.into_iter().enumerate() {
        match &command {
            TraceCommand::SelectAction { pc, action, .. } => {
                actions_selected_this_boundary.insert(*pc, *action);
            }
            TraceCommand::CancelAction { pc: Some(pc), .. } => {
                actions_selected_this_boundary.remove(pc);
            }
            TraceCommand::CancelAction { pc: None, .. } => {
                actions_selected_this_boundary.clear();
            }
            TraceCommand::OrientActionAt { actor, action, .. }
                if final_popup_orientations.iter().any(
                    |(known_actor, known_action, known_index, count)| {
                        known_actor == actor
                            && known_action == action
                            && *known_index == index
                            && (*count > 1
                                || force_single_popup_orientation_late
                                || !ordinary_refresh_eligible.contains(&(*actor, *action)))
                    },
                ) =>
            {
                after_hourglass.push(command);
                continue;
            }
            TraceCommand::OrientActionAt { actor, action, .. }
                if actions_selected_this_boundary.get(actor) == Some(action) =>
            {
                after_hourglass.push(command);
                continue;
            }
            _ => {}
        }
        before_hourglass.push(command);
    }

    (before_hourglass, after_hourglass)
}

fn advance_trace_qa_recording_state(recording: &mut bool, command: &TraceCommand) {
    if matches!(command, TraceCommand::StartRecordingMacro { .. }) {
        *recording = true;
        return;
    }
    if *recording
        && matches!(
            command,
            TraceCommand::GroupMove { .. }
                | TraceCommand::LaunchInteraction { .. }
                | TraceCommand::LaunchGroundTarget { .. }
                | TraceCommand::DropAleAt { .. }
                | TraceCommand::LaunchSelfAbility { .. }
                | TraceCommand::LaunchScrollRead { .. }
                | TraceCommand::SwordStrike { .. }
                | TraceCommand::CrouchDown
                | TraceCommand::StandUp
        )
    {
        // These are the command shapes stored by the engine's QA hook. Their
        // successful original-game handlers stop macro recording globally.
        *recording = false;
    }
}

impl From<TraceAction> for Action {
    fn from(value: TraceAction) -> Self {
        match value {
            TraceAction::NoAction => Self::NoAction,
            TraceAction::Bow => Self::Bow,
            TraceAction::Hit => Self::Hit,
            TraceAction::HitHard => Self::HitHard,
            TraceAction::Purse => Self::Purse,
            TraceAction::Stone => Self::Stone,
            TraceAction::Shield => Self::Shield,
            TraceAction::BigShield => Self::BigShield,
            TraceAction::Strangle => Self::Strangle,
            TraceAction::Lever => Self::Lever,
            TraceAction::HelpToClimb => Self::HelpToClimb,
            TraceAction::Apple => Self::Apple,
            TraceAction::Ale => Self::Ale,
            TraceAction::Eat => Self::Eat,
            TraceAction::Guzzle => Self::Guzzle,
            TraceAction::Listen => Self::Listen,
            TraceAction::Heal => Self::Heal,
            TraceAction::Net => Self::Net,
            TraceAction::Beggar => Self::Beggar,
            TraceAction::WaspNest => Self::WaspNest,
            TraceAction::Whistle => Self::Whistle,
            TraceAction::Climb => Self::Climb,
            TraceAction::Jump => Self::Jump,
            TraceAction::Search => Self::Search,
            TraceAction::Resuscitate => Self::Resuscitate,
            TraceAction::LittleJohnCarry => Self::LittleJohnCarry,
            TraceAction::FarmerCarry => Self::FarmerCarry,
            TraceAction::Tie => Self::Tie,
            TraceAction::Lockpick => Self::Lockpick,
            TraceAction::Execute => Self::Execute,
            TraceAction::Test => Self::Test,
        }
    }
}

impl TraceCommand {
    fn into_player_command(
        self,
        entity_map: &EntityMap,
        engine: &Engine,
        drop_ale_resolution: Option<ReplayDropAleResolution>,
        group_move_resolution: Option<ReplayGroupMoveResolution>,
    ) -> Option<PlayerCommand> {
        assert!(
            drop_ale_resolution.is_none() || matches!(&self, Self::DropAleAt { .. }),
            "DropAle route metadata was attached to a non-DropAle command"
        );
        assert!(
            group_move_resolution.is_none() || matches!(&self, Self::GroupMove { .. }),
            "group-move route metadata was attached to a non-group-move command"
        );
        Some(match self {
            Self::BoxSelect {
                first,
                second,
                append,
            } => {
                // The Original records the root drag gesture and then records
                // each resolved nested selection message as another command.
                // Replaying both would apply selection (and its speech/echo
                // side effects) twice. Keep accepting the gesture metadata,
                // but replay only the following resolved commands.
                let _ = (first, second, append);
                return None;
            }
            Self::GroupMove {
                actors,
                destination,
                running,
                show_marker,
                goal_sector,
                goal_layer,
            } => {
                let destination: MapPoint = destination.into();
                let (goal_override, goal_sector_index_override) = match entity_map
                    .translate_group_move_goal_sector(
                        goal_sector,
                        goal_layer,
                        group_move_resolution
                            .as_ref()
                            .and_then(|resolution| resolution.unmapped_goal_search_sector),
                    ) {
                    GroupMoveGoalTranslation::Runtime(goal, index) => {
                        engine
                            .fast_grid()
                            .level
                            .sector_number_map
                            .get(&goal.0)
                            .and_then(|&index| engine.fast_grid().level.sectors.get(index))
                            .unwrap_or_else(|| {
                                panic!(
                                    "group-move Original sector {goal_sector} maps to missing Rust \
                                     position sector {}",
                                    goal.0
                                )
                            });
                        (Some(goal), Some(index))
                    }
                    GroupMoveGoalTranslation::RecordedUnmapped(goal) => {
                        let runtime_collision = engine
                            .fast_grid()
                            .level
                            .sector_number_map
                            .get(&goal.0)
                            .and_then(|&index| engine.fast_grid().level.sectors.get(index));
                        assert!(
                            runtime_collision.is_none_or(|sector| sector.sector_type.is_jump()),
                            "unmapped Original group-move sector {goal_sector} collides with Rust \
                             runtime position sector {}",
                            goal.0
                        );
                        (Some(goal), None)
                    }
                };
                PlayerCommand::GroupMove {
                    actors: actors
                        .into_iter()
                        .map(|id| entity_map.translate(id))
                        .collect(),
                    destination,
                    running,
                    show_marker,
                    goal_override,
                    goal_sector_index_override,
                    door_route_override: group_move_resolution
                        .as_ref()
                        .map(|resolution| resolution.door_route),
                    recorded_gate_routes: group_move_resolution
                        .as_ref()
                        .map(|resolution| {
                            resolution
                                .recorded_gate_routes
                                .iter()
                                .map(|(actor, gates)| {
                                    (
                                        entity_map.translate(*actor),
                                        gates
                                            .iter()
                                            .map(|&(gate, direct)| {
                                                (entity_map.translate_gate(gate), direct)
                                            })
                                            .collect(),
                                    )
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    recorded_failed_gate_routes: group_move_resolution
                        .map(|resolution| {
                            resolution
                                .recorded_failed_gate_routes
                                .into_iter()
                                .map(|actor| entity_map.translate(actor))
                                .collect()
                        })
                        .unwrap_or_default(),
                }
            }
            Self::LaunchInteraction {
                actor,
                target,
                original_command: _,
                original_command_name,
                running,
            } => PlayerCommand::LaunchInteraction {
                actor: entity_map.translate(actor),
                target: entity_map.translate(target),
                command: command_from_stable_name(&original_command_name),
                running,
            },
            Self::LaunchSelfAbility {
                actor,
                original_command: _,
                original_command_name,
            } => PlayerCommand::LaunchSelfAbility {
                actor: entity_map.translate(actor),
                command: command_from_stable_name(&original_command_name),
            },
            Self::LaunchGroundTarget {
                actor,
                target,
                original_command: _,
                original_command_name,
                original_target_field,
                titbit_layer,
            } => {
                // The original game assigns these stable
                // field numbers. Translate semantically because Rust's Field
                // enum intentionally omits unrelated legacy properties.
                let target_field = match (original_command_name.as_str(), original_target_field) {
                    ("throw_purse", 30) => robin_engine::sequence::Field::PurseTarget,
                    ("throw_net", 31) => robin_engine::sequence::Field::NetTarget,
                    ("throw_wasp_nest", 32) => robin_engine::sequence::Field::WaspNestTarget,
                    (command, field) => panic!(
                        "unsupported Original ground-target command/field {command:?}/{field}"
                    ),
                };
                PlayerCommand::LaunchGroundTarget {
                    actor: entity_map.translate(actor),
                    target_pos: target.into(),
                    command: command_from_stable_name(&original_command_name),
                    target_field,
                    titbit_layer,
                }
            }
            Self::LaunchScrollRead {
                actor,
                target,
                running,
            } => PlayerCommand::LaunchScrollRead {
                actor: entity_map.translate(actor),
                target: entity_map.translate(target),
                running,
            },
            Self::SwordStrike {
                actor,
                target,
                original_command: _,
                original_command_name,
                with_seek,
                seek_distance,
            } => PlayerCommand::SwordStrikeCmd {
                actor: entity_map.translate(actor),
                target: entity_map.translate(target),
                command: command_from_stable_name(&original_command_name),
                composite: None,
                gesture_quality: GestureQuality::PERFECT,
                with_seek,
                seek_distance: trace_sword_seek_distance(with_seek, seek_distance),
            },
            Self::SelectPc { pc, append } => PlayerCommand::SelectPc {
                pc_id: entity_map.translate(pc),
                append,
            },
            Self::UnselectAllPcs => PlayerCommand::UnselectAllPcs,
            Self::StopPc { pc } => PlayerCommand::StopPc {
                pc_id: entity_map.translate(pc),
            },
            Self::SelectAction { pc, action, .. } => PlayerCommand::SelectResolvedAction {
                pc_id: entity_map.translate(pc),
                action: action.into(),
            },
            Self::CancelAction { pc, .. } => match pc {
                Some(pc) => PlayerCommand::CancelAction {
                    pc_id: entity_map.translate(pc),
                },
                None => PlayerCommand::UnselectAllActions,
            },
            Self::OrientActionAt {
                action,
                actor,
                mouse_map,
                target,
                original_action: _,
            } => PlayerCommand::PerformResolvedOrientation {
                pc_id: entity_map.translate(actor),
                action: action.into(),
                mouse_map: mouse_map.into(),
                target: target.into(),
            },
            Self::MakePcFast { entity } => PlayerCommand::MakePcFast {
                pc_id: entity_map.translate(entity),
            },
            Self::CrouchDown => PlayerCommand::CrouchDown,
            Self::StandUp => PlayerCommand::StandUp,
            Self::DropAleAt {
                actor,
                target,
                running,
            } => {
                let (
                    already_authorized,
                    goal_override,
                    goal_sector_index_override,
                    recorded_gate_path,
                ) = drop_ale_resolution
                    .map(|resolution| {
                        (
                            true,
                            Some(resolution.goal),
                            resolution.goal_sector_index,
                            resolution.recorded_gate_path,
                        )
                    })
                    .unwrap_or((false, None, None, None));
                PlayerCommand::DropAleAt {
                    actor: entity_map.translate(actor),
                    target_pos: target.into(),
                    running,
                    already_authorized,
                    goal_override,
                    goal_sector_index_override,
                    recorded_gate_path,
                }
            }
            Self::ShieldSelectProtected {
                actor,
                protected_pc,
            } => PlayerCommand::ShieldSelectProtected {
                actor: entity_map.translate(actor),
                protected_pc: entity_map.translate(protected_pc),
            },
            Self::BoxUnselect {
                first,
                second,
                append,
            } => {
                // Same shape as `BoxSelect`: the drag gesture and each
                // resolved nested unselect message are both recorded, so
                // replay only the resolved commands that follow.
                let _ = (first, second, append);
                return None;
            }
            Self::RaiseShieldWithDanger {
                actor,
                protected_pc,
                danger_point,
                danger_point_layer,
            } => {
                let danger_point: WorldPoint3D = danger_point.into();
                PlayerCommand::RaiseShieldWithDanger {
                    actor: entity_map.translate(actor),
                    protected_pc: entity_map.translate(protected_pc),
                    danger_point,
                    danger_point_layer,
                }
            }
            Self::TeleportSelected {
                destination,
                goal_sector,
                goal_layer,
            } => PlayerCommand::TeleportSelectedToPoint {
                dest: destination.into(),
                layer: goal_layer,
                // The Original records the selected sector's own number
                // (or -1 when no sector was selected), not a fast-grid
                // array index, so it transfers directly.
                sector: u16::try_from(goal_sector)
                    .ok()
                    .and_then(robin_engine::position_interface::SectorHandle::new),
            },
            Self::SelectAllPcs => PlayerCommand::SelectAllPcs,
            Self::UnselectPc { pc } => PlayerCommand::UnselectPc {
                pc_id: entity_map.translate(pc),
            },
            Self::SelectActionIndex { index } => {
                // The Original resolves the action-bar shortcut against
                // the single selected PC and does nothing at all for any
                // other selection cardinality.
                match engine.selected_hero_ids() {
                    [pc_id] => PlayerCommand::SelectAction {
                        pc_id: *pc_id,
                        action_index: index,
                    },
                    _ => return None,
                }
            }
            Self::SetLockAlt { on } => PlayerCommand::SetLockAlt(on),
            Self::KeyControl => PlayerCommand::KeyControl,
            Self::KeyReleaseControl => PlayerCommand::KeyReleaseControl,
            Self::StartMacro { pc, slot } => PlayerCommand::StartMacro {
                pc: pc.map(|pc| entity_map.translate(pc)),
                slot,
            },
            Self::DeleteMacro { pc, slot } => PlayerCommand::DeleteMacro {
                pc: pc.map(|pc| entity_map.translate(pc)),
                slot,
            },
            Self::StartRecordingMacro { pc, slot } => PlayerCommand::StartRecordingMacro {
                pc: pc.map(|pc| entity_map.translate(pc)),
                slot,
            },
            Self::ChangeQaMemory { slot } => PlayerCommand::ChangeQaMemory { slot },
            Self::HeroRefusedAction {
                actor,
                action,
                original_action: _,
                target: _,
                reason,
            } => {
                // Every refusal the recorder knows about barks the same line.
                // A new one must be taught here rather than silently replayed
                // as this one.
                match reason.as_str() {
                    "anonymous_archer_contest" | "locked_patch" => {}
                    other => panic!(
                        "unsupported refused-action reason {other:?} for {action:?} \
                         by {actor:?}"
                    ),
                }
                PlayerCommand::HeroSpeak {
                    pc_id: entity_map.translate(actor),
                    expression: robin_engine::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
                }
            }
            Self::BeggarDontTalkStamp { entity } => PlayerCommand::BeggarDontTalkStamp {
                beggar_id: entity_map.translate(entity),
            },
        })
    }
}

fn trace_sword_seek_distance(with_seek: bool, seek_distance: f32) -> Option<f32> {
    (with_seek && !seek_distance.is_nan()).then_some(seek_distance)
}

fn missing_legacy_seek_distance() -> f32 {
    // Schema-16 recordings made before the additive seek-distance diagnostic
    // cannot reconstruct it. A quiet NaN is outside the valid distance domain,
    // survives the unchanged native f32 layout, and is mapped back to `None`
    // before command admission.
    f32::NAN
}

fn command_from_stable_name(name: &str) -> Command {
    let rust_name = match name {
        "camera_jumpto" => "CameraJumpTo".to_owned(),
        // The original game uses the low-level rolling movement and the
        // contextual player ability JUMP. Rust's historical names are Jump
        // and JumpCmd respectively.
        "roll" => "Jump".to_owned(),
        "jump" => "JumpCmd".to_owned(),
        "search" => "SearchCmd".to_owned(),
        "hit" => "HitCmd".to_owned(),
        "heal" => "HealCmd".to_owned(),
        "eat" => "EatCmd".to_owned(),
        "tie" => "TieCmd".to_owned(),
        "strangle" => "StrangleCmd".to_owned(),
        "whistle" => "WhistleCmd".to_owned(),
        "launch_postseek" => "LaunchPostSeek".to_owned(),
        "launch_quickaction" => "LaunchQuickAction".to_owned(),
        other => other
            .split('_')
            .map(|word| {
                let mut chars = word.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                    None => String::new(),
                }
            })
            .collect(),
    };
    serde_json::from_value(serde_json::Value::String(rust_name))
        .unwrap_or_else(|_| panic!("unsupported stable original-game command name {name:?}"))
}

/// Element layout embedded in version-68 native frame records.
///
/// ON-DISK FORMAT INVARIANT: do not change fields, their order, or their
/// types without bumping `TRACE_NATIVE_VERSION` and freezing this layout in a
/// version-named compatibility type, as done by [`TraceElementV67`].
#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceElement {
    entity_id: TraceEntityId,
    creation_order: u32,
    class_id: u16,
    kind: TraceEntityKind,
    active: bool,
    blipped: bool,
    unreachable: bool,
    surface_id: u32,
    posture: u32,
    position_map: TracePoint,
    old_position_map: TracePoint,
    position_goal_map: TracePoint,
    elevation: TraceFloat,
    old_elevation: TraceFloat,
    increment_map: TracePoint,
    /// Missing in early schema-16 frames. Presence, including an authoritative
    /// `false`, must survive conversion to the native trace.
    #[serde(default)]
    increment_map_valid: Option<bool>,
    movement_map: TracePoint,
    layer: u16,
    layer_goal: u16,
    sector: u16,
    direction: i16,
    direction_goal: i16,
    moving: bool,
    moving_map: bool,
    sprite_row: u16,
    sprite_frame: u16,
    sprite_frame_count: u16,
    #[serde(default)]
    actor: Option<TraceActor>,
    #[serde(default)]
    human: Option<TraceHuman>,
    #[serde(default)]
    pc: Option<TraceElementPc>,
    #[serde(default)]
    ai: Option<TraceAi>,
    #[serde(default)]
    detection: Option<TraceDetection>,
    /// Whole-entity serialized position/sprite frontier. Early schema-16
    /// recordings omit it; JSON null is the native-layout-compatible marker
    /// for "not recorded" and is excluded from logical comparison.
    #[serde(default = "missing_legacy_trace_json_value")]
    runtime: TraceJsonValue,
}

/// Element snapshot layout embedded in native trace version 67. Keep this
/// frozen: even a field-order-only edit changes bitcode's on-disk shape.
#[derive(Debug, bitcode::Encode, bitcode::Decode)]
struct TraceElementV67 {
    entity_id: TraceEntityId,
    creation_order: u32,
    class_id: u16,
    kind: TraceEntityKind,
    active: bool,
    blipped: bool,
    unreachable: bool,
    surface_id: u32,
    posture: u32,
    position_map: TracePoint,
    old_position_map: TracePoint,
    position_goal_map: TracePoint,
    elevation: TraceFloat,
    old_elevation: TraceFloat,
    increment_map: TracePoint,
    increment_map_valid: bool,
    movement_map: TracePoint,
    layer: u16,
    layer_goal: u16,
    sector: u16,
    direction: i16,
    direction_goal: i16,
    moving: bool,
    moving_map: bool,
    sprite_row: u16,
    sprite_frame: u16,
    sprite_frame_count: u16,
    actor: Option<TraceActor>,
    human: Option<TraceHuman>,
    pc: Option<TraceElementPc>,
    ai: Option<TraceAi>,
    detection: Option<TraceDetection>,
    runtime: TraceJsonValue,
}

#[derive(Debug, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceElementV66 {
    entity_id: TraceEntityId,
    creation_order: u32,
    class_id: u16,
    kind: TraceEntityKind,
    active: bool,
    blipped: bool,
    unreachable: bool,
    surface_id: u32,
    posture: u32,
    position_map: TracePoint,
    old_position_map: TracePoint,
    position_goal_map: TracePoint,
    elevation: TraceFloat,
    old_elevation: TraceFloat,
    increment_map: TracePoint,
    increment_map_valid: Option<bool>,
    movement_map: TracePoint,
    layer: u16,
    layer_goal: u16,
    sector: u16,
    direction: i16,
    direction_goal: i16,
    moving: bool,
    moving_map: bool,
    sprite_row: u16,
    sprite_frame: u16,
    sprite_frame_count: Option<u16>,
    actor: Option<TraceActorV66>,
    human: Option<TraceHumanV66>,
    pc: Option<TraceElementPc>,
    ai: Option<TraceAiV66>,
    detection: Option<TraceDetection>,
    runtime: Option<TraceJsonValue>,
}

#[derive(Debug, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceActorV66 {
    action_state: u32,
    animation: u32,
    command: u16,
    command_name: String,
    motion_state: u32,
    wait_time: u32,
    passing_door_directly: Option<bool>,
    active_pass_door: Option<Option<TracePassDoor>>,
    sequence_element: Option<Option<TraceSequenceElement>>,
    position_interface: Option<TraceJsonValue>,
}

#[derive(Debug, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceHumanV66 {
    life_points: i16,
    dead: bool,
    unconscious: bool,
    camp: String,
    original_camp: i32,
    vip: bool,
    civilian: bool,
    opponents: Option<Vec<TraceEntityId>>,
    opponent_jump_lines: Option<Vec<Option<TraceJumpLine>>>,
}

#[derive(Debug, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceAiV66 {
    state: u32,
    substate: u32,
    script_locked: Option<bool>,
    locked: Option<bool>,
    locks: Option<u8>,
    was_busy: Option<bool>,
    very_busy: Option<bool>,
    macro_timer_running: Option<bool>,
    macro_timer_ring: Option<u32>,
    macro_cursor: Option<Option<u16>>,
    macro_remaining: Option<u16>,
    macro_in_progress: Option<bool>,
    list_us: Option<Vec<TraceEntityId>>,
    list_them: Option<Vec<TraceEntityId>>,
    my_line_jump: Option<Option<TraceJumpLine>>,
}

impl TraceActorV66 {
    fn into_current(self) -> TraceActor {
        TraceActor {
            action_state: self.action_state,
            animation: self.animation,
            command: self.command,
            command_name: self.command_name,
            motion_state: self.motion_state,
            wait_time: self.wait_time,
            passing_door_directly: self.passing_door_directly.unwrap_or(false),
            active_pass_door: self.active_pass_door.flatten(),
            sequence_element: self.sequence_element.flatten(),
            position_interface: self
                .position_interface
                .unwrap_or_else(missing_legacy_trace_json_value),
        }
    }
}

impl TraceHumanV66 {
    fn into_current(self) -> TraceHuman {
        TraceHuman {
            life_points: self.life_points,
            dead: self.dead,
            unconscious: self.unconscious,
            camp: self.camp,
            original_camp: self.original_camp,
            vip: self.vip,
            civilian: self.civilian,
            opponents: self.opponents.unwrap_or_default(),
            opponent_jump_lines: self.opponent_jump_lines.unwrap_or_default(),
        }
    }
}

impl TraceAiV66 {
    fn into_current(self) -> TraceAi {
        TraceAi {
            state: self.state,
            substate: self.substate,
            script_locked: self.script_locked.unwrap_or(false),
            locked: self.locked.unwrap_or(false),
            locks: self.locks.unwrap_or(0),
            was_busy: self.was_busy.unwrap_or(false),
            very_busy: self.very_busy.unwrap_or(false),
            macro_timer_running: self.macro_timer_running.unwrap_or(false),
            macro_timer_ring: self.macro_timer_ring.unwrap_or(0),
            macro_cursor: self.macro_cursor.flatten(),
            macro_remaining: self.macro_remaining.unwrap_or(0),
            macro_in_progress: self.macro_in_progress.unwrap_or(false),
            list_us: self.list_us.unwrap_or_default(),
            list_them: self.list_them.unwrap_or_default(),
            my_line_jump: self.my_line_jump.flatten(),
        }
    }
}

impl TraceElementV66 {
    fn into_current(self) -> TraceElement {
        TraceElement {
            entity_id: self.entity_id,
            creation_order: self.creation_order,
            class_id: self.class_id,
            kind: self.kind,
            active: self.active,
            blipped: self.blipped,
            unreachable: self.unreachable,
            surface_id: self.surface_id,
            posture: self.posture,
            position_map: self.position_map,
            old_position_map: self.old_position_map,
            position_goal_map: self.position_goal_map,
            elevation: self.elevation,
            old_elevation: self.old_elevation,
            increment_map: self.increment_map,
            increment_map_valid: self.increment_map_valid,
            movement_map: self.movement_map,
            layer: self.layer,
            layer_goal: self.layer_goal,
            sector: self.sector,
            direction: self.direction,
            direction_goal: self.direction_goal,
            moving: self.moving,
            moving_map: self.moving_map,
            sprite_row: self.sprite_row,
            sprite_frame: self.sprite_frame,
            sprite_frame_count: self.sprite_frame_count.unwrap_or(0),
            actor: self.actor.map(TraceActorV66::into_current),
            human: self.human.map(TraceHumanV66::into_current),
            pc: self.pc,
            ai: self.ai.map(TraceAiV66::into_current),
            detection: self.detection,
            runtime: self.runtime.unwrap_or_else(missing_legacy_trace_json_value),
        }
    }
}

impl TraceElementV67 {
    fn into_current(self, increment_map_valid_was_recorded: bool) -> TraceElement {
        TraceElement {
            entity_id: self.entity_id,
            creation_order: self.creation_order,
            class_id: self.class_id,
            kind: self.kind,
            active: self.active,
            blipped: self.blipped,
            unreachable: self.unreachable,
            surface_id: self.surface_id,
            posture: self.posture,
            position_map: self.position_map,
            old_position_map: self.old_position_map,
            position_goal_map: self.position_goal_map,
            elevation: self.elevation,
            old_elevation: self.old_elevation,
            increment_map: self.increment_map,
            increment_map_valid: increment_map_valid_was_recorded
                .then_some(self.increment_map_valid),
            movement_map: self.movement_map,
            layer: self.layer,
            layer_goal: self.layer_goal,
            sector: self.sector,
            direction: self.direction,
            direction_goal: self.direction_goal,
            moving: self.moving,
            moving_map: self.moving_map,
            sprite_row: self.sprite_row,
            sprite_frame: self.sprite_frame,
            sprite_frame_count: self.sprite_frame_count,
            actor: self.actor,
            human: self.human,
            pc: self.pc,
            ai: self.ai,
            detection: self.detection,
            runtime: self.runtime,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceActor {
    action_state: u32,
    animation: u32,
    command: u16,
    command_name: String,
    motion_state: u32,
    wait_time: u32,
    #[serde(default)]
    passing_door_directly: bool,
    /// Explicitly null when there is no active PassDoor.
    #[serde(default, deserialize_with = "deserialize_nullable_pass_door")]
    active_pass_door: Option<TracePassDoor>,
    /// Rust does not yet expose a stable public current-sequence snapshot with
    /// Original's element identities.
    /// TODO(parity-sequence): compare the remaining fields once that capture
    /// can be produced without walking mutable sequence-manager internals.
    #[serde(default, deserialize_with = "deserialize_nullable_sequence_element")]
    sequence_element: Option<TraceSequenceElement>,
    /// PositionInterface diagnostics. Kept as a cache-safe JSON
    /// tree because it is observational evidence rather than comparable
    /// engine state yet.
    #[serde(default = "missing_legacy_trace_json_value")]
    position_interface: TraceJsonValue,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TracePassDoor {
    gate_id: u32,
    direct: bool,
    direction: i16,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceSequenceElement {
    id: u32,
    #[serde(rename = "type")]
    element_type: u8,
    state: u32,
    command_level: u16,
    command: u16,
    command_name: String,
    order_count: u16,
    priority: u32,
    posture_after_transition: u32,
    action_state_after_transition: u32,
    #[serde(default, deserialize_with = "deserialize_nullable_sequence_movement")]
    movement: Option<TraceSequenceMovement>,
    /// Current sequence topology and active-order diagnostics. These are
    /// nullable or command-shaped in the Original recorder, so retaining the
    /// draft payload verbatim is safer than inventing a false common shape.
    #[serde(default, deserialize_with = "deserialize_nullable_trace_json_value")]
    following: Option<TraceJsonValue>,
    #[serde(default, deserialize_with = "deserialize_nullable_trace_json_value")]
    postponed: Option<TraceJsonValue>,
    #[serde(default, deserialize_with = "deserialize_nullable_trace_json_value")]
    current_order: Option<TraceJsonValue>,
    #[serde(default, deserialize_with = "deserialize_nullable_trace_json_value")]
    movement_payload: Option<TraceJsonValue>,
}

fn deserialize_nullable_sequence_movement<'de, D>(
    deserializer: D,
) -> Result<Option<TraceSequenceMovement>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<TraceSequenceMovement>::deserialize(deserializer)
}

fn deserialize_nullable_trace_json_value<'de, D>(
    deserializer: D,
) -> Result<Option<TraceJsonValue>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<TraceJsonValue>::deserialize(deserializer)
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceSequenceMovement {
    /// Absent in current schema-16 traces when the movement-element
    /// constructor does not initialize `maction` (for example WAIT_FREE_LIFT).
    #[serde(default)]
    action: Option<u32>,
    #[serde(default)]
    pass_door: Option<TracePassDoor>,
}

fn deserialize_nullable_pass_door<'de, D>(
    deserializer: D,
) -> Result<Option<TracePassDoor>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<TracePassDoor>::deserialize(deserializer)
}

fn deserialize_nullable_sequence_element<'de, D>(
    deserializer: D,
) -> Result<Option<TraceSequenceElement>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<TraceSequenceElement>::deserialize(deserializer)
}

fn trace_pass_door_key(pass: &TracePassDoor) -> (u32, bool) {
    assert_eq!(
        pass.direct,
        pass.direction != 0,
        "current-schema active PassDoor direct flag disagrees with its direction"
    );
    (pass.gate_id, pass.direct)
}

fn active_pass_door_keys_match(
    expected: Option<&TracePassDoor>,
    actual: Option<(u32, bool)>,
) -> bool {
    expected.map(trace_pass_door_key) == actual
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceHuman {
    life_points: i16,
    dead: bool,
    unconscious: bool,
    camp: String,
    original_camp: i32,
    vip: bool,
    civilian: bool,
    opponents: Vec<TraceEntityId>,
    opponent_jump_lines: Vec<Option<TraceJumpLine>>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
struct TraceJumpLine {
    a: TracePoint,
    b: TracePoint,
}

fn trace_jump_line_bits(line: &TraceJumpLine) -> [u32; 4] {
    [line.a.x.bits, line.a.y.bits, line.b.x.bits, line.b.y.bits]
}

fn runtime_jump_line_bits(line: &robin_engine::jump_line::JumpLine) -> [u32; 4] {
    [
        line.point_a.x.to_bits(),
        line.point_a.y.to_bits(),
        line.point_b.x.to_bits(),
        line.point_b.y.to_bits(),
    ]
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceElementPc {
    ammo: TraceElementAmmo,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceElementAmmo {
    ales: u16,
    apples: u16,
    arrows: u16,
    nets: u16,
    plants: u16,
    purses: u16,
    rations: u16,
    stones: u16,
    wasp_nests: u16,
}

fn deserialize_nullable_u16<'de, D>(deserializer: D) -> Result<Option<u16>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<u16>::deserialize(deserializer)
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceAi {
    state: u32,
    substate: u32,
    #[serde(default)]
    script_locked: bool,
    #[serde(default)]
    locked: bool,
    #[serde(default)]
    locks: u8,
    #[serde(default)]
    was_busy: bool,
    #[serde(default)]
    very_busy: bool,
    #[serde(default)]
    macro_timer_running: bool,
    #[serde(default)]
    macro_timer_ring: u32,
    /// Explicitly null for an inactive macro.
    #[serde(default, deserialize_with = "deserialize_nullable_u16")]
    macro_cursor: Option<u16>,
    #[serde(default)]
    macro_remaining: u16,
    #[serde(default)]
    macro_in_progress: bool,
    #[serde(default)]
    list_us: Vec<TraceEntityId>,
    #[serde(default)]
    list_them: Vec<TraceEntityId>,
    /// Authoritative jump-line reference, explicitly null when absent.
    #[serde(default, deserialize_with = "deserialize_nullable_jump_line")]
    my_line_jump: Option<TraceJumpLine>,
}

fn deserialize_nullable_jump_line<'de, D>(
    deserializer: D,
) -> Result<Option<TraceJumpLine>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<TraceJumpLine>::deserialize(deserializer)
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceDetection {
    suspects: Vec<u16>,
    maximal_suspect: u16,
    maximal_visibility: u32,
    view_status: u8,
    alert_status: u32,
    detectables: Vec<TraceDetectable>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceDetectable {
    #[serde(rename = "type")]
    detectable_type: u32,
    target: TraceEntityId,
    seen_now: bool,
    seen_last_frame: bool,
    heard_last_frame: bool,
    shadow_seen_now: bool,
    shadow_seen_last_frame: bool,
    last_visibility: TraceFloat,
}

#[derive(Clone, Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceVisibilityQuery {
    origin: TracePoint3,
    destination: TracePoint3,
    result: bool,
    cache_hit: bool,
    cache_key: u64,
    cache_offset: u64,
    candidate_count: u16,
    reason: String,
    blocking_obstacle: Option<TraceSightObstacle>,
}

#[derive(Clone, Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceSightObstacle {
    id: u32,
    index: i64,
    type_mask: i32,
    types: TraceSightObstacleTypes,
    active: bool,
    on_ground: bool,
    layer: u16,
    sector: u16,
    box_ground: TraceSightObstacleBox,
    points: Vec<TraceSightObstaclePoint>,
}

#[derive(Clone, Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceSightObstacleTypes {
    solid: bool,
    opaque: bool,
    projection_area: bool,
    mouse: bool,
    shield: bool,
    show_shadow_polygon: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceSightObstacleBox {
    min: TracePoint,
    max: TracePoint,
}

#[derive(Clone, Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceSightObstaclePoint {
    x: TraceFloat,
    y: TraceFloat,
    z_top: TraceFloat,
    z_bottom: TraceFloat,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceResolvedExclamation {
    actor: TraceEntityId,
    identifier: u32,
    exclamation_id: u16,
    selected_variant: i32,
    selected_entry: Option<u32>,
    duration_frames: u32,
}

/// One exact, ordered original-game motion position commit.
/// This additive diagnostic is absent unless the Original recorder was run
/// with `RH_PARITY_MOVEMENT_STEPS` enabled.
#[derive(Clone, Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceMovementStep {
    entity: TraceEntityId,
    order_id: u32,
    order_action: u32,
    animation: u32,
    motion_method: u32,
    pre_position: TracePoint,
    old_position: TracePoint,
    goal: TracePoint,
    cached_increment: TracePoint,
    frame_distance_raw: TraceFloat,
    speed_factor: TraceFloat,
    effective_distance: TraceFloat,
    anti_collision: bool,
    reverse: bool,
    raw_post_position: TracePoint,
    raw_committed_delta: TracePoint,
    post_position: TracePoint,
    committed_delta: TracePoint,
    goal_reached: bool,
    snapped_to_goal: bool,
}

/// One exact, ordered original-game flight execution.
#[derive(Clone, Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceFlightStep {
    entity: TraceEntityId,
    order_id: u32,
    order_action: u32,
    animation: u32,
    flight_style: u32,
    entry_position: TracePoint3,
    entry_position_map: TracePoint,
    old_position: TracePoint3,
    old_position_map: TracePoint,
    goal: TracePoint3,
    cached_increment: TracePoint3,
    applied_increment: TracePoint3,
    raw_post_position: TracePoint3,
    raw_post_position_map: TracePoint,
    motion_state: u32,
    post_position: TracePoint3,
    post_position_map: TracePoint,
    snapped_to_goal: bool,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceRouteConstructionEvent {
    kind: String,
    actor: TraceEntityId,
    source: TracePoint,
    source_sector: u16,
    source_level: u16,
    goal: TracePoint,
    goal_sector: u16,
    goal_level: u16,
    gates: Vec<TraceRouteGate>,
    /// Schema-16 may extend route events while its diagnostic contract is
    /// being exercised against real recordings. Retain every additive field
    /// in the native cache instead of silently discarding useful evidence.
    #[serde(flatten)]
    draft_diagnostics: BTreeMap<String, TraceJsonValue>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceRouteGate {
    gate_id: u32,
    direct: bool,
    sector_out: u16,
    level_out: u16,
    sector_in: u16,
    level_in: u16,
    #[serde(flatten)]
    draft_diagnostics: BTreeMap<String, TraceJsonValue>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReplayDropAleResolution {
    goal: (SectorNumber, u16),
    goal_sector_index: Option<robin_engine::fast_find_grid::SectorIndex>,
    recorded_gate_path: Option<robin_engine::gate::RecordedGatePath>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReplayGroupMoveResolution {
    door_route: bool,
    /// A patch sector is a real Original motion-area identity but has no
    /// standalone Rust position-sector number. For a successful recorded
    /// gate route, its terminal gate exit is the equivalent Rust graph goal.
    unmapped_goal_search_sector: Option<u16>,
    /// Successful movement-sequence gate paths are already
    /// authoritative at this boundary. Replaying them avoids a second A*
    /// search choosing a different valid path and changing the emitted
    /// building waits (including their RNG draws).
    recorded_gate_routes: Vec<(TraceEntityId, Vec<(u32, bool)>)>,
    /// Authoritative failed movement-sequence searches, keyed by actor.
    /// The empty gate list is an observed failure outcome, not permission to
    /// run Rust's A* again against reconstructed topology.
    recorded_failed_gate_routes: Vec<TraceEntityId>,
}

fn required_route_construction_ordinal(event: &TraceRouteConstructionEvent) -> u64 {
    match event
        .draft_diagnostics
        .get("ordinal")
        .map(TraceJsonValue::tree)
    {
        Some(TraceJsonTree::Unsigned(ordinal)) => ordinal,
        other => panic!("schema-16 route event lacks an unsigned ordinal: {other:?}"),
    }
}

/// Restore additive route diagnostics omitted by early schema-16 captures.
///
/// Original appends each event immediately after assigning the incrementing
/// per-frame counter, so vector position is the
/// authoritative missing ordinal. The archived pre-failure-diagnostics
/// recorder emitted route events only after successful construction; the
/// later route-attempt/failure reporting added explicit failure events and
/// the `result` field together. Therefore
/// an omitted result in this legacy generation is authoritatively success.
/// This is called only for the legacy header generation; current captures
/// must continue to carry both fields explicitly.
fn restore_legacy_route_construction_diagnostics(events: &mut [TraceRouteConstructionEvent]) {
    for (ordinal, event) in events.iter_mut().enumerate() {
        let ordinal = u64::try_from(ordinal).expect("route event count exceeds u64");
        match event
            .draft_diagnostics
            .get("ordinal")
            .map(TraceJsonValue::tree)
        {
            None => {
                event.draft_diagnostics.insert(
                    "ordinal".to_owned(),
                    TraceJsonValue::from(TraceJsonTree::Unsigned(ordinal)),
                );
            }
            Some(TraceJsonTree::Unsigned(recorded)) => assert_eq!(
                recorded, ordinal,
                "legacy schema-16 route ordinal disagrees with Original append order"
            ),
            other => panic!("legacy schema-16 route event has an invalid ordinal: {other:?}"),
        }
        event
            .draft_diagnostics
            .entry("result".to_owned())
            .or_insert_with(|| TraceJsonValue::from(TraceJsonTree::String("success".to_owned())));
    }
}

/// Recover whether movement-sequence construction selected its internal
/// gate-path or door-entry branch for a current-schema group move.
/// Both branches record route kind `move`; `move_to_door` belongs to the
/// separate door-movement API and is not this discriminator.
/// The retained Original sparse sector topology preserves that distinction:
/// a door goal names its exact gate, while an ordinary patch remains ordinary
/// even when its route happens to end across that same overlay door.
// TODO(parity-schema): record the selected group-move route constructor on
// TraceCommand::GroupMove so future traces do not need this event join.
fn resolve_current_group_move_route(
    command: &TraceCommand,
    route_events: &[TraceRouteConstructionEvent],
    consumed_route_ordinals: &mut BTreeSet<u64>,
    entity_map: &EntityMap,
    retained_sector_kinds: &[LegacyGridSectorAsset],
) -> Option<ReplayGroupMoveResolution> {
    let TraceCommand::GroupMove {
        actors,
        goal_sector,
        ..
    } = command
    else {
        return None;
    };
    let goal_sector = u16::try_from(*goal_sector)
        .unwrap_or_else(|_| panic!("schema-16 group-move goal sector is negative: {goal_sector}"));
    let goal_kind = retained_sector_kinds
        .get(usize::from(goal_sector))
        .unwrap_or_else(|| {
            panic!(
                "schema-16 group-move goal sector {goal_sector} is absent from retained Original topology"
            )
        });
    let goal_door = match goal_kind {
        LegacyGridSectorAsset::Door { gate_index } => Some(entity_map.translate_gate(*gate_index)),
        LegacyGridSectorAsset::NullOrOrdinary
        | LegacyGridSectorAsset::Building
        | LegacyGridSectorAsset::Lift => None,
    };

    let mut matching = route_events
        .iter()
        .filter_map(|event| {
            let ordinal = required_route_construction_ordinal(event);
            if consumed_route_ordinals.contains(&ordinal)
                || !actors.contains(&event.actor)
                || event.goal_sector != goal_sector
                || event.kind != "move"
            {
                return None;
            }
            Some((ordinal, event))
        })
        .collect::<Vec<_>>();
    // Same-sector moves do not construct a gate route, but retained Original
    // topology still authoritatively identifies whether group movement's
    // selected goal was a door. Do not fall back to Rust's overlapping-polygon
    // hit in that case.
    if matching.is_empty() {
        return Some(ReplayGroupMoveResolution {
            door_route: goal_door.is_some(),
            unmapped_goal_search_sector: None,
            recorded_gate_routes: Vec::new(),
            recorded_failed_gate_routes: Vec::new(),
        });
    }
    matching.sort_unstable_by_key(|(ordinal, _)| *ordinal);
    // Group movement executes movement once for each selected actor, and
    // that reaches at most one movement-sequence route construction
    // in the original game. A
    // frame can nevertheless contain several group-move commands with the
    // same actor and goal sector. Their route events share the only identities
    // schema 16 recorded for this join, so greedily taking every match assigns
    // later commands' routes to the first command. Consume the earliest
    // unclaimed route per actor and leave later ordinals for the following
    // command in frame order.
    let mut actors_with_route = BTreeSet::new();
    matching.retain(|(_, event)| actors_with_route.insert(event.actor));
    if let Some(goal_door) = goal_door {
        assert!(
            matching.iter().all(|(_, event)| {
                !matches!(
                    event.draft_diagnostics.get("result").map(TraceJsonValue::tree),
                    Some(TraceJsonTree::String(result)) if result == "success"
                ) || event
                    .gates
                    .last()
                    .is_some_and(|gate| entity_map.translate_gate(gate.gate_id) == goal_door)
            }),
            "successful door-target group move did not terminate at retained goal door {goal_door}: {matching:?}"
        );
    }
    let door_route = goal_door.is_some();
    let mut terminal_exit_sectors = matching
        .iter()
        .filter_map(|(_, event)| {
            event.gates.last().map(|gate| {
                if gate.direct {
                    gate.sector_in
                } else {
                    gate.sector_out
                }
            })
        })
        .collect::<BTreeSet<_>>();
    assert!(
        terminal_exit_sectors.len() <= 1,
        "one group move produced routes ending in different sectors: {matching:?}"
    );
    let unmapped_goal_search_sector = terminal_exit_sectors.pop_first();
    let recorded_gate_routes = matching
        .iter()
        .filter(|(_, event)| {
            matches!(
                event.draft_diagnostics.get("result").map(TraceJsonValue::tree),
                Some(TraceJsonTree::String(result)) if result == "success"
            ) && !event.gates.is_empty()
        })
        .map(|(_, event)| {
            (
                event.actor,
                event
                    .gates
                    .iter()
                    .map(|gate| (gate.gate_id, gate.direct))
                    .collect(),
            )
        })
        .collect();
    let recorded_failed_gate_routes = matching
        .iter()
        .filter(|(_, event)| {
            matches!(
                event.draft_diagnostics.get("result").map(TraceJsonValue::tree),
                Some(TraceJsonTree::String(result)) if result == "failure"
            )
        })
        .map(|(_, event)| {
            assert!(
                event.gates.is_empty(),
                "failed schema-16 group-move route unexpectedly retained gates: {event:?}"
            );
            event.actor
        })
        .collect::<Vec<_>>();
    assert_eq!(
        recorded_failed_gate_routes
            .iter()
            .collect::<BTreeSet<_>>()
            .len(),
        recorded_failed_gate_routes.len(),
        "one schema-16 group move recorded multiple failed routes for the same actor"
    );
    for (ordinal, _) in matching {
        assert!(
            consumed_route_ordinals.insert(ordinal),
            "schema-16 route ordinal {ordinal} matched twice"
        );
    }
    Some(ReplayGroupMoveResolution {
        door_route,
        unmapped_goal_search_sector,
        recorded_gate_routes,
        recorded_failed_gate_routes,
    })
}

/// Recover the goal that Original retained before authorizing a DropAle
/// destination. The current schema does not yet carry that goal on
/// `drop_ale_at`, but a
/// cross-sector Seek records it in the same frame's route-construction stream.
/// Match by route ordinal plus stable actor/point identity; projected point
/// containment is intentionally not consulted because overlapping floors can
/// select a different layer. Same-sector seeks publish no route event, so the
/// caller also supplies a geometrically verified same-sector identity as a
/// fallback. Legacy schema-16 artifacts can omit the route stream entirely;
/// in that case a target outside the actor's exact sector must fall back to
/// ordinary live resolution rather than being mislabeled as same-sector.
// TODO(parity-schema): record DropAle's pre-authorization goal sector/layer on
// the command itself so replay does not have to infer same-sector destinations.
fn resolve_current_drop_ale(
    command: &TraceCommand,
    route_events: &[TraceRouteConstructionEvent],
    consumed_route_ordinals: &mut BTreeSet<u64>,
    entity_map: &EntityMap,
    same_sector_goal: Option<ReplayDropAleResolution>,
    qa_recording: bool,
) -> Option<ReplayDropAleResolution> {
    let TraceCommand::DropAleAt { actor, target, .. } = command else {
        return None;
    };

    let actor = entity_map.translate(*actor);
    let matching_events = route_events
        .iter()
        .filter_map(|event| {
            let ordinal = required_route_construction_ordinal(event);
            if consumed_route_ordinals.contains(&ordinal)
                || event.kind != "move"
                || entity_map.translate(event.actor) != actor
                || event.goal.x.bits != target.x.bits
                || event.goal.y.bits != target.y.bits
                || event.source_sector == event.goal_sector
            {
                return None;
            }
            Some((ordinal, event))
        })
        .collect::<Vec<_>>();
    assert!(
        matching_events.len() <= 1,
        "schema-16 DropAle command matched {} exact route events",
        matching_events.len()
    );
    let Some((ordinal, event)) = matching_events.into_iter().next() else {
        if qa_recording {
            // The original game stores the selected target-sector identity in the quick action's
            // Seek without launching it, so there is intentionally no route
            // event from which schema 16 can recover that pointer. The actor
            // sector is not a valid substitute: the selected target may be a
            // different (or duplicate-number) sector.
            //
            // TODO(parity-schema): record DropAle's selected target sector,
            // exact arena identity, and layer directly on the command.
            panic!(
                "schema-16 DropAle recorded as a quick action has no authoritative target-sector identity"
            );
        }
        // A cross-sector DropAle must have an authoritative route event. Do
        // not disguise a mismatched/corrupt event as a same-sector command.
        let has_actor_route = route_events.iter().any(|event| {
            event.kind == "move"
                && entity_map.translate(event.actor) == actor
                && event.source_sector != event.goal_sector
        });
        return (!has_actor_route).then_some(same_sector_goal).flatten();
    };
    assert!(
        consumed_route_ordinals.insert(ordinal),
        "schema-16 route ordinal {ordinal} matched twice"
    );

    let (goal_sector, goal_sector_index) =
        entity_map.translate_required_drop_ale_goal_sector(event.goal_sector);
    let recorded_gate_path = recorded_gate_path_from_event(event, entity_map);
    Some(ReplayDropAleResolution {
        goal: (goal_sector, event.goal_level),
        goal_sector_index: Some(goal_sector_index),
        recorded_gate_path: Some(recorded_gate_path),
    })
}

fn recorded_gate_path_from_event(
    event: &TraceRouteConstructionEvent,
    entity_map: &EntityMap,
) -> robin_engine::gate::RecordedGatePath {
    let (source_sector, source_sector_index) =
        entity_map.translate_required_drop_ale_goal_sector(event.source_sector);
    let outcome = match event
        .draft_diagnostics
        .get("result")
        .map(TraceJsonValue::tree)
    {
        Some(TraceJsonTree::String(result)) if result == "success" => {
            assert!(
                !event.gates.is_empty(),
                "successful cross-sector DropAle route has no gates: {event:?}"
            );
            robin_engine::gate::RecordedGateOutcome::Success(
                event
                    .gates
                    .iter()
                    .map(|gate| robin_engine::gate::GatePathStep {
                        door_index: robin_engine::gate::DoorIndex::from(
                            entity_map.translate_gate(gate.gate_id),
                        ),
                        direct: gate.direct,
                    })
                    .collect(),
            )
        }
        Some(TraceJsonTree::String(result)) if result == "failure" => {
            assert!(
                event.gates.is_empty(),
                "failed cross-sector DropAle route retained gates: {event:?}"
            );
            robin_engine::gate::RecordedGateOutcome::Failure
        }
        other => panic!("schema-16 DropAle route has invalid result: {other:?}"),
    };
    robin_engine::gate::RecordedGatePath {
        source_sector,
        source_sector_index: Some(source_sector_index),
        source_layer: event.source_level,
        outcome,
    }
}

fn collect_current_delayed_drop_ale_routes(
    route_events: &[TraceRouteConstructionEvent],
    consumed_drop_ale_route_ordinals: &mut BTreeSet<u64>,
    consumed_group_move_route_ordinals: &BTreeSet<u64>,
    entity_map: &EntityMap,
    engine: &mut Engine,
) -> Vec<robin_engine::engine::RecordedDropAleRoute> {
    let replay_setup = engine.parity_replay_setup();
    collect_current_delayed_drop_ale_routes_matching(
        route_events,
        consumed_drop_ale_route_ordinals,
        consumed_group_move_route_ordinals,
        entity_map,
        |actor, destination| replay_setup.has_pending_recorded_drop_ale_route(actor, destination),
    )
}

fn collect_current_delayed_drop_ale_routes_matching(
    route_events: &[TraceRouteConstructionEvent],
    consumed_drop_ale_route_ordinals: &mut BTreeSet<u64>,
    consumed_group_move_route_ordinals: &BTreeSet<u64>,
    entity_map: &EntityMap,
    mut has_pending_replay_seek: impl FnMut(EntityId, robin_engine::coordinates::MapPoint) -> bool,
) -> Vec<robin_engine::engine::RecordedDropAleRoute> {
    assert!(
        consumed_drop_ale_route_ordinals.is_disjoint(consumed_group_move_route_ordinals),
        "schema-16 route ordinal was consumed independently by DropAle and group-move joins"
    );
    let mut seen_route_ordinals = BTreeSet::new();
    let mut routes = Vec::new();
    for event in route_events {
        let ordinal = required_route_construction_ordinal(event);
        if event.kind != "move" || event.source_sector == event.goal_sector {
            continue;
        }
        assert!(
            seen_route_ordinals.insert(ordinal),
            "schema-16 route stream contains duplicate ordinal {ordinal}"
        );
        if consumed_drop_ale_route_ordinals.contains(&ordinal)
            || consumed_group_move_route_ordinals.contains(&ordinal)
        {
            continue;
        }
        let actor = entity_map.translate(event.actor);
        let destination = event.goal.into();
        // A route-construction event is a shared diagnostic stream: ordinary
        // movement and group moves can leave cross-sector entries here too.
        // Only a staged point-Seek proves that this event belongs to a delayed
        // DropAle command. Older recordings cannot carry a separate provenance
        // bit, so treating every otherwise-unclaimed move route as DropAle
        // both rejects valid traces and is not justified by Original's flow.
        if !has_pending_replay_seek(actor, destination) {
            continue;
        }
        claim_delayed_drop_ale_route_ordinal(
            ordinal,
            consumed_drop_ale_route_ordinals,
            consumed_group_move_route_ordinals,
        );
        let (goal_sector, goal_sector_index) =
            entity_map.translate_required_drop_ale_goal_sector(event.goal_sector);
        routes.push(robin_engine::engine::RecordedDropAleRoute {
            actor,
            destination,
            goal_sector,
            goal_sector_index,
            goal_layer: event.goal_level,
            recorded_gate_path: recorded_gate_path_from_event(event, entity_map),
        });
    }
    routes
}

fn claim_delayed_drop_ale_route_ordinal(
    ordinal: u64,
    consumed_drop_ale_route_ordinals: &mut BTreeSet<u64>,
    consumed_group_move_route_ordinals: &BTreeSet<u64>,
) {
    assert!(
        !consumed_group_move_route_ordinals.contains(&ordinal),
        "schema-16 route ordinal {ordinal} was already consumed by the group-move join"
    );
    assert!(
        consumed_drop_ale_route_ordinals.insert(ordinal),
        "schema-16 delayed DropAle route ordinal {ordinal} matched twice"
    );
}

fn legacy_drop_ale_target_is_same_exact_sector(
    actor: robin_engine::position_interface::SectorHandle,
    target: robin_engine::position_interface::SectorHandle,
) -> bool {
    target.number() == actor.number()
        && target.arena_index().is_some()
        && target.arena_index() == actor.arena_index()
}

fn current_drop_ale_same_sector_goal(
    command: &TraceCommand,
    entity_map: &EntityMap,
    engine: &Engine,
) -> Option<ReplayDropAleResolution> {
    let TraceCommand::DropAleAt { actor, target, .. } = command else {
        return None;
    };
    let actor = entity_map.translate(*actor);
    let entity = engine
        .get_entity(actor)
        .unwrap_or_else(|| panic!("schema-16 DropAle actor {actor:?} is missing"));
    let element = entity.element_data();
    let sector = element
        .sector()
        .unwrap_or_else(|| panic!("schema-16 DropAle actor {actor:?} has no current sector"));
    let target_point: MapPoint = (*target).into();
    let grid = engine.fast_grid();
    let hit = grid.get_sector_screen(target_point, element.position_map());
    let target_sector = hit.sector_idx.and_then(|index| {
        let selected = grid
            .level
            .sectors
            .get(usize::from(index))
            .unwrap_or_else(|| panic!("schema-16 DropAle target sector {index} is missing"));
        if selected.sector_type.is_patch() || selected.sector_type.is_jump() {
            let underlying = selected.underlying_sector.unwrap_or_else(|| {
                panic!("schema-16 DropAle target overlay {index} has no underlying sector")
            });
            let underlying_sector = grid
                .level
                .sectors
                .get(usize::from(underlying))
                .unwrap_or_else(|| {
                    panic!(
                        "schema-16 DropAle target overlay {index} references missing sector {underlying}"
                    )
                });
            u16::try_from(underlying_sector.sector_number.get())
                .ok()
                .and_then(robin_engine::position_interface::SectorHandle::new)
                .map(|sector| sector.with_arena_index(underlying))
        } else {
            hit.sector_handle()
        }
    });
    let target_is_same_exact_sector = target_sector
        .is_some_and(|target| legacy_drop_ale_target_is_same_exact_sector(sector, target));
    if !target_is_same_exact_sector {
        // The old trace did not attest a gate path. Let DropAle's normal live
        // target/route resolver reconstruct it from the already-authorized
        // target point instead of inventing the actor sector as its goal.
        return None;
    }
    let public = i16::try_from(sector.get()).unwrap_or_else(|_| {
        panic!(
            "schema-16 DropAle actor {actor:?} sector {} exceeds its signed identity domain",
            sector.get()
        )
    });
    Some(ReplayDropAleResolution {
        goal: (SectorNumber::new(public), element.layer()),
        // Same-sector DropAle has no route event with which to recover a
        // stronger identity. Preserve the actor's current representation so
        // the seek compares equal whether it is exact or number-only.
        goal_sector_index: sector.arena_index(),
        // Same-sector DropAle never searches gate paths.
        recorded_gate_path: None,
    })
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
struct TracePointBox {
    top_left: TracePoint,
    bottom_right: TracePoint,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
struct TracePopupEvent {
    #[serde(default)]
    ordinal: Option<u64>,
    stage: String,
    #[serde(default)]
    universal_frame_counter: Option<u64>,
    #[serde(default)]
    last_popup_frame: Option<u64>,
    #[serde(default)]
    last_popup_frame_initialized: Option<bool>,
    #[serde(default)]
    same_frame_suppressed: Option<bool>,
    #[serde(default)]
    colorize_background: Option<bool>,
    #[serde(default)]
    modal: Option<bool>,
    #[serde(default)]
    centered: Option<bool>,
    #[serde(default)]
    popup_text_id: Option<u64>,
    #[serde(default)]
    source_surface: Option<u64>,
    #[serde(default)]
    remove_mouse: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
struct TraceForecastGate {
    gate_id: u32,
    kind: String,
    active: bool,
    point_out: TracePoint,
    sector_out: u16,
    level_out: u16,
    point_in: TracePoint,
    sector_in: u16,
    level_in: u16,
    penalty: TraceFloat,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
struct TraceAiForecastInput {
    position: TracePoint,
    sector: u16,
    level: u16,
    #[serde(default)]
    direction: Option<u16>,
    passing_door: bool,
    #[serde(default)]
    passing_door_directly: Option<bool>,
    #[serde(default)]
    door: Option<TraceForecastGate>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
struct TraceAiForecastResolved {
    position: TracePoint,
    sector: u16,
    level: u16,
    #[serde(default)]
    direction: Option<u16>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
struct TraceAiForecastEvent {
    ordinal: u64,
    #[serde(default)]
    phase: Option<String>,
    target: TraceEntityId,
    input: TraceAiForecastInput,
    moving_upwards: bool,
    resolution: String,
    resolved: TraceAiForecastResolved,
    #[serde(default)]
    selected_building_exit: Option<TraceForecastGate>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
struct TraceAlertEligibility {
    rank: bool,
    able_to_help: Option<bool>,
    allowed_to_leave_post: Option<bool>,
    can_call: Option<bool>,
    max_radius: Option<bool>,
    squared_radius: Option<bool>,
    capacity: Option<bool>,
    think: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
struct TraceAlertFormationEvent {
    #[serde(default)]
    ordinal: Option<u64>,
    stage: String,
    invocation: u64,
    #[serde(default)]
    officer: Option<TraceEntityId>,
    #[serde(default)]
    officer_position: Option<TracePoint>,
    #[serde(default)]
    soldier_scan_count: Option<u16>,
    #[serde(default)]
    officer_in_building: Option<bool>,
    #[serde(default)]
    scan_index: Option<u16>,
    #[serde(default)]
    candidate: Option<TraceEntityId>,
    #[serde(default)]
    active: Option<bool>,
    #[serde(default)]
    script_locked: Option<bool>,
    #[serde(default)]
    eligibility: Option<TraceAlertEligibility>,
    #[serde(default)]
    rejection_stage: Option<String>,
    #[serde(default)]
    insertion_index: Option<u16>,
    #[serde(default)]
    squared_distance: Option<TraceFloat>,
    #[serde(default)]
    normalized_contribution: Option<TracePoint>,
    #[serde(default)]
    running_average: Option<TracePoint>,
    #[serde(default)]
    selected_index: Option<u16>,
    #[serde(default)]
    outside_step: Option<u16>,
    #[serde(default)]
    direction: Option<u16>,
    #[serde(default)]
    soldier_count: Option<u16>,
    #[serde(default)]
    slot_index: Option<u16>,
    #[serde(default)]
    layer: Option<u16>,
    #[serde(default)]
    sector: Option<u16>,
    #[serde(default)]
    destination: Option<TracePoint>,
    #[serde(default)]
    destination_box: Option<TracePointBox>,
    #[serde(default)]
    position_authorized: Option<bool>,
    #[serde(default)]
    thick_corridor_authorized: Option<bool>,
    #[serde(default)]
    blocker_ids_available: Option<bool>,
    #[serde(default)]
    blocking_motion_line_ids: Option<Vec<u32>>,
    #[serde(default)]
    blocking_mobile_line_ids: Option<Vec<u32>>,
    #[serde(default)]
    accepted: Option<bool>,
    #[serde(default)]
    result: Option<String>,
    #[serde(default)]
    average_direction: Option<u16>,
    #[serde(default)]
    selected_direction: Option<u16>,
    #[serde(default)]
    final_sector: Option<u16>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
struct TraceGoToSource {
    point: TracePoint,
    layer: u16,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
struct TraceGoToDestination {
    point: TracePoint,
    sector: Option<u16>,
    layer: u16,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
struct TraceGoToAuthorizationEvent {
    ordinal: u64,
    actor: Option<TraceEntityId>,
    #[serde(default)]
    source: Option<TraceGoToSource>,
    #[serde(default)]
    move_box: Option<TracePointBox>,
    destination: TraceGoToDestination,
    requested_flags: u16,
    effective_flags: u16,
    phase: String,
    outcome: String,
    straight_authorized: Option<bool>,
    path_authorized: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum TraceTargetLifecyclePayload {
    ActivateSword,
    PlayAnimFreeze {
        animation_id: Option<u32>,
    },
    SendMessage {
        message: Option<u32>,
        argument: Option<i32>,
        argument_raw: Option<u32>,
        extended_argument: Option<i32>,
        extended_argument_raw: Option<u32>,
    },
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
struct TraceTargetLifecycleEvent {
    ordinal: u64,
    #[serde(default)]
    frame_ordinal: Option<u64>,
    phase: String,
    sequence_id: Option<u32>,
    sequence_element_id: u32,
    command_level: u16,
    state: u32,
    command: u16,
    command_name: String,
    owner: Option<TraceEntityId>,
    context: Option<TraceEntityId>,
    antagonist: Option<TraceEntityId>,
    #[serde(default)]
    antagonist_observed: Option<bool>,
    payload: TraceTargetLifecyclePayload,
    #[serde(default)]
    payload_observed: Option<bool>,
    script_enabled: Option<bool>,
    class_instantiated: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
struct TraceStrikeProposalEvent {
    invocation: u32,
    ordinal: u64,
    frame_ordinal: u64,
    phase: String,
    actor: Option<TraceEntityId>,
    actor_creation_order: u32,
    threat: Option<TraceEntityId>,
    threat_creation_order: Option<u32>,
    principal_opponent: Option<TraceEntityId>,
    principal_opponent_creation_order: Option<u32>,
    command: Option<u16>,
    command_name: Option<String>,
    also_parade: Option<bool>,
    only_parade: Option<bool>,
    fighting_ability: Option<u16>,
    blood_alcohol: Option<u16>,
    special_gate_fighting_ability: Option<u16>,
    parade_gate_fighting_ability: Option<u16>,
    first_random_raw: Option<u32>,
    first_random_modulo: Option<u32>,
    second_random_raw: Option<u32>,
    second_random_modulo: Option<u32>,
    reason: Option<String>,
    opponent_animation: Option<u32>,
    opponent_strike: Option<i32>,
    candidate_strike: Option<i32>,
    time_limit: Option<i32>,
    minimum_skill: Option<u16>,
    maximum_alcohol: Option<u16>,
    boredom_before: Option<u16>,
    boredom_after_decay: Option<u16>,
    skill_eligible: Option<bool>,
    alcohol_eligible: Option<bool>,
    time_eligible: Option<bool>,
    raw_damage: Option<i32>,
    victim_count: Option<i32>,
    boredom_penalty: Option<i32>,
    drunken_bonus: Option<i32>,
    adjusted_damage: Option<i32>,
    group_strike: Option<bool>,
    group_condition: Option<bool>,
    accepted_as_best: Option<bool>,
    selected_strike: Option<i32>,
    parry_transition_frames: Option<i32>,
    parry_time_eligible: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
struct TraceSequenceLifecycleEvent {
    ordinal: u64,
    frame_ordinal: u64,
    event: String,
    phase: String,
    element_id: u32,
    sequence_id: Option<u32>,
    owner: Option<TraceEntityId>,
    owner_creation_order: Option<u32>,
    command: u16,
    command_name: Option<String>,
    command_level: u16,
    state: Option<u32>,
    priority: Option<u32>,
    queue_size_before: Option<u32>,
    queue_size_after: Option<u32>,
    actor: Option<TraceEntityId>,
    actor_creation_order: Option<u32>,
    selected_sequence_id: Option<u32>,
    selected_command: Option<u16>,
    current_order_id: Option<u32>,
    current_order_action: Option<u32>,
    decision: Option<i32>,
    accepted: Option<bool>,
}

/// Frame layout embedded in version-68 native records.
///
/// ON-DISK FORMAT INVARIANT: any bitcode-shape change requires a native
/// version bump plus a frozen compatibility decoder for this layout.
#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
struct TraceFrame {
    #[serde(rename = "type")]
    record_type: String,
    frame_before: u64,
    frame_after: u64,
    game_code: i32,
    simulation_body_ran: bool,
    commands: Vec<TraceCommand>,
    director_completions: Vec<TraceDirectorCompletion>,
    selected_pcs: Vec<TraceEntityId>,
    elements: Vec<TraceElement>,
    visibility_queries: Vec<TraceVisibilityQuery>,
    rng_draws: TraceRngBatch,
    motion_line_changes: Vec<TraceMotionLineChange>,
    path_events: Vec<TracePathEvent>,
    /// Retained and printed on divergence; Rust has no
    /// side-effect-free route-construction event capture yet.
    /// TODO(parity-schema): compare once the sequence builders publish the
    /// same source/goal and ordered gate list.
    route_construction_events: Vec<TraceRouteConstructionEvent>,
    popup_events: Vec<TracePopupEvent>,
    ai_forecast_events: Vec<TraceAiForecastEvent>,
    alert_formation_events: Vec<TraceAlertFormationEvent>,
    goto_authorization_events: Vec<TraceGoToAuthorizationEvent>,
    strike_proposal_events: Vec<TraceStrikeProposalEvent>,
    sequence_lifecycle_events: Vec<TraceSequenceLifecycleEvent>,
    target_lifecycle_events: Vec<TraceTargetLifecycleEvent>,
    resolved_exclamations: Vec<TraceResolvedExclamation>,
    /// Optional diagnostics in early schema-16 recordings. They are retained
    /// whenever present and default only at the JSON-to-native compatibility
    /// boundary; logical state comparison never invents recorded operands.
    #[serde(default)]
    movement_steps: Vec<TraceMovementStep>,
    #[serde(default)]
    flight_steps: Vec<TraceFlightStep>,
}

/// Frame layout embedded in native trace version 67. Its element type retains
/// the original plain-`bool` `increment_map_valid` field.
#[derive(Debug, bitcode::Encode, bitcode::Decode)]
struct TraceFrameV67 {
    record_type: String,
    frame_before: u64,
    frame_after: u64,
    game_code: i32,
    simulation_body_ran: bool,
    commands: Vec<TraceCommand>,
    director_completions: Vec<TraceDirectorCompletion>,
    selected_pcs: Vec<TraceEntityId>,
    elements: Vec<TraceElementV67>,
    visibility_queries: Vec<TraceVisibilityQuery>,
    rng_draws: TraceRngBatch,
    motion_line_changes: Vec<TraceMotionLineChange>,
    path_events: Vec<TracePathEvent>,
    route_construction_events: Vec<TraceRouteConstructionEvent>,
    popup_events: Vec<TracePopupEvent>,
    ai_forecast_events: Vec<TraceAiForecastEvent>,
    alert_formation_events: Vec<TraceAlertFormationEvent>,
    goto_authorization_events: Vec<TraceGoToAuthorizationEvent>,
    strike_proposal_events: Vec<TraceStrikeProposalEvent>,
    sequence_lifecycle_events: Vec<TraceSequenceLifecycleEvent>,
    target_lifecycle_events: Vec<TraceTargetLifecycleEvent>,
    resolved_exclamations: Vec<TraceResolvedExclamation>,
    movement_steps: Vec<TraceMovementStep>,
    flight_steps: Vec<TraceFlightStep>,
}

/// Frame layout embedded in native trace version 66.
#[derive(Debug, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceFrameV66 {
    #[serde(rename = "type")]
    record_type: String,
    frame_before: u64,
    frame_after: u64,
    game_code: i32,
    simulation_body_ran: bool,
    commands: Vec<TraceCommandV66>,
    director_completions: Vec<TraceDirectorCompletion>,
    campaign: Option<TraceCampaign>,
    engine_state: Option<TraceEngineStateV66>,
    selected_pcs: Vec<TraceEntityId>,
    elements: Vec<TraceElementV66>,
    visibility_queries: Vec<TraceVisibilityQuery>,
    rng_draws: TraceRngBatch,
    motion_line_changes: Vec<TraceMotionLineChange>,
    path_events: Vec<TracePathEvent>,
    route_construction_events: Option<Vec<TraceRouteConstructionEvent>>,
    popup_events: Option<Vec<TracePopupEvent>>,
    ai_forecast_events: Option<Vec<TraceAiForecastEvent>>,
    alert_formation_events: Option<Vec<TraceAlertFormationEvent>>,
    goto_authorization_events: Option<Vec<TraceGoToAuthorizationEvent>>,
    strike_proposal_events: Option<Vec<TraceStrikeProposalEvent>>,
    sequence_lifecycle_events: Option<Vec<TraceSequenceLifecycleEvent>>,
    target_lifecycle_events: Option<Vec<TraceTargetLifecycleEvent>>,
    resolved_exclamations: Vec<TraceResolvedExclamation>,
    movement_steps: Vec<TraceMovementStep>,
    flight_steps: Vec<TraceFlightStep>,
}

#[derive(Debug, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceEngineStateV66 {
    cheat_used_flags: u32,
    next_creation_order: u32,
    chorus_timer: u16,
    force_check: bool,
    men_to_blazon_conversion: bool,
    game_ui: Option<TraceJsonValue>,
    messenger_controller: Option<TraceJsonValue>,
    shield_controller: Option<TraceJsonValue>,
    pc_registry: TraceJsonValue,
    lock_engine: bool,
    freeze_all: bool,
    locker: bool,
    speed: TraceFloat,
    speed_int: u16,
    mission_won: bool,
    mission_won_first_time: bool,
    quit_won: bool,
    quit_lost: bool,
    quit_interrupted: bool,
    script_globals: Vec<i32>,
    sequence_manager: TraceJsonValue,
    script_runtime: TraceJsonValue,
    pathfinder: TraceJsonValue,
    view_radius_cache: TraceJsonValue,
    sound_sources: TraceJsonValue,
    sound_completion_frontier: Option<TraceJsonValue>,
    ai_global: TraceJsonValue,
    engine_runtime_roots: TraceJsonValue,
    world_interactables: TraceJsonValue,
    repulsive_points: TraceJsonValue,
    titbit_manager: TraceJsonValue,
    failed_path_requests: Vec<TraceFailedPathRequestV66>,
}

#[derive(Debug, Serialize, bitcode::Encode, bitcode::Decode)]
struct TraceFailedPathRequestV66 {
    actor: TraceEntityId,
    antagonist: Option<TraceEntityId>,
    layer: u16,
    area: u16,
    source: TracePoint,
    goal: TracePoint,
    half_diagonal_index: u16,
    half_diagonal: TracePoint,
    animation: u32,
    reverse: bool,
    speed: u8,
    tolerance: TraceFloat,
    use_first_point: bool,
    sector: u16,
    time: u32,
}

impl TraceFrameV66 {
    fn into_current(self) -> TraceFrame {
        TraceFrame {
            record_type: self.record_type,
            frame_before: self.frame_before,
            frame_after: self.frame_after,
            game_code: self.game_code,
            simulation_body_ran: self.simulation_body_ran,
            commands: self
                .commands
                .into_iter()
                .map(TraceCommandV66::into_current)
                .collect(),
            director_completions: self.director_completions,
            selected_pcs: self.selected_pcs,
            elements: self
                .elements
                .into_iter()
                .map(TraceElementV66::into_current)
                .collect(),
            visibility_queries: self.visibility_queries,
            rng_draws: self.rng_draws,
            motion_line_changes: self.motion_line_changes,
            path_events: self.path_events,
            route_construction_events: self.route_construction_events.unwrap_or_default(),
            popup_events: self.popup_events.unwrap_or_default(),
            ai_forecast_events: self.ai_forecast_events.unwrap_or_default(),
            alert_formation_events: self.alert_formation_events.unwrap_or_default(),
            goto_authorization_events: self.goto_authorization_events.unwrap_or_default(),
            strike_proposal_events: self.strike_proposal_events.unwrap_or_default(),
            sequence_lifecycle_events: self.sequence_lifecycle_events.unwrap_or_default(),
            target_lifecycle_events: self.target_lifecycle_events.unwrap_or_default(),
            resolved_exclamations: self.resolved_exclamations,
            movement_steps: self.movement_steps,
            flight_steps: self.flight_steps,
        }
    }
}

impl TraceFrameV67 {
    fn into_current(self, increment_map_valid_was_recorded: bool) -> TraceFrame {
        TraceFrame {
            record_type: self.record_type,
            frame_before: self.frame_before,
            frame_after: self.frame_after,
            game_code: self.game_code,
            simulation_body_ran: self.simulation_body_ran,
            commands: self.commands,
            director_completions: self.director_completions,
            selected_pcs: self.selected_pcs,
            elements: self
                .elements
                .into_iter()
                .map(|element| element.into_current(increment_map_valid_was_recorded))
                .collect(),
            visibility_queries: self.visibility_queries,
            rng_draws: self.rng_draws,
            motion_line_changes: self.motion_line_changes,
            path_events: self.path_events,
            route_construction_events: self.route_construction_events,
            popup_events: self.popup_events,
            ai_forecast_events: self.ai_forecast_events,
            alert_formation_events: self.alert_formation_events,
            goto_authorization_events: self.goto_authorization_events,
            strike_proposal_events: self.strike_proposal_events,
            sequence_lifecycle_events: self.sequence_lifecycle_events,
            target_lifecycle_events: self.target_lifecycle_events,
            resolved_exclamations: self.resolved_exclamations,
            movement_steps: self.movement_steps,
            flight_steps: self.flight_steps,
        }
    }
}

/// Recursive JSON tree used for high-volume trace snapshots.
///
/// `serde_json::Value` deliberately has no native binary-codec derives. This
/// equivalent tree keeps frame parsing strict; it is the serde
/// (JSONL) view of [`TraceJsonValue`], which stores the same data flat.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
enum TraceJsonTree {
    Null(()),
    Bool(bool),
    Unsigned(u64),
    Signed(i64),
    Float(f64),
    String(String),
    Array(Vec<TraceJsonTree>),
    Object(BTreeMap<String, TraceJsonTree>),
}

impl TraceJsonTree {
    fn to_json(&self) -> serde_json::Value {
        match self {
            Self::Null(()) => serde_json::Value::Null,
            Self::Bool(value) => (*value).into(),
            Self::Unsigned(value) => (*value).into(),
            Self::Signed(value) => (*value).into(),
            Self::Float(value) => (*value).into(),
            Self::String(value) => value.clone().into(),
            Self::Array(values) => values.iter().map(Self::to_json).collect(),
            Self::Object(values) => values
                .iter()
                .map(|(key, value)| (key.clone(), value.to_json()))
                .collect(),
        }
    }

    fn flatten_into(&self, tokens: &mut Vec<TraceJsonToken>) {
        match self {
            Self::Null(()) => tokens.push(TraceJsonToken::Null),
            Self::Bool(value) => tokens.push(TraceJsonToken::Bool(*value)),
            Self::Unsigned(value) => tokens.push(TraceJsonToken::Unsigned(*value)),
            Self::Signed(value) => tokens.push(TraceJsonToken::Signed(*value)),
            Self::Float(value) => tokens.push(TraceJsonToken::Float(*value)),
            Self::String(value) => tokens.push(TraceJsonToken::String(value.clone())),
            Self::Array(values) => {
                tokens.push(TraceJsonToken::Array(
                    u32::try_from(values.len()).expect("JSON array length exceeds u32"),
                ));
                for value in values {
                    value.flatten_into(tokens);
                }
            }
            Self::Object(values) => {
                tokens.push(TraceJsonToken::Object(
                    u32::try_from(values.len()).expect("JSON object length exceeds u32"),
                ));
                for (key, value) in values {
                    tokens.push(TraceJsonToken::Key(key.clone()));
                    value.flatten_into(tokens);
                }
            }
        }
    }

    fn unflatten(tokens: &mut std::slice::Iter<'_, TraceJsonToken>) -> Self {
        match tokens.next().expect("flat JSON token stream ended early") {
            TraceJsonToken::Null => Self::Null(()),
            TraceJsonToken::Bool(value) => Self::Bool(*value),
            TraceJsonToken::Unsigned(value) => Self::Unsigned(*value),
            TraceJsonToken::Signed(value) => Self::Signed(*value),
            TraceJsonToken::Float(value) => Self::Float(*value),
            TraceJsonToken::String(value) => Self::String(value.clone()),
            TraceJsonToken::Array(len) => {
                Self::Array((0..*len).map(|_| Self::unflatten(tokens)).collect())
            }
            TraceJsonToken::Object(len) => Self::Object(
                (0..*len)
                    .map(|_| {
                        let TraceJsonToken::Key(key) = tokens
                            .next()
                            .expect("flat JSON object ended before its key")
                        else {
                            panic!("flat JSON object entry does not start with a key")
                        };
                        (key.clone(), Self::unflatten(tokens))
                    })
                    .collect(),
            ),
            TraceJsonToken::Key(key) => {
                panic!("unexpected flat JSON key {key:?} in value position")
            }
        }
    }
}

/// One pre-order token of a flattened [`TraceJsonTree`]. `Array`/`Object`
/// carry their child count; object entries are `Key` followed by a value.
#[derive(Clone, Debug, PartialEq, bitcode::Decode, bitcode::Encode)]
enum TraceJsonToken {
    Null,
    Bool(bool),
    Unsigned(u64),
    Signed(i64),
    Float(f64),
    String(String),
    Array(u32),
    Object(u32),
    Key(String),
}

/// Cache-safe JSON value: a [`TraceJsonTree`] stored as a flat pre-order
/// token list. bitcode's derives cannot encode recursive types (the derived
/// encoder would be infinitely sized), so the binary codecs see a plain
/// `Vec<TraceJsonToken>` while serde still reads and writes the JSON shape.
#[derive(Clone, Debug, PartialEq, bitcode::Decode, bitcode::Encode)]
struct TraceJsonValue {
    tokens: Vec<TraceJsonToken>,
}

impl TraceJsonValue {
    fn tree(&self) -> TraceJsonTree {
        let mut tokens = self.tokens.iter();
        let tree = TraceJsonTree::unflatten(&mut tokens);
        assert!(
            tokens.next().is_none(),
            "flat JSON token stream has trailing tokens"
        );
        tree
    }

    fn to_json(&self) -> serde_json::Value {
        self.tree().to_json()
    }
}

impl From<TraceJsonTree> for TraceJsonValue {
    fn from(tree: TraceJsonTree) -> Self {
        let mut tokens = Vec::new();
        tree.flatten_into(&mut tokens);
        Self { tokens }
    }
}

fn missing_legacy_trace_json_value() -> TraceJsonValue {
    TraceJsonTree::Null(()).into()
}

impl Serialize for TraceJsonValue {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.tree().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for TraceJsonValue {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        TraceJsonTree::deserialize(deserializer).map(Self::from)
    }
}

fn print_current_trace_events<T: Serialize>(label: &str, events: &[T]) {
    if !events.is_empty() {
        eprintln!(
            "  Original schema-{TRACE_SCHEMA_VERSION} {label} this frame: {}",
            serde_json::to_string(events).expect("serialize current-schema event diagnostics")
        );
    }
}

fn print_current_trace_actor_diagnostics(elements: &[TraceElement]) {
    let diagnostics = elements
        .iter()
        .filter_map(|element| {
            let actor = element.actor.as_ref()?;
            let sequence = actor.sequence_element.as_ref();
            Some(serde_json::json!({
                "entity": element.entity_id,
                "creation_order": element.creation_order,
                "position_interface": actor.position_interface,
                "following": sequence.and_then(|value| value.following.as_ref()),
                "postponed": sequence.and_then(|value| value.postponed.as_ref()),
                "current_order": sequence.and_then(|value| value.current_order.as_ref()),
                "movement_payload": sequence.and_then(|value| value.movement_payload.as_ref()),
            }))
        })
        .take(40)
        .collect::<Vec<_>>();
    if !diagnostics.is_empty() {
        eprintln!(
            "  Original schema-{TRACE_SCHEMA_VERSION} actor diagnostics (up to 40): {}",
            serde_json::to_string(&diagnostics)
                .expect("serialize current-schema actor diagnostics")
        );
    }
}

fn validate_jump_line_shapes(frame: &TraceFrame, legacy_additive_omissions: bool) {
    for element in &frame.elements {
        let Some(human) = element.human.as_ref() else {
            continue;
        };
        validate_human_jump_line_shape(&element.entity_id, human, legacy_additive_omissions);
    }
}

fn validate_human_jump_line_shape(
    entity: &TraceEntityId,
    human: &TraceHuman,
    legacy_additive_omissions: bool,
) {
    // The opponent jump-line diagnostic was added after the first schema-16
    // recordings. Its v66 compatibility representation preserves absence as
    // `None`, but the original v66 -> current migration flattened that to an
    // empty vector before archival reblocking. Such a vector means "not
    // recorded" only when the header also lacks `initial_npc_transients`,
    // which was introduced later than the jump-line diagnostic. Do not invent
    // null jump lines: comparison already skips all unavailable additive
    // human fields for that legacy header generation. Modern recordings, and
    // legacy recordings carrying any jump-line entry, remain strict.
    //
    // Original emits exactly one JumpLineState slot per opponent
    // including JSON null for no line.
    if legacy_additive_omissions && human.opponent_jump_lines.is_empty() {
        return;
    }
    assert_eq!(
        human.opponents.len(),
        human.opponent_jump_lines.len(),
        "schema-{TRACE_SCHEMA_VERSION} {entity:?} opponent and jump-line arrays differ in length"
    );
}

fn validate_sequence_diagnostic_order(frame: &TraceFrame) {
    let proposals = &frame.strike_proposal_events;
    let lifecycle = &frame.sequence_lifecycle_events;
    for (expected, event) in proposals.iter().enumerate() {
        assert_eq!(
            event.ordinal, expected as u64,
            "schema-{TRACE_SCHEMA_VERSION} strike-proposal event ordinals are not contiguous"
        );
    }
    for (expected, event) in lifecycle.iter().enumerate() {
        assert_eq!(
            event.ordinal, expected as u64,
            "schema-{TRACE_SCHEMA_VERSION} sequence-lifecycle event ordinals are not contiguous"
        );
    }
    for (expected, event) in frame.target_lifecycle_events.iter().enumerate() {
        assert_eq!(
            event.ordinal, expected as u64,
            "schema-{TRACE_SCHEMA_VERSION} target-lifecycle event ordinals are not contiguous"
        );
    }

    let mut frame_ordinals = proposals
        .iter()
        .map(|event| event.frame_ordinal)
        .chain(lifecycle.iter().map(|event| event.frame_ordinal))
        .chain(
            frame
                .target_lifecycle_events
                .iter()
                .filter_map(|event| event.frame_ordinal),
        )
        .collect::<Vec<_>>();
    frame_ordinals.sort_unstable();
    assert_eq!(
        frame_ordinals,
        (0..frame_ordinals.len() as u64).collect::<Vec<_>>(),
        "schema-{TRACE_SCHEMA_VERSION} strike/sequence/target frame ordinals are not a single contiguous timeline"
    );

    let mut invocation_state = BTreeMap::<u32, (bool, bool)>::new();
    for event in proposals {
        let state = invocation_state.entry(event.invocation).or_default();
        assert!(!state.1, "strike-proposal event follows its result");
        if event.phase == "entry" {
            assert!(!state.0, "strike-proposal invocation has duplicate entry");
            state.0 = true;
        } else {
            assert!(state.0, "strike-proposal event precedes its entry");
        }
        if event.phase == "result" {
            state.1 = true;
        }
    }
    for (invocation, (started, finished)) in invocation_state {
        assert!(
            started,
            "strike-proposal invocation {invocation} has no entry"
        );
        assert!(
            finished,
            "strike-proposal invocation {invocation} has no result"
        );
    }
}

#[cfg(test)]
fn validate_trace_frame(schema: u32, frame: &TraceFrame) {
    validate_trace_frame_with_legacy_additive_omissions(schema, frame, false);
}

fn validate_trace_frame_with_legacy_additive_omissions(
    schema: u32,
    frame: &TraceFrame,
    legacy_additive_omissions: bool,
) {
    // Opponent jump-line slots were added with schema 16. Schemas 12-15
    // legitimately decode the absent additive array as empty.
    if schema >= 16 {
        validate_jump_line_shapes(frame, legacy_additive_omissions);
    }
    validate_sequence_diagnostic_order(frame);
}

const TRACE_NATIVE_VERSION: u32 = 68;
const TRACE_NATIVE_LEGACY_VERSION: u32 = 67;
const TRACE_NATIVE_V66_VERSION: u32 = 66;
/// The native parity trace is the authoritative artifact once its JSONL
/// source has been converted (and possibly deleted), so its name carries no
/// version: compatibility is enforced through the versioned header/footer,
/// and an incompatible file must be migrated, never silently regenerated.
/// The suffix appends to the full recording name (`X.jsonl.zst` becomes
/// `X.jsonl.zst.parity.bitcode.zst`) because the `.jsonl.zst` path is the
/// stable trace identity used by sweep status keys, EOF ledgers, and
/// completion markers.
const TRACE_NATIVE_SUFFIX: &str = ".parity.bitcode.zst";
const TRACE_CONVERSION_QUARANTINE_SUFFIX: &str = ".parity-conversion-source";
const TRACE_REBLOCK_SOURCE_SUFFIX: &str = ".parity-reblock-source-v67";
const TRACE_REBLOCK_BINDING_SUFFIX: &str = ".parity-reblock-binding-v67.json";
const TRACE_NATIVE_FOOTER_MAGIC: [u8; 16] = *b"RHPRTRACEFOOTER!";
const TRACE_NATIVE_FOOTER_LEN: u64 = 16 + 4 + 8 + 8;
// Full-session JSONL recordings are compressed as a single zstd frame. Some
// encoders select a frame window from the total uncompressed size, so long
// recordings legitimately exceed zstd's conservative 128 MiB decoder default.
// Keep the reader bounded at zstd's platform maximum while accepting those
// valid trace frames.
const TRACE_ZSTD_WINDOW_LOG_MAX: u32 = if usize::BITS >= 64 { 31 } else { 30 };
// Prefer maximum archival density for parity recordings. A representative
// min/median/max corpus benchmark made level 19 16-19% smaller than level 9,
// at the cost of substantially slower conversion. Long-distance matching was
// neutral at level 19, so the native writer deliberately leaves it disabled.
// Rechecked with the production 512 MiB window on interactive session 21
// (11,594 frames, 8,028.68 MiB raw bitcode): LDM off and on both rounded to
// 18.78 MiB (less than 0.01 MiB apart). The single-pass timings, which also
// included native decode and bitcode encode, were 233.75s off and 225.13s on;
// that is not a compression-density reason to pay LDM's extra working state.
const TRACE_NATIVE_ZSTD_LEVEL: i32 = 19;
const TRACE_NATIVE_LONG_DISTANCE_MATCHING: bool = false;
/// Frames per newly-written on-disk block. A current-schema frame contains a
/// complete Original state envelope, so the former 1,000-record policy could
/// require 2-3.6 GiB of live Rust allocations. Readers have always accepted any
/// non-empty block size, so this storage-only change remains compatible with
/// every existing version-68 reader and does not change the wire layout.
const TRACE_NATIVE_BLOCK_RECORDS: usize = 32;
/// Bound the zstd history retained by every replay process. Cross-frame
/// repetition is already captured inside bitcode blocks. A 64 MiB history keeps
/// replay lanes bounded while preserving useful cross-block compression.
const TRACE_NATIVE_WINDOW_LOG: u32 = 26;
const TRACE_NATIVE_MIN_WINDOW_LOG: u32 = 20;
const TRACE_NATIVE_MAX_REBLOCK_WINDOW_LOG: u32 = 29;
const TRACE_NATIVE_MAX_REBLOCK_RECORDS: usize = 1000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NativeStoragePolicy {
    block_records: usize,
    window_log: u32,
}

impl Default for NativeStoragePolicy {
    fn default() -> Self {
        Self {
            block_records: TRACE_NATIVE_BLOCK_RECORDS,
            window_log: TRACE_NATIVE_WINDOW_LOG,
        }
    }
}

impl NativeStoragePolicy {
    fn new(block_records: usize, window_log: u32) -> Self {
        assert!(
            (1..=TRACE_NATIVE_MAX_REBLOCK_RECORDS).contains(&block_records),
            "--reblock-records must be between 1 and {TRACE_NATIVE_MAX_REBLOCK_RECORDS}"
        );
        assert!(
            (TRACE_NATIVE_MIN_WINDOW_LOG..=TRACE_NATIVE_MAX_REBLOCK_WINDOW_LOG)
                .contains(&window_log),
            "--reblock-window-log must be between {TRACE_NATIVE_MIN_WINDOW_LOG} and {TRACE_NATIVE_MAX_REBLOCK_WINDOW_LOG}"
        );
        Self {
            block_records,
            window_log,
        }
    }
}

/// Native trace header layout for version 68. Do not change its bitcode shape
/// without bumping `TRACE_NATIVE_VERSION` and retaining this type as the v68
/// compatibility decoder.
#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
struct BinaryTraceHeaderV68 {
    version: u32,
    source_fingerprint: String,
    trace: TraceHeader,
    rng_prefix: TraceRngPrefix,
}

#[derive(Debug, bitcode::Encode, bitcode::Decode)]
struct BinaryTraceHeaderV67 {
    version: u32,
    source_fingerprint: String,
    trace: TraceHeaderV67,
    rng_prefix: TraceRngPrefix,
}

/// Late version-67 header layout written after `1a932c148` changed
/// `initial_npc_transients` back to an `Option<Vec<_>>` without bumping the
/// native format version. A small retained interactive corpus was converted
/// during that window. Keep this separate from both the original v67 layout
/// and the identically-shaped v68 header so the accidental wire generation is
/// explicit and cannot silently drift again.
#[derive(Debug, bitcode::Encode, bitcode::Decode)]
struct BinaryTraceHeaderV67Late {
    version: u32,
    source_fingerprint: String,
    trace: TraceHeaderV67Late,
    rng_prefix: TraceRngPrefix,
}

#[derive(Debug, bitcode::Encode, bitcode::Decode)]
struct BinaryTraceHeaderV66 {
    version: u32,
    source_fingerprint: String,
    trace: TraceHeaderV66,
    rng_prefix: TraceRngPrefix,
}

impl From<BinaryTraceHeaderV66> for BinaryTraceHeaderV68 {
    fn from(header: BinaryTraceHeaderV66) -> Self {
        Self {
            version: header.version,
            source_fingerprint: header.source_fingerprint,
            trace: header.trace.into(),
            rng_prefix: header.rng_prefix,
        }
    }
}

#[cfg(test)]
impl From<BinaryTraceHeaderV68> for BinaryTraceHeaderV66 {
    fn from(header: BinaryTraceHeaderV68) -> Self {
        Self {
            version: TRACE_NATIVE_V66_VERSION,
            source_fingerprint: header.source_fingerprint,
            trace: header.trace.into(),
            rng_prefix: header.rng_prefix,
        }
    }
}

impl From<BinaryTraceHeaderV67> for BinaryTraceHeaderV68 {
    fn from(header: BinaryTraceHeaderV67) -> Self {
        Self {
            version: header.version,
            source_fingerprint: header.source_fingerprint,
            trace: header.trace.into(),
            rng_prefix: header.rng_prefix,
        }
    }
}

impl From<BinaryTraceHeaderV67Late> for BinaryTraceHeaderV68 {
    fn from(header: BinaryTraceHeaderV67Late) -> Self {
        Self {
            version: header.version,
            source_fingerprint: header.source_fingerprint,
            trace: header.trace.into(),
            rng_prefix: header.rng_prefix,
        }
    }
}

#[cfg(test)]
impl From<BinaryTraceHeaderV68> for BinaryTraceHeaderV67Late {
    fn from(header: BinaryTraceHeaderV68) -> Self {
        Self {
            version: TRACE_NATIVE_LEGACY_VERSION,
            source_fingerprint: header.source_fingerprint,
            trace: header.trace.into(),
            rng_prefix: header.rng_prefix,
        }
    }
}

#[cfg(test)]
impl From<BinaryTraceHeaderV68> for BinaryTraceHeaderV67 {
    fn from(header: BinaryTraceHeaderV68) -> Self {
        Self {
            version: TRACE_NATIVE_LEGACY_VERSION,
            source_fingerprint: header.source_fingerprint,
            trace: header.trace.into(),
            rng_prefix: header.rng_prefix,
        }
    }
}

/// Native record layout for version 68.
///
/// ON-DISK FORMAT INVARIANT: this enum and every transitively encoded child
/// type are immutable for version 68. Shape changes require a version bump and
/// an explicit legacy decoder such as [`BinaryTraceRecordV67`].
#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
enum BinaryTraceRecord {
    Frame(TraceFrame),
    End {
        rng_suffix: Option<TraceRngBatch>,
        final_frame: Option<u64>,
        frame_count: Option<u64>,
    },
}

/// Record layout written by native trace version 67. Do not modify this type
/// or any of its versioned children.
#[derive(Debug, bitcode::Encode, bitcode::Decode)]
enum BinaryTraceRecordV67 {
    Frame(TraceFrameV67),
    End {
        rng_suffix: Option<TraceRngBatch>,
        final_frame: Option<u64>,
        frame_count: Option<u64>,
    },
}

/// Record envelope written during the same accidental late-v67 window as
/// [`BinaryTraceHeaderV67Late`]. Its frame uses the then-current optional
/// `increment_map_valid` representation rather than [`TraceFrameV67`]'s bool.
#[derive(Debug, bitcode::Encode, bitcode::Decode)]
enum BinaryTraceRecordV67Late {
    Frame(TraceFrame),
    End {
        rng_suffix: Option<TraceRngBatch>,
        final_frame: Option<u64>,
        frame_count: Option<u64>,
    },
}

#[derive(Debug, bitcode::Encode, bitcode::Decode)]
enum BinaryTraceRecordV66 {
    Frame(TraceFrameV66),
    End {
        rng_suffix: Option<TraceRngBatch>,
        final_frame: Option<u64>,
        frame_count: Option<u64>,
    },
}

impl BinaryTraceRecordV66 {
    fn into_current(self) -> BinaryTraceRecord {
        match self {
            Self::Frame(frame) => BinaryTraceRecord::Frame(frame.into_current()),
            Self::End {
                rng_suffix,
                final_frame,
                frame_count,
            } => BinaryTraceRecord::End {
                rng_suffix,
                final_frame,
                frame_count,
            },
        }
    }
}

impl BinaryTraceRecordV67 {
    fn into_current(self, increment_map_valid_was_recorded: bool) -> BinaryTraceRecord {
        match self {
            Self::Frame(frame) => {
                BinaryTraceRecord::Frame(frame.into_current(increment_map_valid_was_recorded))
            }
            Self::End {
                rng_suffix,
                final_frame,
                frame_count,
            } => BinaryTraceRecord::End {
                rng_suffix,
                final_frame,
                frame_count,
            },
        }
    }
}

impl BinaryTraceRecordV67Late {
    fn into_current(self) -> BinaryTraceRecord {
        match self {
            Self::Frame(frame) => BinaryTraceRecord::Frame(frame),
            Self::End {
                rng_suffix,
                final_frame,
                frame_count,
            } => BinaryTraceRecord::End {
                rng_suffix,
                final_frame,
                frame_count,
            },
        }
    }
}

struct BinaryTraceReader {
    path: PathBuf,
    reader: Box<dyn Read>,
    footer: BinaryTraceFooter,
    /// Records of the current block not yet handed out by [`Self::read_record`].
    pending: VecDeque<BinaryTraceRecord>,
    /// V67 used an empty header transient vector to identify recorder builds
    /// where `increment_map_valid` was absent and defaulted to false.
    v67_increment_map_valid_was_recorded: bool,
    /// `1a932c148` accidentally changed both the v67 header and frame layouts
    /// without changing their version number. This selects that late decoder
    /// after the header payload has identified the generation.
    v67_late_layout: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BinaryTraceFooter {
    version: u32,
    frame_count: u64,
    final_frame: u64,
}

#[derive(Debug, Deserialize, Serialize)]
struct NativeReblockBinding {
    version: u32,
    canonical_path: PathBuf,
    source_content_sha256: String,
    source_bytes: u64,
    source_semantic_sha256: String,
    frame_count: u64,
    final_frame: u64,
    #[cfg(unix)]
    source_device: u64,
    #[cfg(unix)]
    source_inode: u64,
}

/// Structural extent of the authoritative Original frame stream.
///
/// `frame_count` counts snapshots, not universal-frame increments.  Most
/// snapshots advance the universal frame once, but Original records a final
/// mission-success/interruption snapshot after the simulation tick returns
/// before incrementing the clock.  Such a record legitimately has
/// `frame_before == frame_after`, so `initial_frame + frame_count` is not the
/// stream's final frame.  The explicit frame envelopes are the authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TraceTimeline {
    next_frame_before: u64,
    frame_count: u64,
}

impl TraceTimeline {
    fn new(initial_frame: u64) -> Self {
        Self {
            next_frame_before: initial_frame,
            frame_count: 0,
        }
    }

    fn observe(&mut self, frame_before: u64, frame_after: u64) -> Result<(), String> {
        if frame_before != self.next_frame_before {
            return Err(format!(
                "frame {frame_before}->{frame_after} does not continue after frame {}",
                self.next_frame_before
            ));
        }
        let advanced_frame = frame_before
            .checked_add(1)
            .ok_or_else(|| format!("frame {frame_before} cannot advance without overflowing"))?;
        if frame_after != frame_before && frame_after != advanced_frame {
            return Err(format!(
                "frame {frame_before}->{frame_after} must either retain or advance the universal frame once"
            ));
        }
        self.next_frame_before = frame_after;
        self.frame_count = self
            .frame_count
            .checked_add(1)
            .ok_or_else(|| "parity frame count overflowed u64".to_owned())?;
        Ok(())
    }

    fn validate_terminator(&self, frame_count: u64, final_frame: u64) -> Result<(), String> {
        if frame_count != self.frame_count {
            return Err(format!(
                "terminator frame_count={frame_count} disagrees with {} frame records",
                self.frame_count
            ));
        }
        if final_frame != self.next_frame_before {
            return Err(format!(
                "terminator final_frame={final_frame} disagrees with the last frame_after={}",
                self.next_frame_before
            ));
        }
        Ok(())
    }
}

/// Recover the quit-mission message omitted by old schema-16 recorders.
///
/// The engine tick can report success without advancing only
/// through its leading won-quit branch. The UI message which set that flag
/// was not captured, so this exact terminal envelope proves the omitted input.
// TODO(parity-trace): record mission quitting in the original game and remove this
// compatibility inference after every surviving schema-16 trace includes it.
fn is_legacy_retained_terminal_success(
    schema: u32,
    frame_before: u64,
    frame_after: u64,
    simulation_body_ran: bool,
    game_code: i32,
) -> bool {
    schema == TRACE_SCHEMA_VERSION
        && frame_before == frame_after
        && !simulation_body_ran
        && game_code == GameCode::LevelSucceeded as i32
}

/// Stable host identity for campaign attempts synthesized by this replay tool.
///
/// Original predates native attempt history, so an archived trace cannot carry
/// the host nonce which current live sessions attach to terminal commands. Use
/// the logical recording-family name: chained `-session-NNNN` files belong to
/// one host run and therefore receive one identity, independent of their disk
/// location. The domain separator keeps this namespace distinct from future
/// deterministic tool identities.
fn replay_campaign_run_id(trace_path: &Path, session_index: u32) -> u64 {
    let file_name = trace_path.file_name().unwrap_or_else(|| {
        panic!(
            "parity trace path {} has no file name for campaign identity",
            trace_path.display()
        )
    });
    let file_name = file_name.to_string_lossy();
    let file_name = file_name
        .strip_suffix(TRACE_NATIVE_SUFFIX)
        .unwrap_or(&file_name);
    let session_suffix = format!("-session-{session_index:04}");
    let logical_stem = file_name.strip_suffix(".jsonl.zst").unwrap_or(&file_name);
    let recording_family = logical_stem
        .strip_suffix(&session_suffix)
        .unwrap_or(logical_stem);

    let mut digest = Sha256::new();
    digest.update(b"robin-original-parity-campaign-run-v1\0");
    digest.update(recording_family.as_bytes());
    let digest = digest.finalize();
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    let identity = u64::from_le_bytes(bytes);
    // Zero is reserved as an invalid durable campaign-history identity.
    if identity == 0 { u64::MAX } else { identity }
}

#[allow(clippy::too_many_arguments)]
fn append_legacy_retained_terminal_success_repair(
    commands_before_hourglass: &mut Vec<PlayerCommand>,
    commands_after_hourglass: &mut Vec<PlayerCommand>,
    difficulty: robin_engine::player_profile::DifficultyLevel,
    campaign_run_id: u64,
    schema: u32,
    frame_before: u64,
    frame_after: u64,
    simulation_body_ran: bool,
    game_code: i32,
    already_applied: &mut bool,
) -> bool {
    if *already_applied
        || !is_legacy_retained_terminal_success(
            schema,
            frame_before,
            frame_after,
            simulation_body_ran,
            game_code,
        )
    {
        return false;
    }

    // The omitted original-game quit-mission message both applies campaign/stat updates
    // and arms the won-quit flag before the retained simulation boundary. Rust keeps
    // terminal campaign updates in a post-hourglass command so achievement
    // evidence is finalized only after that final engine boundary, matching the
    // live Rust session transaction. Both phases still complete before parity
    // compares the Original terminal snapshot.
    *already_applied = true;
    commands_before_hourglass.push(PlayerCommand::QuitMissionRequested);
    commands_after_hourglass.push(PlayerCommand::ApplyQuitMissionUpdates {
        exit_code: GameCode::LevelSucceeded,
        difficulty,
        completed_at_unix_seconds: None,
        campaign_run_nonce: Some(campaign_run_id),
    });
    true
}

/// Legacy schemas 12 through 16 do not record the host lifecycle event that produced some
/// presentation-only random sprite-frame selection. It does retain the exact
/// RNG values and callsites, so the missing RNG boundary can be reconstructed
/// without inventing any retained engine side effects.
///
/// Admit only an otherwise ordinary in-progress frame with no resolved host
/// command and a previously unseen terminal callsite burst longer than the
/// retained scroll set. Besides random frame selection's homogeneous burst,
/// Original's mobile-element presentation update consumes one X/Y pair per
/// visual child, producing a repeated pair of
/// adjacent callsites. Requiring distinct retained values further rejects
/// accidental ordinary-callsite runs. Numeric addresses are build-specific and
/// deliberately never classified.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LegacyPresentationEntityState {
    entity_id: TraceEntityId,
    creation_order: u32,
    kind: TraceEntityKind,
    active: bool,
    position_bits: [u32; 2],
}

fn legacy_presentation_entity_states(
    elements: &[TraceElement],
) -> Vec<LegacyPresentationEntityState> {
    elements
        .iter()
        .map(|element| LegacyPresentationEntityState {
            entity_id: element.entity_id,
            creation_order: element.creation_order,
            kind: element.kind,
            active: element.active,
            position_bits: [element.position_map.x.bits, element.position_map.y.bits],
        })
        .collect()
}

/// Teleporting creates five no-supplier unconscious-star titbits at
/// the old position and five at the new position. The following draw refreshes
/// each transient titbit and randomizes its sprite frame once. Schema 16
/// omits both the host Draw boundary and these presentation-only entities, but
/// retains the actor/target teleport transition.
fn has_legacy_teleport_star_lifecycle(
    previous: Option<&[LegacyPresentationEntityState]>,
    current: &[LegacyPresentationEntityState],
) -> bool {
    let Some(previous) = previous else {
        return false;
    };
    if previous.len() != current.len() {
        return false;
    }

    let mut activated_moved_pcs = 0;
    let mut deactivated_targets = 0;
    for (before, after) in previous.iter().zip(current) {
        if before.entity_id != after.entity_id
            || before.creation_order != after.creation_order
            || before.kind != after.kind
        {
            return false;
        }
        match (before.kind, before.active, after.active) {
            (TraceEntityKind::Pc, false, true) if before.position_bits != after.position_bits => {
                activated_moved_pcs += 1;
            }
            (TraceEntityKind::Target, true, false) => deactivated_targets += 1,
            (TraceEntityKind::Pc | TraceEntityKind::Target, before_active, after_active)
                if before_active != after_active =>
            {
                return false;
            }
            _ => {}
        }
    }
    activated_moved_pcs == 1 && deactivated_targets == 1
}

fn legacy_presentation_sprite_rng_burst(
    schema: u32,
    trace_commands_were_empty: bool,
    game_code: i32,
    simulation_body_ran: bool,
    scroll_count: usize,
    has_teleport_star_lifecycle: bool,
    gameplay_callsite_offsets: &[u32],
    gameplay_values: &[u32],
) -> Option<usize> {
    if !trace_schema_is_supported(schema)
        || game_code != GameCode::LevelInProgress as i32
        || !simulation_body_ran
        || gameplay_callsite_offsets.len() != gameplay_values.len()
    {
        return None;
    }

    let terminal_callsite = *gameplay_callsite_offsets.last()?;
    let homogeneous_suffix_start = gameplay_callsite_offsets
        .iter()
        .rposition(|&offset| offset != terminal_callsite)
        .map_or(0, |index| index + 1);
    let alternating_suffix_start = (gameplay_callsite_offsets.len() >= 2).then(|| {
        let second = gameplay_callsite_offsets[gameplay_callsite_offsets.len() - 2];
        if second == terminal_callsite {
            return gameplay_callsite_offsets.len();
        }
        let mut start = gameplay_callsite_offsets.len() - 2;
        while start >= 2
            && gameplay_callsite_offsets[start - 2] == second
            && gameplay_callsite_offsets[start - 1] == terminal_callsite
        {
            start -= 2;
        }
        start
    });
    let suffix_start = alternating_suffix_start
        .filter(|&start| gameplay_callsite_offsets.len() - start >= 4)
        .unwrap_or(homogeneous_suffix_start);
    let (ordinary_prefix, presentation_suffix) = gameplay_callsite_offsets.split_at(suffix_start);
    let suffix_values = &gameplay_values[suffix_start..];
    let homogeneous = suffix_start == homogeneous_suffix_start;
    let large_unretained_burst = if homogeneous {
        trace_commands_were_empty && scroll_count != 0 && presentation_suffix.len() > scroll_count
    } else {
        // Three visual children are sufficient to distinguish the mobile
        // element's repeated X/Y vibration loop from one ordinary paired
        // gameplay calculation. The exact post-tick cursor check below still
        // requires this whole suffix, and only this suffix, to be unconsumed.
        presentation_suffix.len() >= 6
    };
    let exact_teleport_star_burst = homogeneous
        && trace_commands_were_empty
        && presentation_suffix.len() == 10
        && has_teleport_star_lifecycle;
    let callsites_were_seen = if homogeneous {
        ordinary_prefix.contains(&terminal_callsite)
    } else {
        ordinary_prefix.contains(&presentation_suffix[0])
            || ordinary_prefix.contains(&presentation_suffix[1])
    };
    if (!large_unretained_burst && !exact_teleport_star_burst)
        || callsites_were_seen
        || (homogeneous
            && suffix_values
                .iter()
                .enumerate()
                .any(|(index, value)| suffix_values[..index].contains(value)))
    {
        return None;
    }
    Some(presentation_suffix.len())
}

/// Admit a legacy presentation-only RNG candidate only when the retained
/// simulation body left exactly that suffix unconsumed.
///
/// A homogeneous original-game event run is not sufficient evidence by itself:
/// ordinary gameplay sites such as `BoredAnimationChoice` can produce a first,
/// distinct run longer than the retained scroll set. In that case Rust already
/// consumes the complete frame batch and replaying the candidate would consume
/// the same values twice. Conversely, a genuinely omitted host presentation
/// boundary leaves precisely the candidate suffix between the post-tick cursor
/// and the recorded frame end.
fn missing_legacy_presentation_sprite_rng_draws(
    candidate: Option<usize>,
    rust_rng_after_tick: usize,
    original_rng_end: usize,
) -> Option<usize> {
    let missing = original_rng_end.checked_sub(rust_rng_after_tick)?;
    (missing != 0 && candidate == Some(missing)).then_some(missing)
}

/// Recover repeated falling-arrow presentation passes omitted by legacy
/// schemas.  A single pass may contain several falling arrows, so compare the
/// leading original-game event run with the engine's exact pending-pass draw
/// count.  The callsite must already have correlated with `ArrowFallingFrame`
/// on an earlier exact frame in this trace; numeric offsets are build-specific.
fn legacy_additional_arrow_refresh_draws(
    schema: u32,
    gameplay_callsite_offsets: &[u32],
    known_arrow_falling_callsites: &BTreeSet<u32>,
    pending_pass_draws: usize,
) -> Option<usize> {
    if !trace_schema_is_supported(schema) || pending_pass_draws == 0 {
        return None;
    }
    let callsite = *gameplay_callsite_offsets.first()?;
    if !known_arrow_falling_callsites.contains(&callsite) {
        return None;
    }
    let leading_draws = gameplay_callsite_offsets
        .iter()
        .take_while(|&&candidate| candidate == callsite)
        .count();
    (leading_draws > pending_pass_draws).then_some(leading_draws - pending_pass_draws)
}

fn cross_post_initialize_frame(engine: &mut Engine, assets: &LevelAssets) {
    engine
        .parity_replay_setup()
        // Schema 16 omits the capture viewport and per-Draw camera position.
        // Keep its presentation-only edge compatibility isolated here; a
        // future schema carrying that provenance must pass `false` instead.
        .refresh_sprite_dimension_cache(assets, true);
    engine
        .advance_frame(
            assets,
            robin_engine::engine::SimulationFrameInput::no_hourglass().with_post_initialize(true),
        )
        .unwrap_or_else(|error| panic!("admit Original PostInitialize boundary: {error}"));
}

struct Options {
    inspect_capabilities: bool,
    scan_all: bool,
    no_auto_dump: bool,
    visual: bool,
    trace_path: PathBuf,
    dump: Option<DumpOptions>,
    http_server: Option<u16>,
    #[cfg(feature = "client")]
    start_paused: bool,
    frame_zero_screenshot_dir: Option<PathBuf>,
    bench_encodings: bool,
    convert: bool,
    reblock: bool,
    reblock_policy: NativeStoragePolicy,
    validate_native: bool,
}

struct DumpOptions {
    path: PathBuf,
    from_frame: u64,
    through_frame: u64,
    entities: Vec<TraceEntityId>,
}

const AUTOMATIC_DUMP_PRIOR_FRAMES: usize = 32;

struct RollingDumpFrame {
    engine: Engine,
    frame_before: u64,
    frame_after: u64,
    selected_pcs: Vec<TraceEntityId>,
    rng_draws: TraceRngBatch,
    resolved_commands: serde_json::Value,
    original_path_events: Vec<TracePathEvent>,
    rust_path_events: Vec<robin_engine::pathfinder::ParityPathEvent>,
    original_visibility_queries: Vec<TraceVisibilityQuery>,
    rust_visibility_queries: Vec<robin_engine::sight_obstacle::ParityVisibilityQuery>,
    original_movement_steps: Vec<TraceMovementStep>,
    rust_movement_steps: Vec<robin_engine::movement_diagnostics::ParityMovementStep>,
    original_flight_steps: Vec<TraceFlightStep>,
    rust_flight_steps: Vec<robin_engine::movement_diagnostics::ParityFlightStep>,
    rust_move_box_extractions: Vec<robin_engine::movement_diagnostics::ParityMoveBoxExtraction>,
    rng_start: usize,
    expected_rng_end: usize,
    actual_rng_end: usize,
    rust_rng_sites: Vec<robin_engine::sim_rng::RngSite>,
    rust_rng_diagnostics: robin_engine::sim_rng::OriginalRngDiagnostics,
    differences: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct MotionLineSignature {
    layer: u16,
    ax: u32,
    ay: u32,
    bx: u32,
    by: u32,
    repulsive: bool,
}

impl MotionLineSignature {
    fn original(layer: u16, line: &TraceMotionLine) -> Self {
        Self {
            layer,
            ax: line.a.x.bits,
            ay: line.a.y.bits,
            bx: line.b.x.bits,
            by: line.b.y.bits,
            repulsive: line.type_mask & 128 != 0,
        }
    }

    fn rust(layer: u16, line: &robin_engine::fast_find_grid::GridLine) -> Self {
        Self {
            layer,
            ax: line.a.x.to_bits(),
            ay: line.a.y.to_bits(),
            bx: line.b.x.to_bits(),
            by: line.b.y.to_bits(),
            repulsive: line.is_repulsive,
        }
    }
}

/// Isomorphic mapping from the Original's layer-local line indices to Rust's
/// flat `LineIndex` arena. Geometry and the behavior-relevant repulsive flag
/// form the identity; identical duplicate lines are paired by occurrence.
struct MotionLineParity {
    original_to_rust: BTreeMap<(u16, u16), LineIndex>,
    expected_active: BTreeMap<(u16, u16), bool>,
    initial_differences: Vec<String>,
}

impl MotionLineParity {
    fn build(engine: &Engine, original: &TraceMotionGrid) -> Self {
        let mut original_groups =
            BTreeMap::<MotionLineSignature, Vec<(u16, &TraceMotionLine)>>::new();
        let mut expected_active = BTreeMap::new();
        let mut initial_differences = Vec::new();
        for layer in &original.layers {
            for line in &layer.lines {
                let address = (layer.layer, line.index);
                if expected_active.insert(address, line.active).is_some() {
                    initial_differences.push(format!(
                        "motion_grid.static_mapping: duplicate Original line address layer={} index={}",
                        layer.layer, line.index
                    ));
                }
                if line.type_mask & 2 == 0 {
                    initial_differences.push(format!(
                        "motion_grid.static_mapping: Original layer={} index={} is not LINE_MOTION (type_mask={} sector={})",
                        layer.layer, line.index, line.type_mask, line.associated_sector
                    ));
                }
                original_groups
                    .entry(MotionLineSignature::original(layer.layer, line))
                    .or_default()
                    .push((layer.layer, line));
            }
        }

        let grid = engine.fast_grid();
        let mut rust_groups = BTreeMap::<MotionLineSignature, Vec<LineIndex>>::new();
        for (layer_index, layer) in grid.level.layers.iter().enumerate() {
            let layer_number =
                u16::try_from(layer_index).expect("Rust motion-grid layer index exceeds u16");
            for &line_index in &layer.line_indices {
                let line = &grid.level.lines[usize::from(line_index)];
                if line.is_motion {
                    rust_groups
                        .entry(MotionLineSignature::rust(layer_number, line))
                        .or_default()
                        .push(line_index);
                }
            }
        }

        let signatures = original_groups
            .keys()
            .chain(rust_groups.keys())
            .copied()
            .collect::<BTreeSet<_>>();
        let mut original_to_rust = BTreeMap::new();
        for signature in signatures {
            let originals = original_groups
                .get(&signature)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let rust = rust_groups
                .get(&signature)
                .map(Vec::as_slice)
                .unwrap_or_default();
            if originals.len() != rust.len() {
                initial_differences.push(format!(
                    "motion_grid.static_mapping: signature={signature:?} original_count={} rust_count={}",
                    originals.len(),
                    rust.len()
                ));
            }
            for ((layer, original_line), &rust_line) in originals.iter().zip(rust) {
                let address = (*layer, original_line.index);
                original_to_rust.insert(address, rust_line);
                let rust_active = grid.is_line_active(rust_line);
                if original_line.active != rust_active {
                    initial_differences.push(format!(
                        "motion_grid.initial_active[layer={} index={} rust_line={}]: original={} rust={} type_mask={} sector={}",
                        layer,
                        original_line.index,
                        rust_line,
                        original_line.active,
                        rust_active,
                        original_line.type_mask,
                        original_line.associated_sector
                    ));
                }
            }
        }

        Self {
            original_to_rust,
            expected_active,
            initial_differences,
        }
    }

    fn apply_changes_and_compare(
        &mut self,
        engine: &Engine,
        changes: &[TraceMotionLineChange],
    ) -> Vec<String> {
        let mut differences = std::mem::take(&mut self.initial_differences);
        for change in changes {
            let address = (change.layer, change.index);
            let Some(expected) = self.expected_active.get_mut(&address) else {
                differences.push(format!(
                    "motion_grid.line_active[layer={} index={}]: Original change references an unknown static line",
                    change.layer, change.index
                ));
                continue;
            };
            *expected = change.active;
        }

        let grid = engine.fast_grid();
        for (&(layer, original_index), &expected) in &self.expected_active {
            let Some(&rust_index) = self.original_to_rust.get(&(layer, original_index)) else {
                continue;
            };
            let actual = grid.is_line_active(rust_index);
            if expected != actual {
                differences.push(format!(
                    "motion_grid.line_active[layer={layer} index={original_index} rust_line={rust_index}]: original={expected:?} rust={actual:?}"
                ));
            }
        }
        differences
    }
}

impl DumpOptions {
    fn includes(&self, frame: u64) -> bool {
        (self.from_frame..=self.through_frame).contains(&frame)
    }
}

#[cfg(feature = "client")]
struct ActiveHttpStep {
    request: robin_rs::http_server::PendingStep,
    direction: &'static str,
    from_frame: u32,
    remaining: u32,
    requested: u32,
}

#[cfg(feature = "client")]
struct VisualReplay {
    window: robin_rs::window::GameWindow,
    renderer: Renderer,
    host: Host,
    sprite_images: BTreeMap<u32, (GpuImage, u16, u16)>,
}

#[cfg(feature = "client")]
impl VisualReplay {
    fn new(
        mut window: robin_rs::window::GameWindow,
        mut host: Host,
        engine: &Engine,
        background: robin_engine::engine::level_loading::PreDecodedBackground,
    ) -> Self {
        window.set_logical_size(1024, 768);
        let mut renderer = Renderer::new(&window, 1024, 768, TextureScaleMode::Nearest);
        robin_rs::level_loading_host::initialize_sprite_variants(&mut host, engine);
        robin_rs::level_loading_host::apply_background_map(
            engine,
            &mut host,
            &mut renderer,
            background,
        );
        Self {
            window,
            renderer,
            host,
            sprite_images: BTreeMap::new(),
        }
    }

    /// Draw the parity engine's current state while deliberately ignoring all
    /// live keyboard/mouse commands. The trace remains the only input source.
    fn queue_frame(&mut self, engine: &Engine) {
        let focus = engine
            .selected_hero_ids()
            .first()
            .and_then(|id| engine.get_entity(*id))
            .or_else(|| {
                engine
                    .entities_with_ids_iter()
                    .find_map(|(_, entity)| entity.is_human().then_some(entity))
            })
            .map(|entity| entity.element_data().position_map())
            .unwrap_or(MapPoint::ZERO);
        self.host.frontend.viewport.view_position =
            MapPoint::new((focus.x - 512.0).max(0.0), (focus.y - 319.0).max(0.0));
        self.host.frontend.viewport.zoom_factor = 1.0;
        engine.draw_background(&mut self.host, &mut self.renderer);

        let mut entities: Vec<_> = engine.entities_with_ids_iter().collect();
        entities.sort_by(|(_, left), (_, right)| {
            left.sprite_visual_map_position()
                .y
                .total_cmp(&right.sprite_visual_map_position().y)
        });
        for (_, entity) in entities {
            if !entity.element_data().active
                || entity.element_data().hidden_in_building
                || !entity.is_to_be_displayed(true)
            {
                continue;
            }
            let sprite = entity.sprite();
            if sprite.current_width == 0 || sprite.current_height == 0 {
                continue;
            }
            let bank_id = sprite.bank_id_for(sprite.current_row, sprite.current_frame);
            if !self.sprite_images.contains_key(&bank_id) {
                let width = self.host.frontend.frame_holder.sprite_width(bank_id);
                let height = self.host.frontend.frame_holder.sprite_height(bank_id);
                if width == 0 || height == 0 {
                    continue;
                }
                let rgba = if let Some(rgba) = self.host.frontend.frame_holder.rgba_data(bank_id) {
                    rgba.to_vec()
                } else {
                    let mut pixels = vec![0_u16; usize::from(width) * usize::from(height)];
                    self.host.frontend.frame_holder.uncompress_frame(
                        &mut pixels,
                        usize::from(width),
                        bank_id,
                        robin_assets::frame_holder::SpriteVariant::Day,
                        engine.weather().night_color,
                        16,
                    );
                    let mut rgba = Vec::with_capacity(pixels.len() * 4);
                    for pixel in pixels {
                        if pixel == robin_assets::frame_holder::TRANSPARENT_COLOR_16 {
                            rgba.extend_from_slice(&[0, 0, 0, 0]);
                        } else {
                            let (r, g, b) = rgb565_to_rgb8(pixel);
                            rgba.extend_from_slice(&[r, g, b, 255]);
                        }
                    }
                    rgba
                };
                let image = self
                    .renderer
                    .create_rgba_gpu_image(width, height, &rgba, "parity replay sprite")
                    .unwrap_or_else(|| panic!("create parity sprite image for bank {bank_id}"));
                self.sprite_images.insert(bank_id, (image, width, height));
            }
            let (image, width, height) = &self.sprite_images[&bank_id];
            let world = entity.sprite_visual_map_position();
            let offset = sprite.offset(sprite.current_row, sprite.current_frame);
            let sprite_x = (world.x - sprite.center.x).floor() + offset.x;
            let sprite_y = (world.y - sprite.center.y).floor() + offset.y;
            let dst = BBox::from_coords(
                sprite_x - self.host.frontend.viewport.view_position.x,
                sprite_y - self.host.frontend.viewport.view_position.y,
                sprite_x - self.host.frontend.viewport.view_position.x + f32::from(*width),
                sprite_y - self.host.frontend.viewport.view_position.y + f32::from(*height),
            );
            self.renderer
                .render_gpu_image(image, None, Some(&dst), BlendMode::Blend);
        }
    }

    fn render(&mut self, engine: &Engine) -> bool {
        let _events = self.window.poll_events();
        if self.window.close_requested {
            return false;
        }

        self.queue_frame(engine);
        self.renderer.present();
        std::thread::sleep(std::time::Duration::from_millis(16));
        true
    }

    fn wait_until_closed(&mut self) {
        while !self.window.close_requested {
            let _events = self.window.poll_events();
            std::thread::sleep(std::time::Duration::from_millis(16));
        }
    }
}

fn register_language_data_paths_for_tool() {
    #[cfg(feature = "client")]
    robin_rs::main_entry::register_language_data_paths_for_tool();
    #[cfg(not(feature = "client"))]
    crate::register_language_data_paths();
}

fn parse_options() -> Options {
    const USAGE: &str = "usage: original_parity_replay [--inspect-capabilities] [--scan-all] [--no-auto-dump] [--visual] \
        [--frame-zero-screenshot-dir DIR] \
        [--frame-zero-screenshot-only] \
        [--http-server PORT [--start-paused]] \
        [--dump-jsonl PATH [--dump-from FRAME] [--dump-through FRAME] \
        [--dump-entity KIND:INDEX]...] [--bench-encodings] [--convert] \
        [--reblock [--reblock-records N] [--reblock-window-log N]] \
        [--validate-native] TRACE.jsonl[.zst]";

    let mut args = std::env::args_os().skip(1);
    let mut bench_encodings = false;
    let mut convert = false;
    let mut reblock = false;
    let mut reblock_records = TRACE_NATIVE_BLOCK_RECORDS;
    let mut reblock_window_log = TRACE_NATIVE_WINDOW_LOG;
    let mut reblock_policy_requested = false;
    let mut validate_native = false;
    let mut inspect_capabilities = false;
    let mut scan_all = false;
    let mut no_auto_dump = false;
    let mut visual = false;
    let mut trace_path = None;
    let mut dump_path = None;
    let mut dump_from = 0;
    let mut dump_through = u64::MAX;
    let mut dump_entities = Vec::new();
    let mut http_server = None;
    let mut start_paused = false;
    let mut frame_zero_screenshot_dir = None;
    let mut frame_zero_screenshot_only = false;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--inspect-capabilities") => inspect_capabilities = true,
            Some("--scan-all") => scan_all = true,
            Some("--bench-encodings") => bench_encodings = true,
            Some("--convert") => convert = true,
            Some("--reblock") => reblock = true,
            Some("--reblock-records") => {
                reblock_policy_requested = true;
                reblock_records =
                    usize::try_from(parse_u64_option(args.next(), "--reblock-records"))
                        .expect("--reblock-records exceeds usize");
            }
            Some("--reblock-window-log") => {
                reblock_policy_requested = true;
                reblock_window_log =
                    u32::try_from(parse_u64_option(args.next(), "--reblock-window-log"))
                        .expect("--reblock-window-log exceeds u32");
            }
            Some("--validate-native") => validate_native = true,
            Some("--no-auto-dump") => no_auto_dump = true,
            Some("--visual") => visual = true,
            Some("--frame-zero-screenshot-dir") => {
                let value = args.next().unwrap_or_else(|| panic!("{USAGE}"));
                assert!(
                    frame_zero_screenshot_dir
                        .replace(PathBuf::from(value))
                        .is_none(),
                    "{USAGE}"
                );
            }
            Some("--frame-zero-screenshot-only") => frame_zero_screenshot_only = true,
            Some("--http-server") => {
                let port = parse_u64_option(args.next(), "--http-server");
                let port = u16::try_from(port).expect("--http-server port exceeds 65535");
                assert_ne!(
                    port, 0,
                    "--http-server 0 cannot serve parity replay controls"
                );
                assert!(http_server.replace(port).is_none(), "{USAGE}");
            }
            Some("--start-paused") => start_paused = true,
            Some("--dump-jsonl") => {
                let value = args.next().unwrap_or_else(|| panic!("{USAGE}"));
                assert!(dump_path.replace(PathBuf::from(value)).is_none(), "{USAGE}");
            }
            Some("--dump-from") => {
                dump_from = parse_u64_option(args.next(), "--dump-from");
            }
            Some("--dump-through") => {
                dump_through = parse_u64_option(args.next(), "--dump-through");
            }
            Some("--dump-entity") => {
                let value = args.next().unwrap_or_else(|| panic!("{USAGE}"));
                dump_entities.push(parse_dump_entity(&value.to_string_lossy()));
            }
            Some(value) if value.starts_with('-') => panic!("unknown option {value:?}\n{USAGE}"),
            _ => {
                assert!(trace_path.replace(PathBuf::from(arg)).is_none(), "{USAGE}");
            }
        }
    }
    let trace_path = trace_path.unwrap_or_else(|| panic!("{USAGE}"));
    assert!(
        dump_from <= dump_through,
        "--dump-from exceeds --dump-through"
    );
    assert!(
        dump_path.is_some()
            || (dump_from == 0 && dump_through == u64::MAX && dump_entities.is_empty()),
        "--dump-from, --dump-through, and --dump-entity require --dump-jsonl"
    );
    assert!(
        !start_paused || http_server.is_some(),
        "--start-paused requires --http-server"
    );
    assert!(
        !frame_zero_screenshot_only || frame_zero_screenshot_dir.is_some(),
        "--frame-zero-screenshot-only requires --frame-zero-screenshot-dir"
    );
    assert!(
        usize::from(bench_encodings)
            + usize::from(inspect_capabilities)
            + usize::from(convert)
            + usize::from(reblock)
            + usize::from(validate_native)
            <= 1,
        "--inspect-capabilities, --bench-encodings, --convert, --reblock, and --validate-native are mutually exclusive"
    );
    assert!(
        reblock || !reblock_policy_requested,
        "--reblock-records and --reblock-window-log require --reblock"
    );
    let reblock_policy = NativeStoragePolicy::new(reblock_records, reblock_window_log);
    Options {
        inspect_capabilities,
        scan_all,
        no_auto_dump,
        visual,
        trace_path,
        http_server,
        #[cfg(feature = "client")]
        start_paused,
        frame_zero_screenshot_dir,
        bench_encodings,
        convert,
        reblock,
        reblock_policy,
        validate_native,
        dump: dump_path.map(|path| DumpOptions {
            path,
            from_frame: dump_from,
            through_frame: dump_through,
            entities: dump_entities,
        }),
    }
}

#[cfg(feature = "client")]
fn frame_zero_screenshot_path(output_dir: &Path, trace_path: &Path) -> PathBuf {
    let relative = trace_path
        .ancestors()
        .find(|ancestor| ancestor.file_name().is_some_and(|name| name == "traces"))
        .and_then(|trace_root| trace_path.strip_prefix(trace_root).ok());
    let source_name = relative.unwrap_or_else(|| {
        trace_path
            .file_name()
            .map(Path::new)
            .expect("parity trace path has no filename")
    });
    let mut flat_name = source_name
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("__");
    for suffix in [".rhrec.jsonl.zst", ".jsonl.zst", ".rhrec.jsonl", ".jsonl"] {
        if let Some(stem) = flat_name.strip_suffix(suffix) {
            flat_name = stem.to_owned();
            break;
        }
    }
    output_dir.join(format!("{flat_name}.png"))
}

fn parse_trace_frame(line: &str, line_number: usize) -> Option<TraceFrame> {
    match serde_json::from_str(line) {
        Ok(frame) => {
            let frame: TraceFrame = frame;
            assert_eq!(
                frame.record_type, "frame",
                "invalid parity frame record type on line {line_number}"
            );
            Some(frame)
        }
        Err(frame_error) => {
            let marker: TraceRecordMarker = serde_json::from_str(line).unwrap_or_else(|_| {
                panic!("parse trace frame on line {line_number}: {frame_error}")
            });
            if marker.record_type.as_deref() == Some("rng_suffix") {
                None
            } else {
                panic!("parse trace frame on line {line_number}: {frame_error}");
            }
        }
    }
}

#[cfg(feature = "client")]
fn drain_headless_http(
    engine: &mut Engine,
    display: &mut HostDisplayState,
    assets: &LevelAssets,
    input: &mut InputState,
    selected_view_element: &mut Option<EntityId>,
    manual_pause: &mut bool,
    active_step: &mut Option<ActiveHttpStep>,
) -> robin_engine::player_command::FrameCommands {
    let commands = robin_rs::http_server::drain_global_headless(
        engine,
        display,
        assets,
        input,
        selected_view_element,
    );
    for request in robin_rs::http_server::take_pending_steps() {
        match request.kind {
            robin_rs::http_server::StepKind::Forward { n, .. } => {
                if n == 0 {
                    request.respond_ok(serde_json::json!({
                        "direction": "forward",
                        "from_frame": engine.frame_counter(),
                        "frame": engine.frame_counter(),
                        "advanced": 0,
                        "parity": "matched",
                    }));
                } else if active_step.is_some() {
                    request.respond_err("another parity replay step is already active");
                } else {
                    *active_step = Some(ActiveHttpStep {
                        request,
                        direction: "forward",
                        from_frame: engine.frame_counter(),
                        remaining: n,
                        requested: n,
                    });
                }
            }
            robin_rs::http_server::StepKind::Back { .. } => {
                request.respond_err(
                    "step-back is unavailable for Original parity traces; restart and go-to-frame",
                );
            }
            robin_rs::http_server::StepKind::GoToFrame { target, .. } => {
                let current = engine.frame_counter();
                if target < current {
                    request.respond_err(
                        "backward go-to-frame is unavailable for Original parity traces; restart the runner",
                    );
                } else if target == current {
                    request.respond_ok(serde_json::json!({
                        "direction": "go-to-frame",
                        "from_frame": current,
                        "frame": current,
                        "advanced": 0,
                        "parity": "matched",
                    }));
                } else if active_step.is_some() {
                    request.respond_err("another parity replay step is already active");
                } else {
                    *active_step = Some(ActiveHttpStep {
                        request,
                        direction: "go-to-frame",
                        from_frame: current,
                        remaining: target - current,
                        requested: target - current,
                    });
                }
            }
            robin_rs::http_server::StepKind::SetPaused { paused } => {
                *manual_pause = paused;
                request.respond_ok(serde_json::json!({
                    "paused": paused,
                    "frame": engine.frame_counter(),
                }));
            }
        }
    }
    commands
}

#[cfg(feature = "client")]
fn serve_halted_http(
    engine: &mut Engine,
    display: &mut HostDisplayState,
    assets: &LevelAssets,
    input: &mut InputState,
    selected_view_element: &mut Option<EntityId>,
) -> ! {
    loop {
        let _ = robin_rs::http_server::drain_global_headless(
            engine,
            display,
            assets,
            input,
            selected_view_element,
        );
        for request in robin_rs::http_server::take_pending_steps() {
            request.respond_err(format!(
                "parity replay is halted at divergent frame {}",
                engine.frame_counter()
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn parse_u64_option(value: Option<std::ffi::OsString>, option: &str) -> u64 {
    value
        .unwrap_or_else(|| panic!("{option} requires a value"))
        .to_string_lossy()
        .parse()
        .unwrap_or_else(|_| panic!("{option} must be an unsigned frame number"))
}

fn parse_dump_entity(value: &str) -> TraceEntityId {
    let (kind, index) = value
        .split_once(':')
        .unwrap_or_else(|| panic!("--dump-entity must be KIND:INDEX, got {value:?}"));
    let kind = match kind {
        "pc" => TraceEntityKind::Pc,
        "soldier" => TraceEntityKind::Soldier,
        "civilian" => TraceEntityKind::Civilian,
        "fx" => TraceEntityKind::Fx,
        "target" => TraceEntityKind::Target,
        "bonus" => TraceEntityKind::Bonus,
        "scroll" => TraceEntityKind::Scroll,
        "projectile" => TraceEntityKind::Projectile,
        "net" => TraceEntityKind::Net,
        _ => panic!("unknown --dump-entity kind {kind:?}"),
    };
    TraceEntityId {
        kind,
        index: index
            .parse()
            .unwrap_or_else(|_| panic!("invalid --dump-entity index in {value:?}")),
    }
}

#[allow(clippy::too_many_arguments)]
fn write_engine_dump_frame(
    writer: &mut BufWriter<File>,
    options: &DumpOptions,
    engine: &Engine,
    entity_map: &EntityMap,
    frame: &TraceFrame,
    resolved_commands: serde_json::Value,
    rng_start: usize,
    expected_rng_end: usize,
    actual_rng_end: usize,
    rust_rng_sites: &[robin_engine::sim_rng::RngSite],
    rust_rng_diagnostics: &robin_engine::sim_rng::OriginalRngDiagnostics,
    rust_path_events: &[robin_engine::pathfinder::ParityPathEvent],
    rust_visibility_queries: &[robin_engine::sight_obstacle::ParityVisibilityQuery],
    original_movement_steps: &[TraceMovementStep],
    rust_movement_steps: &[robin_engine::movement_diagnostics::ParityMovementStep],
    original_flight_steps: &[TraceFlightStep],
    rust_flight_steps: &[robin_engine::movement_diagnostics::ParityFlightStep],
    rust_move_box_extractions: &[robin_engine::movement_diagnostics::ParityMoveBoxExtraction],
    differences: &[String],
) {
    let diagnostic_engine = engine.diagnostic_snapshot_without_original_rng_replay();
    let original_entities = options
        .entities
        .iter()
        .map(|entity_id| {
            frame
                .elements
                .iter()
                .find(|element| element.entity_id == *entity_id)
                .unwrap_or_else(|| {
                    panic!(
                        "manual parity dump requested missing Original entity {entity_id:?} at frame {}",
                        frame.frame_after
                    )
                })
        })
        .collect::<Vec<_>>();
    write_engine_dump_snapshot_frame(
        writer,
        options,
        &diagnostic_engine,
        entity_map,
        frame.frame_before,
        frame.frame_after,
        &frame.selected_pcs,
        &frame.rng_draws,
        &frame.path_events,
        rust_path_events,
        &frame.visibility_queries,
        rust_visibility_queries,
        original_movement_steps,
        rust_movement_steps,
        original_flight_steps,
        rust_flight_steps,
        rust_move_box_extractions,
        resolved_commands,
        rng_start,
        expected_rng_end,
        actual_rng_end,
        rust_rng_sites,
        rust_rng_diagnostics,
        differences,
        Some(&original_entities),
    );
}

#[allow(clippy::too_many_arguments)]
fn write_engine_dump_snapshot_frame(
    writer: &mut BufWriter<File>,
    options: &DumpOptions,
    diagnostic_engine: &Engine,
    entity_map: &EntityMap,
    frame_before: u64,
    frame_after: u64,
    selected_pcs: &[TraceEntityId],
    rng_draws: &TraceRngBatch,
    original_path_events: &[TracePathEvent],
    rust_path_events: &[robin_engine::pathfinder::ParityPathEvent],
    original_visibility_queries: &[TraceVisibilityQuery],
    rust_visibility_queries: &[robin_engine::sight_obstacle::ParityVisibilityQuery],
    original_movement_steps: &[TraceMovementStep],
    rust_movement_steps: &[robin_engine::movement_diagnostics::ParityMovementStep],
    original_flight_steps: &[TraceFlightStep],
    rust_flight_steps: &[robin_engine::movement_diagnostics::ParityFlightStep],
    rust_move_box_extractions: &[robin_engine::movement_diagnostics::ParityMoveBoxExtraction],
    resolved_commands: serde_json::Value,
    rng_start: usize,
    expected_rng_end: usize,
    actual_rng_end: usize,
    rust_rng_sites: &[robin_engine::sim_rng::RngSite],
    rust_rng_diagnostics: &robin_engine::sim_rng::OriginalRngDiagnostics,
    differences: &[String],
    original_entities: Option<&[&TraceElement]>,
) {
    let mapped_entities = options
        .entities
        .iter()
        .map(|original| {
            let rust = entity_map.translate(*original);
            serde_json::json!({
                "original": original,
                "rust": {
                    "kind": format!("{:?}", rust.kind()).to_lowercase(),
                    "index": rust.index(),
                },
            })
        })
        .collect::<Vec<_>>();
    let selected_rust_indices = options
        .entities
        .iter()
        .map(|original| entity_map.translate(*original).index() as usize)
        .collect::<BTreeSet<_>>();
    let mut engine_value = serde_to_json_value(diagnostic_engine);
    if !selected_rust_indices.is_empty() {
        let entities = engine_value
            .get_mut("world")
            .and_then(|world| world.get_mut("entities"))
            .and_then(serde_json::Value::as_array_mut)
            .expect("serialized Engine.world.entities must be an array");
        for (index, entity) in entities.iter_mut().enumerate() {
            if !selected_rust_indices.contains(&index) {
                *entity = serde_json::Value::Null;
            }
        }
    }
    let mut record = serde_json::json!({
        "schema": "robin-parity-engine-dump.v1",
        "type": "frame",
        "frame_before": frame_before,
        "frame_after": frame_after,
        "input": {
            "resolved_commands": resolved_commands,
            "selected_pcs": selected_pcs,
        },
        "original_path_events": original_path_events,
        "rust_path_events": rust_path_events,
        "visibility_queries": {
            "original": original_visibility_queries,
            "rust": rust_visibility_queries,
        },
        "original_movement_steps": original_movement_steps,
        "rust_movement_steps": rust_movement_steps,
        "original_flight_steps": original_flight_steps,
        "rust_flight_steps": rust_flight_steps,
        "rust_move_box_extractions": rust_move_box_extractions,
        "rng": {
            "cursor_before": rng_start,
            "expected_cursor_after": expected_rng_end,
            "actual_cursor_after": actual_rng_end,
            "rust_sites": rust_rng_sites,
            "rust_script_diagnostics": rust_rng_diagnostics,
            "original_frame_draws": rng_draws,
            "engine_original_replay_stream_omitted": true,
        },
        "entity_mapping": mapped_entities,
        "parity_differences": differences,
        "engine": engine_value,
    });
    if let Some(original_entities) = original_entities {
        record["original_entities"] = serde_json::to_value(original_entities)
            .expect("serialize selected Original parity dump entities");
    }
    write_jsonl_record(writer, &record);
}

fn push_rolling_window<T>(frames: &mut VecDeque<T>, frame: T) {
    frames.push_back(frame);
    let capacity = AUTOMATIC_DUMP_PRIOR_FRAMES + 1;
    while frames.len() > capacity {
        frames.pop_front();
    }
}

fn write_automatic_rolling_dump(
    frames: &VecDeque<RollingDumpFrame>,
    trace_path: &std::path::Path,
    header: &TraceHeader,
    entity_map: &EntityMap,
    divergent_frame: u64,
) -> PathBuf {
    assert!(
        !frames.is_empty(),
        "automatic parity dump requires at least one captured frame"
    );
    let prefix = format!("robin-parity-divergence-frame-{divergent_frame}-");
    // Replay changes cwd to the selected data directory during engine setup,
    // so current_dir is not a stable workspace anchor here. Prefer the source
    // trace's repository ancestor and retain the compile-time workspace as a
    // fallback for traces recorded outside this checkout.
    let workspace_root = trace_path
        .ancestors()
        .find(|ancestor| ancestor.join(".git").exists())
        .map(PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let dump_dir = workspace_root.join(".codex-tmp").join("parity-dumps");
    std::fs::create_dir_all(&dump_dir).expect("create workspace automatic parity dump directory");
    let temporary = tempfile::Builder::new()
        .prefix(&prefix)
        .suffix(".jsonl")
        .tempfile_in(&dump_dir)
        .expect("create unique automatic parity dump");
    let (file, path) = temporary
        .keep()
        .expect("persist unique automatic parity dump");
    let mut writer = BufWriter::new(file);
    let first_frame = frames.front().expect("rolling dump has a first frame");
    let last_frame = frames.back().expect("rolling dump has a last frame");
    let options = DumpOptions {
        path: path.clone(),
        from_frame: first_frame.frame_after,
        through_frame: last_frame.frame_after,
        entities: Vec::new(),
    };
    write_jsonl_record(
        &mut writer,
        &serde_json::json!({
            "schema": "robin-parity-engine-dump.v1",
            "type": "header",
            "source_trace": trace_path,
            "mission": header.mission,
            "rng_seed": header.rng_seed,
            "frame_range": {
                "from": options.from_frame,
                "through": options.through_frame,
            },
            "entity_filter": options.entities,
            "automatic_rolling_window": true,
        }),
    );
    for frame in frames {
        write_engine_dump_snapshot_frame(
            &mut writer,
            &options,
            &frame.engine,
            entity_map,
            frame.frame_before,
            frame.frame_after,
            &frame.selected_pcs,
            &frame.rng_draws,
            &frame.original_path_events,
            &frame.rust_path_events,
            &frame.original_visibility_queries,
            &frame.rust_visibility_queries,
            &frame.original_movement_steps,
            &frame.rust_movement_steps,
            &frame.original_flight_steps,
            &frame.rust_flight_steps,
            &frame.rust_move_box_extractions,
            frame.resolved_commands.clone(),
            frame.rng_start,
            frame.expected_rng_end,
            frame.actual_rng_end,
            &frame.rust_rng_sites,
            &frame.rust_rng_diagnostics,
            &frame.differences,
            None,
        );
    }
    writer.flush().expect("flush automatic parity dump");
    eprintln!("automatic parity engine dump: {}", path.display());
    path
}

fn write_jsonl_record(writer: &mut BufWriter<File>, value: &serde_json::Value) {
    serde_json::to_writer(&mut *writer, value).expect("serialize diagnostic JSONL record");
    writer
        .write_all(b"\n")
        .expect("write diagnostic JSONL newline");
    writer.flush().expect("flush diagnostic JSONL record");
}

fn serde_to_json_value<T: Serialize + ?Sized>(value: &T) -> serde_json::Value {
    serde_value_to_json(serde_value::to_value(value).expect("serialize diagnostic engine state"))
}

fn serde_value_to_json(value: serde_value::Value) -> serde_json::Value {
    match value {
        serde_value::Value::Bool(v) => serde_json::Value::Bool(v),
        serde_value::Value::I8(v) => serde_json::json!(v),
        serde_value::Value::I16(v) => serde_json::json!(v),
        serde_value::Value::I32(v) => serde_json::json!(v),
        serde_value::Value::I64(v) => serde_json::json!(v),
        serde_value::Value::U8(v) => serde_json::json!(v),
        serde_value::Value::U16(v) => serde_json::json!(v),
        serde_value::Value::U32(v) => serde_json::json!(v),
        serde_value::Value::U64(v) => serde_json::json!(v),
        serde_value::Value::F32(v) => serde_json::json!(v),
        serde_value::Value::F64(v) => serde_json::json!(v),
        serde_value::Value::Char(v) => serde_json::json!(v.to_string()),
        serde_value::Value::String(v) => serde_json::Value::String(v),
        serde_value::Value::Bytes(v) => serde_json::json!(v),
        serde_value::Value::Unit => serde_json::Value::Null,
        serde_value::Value::Option(v) => v
            .map(|v| serde_value_to_json(*v))
            .unwrap_or(serde_json::Value::Null),
        serde_value::Value::Newtype(v) => serde_value_to_json(*v),
        serde_value::Value::Seq(values) => {
            serde_json::Value::Array(values.into_iter().map(serde_value_to_json).collect())
        }
        serde_value::Value::Map(entries) => serde_json::Value::Object(
            entries
                .into_iter()
                .map(|(key, value)| (serde_value_key_to_string(key), serde_value_to_json(value)))
                .collect(),
        ),
    }
}

fn serde_value_key_to_string(key: serde_value::Value) -> String {
    match key {
        serde_value::Value::String(v) => v,
        serde_value::Value::Char(v) => v.to_string(),
        serde_value::Value::Bool(v) => v.to_string(),
        serde_value::Value::I8(v) => v.to_string(),
        serde_value::Value::I16(v) => v.to_string(),
        serde_value::Value::I32(v) => v.to_string(),
        serde_value::Value::I64(v) => v.to_string(),
        serde_value::Value::U8(v) => v.to_string(),
        serde_value::Value::U16(v) => v.to_string(),
        serde_value::Value::U32(v) => v.to_string(),
        serde_value::Value::U64(v) => v.to_string(),
        other => format!("{other:?}"),
    }
}

fn restore_campaign(
    trace: &TraceCampaign,
    profiles: &robin_engine::profiles::ProfileManager,
) -> robin_engine::campaign::Campaign {
    use robin_engine::campaign::{Campaign, CampaignValue, PcDescription};
    use robin_engine::mission::{Mission, MissionStatus};
    use robin_engine::pc_status::{HumanStatus, PcStatus, Skill};
    use robin_engine::profiles::CharacterProfileIdx;
    use robin_engine::sector_production::{Occupant, SectorProduction, Type};

    assert_eq!(trace.version, 1, "unsupported campaign snapshot version");
    const VALUE_KEYS: [CampaignValue; 27] = [
        CampaignValue::Amulets,
        CampaignValue::Ransom,
        CampaignValue::Score,
        CampaignValue::Blazon,
        CampaignValue::LivingSoldiers,
        CampaignValue::DeadSoldiers,
        CampaignValue::MissionLength,
        CampaignValue::Custom1,
        CampaignValue::Custom2,
        CampaignValue::Custom3,
        CampaignValue::Custom4,
        CampaignValue::Custom5,
        CampaignValue::Custom6,
        CampaignValue::Custom7,
        CampaignValue::Custom8,
        CampaignValue::Custom9,
        CampaignValue::Custom10,
        CampaignValue::Custom11,
        CampaignValue::Custom12,
        CampaignValue::Custom13,
        CampaignValue::Custom14,
        CampaignValue::Custom15,
        CampaignValue::Custom16,
        CampaignValue::Custom17,
        CampaignValue::Custom18,
        CampaignValue::Custom19,
        CampaignValue::Custom20,
    ];
    assert_eq!(
        trace.values.len(),
        VALUE_KEYS.len(),
        "campaign value table has the wrong cardinality"
    );

    let mut campaign = Campaign::default();
    for (key, value) in VALUE_KEYS.into_iter().zip(trace.values.iter().copied()) {
        campaign.values[key] = value;
    }
    campaign.ares = trace.ares;
    campaign.missions = trace
        .missions
        .iter()
        .map(|source| {
            let profile = profiles
                .missions
                .get(source.profile_index as usize)
                .unwrap_or_else(|| {
                    panic!(
                        "campaign mission profile index {} is out of range",
                        source.profile_index
                    )
                });
            assert_eq!(profile.id, source.profile_id, "mission profile ID mismatch");
            assert!(
                profile
                    .mission_filename
                    .eq_ignore_ascii_case(&source.mission)
                    && profile
                        .proto_level_filename
                        .eq_ignore_ascii_case(&source.proto_level),
                "mission profile {} names disagree with trace {}/{}",
                source.profile_index,
                source.mission,
                source.proto_level
            );
            Mission {
                age: source.age,
                blazon_price: source.blazon_price,
                status: match source.status {
                    0 => MissionStatus::Available,
                    1 => MissionStatus::Won,
                    2 => MissionStatus::Lost,
                    other => panic!("invalid campaign mission status {other}"),
                },
                profile_idx: Some(source.profile_index),
                ares_state_override: (source.ares_state_succeeded != profile.ares_state_succeeded)
                    .then_some(source.ares_state_succeeded),
                attempt_history: Default::default(),
            }
        })
        .collect();
    let mission_count = campaign.missions.len();
    let validate_mission_index = |index: usize| {
        assert!(
            index < mission_count,
            "campaign mission index {index} is out of range {mission_count}"
        );
        index
    };
    campaign.accessible_mission_indices = trace
        .accessible_mission_indices
        .iter()
        .copied()
        .map(validate_mission_index)
        .collect();
    campaign.pending_accessible_mission_indices = trace
        .pending_accessible_mission_indices
        .iter()
        .copied()
        .map(validate_mission_index)
        .collect();
    campaign.last_mission_idx = trace.last_mission_index.map(validate_mission_index);
    campaign.current_mission_idx = trace.current_mission_index.map(validate_mission_index);
    campaign.next_mission_idx = trace.next_mission_index.map(validate_mission_index);
    campaign.blazon_mission_idx = trace.blazon_mission_index.map(validate_mission_index);
    let recent_launches: Vec<usize> = trace
        .last_played_mission_indices
        .iter()
        .copied()
        .map(validate_mission_index)
        .collect();
    campaign.reconstruct_original_save_history(&recent_launches);
    campaign.last_pseudo_mission_status = match trace.last_pseudo_mission_status {
        0 => MissionStatus::Available,
        1 => MissionStatus::Won,
        2 => MissionStatus::Lost,
        other => panic!("invalid last pseudo mission status {other}"),
    };
    campaign.last_pseudo_mission_id = trace.last_pseudo_mission_id;

    campaign.characters = trace
        .characters
        .iter()
        .map(|source| {
            let profile = profiles
                .characters
                .get(source.profile_index as usize)
                .unwrap_or_else(|| {
                    panic!(
                        "campaign character profile index {} is out of range",
                        source.profile_index
                    )
                });
            assert_eq!(
                profile.profile_name, source.profile_name,
                "character profile name mismatch"
            );
            PcDescription {
                character_profile_idx: Some(CharacterProfileIdx(source.profile_index)),
                instanced: source.instanced,
                status: PcStatus {
                    human_status: HumanStatus {
                        hand_to_hand: Skill {
                            capacity: source.status.hand_to_hand.capacity,
                            experience: source.status.hand_to_hand.experience,
                        },
                        bow: Skill {
                            capacity: source.status.bow.capacity,
                            experience: source.status.bow.experience,
                        },
                    },
                    life_points: source.status.life_points,
                    in_coma: source.status.in_coma,
                    num_ales: source.status.ales,
                    num_arrows: source.status.arrows,
                    num_apples: source.status.apples,
                    num_rations: source.status.rations,
                    num_stones: source.status.stones,
                    num_wasp_nests: source.status.wasp_nests,
                    num_nets: source.status.nets,
                    num_plants: source.status.plants,
                    num_purses: source.status.purses,
                    name: source.status.name.clone(),
                    name_override: None,
                    beam_me_index_in_sherwood: source.status.beam_me_index_in_sherwood,
                },
            }
        })
        .collect();
    let character_count = campaign.characters.len();
    let validate_character_index = |index: usize| {
        assert!(
            index < character_count,
            "campaign character index {index} is out of range {character_count}"
        );
        index
    };
    campaign.gang_indices = trace
        .gang_indices
        .iter()
        .copied()
        .map(validate_character_index)
        .collect();
    campaign.reservist_indices = trace
        .reservist_indices
        .iter()
        .copied()
        .map(validate_character_index)
        .collect();
    campaign.mission_team_indices = trace
        .mission_team_indices
        .iter()
        .copied()
        .map(validate_character_index)
        .collect();
    campaign.peasant_names = trace.peasant_names.clone();
    campaign.reservists_are_back = trace.reservists_are_back;
    campaign.collected_relics = trace.collected_relics.clone();
    campaign.production_sectors = trace
        .production_sectors
        .iter()
        .map(|source| SectorProduction {
            prod_type: match source.r#type {
                0 => Type::MakeArrow,
                1 => Type::MakePurse,
                2 => Type::MakeStone,
                3 => Type::MakeApple,
                4 => Type::MakeAle,
                5 => Type::MakeLamblegg,
                6 => Type::MakePlant,
                7 => Type::MakeNet,
                8 => Type::MakeWaspNest,
                9 => Type::TrainBow,
                10 => Type::TrainHandToHand,
                11 => Type::Heal,
                12 => Type::Relic,
                other => panic!("invalid campaign production type {other}"),
            },
            script_zone: None,
            speed: source.speed,
            production_points: Vec::new(),
            occupants: source
                .occupants
                .iter()
                .map(|occupant| Occupant {
                    pc_description_idx: validate_character_index(occupant.character_index),
                    x: occupant.x.value(),
                    y: occupant.y.value(),
                    obstacle:
                        robin_engine::position_interface::ObstacleHandle::from_serialized_pointer(
                            occupant.obstacle,
                        ),
                })
                .collect(),
            amount: source.amount,
            produced_amount: source.produced_amount,
            max_amount_reached: source.max_amount_reached,
        })
        .collect();

    campaign
}

#[cfg(not(feature = "client"))]
fn initialize_headless_engine(
    header: &TraceHeader,
    rng_prefix: Vec<u32>,
) -> (Engine, LevelAssets, robin_engine::scb::ScbFile) {
    let mut profile_manager = robin_engine::profiles::ProfileManager::new();
    let mut cpf = robin_engine::sbfile::SbFile::open(
        "Data/Configuration/profile.cpf",
        robin_engine::sbfile::SB_FILE_READ,
    )
    .expect("open profile.cpf");
    profile_manager
        .load_all_legacy_cpf(&mut cpf)
        .expect("parse profile.cpf");
    profile_manager.import_beam_mes("Data/Levels");

    let campaign = restore_campaign(&header.campaign, &profile_manager);
    let mission_idx = campaign
        .current_mission_idx
        .expect("recorded campaign has no current mission");
    let current_profile = campaign.missions[mission_idx].profile(&profile_manager);
    assert!(
        current_profile
            .mission_filename
            .eq_ignore_ascii_case(&header.mission)
            && current_profile
                .proto_level_filename
                .eq_ignore_ascii_case(&header.proto_level),
        "trace header mission/proto {}/{} disagrees with campaign current mission {}/{}",
        header.mission,
        header.proto_level,
        current_profile.mission_filename,
        current_profile.proto_level_filename
    );

    let profiles = Arc::new(profile_manager);
    let mut assets = LevelAssets::new();
    // The runner has already prepared its datadir and locale mounts. Capture
    // that tool authority once; replay execution must not consult live globals.
    assets.sprite_scriptor = Arc::new(robin_engine::sprite_script::SpriteScriptor::legacy_tool());
    assets.profile_manager = profiles.clone();
    crate::populate_localized_names(&mut assets)
        .expect("load localized names for deterministic PC construction");

    let mut frame_holder = robin_assets::frame_holder::FrameHolder::new();
    frame_holder
        .initialize_sprite_bank(".")
        .expect("initialize sprite bank");
    assets.bank_signature = frame_holder.signature();

    let mission_name = campaign.missions[mission_idx]
        .profile(&profiles)
        .mission_filename
        .clone();
    let script_path = format!("Data/Levels/{mission_name}.scb");
    let bytes = robin_engine::sbfile::SbFile::read_all(&script_path)
        .unwrap_or_else(|status| panic!("read mission script {script_path}: status {status}"));
    let scb = robin_assets::scb::parse_bytes(&bytes).expect("parse mission script");
    assets.scripts.mission_programs = Arc::new(BTreeMap::from([(
        mission_name,
        Arc::new(
            robin_engine::script_manager::ScriptProgram::from_scb(scb.clone())
                .expect("prepare mission script bytecode"),
        ),
    )]));

    let loaded = robin_engine::engine::level_loading::load_mission_for_campaign(
        &campaign,
        &profiles,
        "Data/Levels",
        &mut |_| {},
    )
    .expect("load mission");
    let ambiance = robin_engine::engine::Ambiance::from_raw(loaded.mission.header.ambiance);
    let bg_pixel_dims = crate::background_dimensions(
        &loaded.mission.header.map_filename,
        ambiance.directory(),
        "Data/Levels",
    )
    .expect("read background dimensions");

    let engine = Engine::new(robin_engine::engine::EngineArgs {
        campaign,
        level: robin_engine::engine::LevelLoadArgs {
            assets: &mut assets,
            level_directory: "Data/Levels",
            progress: &mut |_| {},
            loaded,
            bg_pixel_dims,
        },
        ground_mark_sprite: None,
        titbit_row_frame_counts: Vec::new(),
        rng_seed: header.rng_seed,
        original_rng_replay: Some(rng_prefix),
        sim_config: header
            .sim_config
            .to_sim_config(header.synchronous_pathfinding),
    })
    .expect("initialize engine");
    crate::populate_sound_duration_tables(&mut assets, &profiles, "Data/Sounds")
        .expect("load deterministic sound duration tables");
    assets.pixel_opacity = Some(Arc::new(frame_holder));
    (engine, assets, scb)
}

#[cfg(feature = "client")]
fn initialize_engine(
    header: &TraceHeader,
    rng_prefix: Vec<u32>,
) -> (
    Engine,
    LevelAssets,
    Host,
    robin_engine::engine::level_loading::PreDecodedBackground,
    robin_engine::scb::ScbFile,
    robin_rs::ingame_menu::resources::MenuText,
) {
    let mut pm = robin_engine::profiles::ProfileManager::new();
    let mut cpf = robin_engine::sbfile::SbFile::open(
        "Data/Configuration/profile.cpf",
        robin_engine::sbfile::SB_FILE_READ,
    )
    .expect("open profile.cpf");
    pm.load_all_legacy_cpf(&mut cpf).expect("parse profile.cpf");
    pm.import_beam_mes("Data/Levels");

    let campaign = restore_campaign(&header.campaign, &pm);
    let mission_idx = campaign
        .current_mission_idx
        .expect("recorded campaign has no current mission");
    let current_profile = campaign.missions[mission_idx].profile(&pm);
    assert!(
        current_profile
            .mission_filename
            .eq_ignore_ascii_case(&header.mission)
            && current_profile
                .proto_level_filename
                .eq_ignore_ascii_case(&header.proto_level),
        "trace header mission/proto {}/{} disagrees with campaign current mission {}/{}",
        header.mission,
        header.proto_level,
        current_profile.mission_filename,
        current_profile.proto_level_filename
    );

    let profiles = Arc::new(pm);
    let mut assets = LevelAssets::new();
    // Keep the client-backed parity path on the same explicit, pinned tool
    // resource boundary as the headless runner.
    assets.sprite_scriptor = Arc::new(robin_engine::sprite_script::SpriteScriptor::legacy_tool());
    assets.profile_manager = profiles.clone();
    let mut text_res = robin_assets::resource_manager::ResourceManager::legacy_tool();
    text_res
        .attach_resource_file("Data/Text/Level.res")
        .expect("load Data/Text/Level.res for Original rescue-PC names");
    (assets.peasant_firstnames, assets.peasant_surnames) =
        robin_rs::game_session::load_peasant_name_pool(&mut text_res);
    assets.fixed_vip_names = robin_rs::game_session::load_fixed_vip_name_map(&mut text_res);
    let _ = text_res.attach_resource_file("Data/Interface/Start.sxt");
    let menu_text = robin_rs::ingame_menu::resources::MenuText::load(&mut text_res);
    let mut host = Host::scratch(1024.0, 768.0);
    host.frontend
        .frame_holder_mut()
        .initialize_sprite_bank(".")
        .expect("initialize sprite bank");
    assets.bank_signature = host.frontend.frame_holder.signature();

    let mission_name = campaign.missions[mission_idx]
        .profile(&profiles)
        .mission_filename
        .clone();
    let script_path = format!("Data/Levels/{mission_name}.scb");
    let resolved =
        robin_engine::sbfile::resolve_case_insensitive(std::path::Path::new(&script_path))
            .unwrap_or_else(|| PathBuf::from(&script_path));
    let bytes = std::fs::read(&resolved)
        .unwrap_or_else(|e| panic!("read mission script {}: {e}", resolved.display()));
    let scb = robin_assets::scb::parse_bytes(&bytes).expect("parse mission script");
    assets.scripts.mission_programs = Arc::new(std::collections::BTreeMap::from([(
        mission_name,
        Arc::new(
            robin_engine::script_manager::ScriptProgram::from_scb(scb.clone())
                .expect("prepare mission script bytecode"),
        ),
    )]));

    let loaded = robin_engine::engine::level_loading::load_mission_for_campaign(
        &campaign,
        &profiles,
        "Data/Levels",
        &mut |_| {},
    )
    .expect("load mission");
    let ambiance = robin_engine::engine::Ambiance::from_raw(loaded.mission.header.ambiance)
        .directory()
        .to_string();
    let background = robin_rs::level_loading_host::pre_decode_background_map(
        &loaded.mission.header.map_filename,
        &ambiance,
        "Data/Levels",
        None,
        &mut |_| {},
    )
    .expect("decode background map")
    .expect("mission has no background map");
    let bg_pixel_dims = (background.width as f32, background.height as f32);

    let engine = Engine::new(robin_engine::engine::EngineArgs {
        campaign,
        level: robin_engine::engine::LevelLoadArgs {
            assets: &mut assets,
            level_directory: "Data/Levels",
            progress: &mut |_| {},
            loaded,
            bg_pixel_dims,
        },
        ground_mark_sprite: None,
        titbit_row_frame_counts: Vec::new(),
        rng_seed: header.rng_seed,
        original_rng_replay: Some(rng_prefix),
        sim_config: header
            .sim_config
            .to_sim_config(header.synchronous_pathfinding),
    })
    .expect("initialize engine");
    robin_rs::game_session::setup_mission_audio_for_tool(
        &mut host,
        &engine,
        &mut assets,
        &profiles,
        "Data/Sounds",
    );
    // The original game's target-sprite creation writes the active bank frame's native
    // dimensions into the serialized sprite frontier. The parity engine is
    // intentionally headless, so publish the immutable frame metadata used
    // to project that post-render state without mutating the simulation.
    assets.pixel_opacity = Some(host.frontend.publish_frame_holder_opacity());
    (engine, assets, host, background, scb, menu_text)
}

struct EntityMap {
    /// Per-frame Original array index to Rust entity. Original array indices
    /// can shift after a physical removal, so this is a view rather than the
    /// durable identity registry.
    entities: BTreeMap<TraceEntityId, EntityId>,
    /// Original's immutable per-engine construction serial is the durable
    /// identity anchor for refreshing the per-frame raw-index view.
    entities_by_creation_order: BTreeMap<u32, EntityId>,
    /// Original sparse fast-grid sector slot to this implementation's compact
    /// canonical position-sector number. The raw numbers are allocation
    /// details rather than gameplay identity.
    sectors: BTreeMap<u16, u16>,
    /// Original sparse sector slot to the exact Rust FastFindGrid arena slot.
    /// Public sector numbers are insufficient when retained overlays share an
    /// identity.
    sector_indices: BTreeMap<u16, robin_engine::fast_find_grid::SectorIndex>,
    /// Original mixed gate-array slot to Rust's runtime door-table index.
    gates: Vec<robin_engine::gate::DoorIndex>,
    /// One past the highest creation order the mission start established.
    /// Below it the Original's serials come from the mission file or the save
    /// and are exact; at or above it they also count the throwaway elements
    /// described on [`Self::extend_runtime_entities`], so only their relative
    /// order is comparable.
    runtime_creation_order_boundary: u32,
}

impl EntityMap {
    /// Build the exact one-to-one correspondence from Original's immutable
    /// per-engine construction serial.
    ///
    /// The trace frame is post-tick while this runs against Rust's pre-tick
    /// state, so position, active state and posture are all invalid identity
    /// labels here. In particular, several inactive beam PCs can be colocated
    /// and then start moving on the first recorded frame. Both mission loading
    /// and legacy-save adoption install the authoritative Original creation
    /// order on every Rust entity, including gaps consumed by mobile masters.
    fn build(engine: &Engine, assets: &LevelAssets, frame: &TraceFrame) -> Self {
        let mut rust_by_creation_order = BTreeMap::new();
        for (id, entity) in engine.entities_with_ids_iter() {
            let creation_order = engine.original_creation_order(id);
            if let Some((previous, _)) =
                rust_by_creation_order.insert(creation_order, (id, entity.entity_id_kind()))
            {
                panic!(
                    "Rust entities {previous:?} and {id:?} share Original creation order \
                     {creation_order}"
                );
            }
        }
        assert_eq!(
            frame.elements.len(),
            rust_by_creation_order.len(),
            "entity tables have different cardinality"
        );

        let mut result = BTreeMap::new();
        let mut entities_by_creation_order = BTreeMap::new();
        for original in &frame.elements {
            let expected_kind = EntityIdKind::from(original.entity_id.kind);
            let &(rust_id, actual_kind) = rust_by_creation_order
                .get(&original.creation_order)
                .unwrap_or_else(|| {
                    panic!(
                        "Original {:?} has creation order {}, absent from the Rust identity table",
                        original.entity_id, original.creation_order
                    )
                });
            assert_eq!(
                actual_kind, expected_kind,
                "Original {:?} creation order {} has kind {:?}, but Rust {rust_id:?} has kind \
                 {actual_kind:?}",
                original.entity_id, original.creation_order, expected_kind,
            );
            assert!(
                result.insert(original.entity_id, rust_id).is_none(),
                "Original trace entity {:?} occurs twice",
                original.entity_id
            );
            assert!(
                entities_by_creation_order
                    .insert(original.creation_order, rust_id)
                    .is_none(),
                "Original creation order {} occurs twice in the trace",
                original.creation_order
            );
        }
        let retained = assets
            .legacy_grid_topology
            .as_ref()
            .expect("parity replay requires retained Original fast-grid topology");
        let mut sectors = BTreeMap::new();
        let mut sector_indices = BTreeMap::new();
        for (original, (runtime, runtime_index)) in retained
            .position_sector_numbers
            .iter()
            .zip(&retained.position_sector_indices)
            .enumerate()
        {
            let Some(runtime) = runtime else {
                assert!(
                    runtime_index.is_none(),
                    "Original sparse sector slot {original} has an arena index but no public number"
                );
                continue;
            };
            let runtime_index = runtime_index.unwrap_or_else(|| {
                panic!("Original sparse sector slot {original} has a public number but no exact Rust arena index")
            });
            let original = u16::try_from(original)
                .expect("Original sparse sector slot exceeds its u16 identity domain");
            let runtime =
                u16::try_from(*runtime).expect("Rust canonical sector number is negative");
            assert!(
                sectors.insert(original, runtime).is_none(),
                "Original sparse sector slot {original} was mapped twice"
            );
            assert!(
                sector_indices.insert(original, runtime_index).is_none(),
                "Original sparse sector slot {original} had two exact arena mappings"
            );
        }
        let runtime_creation_order_boundary = entities_by_creation_order
            .keys()
            .next_back()
            .map_or(0, |highest| highest + 1);
        Self {
            entities: result,
            entities_by_creation_order,
            sectors,
            sector_indices,
            gates: engine.legacy_gate_order(assets),
            runtime_creation_order_boundary,
        }
    }

    /// Whether the Original's raw serial for this element is an exact
    /// cross-engine value rather than one carrying presentation-only gaps.
    fn creation_order_is_exact(&self, creation_order: u32) -> bool {
        creation_order < self.runtime_creation_order_boundary
    }

    fn refresh_trace_indices(&mut self, frame: &TraceFrame) {
        for original in &frame.elements {
            self.refresh_trace_index(original.entity_id, original.creation_order);
        }
    }

    /// Verify hidden-building occupants against the construction-topology
    /// isomorphism. The mapping itself must not be learned from mutable frame
    /// state: an initially active tenant can cross the first-frame comparison
    /// boundary before any inactive occupant exposes that building.
    fn validate_building_sector_mapping(&self, engine: &Engine, frame: &TraceFrame) {
        for original in frame
            .elements
            .iter()
            .filter(|element| element.actor.is_some() && !element.active)
        {
            let Some(&rust_id) = self.entities.get(&original.entity_id) else {
                continue;
            };
            let Some(actual) = engine.get_entity(rust_id) else {
                continue;
            };
            let actual_element = actual.element_data();
            if actual_element.active || !actual_element.hidden_in_building {
                continue;
            }
            let Some(actual_sector) = actual_element.sector().map(|sector| sector.get()) else {
                continue;
            };
            let expected_position: MapPoint = original.position_map.into();
            let actual_position = actual_element.position_map();
            if expected_position.x.to_bits() != actual_position.x.to_bits()
                || expected_position.y.to_bits() != actual_position.y.to_bits()
                || original.sector == actual_sector
            {
                continue;
            }
            assert_eq!(
                self.sectors.get(&original.sector),
                Some(&actual_sector),
                "hidden building occupant exposed a sector identity absent from retained topology"
            );
        }
    }

    fn refresh_trace_index(&mut self, trace_id: TraceEntityId, creation_order: u32) {
        if let Some(rust_id) = self
            .entities_by_creation_order
            .get(&creation_order)
            .copied()
        {
            self.entities.insert(trace_id, rust_id);
        }
    }

    /// Extend the mission-start bijection for persistent entities created while
    /// replaying the mission.
    ///
    /// Runtime creation orders are not exact cross-engine identities. Original
    /// cursor previews construct temporary element projectiles (for
    /// example the temporary arrow in trajectory validation), consuming
    /// global construction orders without ever adding those objects to the
    /// world. Rust computes the same presentation preview without constructing
    /// an entity. Match newly persistent entities isomorphically by global
    /// persistent construction rank and require the concrete kind at every
    /// rank to agree; raw order numbers may differ by presentation-only gaps.
    fn extend_runtime_entities(&mut self, engine: &Engine, frame: &TraceFrame) {
        self.refresh_trace_indices(frame);
        let originals: Vec<_> = frame
            .elements
            .iter()
            .filter(|element| {
                !self
                    .entities_by_creation_order
                    .contains_key(&element.creation_order)
            })
            .collect();
        let used: BTreeSet<_> = self.entities_by_creation_order.values().copied().collect();
        let mut rust_by_creation_order = BTreeMap::new();
        for (id, entity) in engine
            .entities_with_ids_iter()
            .filter(|(id, _)| !used.contains(id))
        {
            let creation_order = engine.original_creation_order(id);
            if let Some((previous, _)) =
                rust_by_creation_order.insert(creation_order, (id, entity.entity_id_kind()))
            {
                panic!(
                    "unmapped Rust entities {previous:?} and {id:?} share Original creation \
                     order {creation_order}"
                );
            }
        }
        let original_identities: Vec<_> = originals
            .iter()
            .map(|element| {
                (
                    element.entity_id,
                    element.creation_order,
                    EntityIdKind::from(element.entity_id.kind),
                )
            })
            .collect();
        let rust_identities: Vec<_> = rust_by_creation_order
            .iter()
            .map(|(&creation_order, &(id, kind))| (id, creation_order, kind))
            .collect();
        let pairs = pair_runtime_identities_by_persistent_rank(
            original_identities.clone(),
            rust_identities.clone(),
        )
        .unwrap_or_else(|detail| {
            panic!(
                "runtime persistent entity identity mismatch: {detail}; \
                 Original={original_identities:?}; Rust={rust_identities:?}"
            )
        });

        let originals_by_id: BTreeMap<_, _> = originals
            .into_iter()
            .map(|original| (original.entity_id, original))
            .collect();
        for (original_id, original_creation_order, rust_id) in pairs {
            let original = originals_by_id
                .get(&original_id)
                .expect("runtime identity pairing returned an unknown Original entity");
            debug_assert_eq!(original.creation_order, original_creation_order);
            self.entities.insert(original.entity_id, rust_id);
            assert!(
                self.entities_by_creation_order
                    .insert(original.creation_order, rust_id)
                    .is_none(),
                "Original creation order {} was mapped twice",
                original.creation_order
            );
        }
    }

    fn translate(&self, original: TraceEntityId) -> EntityId {
        *self
            .entities
            .get(&original)
            .unwrap_or_else(|| panic!("original entity {original:?} has no Rust correspondence"))
    }

    fn translate_gate(&self, original: u32) -> u32 {
        let original_index = usize::try_from(original)
            .unwrap_or_else(|_| panic!("Original gate index {original} exceeds usize"));
        self.gates
            .get(original_index)
            .copied()
            .unwrap_or_else(|| {
                panic!("Original gate index {original} is absent from the retained gate topology")
            })
            .into()
    }

    /// Preserve the patch-aware goal sector recorded by group movement.
    /// A retained canonical position sector is translated normally. A true
    /// unmapped initial value remains an authoritative route goal in the original game's
    /// sparse identity domain: keeping
    /// its number forces the same movement-sequence gate search instead of
    /// silently substituting Rust's coincident selected-sector overlay.
    fn translate_group_move_goal_sector(
        &self,
        original: i16,
        layer: u16,
        unmapped_goal_search_sector: Option<u16>,
    ) -> GroupMoveGoalTranslation {
        let original = u16::try_from(original)
            .unwrap_or_else(|_| panic!("Original group-move sector is negative: {original}"));
        if let Some(&runtime) = self.sectors.get(&original) {
            let runtime = i16::try_from(runtime).unwrap_or_else(|_| {
                panic!("Rust position sector {runtime} exceeds its signed identity domain")
            });
            let index = *self.sector_indices.get(&original).unwrap_or_else(|| {
                panic!("mapped Original group-move sector {original} lost its exact Rust arena identity")
            });
            GroupMoveGoalTranslation::Runtime((SectorNumber::new(runtime), layer), index)
        } else if let Some(search_sector) = unmapped_goal_search_sector {
            let runtime = self.sectors.get(&search_sector).copied().unwrap_or_else(|| {
                panic!(
                    "successful group-move route terminal Original sector {search_sector} has no retained Rust position-sector mapping"
                )
            });
            let runtime = i16::try_from(runtime).unwrap_or_else(|_| {
                panic!("Rust position sector {runtime} exceeds its signed identity domain")
            });
            let index = *self.sector_indices.get(&search_sector).unwrap_or_else(|| {
                panic!("mapped group-move terminal sector {search_sector} lost its exact Rust arena identity")
            });
            GroupMoveGoalTranslation::Runtime((SectorNumber::new(runtime), layer), index)
        } else {
            let recorded = i16::try_from(original).unwrap_or_else(|_| {
                panic!("Original group-move sector {original} exceeds its signed identity domain")
            });
            GroupMoveGoalTranslation::RecordedUnmapped((SectorNumber::new(recorded), layer))
        }
    }

    fn translate_required_drop_ale_goal_sector(
        &self,
        original: u16,
    ) -> (SectorNumber, robin_engine::fast_find_grid::SectorIndex) {
        let runtime = self.sectors.get(&original).copied().unwrap_or_else(|| {
            panic!(
                "schema-16 DropAle route goal Original sector {original} has no retained Rust position-sector mapping"
            )
        });
        let runtime = i16::try_from(runtime).unwrap_or_else(|_| {
            panic!("Rust DropAle goal sector {runtime} exceeds its signed identity domain")
        });
        let index = *self.sector_indices.get(&original).unwrap_or_else(|| {
            panic!(
                "mapped schema-16 DropAle route goal Original sector {original} lost its exact Rust arena identity"
            )
        });
        (SectorNumber::new(runtime), index)
    }

    fn sectors_equivalent(&self, original: u16, rust: u16) -> bool {
        original == rust || self.sectors.get(&original) == Some(&rust)
    }

    fn translate_sector(&self, original: u16) -> u16 {
        self.sectors.get(&original).copied().unwrap_or(original)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GroupMoveGoalTranslation {
    Runtime(
        (SectorNumber, u16),
        robin_engine::fast_find_grid::SectorIndex,
    ),
    RecordedUnmapped((SectorNumber, u16)),
}

fn pair_runtime_identities_by_persistent_rank(
    mut originals: Vec<(TraceEntityId, u32, EntityIdKind)>,
    mut rust: Vec<(EntityId, u32, EntityIdKind)>,
) -> Result<Vec<(TraceEntityId, u32, EntityId)>, String> {
    originals.sort_by_key(|(_, creation_order, _)| *creation_order);
    rust.sort_by_key(|(_, creation_order, _)| *creation_order);
    if originals.len() != rust.len() {
        return Err(format!(
            "different persistent cardinality (Original {}, Rust {})",
            originals.len(),
            rust.len()
        ));
    }

    originals
        .into_iter()
        .zip(rust)
        .enumerate()
        .map(
            |(
                rank,
                ((original_id, original_order, original_kind), (rust_id, rust_order, rust_kind)),
            )| {
                if original_kind != rust_kind {
                    return Err(format!(
                        "persistent creation rank {rank} has Original {original_kind:?} order \
                         {original_order}, but Rust {rust_kind:?} order {rust_order}"
                    ));
                }
                Ok((original_id, original_order, rust_id))
            },
        )
        .collect()
}

impl From<TraceEntityKind> for EntityIdKind {
    fn from(value: TraceEntityKind) -> Self {
        match value {
            TraceEntityKind::Pc => Self::Pc,
            TraceEntityKind::Soldier => Self::Soldier,
            TraceEntityKind::Civilian => Self::Civilian,
            TraceEntityKind::Fx => Self::Fx,
            TraceEntityKind::Target => Self::Target,
            TraceEntityKind::Bonus => Self::Bonus,
            TraceEntityKind::Scroll => Self::Scroll,
            TraceEntityKind::Projectile => Self::Projectile,
            TraceEntityKind::Net => Self::Net,
        }
    }
}

/// Dumps the order/sprite motion bookkeeping of one actor around a frame
/// boundary. Selected with `PARITY_DEBUG_ELEMENT=<kind>:<index>`, where kind is
/// `pc`, `soldier` or `civilian` and index is the Rust entity index — the same
/// pair the divergence report prints as `Pc(PcId(103))`. The window is
/// `PARITY_DEBUG_FROM`..=`PARITY_DEBUG_UNTIL`.
fn print_debug_element(label: &str, engine: &Engine, frame: &TraceFrame) {
    let Some(spec) = std::env::var_os("PARITY_DEBUG_ELEMENT") else {
        return;
    };
    let frame_bound = |name: &str, fallback: u64| {
        std::env::var(name)
            .map(|value| {
                value
                    .parse::<u64>()
                    .unwrap_or_else(|_| panic!("{name} must be a u64"))
            })
            .unwrap_or(fallback)
    };
    let from = frame_bound("PARITY_DEBUG_FROM", 0);
    let until = frame_bound("PARITY_DEBUG_UNTIL", 10);
    if frame.frame_after < from || frame.frame_after > until {
        return;
    }
    let spec = spec.to_string_lossy().to_string();
    let (kind, index) = spec
        .split_once(':')
        .expect("PARITY_DEBUG_ELEMENT must look like pc:342");
    let index: u32 = index
        .parse()
        .expect("PARITY_DEBUG_ELEMENT index must be u32");
    let id = match kind {
        "pc" => EntityId::Pc(robin_engine::entity_id::PcId(index)),
        "soldier" => EntityId::Soldier(robin_engine::entity_id::SoldierId(index)),
        "civilian" => EntityId::Civilian(robin_engine::entity_id::CivilianId(index)),
        other => panic!("unsupported PARITY_DEBUG_ELEMENT kind {other}"),
    };
    // The slot may not exist yet on early frames; stay quiet until it does
    // rather than aborting the whole replay.
    let Some(entity) = engine.get_entity(id) else {
        return;
    };
    let sprite = &entity.element_data().sprite;
    let actor = entity.actor_data().expect("debug element is an actor");
    eprintln!(
        "{label} frame {} {:?} dir={} dir_goal={} posture={:?} action_state={:?} order={:?} installed={:?} last_processed_order={} actor_motion={:?} sprite_motion={:?} last_action={:?} row={} frame={}/{} command={:?} execute_init={} last_execute_order={:?}",
        frame.frame_after,
        id,
        entity.element_data().direction(),
        sprite.position_iface.get_direction_goal().as_u8(),
        entity.element_data().posture,
        actor.action_state,
        engine.actor_order_type(id),
        actor.installed_order.map(|order| order.order_id),
        sprite.last_processed_order_id,
        actor.continuation.motion_state,
        sprite.last_motion_state,
        sprite.last_action,
        sprite.current_row,
        sprite.current_frame,
        sprite.frame_count,
        engine.actor_command(id),
        actor.execute_order_initialising,
        actor.last_execute_order_id,
    );
}

/// Opt-in lifecycle diagnostic paired with Original's
/// `record_frame_pre_serialize` hook. It observes, but never changes, the
/// projectile state that the parity comparison is about to publish.
fn record_arrow_publication_before_compare(
    engine: &Engine,
    frame: &TraceFrame,
    entity_map: &EntityMap,
) {
    if std::env::var_os("PARITY_DEBUG_ARROW_PUBLICATION").is_none() {
        return;
    }
    let parse_filter = |name: &str| {
        std::env::var(name).ok().map(|value| {
            value.parse::<u32>().unwrap_or_else(|error| {
                panic!("invalid {name}={value:?} for arrow publication diagnostic: {error}")
            })
        })
    };
    if parse_filter("PARITY_DEBUG_ARROW_PUBLICATION_FRAME_AFTER")
        .is_some_and(|value| u64::from(value) != frame.frame_after)
    {
        return;
    }
    let projectile_filter =
        parse_filter("PARITY_DEBUG_ARROW_PUBLICATION_PROJECTILE_CREATION_ORDER");
    let shooter_filter = parse_filter("PARITY_DEBUG_ARROW_PUBLICATION_SHOOTER_CREATION_ORDER");

    for original in frame.elements.iter().filter(|element| {
        element.kind == TraceEntityKind::Projectile
            && projectile_filter.is_none_or(|value| value == element.creation_order)
    }) {
        let id = entity_map.translate(original.entity_id);
        let entity = engine
            .get_entity(id)
            .unwrap_or_else(|| panic!("mapped diagnostic projectile {id:?} is missing"));
        let Entity::Projectile(arrow) = entity else {
            panic!("mapped diagnostic projectile {id:?} changed entity kind");
        };
        if arrow.object.object_type != robin_engine::element_kinds::ObjectType::Arrow {
            continue;
        }
        let shooter = arrow
            .projectile
            .shooter
            .expect("diagnostic arrow is missing its required shooter");
        let shooter_creation_order = engine.original_creation_order(shooter);
        if shooter_filter.is_some_and(|value| value != shooter_creation_order) {
            continue;
        }
        let sprite = &arrow.element.sprite;
        let position = sprite.position_iface.get_position();
        eprintln!(
            "PARITY_ARROW_PUBLICATION_RUST stage=record_frame_pre_compare frame_after={} \
             projectile_creation_order={} shooter_creation_order={} active={} flying={} \
             falling={} trajectory_size={} row={} frame={} frame_count={} \
             position_bits=[{:08x},{:08x},{:08x}]",
            frame.frame_after,
            original.creation_order,
            shooter_creation_order,
            arrow.element.active,
            arrow.projectile.flying,
            arrow.projectile.falling,
            arrow.projectile.trajectory.len(),
            sprite.current_row,
            sprite.current_frame,
            sprite.frame_count,
            position.x.to_bits(),
            position.y.to_bits(),
            position.z.to_bits(),
        );
    }
}

fn print_startup_actors(label: &str, engine: &Engine, frame: &TraceFrame, entity_map: &EntityMap) {
    eprintln!("{label}:");
    let expected_inactive: Vec<_> = frame
        .elements
        .iter()
        .filter(|element| element.actor.is_some() && !element.active)
        .map(|element| element.entity_id)
        .collect();
    let rust_inactive: Vec<_> = engine
        .entities_with_ids_iter()
        .filter(|(_, entity)| entity.actor_data().is_some() && !entity.element_data().active)
        .map(|(id, _)| id)
        .collect();
    eprintln!("  expected inactive actors: {expected_inactive:?}");
    eprintln!("  Rust inactive actors: {rust_inactive:?}");
    let rust_hidden: Vec<_> = engine
        .entities_with_ids_iter()
        .filter(|(_, entity)| entity.element_data().hidden_in_building)
        .map(|(id, entity)| {
            (
                id,
                entity.element_data().sector(),
                entity.element_data().layer(),
                entity.element_data().position_map(),
            )
        })
        .collect();
    eprintln!("  Rust script-hidden actors: {rust_hidden:?}");
    for expected in frame.elements.iter().filter(|element| {
        element.actor.is_some()
            && (element.posture == robin_engine::element::Posture::Sitting as u32
                || element.entity_id
                    == TraceEntityId {
                        kind: TraceEntityKind::Soldier,
                        index: 119,
                    })
    }) {
        let id = entity_map.translate(expected.entity_id);
        let actual = engine
            .get_entity(id)
            .unwrap_or_else(|| panic!("mapped startup actor {id:?} is missing"));
        let ai_debug = actual.ai_controller().map(|ai| {
            (
                ai.current_state,
                ai.current_substate,
                ai.already_on_point,
                ai.last_goto_destination,
                ai.initial_view_direction,
                ai.outbox.reentrant.self_stimuli.clone(),
                ai.outbox.actor.launch_commands.clone(),
                ai.outbox.actor.orders.clone(),
            )
        });
        eprintln!(
            "  original={:?} rust={id:?} expected_posture={} rust_posture={:?} expected_dir={}/{} rust_dir={:?}/{:?} pos={:?} goal_pos={:?} action={:?} last_action={:?} alt_profile={} command={:?} order={:?} sector={:?} ai={ai_debug:?}",
            expected.entity_id,
            expected.posture,
            actual.element_data().posture,
            expected.direction,
            expected.direction_goal,
            actual.element_data().sprite.position_iface.get_direction(),
            actual
                .element_data()
                .sprite
                .position_iface
                .get_direction_goal(),
            actual.element_data().position_map(),
            actual.element_data().sprite.position_iface.map_goal(),
            actual.actor_data().map(|actor| actor.action_state),
            actual.element_data().sprite.last_action,
            actual.element_data().sprite.use_alternate_profile,
            engine.actor_command(id),
            engine.actor_order_type(id),
            actual.element_data().sector(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcode_parity as bitcode;

    #[test]
    fn late_movement_retranslation_drops_only_the_dangling_actor_animation() {
        let retransmitted = EntityId::Pc(robin_engine::entity_id::PcId(171));
        let unaffected = EntityId::Pc(robin_engine::entity_id::PcId(173));
        let late = [retransmitted];

        assert!(!original_actor_animation_is_logical(retransmitted, &late));
        assert!(original_actor_animation_is_logical(unaffected, &late));
    }

    fn lifecycle_event(
        actor_creation_order: u32,
        event: &str,
        phase: &str,
    ) -> TraceSequenceLifecycleEvent {
        TraceSequenceLifecycleEvent {
            ordinal: 0,
            frame_ordinal: 0,
            event: event.to_owned(),
            phase: phase.to_owned(),
            element_id: 1,
            sequence_id: Some(1),
            owner: None,
            owner_creation_order: None,
            command: 0,
            command_name: None,
            command_level: 1,
            state: None,
            priority: None,
            queue_size_before: None,
            queue_size_after: None,
            actor: None,
            actor_creation_order: Some(actor_creation_order),
            selected_sequence_id: None,
            selected_command: None,
            current_order_id: None,
            current_order_action: None,
            decision: None,
            accepted: Some(true),
        }
    }

    #[test]
    fn completed_during_translation_drops_only_stale_execution_telemetry() {
        let completed = [lifecycle_event(
            167,
            "actor_instruct_result",
            "completed_during_translation",
        )];
        assert!(!original_actor_execution_telemetry_is_logical(
            167, &completed,
        ));
        assert!(original_actor_execution_telemetry_is_logical(
            168, &completed,
        ));
    }

    #[test]
    fn original_motion_state_comparison_rejects_only_undefined_stack_values() {
        for raw in 0..=5 {
            assert!(original_motion_state_is_defined(raw));
        }
        assert!(!original_motion_state_is_defined(6));
        assert!(!original_motion_state_is_defined(1_494_023_856));
    }

    fn blocked_box_json(
        last_processed_order_id: u32,
        min_x: u32,
        min_y: u32,
        max_x: u32,
        max_y: u32,
    ) -> serde_json::Value {
        serde_json::json!({
            "sprite": {"last_processed_order_id": last_processed_order_id},
            "position": {
                "blocked_box": {
                    "min": {"x": {"bits": min_x}, "y": {"bits": min_y}},
                    "max": {"x": {"bits": max_x}, "y": {"bits": max_y}}
                }
            }
        })
    }

    fn deviated_blocked_box_json(
        last_processed_order_id: u32,
        map: (f32, f32),
        old_map: (f32, f32),
        deviated: bool,
        anti_collision_on: bool,
        blocked_count: u32,
    ) -> serde_json::Value {
        let half = 0.49_f32;
        serde_json::json!({
            "sprite": {"last_processed_order_id": last_processed_order_id},
            "position": {
                "anti_collision_on": anti_collision_on,
                "blocked_count": blocked_count,
                "deviated": deviated,
                "map": {
                    "x": {"bits": map.0.to_bits()},
                    "y": {"bits": map.1.to_bits()}
                },
                "old_map": {
                    "x": {"bits": old_map.0.to_bits()},
                    "y": {"bits": old_map.1.to_bits()}
                },
                "blocked_box": {
                    "min": {
                        "x": {"bits": (map.0 - half).to_bits()},
                        "y": {"bits": (map.1 - half).to_bits()}
                    },
                    "max": {
                        "x": {"bits": (map.0 + half).to_bits()},
                        "y": {"bits": (map.1 + half).to_bits()}
                    }
                }
            }
        })
    }

    fn trace_actor_order(
        action: robin_engine::order::OrderType,
        movement_sequence: bool,
        motion_state: robin_engine::sprite::MotionState,
        current_order_id: u32,
    ) -> TraceActor {
        TraceActor {
            action_state: 0,
            animation: action as u32,
            command: 22,
            command_name: "move_ok".to_owned(),
            motion_state: motion_state as u32,
            wait_time: 0,
            passing_door_directly: false,
            active_pass_door: None,
            sequence_element: Some(TraceSequenceElement {
                id: 1,
                element_type: 4,
                state: 2,
                command_level: 2,
                command: 22,
                command_name: "move_ok".to_owned(),
                order_count: 1,
                priority: 8,
                posture_after_transition: 1,
                action_state_after_transition: 0,
                movement: movement_sequence.then_some(TraceSequenceMovement {
                    action: None,
                    pass_door: None,
                }),
                following: None,
                postponed: None,
                current_order: Some(
                    serde_json::from_value(
                        serde_json::json!({"id": current_order_id, "action": action as u32}),
                    )
                    .expect("current-order fixture must decode"),
                ),
                movement_payload: None,
            }),
            position_interface: missing_legacy_trace_json_value(),
        }
    }

    #[test]
    fn legacy_blocked_box_reset_requires_perform_motion_execute_arm() {
        let soldier = EntityId::new(122, robin_engine::element::EntityIdKind::Soldier);
        assert!(!original_reset_blocked_box_this_frame(
            &trace_actor_order(
                robin_engine::order::OrderType::TransitionWaitingUprightBoredWaitingUpright,
                true,
                robin_engine::sprite::MotionState::Start,
                10,
            ),
            soldier,
            10,
            false,
            None,
            None,
            None,
        ));
        assert!(!original_reset_blocked_box_this_frame(
            &trace_actor_order(
                robin_engine::order::OrderType::TransitionWalkingUprightWaitingUpright,
                false,
                robin_engine::sprite::MotionState::Start,
                10,
            ),
            soldier,
            10,
            false,
            None,
            None,
            None,
        ));
        assert!(original_reset_blocked_box_this_frame(
            &trace_actor_order(
                robin_engine::order::OrderType::WalkingUpright,
                true,
                robin_engine::sprite::MotionState::Start,
                10,
            ),
            soldier,
            10,
            false,
            None,
            None,
            None,
        ));
        assert!(original_reset_blocked_box_this_frame(
            &trace_actor_order(
                robin_engine::order::OrderType::WalkingUpright,
                true,
                robin_engine::sprite::MotionState::InProgress,
                11,
            ),
            soldier,
            10,
            true,
            None,
            None,
            None,
        ));
        assert!(!original_reset_blocked_box_this_frame(
            &trace_actor_order(
                robin_engine::order::OrderType::WalkingUpright,
                true,
                robin_engine::sprite::MotionState::InProgress,
                10,
            ),
            soldier,
            10,
            true,
            None,
            None,
            None,
        ));
        assert!(original_reset_blocked_box_this_frame(
            &trace_actor_order(
                robin_engine::order::OrderType::WalkingUpright,
                true,
                robin_engine::sprite::MotionState::InProgress,
                10,
            ),
            soldier,
            10,
            true,
            Some(10),
            None,
            None,
        ));
        assert!(!original_reset_blocked_box_this_frame(
            &trace_actor_order(
                robin_engine::order::OrderType::WalkingUpright,
                true,
                robin_engine::sprite::MotionState::InProgress,
                11,
            ),
            soldier,
            10,
            false,
            None,
            None,
            None,
        ));
        let pc = EntityId::new(101, robin_engine::element::EntityIdKind::Pc);
        assert!(original_reset_blocked_box_this_frame(
            &trace_actor_order(
                robin_engine::order::OrderType::RunningWithSword,
                true,
                robin_engine::sprite::MotionState::InProgress,
                30,
            ),
            pc,
            30,
            false,
            Some(30),
            None,
            None,
        ));
        assert!(!original_reset_blocked_box_this_frame(
            &trace_actor_order(
                robin_engine::order::OrderType::RunningWithSword,
                true,
                robin_engine::sprite::MotionState::InProgress,
                30,
            ),
            pc,
            30,
            false,
            Some(29),
            None,
            None,
        ));
        assert!(!original_reset_blocked_box_this_frame(
            &trace_actor_order(
                robin_engine::order::OrderType::RunningWithSword,
                true,
                robin_engine::sprite::MotionState::InProgress,
                31,
            ),
            pc,
            30,
            false,
            Some(30),
            None,
            None,
        ));
        assert!(!original_reset_blocked_box_this_frame(
            &trace_actor_order(
                robin_engine::order::OrderType::RunningWithSword,
                false,
                robin_engine::sprite::MotionState::InProgress,
                30,
            ),
            pc,
            30,
            false,
            Some(30),
            None,
            None,
        ));
    }

    #[test]
    fn legacy_blocked_box_reset_recognizes_hidden_stop_movement_rewrite() {
        let soldier = EntityId::new(126, robin_engine::element::EntityIdKind::Soldier);
        let turn_after_stop = trace_actor_order(
            robin_engine::order::OrderType::TransitionWalkingUprightWaitingUpright,
            false,
            robin_engine::sprite::MotionState::InProgress,
            30,
        );
        let prior_walking = LegacyStoppableMotionOrder {
            id: 20,
            stop_animation: robin_engine::order::OrderType::TransitionWalkingUprightWaitingUpright
                as u32,
        };
        assert!(original_reset_blocked_box_this_frame(
            &turn_after_stop,
            soldier,
            25,
            false,
            None,
            Some(prior_walking),
            Some(20),
        ));
        assert!(!original_reset_blocked_box_this_frame(
            &turn_after_stop,
            soldier,
            25,
            false,
            None,
            Some(prior_walking),
            Some(19),
        ));
        assert!(!original_reset_blocked_box_this_frame(
            &turn_after_stop,
            soldier,
            30,
            false,
            None,
            Some(prior_walking),
            Some(20),
        ));
        assert!(!original_reset_blocked_box_this_frame(
            &turn_after_stop,
            soldier,
            25,
            true,
            None,
            Some(prior_walking),
            Some(20),
        ));

        for (action, stop_animation) in [
            (
                robin_engine::order::OrderType::WalkingUpright,
                robin_engine::order::OrderType::TransitionWalkingUprightWaitingUpright,
            ),
            (
                robin_engine::order::OrderType::RunningUpright,
                robin_engine::order::OrderType::TransitionRunningUprightWaitingUpright,
            ),
            (
                robin_engine::order::OrderType::WalkingCrouched,
                robin_engine::order::OrderType::TransitionWalkingCrouchedWaitingCrouched,
            ),
        ] {
            assert_eq!(
                original_stoppable_current_motion_order(&trace_actor_order(
                    action,
                    true,
                    robin_engine::sprite::MotionState::InProgress,
                    20,
                )),
                Some(LegacyStoppableMotionOrder {
                    id: 20,
                    stop_animation: stop_animation as u32,
                }),
            );
        }
        assert_eq!(
            original_stoppable_current_motion_order(&trace_actor_order(
                robin_engine::order::OrderType::WaitingUpright,
                true,
                robin_engine::sprite::MotionState::InProgress,
                20,
            )),
            None,
        );

        let mut movement_with_following = trace_actor_order(
            robin_engine::order::OrderType::WalkingUpright,
            true,
            robin_engine::sprite::MotionState::InProgress,
            20,
        );
        movement_with_following
            .sequence_element
            .as_mut()
            .unwrap()
            .order_count = 2;
        assert_eq!(
            original_stoppable_current_motion_order(&movement_with_following),
            Some(prior_walking),
            "stopping movement rewrites the current movement even when a following action remains",
        );
    }

    #[test]
    fn legacy_blocked_box_shadow_tracks_resets_and_revalidation() {
        let tuple = LegacyBlockedBoxTuple {
            min_x: 1,
            min_y: 2,
            max_x: 3,
            max_y: 4,
        };
        let mut shadows = BTreeMap::from([(
            10,
            LegacyBlockedBoxShadow {
                tuple,
                validity: LegacyBlockedBoxValidity::Set,
                last_processed_order_id: 20,
                pending_motion_order_id: None,
                stoppable_motion_order: None,
                deviated: None,
                direct_validity_observed: false,
            },
        )]);

        let mut active = blocked_box_json(20, 1, 2, 3, 4);
        assert!(!canonicalize_legacy_blocked_box(
            &mut active,
            10,
            false,
            None,
            None,
            &mut shadows,
        ));
        assert!(!active.pointer("/position/blocked_box").unwrap().is_null());

        let mut continuing_motion = blocked_box_json(20, 1, 2, 3, 4);
        assert!(!canonicalize_legacy_blocked_box(
            &mut continuing_motion,
            10,
            true,
            None,
            None,
            &mut shadows,
        ));
        assert_eq!(
            shadows[&10].validity,
            LegacyBlockedBoxValidity::Set,
            "MotionState::Start alone is insufficient without a new raw order id"
        );

        let mut pending_motion = blocked_box_json(20, 1, 2, 3, 4);
        assert!(!canonicalize_legacy_blocked_box(
            &mut pending_motion,
            10,
            true,
            Some(22),
            None,
            &mut shadows,
        ));
        assert_eq!(shadows[&10].pending_motion_order_id, Some(22));

        let mut began_pending_motion = blocked_box_json(22, 1, 2, 3, 4);
        assert!(canonicalize_legacy_blocked_box(
            &mut began_pending_motion,
            10,
            true,
            Some(22),
            None,
            &mut shadows,
        ));
        assert!(
            began_pending_motion
                .pointer("/position/blocked_box")
                .unwrap()
                .is_null(),
            "a previously observed pending motion order resets when it becomes processed"
        );

        // Re-establish a valid tuple for the remaining independent transition
        // cases below.
        shadows.get_mut(&10).unwrap().validity = LegacyBlockedBoxValidity::Set;

        let mut action_order = blocked_box_json(21, 1, 2, 3, 4);
        assert!(!canonicalize_legacy_blocked_box(
            &mut action_order,
            10,
            false,
            None,
            None,
            &mut shadows,
        ));
        assert_eq!(
            shadows[&10].validity,
            LegacyBlockedBoxValidity::Set,
            "action processing also changes the raw order id but does not reset the box"
        );

        let mut reset = blocked_box_json(22, 1, 2, 3, 4);
        assert!(canonicalize_legacy_blocked_box(
            &mut reset,
            10,
            true,
            None,
            None,
            &mut shadows,
        ));
        assert!(reset.pointer("/position/blocked_box").unwrap().is_null());

        let mut still_unset = blocked_box_json(22, 1, 2, 3, 4);
        assert!(canonicalize_legacy_blocked_box(
            &mut still_unset,
            10,
            false,
            None,
            None,
            &mut shadows,
        ));

        let mut revalidated = blocked_box_json(23, 1, 2, 3, 5);
        assert!(!canonicalize_legacy_blocked_box(
            &mut revalidated,
            10,
            true,
            None,
            None,
            &mut shadows,
        ));
        assert_eq!(
            shadows[&10].validity,
            LegacyBlockedBoxValidity::Set,
            "a post-reset tuple update revalidates the box"
        );

        let mut revisited_old_tuple = blocked_box_json(23, 1, 2, 3, 4);
        assert!(!canonicalize_legacy_blocked_box(
            &mut revisited_old_tuple,
            10,
            false,
            None,
            None,
            &mut shadows,
        ));
        assert!(
            !revisited_old_tuple
                .pointer("/position/blocked_box")
                .unwrap()
                .is_null()
        );
    }

    #[test]
    fn legacy_blocked_box_shadow_recognizes_same_tuple_deviation_revalidation() {
        let map = (639.58826_f32, 1250.3961_f32);
        let mut runtime = deviated_blocked_box_json(20, map, (644.4594, 1249.2684), true, true, 0);
        let tuple = legacy_blocked_box_tuple(&runtime).unwrap();
        assert!(legacy_blocked_box_revalidated_by_deviation(
            &runtime,
            tuple,
            Some(false),
        ));

        let mut shadows = BTreeMap::from([(
            10,
            LegacyBlockedBoxShadow {
                tuple,
                validity: LegacyBlockedBoxValidity::Unset,
                last_processed_order_id: 20,
                pending_motion_order_id: None,
                stoppable_motion_order: None,
                deviated: Some(false),
                direct_validity_observed: false,
            },
        )]);
        assert!(!canonicalize_legacy_blocked_box(
            &mut runtime,
            10,
            false,
            None,
            None,
            &mut shadows,
        ));
        assert_eq!(shadows[&10].validity, LegacyBlockedBoxValidity::Set);

        for mut stale in [
            deviated_blocked_box_json(20, map, (644.4594, 1249.2684), false, true, 0),
            deviated_blocked_box_json(20, map, (644.4594, 1249.2684), true, false, 0),
            deviated_blocked_box_json(20, map, (644.4594, 1249.2684), true, true, 1),
            deviated_blocked_box_json(20, map, map, true, true, 0),
        ] {
            assert!(!legacy_blocked_box_revalidated_by_deviation(
                &stale,
                legacy_blocked_box_tuple(&stale).unwrap(),
                Some(false),
            ));
            stale["position"]["blocked_box"]["max"]["x"]["bits"] = serde_json::json!(0_u32);
            assert!(!legacy_blocked_box_revalidated_by_deviation(
                &stale,
                legacy_blocked_box_tuple(&stale).unwrap(),
                Some(false),
            ));
        }
        let unchanged_deviation =
            deviated_blocked_box_json(20, map, (644.4594, 1249.2684), true, true, 0);
        assert!(!legacy_blocked_box_revalidated_by_deviation(
            &unchanged_deviation,
            legacy_blocked_box_tuple(&unchanged_deviation).unwrap(),
            Some(true),
        ));
        let mut unchanged_deviation = unchanged_deviation;
        let unchanged_tuple = legacy_blocked_box_tuple(&unchanged_deviation).unwrap();
        let mut stale_shadows = BTreeMap::from([(
            10,
            LegacyBlockedBoxShadow {
                tuple: unchanged_tuple,
                validity: LegacyBlockedBoxValidity::Unset,
                last_processed_order_id: 20,
                pending_motion_order_id: None,
                stoppable_motion_order: None,
                deviated: Some(true),
                direct_validity_observed: false,
            },
        )]);
        assert!(canonicalize_legacy_blocked_box(
            &mut unchanged_deviation,
            10,
            false,
            None,
            None,
            &mut stale_shadows,
        ));
        assert!(
            unchanged_deviation
                .pointer("/position/blocked_box")
                .unwrap()
                .is_null()
        );
    }

    #[test]
    fn legacy_blocked_box_shadow_does_not_guess_for_runtime_actor() {
        let mut shadows = BTreeMap::new();
        let mut first = blocked_box_json(20, 1, 2, 3, 4);
        assert!(!canonicalize_legacy_blocked_box(
            &mut first,
            10,
            false,
            None,
            None,
            &mut shadows,
        ));
        assert_eq!(shadows[&10].validity, LegacyBlockedBoxValidity::Unknown);

        let mut reset = blocked_box_json(21, 1, 2, 3, 4);
        assert!(canonicalize_legacy_blocked_box(
            &mut reset,
            10,
            true,
            None,
            None,
            &mut shadows,
        ));
        assert!(reset.pointer("/position/blocked_box").unwrap().is_null());
    }

    #[test]
    fn legacy_blocked_box_shadow_preserves_save_proven_inactive_box() {
        let mut shadows = BTreeMap::from([(
            10,
            LegacyBlockedBoxShadow {
                tuple: LegacyBlockedBoxTuple {
                    min_x: 1,
                    min_y: 2,
                    max_x: 3,
                    max_y: 4,
                },
                validity: LegacyBlockedBoxValidity::Unset,
                last_processed_order_id: 20,
                pending_motion_order_id: None,
                stoppable_motion_order: None,
                deviated: None,
                direct_validity_observed: false,
            },
        )]);
        let mut runtime = blocked_box_json(20, 1, 2, 3, 4);
        assert!(canonicalize_legacy_blocked_box(
            &mut runtime,
            10,
            false,
            None,
            None,
            &mut shadows,
        ));
    }

    #[test]
    fn native_suffix_appends_to_the_recording_identity() {
        // The `.jsonl.zst` path is the stable trace identity; the native
        // artifact must derive from it by appending, never by renaming.
        let native = native_binary_trace_path(Path::new("dir/replay-001-session-0001.jsonl.zst"));
        assert_eq!(
            native,
            PathBuf::from(format!(
                "dir/replay-001-session-0001.jsonl.zst{TRACE_NATIVE_SUFFIX}"
            ))
        );
        assert!(!TRACE_NATIVE_SUFFIX.contains("-v"));

        // Direct-from-capture conversions skip the interim zstd recording
        // but keep the identical artifact identity.
        let uncompressed = native_binary_trace_path(Path::new("dir/replay-001-session-0001.jsonl"));
        assert_eq!(uncompressed, native);
    }

    #[test]
    fn npc_boundary_transients_are_typed_bounded_and_legacy_defaulted() {
        let parsed: Vec<TraceInitialNpcTransient> = serde_json::from_value(serde_json::json!([
            {"creation_order": 96, "maximal_visibility": 31},
            {"creation_order": 117, "maximal_visibility": 47}
        ]))
        .expect("parse schema-16 NPC boundary transients");
        assert_eq!(
            parsed,
            [
                TraceInitialNpcTransient {
                    creation_order: 96,
                    maximal_visibility: 31,
                },
                TraceInitialNpcTransient {
                    creation_order: 117,
                    maximal_visibility: 47,
                },
            ]
        );
        assert!(
            serde_json::from_value::<TraceInitialNpcTransient>(serde_json::json!({
                "creation_order": 96,
                "maximal_visibility": 65_536
            }))
            .is_err(),
            "The original game's maximum visibility is 16-bit and must not be widened silently"
        );

        let mut header = serde_json::to_value(minimal_test_native_header("test").trace).unwrap();
        header
            .as_object_mut()
            .unwrap()
            .remove("initial_npc_transients");
        let legacy = serde_json::from_value::<TraceHeader>(header)
            .expect("legacy schema-16 headers default the additive NPC transient boundary");
        assert!(legacy.initial_npc_transients.is_none());

        let present_empty = minimal_test_native_header("test").trace;
        assert_eq!(present_empty.initial_npc_transients, Some(Vec::new()));
    }

    #[test]
    fn legacy_segment_visibility_fallback_matches_original_uword_conversion() {
        assert_eq!(
            reconstruct_unrecorded_maximal_visibility(false, [0.0, 1.599_999_9, 0.25]),
            31
        );
        assert_eq!(
            reconstruct_unrecorded_maximal_visibility(false, [2.399_999_9]),
            47
        );
        assert_eq!(
            reconstruct_unrecorded_maximal_visibility(true, [1.599_999_9]),
            319
        );
        assert_eq!(
            reconstruct_unrecorded_maximal_visibility(false, std::iter::empty()),
            0
        );
    }

    #[test]
    fn legacy_visibility_fallback_only_applies_to_in_process_reload_envelopes() {
        assert!(legacy_loaded_save_retains_process_transients(0));
        assert!(!legacy_loaded_save_retains_process_transients(76));
    }

    fn write_test_native_records(
        records: &[BinaryTraceRecord],
        footer: Option<BinaryTraceFooter>,
    ) -> tempfile::NamedTempFile {
        write_test_native_records_with_compression(
            records,
            footer,
            TRACE_NATIVE_ZSTD_LEVEL,
            TRACE_NATIVE_LONG_DISTANCE_MATCHING,
        )
    }

    fn minimal_test_native_header(source_fingerprint: &str) -> BinaryTraceHeaderV68 {
        BinaryTraceHeaderV68 {
            version: TRACE_NATIVE_VERSION,
            source_fingerprint: source_fingerprint.to_owned(),
            trace: TraceHeader {
                record_type: "header".to_owned(),
                mission: "test".to_owned(),
                proto_level: "test".to_owned(),
                rng_seed: 1,
                schema: TRACE_SCHEMA_VERSION,
                session_index: 1,
                start_state: TraceStartState::MissionStart,
                initial_frame: 0,
                simulation_hz: 25,
                synchronous_pathfinding: true,
                rng_stream: "libc_rand_raw_global_draw_order".to_owned(),
                visibility_queries: "opaque_is_reachable".to_owned(),
                random_input_seed: None,
                sim_config: TraceSimConfig {
                    difficulty: TraceDifficulty::Medium,
                    script_enabled: true,
                    highlander: false,
                    highlander2: false,
                    golden_eye: false,
                    ignore_default_loose: false,
                    bypass_fog_sprites_crash: false,
                    amount_of_speaking: 0,
                },
                campaign: TraceCampaign {
                    version: 1,
                    values: Vec::new(),
                    ares: 0,
                    missions: Vec::new(),
                    accessible_mission_indices: Vec::new(),
                    pending_accessible_mission_indices: Vec::new(),
                    last_mission_index: None,
                    current_mission_index: None,
                    next_mission_index: None,
                    blazon_mission_index: None,
                    last_played_mission_indices: Vec::new(),
                    last_pseudo_mission_status: 0,
                    last_pseudo_mission_id: 0,
                    characters: Vec::new(),
                    gang_indices: Vec::new(),
                    reservist_indices: Vec::new(),
                    mission_team_indices: Vec::new(),
                    peasant_names: Vec::new(),
                    reservists_are_back: false,
                    collected_relics: Vec::new(),
                    production_sectors: Vec::new(),
                },
                motion_grid: TraceMotionGrid { layers: Vec::new() },
                initial_npc_transients: Some(Vec::new()),
                initial_save: None,
            },
            rng_prefix: TraceRngPrefix {
                r#type: "rng_prefix".to_owned(),
                draws: TraceRngBatch {
                    first_index: 0,
                    values: Vec::new(),
                    callsite_offsets: Vec::new(),
                    main_thread: Vec::new(),
                    domains: Vec::new(),
                },
            },
        }
    }

    fn write_synthetic_native_trace(path: &Path, source_fingerprint: &str, checksum: bool) {
        let file = File::create(path).unwrap();
        let mut encoder = zstd::stream::write::Encoder::new(BufWriter::new(file), 1).unwrap();
        encoder.window_log(20).unwrap();
        encoder.include_checksum(checksum).unwrap();
        write_binary_record(
            &mut encoder,
            &minimal_test_native_header(source_fingerprint),
            "synthetic native header",
        );
        write_binary_record(
            &mut encoder,
            std::slice::from_ref(&complete_test_end(0, 0)),
            "synthetic native block",
        );
        let mut writer = encoder.finish().unwrap();
        write_binary_trace_footer(
            &mut writer,
            BinaryTraceFooter {
                version: TRACE_NATIVE_VERSION,
                frame_count: 0,
                final_frame: 0,
            },
        )
        .unwrap();
        writer.flush().unwrap();
        writer.get_ref().sync_all().unwrap();
    }

    fn write_synthetic_version_67_native_trace(path: &Path, source_fingerprint: &str) {
        let file = File::create(path).unwrap();
        let mut encoder = zstd::stream::write::Encoder::new(BufWriter::new(file), 1).unwrap();
        encoder.window_log(20).unwrap();
        let header: BinaryTraceHeaderV67 = minimal_test_native_header(source_fingerprint).into();
        write_binary_record(&mut encoder, &header, "synthetic version-67 native header");
        let end = BinaryTraceRecordV67::End {
            rng_suffix: Some(TraceRngBatch {
                first_index: 0,
                values: Vec::new(),
                callsite_offsets: Vec::new(),
                main_thread: Vec::new(),
                domains: Vec::new(),
            }),
            final_frame: Some(0),
            frame_count: Some(0),
        };
        write_binary_record(
            &mut encoder,
            std::slice::from_ref(&end),
            "synthetic version-67 native block",
        );
        let mut writer = encoder.finish().unwrap();
        write_binary_trace_footer(
            &mut writer,
            BinaryTraceFooter {
                version: TRACE_NATIVE_LEGACY_VERSION,
                frame_count: 0,
                final_frame: 0,
            },
        )
        .unwrap();
        writer.flush().unwrap();
        writer.get_ref().sync_all().unwrap();
    }

    fn write_synthetic_version_66_native_trace(path: &Path, source_fingerprint: &str) {
        let file = File::create(path).unwrap();
        let mut encoder = zstd::stream::write::Encoder::new(BufWriter::new(file), 1).unwrap();
        encoder.window_log(20).unwrap();
        let header: BinaryTraceHeaderV66 = minimal_test_native_header(source_fingerprint).into();
        write_binary_record(&mut encoder, &header, "synthetic version-66 native header");
        let end = BinaryTraceRecordV66::End {
            rng_suffix: Some(TraceRngBatch {
                first_index: 0,
                values: Vec::new(),
                callsite_offsets: Vec::new(),
                main_thread: Vec::new(),
                domains: Vec::new(),
            }),
            final_frame: Some(0),
            frame_count: Some(0),
        };
        write_binary_record(
            &mut encoder,
            std::slice::from_ref(&end),
            "synthetic version-66 native block",
        );
        let mut writer = encoder.finish().unwrap();
        write_binary_trace_footer(
            &mut writer,
            BinaryTraceFooter {
                version: TRACE_NATIVE_V66_VERSION,
                frame_count: 0,
                final_frame: 0,
            },
        )
        .unwrap();
        writer.flush().unwrap();
        writer.get_ref().sync_all().unwrap();
    }

    fn write_synthetic_late_version_67_native_trace(path: &Path, source_fingerprint: &str) {
        let file = File::create(path).unwrap();
        let mut encoder = zstd::stream::write::Encoder::new(BufWriter::new(file), 1).unwrap();
        encoder.window_log(20).unwrap();
        let header: BinaryTraceHeaderV67Late =
            minimal_test_native_header(source_fingerprint).into();
        write_binary_record(
            &mut encoder,
            &header,
            "synthetic late version-67 native header",
        );
        let end = BinaryTraceRecordV67Late::End {
            rng_suffix: Some(TraceRngBatch {
                first_index: 0,
                values: Vec::new(),
                callsite_offsets: Vec::new(),
                main_thread: Vec::new(),
                domains: Vec::new(),
            }),
            final_frame: Some(0),
            frame_count: Some(0),
        };
        write_binary_record(
            &mut encoder,
            std::slice::from_ref(&end),
            "synthetic late version-67 native block",
        );
        let mut writer = encoder.finish().unwrap();
        write_binary_trace_footer(
            &mut writer,
            BinaryTraceFooter {
                version: TRACE_NATIVE_LEGACY_VERSION,
                frame_count: 0,
                final_frame: 0,
            },
        )
        .unwrap();
        writer.flush().unwrap();
        writer.get_ref().sync_all().unwrap();
    }

    fn write_test_native_records_with_compression(
        records: &[BinaryTraceRecord],
        footer: Option<BinaryTraceFooter>,
        level: i32,
        long_distance_matching: bool,
    ) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        {
            let mut encoder =
                zstd::stream::write::Encoder::new(BufWriter::new(file.as_file_mut()), level)
                    .unwrap();
            if long_distance_matching {
                encoder.long_distance_matching(true).unwrap();
            }
            encoder.window_log(20).unwrap();
            for record in records {
                write_binary_record(
                    &mut encoder,
                    std::slice::from_ref(record),
                    "test native trace block",
                );
            }
            let mut writer = encoder.finish().unwrap();
            if let Some(footer) = footer {
                write_binary_trace_footer(&mut writer, footer).unwrap();
            }
            writer.flush().unwrap();
        }
        file.as_file().sync_all().unwrap();
        file
    }

    fn write_test_native_stream_with_policy(
        header: &BinaryTraceHeaderV68,
        records: &[BinaryTraceRecord],
        footer: BinaryTraceFooter,
        policy: NativeStoragePolicy,
    ) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        {
            let mut encoder = zstd::stream::write::Encoder::new(
                BufWriter::new(file.as_file_mut()),
                TRACE_NATIVE_ZSTD_LEVEL,
            )
            .unwrap();
            configure_cache_compression(&mut encoder, None, policy.window_log);
            write_binary_record(&mut encoder, header, "test native trace header");
            for block in records.chunks(policy.block_records) {
                write_binary_record(&mut encoder, block, "test native trace block");
            }
            let mut writer = encoder.finish().unwrap();
            write_binary_trace_footer(&mut writer, footer).unwrap();
            writer.flush().unwrap();
        }
        file.as_file().sync_all().unwrap();
        file
    }

    fn complete_test_end(frame_count: u64, final_frame: u64) -> BinaryTraceRecord {
        BinaryTraceRecord::End {
            rng_suffix: Some(TraceRngBatch {
                first_index: 0,
                values: Vec::new(),
                callsite_offsets: Vec::new(),
                main_thread: Vec::new(),
                domains: Vec::new(),
            }),
            final_frame: Some(final_frame),
            frame_count: Some(frame_count),
        }
    }

    #[test]
    fn version_67_header_uses_legacy_vec_layout() {
        let legacy: BinaryTraceHeaderV67 = minimal_test_native_header("legacy-v67").into();
        let mut encoded_record = Vec::new();
        write_binary_record(&mut encoded_record, &legacy, "test version-67 header");

        let decoded = read_binary_trace_header_record(
            &mut std::io::Cursor::new(encoded_record),
            TRACE_NATIVE_LEGACY_VERSION,
        )
        .expect("decode version-67 header through its original bitcode layout");

        assert_eq!(decoded.version, TRACE_NATIVE_LEGACY_VERSION);
        assert_eq!(decoded.source_fingerprint, "legacy-v67");
        assert!(decoded.trace.initial_npc_transients.is_none());
    }

    #[test]
    fn late_version_67_header_preserves_optional_transient_layout() {
        let legacy: BinaryTraceHeaderV67Late = minimal_test_native_header("late-legacy-v67").into();
        let mut encoded_record = Vec::new();
        write_binary_record(&mut encoded_record, &legacy, "test late version-67 header");

        let (decoded, late_layout) = read_binary_trace_header_record_with_layout(
            &mut std::io::Cursor::new(encoded_record),
            TRACE_NATIVE_LEGACY_VERSION,
        )
        .expect("decode late version-67 header through its historical bitcode layout");

        assert!(late_layout);
        assert_eq!(decoded.version, TRACE_NATIVE_LEGACY_VERSION);
        assert_eq!(decoded.source_fingerprint, "late-legacy-v67");
        assert_eq!(decoded.trace.initial_npc_transients, Some(Vec::new()));
    }

    #[test]
    fn version_66_header_uses_frozen_legacy_layout() {
        let legacy: BinaryTraceHeaderV66 = minimal_test_native_header("legacy-v66").into();
        let mut encoded_record = Vec::new();
        write_binary_record(&mut encoded_record, &legacy, "test version-66 header");

        let decoded = read_binary_trace_header_record(
            &mut std::io::Cursor::new(encoded_record),
            TRACE_NATIVE_V66_VERSION,
        )
        .expect("decode version-66 header through its original bitcode layout");

        assert_eq!(decoded.version, TRACE_NATIVE_V66_VERSION);
        assert_eq!(decoded.source_fingerprint, "legacy-v66");
        assert_eq!(decoded.trace.initial_npc_transients, Some(Vec::new()));
    }

    #[test]
    fn reblock_migrates_version_67_to_current_without_semantic_drift() {
        let directory = tempfile::tempdir().unwrap();
        let native = directory.path().join("legacy.parity.bitcode.zst");
        write_synthetic_version_67_native_trace(&native, "legacy-reblock");
        let before = native_reblock_semantic_identity(&native);

        reblock_native_trace(&native, NativeStoragePolicy::default());

        let footer = read_binary_trace_footer(&native).unwrap();
        let header = read_binary_trace_header(&native);
        assert_eq!(footer.version, TRACE_NATIVE_VERSION);
        assert_eq!(header.version, TRACE_NATIVE_VERSION);
        assert_eq!(native_reblock_semantic_identity(&native), before);
    }

    #[test]
    fn reblock_migrates_late_version_67_to_current_without_semantic_drift() {
        let directory = tempfile::tempdir().unwrap();
        let native = directory.path().join("late-legacy.parity.bitcode.zst");
        write_synthetic_late_version_67_native_trace(&native, "late-legacy-reblock");
        let before = native_reblock_semantic_identity(&native);

        reblock_native_trace(&native, NativeStoragePolicy::default());

        let footer = read_binary_trace_footer(&native).unwrap();
        let header = read_binary_trace_header(&native);
        assert_eq!(footer.version, TRACE_NATIVE_VERSION);
        assert_eq!(header.version, TRACE_NATIVE_VERSION);
        assert_eq!(native_reblock_semantic_identity(&native), before);
    }

    #[test]
    fn reblock_resumes_version_67_recovery_binding() {
        let directory = tempfile::tempdir().unwrap();
        let native = directory.path().join("legacy-recovery.parity.bitcode.zst");
        write_synthetic_version_67_native_trace(&native, "legacy-recovery");
        let source = native_reblock_source_path(&native);
        let binding_path = native_reblock_binding_path(&native);
        let (source_bytes, source_content_sha256, source_device, source_inode) =
            native_reblock_file_identity(&native);
        let (frame_count, final_frame, source_semantic_sha256) =
            native_reblock_semantic_identity_with_version_policy(&native, false);
        let binding = NativeReblockBinding {
            version: TRACE_NATIVE_LEGACY_VERSION,
            canonical_path: native_reblock_canonical_path(&native),
            source_content_sha256,
            source_bytes,
            source_semantic_sha256: format!("stale-{source_semantic_sha256}"),
            frame_count,
            final_frame,
            #[cfg(unix)]
            source_device,
            #[cfg(unix)]
            source_inode,
        };
        write_native_reblock_binding(&binding_path, &binding);
        std::fs::hard_link(&native, &source).unwrap();

        reblock_native_trace(&native, NativeStoragePolicy::default());

        assert_eq!(
            read_binary_trace_footer(&native).unwrap().version,
            TRACE_NATIVE_VERSION
        );
        assert!(!source.exists());
        assert!(!binding_path.exists());
    }

    #[test]
    fn reblock_migrates_version_66_to_small_current_blocks_without_semantic_drift() {
        let directory = tempfile::tempdir().unwrap();
        let native = directory.path().join("legacy-v66.parity.bitcode.zst");
        write_synthetic_version_66_native_trace(&native, "legacy-v66-reblock");
        let before = native_reblock_semantic_identity(&native);

        reblock_native_trace(&native, NativeStoragePolicy::default());

        assert_eq!(native_reblock_semantic_identity(&native), before);
        assert_eq!(
            read_binary_trace_footer(&native).unwrap().version,
            TRACE_NATIVE_VERSION
        );
        assert_eq!(
            read_binary_trace_header(&native).version,
            TRACE_NATIVE_VERSION
        );
    }

    #[test]
    fn reblock_refreshes_stale_semantic_digest_for_exact_bound_source() {
        let directory = tempfile::tempdir().unwrap();
        let native = directory.path().join("stale-binding.parity.bitcode.zst");
        write_synthetic_native_trace(&native, "stale-binding", false);
        let source = native_reblock_source_path(&native);
        let binding_path = native_reblock_binding_path(&native);
        let mut binding = create_native_reblock_binding(&native);
        binding.source_semantic_sha256 = "pre-projection-change-digest".to_owned();
        write_native_reblock_binding(&binding_path, &binding);
        std::fs::hard_link(&native, &source).unwrap();

        reblock_native_trace(&native, NativeStoragePolicy::default());

        assert_eq!(
            read_binary_trace_footer(&native).unwrap().version,
            TRACE_NATIVE_VERSION
        );
        assert!(!source.exists());
        assert!(!binding_path.exists());
    }

    #[test]
    fn native_small_block_policy_round_trips_records_and_footer() {
        assert_eq!(TRACE_NATIVE_V66_VERSION, 66);
        assert_eq!(TRACE_NATIVE_LEGACY_VERSION, 67);
        assert_eq!(TRACE_NATIVE_VERSION, 68);
        assert_eq!(TRACE_NATIVE_ZSTD_LEVEL, 19);
        assert!(!TRACE_NATIVE_LONG_DISTANCE_MATCHING);
        assert_eq!(TRACE_NATIVE_BLOCK_RECORDS, 32);
        assert_eq!(TRACE_NATIVE_WINDOW_LOG, 26);
        assert_eq!(
            NativeStoragePolicy::new(1, TRACE_NATIVE_MIN_WINDOW_LOG),
            NativeStoragePolicy {
                block_records: 1,
                window_log: 20,
            }
        );
        assert_eq!(
            NativeStoragePolicy::new(
                TRACE_NATIVE_MAX_REBLOCK_RECORDS,
                TRACE_NATIVE_MAX_REBLOCK_WINDOW_LOG,
            ),
            NativeStoragePolicy {
                block_records: 1000,
                window_log: 29,
            }
        );

        let footer = BinaryTraceFooter {
            version: TRACE_NATIVE_VERSION,
            frame_count: 0,
            final_frame: 10,
        };
        let native = write_test_native_records(&[complete_test_end(0, 10)], Some(footer));
        let mut reader = BinaryTraceReader::open(native.path());
        assert!(matches!(
            reader.read_record(),
            BinaryTraceRecord::End { .. }
        ));
        reader.validate_terminator(0, 10).unwrap();
        assert_eq!(read_binary_trace_footer(native.path()).unwrap(), footer);
    }

    #[test]
    #[should_panic(expected = "--reblock-records must be between 1 and 1000")]
    fn native_storage_policy_rejects_empty_blocks() {
        let _ = NativeStoragePolicy::new(0, TRACE_NATIVE_WINDOW_LOG);
    }

    #[test]
    #[should_panic(expected = "--reblock-window-log must be between 20 and 29")]
    fn native_storage_policy_rejects_oversized_windows() {
        let _ = NativeStoragePolicy::new(
            TRACE_NATIVE_BLOCK_RECORDS,
            TRACE_NATIVE_MAX_REBLOCK_WINDOW_LOG + 1,
        );
    }

    #[test]
    fn current_reader_preserves_small_block_semantics_digest_and_terminator() {
        let policy = NativeStoragePolicy::default();
        assert_eq!(policy, NativeStoragePolicy::new(32, 26));
        let header = minimal_test_native_header("small-block-v68");
        let mut records = Vec::new();
        for frame_before in 0..65_u64 {
            let mut value = minimal_frame_json();
            value["frame_before"] = frame_before.into();
            value["frame_after"] = (frame_before + 1).into();
            records.push(BinaryTraceRecord::Frame(
                serde_json::from_value(value).unwrap(),
            ));
        }
        records.push(complete_test_end(65, 65));
        let footer = BinaryTraceFooter {
            version: TRACE_NATIVE_VERSION,
            frame_count: 65,
            final_frame: 65,
        };
        let mut expected_digest = Sha256::new();
        update_native_semantic_digest(&mut expected_digest, &header);
        for record in &records {
            update_native_semantic_digest(&mut expected_digest, record);
        }
        let expected_digest = expected_digest.finalize();
        let native = write_test_native_stream_with_policy(&header, &records, footer, policy);

        // This is the ordinary version-68 reader: block cardinality is not
        // represented in the header or footer and has never been fixed at 1,000.
        let mut reader = BinaryTraceReader::open(native.path());
        assert_eq!(reader.read_header().version, TRACE_NATIVE_VERSION);
        assert!(matches!(reader.read_record(), BinaryTraceRecord::Frame(_)));
        assert_eq!(reader.pending.len(), 31);
        for _ in 1..32 {
            assert!(matches!(reader.read_record(), BinaryTraceRecord::Frame(_)));
        }
        assert!(reader.pending.is_empty());
        assert!(matches!(reader.read_record(), BinaryTraceRecord::Frame(_)));
        assert_eq!(reader.pending.len(), 31);

        let (decoded_frames, actual_digest) = digest_and_validate_native_trace(native.path());
        assert_eq!(decoded_frames, 65);
        assert_eq!(actual_digest, expected_digest);
        assert_eq!(read_binary_trace_footer(native.path()).unwrap(), footer);
    }

    #[test]
    fn reblock_recovery_source_is_adjacent_and_version_specific() {
        let native = Path::new("dir/replay-001-session-0001.jsonl.zst.parity.bitcode.zst");
        assert_eq!(
            native_reblock_source_path(native),
            PathBuf::from(
                "dir/replay-001-session-0001.jsonl.zst.parity.bitcode.zst.parity-reblock-source-v67"
            )
        );
        assert_eq!(
            native_reblock_binding_path(native),
            PathBuf::from(
                "dir/replay-001-session-0001.jsonl.zst.parity.bitcode.zst.parity-reblock-binding-v67.json"
            )
        );
    }

    #[test]
    fn reblock_binding_rejects_foreign_content_and_inode() {
        let binding = NativeReblockBinding {
            version: TRACE_NATIVE_VERSION,
            canonical_path: PathBuf::from("trace.parity.bitcode.zst"),
            source_content_sha256: "bound-content".to_owned(),
            source_bytes: 123,
            source_semantic_sha256: "bound-semantics".to_owned(),
            frame_count: 10,
            final_frame: 20,
            #[cfg(unix)]
            source_device: 30,
            #[cfg(unix)]
            source_inode: 40,
        };
        assert!(native_reblock_file_identity_matches(
            &binding,
            123,
            "bound-content",
            30,
            40
        ));
        assert!(!native_reblock_file_identity_matches(
            &binding,
            123,
            "foreign-content",
            30,
            40
        ));
        #[cfg(unix)]
        assert!(!native_reblock_file_identity_matches(
            &binding,
            123,
            "bound-content",
            30,
            41
        ));
    }

    #[test]
    fn foreign_reblock_recovery_without_binding_cannot_replace_canonical() {
        let directory = tempfile::tempdir().unwrap();
        let native = directory.path().join("trace.parity.bitcode.zst");
        let source = native_reblock_source_path(&native);
        let binding = native_reblock_binding_path(&native);
        write_synthetic_native_trace(&native, "canonical", false);
        write_synthetic_native_trace(&source, "foreign", false);
        let canonical_before = std::fs::read(&native).unwrap();
        let foreign_before = std::fs::read(&source).unwrap();

        assert!(
            classify_native_reblock_recovery_state(true, source.exists(), binding.exists())
                .is_err()
        );
        assert_eq!(std::fs::read(&native).unwrap(), canonical_before);
        assert_eq!(std::fs::read(&source).unwrap(), foreign_before);
        assert!(!binding.exists());
    }

    #[test]
    fn stale_bound_reblock_source_fails_closed() {
        let directory = tempfile::tempdir().unwrap();
        let native = directory.path().join("trace.parity.bitcode.zst");
        let source = native_reblock_source_path(&native);
        let binding_path = native_reblock_binding_path(&native);
        write_synthetic_native_trace(&native, "canonical", false);
        let binding = create_native_reblock_binding(&native);
        write_native_reblock_binding(&binding_path, &binding);
        write_synthetic_native_trace(&source, "foreign", false);
        let canonical_before = std::fs::read(&native).unwrap();
        let source_before = std::fs::read(&source).unwrap();
        let binding_before = std::fs::read(&binding_path).unwrap();

        assert!(validate_native_reblock_source_file_identity(&source, &binding).is_err());
        assert_eq!(std::fs::read(&native).unwrap(), canonical_before);
        assert_eq!(std::fs::read(&source).unwrap(), source_before);
        assert_eq!(std::fs::read(&binding_path).unwrap(), binding_before);
    }

    #[test]
    fn authenticated_reblock_source_survives_missing_canonical() {
        let directory = tempfile::tempdir().unwrap();
        let native = directory.path().join("trace.parity.bitcode.zst");
        let source = native_reblock_source_path(&native);
        let binding_path = native_reblock_binding_path(&native);
        write_synthetic_native_trace(&native, "canonical", false);
        let binding = create_native_reblock_binding(&native);
        write_native_reblock_binding(&binding_path, &binding);
        std::fs::hard_link(&native, &source).unwrap();
        std::fs::remove_file(&native).unwrap();

        assert!(matches!(
            prepare_native_reblock_source(&native, &source, &binding_path),
            NativeReblockPreparation::Ready(_)
        ));
        assert!(!native.exists());
        assert!(source.exists());
        assert!(binding_path.exists());
    }

    #[test]
    fn missing_reblock_canonical_without_authenticated_pair_fails_closed() {
        let directory = tempfile::tempdir().unwrap();
        let native = directory.path().join("trace.parity.bitcode.zst");
        let source = native_reblock_source_path(&native);
        let binding = native_reblock_binding_path(&native);
        assert!(
            classify_native_reblock_recovery_state(
                native.exists(),
                source.exists(),
                binding.exists()
            )
            .is_err()
        );
    }

    #[test]
    fn binding_only_without_canonical_or_source_fails_closed() {
        let directory = tempfile::tempdir().unwrap();
        let native = directory.path().join("trace.parity.bitcode.zst");
        let source = native_reblock_source_path(&native);
        let binding_path = native_reblock_binding_path(&native);
        write_synthetic_native_trace(&native, "canonical", false);
        let binding = create_native_reblock_binding(&native);
        write_native_reblock_binding(&binding_path, &binding);
        std::fs::remove_file(&native).unwrap();

        assert!(
            classify_native_reblock_recovery_state(
                native.exists(),
                source.exists(),
                binding_path.exists()
            )
            .is_err()
        );
        assert!(binding_path.exists());
    }

    #[test]
    fn binding_only_same_inode_canonical_recreates_recovery_link() {
        let directory = tempfile::tempdir().unwrap();
        let native = directory.path().join("trace.parity.bitcode.zst");
        let source = native_reblock_source_path(&native);
        let binding_path = native_reblock_binding_path(&native);
        write_synthetic_native_trace(&native, "canonical", false);
        let binding = create_native_reblock_binding(&native);
        write_native_reblock_binding(&binding_path, &binding);

        assert!(matches!(
            prepare_native_reblock_source(&native, &source, &binding_path),
            NativeReblockPreparation::Ready(_)
        ));
        #[cfg(unix)]
        assert_eq!(
            std::fs::metadata(&native).unwrap().ino(),
            std::fs::metadata(&source).unwrap().ino()
        );
    }

    #[test]
    fn semantic_equal_postpublish_state_only_cleans_binding() {
        let directory = tempfile::tempdir().unwrap();
        let native = directory.path().join("trace.parity.bitcode.zst");
        let source = native_reblock_source_path(&native);
        let binding_path = native_reblock_binding_path(&native);
        let replacement = directory.path().join("replacement.parity.bitcode.zst");
        write_synthetic_native_trace(&native, "canonical", false);
        let binding = create_native_reblock_binding(&native);
        write_native_reblock_binding(&binding_path, &binding);
        std::fs::hard_link(&native, &source).unwrap();
        write_synthetic_native_trace(&replacement, "canonical", true);
        assert_ne!(
            trace_content_sha256(&native),
            trace_content_sha256(&replacement)
        );
        std::fs::rename(&replacement, &native).unwrap();
        std::fs::remove_file(&source).unwrap();

        assert!(matches!(
            prepare_native_reblock_source(&native, &source, &binding_path),
            NativeReblockPreparation::AlreadyCommitted
        ));
        assert!(native.exists());
        assert!(!source.exists());
        assert!(!binding_path.exists());
    }

    #[test]
    fn reblock_orphan_cleanup_is_trace_scoped_and_preserves_symlinks() {
        let directory = tempfile::tempdir().unwrap();
        let native = directory.path().join("trace.parity.bitcode.zst");
        let other_native = directory.path().join("other.parity.bitcode.zst");
        let output_orphan = directory.path().join(format!(
            "{}dead",
            native_reblock_temporary_prefix(&native, false)
        ));
        let binding_orphan = directory.path().join(format!(
            "{}dead",
            native_reblock_temporary_prefix(&native, true)
        ));
        let other_orphan = directory.path().join(format!(
            "{}live",
            native_reblock_temporary_prefix(&other_native, false)
        ));
        let unrelated = directory.path().join(".parity-reblock-v67-unrelated");
        std::fs::write(&output_orphan, b"partial output").unwrap();
        std::fs::write(&binding_orphan, b"partial binding").unwrap();
        std::fs::write(&other_orphan, b"another trace").unwrap();
        std::fs::write(&unrelated, b"not ours").unwrap();
        #[cfg(unix)]
        let symlink = {
            let symlink = directory.path().join(format!(
                "{}symlink",
                native_reblock_temporary_prefix(&native, false)
            ));
            std::os::unix::fs::symlink(&unrelated, &symlink).unwrap();
            symlink
        };

        cleanup_native_reblock_orphans(&native);

        assert!(!output_orphan.exists());
        assert!(!binding_orphan.exists());
        assert!(other_orphan.exists());
        assert!(unrelated.exists());
        #[cfg(unix)]
        assert!(
            std::fs::symlink_metadata(symlink)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[cfg(unix)]
    #[test]
    fn reblock_temporary_owner_hash_distinguishes_non_utf8_paths() {
        use std::os::unix::ffi::OsStringExt as _;

        let directory = tempfile::tempdir().unwrap();
        let first = directory
            .path()
            .join(std::ffi::OsString::from_vec(b"trace-\xfe".to_vec()));
        let second = directory
            .path()
            .join(std::ffi::OsString::from_vec(b"trace-\xff".to_vec()));
        assert_ne!(
            native_reblock_temporary_prefix(&first, false),
            native_reblock_temporary_prefix(&second, false)
        );
    }

    #[test]
    fn native_maintenance_commands_accept_logical_and_native_paths() {
        let directory = tempfile::tempdir().unwrap();
        let logical = directory.path().join("replay-001-session-0001.jsonl.zst");
        let native = native_binary_trace_path(&logical);
        assert_eq!(requested_native_trace_path(&logical), native);
        assert_eq!(requested_native_trace_path(&native), native);

        std::fs::write(&logical, b"coexisting source with a different fingerprint").unwrap();
        write_synthetic_native_trace(&native, "native-parity-v67:legacy", true);
        let before = trace_content_sha256(&native);
        assert_eq!(ensure_native_binary_trace(&native), native);
        assert_eq!(trace_content_sha256(&native), before);
    }

    #[test]
    fn native_reader_accepts_current_version_with_level_nineteen_ldm() {
        let footer = BinaryTraceFooter {
            version: TRACE_NATIVE_VERSION,
            frame_count: 0,
            final_frame: 10,
        };
        let native = write_test_native_records_with_compression(
            &[complete_test_end(0, 10)],
            Some(footer),
            19,
            true,
        );
        let mut reader = BinaryTraceReader::open(native.path());
        assert!(matches!(
            reader.read_record(),
            BinaryTraceRecord::End { .. }
        ));
        reader.validate_terminator(0, 10).unwrap();
    }

    #[test]
    fn conversion_cleanup_resolves_relative_and_absolute_paths() {
        let directory = tempfile::tempdir().unwrap();
        let relative = Path::new("relative-trace.jsonl.zst");
        let resolved_relative = absolute_trace_path_from(relative, directory.path());
        assert_eq!(resolved_relative, directory.path().join(relative));

        let absolute = directory.path().join("absolute-trace.jsonl.zst");
        assert_eq!(
            absolute_trace_path_from(&absolute, Path::new("/ignored")),
            absolute
        );

        for trace in [&resolved_relative, &absolute] {
            std::fs::write(trace, b"recording").unwrap();
            let fingerprint = trace_source_fingerprint(trace);
            let quarantine = conversion_quarantine_path(trace);
            let obsolete = PathBuf::from(format!(
                "{}.parity-cache-v63.test",
                trace.as_os_str().to_string_lossy()
            ));
            std::fs::write(&obsolete, b"derived").unwrap();
            let verified = VerifiedNativeReadback {
                decoded_frames: 0,
                source_path: trace.to_path_buf(),
                source_fingerprint: fingerprint,
            };
            move_verified_recording_to_quarantine(trace, &quarantine, &verified).unwrap();
            assert_eq!(
                finish_verified_conversion(
                    trace,
                    &quarantine,
                    &VerifiedNativeReadback {
                        source_path: quarantine.clone(),
                        ..verified
                    },
                ),
                1
            );
            assert!(!trace.exists());
            assert!(!obsolete.exists());
        }
    }

    #[test]
    fn replaced_recording_is_not_eligible_for_verified_deletion() {
        let directory = tempfile::tempdir().unwrap();
        let recording = directory.path().join("recording.jsonl.zst");
        std::fs::write(&recording, b"authoritative recording").unwrap();
        let verified = VerifiedNativeReadback {
            decoded_frames: 0,
            source_path: recording.clone(),
            source_fingerprint: trace_source_fingerprint(&recording),
        };
        // Preserve the byte length so this specifically proves the content
        // digest catches a replacement that the old line-count gate missed.
        std::fs::write(&recording, b"replacement recording!!").unwrap();
        let quarantine = conversion_quarantine_path(&recording);

        assert!(
            move_verified_recording_to_quarantine(&recording, &quarantine, &verified)
                .unwrap_err()
                .contains("changed after native readback")
        );
        assert!(recording.exists());
        assert_eq!(
            std::fs::read(&recording).unwrap(),
            b"replacement recording!!"
        );
    }

    #[test]
    fn quarantine_restore_never_overwrites_a_recreated_recording() {
        let parent = tempfile::tempdir().unwrap();
        let quarantined_path = parent.path().join("recording.parity-conversion-source");
        std::fs::write(&quarantined_path, b"quarantined replacement").unwrap();
        let canonical_path = parent.path().join("recording.jsonl.zst");
        std::fs::write(&canonical_path, b"new producer recording").unwrap();

        let error = restore_quarantined_recording_no_replace(&quarantined_path, &canonical_path)
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(
            std::fs::read(&canonical_path).unwrap(),
            b"new producer recording"
        );
        assert_eq!(
            std::fs::read(&quarantined_path).unwrap(),
            b"quarantined replacement"
        );
    }

    #[test]
    fn conversion_commit_never_unlinks_a_recreated_recording() {
        let directory = tempfile::tempdir().unwrap();
        let quarantine = directory
            .path()
            .join("recording.jsonl.zst.parity-conversion-source");
        let canonical = directory.path().join("recording.jsonl.zst");
        std::fs::write(&quarantine, b"verified source").unwrap();

        let conflict = commit_verified_conversion_files(&canonical, &quarantine, || {
            // This models a producer winning the pathname after the initial
            // conflict check and immediately before quarantine deletion.
            std::fs::write(&canonical, b"new producer recording").unwrap();
        })
        .unwrap_err();

        assert!(conflict.contains("new recording was preserved"));
        assert_eq!(
            std::fs::read(&canonical).unwrap(),
            b"new producer recording"
        );
        assert!(!quarantine.exists());
    }

    #[cfg(unix)]
    #[test]
    fn conversion_rejects_symlinked_logical_inputs() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target.jsonl.zst");
        let link = directory.path().join("recording.jsonl.zst");
        std::fs::write(&target, b"recording").unwrap();
        symlink(&target, &link).unwrap();
        assert!(conversion_path_is_symlink(&link));
        assert!(!conversion_path_is_symlink(&target));
        assert_eq!(std::fs::read(&target).unwrap(), b"recording");
    }

    #[test]
    fn obsolete_derivation_cleanup_requires_a_numeric_version() {
        let prefix = "trace.jsonl.zst.parity-cache-v";
        assert!(is_obsolete_native_derivation(
            "trace.jsonl.zst.parity-cache-v64.native-bincode.zst",
            prefix
        ));
        assert!(is_obsolete_native_derivation(
            "trace.jsonl.zst.parity-cache-v9",
            prefix
        ));
        assert!(!is_obsolete_native_derivation(
            "trace.jsonl.zst.parity-cache-vicious",
            prefix
        ));
        assert!(!is_obsolete_native_derivation(
            "trace.jsonl.zst.parity-cache-v64backup",
            prefix
        ));
    }

    #[test]
    fn trace_timeline_accepts_terminal_snapshot_without_clock_advance() {
        let mut timeline = TraceTimeline::new(9_245);
        timeline.observe(9_245, 9_246).unwrap();
        timeline.observe(9_246, 9_247).unwrap();
        timeline.observe(9_247, 9_247).unwrap();

        timeline.validate_terminator(3, 9_247).unwrap();
        assert!(
            timeline
                .validate_terminator(3, 9_248)
                .unwrap_err()
                .contains("last frame_after=9247")
        );
        assert!(
            timeline
                .validate_terminator(2, 9_247)
                .unwrap_err()
                .contains("3 frame records")
        );
    }

    #[test]
    fn trace_timeline_rejects_gaps_rewinds_and_multi_tick_records() {
        let mut gap = TraceTimeline::new(10);
        assert!(
            gap.observe(11, 12)
                .unwrap_err()
                .contains("continue after frame 10")
        );

        let mut rewind = TraceTimeline::new(10);
        assert!(
            rewind
                .observe(10, 9)
                .unwrap_err()
                .contains("retain or advance")
        );

        let mut jump = TraceTimeline::new(10);
        assert!(
            jump.observe(10, 12)
                .unwrap_err()
                .contains("retain or advance")
        );
    }

    #[test]
    fn retained_terminal_success_selects_the_omitted_quit_repair_only() {
        assert!(is_legacy_retained_terminal_success(
            TRACE_SCHEMA_VERSION,
            9_602,
            9_602,
            false,
            GameCode::LevelSucceeded as i32,
        ));
        assert!(!is_legacy_retained_terminal_success(
            TRACE_SCHEMA_VERSION,
            9_601,
            9_602,
            false,
            GameCode::LevelSucceeded as i32,
        ));
        assert!(!is_legacy_retained_terminal_success(
            TRACE_SCHEMA_VERSION,
            9_602,
            9_602,
            true,
            GameCode::LevelSucceeded as i32,
        ));
        assert!(!is_legacy_retained_terminal_success(
            TRACE_SCHEMA_VERSION,
            9_602,
            9_602,
            false,
            GameCode::LevelFailed as i32,
        ));
    }

    #[test]
    fn retained_terminal_success_repair_is_emitted_exactly_once() {
        let mut commands_before_hourglass = Vec::new();
        let mut commands_after_hourglass = Vec::new();
        let mut applied = false;
        let campaign_run_id = 0x1234_5678_9abc_def0;
        assert!(append_legacy_retained_terminal_success_repair(
            &mut commands_before_hourglass,
            &mut commands_after_hourglass,
            robin_engine::player_profile::DifficultyLevel::Medium,
            campaign_run_id,
            TRACE_SCHEMA_VERSION,
            9_602,
            9_602,
            false,
            GameCode::LevelSucceeded as i32,
            &mut applied,
        ));
        assert!(!append_legacy_retained_terminal_success_repair(
            &mut commands_before_hourglass,
            &mut commands_after_hourglass,
            robin_engine::player_profile::DifficultyLevel::Medium,
            campaign_run_id,
            TRACE_SCHEMA_VERSION,
            9_602,
            9_602,
            false,
            GameCode::LevelSucceeded as i32,
            &mut applied,
        ));
        assert_eq!(commands_before_hourglass.len(), 1);
        assert_eq!(commands_after_hourglass.len(), 1);
        assert!(matches!(
            commands_before_hourglass[0],
            PlayerCommand::QuitMissionRequested
        ));
        assert!(matches!(
            commands_after_hourglass[0],
            PlayerCommand::ApplyQuitMissionUpdates {
                exit_code: GameCode::LevelSucceeded,
                campaign_run_nonce: Some(actual),
                ..
            } if actual == campaign_run_id
        ));
    }

    #[test]
    fn legacy_presentation_sprite_rng_requires_exact_new_terminal_burst() {
        let offsets = [11, 12, 91, 91, 91, 91];
        let values = [1, 2, 3, 4, 5, 6];
        assert_eq!(
            legacy_presentation_sprite_rng_burst(
                14,
                true,
                GameCode::LevelInProgress as i32,
                true,
                3,
                false,
                &offsets,
                &values,
            ),
            Some(4),
        );
        assert_eq!(
            legacy_presentation_sprite_rng_burst(
                TRACE_SCHEMA_VERSION,
                true,
                GameCode::LevelInProgress as i32,
                true,
                4,
                false,
                &[11, 12, 91, 91, 91, 91, 91, 91],
                &[1, 2, 3, 4, 5, 6, 7, 8],
            ),
            Some(6),
        );

        assert_eq!(
            legacy_presentation_sprite_rng_burst(
                OLDEST_SUPPORTED_TRACE_SCHEMA - 1,
                true,
                GameCode::LevelInProgress as i32,
                true,
                4,
                false,
                &offsets,
                &values,
            ),
            None
        );
        assert_eq!(
            legacy_presentation_sprite_rng_burst(
                TRACE_SCHEMA_VERSION,
                false,
                GameCode::LevelInProgress as i32,
                true,
                4,
                false,
                &offsets,
                &values,
            ),
            None
        );
        assert_eq!(
            legacy_presentation_sprite_rng_burst(
                TRACE_SCHEMA_VERSION,
                true,
                GameCode::LevelInterrupted as i32,
                true,
                4,
                false,
                &offsets,
                &values,
            ),
            None
        );
        assert_eq!(
            legacy_presentation_sprite_rng_burst(
                TRACE_SCHEMA_VERSION,
                true,
                GameCode::LevelInProgress as i32,
                false,
                4,
                false,
                &offsets,
                &values,
            ),
            None
        );
        assert_eq!(
            legacy_presentation_sprite_rng_burst(
                TRACE_SCHEMA_VERSION,
                true,
                GameCode::LevelInProgress as i32,
                true,
                5,
                false,
                &offsets,
                &values,
            ),
            None
        );
    }

    #[test]
    fn legacy_presentation_sprite_rng_rejects_ambiguous_bursts() {
        assert_eq!(
            legacy_presentation_sprite_rng_burst(
                TRACE_SCHEMA_VERSION,
                true,
                GameCode::LevelInProgress as i32,
                true,
                4,
                false,
                &[91, 12, 91, 91, 91, 91],
                &[1, 2, 3, 4, 5, 6],
            ),
            None
        );
        assert_eq!(
            legacy_presentation_sprite_rng_burst(
                TRACE_SCHEMA_VERSION,
                true,
                GameCode::LevelInProgress as i32,
                true,
                4,
                false,
                &[91, 91, 92, 91, 91, 91],
                &[1, 2, 3, 4, 5, 6],
            ),
            None
        );
        assert_eq!(
            legacy_presentation_sprite_rng_burst(
                TRACE_SCHEMA_VERSION,
                true,
                GameCode::LevelInProgress as i32,
                true,
                4,
                false,
                &[91, 91, 91, 91],
                &[1, 2, 3, 4],
            ),
            None
        );
        assert_eq!(
            legacy_presentation_sprite_rng_burst(
                TRACE_SCHEMA_VERSION,
                true,
                GameCode::LevelInProgress as i32,
                true,
                0,
                false,
                &[11, 12],
                &[1, 2],
            ),
            None
        );
        assert_eq!(
            legacy_presentation_sprite_rng_burst(
                TRACE_SCHEMA_VERSION,
                true,
                GameCode::LevelInProgress as i32,
                true,
                3,
                false,
                &[11, 12, 91, 91, 91, 91],
                &[1, 2, 3, 4, 3, 6],
            ),
            None
        );
    }

    #[test]
    fn legacy_mobile_vibration_rng_accepts_only_new_terminal_xy_pairs() {
        assert_eq!(
            legacy_presentation_sprite_rng_burst(
                14,
                false,
                GameCode::LevelInProgress as i32,
                true,
                3,
                false,
                &[11, 12, 71, 72, 71, 72, 71, 72],
                &[1, 2, 3, 4, 5, 6, 7, 8],
            ),
            Some(6),
        );
        assert_eq!(
            legacy_presentation_sprite_rng_burst(
                TRACE_SCHEMA_VERSION,
                true,
                GameCode::LevelInProgress as i32,
                true,
                3,
                false,
                &[71, 12, 71, 72, 71, 72, 71, 72],
                &[1, 2, 3, 4, 5, 6, 7, 8],
            ),
            None,
            "either X/Y site appearing in the gameplay prefix is ambiguous",
        );
        assert_eq!(
            legacy_presentation_sprite_rng_burst(
                TRACE_SCHEMA_VERSION,
                true,
                GameCode::LevelInProgress as i32,
                true,
                3,
                false,
                &[11, 72, 71, 72, 71, 72, 71, 72],
                &[1, 2, 3, 4, 5, 6, 7, 8],
            ),
            None,
            "either X/Y site appearing in the gameplay prefix is ambiguous",
        );
        assert_eq!(
            legacy_presentation_sprite_rng_burst(
                TRACE_SCHEMA_VERSION,
                true,
                GameCode::LevelInProgress as i32,
                true,
                3,
                false,
                &[11, 12, 71, 72],
                &[1, 2, 3, 4],
            ),
            None,
            "one X/Y pair is not a repeated mobile-child burst",
        );
    }

    #[test]
    fn legacy_presentation_sprite_rng_requires_an_exact_unconsumed_suffix() {
        assert_eq!(
            missing_legacy_presentation_sprite_rng_draws(Some(13), 160, 173),
            Some(13)
        );
        assert_eq!(
            missing_legacy_presentation_sprite_rng_draws(Some(13), 173, 173),
            None,
            "a gameplay tick which consumed the homogeneous run must not replay it as presentation RNG"
        );
        assert_eq!(
            missing_legacy_presentation_sprite_rng_draws(Some(13), 161, 173),
            None,
            "a partially unmatched candidate is ambiguous"
        );
        assert_eq!(
            missing_legacy_presentation_sprite_rng_draws(Some(13), 174, 173),
            None,
            "an over-consuming tick remains an ordinary RNG mismatch"
        );
    }

    #[test]
    fn legacy_repeated_arrow_refresh_requires_a_previously_correlated_callsite() {
        let known = BTreeSet::from([71]);
        assert_eq!(
            legacy_additional_arrow_refresh_draws(
                TRACE_SCHEMA_VERSION,
                &[71, 71, 71, 71, 83],
                &known,
                1,
            ),
            Some(3),
        );
        assert_eq!(
            legacy_additional_arrow_refresh_draws(TRACE_SCHEMA_VERSION, &[71, 71, 83], &known, 2,),
            None,
            "two retained falling arrows explain one two-draw refresh",
        );
        assert_eq!(
            legacy_additional_arrow_refresh_draws(
                TRACE_SCHEMA_VERSION,
                &[72, 72, 72, 83],
                &known,
                1,
            ),
            None,
            "an uncorrelated homogeneous gameplay prefix is not presentation evidence",
        );
        assert_eq!(
            legacy_additional_arrow_refresh_draws(TRACE_SCHEMA_VERSION, &[71, 71, 83], &known, 0,),
            None,
            "a trace callsite cannot manufacture a missing falling arrow",
        );
    }

    #[test]
    fn legacy_teleport_star_rng_requires_exact_retained_lifecycle_and_ten_draws() {
        let state = |kind, index, creation_order, active, x| LegacyPresentationEntityState {
            entity_id: TraceEntityId { kind, index },
            creation_order,
            kind,
            active,
            position_bits: [x, 0],
        };
        let previous = [
            state(TraceEntityKind::Pc, 130, 130, false, 10),
            state(TraceEntityKind::Target, 223, 223, true, 20),
            state(TraceEntityKind::Pc, 7, 7, true, 30),
            state(TraceEntityKind::Bonus, 234, 234, false, 40),
        ];
        let current = [
            state(TraceEntityKind::Pc, 130, 130, true, 11),
            state(TraceEntityKind::Target, 223, 223, false, 20),
            state(TraceEntityKind::Pc, 7, 7, true, 30),
            state(TraceEntityKind::Bonus, 234, 234, true, 40),
        ];
        assert!(has_legacy_teleport_star_lifecycle(
            Some(&previous),
            &current
        ));

        let offsets = [11, 12, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91];
        let values = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        assert_eq!(
            legacy_presentation_sprite_rng_burst(
                TRACE_SCHEMA_VERSION,
                true,
                GameCode::LevelInProgress as i32,
                true,
                29,
                true,
                &offsets,
                &values,
            ),
            Some(10)
        );
        assert_eq!(
            legacy_presentation_sprite_rng_burst(
                TRACE_SCHEMA_VERSION,
                true,
                GameCode::LevelInProgress as i32,
                true,
                29,
                false,
                &offsets,
                &values,
            ),
            None
        );
        assert_eq!(
            legacy_presentation_sprite_rng_burst(
                TRACE_SCHEMA_VERSION,
                true,
                GameCode::LevelInProgress as i32,
                true,
                29,
                true,
                &offsets[..11],
                &values[..11],
            ),
            None
        );

        let mut pc_did_not_move = current;
        pc_did_not_move[0].position_bits = previous[0].position_bits;
        assert!(!has_legacy_teleport_star_lifecycle(
            Some(&previous),
            &pc_did_not_move
        ));
        let mut unrelated_transition = current;
        unrelated_transition[2].active = false;
        assert!(!has_legacy_teleport_star_lifecycle(
            Some(&previous),
            &unrelated_transition
        ));
        assert!(!has_legacy_teleport_star_lifecycle(None, &current));
        assert!(!has_legacy_teleport_star_lifecycle(
            Some(&previous[..2]),
            &current
        ));
    }

    #[test]
    fn replay_campaign_identity_is_stable_across_recording_family_sessions() {
        let first = replay_campaign_run_id(
            Path::new("interactive-session-002-session-0001.jsonl.zst"),
            1,
        );
        let ninth = replay_campaign_run_id(
            Path::new("interactive-session-002-session-0009.jsonl.zst"),
            9,
        );
        let other = replay_campaign_run_id(
            Path::new("interactive-session-003-session-0001.jsonl.zst"),
            1,
        );

        assert_ne!(first, 0);
        assert_eq!(first, ninth);
        assert_eq!(
            first,
            replay_campaign_run_id(
                Path::new("interactive-session-002-session-0001.jsonl.zst.parity.bitcode.zst"),
                1,
            )
        );
        assert_ne!(first, other);
    }

    #[test]
    fn fixed_native_footer_rejects_early_end_and_trailing_records() {
        let footer = BinaryTraceFooter {
            version: TRACE_NATIVE_VERSION,
            frame_count: 2,
            final_frame: 12,
        };
        let early = write_test_native_records(&[complete_test_end(1, 11)], Some(footer));
        let mut reader = BinaryTraceReader::open(early.path());
        assert!(matches!(
            reader.read_record(),
            BinaryTraceRecord::End { .. }
        ));
        assert!(
            reader
                .validate_terminator(1, 11)
                .unwrap_err()
                .contains("fixed footer says frame_count=2")
        );

        let footer = BinaryTraceFooter {
            version: TRACE_NATIVE_VERSION,
            frame_count: 0,
            final_frame: 10,
        };
        let trailing = write_test_native_records(
            &[complete_test_end(0, 10), complete_test_end(0, 10)],
            Some(footer),
        );
        let mut reader = BinaryTraceReader::open(trailing.path());
        assert!(matches!(
            reader.read_record(),
            BinaryTraceRecord::End { .. }
        ));
        assert!(
            reader
                .validate_terminator(0, 10)
                .unwrap_err()
                .contains("first End")
        );

        // Same trailing record, but inside the End's own block.
        let footer = BinaryTraceFooter {
            version: TRACE_NATIVE_VERSION,
            frame_count: 0,
            final_frame: 10,
        };
        let mut file = tempfile::NamedTempFile::new().unwrap();
        {
            let mut encoder = zstd::stream::write::Encoder::new(
                BufWriter::new(file.as_file_mut()),
                TRACE_NATIVE_ZSTD_LEVEL,
            )
            .unwrap();
            // Coerce to a slice: bitcode encodes fixed-size arrays without a
            // length, which would not decode as the reader's `Vec` blocks.
            write_binary_record(
                &mut encoder,
                [complete_test_end(0, 10), complete_test_end(0, 10)].as_slice(),
                "test native trace block",
            );
            let mut writer = encoder.finish().unwrap();
            write_binary_trace_footer(&mut writer, footer).unwrap();
            writer.flush().unwrap();
        }
        let mut reader = BinaryTraceReader::open(file.path());
        assert!(matches!(
            reader.read_record(),
            BinaryTraceRecord::End { .. }
        ));
        assert!(
            reader
                .validate_terminator(0, 10)
                .unwrap_err()
                .contains("first End")
        );
    }

    #[test]
    fn fixed_native_footer_rejects_missing_and_malformed_data() {
        let missing = write_test_native_records(&[complete_test_end(0, 10)], None);
        let missing_error = read_binary_trace_footer(missing.path()).unwrap_err();
        assert!(
            missing_error.contains("footer magic") || missing_error.contains("shorter than"),
            "unexpected missing-footer error: {missing_error}"
        );

        let mut malformed = tempfile::NamedTempFile::new().unwrap();
        malformed
            .write_all(&vec![0_u8; TRACE_NATIVE_FOOTER_LEN as usize])
            .unwrap();
        assert!(
            read_binary_trace_footer(malformed.path())
                .unwrap_err()
                .contains("footer magic")
        );
    }

    #[test]
    fn movement_and_flight_steps_preserve_exact_operands() {
        let empty: TraceFrame = serde_json::from_value(minimal_frame_json()).unwrap();
        assert!(empty.movement_steps.is_empty());
        assert!(empty.flight_steps.is_empty());

        let mut instrumented = minimal_frame_json();
        instrumented["movement_steps"] = serde_json::json!([{
            "entity": { "kind": "pc", "index": 344 },
            "order_id": 91,
            "order_action": 303,
            "animation": 17,
            "motion_method": 2,
            "pre_position": {
                "x": { "bits": 0x44a1_0001_u32 },
                "y": { "bits": 0x4480_0002_u32 }
            },
            "old_position": {
                "x": { "bits": 0x44a0_0003_u32 },
                "y": { "bits": 0x447f_0004_u32 }
            },
            "goal": {
                "x": { "bits": 0x44b0_0005_u32 },
                "y": { "bits": 0x4490_0006_u32 }
            },
            "cached_increment": {
                "x": { "bits": 0x3f00_0007_u32 },
                "y": { "bits": 0x3f40_0008_u32 }
            },
            "frame_distance_raw": { "bits": 0x4000_0009_u32 },
            "speed_factor": { "bits": 0x3f80_000a_u32 },
            "effective_distance": { "bits": 0x4000_000b_u32 },
            "anti_collision": true,
            "reverse": false,
            "raw_post_position": {
                "x": { "bits": 0x44a1_800c_u32 },
                "y": { "bits": 0x4480_800d_u32 }
            },
            "raw_committed_delta": {
                "x": { "bits": 0x3f00_0010_u32 },
                "y": { "bits": 0x3e80_0011_u32 }
            },
            "post_position": {
                "x": { "bits": 0x44a2_000c_u32 },
                "y": { "bits": 0x4481_000d_u32 }
            },
            "committed_delta": {
                "x": { "bits": 0x3f80_000e_u32 },
                "y": { "bits": 0x3f00_000f_u32 }
            },
            "goal_reached": true,
            "snapped_to_goal": true
        }]);
        instrumented["flight_steps"] = serde_json::json!([{
            "entity": { "kind": "soldier", "index": 102 },
            "order_id": 1056453,
            "order_action": 303,
            "animation": 17,
            "flight_style": 0,
            "entry_position": { "x": { "bits": 0x4448_4eb2_u32 }, "y": { "bits": 0x4505_7800_u32 }, "z": { "bits": 0_u32 } },
            "entry_position_map": { "x": { "bits": 0x4448_4eb2_u32 }, "y": { "bits": 0x4505_7800_u32 } },
            "old_position": { "x": { "bits": 0x4448_4eb2_u32 }, "y": { "bits": 0x4505_7800_u32 }, "z": { "bits": 0_u32 } },
            "old_position_map": { "x": { "bits": 0x4448_4eb2_u32 }, "y": { "bits": 0x4505_7800_u32 } },
            "goal": { "x": { "bits": 0x4448_4eb3_u32 }, "y": { "bits": 0x4505_7804_u32 }, "z": { "bits": 0_u32 } },
            "cached_increment": { "x": { "bits": 0_u32 }, "y": { "bits": 0_u32 }, "z": { "bits": 0_u32 } },
            "applied_increment": { "x": { "bits": 0_u32 }, "y": { "bits": 0_u32 }, "z": { "bits": 0_u32 } },
            "raw_post_position": { "x": { "bits": 0x4448_4eb2_u32 }, "y": { "bits": 0x4505_7800_u32 }, "z": { "bits": 0_u32 } },
            "raw_post_position_map": { "x": { "bits": 0x4448_4eb2_u32 }, "y": { "bits": 0x4505_7800_u32 } },
            "motion_state": 3,
            "post_position": { "x": { "bits": 0x4448_4eb3_u32 }, "y": { "bits": 0x4505_7804_u32 }, "z": { "bits": 0_u32 } },
            "post_position_map": { "x": { "bits": 0x4448_4eb3_u32 }, "y": { "bits": 0x4505_7804_u32 } },
            "snapped_to_goal": true
        }]);
        let parsed: TraceFrame = serde_json::from_value(instrumented).unwrap();
        let step = parsed
            .movement_steps
            .first()
            .expect("instrumented frame retains its movement step");
        assert_eq!(step.entity.index, 344);
        assert_eq!(step.order_id, 91);
        assert_eq!(step.pre_position.x.bits, 0x44a1_0001);
        assert_eq!(step.cached_increment.y.bits, 0x3f40_0008);
        assert_eq!(step.effective_distance.bits, 0x4000_000b);
        assert_eq!(step.raw_post_position.x.bits, 0x44a1_800c);
        assert_eq!(step.committed_delta.y.bits, 0x3f00_000f);
        assert!(step.goal_reached);
        assert!(step.snapped_to_goal);
        let flight = parsed
            .flight_steps
            .first()
            .expect("instrumented frame retains its flight step");
        assert_eq!(flight.entity.index, 102);
        assert_eq!(flight.raw_post_position_map.x.bits, 0x4448_4eb2);
        assert_eq!(flight.post_position_map.x.bits, 0x4448_4eb3);
        assert!(flight.snapped_to_goal);

        let encoded = bitcode::encode(&parsed);
        let roundtrip: TraceFrame =
            bitcode::decode(&encoded).expect("decode instrumented frame with the cache layout");
        assert_eq!(roundtrip.movement_steps.len(), 1);
        assert_eq!(roundtrip.movement_steps[0].entity.index, 344);
        assert_eq!(
            roundtrip.movement_steps[0].raw_committed_delta.x.bits,
            0x3f00_0010
        );
        assert_eq!(roundtrip.flight_steps.len(), 1);
        assert_eq!(roundtrip.flight_steps[0].entity.index, 102);
        assert_eq!(
            roundtrip.flight_steps[0].post_position_map.x.bits,
            0x4448_4eb3
        );
    }

    #[test]
    fn sword_seek_distance_defaults_legacy_records_and_direct_distance_is_ignored() {
        let missing = serde_json::json!({
            "type": "sword_strike",
            "actor": { "kind": "pc", "index": 3 },
            "target": { "kind": "soldier", "index": 7 },
            "original_command": 78,
            "original_command_name": "swordstrike_thrust_a",
            "with_seek": true
        });
        let missing: TraceCommand = serde_json::from_value(missing)
            .expect("legacy sword-strike command may omit seek distance");
        let TraceCommand::SwordStrike { seek_distance, .. } = missing else {
            panic!("decoded wrong trace command")
        };
        assert!(seek_distance.is_nan());

        assert_eq!(trace_sword_seek_distance(false, 0.0), None);
        assert_eq!(trace_sword_seek_distance(true, seek_distance), None);
        assert_eq!(trace_sword_seek_distance(true, 63.0), Some(63.0));
    }

    #[test]
    fn trace_content_hash_distinguishes_equal_length_sources() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first.jsonl.zst");
        let second = directory.path().join("second.jsonl.zst");
        std::fs::write(&first, b"equal-length-a").unwrap();
        std::fs::write(&second, b"equal-length-b").unwrap();

        assert_eq!(
            std::fs::metadata(&first).unwrap().len(),
            std::fs::metadata(&second).unwrap().len()
        );
        assert_ne!(trace_content_sha256(&first), trace_content_sha256(&second));
        assert!(trace_source_fingerprint(&first).contains(":sha256="));
    }

    fn valid_initial_save_with_profile(source_profile: TraceSaveSourceProfile) -> TraceInitialSave {
        let mut bytes = Vec::from(*source_profile.expected_magic());
        bytes.extend_from_slice(&48_u32.to_le_bytes());
        bytes.extend_from_slice(&16_723_u32.to_le_bytes());
        bytes.extend_from_slice(&48_u32.to_le_bytes());
        bytes.extend_from_slice(b"serialized save body");
        TraceInitialSave {
            format: "rhsg".to_owned(),
            source_profile,
            encoding: "base64".to_owned(),
            byte_length: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
            slot: "Restart".to_owned(),
            header_version: 48,
            mission_id: 16_723,
            stream_version: 48,
            data: base64::engine::general_purpose::STANDARD.encode(bytes),
        }
    }

    fn valid_initial_save() -> TraceInitialSave {
        valid_initial_save_with_profile(TraceSaveSourceProfile::LinuxI386RhsgV48)
    }

    #[test]
    fn jsonl_trace_reader_accepts_plain_and_zstd_content() {
        let jsonl = b"{\"type\":\"header\"}\n{\"type\":\"rng_prefix\"}\n";
        let directory = tempfile::tempdir().unwrap();
        let plain_path = directory.path().join("trace.jsonl");
        let compressed_path = directory.path().join("trace.jsonl.zst");
        std::fs::write(&plain_path, jsonl).unwrap();
        std::fs::write(
            &compressed_path,
            zstd::stream::encode_all(std::io::Cursor::new(jsonl), 1).unwrap(),
        )
        .unwrap();

        for path in [plain_path, compressed_path] {
            let mut decoded = String::new();
            open_jsonl_trace(&path)
                .read_to_string(&mut decoded)
                .unwrap();
            assert_eq!(decoded.as_bytes(), jsonl);
        }
    }

    #[test]
    fn jsonl_trace_reader_accepts_zstd_frames_with_large_declared_windows() {
        let jsonl = b"{\"type\":\"header\"}\n{\"type\":\"rng_prefix\"}\n";
        let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 1).unwrap();
        encoder.window_log(28).unwrap();
        encoder.write_all(jsonl).unwrap();
        let compressed = encoder.finish().unwrap();

        // Ensure this fixture exercises a frame rejected by zstd's default
        // 128 MiB window cap rather than merely duplicating the ordinary-zstd
        // coverage above.
        let mut default_decoder =
            zstd::stream::read::Decoder::new(std::io::Cursor::new(&compressed)).unwrap();
        let mut default_output = Vec::new();
        assert!(default_decoder.read_to_end(&mut default_output).is_err());

        let directory = tempfile::tempdir().unwrap();
        let compressed_path = directory.path().join("large-window-trace.jsonl.zst");
        std::fs::write(&compressed_path, compressed).unwrap();

        let mut decoded = String::new();
        open_jsonl_trace(&compressed_path)
            .read_to_string(&mut decoded)
            .unwrap();
        assert_eq!(decoded.as_bytes(), jsonl);
    }

    #[test]
    fn compression_window_is_bounded_for_parallel_replay_lanes() {
        // Unknown and sufficiently large streams use the 64 MiB maximum;
        // known smaller streams retain only the power-of-two window they need.
        assert_eq!(native_stream_window_log(None), TRACE_NATIVE_WINDOW_LOG);
        assert_eq!(native_stream_window_log(Some(32 * 1024 * 1024)), 25);
        assert_eq!(native_stream_window_log(Some(64 * 1024 * 1024)), 26);
        assert_eq!(
            native_stream_window_log(Some(64 * 1024 * 1024 + 1)),
            TRACE_NATIVE_WINDOW_LOG
        );
        // Tiny estimates retain the encoder's accepted minimum.
        assert_eq!(native_stream_window_log(Some(0)), 20);
        assert_eq!(native_stream_window_log(Some(1)), 20);
        assert_eq!(
            native_stream_window_log(Some(u64::MAX)),
            TRACE_NATIVE_WINDOW_LOG
        );
        assert_eq!(
            native_stream_window_log_capped(None, TRACE_NATIVE_MIN_WINDOW_LOG),
            TRACE_NATIVE_MIN_WINDOW_LOG
        );
        assert_eq!(TRACE_NATIVE_BLOCK_RECORDS, 32);
    }

    #[test]
    fn recording_size_comes_from_the_zstd_frame_when_the_recording_is_compressed() {
        let directory = tempfile::tempdir().unwrap();
        let jsonl = vec![b'{'; 4096];

        let plain = directory.path().join("plain.jsonl");
        std::fs::write(&plain, &jsonl).unwrap();
        assert_eq!(recording_uncompressed_bytes(&plain), Some(4096));

        // A recording compressed as a single frame declares its content size,
        // which is what the window has to be sized against -- the file length
        // would size the window against the compressed bytes instead.
        let compressed = directory.path().join("declared.jsonl.zst");
        std::fs::write(&compressed, zstd::bulk::compress(&jsonl, 1).unwrap()).unwrap();
        assert!(std::fs::metadata(&compressed).unwrap().len() < 4096);
        assert_eq!(recording_uncompressed_bytes(&compressed), Some(4096));

        // A streamed frame may not declare one; the caller then uses the
        // bounded replay-memory window rather than guessing from file size.
        let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 1).unwrap();
        encoder.write_all(&jsonl).unwrap();
        let streamed = directory.path().join("undeclared.jsonl.zst");
        std::fs::write(&streamed, encoder.finish().unwrap()).unwrap();
        assert_eq!(recording_uncompressed_bytes(&streamed), None);
        assert_eq!(
            native_stream_window_log(expected_native_stream_bytes(&streamed)),
            TRACE_NATIVE_WINDOW_LOG
        );
    }

    #[test]
    fn all_existing_trace_schemas_are_accepted() {
        for schema in OLDEST_SUPPORTED_TRACE_SCHEMA..=TRACE_SCHEMA_VERSION {
            validate_trace_schema(schema);
            assert!(trace_schema_is_supported(schema));
        }
        assert!(!trace_schema_is_supported(
            OLDEST_SUPPORTED_TRACE_SCHEMA - 1
        ));
        assert!(!trace_schema_is_supported(TRACE_SCHEMA_VERSION + 1));
    }

    #[test]
    fn current_door_and_route_diagnostics_are_typed() {
        let actor: TraceActor = serde_json::from_value(serde_json::json!({
            "action_state": 1,
            "animation": 12,
            "command": 19,
            "command_name": "pass_door",
            "motion_state": 2,
            "wait_time": 0,
            "passing_door_directly": true,
            "active_pass_door": { "gate_id": 51, "direct": true, "direction": 1 },
            "sequence_element": {
                "id": 160,
                "type": 4,
                "state": 2,
                "command_level": 7,
                "command": 19,
                "command_name": "pass_door",
                "order_count": 3,
                "priority": 8,
                "posture_after_transition": 1,
                "action_state_after_transition": 1,
                "movement": {
                    "action": 12,
                    "pass_door": { "gate_id": 51, "direct": true, "direction": 1 }
                },
                "following": null,
                "postponed": null,
                "current_order": null,
                "movement_payload": null
            },
            "position_interface": {}
        }))
        .expect("parse current-schema actor diagnostics");
        assert!(actor.passing_door_directly);
        assert_eq!(
            actor
                .active_pass_door
                .as_ref()
                .map(|pass| (pass.gate_id, pass.direction)),
            Some((51, 1))
        );
        let pass = actor.active_pass_door.as_ref().unwrap();
        assert!(active_pass_door_keys_match(Some(pass), Some((51, true))));
        assert!(!active_pass_door_keys_match(Some(pass), Some((51, false))));
        assert!(!active_pass_door_keys_match(Some(pass), None));
        assert_eq!(
            actor
                .sequence_element
                .as_ref()
                .map(|element| (element.id, element.order_count)),
            Some((160, 3))
        );

        let mut frame_json = minimal_frame_json();
        frame_json["route_construction_events"] = serde_json::json!([{
            "kind": "move",
            "actor": { "kind": "soldier", "index": 43 },
            "source": { "x": { "bits": 1154109440_u32 }, "y": { "bits": 1153748992_u32 } },
            "source_sector": 61,
            "source_level": 11,
            "goal": { "x": { "bits": 1155563520_u32 }, "y": { "bits": 1147920384_u32 } },
            "goal_sector": 72,
            "goal_level": 11,
            "gates": [{
                "gate_id": 51,
                "direct": false,
                "sector_out": 60,
                "level_out": 11,
                "sector_in": 61,
                "level_in": 11
            }]
        }]);
        let frame: TraceFrame = serde_json::from_value(frame_json)
            .expect("parse current-schema route-construction diagnostics");
        validate_trace_frame(TRACE_SCHEMA_VERSION, &frame);
        let route = &frame.route_construction_events[0];
        assert_eq!(route.actor.index, 43);
        assert_eq!(route.gates[0].gate_id, 51);
        assert!(!route.gates[0].direct);
    }

    #[test]
    fn current_schema_jump_lines_are_typed_parallel_and_cache_safe() {
        let human: TraceHuman = serde_json::from_value(serde_json::json!({
            "life_points": 40,
            "dead": false,
            "unconscious": false,
            "camp": "lacklandists",
            "original_camp": 1,
            "vip": false,
            "civilian": false,
            "opponents": [{"kind": "pc", "index": 3}],
            "opponent_jump_lines": [{
                "a": {"x": {"bits": 1065353216}, "y": {"bits": 1073741824}},
                "b": {"x": {"bits": 1077936128}, "y": {"bits": 1082130432}}
            }]
        }))
        .expect("parse schema-16 opponent jump-line geometry");
        let entity: TraceEntityId = serde_json::from_value(serde_json::json!({
            "kind": "soldier", "index": 8
        }))
        .unwrap();
        validate_human_jump_line_shape(&entity, &human, false);

        let ai: TraceAi = serde_json::from_value(serde_json::json!({
            "state": 1,
            "substate": 2,
            "script_locked": false,
            "locked": false,
            "locks": 0,
            "was_busy": false,
            "very_busy": false,
            "macro_timer_running": false,
            "macro_timer_ring": 0,
            "macro_cursor": null,
            "macro_remaining": 0,
            "macro_in_progress": false,
            "list_us": [],
            "list_them": [],
            "my_line_jump": null
        }))
        .expect("parse authoritative null soldier jump line");
        assert!(ai.my_line_jump.is_none());

        let encoded = bitcode::encode(&(human, ai));
        let (cached_human, cached_ai): (TraceHuman, TraceAi) =
            bitcode::decode(&encoded).expect("restore typed jump-line snapshots");
        assert_eq!(cached_human.opponent_jump_lines.len(), 1);
        assert!(cached_ai.my_line_jump.is_none());

        let malformed_line = serde_json::json!({
            "a": {"x": {"bits": 0}, "y": {"bits": 0}},
            "b": {"x": {"bits": 0}, "y": {"bits": 0}},
            "pointer": 123
        });
        assert!(serde_json::from_value::<TraceJumpLine>(malformed_line).is_err());
    }

    #[test]
    #[should_panic(expected = "opponent and jump-line arrays differ in length")]
    fn current_schema_rejects_misaligned_opponent_jump_lines() {
        let human: TraceHuman = serde_json::from_value(serde_json::json!({
            "life_points": 40,
            "dead": false,
            "unconscious": false,
            "camp": "lacklandists",
            "original_camp": 1,
            "vip": false,
            "civilian": false,
            "opponents": [{"kind": "pc", "index": 3}],
            "opponent_jump_lines": []
        }))
        .unwrap();
        let entity: TraceEntityId = serde_json::from_value(serde_json::json!({
            "kind": "soldier", "index": 8
        }))
        .unwrap();
        validate_human_jump_line_shape(&entity, &human, false);
    }

    #[test]
    fn legacy_schema_accepts_omitted_opponent_jump_lines() {
        let human: TraceHuman = serde_json::from_value(serde_json::json!({
            "life_points": 40,
            "dead": false,
            "unconscious": false,
            "camp": "lacklandists",
            "original_camp": 1,
            "vip": false,
            "civilian": false,
            "opponents": [{"kind": "pc", "index": 3}],
            "opponent_jump_lines": []
        }))
        .unwrap();
        let entity: TraceEntityId = serde_json::from_value(serde_json::json!({
            "kind": "soldier", "index": 8
        }))
        .unwrap();

        validate_human_jump_line_shape(&entity, &human, true);
    }

    #[test]
    fn current_schema_event_window_resets_only_after_record_frame() {
        let mut pending = Vec::<(u64, &'static str)>::new();
        let mut next_ordinal = 0_u64;
        fn emit(
            pending: &mut Vec<(u64, &'static str)>,
            next_ordinal: &mut u64,
            stage: &'static str,
        ) {
            pending.push((*next_ordinal, stage));
            *next_ordinal += 1;
        }

        emit(&mut pending, &mut next_ordinal, "input_before_hourglass");
        emit(&mut pending, &mut next_ordinal, "simulation_body");
        let first_frame = std::mem::take(&mut pending);
        next_ordinal = 0; // Frame recording serialized and then reset the window.

        emit(
            &mut pending,
            &mut next_ordinal,
            "refresh_after_record_frame",
        );
        // Starting an engine frame intentionally does not clear or reset event state.
        emit(
            &mut pending,
            &mut next_ordinal,
            "next_input_before_hourglass",
        );
        emit(&mut pending, &mut next_ordinal, "next_simulation_body");
        let second_frame = std::mem::take(&mut pending);

        assert_eq!(
            first_frame,
            vec![(0, "input_before_hourglass"), (1, "simulation_body")]
        );
        assert_eq!(
            second_frame,
            vec![
                (0, "refresh_after_record_frame"),
                (1, "next_input_before_hourglass"),
                (2, "next_simulation_body")
            ]
        );
    }

    #[test]
    fn current_schema_target_unreached_observations_are_explicitly_nullable() {
        let event: TraceTargetLifecycleEvent = serde_json::from_value(serde_json::json!({
            "ordinal": 0,
            "frame_ordinal": 0,
            "phase": "engine_send_message_entry",
            "sequence_id": null,
            "sequence_element_id": 2,
            "command_level": 1,
            "state": 0,
            "command": 15,
            "command_name": "send_message",
            "owner": null,
            "context": null,
            "antagonist": null,
            "antagonist_observed": null,
            "payload": {
                "kind": "send_message",
                "message": null,
                "argument": null,
                "argument_raw": null,
                "extended_argument": null,
                "extended_argument_raw": null
            },
            "payload_observed": false,
            "script_enabled": false,
            "class_instantiated": null
        }))
        .expect("parse unreached schema-16 target evidence");
        assert!(event.sequence_id.is_none());
        assert_eq!(event.payload_observed, Some(false));
        assert!(matches!(
            event.payload,
            TraceTargetLifecyclePayload::SendMessage {
                message: None,
                argument: None,
                ..
            }
        ));
        let encoded = bitcode::encode(&event);
        let cached: TraceTargetLifecycleEvent = bitcode::decode(&encoded).unwrap();
        assert!(cached.sequence_id.is_none());
        assert_eq!(cached.payload_observed, Some(false));
    }

    #[test]
    fn current_schema_movement_action_is_only_present_when_initialized() {
        let sequence = |command: u16,
                        command_name: &str,
                        movement: serde_json::Value,
                        movement_payload: serde_json::Value| {
            serde_json::from_value::<TraceSequenceElement>(serde_json::json!({
                "id": 5,
                "type": 4,
                "state": 2,
                "command_level": 1,
                "command": command,
                "command_name": command_name,
                "order_count": 1,
                "priority": 0,
                "posture_after_transition": 0,
                "action_state_after_transition": 0,
                "movement": movement,
                "following": null,
                "postponed": null,
                "current_order": {"id": 77, "action": 0},
                "movement_payload": movement_payload
            }))
            .expect("parse schema-16 movement constructor-shaped evidence")
        };

        let wait_free_lift = sequence(
            0,
            "wait_free_lift",
            serde_json::json!({}),
            serde_json::json!({
                "speed_factor": {"bits": 1065353216, "value": 1.0},
                "destination": {"x": {"bits": 0}, "y": {"bits": 0}},
                "tolerance": {"bits": 0, "value": 0.0},
                "flags": 0,
                "target": null,
                "sector": null
            }),
        );
        assert_eq!(
            wait_free_lift
                .movement
                .as_ref()
                .expect("movement evidence")
                .action,
            None
        );
        assert!(
            wait_free_lift
                .movement_payload
                .as_ref()
                .expect("movement payload")
                .to_json()
                .get("action")
                .is_none()
        );

        let movement = sequence(
            1,
            "move",
            serde_json::json!({"action": 12}),
            serde_json::json!({
                "speed_factor": {"bits": 1065353216, "value": 1.0},
                "action": 12,
                "destination": {"x": {"bits": 0}, "y": {"bits": 0}},
                "tolerance": {"bits": 0, "value": 0.0},
                "flags": 3,
                "target": null,
                "sector": null
            }),
        );
        assert_eq!(
            movement
                .movement
                .as_ref()
                .expect("movement evidence")
                .action,
            Some(12)
        );
        assert_eq!(
            movement
                .movement_payload
                .as_ref()
                .expect("movement payload")
                .to_json()
                .get("action"),
            Some(&serde_json::json!(12))
        );

        let encoded = bitcode::encode(&[wait_free_lift, movement]);
        let cached: [TraceSequenceElement; 2] = bitcode::decode(&encoded).unwrap();
        assert_eq!(cached[0].movement.as_ref().unwrap().action, None);
        assert_eq!(cached[1].movement.as_ref().unwrap().action, Some(12));
    }

    #[test]
    fn current_diagnostics_are_cached_and_required() {
        validate_trace_schema(TRACE_SCHEMA_VERSION);

        let actor: TraceActor = serde_json::from_value(serde_json::json!({
            "action_state": 1,
            "animation": 12,
            "command": 19,
            "command_name": "pass_door",
            "motion_state": 2,
            "wait_time": 0,
            "passing_door_directly": true,
            "active_pass_door": null,
            "position_interface": {
                "move_box": {
                    "top_left": {"x": {"bits": 0}, "y": {"bits": 0}},
                    "bottom_right": {"x": {"bits": 0}, "y": {"bits": 0}}
                },
                "anti_collision_on": true,
                "deviated": false,
                "blocked_count": 0,
                "box_blocked": {
                    "top_left": {"x": {"bits": 0}, "y": {"bits": 0}},
                    "bottom_right": {"x": {"bits": 0}, "y": {"bits": 0}}
                },
                "radius": {"bits": 1065353216, "value": 1.0}
            },
            "sequence_element": {
                "id": 5,
                "type": 4,
                "state": 2,
                "command_level": 7,
                "command": 19,
                "command_name": "pass_door",
                "order_count": 1,
                "priority": 8,
                "posture_after_transition": 1,
                "action_state_after_transition": 1,
                "movement": {"action": 12},
                "following": null,
                "postponed": {"id": 6, "command": 2},
                "current_order": {"id": 77, "action": 12},
                "movement_payload": {"flags": 3, "speed_factor": {"bits": 1065353216, "value": 1.0}}
            }
        }))
        .expect("parse current actor and sequence diagnostics");
        let sequence = actor.sequence_element.as_ref().unwrap();
        assert!(sequence.current_order.is_some());
        assert!(sequence.movement_payload.is_some());

        let mut frame_json = minimal_frame_json();
        frame_json["route_construction_events"] = serde_json::json!([{
            "kind": "move",
            "actor": { "kind": "soldier", "index": 7 },
            "source": { "x": { "bits": 0 }, "y": { "bits": 0 } },
            "source_sector": 1,
            "source_level": 2,
            "goal": { "x": { "bits": 0 }, "y": { "bits": 0 } },
            "goal_sector": 3,
            "goal_level": 2,
            "gates": [{
                "gate_id": 9,
                "direct": true,
                "sector_out": 1,
                "level_out": 2,
                "sector_in": 3,
                "level_in": 2,
                "kind": "door",
                "score": { "bits": 1065353216, "value": 1.0 }
            }],
            "ordinal": 4,
            "phase": "constructed",
            "result": "success"
        }]);
        frame_json["popup_events"] = serde_json::json!([{
            "ordinal": 0,
            "stage": "nested_refresh_entry",
            "universal_frame_counter": 44,
            "source_surface": 7,
            "colorize_background": true,
            "remove_mouse": false
        }]);
        frame_json["ai_forecast_events"] = serde_json::json!([{
            "ordinal": 0,
            "phase": "resolved",
            "target": {"kind": "soldier", "index": 7},
            "input": {
                "position": {"x": {"bits": 0}, "y": {"bits": 0}},
                "sector": 1,
                "level": 2,
                "passing_door": false
            },
            "moving_upwards": false,
            "resolution": "current_position",
            "resolved": {
                "position": {"x": {"bits": 0}, "y": {"bits": 0}},
                "sector": 1,
                "level": 2,
                "direction": 3
            }
        }]);
        frame_json["alert_formation_events"] = serde_json::json!([{
            "ordinal": 0,
            "stage": "candidate",
            "invocation": 9,
            "scan_index": 2,
            "candidate": {"kind": "soldier", "index": 8},
            "active": true,
            "script_locked": false,
            "eligibility": {
                "rank": true,
                "able_to_help": true,
                "allowed_to_leave_post": true,
                "can_call": false,
                "max_radius": null,
                "squared_radius": null,
                "capacity": null,
                "think": null
            },
            "rejection_stage": "can_call"
        }]);
        frame_json["goto_authorization_events"] = serde_json::json!([
            {
                "ordinal": 0,
                "actor": {"kind": "soldier", "index": 7},
                "source": {"point": {"x": {"bits": 0}, "y": {"bits": 0}}, "layer": 2},
                "move_box": {
                    "top_left": {"x": {"bits": 0}, "y": {"bits": 0}},
                    "bottom_right": {"x": {"bits": 0}, "y": {"bits": 0}}
                },
                "destination": {
                    "point": {"x": {"bits": 0}, "y": {"bits": 0}},
                    "sector": 3,
                    "layer": 2
                },
                "requested_flags": 1,
                "effective_flags": 3,
                "phase": "straight",
                "outcome": "blocked",
                "straight_authorized": false,
                "path_authorized": null
            },
            {
                "ordinal": 1,
                "actor": {"kind": "soldier", "index": 7},
                "source": null,
                "move_box": null,
                "destination": {
                    "point": {"x": {"bits": 0}, "y": {"bits": 0}},
                    "sector": null,
                    "layer": 2
                },
                "requested_flags": 1,
                "effective_flags": 1,
                "phase": "already_there",
                "outcome": "accepted",
                "straight_authorized": null,
                "path_authorized": null
            }
        ]);
        frame_json["target_lifecycle_events"] = serde_json::json!([{
            "ordinal": 0,
            "frame_ordinal": 6,
            "phase": "engine_send_message_callback_entry",
            "sequence_id": 701,
            "sequence_element_id": 902,
            "command_level": 2,
            "state": 0,
            "command": 15,
            "command_name": "send_message",
            "owner": null,
            "context": null,
            "antagonist": null,
            "antagonist_observed": null,
            "payload": {
                "kind": "send_message",
                "message": 10,
                "argument": -1,
                "argument_raw": 4294967295u64,
                "extended_argument": 0,
                "extended_argument_raw": 0
            },
            "payload_observed": true,
            "script_enabled": true,
            "class_instantiated": null
        }]);
        frame_json["strike_proposal_events"] = serde_json::json!([
            {
                "invocation": 0,
                "ordinal": 0,
                "frame_ordinal": 0,
                "phase": "entry",
                "actor": {"kind": "pc", "index": 342},
                "actor_creation_order": 373,
                "threat": {"kind": "soldier", "index": 140},
                "threat_creation_order": 171,
                "principal_opponent": null,
                "principal_opponent_creation_order": null,
                "command": 0,
                "command_name": "null",
                "also_parade": true,
                "only_parade": false,
                "candidate_strike": null,
                "skill_eligible": null,
                "alcohol_eligible": null,
                "time_eligible": null,
                "time_limit": null,
                "raw_damage": null,
                "victim_count": null,
                "accepted_as_best": null
            },
            {
                "invocation": 0,
                "ordinal": 1,
                "frame_ordinal": 1,
                "phase": "result",
                "actor": {"kind": "pc", "index": 342},
                "actor_creation_order": 373,
                "threat": {"kind": "soldier", "index": 140},
                "threat_creation_order": 171,
                "principal_opponent": {"kind": "soldier", "index": 137},
                "principal_opponent_creation_order": 168,
                "command": 42,
                "command_name": "parry_sword",
                "also_parade": true,
                "only_parade": true,
                "candidate_strike": null,
                "skill_eligible": null,
                "alcohol_eligible": null,
                "time_eligible": null,
                "time_limit": 8,
                "raw_damage": null,
                "victim_count": null,
                "accepted_as_best": null
            },
            {
                "invocation": 1,
                "ordinal": 2,
                "frame_ordinal": 2,
                "phase": "entry",
                "actor": {"kind": "soldier", "index": 12},
                "actor_creation_order": 44
            },
            {
                "invocation": 1,
                "ordinal": 3,
                "frame_ordinal": 3,
                "phase": "candidate",
                "actor": {"kind": "soldier", "index": 12},
                "actor_creation_order": 44,
                "threat": null,
                "threat_creation_order": null,
                "principal_opponent": {"kind": "soldier", "index": 13},
                "principal_opponent_creation_order": 45,
                "command": 30,
                "command_name": "swordstrike_thrust_a",
                "also_parade": false,
                "only_parade": false,
                "candidate_strike": 0,
                "skill_eligible": true,
                "alcohol_eligible": true,
                "time_eligible": true,
                "time_limit": 1000,
                "raw_damage": 35,
                "victim_count": 1,
                "accepted_as_best": true
            },
            {
                "invocation": 1,
                "ordinal": 4,
                "frame_ordinal": 4,
                "phase": "result",
                "actor": {"kind": "soldier", "index": 12},
                "actor_creation_order": 44,
                "command": 30,
                "command_name": "swordstrike_thrust_a",
                "selected_strike": 0,
                "reason": "strike"
            }
        ]);
        frame_json["sequence_lifecycle_events"] = serde_json::json!([
            {
                "ordinal": 0,
                "frame_ordinal": 5,
                "event": "sequence_registered",
                "phase": "manager_fifo",
                "element_id": 900,
                "sequence_id": 700,
                "owner": {"kind": "pc", "index": 342},
                "owner_creation_order": 373,
                "command": 42,
                "command_name": "parry_sword",
                "command_level": 1,
                "state": null,
                "priority": null,
                "queue_size_before": 0,
                "queue_size_after": 1,
                "actor": null,
                "actor_creation_order": null,
                "selected_sequence_id": null,
                "selected_command": null,
                "current_order_id": null,
                "current_order_action": null,
                "decision": null,
                "accepted": null
            },
            {
                "ordinal": 1,
                "frame_ordinal": 7,
                "event": "lazy_wait_created",
                "phase": "actor_hourglass_lazy_wait",
                "element_id": 901,
                "sequence_id": null,
                "owner": {"kind": "pc", "index": 342},
                "owner_creation_order": 373,
                "command": 1,
                "command_name": "wait",
                "command_level": 1,
                "state": 0,
                "priority": 1,
                "queue_size_before": null,
                "queue_size_after": null,
                "actor": {"kind": "pc", "index": 342},
                "actor_creation_order": 373,
                "selected_sequence_id": null,
                "selected_command": null,
                "current_order_id": null,
                "current_order_action": null,
                "decision": null,
                "accepted": null
            },
            {
                "ordinal": 2,
                "frame_ordinal": 8,
                "event": "manager_fifo_pop",
                "phase": "sequence_manager_hourglass",
                "element_id": 900,
                "sequence_id": 700,
                "owner": {"kind": "pc", "index": 342},
                "owner_creation_order": 373,
                "command": 42,
                "command_name": "parry_sword",
                "command_level": 1,
                "state": 0,
                "priority": 6,
                "queue_size_before": 1,
                "queue_size_after": 0,
                "actor": null,
                "actor_creation_order": null,
                "selected_sequence_id": null,
                "selected_command": null,
                "current_order_id": null,
                "current_order_action": null,
                "decision": null,
                "accepted": null
            },
            {
                "ordinal": 3,
                "frame_ordinal": 9,
                "event": "actor_instruct_result",
                "phase": "installed",
                "element_id": 900,
                "sequence_id": 700,
                "owner": {"kind": "pc", "index": 342},
                "owner_creation_order": 373,
                "command": 42,
                "command_name": "parry_sword",
                "command_level": 1,
                "state": 2,
                "priority": 6,
                "queue_size_before": null,
                "queue_size_after": null,
                "actor": {"kind": "pc", "index": 342},
                "actor_creation_order": 373,
                "selected_sequence_id": 900,
                "selected_command": 42,
                "current_order_id": 99,
                "current_order_action": 76,
                "decision": 3,
                "accepted": true
            },
            {
                "ordinal": 4,
                "frame_ordinal": 10,
                "event": "sequence_registered",
                "phase": "manager_fifo",
                "element_id": 902,
                "sequence_id": 701,
                "owner": null,
                "owner_creation_order": null,
                "command": 63,
                "command_name": "send_message",
                "command_level": 2,
                "state": 0,
                "priority": 0,
                "queue_size_before": 0,
                "queue_size_after": 1,
                "actor": null,
                "actor_creation_order": null,
                "selected_sequence_id": null,
                "selected_command": null,
                "current_order_id": null,
                "current_order_action": null,
                "decision": null,
                "accepted": null
            }
        ]);

        let frame: TraceFrame =
            serde_json::from_value(frame_json).expect("parse current-schema diagnostic streams");
        validate_trace_frame(TRACE_SCHEMA_VERSION, &frame);
        assert_eq!(frame.popup_events.len(), 1);
        assert_eq!(frame.popup_events[0].ordinal, Some(0));
        assert_eq!(
            frame.ai_forecast_events[0].phase.as_deref(),
            Some("resolved")
        );
        assert_eq!(frame.alert_formation_events[0].ordinal, Some(0));
        let authorizations = &frame.goto_authorization_events;
        assert_eq!(authorizations[0].source.as_ref().unwrap().layer, 2);
        assert!(authorizations[0].move_box.is_some());
        assert!(authorizations[1].source.is_none());
        assert!(authorizations[1].move_box.is_none());
        let target = &frame.target_lifecycle_events[0];
        assert_eq!(target.sequence_id, Some(701));
        assert_eq!(target.frame_ordinal, Some(6));
        assert_eq!(target.command_level, 2);
        assert!(target.owner.is_none());
        assert!(matches!(
            target.payload,
            TraceTargetLifecyclePayload::SendMessage {
                argument: Some(-1),
                argument_raw: Some(u32::MAX),
                ..
            }
        ));
        let proposals = &frame.strike_proposal_events;
        assert_eq!(proposals[0].actor_creation_order, 373);
        assert_eq!(proposals[0].also_parade, Some(true));
        assert_eq!(proposals[1].only_parade, Some(true));
        assert_eq!(proposals[1].command_name.as_deref(), Some("parry_sword"));
        assert_eq!(
            proposals[2].actor.as_ref().unwrap().kind,
            TraceEntityKind::Soldier
        );
        assert_eq!(proposals[2].threat, None);
        assert_eq!(proposals[3].accepted_as_best, Some(true));
        let lifecycle = &frame.sequence_lifecycle_events;
        assert_eq!(
            lifecycle
                .iter()
                .map(|event| event.ordinal)
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 3, 4]
        );
        assert_eq!(lifecycle[0].queue_size_after, Some(1));
        assert_eq!(lifecycle[0].state, None);
        assert_eq!(lifecycle[0].priority, None);
        assert_eq!(lifecycle[1].selected_sequence_id, None);
        assert_eq!(lifecycle[2].queue_size_after, Some(0));
        assert_eq!(lifecycle[3].current_order_action, Some(76));
        assert!(lifecycle[3].accepted.unwrap());
        assert_eq!(lifecycle[4].owner, None);
        assert_eq!(lifecycle[4].sequence_id, Some(701));
        assert_eq!(lifecycle[4].command_level, 2);
        assert!(
            frame.route_construction_events[0]
                .draft_diagnostics
                .contains_key("ordinal")
        );
        assert!(
            frame.route_construction_events[0].gates[0]
                .draft_diagnostics
                .contains_key("score")
        );
        let encoded = bitcode::encode(&frame);
        let cached: TraceFrame = bitcode::decode(&encoded)
            .expect("decode current-schema frame from native cache representation");
        assert_eq!(cached.alert_formation_events.len(), 1);
        assert_eq!(cached.target_lifecycle_events[0].sequence_id, Some(701));
        let cached_authorizations = &cached.goto_authorization_events;
        assert_eq!(cached_authorizations[0].source.as_ref().unwrap().layer, 2);
        assert!(cached_authorizations[0].move_box.is_some());
        assert!(cached_authorizations[1].source.is_none());
        assert!(cached_authorizations[1].move_box.is_none());
        assert_eq!(cached.strike_proposal_events.len(), 5);
        assert_eq!(cached.sequence_lifecycle_events.len(), 5);
        assert!(
            cached.route_construction_events[0]
                .draft_diagnostics
                .contains_key("result")
        );

        let mut authoritative_state_json = minimal_frame_json();
        authoritative_state_json["campaign"] = serde_json::json!({});
        assert!(serde_json::from_value::<TraceFrame>(authoritative_state_json).is_err());

        let target_with_unknown = serde_json::json!({
            "ordinal": 0,
            "frame_ordinal": 0,
            "phase": "engine_send_message_entry",
            "sequence_id": 1,
            "sequence_element_id": 2,
            "command_level": 2,
            "state": 0,
            "command": 15,
            "command_name": "send_message",
            "owner": null,
            "context": null,
            "antagonist": null,
            "payload": {
                "kind": "send_message", "message": 10,
                "argument": -1, "argument_raw": 4294967295u64,
                "extended_argument": 0, "extended_argument_raw": 0
            },
            "script_enabled": null,
            "class_instantiated": null,
            "unstable_pointer": 123
        });
        assert!(serde_json::from_value::<TraceTargetLifecycleEvent>(target_with_unknown).is_err());

        for required in [
            "route_construction_events",
            "popup_events",
            "ai_forecast_events",
            "alert_formation_events",
            "goto_authorization_events",
            "strike_proposal_events",
            "sequence_lifecycle_events",
            "target_lifecycle_events",
        ] {
            let mut incomplete = minimal_frame_json();
            incomplete.as_object_mut().unwrap().remove(required);
            assert!(
                serde_json::from_value::<TraceFrame>(incomplete).is_err(),
                "current schema accepted a frame missing {required}"
            );
        }
    }

    #[test]
    fn refused_action_decodes_with_and_without_a_target() {
        let with_target: TraceCommand = serde_json::from_value(serde_json::json!({
            "type": "hero_refused_action",
            "actor": {"kind": "pc", "index": 0},
            "action": "bow",
            "original_action": 1,
            "target": {"kind": "soldier", "index": 3},
            "reason": "anonymous_archer_contest",
        }))
        .expect("refused bow click decodes");
        assert!(matches!(
            with_target,
            TraceCommand::HeroRefusedAction {
                action: TraceAction::Bow,
                target: Some(_),
                ..
            }
        ));

        let without_target: TraceCommand = serde_json::from_value(serde_json::json!({
            "type": "hero_refused_action",
            "actor": {"kind": "pc", "index": 0},
            "action": "no_action",
            "original_action": 0,
            "reason": "locked_patch",
        }))
        .expect("refused patch click decodes");
        assert!(matches!(
            without_target,
            TraceCommand::HeroRefusedAction {
                action: TraceAction::NoAction,
                target: None,
                ..
            }
        ));
    }

    #[test]
    fn initial_save_decodes_and_matches_its_rhsg_envelope() {
        let save = valid_initial_save();
        let decoded = save
            .decode_and_validate(16_723)
            .expect("valid current-schema initial_save");
        assert_eq!(&decoded[..4], b"RHSG");
        assert_eq!(u32::from_le_bytes(decoded[4..8].try_into().unwrap()), 48);
        assert_eq!(
            u32::from_le_bytes(decoded[8..12].try_into().unwrap()),
            16_723
        );
        assert_eq!(u32::from_le_bytes(decoded[12..16].try_into().unwrap()), 48);
    }

    #[test]
    fn interactive_chain_requires_the_exact_adjacent_session_name() {
        assert_eq!(
            preceding_interactive_session_path(Path::new("chain-session-0007.jsonl.zst"), 7),
            Some(PathBuf::from("chain-session-0006.jsonl.zst"))
        );
        assert!(
            preceding_interactive_session_path(Path::new("chain-session-0001.jsonl.zst"), 1)
                .is_none()
        );
        assert_eq!(
            preceding_interactive_session_path(
                Path::new("chain-session-0007.jsonl.zst.parity.bitcode.zst"),
                7,
            ),
            Some(PathBuf::from(
                "chain-session-0006.jsonl.zst.parity.bitcode.zst"
            ))
        );
        assert!(preceding_interactive_session_path(Path::new("unrelated.jsonl.zst"), 7).is_none());
    }

    #[test]
    fn terminal_macro_identity_requires_active_cursor_and_unique_position() {
        use robin_engine::level_data::{RawHikingPath, RawWaypoint, WaypointCommand};
        let waypoint = |x| RawWaypoint {
            x,
            y: 1050,
            sector: 0,
            level: 0,
            command: WaypointCommand::Macro(vec![0; 19]),
        };
        let unique = vec![RawHikingPath {
            waypoints: vec![waypoint(353)],
        }];
        assert_eq!(
            terminal_macro_waypoint_at(
                (353_f32.to_bits(), 1050_f32.to_bits()),
                Some(18),
                true,
                &unique,
            )
            .map(|(path, waypoint, offset)| (path.get(), waypoint, offset)),
            Some((0, 0, 18))
        );
        assert!(
            terminal_macro_waypoint_at(
                (353_f32.to_bits(), 1050_f32.to_bits()),
                Some(18),
                false,
                &unique,
            )
            .is_none()
        );
        let ambiguous = vec![
            RawHikingPath {
                waypoints: vec![waypoint(353)],
            },
            RawHikingPath {
                waypoints: vec![waypoint(353)],
            },
        ];
        assert!(
            terminal_macro_waypoint_at(
                (353_f32.to_bits(), 1050_f32.to_bits()),
                Some(18),
                true,
                &ambiguous,
            )
            .is_none()
        );
    }

    #[test]
    fn windows_i386_save_preserves_and_accepts_gshr_magic() {
        let save = valid_initial_save_with_profile(TraceSaveSourceProfile::WindowsI386GshrV48);
        let decoded = save
            .decode_and_validate(16_723)
            .expect("valid Windows i386 current-schema initial_save");
        assert_eq!(&decoded[..4], b"GSHR");
    }

    #[test]
    fn source_profile_must_match_preserved_container_magic() {
        let mut save = valid_initial_save();
        save.source_profile = TraceSaveSourceProfile::WindowsI386GshrV48;
        assert!(
            save.decode_and_validate(16_723)
                .unwrap_err()
                .contains("requires magic")
        );
    }

    #[test]
    fn initial_save_rejects_decoded_length_mismatch() {
        let mut save = valid_initial_save();
        save.byte_length += 1;
        assert!(
            save.decode_and_validate(16_723)
                .unwrap_err()
                .contains("byte_length")
        );
    }

    #[test]
    fn initial_save_rejects_sha256_mismatch() {
        let mut save = valid_initial_save();
        save.sha256.replace_range(0..1, "0");
        if save.sha256
            == sha256_hex(
                &base64::engine::general_purpose::STANDARD
                    .decode(&save.data)
                    .unwrap(),
            )
        {
            save.sha256.replace_range(0..1, "1");
        }
        assert!(
            save.decode_and_validate(16_723)
                .unwrap_err()
                .contains("sha256 mismatch")
        );
    }

    #[test]
    fn initial_save_rejects_metadata_that_disagrees_with_rhsg_header() {
        let mut save = valid_initial_save();
        save.mission_id += 1;
        assert!(
            save.decode_and_validate(save.mission_id)
                .unwrap_err()
                .contains("disagrees with metadata")
        );
    }

    #[test]
    fn recorded_sim_config_restores_every_authoritative_field() {
        let config = TraceSimConfig {
            difficulty: TraceDifficulty::Hard,
            script_enabled: false,
            highlander: true,
            highlander2: true,
            golden_eye: true,
            ignore_default_loose: true,
            bypass_fog_sprites_crash: true,
            amount_of_speaking: 2,
        }
        .to_sim_config(true);

        assert_eq!(
            config.difficulty,
            robin_engine::player_profile::DifficultyLevel::Hard
        );
        assert!(!config.script_enabled);
        assert!(config.highlander);
        assert!(config.highlander2);
        assert!(config.golden_eye);
        assert!(config.ignore_default_loose);
        assert!(config.bypass_fog_sprites_crash);
        assert_eq!(config.amount_of_speaking, 2);
        assert!(config.synchronous_pathfinding);
        assert!(!config.diplomacy);
        assert!(config.npc_faction_wars);
        assert!(!config.more_combat_gestures);
        assert!(!config.gesture_quality_damage);
        assert!(!config.fog_of_war);
    }

    fn minimal_frame_json() -> serde_json::Value {
        serde_json::json!({
            "type": "frame",
            "frame_before": 0,
            "frame_after": 1,
            "game_code": 0,
            "simulation_body_ran": true,
            "commands": [],
            "director_completions": [],
            "selected_pcs": [],
            "elements": [],
            "visibility_queries": [],
            "motion_line_changes": [],
            "path_events": [],
            "route_construction_events": [],
            "popup_events": [],
            "ai_forecast_events": [],
            "alert_formation_events": [],
            "goto_authorization_events": [],
            "strike_proposal_events": [],
            "sequence_lifecycle_events": [],
            "target_lifecycle_events": [],
            "resolved_exclamations": [],
            "movement_steps": [],
            "flight_steps": [],
            "rng_draws": {
                "first_index": 0,
                "values": [],
                "callsite_offsets": [],
                "main_thread": [],
                "domains": []
            }
        })
    }

    fn minimal_element_json() -> serde_json::Value {
        serde_json::json!({
            "entity_id": {"kind": "pc", "index": 1},
            "creation_order": 1,
            "class_id": 0,
            "kind": "pc",
            "active": true,
            "blipped": false,
            "unreachable": false,
            "surface_id": 0,
            "posture": 0,
            "position_map": {"x": {"bits": 0}, "y": {"bits": 0}},
            "old_position_map": {"x": {"bits": 0}, "y": {"bits": 0}},
            "position_goal_map": {"x": {"bits": 0}, "y": {"bits": 0}},
            "elevation": {"bits": 0},
            "old_elevation": {"bits": 0},
            "increment_map": {"x": {"bits": 0}, "y": {"bits": 0}},
            "movement_map": {"x": {"bits": 0}, "y": {"bits": 0}},
            "layer": 0,
            "layer_goal": 0,
            "sector": 0,
            "direction": 0,
            "direction_goal": 0,
            "moving": false,
            "moving_map": false,
            "sprite_row": 0,
            "sprite_frame": 0,
            "sprite_frame_count": 0,
            "runtime": {}
        })
    }

    #[test]
    fn increment_map_valid_preserves_absent_and_explicit_false() {
        let absent: TraceElement = serde_json::from_value(minimal_element_json()).unwrap();
        assert_eq!(absent.increment_map_valid, None);

        let mut present = minimal_element_json();
        present["increment_map_valid"] = serde_json::json!(false);
        let present: TraceElement = serde_json::from_value(present).unwrap();
        assert_eq!(present.increment_map_valid, Some(false));
    }

    #[test]
    fn cache_round_trip_audit_normalizes_floats_and_nulls_and_reports_drops() {
        // A real minimal frame passes the audit end to end.
        let line = minimal_frame_json().to_string();
        let frame: TraceFrame = serde_json::from_str(&line).unwrap();
        verify_trace_line_roundtrip(&frame, &line, 3);

        // The two declared normalizations erase identically on both sides:
        // redundant float renderings and null object entries — while null
        // array elements and bits/value-shaped data inside retained JSON
        // payloads survive untouched.
        let mut recorded = serde_json::json!({
            "elevation": {"bits": 7, "value": 1.5},
            "actor": null,
            "list": [null, {"bits": 7, "value": 1.5, "extra": 0}]
        });
        let mut typed = serde_json::json!({
            "elevation": {"bits": 7},
            "list": [null, {"bits": 7, "value": 1.5, "extra": 0}]
        });
        normalize_trace_json_for_roundtrip(&mut recorded);
        normalize_trace_json_for_roundtrip(&mut typed);
        assert_eq!(first_json_difference("$", &recorded, &typed), None);
        assert_eq!(recorded["list"][1]["value"], serde_json::json!(1.5));

        // Differences are reported with a path into the line.
        let recorded = serde_json::json!({"a": {"b": [{"c": 1, "d": 2}]}});
        let typed = serde_json::json!({"a": {"b": [{"c": 1}]}});
        assert!(
            first_json_difference("$", &recorded, &typed)
                .unwrap()
                .contains("$.a.b[0].d is dropped")
        );

        // A field that a lenient struct silently ignores fails the audit:
        // TraceElement does not deny unknown fields, so parsing accepts the
        // stray key and only the round-trip audit reports the loss.
        let mut stray = minimal_frame_json();
        stray["elements"] = serde_json::json!([{
            "entity_id": {"kind": "pc", "index": 1},
            "creation_order": 1,
            "class_id": 0,
            "kind": "pc",
            "active": true,
            "blipped": false,
            "unreachable": false,
            "surface_id": 0,
            "posture": 0,
            "position_map": {"x": {"bits": 0}, "y": {"bits": 0}},
            "old_position_map": {"x": {"bits": 0}, "y": {"bits": 0}},
            "position_goal_map": {"x": {"bits": 0}, "y": {"bits": 0}},
            "elevation": {"bits": 0},
            "old_elevation": {"bits": 0},
            "increment_map": {"x": {"bits": 0}, "y": {"bits": 0}},
            "increment_map_valid": true,
            "movement_map": {"x": {"bits": 0}, "y": {"bits": 0}},
            "layer": 0,
            "layer_goal": 0,
            "sector": 0,
            "direction": 0,
            "direction_goal": 0,
            "moving": false,
            "moving_map": false,
            "sprite_row": 0,
            "sprite_frame": 0,
            "sprite_frame_count": 0,
            "runtime": {},
            "novel_recorder_field": 123
        }]);
        let line = stray.to_string();
        let frame: TraceFrame = serde_json::from_str(&line).unwrap();
        let mut original: serde_json::Value = serde_json::from_str(&line).unwrap();
        let mut reserialized = serde_json::to_value(frame).unwrap();
        normalize_trace_json_for_roundtrip(&mut original);
        normalize_trace_json_for_roundtrip(&mut reserialized);
        let message = first_json_difference("$", &original, &reserialized)
            .expect("the stray recorder field must be reported as dropped");
        assert!(
            message.contains("$.elements[0].novel_recorder_field is dropped"),
            "unexpected audit failure message: {message}"
        );
    }

    #[test]
    fn alert_eligibility_accepts_only_the_current_post_key() {
        #[derive(bitcode::Encode, bitcode::Decode)]
        struct FrozenTraceAlertEligibilityV66 {
            rank: bool,
            able_to_help: Option<bool>,
            allowed_to_leave_post: Option<bool>,
            can_call: Option<bool>,
            max_radius: Option<bool>,
            squared_radius: Option<bool>,
            capacity: Option<bool>,
            think: Option<bool>,
        }

        let current_json = serde_json::json!({
            "rank": true,
            // Current schema-16 recordings emit this spelling. Keep the
            // false case explicit: stay-on-post rejections are exactly where
            // the pre-fix converter used to fail its lossless round-trip
            // audit by serializing this key under the legacy spelling.
            "allowed_to_leave_post": false,
        });
        let current: TraceAlertEligibility = serde_json::from_value(current_json.clone()).unwrap();
        assert_eq!(current.allowed_to_leave_post, Some(false));

        let frozen_v66 = FrozenTraceAlertEligibilityV66 {
            rank: true,
            able_to_help: Some(false),
            allowed_to_leave_post: Some(true),
            can_call: None,
            max_radius: Some(false),
            squared_radius: Some(true),
            capacity: None,
            think: Some(true),
        };
        let frozen_v66_bytes = bitcode::encode(&frozen_v66);
        let decoded: TraceAlertEligibility = bitcode::decode(&frozen_v66_bytes).unwrap();
        assert!(decoded.rank);
        assert_eq!(decoded.able_to_help, Some(false));
        assert_eq!(decoded.allowed_to_leave_post, Some(true));
        assert_eq!(decoded.can_call, None);
        assert_eq!(decoded.max_radius, Some(false));
        assert_eq!(decoded.squared_radius, Some(true));
        assert_eq!(decoded.capacity, None);
        assert_eq!(decoded.think, Some(true));
        assert_eq!(bitcode::encode(&decoded), frozen_v66_bytes);

        let serialized = serde_json::to_value(&current).unwrap();
        assert_eq!(
            serialized["allowed_to_leave_post"],
            serde_json::json!(false)
        );
        assert!(serialized.get("stay_on_post").is_none());

        assert!(
            serde_json::from_value::<TraceAlertEligibility>(serde_json::json!({
                "rank": true,
                "stay_on_post": true,
            }))
            .is_err(),
            "obsolete schema-16 alert key was accepted"
        );
    }

    #[test]
    fn simulation_body_marker_is_mandatory() {
        let mut frame_without_marker = minimal_frame_json();
        frame_without_marker
            .as_object_mut()
            .unwrap()
            .remove("simulation_body_ran");

        let error = serde_json::from_value::<TraceFrame>(frame_without_marker)
            .expect_err("current frames must report whether the simulation body ran");
        assert!(error.to_string().contains("simulation_body_ran"));
    }

    #[test]
    fn current_path_events_are_typed() {
        let event = serde_json::json!({
            "phase": "completed",
            "actor": {"kind": "soldier", "index": 3},
            "antagonist": null,
            "layer": 2,
            "area": 17,
            "source": {"x": {"bits": 1065353216}, "y": {"bits": 1073741824}},
            "goal": {"x": {"bits": 1077936128}, "y": {"bits": 1082130432}},
            "half_diagonal_index": 1,
            "half_diagonal": {
                "x": {"bits": 1056964608},
                "y": {"bits": 1056964608}
            },
            "animation": 42,
            "reverse": false,
            "speed": 3,
            "tolerance": {"bits": 1092616192},
            "use_first_point": true,
            "valid": true,
            "waypoints": [
                {"x": {"bits": 1077936128}, "y": {"bits": 1082130432}}
            ]
        });

        let parsed: TracePathEvent =
            serde_json::from_value(event).expect("parse schema-9 completed path event");
        match parsed {
            TracePathEvent::Completed {
                actor,
                valid,
                waypoints,
                ..
            } => {
                assert_eq!(actor.index, 3);
                assert!(valid);
                assert_eq!(waypoints.len(), 1);
            }
            TracePathEvent::Queued { .. } => panic!("completed event parsed as queued"),
        }
    }

    #[test]
    fn path_events_compare_ordered_request_bits_and_cancelled_validity() {
        let expected: TracePathEvent = serde_json::from_value(serde_json::json!({
            "phase": "completed",
            "actor": {"kind": "soldier", "index": 3},
            "antagonist": null,
            "layer": 2,
            "area": 17,
            "source": {"x": {"bits": 1065353216}, "y": {"bits": 1073741824}},
            "goal": {"x": {"bits": 1077936128}, "y": {"bits": 1082130432}},
            "half_diagonal_index": 1,
            "half_diagonal": {
                "x": {"bits": 1056964608},
                "y": {"bits": 1056964608}
            },
            "animation": 42,
            "reverse": false,
            "speed": 3,
            "tolerance": {"bits": 1092616192},
            "use_first_point": true,
            "valid": false,
            "waypoints": [
                {"x": {"bits": 1077936128}, "y": {"bits": 1082130432}}
            ]
        }))
        .expect("parse completed path event");
        let original_actor = TraceEntityId {
            kind: TraceEntityKind::Soldier,
            index: 3,
        };
        let rust_actor = EntityId::Soldier(robin_engine::entity_id::SoldierId(30));
        let map = EntityMap {
            entities: BTreeMap::from([(original_actor, rust_actor)]),
            entities_by_creation_order: BTreeMap::new(),
            sectors: BTreeMap::new(),
            sector_indices: BTreeMap::new(),
            gates: Vec::new(),
            runtime_creation_order_boundary: u32::MAX,
        };
        let request = robin_engine::pathfinder::ParityPathRequest {
            actor: rust_actor,
            antagonist: None,
            layer: 2,
            area: 17,
            source: MapPoint::new(f32::from_bits(1065353216), f32::from_bits(1073741824)),
            goal: MapPoint::new(f32::from_bits(1077936128), f32::from_bits(1082130432)),
            half_diagonal_index: 1,
            half_diagonal: robin_engine::coordinates::MoveBoxHalfDiagonal::new(0.5, 0.5),
            animation: 42,
            reverse: false,
            speed: 3,
            tolerance: 10.0,
            use_first_point: true,
        };
        let actual = robin_engine::pathfinder::ParityPathEvent::Completed {
            request: request.clone(),
            valid: false,
            waypoints: vec![request.goal],
        };
        assert!(compare_path_events(&[expected.clone()], &[actual.clone()], &map).is_empty());

        let mismatched = robin_engine::pathfinder::ParityPathEvent::Completed {
            request,
            valid: true,
            waypoints: match actual {
                robin_engine::pathfinder::ParityPathEvent::Completed { waypoints, .. } => waypoints,
                _ => unreachable!(),
            },
        };
        assert!(
            compare_path_events(&[expected], &[mismatched], &map)
                .iter()
                .any(|difference| difference.contains(".valid:"))
        );
    }

    #[test]
    fn mission_start_requires_frame_zero() {
        validate_trace_start(TraceStartState::MissionStart, 7, 0);
    }

    #[test]
    fn loaded_save_is_admitted_to_strict_reconstruction() {
        validate_trace_start(TraceStartState::LoadedSave, 7, 1234);
    }

    #[test]
    fn stable_rng_domains_do_not_depend_on_callsite_offsets() {
        let batch = TraceRngBatch {
            first_index: 0,
            values: vec![1, 2],
            callsite_offsets: vec![3_305_465, 123],
            main_thread: vec![true, false],
            domains: vec![TraceRngDomain::Simulation, TraceRngDomain::Audio],
        };
        assert_eq!(batch.gameplay_draw_count(), 1);
        assert_eq!(batch.gameplay_callsite_offsets(), vec![3_305_465]);
        assert_eq!(simulation_rng_draws(&batch), vec![1]);
    }

    #[test]
    fn rng_preload_is_limited_to_reconstruction_or_diagnostic_override() {
        assert!(!should_preload_complete_rng_stream(
            TraceStartState::MissionStart,
            0,
            false
        ));
        assert!(!should_preload_complete_rng_stream(
            TraceStartState::LoadedSave,
            1,
            false
        ));
        assert!(should_preload_complete_rng_stream(
            TraceStartState::LoadedSave,
            0,
            false
        ));
        assert!(should_preload_complete_rng_stream(
            TraceStartState::MissionStart,
            1,
            true
        ));
    }

    #[test]
    #[should_panic(expected = "occurred off the main thread")]
    fn simulation_rng_draws_from_worker_threads_are_rejected() {
        TraceRngBatch {
            first_index: 41,
            values: vec![7],
            callsite_offsets: vec![123],
            main_thread: vec![false],
            domains: vec![TraceRngDomain::Simulation],
        }
        .validate();
    }

    #[test]
    fn clean_terminator_retains_completion_metadata() {
        let suffix: TraceRngOnly = serde_json::from_value(serde_json::json!({
            "type": "rng_suffix",
            "draws": {
                "first_index": 9,
                "values": [],
                "callsite_offsets": [],
                "main_thread": [],
                "domains": []
            },
            "final_frame": 112,
            "frame_count": 12
        }))
        .expect("parse clean current-schema terminator");
        assert_eq!(suffix.record_type, "rng_suffix");
        assert_eq!(suffix.final_frame, 112);
        assert_eq!(suffix.frame_count, 12);
        suffix.draws.validate();
    }

    #[test]
    fn original_commands_map_by_semantic_name() {
        assert_eq!(Action::from(TraceAction::Bow), Action::Bow);
        assert_eq!(command_from_stable_name("raise_bow"), Command::RaiseBow);
        assert_eq!(command_from_stable_name("jump"), Command::JumpCmd);
        assert_eq!(command_from_stable_name("roll"), Command::Jump);
    }

    #[test]
    fn trace_element_retains_all_recorded_authoritative_state() {
        let element: TraceElement = serde_json::from_value(serde_json::json!({
            "entity_id": {"kind": "soldier", "index": 58},
            "creation_order": 89,
            "class_id": 1,
            "kind": "soldier",
            "active": true,
            "blipped": false,
            "unreachable": false,
            "surface_id": 1226,
            "posture": 1,
            "position_map": {"x": {"bits": 0}, "y": {"bits": 0}},
            "old_position_map": {"x": {"bits": 0}, "y": {"bits": 0}},
            "position_goal_map": {"x": {"bits": 0}, "y": {"bits": 0}},
            "elevation": {"bits": 0},
            "old_elevation": {"bits": 0},
            "increment_map": {"x": {"bits": 0}, "y": {"bits": 0}},
            "increment_map_valid": true,
            "movement_map": {"x": {"bits": 0}, "y": {"bits": 0}},
            "layer": 0,
            "layer_goal": 0,
            "sector": 0,
            "direction": 0,
            "direction_goal": 0,
            "moving": false,
            "moving_map": false,
            "sprite_row": 64,
            "sprite_frame": 3,
            "sprite_frame_count": 65535,
            "actor": {
                "action_state": 0,
                "animation": 254,
                "command": 117,
                "command_name": "whistle",
                "motion_state": 2,
                "wait_time": 25,
                "passing_door_directly": false,
                "active_pass_door": null,
                "sequence_element": null,
                "position_interface": {}
            },
            "human": {
                "life_points": 60,
                "dead": false,
                "unconscious": false,
                "camp": "lacklandists",
                "original_camp": 1,
                "vip": true,
                "civilian": false,
                "opponents": [{"kind": "pc", "index": 2}],
                "opponent_jump_lines": [null]
            },
            "ai": {
                "state": 3,
                "substate": 17,
                "script_locked": false,
                "locked": true,
                "locks": 1,
                "was_busy": true,
                "very_busy": false,
                "macro_timer_running": true,
                "macro_timer_ring": 987,
                "macro_cursor": 4,
                "macro_remaining": 2,
                "macro_in_progress": true,
                "list_us": [{"kind": "soldier", "index": 58}],
                "list_them": [{"kind": "pc", "index": 2}],
                "my_line_jump": null
            },
            "detection": {
                "suspects": [1, 2, 3, 4, 5, 6],
                "maximal_suspect": 6,
                "maximal_visibility": 200,
                "view_status": 1,
                "alert_status": 2,
                "detectables": [{
                    "type": 0,
                    "target": {"kind": "pc", "index": 2},
                    "seen_now": true,
                    "seen_last_frame": false,
                    "heard_last_frame": true,
                    "shadow_seen_now": false,
                    "shadow_seen_last_frame": true,
                    "last_visibility": {"bits": 1120403456}
                }]
            },
            "runtime": {}
        }))
        .expect("parse authoritative recorded element state");

        // Raw class/surface identifiers remain available to dumps even though
        // logical parity compares the concrete kind instead. In particular,
        // Original surface values are renderer allocation handles.
        assert_eq!(element.class_id, 1);
        assert_eq!(element.surface_id, 1226);
        assert_eq!(element.layer_goal, 0);
        assert_eq!(element.sprite_row, 64);
        assert_eq!(element.sprite_frame, 3);
        assert_eq!(element.sprite_frame_count, u16::MAX);
        let human = element.human.expect("human state");
        assert_eq!(human.camp, "lacklandists");
        assert!(human.vip);
        let ai = element.ai.expect("AI state");
        assert_eq!(ai.macro_cursor, Some(4));
        assert_eq!(
            ai.list_them,
            [TraceEntityId {
                kind: TraceEntityKind::Pc,
                index: 2,
            }]
        );
        let detection = element.detection.expect("detection state");
        assert_eq!(detection.suspects, [1, 2, 3, 4, 5, 6]);
        assert_eq!(detection.detectables[0].last_visibility.value(), 100.0);
    }

    #[test]
    fn only_replay_constructed_bonuses_omit_original_undefined_old_position() {
        let boundary = 143;

        assert!(original_runtime_bonus_has_undefined_old_position(
            TraceEntityKind::Bonus,
            326,
            boundary,
        ));
        assert!(!original_runtime_bonus_has_undefined_old_position(
            TraceEntityKind::Bonus,
            142,
            boundary,
        ));
        assert!(!original_runtime_bonus_has_undefined_old_position(
            TraceEntityKind::Projectile,
            326,
            boundary,
        ));
    }

    #[test]
    fn current_ai_core_and_human_snapshots_reject_missing_fields() {
        let complete_ai = serde_json::json!({
            "state": 3,
            "substate": 17,
            "script_locked": false,
            "locked": true,
            "locks": 0,
            "was_busy": false,
            "very_busy": false,
            "macro_timer_running": false,
            "macro_timer_ring": 0,
            "macro_cursor": null,
            "macro_remaining": 0,
            "macro_in_progress": false,
            "list_us": [],
            "list_them": [],
            "my_line_jump": null
        });
        for required in ["state", "substate"] {
            let mut incomplete = complete_ai.clone();
            incomplete.as_object_mut().unwrap().remove(required);
            assert!(
                serde_json::from_value::<TraceAi>(incomplete).is_err(),
                "current AI snapshot accepted missing {required}"
            );
        }
        let additive_defaults: TraceAi = serde_json::from_value(serde_json::json!({
            "state": 3,
            "substate": 17
        }))
        .expect("parse AI snapshot without later additive diagnostics");
        assert!(!additive_defaults.script_locked);
        assert!(!additive_defaults.locked);
        assert_eq!(additive_defaults.locks, 0);
        assert_eq!(additive_defaults.macro_cursor, None);
        assert!(additive_defaults.list_us.is_empty());
        assert!(additive_defaults.list_them.is_empty());
        assert!(additive_defaults.my_line_jump.is_none());

        let complete_human = serde_json::json!({
            "life_points": 60,
            "dead": false,
            "unconscious": false,
            "camp": "royalists",
            "original_camp": 0,
            "vip": false,
            "civilian": false,
            "opponents": [],
            "opponent_jump_lines": []
        });
        for required in ["opponents", "opponent_jump_lines"] {
            let mut incomplete = complete_human.clone();
            incomplete.as_object_mut().unwrap().remove(required);
            assert!(
                serde_json::from_value::<TraceHuman>(incomplete).is_err(),
                "current human snapshot accepted missing {required}"
            );
        }
    }

    #[test]
    fn runtime_snapshot_canonicalizes_original_zero_based_order_ids() {
        let mut expected = serde_json::json!({
            "sprite": {
                "last_processed_order_id": 41,
                "unrelated_id": 41
            },
            "nested": [{"last_processed_order_id": 0}],
            "sentinel": {"last_processed_order_id": u32::MAX}
        });

        canonicalize_original_runtime_representation(&mut expected);

        assert_eq!(expected["sprite"]["last_processed_order_id"], 42);
        assert_eq!(expected["sprite"]["unrelated_id"], 41);
        assert_eq!(expected["nested"][0]["last_processed_order_id"], 1);
        assert_eq!(expected["sentinel"]["last_processed_order_id"], u32::MAX);
    }

    #[test]
    fn runtime_projectile_projects_only_indeterminate_constructor_storage() {
        let mut expected = serde_json::json!({
            "position": {
                "computed_position": 7,
                "computed_increment": 2,
                "posture": 1,
                "old_posture": 1,
                "increment": {"x": 1, "y": 2, "z": 3},
                "door_direction": true,
                "goal_world": {"x": 99, "y": 98, "z": 97},
                "radius": {"bits": 97},
                "material": 1_521_537_396_u32,
                "move_box": {
                    "min": {"x": 10, "y": 20},
                    "max": {"x": 10, "y": 20}
                }
            },
            "sprite": {
                "row": 0,
                "flight_countdown": 9660,
                "behind_display_order_reference": true,
                "display_order_reference": null,
                "last_processed_order_id": 65536
            }
        });

        project_runtime_projectile_constructor_storage(&mut expected);

        let position = expected["position"].as_object().unwrap();
        for omitted in ["door_direction", "goal_world", "radius", "material"] {
            assert!(!position.contains_key(omitted));
        }
        assert!(position["move_box"].is_null());
        assert_eq!(position["computed_position"], 7);
        assert_eq!(position["computed_increment"], 2);
        assert_eq!(
            position["increment"],
            serde_json::json!({"x": 1, "y": 2, "z": 3})
        );
        assert_eq!(position["posture"], 1);
        assert_eq!(position["old_posture"], 1);

        let sprite = expected["sprite"].as_object().unwrap();
        assert!(!sprite.contains_key("flight_countdown"));
        assert!(!sprite.contains_key("behind_display_order_reference"));
        assert_eq!(sprite["row"], 0);
        assert_eq!(sprite["last_processed_order_id"], 65536);
    }

    #[test]
    fn runtime_projectile_material_projects_both_signed_garbage_domains() {
        for material in [serde_json::json!(-143_634_448), serde_json::json!(11)] {
            let mut expected = serde_json::json!({"position": {"material": material}});
            project_runtime_projectile_constructor_storage(&mut expected);
            assert!(expected["position"].get("material").is_none());
        }

        for material in 0..=10 {
            let mut expected = serde_json::json!({"position": {"material": material}});
            project_runtime_projectile_constructor_storage(&mut expected);
            assert_eq!(expected["position"]["material"], material);
        }

        // Malformed trace data is not constructor residue. Preserve it so
        // the strict subset comparator reports the schema/type violation.
        for material in [serde_json::Value::Null, serde_json::json!("invalid")] {
            let mut expected = serde_json::json!({"position": {"material": material.clone()}});
            project_runtime_projectile_constructor_storage(&mut expected);
            assert_eq!(expected["position"]["material"], material);
        }
    }

    #[test]
    fn runtime_snapshot_canonicalizes_only_zero_original_blocked_boxes() {
        let float = |bits| serde_json::json!({"bits": bits, "value": 0.0});
        let zero_box = serde_json::json!({
            "min": {"x": float(0), "y": float(0)},
            "max": {"x": float(0), "y": float(0)}
        });
        let mut expected = serde_json::json!({
            "position": {"blocked_box": zero_box},
            "active_position": {"blocked_box": {
                "min": {"x": float(0), "y": float(0)},
                "max": {"x": float(0x3f80_0000), "y": float(0)}
            }},
            "unrelated_box": zero_box
        });

        canonicalize_original_runtime_representation(&mut expected);

        assert!(expected["position"]["blocked_box"].is_null());
        assert!(expected["active_position"]["blocked_box"].is_object());
        assert!(expected["unrelated_box"].is_object());
    }

    #[test]
    fn runtime_snapshot_compatibility_projection_matches_rust_representation() {
        let mut expected = serde_json::json!({
            "position": {"blocked_box": {
                "min": {
                    "x": {"bits": 0, "value": 0.0},
                    "y": {"bits": 0, "value": 0.0}
                },
                "max": {
                    "x": {"bits": 0, "value": 0.0},
                    "y": {"bits": 0, "value": 0.0}
                }
            }},
            "sprite": {"last_processed_order_id": 41}
        });
        let actual = serde_json::json!({
            "position": {"blocked_box": null},
            "sprite": {"last_processed_order_id": 42}
        });

        canonicalize_original_runtime_representation(&mut expected);
        let mut differences = Vec::new();
        collect_json_subset_differences("runtime", &expected, &actual, &mut differences);

        assert!(differences.is_empty(), "{differences:#?}");
    }

    #[test]
    fn missing_draw_view_projects_only_sprite_presentation_cache() {
        let mut expected = serde_json::json!({
            "sprite": {
                "width": 20,
                "height": 53,
                "masked": true,
                "current_row": 9,
                "current_frame": 3,
                "frame_count": 4,
                "use_alternate_profile": true
            },
            "position": {"layer": 2},
            "width": "unrelated gameplay field"
        });
        let mut actual = serde_json::json!({
            "sprite": {
                "width": 24,
                "height": 55,
                "masked": false,
                "current_row": 9,
                "current_frame": 3,
                "frame_count": 4,
                "use_alternate_profile": true
            },
            "position": {"layer": 2},
            "width": "unrelated gameplay field"
        });

        project_missing_draw_view_sprite_cache(&mut expected);
        let mut differences = Vec::new();
        collect_json_subset_differences("runtime", &expected, &actual, &mut differences);
        assert!(differences.is_empty(), "{differences:#?}");

        actual["sprite"]["current_frame"] = serde_json::json!(4);
        actual["position"]["layer"] = serde_json::json!(3);
        actual["width"] = serde_json::json!("changed");
        collect_json_subset_differences("runtime", &expected, &actual, &mut differences);
        assert_eq!(differences.len(), 3, "{differences:#?}");
        assert!(
            differences
                .iter()
                .any(|difference| difference.contains("sprite.current_frame"))
        );
        assert!(
            differences
                .iter()
                .any(|difference| difference.contains("position.layer"))
        );
        assert!(
            differences
                .iter()
                .any(|difference| difference.contains("runtime.width"))
        );
    }

    #[test]
    fn runtime_snapshot_comparison_requires_recorded_subset_and_float_bits() {
        let expected = serde_json::json!({
            "position": {
                "world": {"bits": 1, "value": 1.401298464324817e-45}
            }
        });
        let actual = serde_json::json!({
            "position": {
                "world": {"bits": 1, "value": 1.4012984643248171e-45}
            },
            "rust_only_diagnostic": true
        });
        let mut differences = Vec::new();
        collect_json_subset_differences("runtime", &expected, &actual, &mut differences);
        assert!(differences.is_empty());

        let wrong_bits = serde_json::json!({
            "position": {
                "world": {"bits": 2, "value": 1.401298464324817e-45}
            }
        });
        collect_json_subset_differences("runtime", &expected, &wrong_bits, &mut differences);
        assert_eq!(differences.len(), 1);
        assert!(differences[0].contains("runtime.position.world.bits"));
    }

    #[test]
    fn visibility_query_retains_authoritative_call_and_diagnostics() {
        let query: TraceVisibilityQuery = serde_json::from_value(serde_json::json!({
            "origin": {"x": {"bits": 1}, "y": {"bits": 2}, "z": {"bits": 3}},
            "destination": {"x": {"bits": 4}, "y": {"bits": 5}, "z": {"bits": 6}},
            "result": false,
            "cache_hit": false,
            "cache_key": 123456,
            "cache_offset": 1456,
            "candidate_count": 1,
            "reason": "wall",
            "blocking_obstacle": {
                "id": 8,
                "index": 7,
                "type_mask": 3,
                "types": {
                    "solid": true,
                    "opaque": true,
                    "projection_area": false,
                    "mouse": false,
                    "shield": false,
                    "show_shadow_polygon": false
                },
                "active": true,
                "on_ground": true,
                "layer": 65535,
                "sector": 65535,
                "box_ground": {
                    "min": {"x": {"bits": 0}, "y": {"bits": 0}},
                    "max": {"x": {"bits": 1065353216}, "y": {"bits": 1065353216}}
                },
                "points": [{
                    "x": {"bits": 0},
                    "y": {"bits": 0},
                    "z_top": {"bits": 1065353216},
                    "z_bottom": {"bits": 0}
                }]
            }
        }))
        .expect("parse complete visibility query");

        assert_eq!(query.origin.x.bits, 1);
        assert_eq!(query.reason, "wall");
        let actual = robin_engine::sight_obstacle::ParityVisibilityQuery {
            origin: [f32::from_bits(1), f32::from_bits(2), f32::from_bits(3)],
            destination: [f32::from_bits(4), f32::from_bits(5), f32::from_bits(6)],
            result: false,
            caller_file: file!(),
            caller_line: line!(),
        };
        assert!(compare_visibility_queries(std::slice::from_ref(&query), &[actual]).is_empty());
        let mismatched = robin_engine::sight_obstacle::ParityVisibilityQuery {
            result: true,
            ..actual
        };
        assert!(
            compare_visibility_queries(std::slice::from_ref(&query), &[mismatched])
                .iter()
                .any(|difference| difference.contains(".result:"))
        );
        let obstacle = query.blocking_obstacle.expect("blocking obstacle");
        assert_eq!(obstacle.index, 7);
        assert!(obstacle.types.opaque);
        assert_eq!(obstacle.points.len(), 1);
    }

    #[test]
    fn global_action_cancel_accepts_the_original_no_pc_shape() {
        let command: TraceCommand = serde_json::from_value(serde_json::json!({
            "type": "cancel_action",
            "action": "no_action",
            "original_action": 0
        }))
        .expect("parse Original global action cancellation");
        assert!(matches!(
            command,
            TraceCommand::CancelAction { pc: None, .. }
        ));
    }

    /// Every resolved-command type the recorder can emit must decode.  A
    /// type the runner does not know aborts the whole trace before any
    /// simulation comparison happens, so the schema has to stay complete
    /// rather than merely covering whatever the current corpus contains.
    #[test]
    fn every_recorded_command_type_decodes() {
        let pc = serde_json::json!({"kind": "pc", "index": 3});
        let point2 = serde_json::json!({
            "x": {"bits": 1065353216, "value": 1.0},
            "y": {"bits": 1073741824, "value": 2.0}
        });
        let point3 = serde_json::json!({
            "x": {"bits": 1065353216, "value": 1.0},
            "y": {"bits": 1073741824, "value": 2.0},
            "z": {"bits": 1077936128, "value": 3.0}
        });
        let recorded = [
            serde_json::json!({"type": "box_select", "first": point2, "second": point2, "append": false}),
            serde_json::json!({"type": "box_unselect", "first": point2, "second": point2, "append": false}),
            serde_json::json!({"type": "group_move", "actors": [pc], "destination": point2,
                "running": true, "show_marker": true, "goal_sector": 4, "goal_layer": 0}),
            serde_json::json!({"type": "launch_interaction", "actor": pc, "target": pc,
                "original_command": 0, "original_command_name": "hit", "running": false}),
            serde_json::json!({"type": "launch_scroll_read", "actor": pc, "target": pc, "running": false}),
            serde_json::json!({"type": "sword_strike", "actor": pc, "target": pc,
                "original_command": 0, "original_command_name": "hit", "with_seek": true,
                "seek_distance": 63.0}),
            serde_json::json!({"type": "launch_self_ability", "actor": pc,
                "original_command": 0, "original_command_name": "eat"}),
            serde_json::json!({"type": "launch_ground_target", "actor": pc, "target": point3,
                "original_command": 0, "original_command_name": "throw_purse",
                "original_target_field": 30, "titbit_layer": 0}),
            serde_json::json!({"type": "drop_ale_at", "actor": pc, "target": point2, "running": false}),
            serde_json::json!({"type": "shield_select_protected", "actor": pc, "protected_pc": pc}),
            serde_json::json!({"type": "raise_shield_with_danger", "actor": pc, "protected_pc": pc,
                "danger_point": point3, "danger_point_layer": 0}),
            serde_json::json!({"type": "teleport_selected", "destination": point2,
                "goal_sector": -1, "goal_layer": 0}),
            serde_json::json!({"type": "stop_pc", "pc": pc}),
            serde_json::json!({"type": "select_pc", "pc": pc, "append": false}),
            serde_json::json!({"type": "select_all_pcs"}),
            serde_json::json!({"type": "unselect_pc", "pc": pc}),
            serde_json::json!({"type": "unselect_all_pcs"}),
            serde_json::json!({"type": "select_action_index", "index": 1}),
            serde_json::json!({"type": "select_action", "action": "bow", "original_action": 1, "pc": pc}),
            serde_json::json!({"type": "cancel_action", "action": "no_action", "original_action": 0}),
            serde_json::json!({"type": "crouch_down"}),
            serde_json::json!({"type": "stand_up"}),
            serde_json::json!({"type": "start_macro", "slot": 1, "pc": pc}),
            serde_json::json!({"type": "delete_macro", "slot": 1}),
            serde_json::json!({"type": "start_recording_macro", "slot": 2, "pc": pc}),
            serde_json::json!({"type": "change_qa_memory", "slot": 0}),
            serde_json::json!({"type": "set_lock_alt", "on": true}),
            serde_json::json!({"type": "key_control"}),
            serde_json::json!({"type": "key_release_control"}),
            serde_json::json!({"type": "make_pc_fast", "entity": pc}),
            serde_json::json!({"type": "beggar_dont_talk_stamp", "entity": pc}),
            serde_json::json!({"type": "orient_action_at", "action": "bow", "original_action": 1,
                "actor": pc, "mouse_map": point2, "target": point3}),
        ];
        for value in recorded {
            let recorded_type = value["type"].clone();
            serde_json::from_value::<TraceCommand>(value.clone())
                .unwrap_or_else(|err| panic!("decode recorded command {recorded_type}: {err}"));
        }
    }

    #[test]
    fn native_bitcode_trace_handles_heterogeneous_command_variants() {
        let commands = [
            TraceCommand::CrouchDown,
            TraceCommand::LaunchGroundTarget {
                actor: TraceEntityId {
                    kind: TraceEntityKind::Pc,
                    index: 126,
                },
                target: TracePoint3 {
                    x: TraceFloat {
                        bits: 834.0_f32.to_bits(),
                    },
                    y: TraceFloat {
                        bits: 765.0_f32.to_bits(),
                    },
                    z: TraceFloat {
                        bits: 0.0_f32.to_bits(),
                    },
                },
                original_command: 86,
                original_command_name: "throw_purse".to_owned(),
                original_target_field: 30,
                titbit_layer: 0,
            },
        ];
        let mut encoded = Vec::new();
        for command in &commands {
            write_binary_record(&mut encoded, command, "test command");
        }

        let mut reader = std::io::Cursor::new(encoded);
        assert!(matches!(
            read_binary_record(&mut reader, "test command").unwrap(),
            TraceCommand::CrouchDown
        ));
        assert!(matches!(
            read_binary_record(&mut reader, "test command").unwrap(),
            TraceCommand::LaunchGroundTarget {
                original_target_field: 30,
                ..
            }
        ));
    }

    #[test]
    fn resolved_orientation_is_bit_exact() {
        let command: TraceCommand = serde_json::from_value(serde_json::json!({
            "type": "orient_action_at",
            "action": "bow",
            "original_action": 1,
            "actor": {"kind": "pc", "index": 198},
            "mouse_map": {
                "x": {"bits": 1065353216, "value": 1.0},
                "y": {"bits": 1073741824, "value": 2.0}
            },
            "target": {
                "x": {"bits": 1077936128, "value": 3.0},
                "y": {"bits": 1082130432, "value": 4.0},
                "z": {"bits": 1084227584, "value": 5.0}
            }
        }))
        .unwrap();
        let TraceCommand::OrientActionAt {
            action,
            mouse_map,
            target,
            ..
        } = command
        else {
            panic!("wrong trace command variant");
        };
        assert!(matches!(action, TraceAction::Bow));
        assert_eq!(MapPoint::from(mouse_map), MapPoint::new(1.0, 2.0));
        assert_eq!(WorldPoint3D::from(target), WorldPoint3D::new(3.0, 4.0, 5.0));
    }

    #[test]
    fn matching_action_selection_marks_only_following_orientation_as_late_refresh() {
        let pc = TraceEntityId {
            kind: TraceEntityKind::Pc,
            index: 282,
        };
        let point = TracePoint {
            x: TraceFloat { bits: 0 },
            y: TraceFloat { bits: 0 },
        };
        let target = TracePoint3 {
            x: TraceFloat { bits: 0 },
            y: TraceFloat { bits: 0 },
            z: TraceFloat { bits: 0 },
        };
        let orientation = || TraceCommand::OrientActionAt {
            action: TraceAction::Purse,
            original_action: 4,
            actor: pc,
            mouse_map: point,
            target,
        };
        let commands = vec![
            orientation(),
            TraceCommand::SelectAction {
                pc,
                action: TraceAction::Purse,
                original_action: 4,
            },
            orientation(),
        ];

        let (before, after) = split_refresh_owned_orientations(commands, false, &[], false);

        assert_eq!(before.len(), 2);
        assert!(matches!(before[0], TraceCommand::OrientActionAt { .. }));
        assert!(matches!(before[1], TraceCommand::SelectAction { .. }));
        assert_eq!(after.len(), 1);
        assert!(matches!(after[0], TraceCommand::OrientActionAt { .. }));
    }

    #[test]
    fn popup_nested_refresh_marks_only_final_purse_orientation_as_late() {
        let pc = TraceEntityId {
            kind: TraceEntityKind::Pc,
            index: 296,
        };
        let point = TracePoint {
            x: TraceFloat { bits: 0 },
            y: TraceFloat { bits: 0 },
        };
        let target = TracePoint3 {
            x: TraceFloat { bits: 0 },
            y: TraceFloat { bits: 0 },
            z: TraceFloat { bits: 0 },
        };
        let orientation = || TraceCommand::OrientActionAt {
            action: TraceAction::Purse,
            original_action: 4,
            actor: pc,
            mouse_map: point,
            target,
        };

        // The ordinary refresh record remains before Hourglass. The final
        // duplicate was emitted by DisplayPopupText's nested refresh.
        let (before, after) = split_refresh_owned_orientations(
            vec![orientation(), orientation()],
            true,
            &[(pc, TraceAction::Purse)],
            false,
        );

        assert_eq!(before.len(), 1);
        assert!(matches!(before[0], TraceCommand::OrientActionAt { .. }));
        assert_eq!(after.len(), 1);
        assert!(matches!(
            after[0],
            TraceCommand::OrientActionAt {
                action: TraceAction::Purse,
                ..
            }
        ));
    }

    #[test]
    fn popup_frame_keeps_a_single_throw_orientation_before_hourglass() {
        let command = TraceCommand::OrientActionAt {
            action: TraceAction::Purse,
            original_action: 4,
            actor: TraceEntityId {
                kind: TraceEntityKind::Pc,
                index: 296,
            },
            mouse_map: TracePoint {
                x: TraceFloat { bits: 0 },
                y: TraceFloat { bits: 0 },
            },
            target: TracePoint3 {
                x: TraceFloat { bits: 0 },
                y: TraceFloat { bits: 0 },
                z: TraceFloat { bits: 0 },
            },
        };

        let (before, after) = split_refresh_owned_orientations(
            vec![command],
            true,
            &[(
                TraceEntityId {
                    kind: TraceEntityKind::Pc,
                    index: 296,
                },
                TraceAction::Purse,
            )],
            false,
        );

        assert_eq!(before.len(), 1);
        assert!(after.is_empty());
    }

    #[test]
    fn legacy_random_popup_keeps_first_post_selection_orientation_early() {
        let pc = TraceEntityId {
            kind: TraceEntityKind::Pc,
            index: 296,
        };
        let selection = vec![
            TraceCommand::SelectPc { pc, append: false },
            TraceCommand::SelectAction {
                pc,
                action: TraceAction::Purse,
                original_action: 4,
            },
        ];
        let orientation = TraceCommand::OrientActionAt {
            action: TraceAction::Purse,
            original_action: 4,
            actor: pc,
            mouse_map: TracePoint {
                x: TraceFloat { bits: 0x4501_0000 },
                y: TraceFloat { bits: 0x4302_0000 },
            },
            target: TracePoint3 {
                x: TraceFloat { bits: 0x4501_0000 },
                y: TraceFloat { bits: 0x4302_0000 },
                z: TraceFloat { bits: 0 },
            },
        };
        let provenance = LegacyRefreshOrientationProvenance::default().advance(&selection, false);

        // Save055 replay-006 reaches its popup on the first boundary after
        // selecting Purse. Its singleton is the preceding ordinary refresh.
        assert!(
            !provenance
                .proves_single_popup_orientation_is_late(std::slice::from_ref(&orientation), true,)
        );
        let (before, after) = split_refresh_owned_orientations(
            vec![orientation],
            true,
            &[(pc, TraceAction::Purse)],
            false,
        );
        assert_eq!(before.len(), 1);
        assert!(after.is_empty());
    }

    #[test]
    fn legacy_random_popup_marks_exact_repeated_orientation_late() {
        let pc = TraceEntityId {
            kind: TraceEntityKind::Pc,
            index: 296,
        };
        let selection = vec![
            TraceCommand::SelectPc { pc, append: false },
            TraceCommand::SelectAction {
                pc,
                action: TraceAction::Purse,
                original_action: 4,
            },
        ];
        let orientation = || TraceCommand::OrientActionAt {
            action: TraceAction::Purse,
            original_action: 4,
            actor: pc,
            mouse_map: TracePoint {
                x: TraceFloat { bits: 1158882171 },
                y: TraceFloat { bits: 1126828606 },
            },
            target: TracePoint3 {
                x: TraceFloat { bits: 1158882171 },
                y: TraceFloat { bits: 1126828606 },
                z: TraceFloat { bits: 0 },
            },
        };
        let selected = LegacyRefreshOrientationProvenance::default().advance(&selection, false);
        let first_orientation = orientation();
        let provenance = selected.advance(std::slice::from_ref(&first_orientation), false);
        let popup_orientation = orientation();
        let force_late = provenance.proves_single_popup_orientation_is_late(
            std::slice::from_ref(&popup_orientation),
            true,
        );

        // Save055 replay-033 first orients in the intervening ordinary
        // refresh. Its identical popup singleton is therefore nested-late.
        assert!(force_late);
        let (before, after) = split_refresh_owned_orientations(
            vec![popup_orientation],
            true,
            &[(pc, TraceAction::Purse)],
            force_late,
        );
        assert!(before.is_empty());
        assert_eq!(after.len(), 1);
    }

    #[test]
    fn legacy_random_popup_duplicate_retains_ordinary_then_nested_order() {
        let pc = TraceEntityId {
            kind: TraceEntityKind::Pc,
            index: 296,
        };
        let orientation = || TraceCommand::OrientActionAt {
            action: TraceAction::Purse,
            original_action: 4,
            actor: pc,
            mouse_map: TracePoint {
                x: TraceFloat { bits: 1 },
                y: TraceFloat { bits: 2 },
            },
            target: TracePoint3 {
                x: TraceFloat { bits: 1 },
                y: TraceFloat { bits: 2 },
                z: TraceFloat { bits: 0 },
            },
        };
        let previous_orientation = orientation();
        let provenance = LegacyRefreshOrientationProvenance::FirstOrdinaryOrientation(
            RefreshOrientationSignature::from_command(&previous_orientation).unwrap(),
        );
        let popup_commands = vec![orientation(), orientation()];

        // Save055 replay-028 records both the preceding ordinary refresh and
        // the popup's nested refresh. Singleton provenance must not claim it;
        // the established duplicate rule splits only the final orientation.
        assert!(!provenance.proves_single_popup_orientation_is_late(&popup_commands, true));
        let (before, after) = split_refresh_owned_orientations(
            popup_commands,
            true,
            &[(pc, TraceAction::Purse)],
            false,
        );
        assert_eq!(before.len(), 1);
        assert_eq!(after.len(), 1);
        assert!(matches!(before[0], TraceCommand::OrientActionAt { .. }));
        assert!(matches!(after[0], TraceCommand::OrientActionAt { .. }));
    }

    #[test]
    fn legacy_random_popup_proof_rejects_intervening_command_or_changed_target() {
        let pc = TraceEntityId {
            kind: TraceEntityKind::Pc,
            index: 296,
        };
        let selected = LegacyRefreshOrientationProvenance::ActionSelected {
            actor: pc,
            action: TraceAction::Purse,
        };
        let orientation = |x_bits| TraceCommand::OrientActionAt {
            action: TraceAction::Purse,
            original_action: 4,
            actor: pc,
            mouse_map: TracePoint {
                x: TraceFloat { bits: x_bits },
                y: TraceFloat { bits: 2 },
            },
            target: TracePoint3 {
                x: TraceFloat { bits: x_bits },
                y: TraceFloat { bits: 2 },
                z: TraceFloat { bits: 0 },
            },
        };
        let interrupted = selected.advance(
            &[orientation(1), TraceCommand::MakePcFast { entity: pc }],
            false,
        );
        assert_eq!(interrupted, LegacyRefreshOrientationProvenance::None);

        let first_orientation = orientation(1);
        let provenance = selected.advance(std::slice::from_ref(&first_orientation), false);
        let changed = orientation(3);
        assert!(
            !provenance
                .proves_single_popup_orientation_is_late(std::slice::from_ref(&changed), true,)
        );
    }

    #[test]
    fn popup_nested_refresh_marks_single_bow_orientation_as_late() {
        let pc = TraceEntityId {
            kind: TraceEntityKind::Pc,
            index: 342,
        };
        let command = TraceCommand::OrientActionAt {
            action: TraceAction::Bow,
            original_action: 1,
            actor: pc,
            mouse_map: TracePoint {
                x: TraceFloat { bits: 0 },
                y: TraceFloat { bits: 0 },
            },
            target: TracePoint3 {
                x: TraceFloat { bits: 0 },
                y: TraceFloat { bits: 0 },
                z: TraceFloat { bits: 0 },
            },
        };

        let (before, after) = split_refresh_owned_orientations(vec![command], true, &[], false);

        assert!(before.is_empty());
        assert_eq!(after.len(), 1);
        assert!(matches!(
            after[0],
            TraceCommand::OrientActionAt {
                action: TraceAction::Bow,
                ..
            }
        ));
    }

    #[test]
    fn trace_index_refresh_uses_stable_creation_order() {
        let old_trace_id = TraceEntityId {
            kind: TraceEntityKind::Projectile,
            index: 127,
        };
        let shifted_trace_id = TraceEntityId {
            kind: TraceEntityKind::Projectile,
            index: 126,
        };
        let rust_id = EntityId::Projectile(robin_engine::entity_id::ProjectileId(158));
        let mut map = EntityMap {
            entities: BTreeMap::from([(old_trace_id, rust_id)]),
            entities_by_creation_order: BTreeMap::from([(158, rust_id)]),
            sectors: BTreeMap::new(),
            sector_indices: BTreeMap::new(),
            gates: Vec::new(),
            runtime_creation_order_boundary: u32::MAX,
        };

        map.refresh_trace_index(shifted_trace_id, 158);

        assert_eq!(map.translate(shifted_trace_id), rust_id);
    }

    #[test]
    fn group_move_translates_retained_sector_identity_without_click_containment() {
        let map = EntityMap {
            entities: BTreeMap::new(),
            entities_by_creation_order: BTreeMap::new(),
            sectors: BTreeMap::from([(55, 23), (56, 23)]),
            sector_indices: BTreeMap::from([
                (
                    55,
                    robin_engine::fast_find_grid::SectorIndex::new(7).unwrap(),
                ),
                (
                    56,
                    robin_engine::fast_find_grid::SectorIndex::new(8).unwrap(),
                ),
            ]),
            gates: Vec::new(),
            runtime_creation_order_boundary: 0,
        };

        // Patch moves record the patch's underlying position sector while the
        // recorded waypoint may lie outside that sector's polygon. Translation
        // therefore depends only on retained construction topology.
        assert_eq!(
            map.translate_group_move_goal_sector(55, 0, None),
            GroupMoveGoalTranslation::Runtime(
                (SectorNumber::new(23), 0),
                robin_engine::fast_find_grid::SectorIndex::new(7).unwrap(),
            )
        );
        assert_eq!(
            map.translate_group_move_goal_sector(56, 0, None),
            GroupMoveGoalTranslation::Runtime(
                (SectorNumber::new(23), 0),
                robin_engine::fast_find_grid::SectorIndex::new(8).unwrap(),
            ),
            "two Original sparse slots may share a public identity while retaining distinct arena identities"
        );
        assert_eq!(
            map.translate_group_move_goal_sector(288, 4, None),
            GroupMoveGoalTranslation::RecordedUnmapped((SectorNumber::new(288), 4)),
            "a coincident overlay must not erase the recorded route goal"
        );
    }

    #[test]
    fn recorded_group_move_gate_uses_retained_mixed_gate_order() {
        let map = EntityMap {
            entities: BTreeMap::new(),
            entities_by_creation_order: BTreeMap::new(),
            sectors: BTreeMap::new(),
            sector_indices: BTreeMap::new(),
            // Original constructed jump gate 1 between two stateful doors;
            // Rust installed the stateful doors first, so its runtime peer is
            // door-table index 3.
            gates: vec![
                robin_engine::gate::DoorIndex::from(0),
                robin_engine::gate::DoorIndex::from(3),
                robin_engine::gate::DoorIndex::from(1),
            ],
            runtime_creation_order_boundary: 0,
        };

        assert_eq!(map.translate_gate(1), 3);
    }

    fn group_move_route_fixture(
        actor: TraceEntityId,
        kind: &str,
        ordinal: u64,
    ) -> TraceRouteConstructionEvent {
        let point = TracePoint {
            x: TraceFloat { bits: 0 },
            y: TraceFloat { bits: 0 },
        };
        TraceRouteConstructionEvent {
            kind: kind.to_owned(),
            actor,
            source: point,
            source_sector: 114,
            source_level: 7,
            goal: point,
            goal_sector: 117,
            goal_level: 8,
            gates: Vec::new(),
            draft_diagnostics: BTreeMap::from([
                (
                    "ordinal".to_owned(),
                    TraceJsonValue::from(TraceJsonTree::Unsigned(ordinal)),
                ),
                (
                    "result".to_owned(),
                    TraceJsonValue::from(TraceJsonTree::String("success".to_owned())),
                ),
            ]),
        }
    }

    #[test]
    fn legacy_route_ordinals_restore_original_append_order() {
        let actor = TraceEntityId {
            kind: TraceEntityKind::Pc,
            index: 344,
        };
        let mut routes = [
            group_move_route_fixture(actor, "move", 0),
            group_move_route_fixture(actor, "move", 1),
        ];
        routes[0].draft_diagnostics.remove("ordinal");
        routes[1].draft_diagnostics.remove("ordinal");
        routes[0].draft_diagnostics.remove("result");
        routes[1].draft_diagnostics.remove("result");

        restore_legacy_route_construction_diagnostics(&mut routes);

        assert_eq!(required_route_construction_ordinal(&routes[0]), 0);
        assert_eq!(required_route_construction_ordinal(&routes[1]), 1);
        for route in &routes {
            assert!(matches!(
                route
                    .draft_diagnostics
                    .get("result")
                    .map(TraceJsonValue::tree),
                Some(TraceJsonTree::String(result)) if result == "success"
            ));
        }
    }

    #[test]
    fn legacy_route_ordinal_restore_preserves_recorded_append_order() {
        let actor = TraceEntityId {
            kind: TraceEntityKind::Pc,
            index: 344,
        };
        let mut routes = [
            group_move_route_fixture(actor, "move", 0),
            group_move_route_fixture(actor, "move", 1),
        ];

        restore_legacy_route_construction_diagnostics(&mut routes);

        assert_eq!(required_route_construction_ordinal(&routes[0]), 0);
        assert_eq!(required_route_construction_ordinal(&routes[1]), 1);
    }

    #[test]
    #[should_panic(expected = "schema-16 route event lacks an unsigned ordinal")]
    fn current_route_event_without_ordinal_remains_invalid() {
        let actor = TraceEntityId {
            kind: TraceEntityKind::Pc,
            index: 344,
        };
        let mut route = group_move_route_fixture(actor, "move", 0);
        route.draft_diagnostics.remove("ordinal");

        required_route_construction_ordinal(&route);
    }

    fn group_move_route_map(max_gate: u32) -> EntityMap {
        EntityMap {
            entities: BTreeMap::new(),
            entities_by_creation_order: BTreeMap::new(),
            sectors: BTreeMap::new(),
            sector_indices: BTreeMap::new(),
            gates: (0..=max_gate)
                .map(robin_engine::gate::DoorIndex::from)
                .collect(),
            runtime_creation_order_boundary: 0,
        }
    }

    fn group_move_sector_kinds(
        max_sector: u16,
        door: Option<(u16, u32)>,
    ) -> Vec<LegacyGridSectorAsset> {
        let mut sectors = vec![LegacyGridSectorAsset::NullOrOrdinary; usize::from(max_sector) + 1];
        if let Some((sector, gate_index)) = door {
            sectors[usize::from(sector)] = LegacyGridSectorAsset::Door { gate_index };
        }
        sectors
    }

    #[test]
    fn current_schema_group_move_recovers_ordinary_route_over_door_overlay() {
        let actor = TraceEntityId {
            kind: TraceEntityKind::Pc,
            index: 344,
        };
        let command = TraceCommand::GroupMove {
            actors: vec![actor],
            destination: TracePoint {
                x: TraceFloat {
                    bits: 357.031_98_f32.to_bits(),
                },
                y: TraceFloat {
                    bits: 714.0_f32.to_bits(),
                },
            },
            running: false,
            show_marker: true,
            goal_sector: 117,
            goal_layer: 8,
        };
        let mut route = group_move_route_fixture(actor, "move", 0);
        route.gates.push(TraceRouteGate {
            gate_id: 53,
            direct: false,
            sector_out: 64,
            level_out: 4,
            sector_in: 290,
            level_in: 13,
            draft_diagnostics: BTreeMap::new(),
        });
        let routes = [route];
        let mut consumed = BTreeSet::new();
        let map = group_move_route_map(53);
        let sectors = group_move_sector_kinds(292, Some((292, 53)));

        assert_eq!(
            resolve_current_group_move_route(&command, &routes, &mut consumed, &map, &sectors,),
            Some(ReplayGroupMoveResolution {
                door_route: false,
                unmapped_goal_search_sector: Some(64),
                recorded_gate_routes: vec![(actor, vec![(53, false)])],
                recorded_failed_gate_routes: Vec::new(),
            })
        );
        assert_eq!(consumed, BTreeSet::from([0]));
    }

    #[test]
    fn current_schema_same_sector_group_move_retains_ordinary_goal_kind() {
        let actor = TraceEntityId {
            kind: TraceEntityKind::Pc,
            index: 320,
        };
        let command = TraceCommand::GroupMove {
            actors: vec![actor],
            destination: TracePoint {
                x: TraceFloat {
                    bits: 2533.968_f32.to_bits(),
                },
                y: TraceFloat {
                    bits: 580.920_04_f32.to_bits(),
                },
            },
            running: false,
            show_marker: true,
            goal_sector: 150,
            goal_layer: 4,
        };
        let mut consumed = BTreeSet::new();

        assert_eq!(
            resolve_current_group_move_route(
                &command,
                &[],
                &mut consumed,
                &group_move_route_map(0),
                &group_move_sector_kinds(150, None),
            ),
            Some(ReplayGroupMoveResolution {
                door_route: false,
                unmapped_goal_search_sector: None,
                recorded_gate_routes: Vec::new(),
                recorded_failed_gate_routes: Vec::new(),
            })
        );
        assert!(consumed.is_empty());
    }

    #[test]
    fn current_schema_group_moves_share_frame_routes_in_command_order() {
        let actor = TraceEntityId {
            kind: TraceEntityKind::Pc,
            index: 345,
        };
        let command = TraceCommand::GroupMove {
            actors: vec![actor],
            destination: TracePoint {
                x: TraceFloat { bits: 0 },
                y: TraceFloat { bits: 0 },
            },
            running: false,
            show_marker: true,
            goal_sector: 117,
            goal_layer: 8,
        };
        let mut first = group_move_route_fixture(actor, "move", 40);
        first.gates.push(TraceRouteGate {
            gate_id: 53,
            direct: false,
            sector_out: 64,
            level_out: 4,
            sector_in: 290,
            level_in: 13,
            draft_diagnostics: BTreeMap::new(),
        });
        let mut second = group_move_route_fixture(actor, "move", 41);
        second.gates.push(TraceRouteGate {
            gate_id: 54,
            direct: true,
            sector_out: 63,
            level_out: 3,
            sector_in: 65,
            level_in: 4,
            draft_diagnostics: BTreeMap::new(),
        });
        let routes = [second, first];
        let mut consumed = BTreeSet::new();
        let map = group_move_route_map(54);
        let sectors = group_move_sector_kinds(117, None);

        let first_resolution =
            resolve_current_group_move_route(&command, &routes, &mut consumed, &map, &sectors)
                .unwrap();
        assert_eq!(
            first_resolution.recorded_gate_routes,
            vec![(actor, vec![(53, false)])]
        );
        assert_eq!(consumed, BTreeSet::from([40]));

        let second_resolution =
            resolve_current_group_move_route(&command, &routes, &mut consumed, &map, &sectors)
                .unwrap();
        assert_eq!(
            second_resolution.recorded_gate_routes,
            vec![(actor, vec![(54, true)])]
        );
        assert_eq!(consumed, BTreeSet::from([40, 41]));
    }

    #[test]
    fn current_schema_group_move_recovers_internal_door_branch_from_retained_goal_kind() {
        let actor = TraceEntityId {
            kind: TraceEntityKind::Pc,
            index: 344,
        };
        let command = TraceCommand::GroupMove {
            actors: vec![actor],
            destination: TracePoint {
                x: TraceFloat { bits: 0 },
                y: TraceFloat { bits: 0 },
            },
            running: false,
            show_marker: true,
            goal_sector: 292,
            goal_layer: 4,
        };
        let mut route = group_move_route_fixture(actor, "move", 4);
        route.goal_sector = 292;
        route.gates.push(TraceRouteGate {
            gate_id: 53,
            direct: false,
            sector_out: 64,
            level_out: 4,
            sector_in: 290,
            level_in: 13,
            draft_diagnostics: BTreeMap::new(),
        });
        let routes = [route];
        let mut consumed = BTreeSet::new();
        let map = group_move_route_map(53);
        let sectors = group_move_sector_kinds(292, Some((292, 53)));

        assert_eq!(
            resolve_current_group_move_route(&command, &routes, &mut consumed, &map, &sectors,),
            Some(ReplayGroupMoveResolution {
                door_route: true,
                unmapped_goal_search_sector: Some(64),
                recorded_gate_routes: vec![(actor, vec![(53, false)])],
                recorded_failed_gate_routes: Vec::new(),
            })
        );
    }

    #[test]
    fn current_schema_group_move_retains_door_branch_for_failed_empty_route() {
        let actor = TraceEntityId {
            kind: TraceEntityKind::Pc,
            index: 344,
        };
        let command = TraceCommand::GroupMove {
            actors: vec![actor],
            destination: TracePoint {
                x: TraceFloat { bits: 0 },
                y: TraceFloat { bits: 0 },
            },
            running: false,
            show_marker: true,
            goal_sector: 292,
            goal_layer: 4,
        };
        let mut route = group_move_route_fixture(actor, "move", 5);
        route.goal_sector = 292;
        route.draft_diagnostics.insert(
            "result".to_owned(),
            TraceJsonValue::from(TraceJsonTree::String("failure".to_owned())),
        );
        let routes = [route];
        let map = group_move_route_map(53);
        let sectors = group_move_sector_kinds(292, Some((292, 53)));
        let mut consumed = BTreeSet::new();

        assert_eq!(
            resolve_current_group_move_route(&command, &routes, &mut consumed, &map, &sectors,),
            Some(ReplayGroupMoveResolution {
                door_route: true,
                unmapped_goal_search_sector: None,
                recorded_gate_routes: Vec::new(),
                recorded_failed_gate_routes: vec![actor],
            })
        );
        assert_eq!(consumed, BTreeSet::from([5]));
    }

    #[test]
    fn current_schema_failed_ordinary_group_move_is_authoritative() {
        let actor = TraceEntityId {
            kind: TraceEntityKind::Pc,
            index: 136,
        };
        let command = TraceCommand::GroupMove {
            actors: vec![actor],
            destination: TracePoint {
                x: TraceFloat {
                    bits: 642.953_6_f32.to_bits(),
                },
                y: TraceFloat {
                    bits: 730.12_f32.to_bits(),
                },
            },
            running: false,
            show_marker: true,
            goal_sector: 421,
            goal_layer: 6,
        };
        let mut route = group_move_route_fixture(actor, "move", 0);
        route.source_sector = 116;
        route.source_level = 8;
        route.goal_sector = 421;
        route.goal_level = 6;
        route.draft_diagnostics.insert(
            "result".to_owned(),
            TraceJsonValue::from(TraceJsonTree::String("failure".to_owned())),
        );
        let mut consumed = BTreeSet::new();

        assert_eq!(
            resolve_current_group_move_route(
                &command,
                &[route],
                &mut consumed,
                &group_move_route_map(0),
                &group_move_sector_kinds(421, None),
            ),
            Some(ReplayGroupMoveResolution {
                door_route: false,
                unmapped_goal_search_sector: None,
                recorded_gate_routes: Vec::new(),
                recorded_failed_gate_routes: vec![actor],
            })
        );
        assert_eq!(consumed, BTreeSet::from([0]));
    }

    #[test]
    fn successful_patch_group_move_uses_terminal_gate_as_rust_search_sector() {
        let actor = TraceEntityId {
            kind: TraceEntityKind::Pc,
            index: 297,
        };
        let command = TraceCommand::GroupMove {
            actors: vec![actor],
            destination: TracePoint {
                x: TraceFloat { bits: 0 },
                y: TraceFloat { bits: 0 },
            },
            running: false,
            show_marker: true,
            goal_sector: 492,
            goal_layer: 0,
        };
        let mut route = group_move_route_fixture(actor, "move", 0);
        route.goal_sector = 492;
        route.gates.push(TraceRouteGate {
            gate_id: 78,
            direct: true,
            sector_out: 0,
            level_out: 0,
            sector_in: 491,
            level_in: 6,
            draft_diagnostics: BTreeMap::new(),
        });
        let mut consumed = BTreeSet::new();
        let map = group_move_route_map(78);
        let sectors = group_move_sector_kinds(492, None);
        let resolution =
            resolve_current_group_move_route(&command, &[route], &mut consumed, &map, &sectors);

        assert_eq!(
            resolution,
            Some(ReplayGroupMoveResolution {
                door_route: false,
                unmapped_goal_search_sector: Some(491),
                recorded_gate_routes: vec![(actor, vec![(78, true)])],
                recorded_failed_gate_routes: Vec::new(),
            })
        );
        let map = EntityMap {
            entities: BTreeMap::new(),
            entities_by_creation_order: BTreeMap::new(),
            sectors: BTreeMap::from([(491, 55)]),
            sector_indices: BTreeMap::from([(
                491,
                robin_engine::fast_find_grid::SectorIndex::new(9).unwrap(),
            )]),
            gates: Vec::new(),
            runtime_creation_order_boundary: 0,
        };
        assert_eq!(
            map.translate_group_move_goal_sector(
                492,
                0,
                resolution.and_then(|resolution| resolution.unmapped_goal_search_sector),
            ),
            GroupMoveGoalTranslation::Runtime(
                (SectorNumber::new(55), 0),
                robin_engine::fast_find_grid::SectorIndex::new(9).unwrap(),
            )
        );
    }

    fn drop_ale_route_fixture(
        actor: TraceEntityId,
        target: TracePoint,
    ) -> TraceRouteConstructionEvent {
        TraceRouteConstructionEvent {
            kind: "move".to_owned(),
            actor,
            source: TracePoint {
                x: TraceFloat {
                    bits: 2413.0_f32.to_bits(),
                },
                y: TraceFloat {
                    bits: 802.0_f32.to_bits(),
                },
            },
            source_sector: 394,
            source_level: 6,
            goal: target,
            goal_sector: 148,
            goal_level: 4,
            gates: vec![TraceRouteGate {
                gate_id: 0,
                direct: false,
                sector_out: 148,
                level_out: 4,
                sector_in: 394,
                level_in: 6,
                draft_diagnostics: BTreeMap::new(),
            }],
            draft_diagnostics: BTreeMap::from([
                (
                    "ordinal".to_owned(),
                    TraceJsonValue::from(TraceJsonTree::Unsigned(7)),
                ),
                (
                    "result".to_owned(),
                    TraceJsonValue::from(TraceJsonTree::String("success".to_owned())),
                ),
            ]),
        }
    }

    fn drop_ale_route_map(actor: TraceEntityId) -> EntityMap {
        let goal_sector_index = robin_engine::fast_find_grid::SectorIndex::new(37).unwrap();
        EntityMap {
            entities: BTreeMap::from([(actor, EntityId::Pc(robin_engine::entity_id::PcId(12)))]),
            entities_by_creation_order: BTreeMap::new(),
            sectors: BTreeMap::from([(148, 55), (394, 56)]),
            sector_indices: BTreeMap::from([
                (148, goal_sector_index),
                (
                    394,
                    robin_engine::fast_find_grid::SectorIndex::new(38).unwrap(),
                ),
            ]),
            gates: vec![robin_engine::gate::DoorIndex::from(42)],
            runtime_creation_order_boundary: 0,
        }
    }

    #[test]
    #[should_panic(expected = "schema-16 DropAle route has invalid result: None")]
    fn current_drop_ale_route_without_result_remains_invalid() {
        let actor = TraceEntityId {
            kind: TraceEntityKind::Pc,
            index: 320,
        };
        let target = TracePoint {
            x: TraceFloat { bits: 0 },
            y: TraceFloat { bits: 0 },
        };
        let mut route = drop_ale_route_fixture(actor, target);
        route.draft_diagnostics.remove("result");

        recorded_gate_path_from_event(&route, &drop_ale_route_map(actor));
    }

    #[test]
    fn current_schema_drop_ale_recovers_save067_route_goal() {
        let actor = TraceEntityId {
            kind: TraceEntityKind::Pc,
            index: 320,
        };
        let target = TracePoint {
            x: TraceFloat {
                bits: 2607.467_041_f32.to_bits(),
            },
            y: TraceFloat {
                bits: 881.610_474_f32.to_bits(),
            },
        };
        let command = TraceCommand::DropAleAt {
            actor,
            target,
            running: false,
        };
        let routes = [drop_ale_route_fixture(actor, target)];
        let mut consumed = BTreeSet::new();

        assert_eq!(
            resolve_current_drop_ale(
                &command,
                &routes,
                &mut consumed,
                &drop_ale_route_map(actor),
                None,
                false,
            ),
            Some(ReplayDropAleResolution {
                goal: (SectorNumber::new(55), 4),
                goal_sector_index: robin_engine::fast_find_grid::SectorIndex::new(37),
                recorded_gate_path: Some(robin_engine::gate::RecordedGatePath {
                    source_sector: SectorNumber::new(56),
                    source_sector_index: robin_engine::fast_find_grid::SectorIndex::new(38),
                    source_layer: 6,
                    outcome: robin_engine::gate::RecordedGateOutcome::Success(vec![
                        robin_engine::gate::GatePathStep {
                            door_index: robin_engine::gate::DoorIndex::from(42),
                            direct: false,
                        },
                    ]),
                }),
            })
        );
        assert_eq!(consumed, BTreeSet::from([7]));
    }

    #[test]
    #[should_panic(expected = "already consumed by the group-move join")]
    fn current_schema_route_ordinal_cannot_be_claimed_by_two_joiners() {
        claim_delayed_drop_ale_route_ordinal(7, &mut BTreeSet::new(), &BTreeSet::from([7]));
    }

    #[test]
    #[should_panic(expected = "matched twice")]
    fn current_schema_delayed_route_ordinal_cannot_be_claimed_twice() {
        let mut delayed = BTreeSet::from([7]);
        claim_delayed_drop_ale_route_ordinal(7, &mut delayed, &BTreeSet::new());
    }

    #[test]
    fn delayed_drop_ale_ignores_route_without_staged_seek() {
        let actor = TraceEntityId {
            kind: TraceEntityKind::Pc,
            index: 320,
        };
        let target = TracePoint {
            x: TraceFloat {
                bits: 2607.467_041_f32.to_bits(),
            },
            y: TraceFloat {
                bits: 881.610_474_f32.to_bits(),
            },
        };

        let mut consumed = BTreeSet::new();
        let routes = collect_current_delayed_drop_ale_routes_matching(
            &[drop_ale_route_fixture(actor, target)],
            &mut consumed,
            &BTreeSet::new(),
            &drop_ale_route_map(actor),
            |_, _| false,
        );

        assert!(routes.is_empty());
        assert!(consumed.is_empty());
    }

    #[test]
    fn current_schema_delayed_drop_ale_retains_recorded_failure_outcome() {
        let actor = TraceEntityId {
            kind: TraceEntityKind::Pc,
            index: 320,
        };
        let target = TracePoint {
            x: TraceFloat {
                bits: 2607.467_041_f32.to_bits(),
            },
            y: TraceFloat {
                bits: 881.610_474_f32.to_bits(),
            },
        };
        let mut event = drop_ale_route_fixture(actor, target);
        event.gates.clear();
        event.draft_diagnostics.insert(
            "result".to_owned(),
            TraceJsonValue::from(TraceJsonTree::String("failure".to_owned())),
        );
        let mut consumed = BTreeSet::new();

        let routes = collect_current_delayed_drop_ale_routes_matching(
            &[event],
            &mut consumed,
            &BTreeSet::new(),
            &drop_ale_route_map(actor),
            |runtime_actor, destination| {
                runtime_actor == EntityId::Pc(robin_engine::entity_id::PcId(12))
                    && destination.x.to_bits() == target.x.bits
                    && destination.y.to_bits() == target.y.bits
            },
        );

        assert_eq!(consumed, BTreeSet::from([7]));
        assert_eq!(routes.len(), 1);
        assert!(matches!(
            &routes[0].recorded_gate_path.outcome,
            robin_engine::gate::RecordedGateOutcome::Failure
        ));
    }

    #[test]
    #[should_panic(expected = "matched 2 exact route events")]
    fn current_schema_drop_ale_rejects_duplicate_exact_command_routes() {
        let actor = TraceEntityId {
            kind: TraceEntityKind::Pc,
            index: 0,
        };
        let target = TracePoint {
            x: TraceFloat {
                bits: 2607.467_041_f32.to_bits(),
            },
            y: TraceFloat {
                bits: 881.610_474_f32.to_bits(),
            },
        };
        let command = TraceCommand::DropAleAt {
            actor,
            target,
            running: false,
        };
        let first = drop_ale_route_fixture(actor, target);
        let mut second = drop_ale_route_fixture(actor, target);
        second.draft_diagnostics.insert(
            "ordinal".to_owned(),
            TraceJsonValue::from(TraceJsonTree::Unsigned(8)),
        );
        resolve_current_drop_ale(
            &command,
            &[first, second],
            &mut BTreeSet::new(),
            &drop_ale_route_map(actor),
            None,
            false,
        );
    }

    #[test]
    fn drop_ale_route_recovery_rejects_nonmatching_point() {
        let actor = TraceEntityId {
            kind: TraceEntityKind::Pc,
            index: 320,
        };
        let target = TracePoint {
            x: TraceFloat { bits: 0x4522_f77a },
            y: TraceFloat { bits: 0x445c_6712 },
        };
        let command = TraceCommand::DropAleAt {
            actor,
            target,
            running: false,
        };
        let mut wrong_target = target;
        wrong_target.x.bits ^= 1;
        let routes = [drop_ale_route_fixture(actor, wrong_target)];
        let map = drop_ale_route_map(actor);
        let mut consumed = BTreeSet::new();

        assert_eq!(
            resolve_current_drop_ale(&command, &routes, &mut consumed, &map, None, false),
            None
        );
        assert!(consumed.is_empty());
    }

    #[test]
    #[should_panic(expected = "has no retained Rust position-sector mapping")]
    fn current_schema_drop_ale_rejects_unmapped_authoritative_goal() {
        let actor = TraceEntityId {
            kind: TraceEntityKind::Pc,
            index: 320,
        };
        let target = TracePoint {
            x: TraceFloat { bits: 0x4522_f77a },
            y: TraceFloat { bits: 0x445c_6712 },
        };
        let command = TraceCommand::DropAleAt {
            actor,
            target,
            running: false,
        };
        let mut map = drop_ale_route_map(actor);
        map.sectors.clear();

        let _ = resolve_current_drop_ale(
            &command,
            &[drop_ale_route_fixture(actor, target)],
            &mut BTreeSet::new(),
            &map,
            None,
            false,
        );
    }

    #[test]
    fn current_schema_drop_ale_recovers_same_sector_actor_goal_without_route() {
        let actor = TraceEntityId {
            kind: TraceEntityKind::Pc,
            index: 320,
        };
        let command = TraceCommand::DropAleAt {
            actor,
            target: TracePoint {
                x: TraceFloat { bits: 0x44d7_a800 },
                y: TraceFloat { bits: 0x4447_8000 },
            },
            running: false,
        };
        let expected = ReplayDropAleResolution {
            goal: (SectorNumber::new(0), 0),
            goal_sector_index: robin_engine::fast_find_grid::SectorIndex::new(0),
            recorded_gate_path: None,
        };
        let mut consumed = BTreeSet::new();

        assert_eq!(
            resolve_current_drop_ale(
                &command,
                &[],
                &mut consumed,
                &drop_ale_route_map(actor),
                Some(expected.clone()),
                false,
            ),
            Some(expected)
        );
        assert!(consumed.is_empty());
    }

    #[test]
    fn legacy_drop_ale_actor_fallback_requires_exact_same_sector() {
        use robin_engine::fast_find_grid::SectorIndex;
        use robin_engine::position_interface::SectorHandle;

        let actor = SectorHandle::new(50)
            .unwrap()
            .with_arena_index(SectorIndex::new(50).unwrap());
        let same = SectorHandle::new(50)
            .unwrap()
            .with_arena_index(SectorIndex::new(50).unwrap());
        let repeated_public_number = SectorHandle::new(50)
            .unwrap()
            .with_arena_index(SectorIndex::new(150).unwrap());
        let cross_sector = SectorHandle::new(0)
            .unwrap()
            .with_arena_index(SectorIndex::new(0).unwrap());
        let number_only = SectorHandle::new(50).unwrap();

        assert!(legacy_drop_ale_target_is_same_exact_sector(actor, same));
        assert!(!legacy_drop_ale_target_is_same_exact_sector(
            actor,
            repeated_public_number
        ));
        assert!(!legacy_drop_ale_target_is_same_exact_sector(
            actor,
            cross_sector
        ));
        assert!(!legacy_drop_ale_target_is_same_exact_sector(
            actor,
            number_only
        ));
    }

    #[test]
    #[should_panic(
        expected = "schema-16 DropAle recorded as a quick action has no authoritative target-sector identity"
    )]
    fn current_schema_drop_ale_qa_rejects_actor_sector_as_a_fake_target_fallback() {
        let actor = TraceEntityId {
            kind: TraceEntityKind::Pc,
            index: 320,
        };
        let command = TraceCommand::DropAleAt {
            actor,
            target: TracePoint {
                x: TraceFloat { bits: 0x44d7_a800 },
                y: TraceFloat { bits: 0x4447_8000 },
            },
            running: false,
        };
        let fake_actor_goal = ReplayDropAleResolution {
            goal: (SectorNumber::new(0), 0),
            goal_sector_index: robin_engine::fast_find_grid::SectorIndex::new(0),
            recorded_gate_path: None,
        };

        let _ = resolve_current_drop_ale(
            &command,
            &[],
            &mut BTreeSet::new(),
            &drop_ale_route_map(actor),
            Some(fake_actor_goal),
            true,
        );
    }

    #[test]
    fn runtime_identity_ignores_only_numeric_gaps_not_persistent_reordering() {
        let original_projectile = TraceEntityId {
            kind: TraceEntityKind::Projectile,
            index: 131,
        };
        let original_bonus = TraceEntityId {
            kind: TraceEntityKind::Bonus,
            index: 132,
        };
        let rust_projectile = EntityId::Projectile(robin_engine::entity_id::ProjectileId(40));
        let rust_bonus = EntityId::Bonus(robin_engine::entity_id::BonusId(12));
        let originals = vec![
            (original_projectile, 172, EntityIdKind::Projectile),
            (original_bonus, 174, EntityIdKind::Bonus),
        ];

        let shifted = pair_runtime_identities_by_persistent_rank(
            originals.clone(),
            vec![
                (rust_projectile, 170, EntityIdKind::Projectile),
                (rust_bonus, 171, EntityIdKind::Bonus),
            ],
        )
        .unwrap();
        assert_eq!(
            shifted,
            vec![
                (original_projectile, 172, rust_projectile),
                (original_bonus, 174, rust_bonus),
            ]
        );

        let reordered = pair_runtime_identities_by_persistent_rank(
            originals,
            vec![
                (rust_bonus, 170, EntityIdKind::Bonus),
                (rust_projectile, 171, EntityIdKind::Projectile),
            ],
        );
        assert!(
            reordered
                .unwrap_err()
                .contains("persistent creation rank 0")
        );
    }

    #[test]
    fn automatic_dump_window_retains_configured_prior_frames_and_current_frame() {
        let mut frames = VecDeque::new();
        for frame in 0..50 {
            push_rolling_window(&mut frames, frame);
        }
        assert_eq!(
            frames.into_iter().collect::<Vec<_>>(),
            (17..50).collect::<Vec<_>>()
        );
    }
}

/// Wraps a Rust entity id so its `Debug` rendering also carries the original-game
/// trace index it was mapped from, e.g. `Pc(PcId(174))[orig:171]`.
///
/// The divergence report is read alongside `--dump-entity` (Original indices)
/// and the Original's own `[DBG]` logs; the id spaces frequently differ.
struct EntityLabel {
    id: robin_engine::entity_id::EntityId,
    original_index: u32,
}

impl std::fmt::Debug for EntityLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}[orig:{}]", self.id, self.original_index)
    }
}
