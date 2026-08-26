//! Replay a plain or zstd-compressed JSONL parity trace produced by the
//! original C++ game.
//!
//! This intentionally accepts the neutral, resolved-command schema emitted by
//! `original-code/RHParity.cpp`, rather than the Rust-native replay schema.
//!
//! Usage:
//!   ROBINHOOD_DATA_DIR=datadirs/demo_leicester_linux \
//!     cargo run --example original_parity_replay -- \
//!       original-code/parity-traces/original-demo-baseline.jsonl

#![recursion_limit = "256"]

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
use fs2::FileExt as _;
use robin_engine::coordinates::MapPoint;
use robin_engine::coordinates::WorldPoint3D;
use robin_engine::element::{Command, Entity, EntityId, EntityIdKind};
use robin_engine::engine::{
    DevState, Engine, HostDisplayState, InputState, LegacyGridSectorAsset, LevelAssets,
};
use robin_engine::fast_find_grid::LineIndex;
use robin_engine::game_operation::GameCode;
use robin_engine::graphic_config::TextureScaleMode;
use robin_engine::player_command::PlayerCommand;
use robin_engine::profiles::Action;
use robin_engine::sector::SectorNumber;
use robin_engine::sprite::BBox;
use robin_rs::Host;
use robin_rs::gfx_types::BlendMode;
use robin_rs::level_loading_host::EngineLevelLoadExt;
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

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
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
    authoritative_state: Option<String>,
    #[serde(default)]
    random_input_seed: Option<u32>,
    sim_config: TraceSimConfig,
    campaign: TraceCampaign,
    motion_grid: TraceMotionGrid,
    /// Schema-16 session-boundary state omitted by Original's RHSG payload.
    /// Older schema-16 recordings do not contain this overlay and use the
    /// legacy-save reconstruction fallback.
    #[serde(default)]
    initial_npc_transients: Option<Vec<TraceInitialNpcTransient>>,
    #[serde(default)]
    initial_save: Option<TraceInitialSave>,
}

#[derive(
    Debug,
    Clone,
    Copy,
    Deserialize,
    Serialize,
    PartialEq,
    Eq,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(deny_unknown_fields)]
struct TraceInitialNpcTransient {
    creation_order: u32,
    maximal_visibility: u16,
}

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
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
    Clone,
    Copy,
    Debug,
    Deserialize,
    Eq,
    PartialEq,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
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

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
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
            script_enabled: self.script_enabled,
            highlander: self.highlander,
            highlander2: self.highlander2,
            golden_eye: self.golden_eye,
            ignore_default_loose: self.ignore_default_loose,
            bypass_fog_sprites_crash: self.bypass_fog_sprites_crash,
            amount_of_speaking: self.amount_of_speaking,
            synchronous_pathfinding,
        }
    }
}

#[derive(
    Clone,
    Copy,
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
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
    Clone,
    Copy,
    Debug,
    Deserialize,
    Eq,
    PartialEq,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(rename_all = "snake_case")]
enum TraceStartState {
    MissionStart,
    LoadedSave,
}

fn validate_trace_schema(schema: u32) {
    assert!(
        matches!(schema, 12 | 13 | 14 | 15 | 16),
        "unsupported parity trace schema {schema}; schemas 12 through 16 are supported"
    );
}

fn validate_trace_header(header: &TraceHeader) {
    validate_trace_schema(header.schema);
    assert_eq!(
        header.record_type, "header",
        "invalid parity header record type"
    );
    assert_eq!(
        header.simulation_hz, 25,
        "schema-12 parity replay requires the Original's 25 Hz simulation"
    );
    assert_eq!(
        header.rng_stream, "libc_rand_raw_global_draw_order",
        "unsupported parity RNG stream"
    );
    assert_eq!(
        header.visibility_queries, "opaque_is_reachable",
        "unsupported parity visibility-query contract"
    );
    match header.schema {
        // Schema 14 revises 12's recorder contract without adopting 13's
        // per-frame payload, which no writer ever produced.
        12 | 14 | 15 | 16 => assert!(
            header.authoritative_state.is_none(),
            "schema-{} trace unexpectedly declares per-frame authoritative state",
            header.schema
        ),
        13 => assert_eq!(
            header.authoritative_state.as_deref(),
            Some("per_frame_v1"),
            "unsupported per-frame authoritative-state contract"
        ),
        _ => unreachable!(),
    }
    assert!(
        header.schema == 16 || header.initial_npc_transients.is_none(),
        "only schema 16 may declare initial_npc_transients"
    );
}

fn decode_and_validate_initial_save(header: &TraceHeader) -> Option<Vec<u8>> {
    match (
        header.schema,
        header.start_state,
        header.initial_save.as_ref(),
    ) {
        (12 | 13 | 14 | 15 | 16, TraceStartState::MissionStart, None) => None,
        (12 | 13 | 14 | 15 | 16, TraceStartState::MissionStart, Some(_)) => {
            panic!("mission_start traces must not contain initial_save")
        }
        (12 | 13 | 14 | 15 | 16, TraceStartState::LoadedSave, None) => {
            panic!("loaded_save traces require initial_save")
        }
        (12 | 13 | 14 | 15 | 16, TraceStartState::LoadedSave, Some(initial_save)) => {
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
        (schema, _, _) => unreachable!("schema {schema} was validated before initial_save"),
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
        "schema-16 initial_npc_transients must cover every NPC exactly once"
    );

    let mut seen = BTreeSet::new();
    for transient in transients {
        assert!(
            seen.insert(transient.creation_order),
            "schema-16 initial_npc_transients repeats creation order {}",
            transient.creation_order
        );
        let id = runtime_by_creation_order
            .get(&transient.creation_order)
            .copied()
            .unwrap_or_else(|| {
                panic!(
                    "schema-16 NPC transient creation order {} is absent from the Rust engine",
                    transient.creation_order
                )
            });
        engine.restore_parity_npc_maximal_visibility(id, transient.maximal_visibility);
    }
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

/// Compatibility for already-recorded multi-segment schema-16 sessions.
///
/// Session 1 begins in a fresh Original process, where the omitted member has
/// its constructor value zero. Later loaded-save segments continue the same
/// process and therefore retain the preceding value. Restrict the inference to
/// dead/unconscious observers: Original returns before clearing the maximum,
/// and the four known interactive failures prove that the maximum-supplying
/// detectable remains serialized in those observers' buckets.
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
        engine.restore_parity_npc_maximal_visibility(id, value);
    }
    restorations.len()
}

/// Whether a legacy loaded-save segment can retain process-local state which
/// the RHSG payload omits.
///
/// An in-game reload calls `BeginSession` immediately before `Serialize` and
/// `CaptureCampaign` immediately afterwards, so its recorded RNG prefix is
/// empty. A fresh `RHGame` instead captures the campaign before mission
/// construction; the nonempty prefix contains those setup draws and the new
/// engine's constructor-zero transient state is authoritative.
fn legacy_loaded_save_retains_process_transients(prefix_draw_count: usize) -> bool {
    prefix_draw_count == 0
}

fn preceding_interactive_session_path(path: &Path, session_index: u32) -> Option<PathBuf> {
    if session_index <= 1 {
        return None;
    }
    let previous = session_index.checked_sub(1)?;
    let name = path.file_name()?.to_str()?;
    let suffix = format!("-session-{session_index:04}.jsonl.zst");
    let stem = name.strip_suffix(&suffix)?;
    Some(path.with_file_name(format!("{stem}-session-{previous:04}.jsonl.zst")))
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
    cursor: Option<Option<u16>>,
    macro_in_progress: Option<bool>,
    paths: &[robin_engine::level_data::RawHikingPath],
) -> Option<(robin_engine::ai::PathId, u8, usize)> {
    if macro_in_progress != Some(true) {
        return None;
    }
    let offset = usize::from(cursor??);
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
    if header.schema != 16
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
    if previous_header.schema != 16
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
            engine.restore_parity_npc_dormant_macro_cursor(id, path_id, waypoint, offset, assets),
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

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
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

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
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

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
struct TraceCampaignCharacter {
    profile_index: u32,
    profile_name: String,
    instanced: bool,
    status: TracePcStatus,
}

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
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

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
struct TraceSkill {
    capacity: u32,
    experience: u32,
}

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
struct TraceProductionSector {
    r#type: u32,
    speed: u16,
    amount: u16,
    produced_amount: u16,
    max_amount_reached: bool,
    occupants: Vec<TraceProductionOccupant>,
}

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
struct TraceProductionOccupant {
    character_index: usize,
    x: TraceFloat,
    y: TraceFloat,
    obstacle: u16,
}

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
struct TraceMotionGrid {
    layers: Vec<TraceMotionLayer>,
}

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
struct TraceMotionLayer {
    layer: u16,
    lines: Vec<TraceMotionLine>,
}

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
struct TraceMotionLine {
    index: u16,
    a: TracePoint,
    b: TracePoint,
    type_mask: i32,
    associated_sector: i16,
    active: bool,
}

#[derive(
    Debug,
    Clone,
    Copy,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
struct TraceMotionLineChange {
    layer: u16,
    index: u16,
    active: bool,
}

#[derive(
    Debug,
    Clone,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
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

#[derive(
    Debug,
    Clone,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
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
}

#[derive(
    Debug,
    Clone,
    Copy,
    Deserialize,
    Serialize,
    PartialEq,
    Eq,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(rename_all = "snake_case")]
enum TraceRngDomain {
    Simulation,
    Audio,
}

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
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
    bincode::Encode,
    bincode::Decode,
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
    bincode::Encode,
    bincode::Decode,
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

#[derive(
    Debug,
    Clone,
    Copy,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
struct TraceFloat {
    bits: u32,
}

impl TraceFloat {
    fn value(self) -> f32 {
        f32::from_bits(self.bits)
    }
}

#[derive(
    Debug,
    Clone,
    Copy,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
struct TracePoint {
    x: TraceFloat,
    y: TraceFloat,
}

impl From<TracePoint> for MapPoint {
    fn from(value: TracePoint) -> Self {
        Self::new(value.x.value(), value.y.value())
    }
}

#[derive(
    Debug,
    Clone,
    Copy,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
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

#[derive(
    Debug,
    Serialize,
    Deserialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
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
        /// Absent only in synthetic pre-audit fixtures; the recorder always
        /// pairs the numeric command with its name.
        original_command: u32,
        original_command_name: String,
        with_seek: bool,
        #[serde(default)]
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
        /// Original's raw numeric action, retained so the cache round trip is
        /// lossless. `action` is its resolved name; replay uses the name.
        original_action: u32,
    },
    CancelAction {
        #[serde(default)]
        pc: Option<TraceEntityId>,
        /// Always `no_action`: MSG_SELECT_ACTION with RHACTION_NOACTION is
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

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
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

/// Split refresh-owned orientation records from commands that entered through
/// the input phase of this simulation boundary.
///
/// `RHGame` processes input before `PerformHourglass`, whereas
/// `RHEngine::PerformOrientation` is called from `PerformRefreshAllElements`
/// (`RHgame.cpp:1559-1643, RHengine.cpp:7489-7515`). Ordinarily a refresh
/// orientation left over from the preceding host pass is already at the front
/// of the next frame's command queue. When an orientation follows this
/// boundary's matching `MSG_SELECT_ACTION`, it came from a refresh reached
/// later in the same host pass and therefore must not affect the actor's
/// earlier Execute/PerformAction call.
fn split_late_refresh_orientations(
    commands: Vec<TraceCommand>,
    popup_nested_refresh: bool,
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
                        .find(|(known_actor, known_action, _)| {
                            known_actor == actor && known_action == action
                        })
                {
                    entry.2 = index;
                } else {
                    final_popup_orientations.push((*actor, *action, index));
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
                if actions_selected_this_boundary.get(actor) == Some(action)
                    || final_popup_orientations.iter().any(
                        |(known_actor, known_action, known_index)| {
                            known_actor == actor && known_action == action && *known_index == index
                        },
                    ) =>
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
                        assert!(
                            !engine
                                .fast_grid()
                                .level
                                .sector_number_map
                                .contains_key(&goal.0),
                            "unmapped Original group-move sector {goal_sector} collides with Rust \
                             runtime sector {}",
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
                                            .into_iter()
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
                // RHSequenceElementGeneric.h assigns these stable Original
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
                with_seek,
                seek_distance: normalized_trace_sword_seek_distance(with_seek, seek_distance),
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
                let (already_authorized, goal_override, goal_sector_index_override) =
                    drop_ale_resolution
                        .map(|resolution| {
                            (true, Some(resolution.goal), resolution.goal_sector_index)
                        })
                        .unwrap_or((false, None, None));
                PlayerCommand::DropAleAt {
                    actor: entity_map.translate(actor),
                    target_pos: target.into(),
                    running,
                    already_authorized,
                    goal_override,
                    goal_sector_index_override,
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
                // The Original records the ground-projected 3D danger
                // point; its map projection is the point the player
                // picked, which is what the command carries.
                // TODO: the engine flattens the danger point back to
                // z = 0 when storing it, so the projected elevation is
                // lost on both the live and replay paths.
                let danger_point: WorldPoint3D = danger_point.into();
                PlayerCommand::RaiseShieldWithDanger {
                    actor: entity_map.translate(actor),
                    protected_pc: entity_map.translate(protected_pc),
                    danger_point: danger_point.to_map(),
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
                match engine.selected_pc_ids() {
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

fn normalized_trace_sword_seek_distance(
    with_seek: bool,
    seek_distance: Option<f32>,
) -> Option<f32> {
    if with_seek { seek_distance } else { None }
}

fn command_from_stable_name(name: &str) -> Command {
    let rust_name = match name {
        "camera_jumpto" => "CameraJumpTo".to_owned(),
        // Original calls the low-level rolling movement ROLL and the
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
        .unwrap_or_else(|_| panic!("unsupported stable Original RHcommand name {name:?}"))
}

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
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
    #[serde(default)]
    sprite_frame_count: Option<u16>,
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
    /// Additive v27 whole-entity serialized position/sprite frontier. Older
    /// raw recordings omit it and therefore cannot assert this state.
    #[serde(default)]
    runtime: Option<TraceJsonValue>,
}

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
struct TraceActor {
    action_state: u32,
    animation: u32,
    command: u16,
    command_name: String,
    motion_state: u32,
    wait_time: u32,
    #[serde(default)]
    passing_door_directly: Option<bool>,
    /// Outer `None` means schema 14 or earlier did not record the field;
    /// inner `None` is schema 15's explicit null for no active PassDoor.
    #[serde(default, deserialize_with = "deserialize_present_nullable_pass_door")]
    active_pass_door: Option<Option<TracePassDoor>>,
    /// Retained for schema-15 diagnostics. Rust does not yet expose a stable
    /// public current-sequence snapshot with Original's element identities.
    /// TODO(parity-schema15): compare the remaining fields once that capture
    /// can be produced without walking mutable sequence-manager internals.
    #[serde(
        default,
        deserialize_with = "deserialize_present_nullable_sequence_element"
    )]
    sequence_element: Option<Option<TraceSequenceElement>>,
    /// Schema-16 PositionInterface diagnostics. Kept as a cache-safe JSON
    /// tree because it is observational evidence rather than comparable
    /// engine state yet.
    #[serde(default)]
    position_interface: Option<TraceJsonValue>,
}

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
struct TracePassDoor {
    gate_id: u32,
    direct: bool,
    direction: i16,
}

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
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
    #[serde(default)]
    movement: Option<TraceSequenceMovement>,
    /// Schema-16 sequence topology and active-order diagnostics. These are
    /// nullable or command-shaped in the Original recorder, so retaining the
    /// draft payload verbatim is safer than inventing a false common shape.
    #[serde(default)]
    following: Option<TraceJsonValue>,
    #[serde(default)]
    postponed: Option<TraceJsonValue>,
    #[serde(default)]
    current_order: Option<TraceJsonValue>,
    #[serde(default)]
    movement_payload: Option<TraceJsonValue>,
}

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
struct TraceSequenceMovement {
    /// Absent in current schema-16 traces when the movement-element
    /// constructor does not initialize `maction` (for example WAIT_FREE_LIFT).
    /// Older schema-15/16 traces may contain the field.
    #[serde(default)]
    action: Option<u32>,
    #[serde(default)]
    pass_door: Option<TracePassDoor>,
}

fn deserialize_present_nullable_pass_door<'de, D>(
    deserializer: D,
) -> Result<Option<Option<TracePassDoor>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<TracePassDoor>::deserialize(deserializer).map(Some)
}

fn deserialize_present_nullable_sequence_element<'de, D>(
    deserializer: D,
) -> Result<Option<Option<TraceSequenceElement>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<TraceSequenceElement>::deserialize(deserializer).map(Some)
}

fn trace_pass_door_key(pass: &TracePassDoor) -> (u32, bool) {
    assert_eq!(
        pass.direct,
        pass.direction != 0,
        "schema-15 active PassDoor direct flag disagrees with its direction"
    );
    (pass.gate_id, pass.direct)
}

fn active_pass_door_keys_match(
    expected: Option<&TracePassDoor>,
    actual: Option<(u32, bool)>,
) -> bool {
    expected.map(trace_pass_door_key) == actual
}

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
struct TraceHuman {
    life_points: i16,
    dead: bool,
    unconscious: bool,
    camp: String,
    original_camp: i32,
    vip: bool,
    civilian: bool,
    #[serde(default)]
    opponents: Option<Vec<TraceEntityId>>,
    #[serde(default)]
    opponent_jump_lines: Option<Vec<Option<TraceJumpLine>>>,
}

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
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

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
struct TraceElementPc {
    ammo: TraceElementAmmo,
}

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
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

fn deserialize_present_nullable_u16<'de, D>(
    deserializer: D,
) -> Result<Option<Option<u16>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<u16>::deserialize(deserializer).map(Some)
}

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
struct TraceAi {
    state: u32,
    substate: u32,
    #[serde(default)]
    script_locked: Option<bool>,
    #[serde(default)]
    locked: Option<bool>,
    #[serde(default)]
    locks: Option<u8>,
    #[serde(default)]
    was_busy: Option<bool>,
    #[serde(default)]
    very_busy: Option<bool>,
    #[serde(default)]
    macro_timer_running: Option<bool>,
    #[serde(default)]
    macro_timer_ring: Option<u32>,
    /// Outer `None` means the additive diagnostic was not recorded; inner
    /// `None` is the recorded null cursor for an inactive macro.
    #[serde(default, deserialize_with = "deserialize_present_nullable_u16")]
    macro_cursor: Option<Option<u16>>,
    #[serde(default)]
    macro_remaining: Option<u16>,
    #[serde(default)]
    macro_in_progress: Option<bool>,
    #[serde(default)]
    list_us: Option<Vec<TraceEntityId>>,
    #[serde(default)]
    list_them: Option<Vec<TraceEntityId>>,
    /// Outer `None` is a legacy schema-16 snapshot without this field; inner
    /// `None` is the authoritative null `mpMyLineJump` for a soldier.
    #[serde(default, deserialize_with = "deserialize_present_nullable_jump_line")]
    my_line_jump: Option<Option<TraceJumpLine>>,
}

fn deserialize_present_nullable_jump_line<'de, D>(
    deserializer: D,
) -> Result<Option<Option<TraceJumpLine>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<TraceJumpLine>::deserialize(deserializer).map(Some)
}

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
struct TraceDetection {
    suspects: Vec<u16>,
    maximal_suspect: u16,
    maximal_visibility: u32,
    view_status: u8,
    alert_status: u32,
    detectables: Vec<TraceDetectable>,
}

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
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

#[derive(
    Clone,
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
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

#[derive(
    Clone,
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
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

#[derive(
    Clone,
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
struct TraceSightObstacleTypes {
    solid: bool,
    opaque: bool,
    projection_area: bool,
    mouse: bool,
    shield: bool,
    show_shadow_polygon: bool,
}

#[derive(
    Clone,
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
struct TraceSightObstacleBox {
    min: TracePoint,
    max: TracePoint,
}

#[derive(
    Clone,
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
struct TraceSightObstaclePoint {
    x: TraceFloat,
    y: TraceFloat,
    z_top: TraceFloat,
    z_bottom: TraceFloat,
}

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
struct TraceResolvedExclamation {
    actor: TraceEntityId,
    identifier: u32,
    exclamation_id: u16,
    selected_variant: i32,
    selected_entry: Option<u32>,
    duration_frames: u32,
}

/// One exact, ordered Original `RHSprite::PerformMotion` position commit.
/// This additive diagnostic is absent unless the Original recorder was run
/// with `RH_PARITY_MOVEMENT_STEPS` enabled.
#[derive(
    Clone,
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
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

/// One exact, ordered Original `RHSprite::PerformFlight` execution.
#[derive(
    Clone,
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
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

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
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

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReplayDropAleResolution {
    goal: (SectorNumber, u16),
    goal_sector_index: Option<robin_engine::fast_find_grid::SectorIndex>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReplayGroupMoveResolution {
    door_route: bool,
    /// A patch sector is a real Original motion-area identity but has no
    /// standalone Rust position-sector number. For a successful recorded
    /// gate route, its terminal gate exit is the equivalent Rust graph goal.
    unmapped_goal_search_sector: Option<u16>,
    /// Successful `AppendMoveToSequence` gate paths are already
    /// authoritative at this boundary. Replaying them avoids a second A*
    /// search choosing a different valid path and changing the emitted
    /// building waits (including their RNG draws).
    recorded_gate_routes: Vec<(TraceEntityId, Vec<(u32, bool)>)>,
    /// Authoritative failed `AppendMoveToSequence` searches, keyed by actor.
    /// The empty gate list is an observed failure outcome, not permission to
    /// run Rust's A* again against reconstructed topology.
    recorded_failed_gate_routes: Vec<TraceEntityId>,
}

/// Recover whether `AppendMoveToSequence` selected its internal
/// `FindPathGates` or `FindPathIntoDoor` branch for a schema-16 group move.
/// Both branches record route kind `move`; `move_to_door` belongs to the
/// separate `AppendMoveToDoorToSequence` API and is not this discriminator.
/// The retained Original sparse sector topology preserves that distinction:
/// a door goal names its exact gate, while an ordinary patch remains ordinary
/// even when its route happens to end across that same overlay door.
// TODO(parity-schema): record the selected group-move route constructor on
// TraceCommand::GroupMove so future traces do not need this event join.
fn resolve_schema_sixteen_group_move_route(
    schema: u32,
    command: &TraceCommand,
    route_events: Option<&[TraceRouteConstructionEvent]>,
    consumed_route_ordinals: &mut BTreeSet<u64>,
    entity_map: &EntityMap,
    retained_sector_kinds: &[LegacyGridSectorAsset],
) -> Option<ReplayGroupMoveResolution> {
    if schema != 16 {
        return None;
    }
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
        .unwrap_or_default()
        .iter()
        .filter_map(|event| {
            let ordinal = match event
                .draft_diagnostics
                .get("ordinal")
                .map(TraceJsonValue::tree)
            {
                Some(TraceJsonTree::Unsigned(ordinal)) => ordinal,
                other => panic!("schema-16 route event lacks an unsigned ordinal: {other:?}"),
            };
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
    // topology still authoritatively identifies whether PerformGroupMove's
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
    // `PerformGroupMove` calls `PerformMove` once for each selected actor, and
    // that reaches at most one `AppendMoveToSequence` route construction
    // (`original-code/RHengine.cpp:5441-5484`, `RHsequence.cpp:339-452`). A
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
/// destination. Schema 16 does not yet carry that goal on `drop_ale_at`, but a
/// cross-sector Seek records it in the same frame's route-construction stream.
/// Match by route ordinal plus stable actor/point identity; projected point
/// containment is intentionally not consulted because overlapping floors can
/// select a different layer. Same-sector seeks publish no route event, so the
/// caller also supplies the actor's exact current sector identity as a fallback.
// TODO(parity-schema): record DropAle's pre-authorization goal sector/layer on
// the command itself so replay does not have to infer same-sector destinations.
fn resolve_schema_sixteen_drop_ale(
    schema: u32,
    command: &TraceCommand,
    route_events: Option<&[TraceRouteConstructionEvent]>,
    consumed_route_ordinals: &mut BTreeSet<u64>,
    entity_map: &EntityMap,
    same_sector_goal: Option<ReplayDropAleResolution>,
) -> Option<ReplayDropAleResolution> {
    if schema != 16 {
        return None;
    }
    let TraceCommand::DropAleAt { actor, target, .. } = command else {
        return None;
    };

    let actor = entity_map.translate(*actor);
    let matching_event = route_events
        .unwrap_or_default()
        .iter()
        .filter_map(|event| {
            let ordinal = match event
                .draft_diagnostics
                .get("ordinal")
                .map(TraceJsonValue::tree)
            {
                Some(TraceJsonTree::Unsigned(ordinal)) => ordinal,
                other => panic!("schema-16 route event lacks an unsigned ordinal: {other:?}"),
            };
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
        .min_by_key(|(ordinal, _)| *ordinal);
    let Some((ordinal, event)) = matching_event else {
        // A cross-sector DropAle must have an authoritative route event. Do
        // not disguise a mismatched/corrupt event as a same-sector command.
        let has_actor_route = route_events.unwrap_or_default().iter().any(|event| {
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
    Some(ReplayDropAleResolution {
        goal: (goal_sector, event.goal_level),
        goal_sector_index: Some(goal_sector_index),
    })
}

fn schema_sixteen_drop_ale_actor_goal(
    schema: u32,
    command: &TraceCommand,
    entity_map: &EntityMap,
    engine: &Engine,
) -> Option<ReplayDropAleResolution> {
    if schema != 16 {
        return None;
    }
    let TraceCommand::DropAleAt { actor, .. } = command else {
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
    })
}

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(deny_unknown_fields)]
struct TracePointBox {
    top_left: TracePoint,
    bottom_right: TracePoint,
}

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
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

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
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

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
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

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(deny_unknown_fields)]
struct TraceAiForecastResolved {
    position: TracePoint,
    sector: u16,
    level: u16,
    #[serde(default)]
    direction: Option<u16>,
}

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
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

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(deny_unknown_fields)]
struct TraceAlertEligibility {
    rank: bool,
    able_to_help: Option<bool>,
    #[serde(alias = "stay_on_post")]
    allowed_to_leave_post: Option<bool>,
    can_call: Option<bool>,
    max_radius: Option<bool>,
    squared_radius: Option<bool>,
    capacity: Option<bool>,
    think: Option<bool>,
}

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
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

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(deny_unknown_fields)]
struct TraceGoToSource {
    point: TracePoint,
    layer: u16,
}

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(deny_unknown_fields)]
struct TraceGoToDestination {
    point: TracePoint,
    sector: Option<u16>,
    layer: u16,
}

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
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

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
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

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
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

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
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

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
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

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(deny_unknown_fields)]
struct TraceFrame {
    #[serde(rename = "type")]
    record_type: String,
    frame_before: u64,
    frame_after: u64,
    game_code: i32,
    simulation_body_ran: bool,
    commands: Vec<TraceCommand>,
    director_completions: Vec<robin_engine::engine::DirectorCompletion>,
    #[serde(default)]
    campaign: Option<TraceCampaign>,
    #[serde(default)]
    engine_state: Option<TraceEngineState>,
    selected_pcs: Vec<TraceEntityId>,
    elements: Vec<TraceElement>,
    visibility_queries: Vec<TraceVisibilityQuery>,
    rng_draws: TraceRngBatch,
    motion_line_changes: Vec<TraceMotionLineChange>,
    path_events: Vec<TracePathEvent>,
    /// Present in schema 15. Retained and printed on divergence; Rust has no
    /// side-effect-free route-construction event capture yet.
    /// TODO(parity-schema15): compare once the sequence builders publish the
    /// same source/goal and ordered gate list.
    #[serde(default)]
    route_construction_events: Option<Vec<TraceRouteConstructionEvent>>,
    #[serde(default)]
    popup_events: Option<Vec<TracePopupEvent>>,
    #[serde(default)]
    ai_forecast_events: Option<Vec<TraceAiForecastEvent>>,
    #[serde(default)]
    alert_formation_events: Option<Vec<TraceAlertFormationEvent>>,
    #[serde(default)]
    goto_authorization_events: Option<Vec<TraceGoToAuthorizationEvent>>,
    #[serde(default)]
    strike_proposal_events: Option<Vec<TraceStrikeProposalEvent>>,
    #[serde(default)]
    sequence_lifecycle_events: Option<Vec<TraceSequenceLifecycleEvent>>,
    #[serde(default)]
    target_lifecycle_events: Option<Vec<TraceTargetLifecycleEvent>>,
    resolved_exclamations: Vec<TraceResolvedExclamation>,
    #[serde(default)]
    movement_steps: Vec<TraceMovementStep>,
    #[serde(default)]
    flight_steps: Vec<TraceFlightStep>,
}

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(deny_unknown_fields)]
struct TraceEngineState {
    cheat_used_flags: u32,
    next_creation_order: u32,
    chorus_timer: u16,
    force_check: bool,
    men_to_blazon_conversion: bool,
    #[serde(default)]
    game_ui: Option<TraceJsonValue>,
    #[serde(default)]
    messenger_controller: Option<TraceJsonValue>,
    #[serde(default)]
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
    #[serde(default)]
    sound_completion_frontier: Option<TraceJsonValue>,
    ai_global: TraceJsonValue,
    engine_runtime_roots: TraceJsonValue,
    world_interactables: TraceJsonValue,
    repulsive_points: TraceJsonValue,
    titbit_manager: TraceJsonValue,
    failed_path_requests: Vec<TraceFailedPathRequest>,
}

/// Recursive JSON tree used for high-volume authoritative snapshots.
///
/// `serde_json::Value` deliberately has no native binary-codec derives. This
/// equivalent tree keeps schema-13 frame parsing strict; it is the serde
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
#[derive(
    Clone, Debug, PartialEq, bincode::Decode, bincode::Encode, bitcode::Decode, bitcode::Encode,
)]
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
#[derive(
    Clone, Debug, PartialEq, bincode::Decode, bincode::Encode, bitcode::Decode, bitcode::Encode,
)]
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

fn print_schema_sixteen_events<T: Serialize>(label: &str, events: Option<&Vec<T>>) {
    if let Some(events) = events
        && !events.is_empty()
    {
        eprintln!(
            "  Original schema-16 {label} this frame: {}",
            serde_json::to_string(events).expect("serialize schema-16 event diagnostics")
        );
    }
}

fn print_schema_sixteen_actor_diagnostics(elements: &[TraceElement]) {
    let diagnostics = elements
        .iter()
        .filter_map(|element| {
            let actor = element.actor.as_ref()?;
            let sequence = actor.sequence_element.as_ref().and_then(Option::as_ref);
            (actor.position_interface.is_some()
                || sequence.is_some_and(|sequence| {
                    sequence.following.is_some()
                        || sequence.postponed.is_some()
                        || sequence.current_order.is_some()
                        || sequence.movement_payload.is_some()
                }))
            .then(|| {
                serde_json::json!({
                    "entity": element.entity_id,
                    "creation_order": element.creation_order,
                    "position_interface": actor.position_interface,
                    "following": sequence.and_then(|value| value.following.as_ref()),
                    "postponed": sequence.and_then(|value| value.postponed.as_ref()),
                    "current_order": sequence.and_then(|value| value.current_order.as_ref()),
                    "movement_payload": sequence.and_then(|value| value.movement_payload.as_ref()),
                })
            })
        })
        .take(40)
        .collect::<Vec<_>>();
    if !diagnostics.is_empty() {
        eprintln!(
            "  Original schema-16 actor diagnostics (up to 40): {}",
            serde_json::to_string(&diagnostics).expect("serialize schema-16 actor diagnostics")
        );
    }
}

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(deny_unknown_fields)]
struct TraceFailedPathRequest {
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

fn trace_frame_envelope_matches(schema: u32, frame: &TraceFrame) -> bool {
    match schema {
        12 | 14 => {
            frame.campaign.is_none()
                && frame.engine_state.is_none()
                && frame.route_construction_events.is_none()
                && frame.popup_events.is_none()
                && frame.ai_forecast_events.is_none()
                && frame.alert_formation_events.is_none()
                && frame.goto_authorization_events.is_none()
                && frame.strike_proposal_events.is_none()
                && frame.sequence_lifecycle_events.is_none()
                && frame.target_lifecycle_events.is_none()
        }
        15 => {
            frame.campaign.is_none()
                && frame.engine_state.is_none()
                && frame.route_construction_events.is_some()
                && frame.popup_events.is_none()
                && frame.ai_forecast_events.is_none()
                && frame.alert_formation_events.is_none()
                && frame.goto_authorization_events.is_none()
                && frame.strike_proposal_events.is_none()
                && frame.sequence_lifecycle_events.is_none()
                && frame.target_lifecycle_events.is_none()
        }
        16 => {
            frame.campaign.is_none()
                && frame.engine_state.is_none()
                && frame.route_construction_events.is_some()
                && frame.popup_events.is_some()
                && frame.ai_forecast_events.is_some()
                && frame.alert_formation_events.is_some()
                && frame.goto_authorization_events.is_some()
        }
        13 => {
            frame.campaign.is_some()
                && frame.engine_state.is_some()
                && frame.route_construction_events.is_none()
                && frame.popup_events.is_none()
                && frame.ai_forecast_events.is_none()
                && frame.alert_formation_events.is_none()
                && frame.goto_authorization_events.is_none()
                && frame.strike_proposal_events.is_none()
                && frame.sequence_lifecycle_events.is_none()
                && frame.target_lifecycle_events.is_none()
        }
        _ => unreachable!("trace schema was validated before frame parsing"),
    }
}

fn validate_jump_line_shapes(frame: &TraceFrame) {
    for element in &frame.elements {
        let Some(human) = element.human.as_ref() else {
            continue;
        };
        validate_human_jump_line_shape(&element.entity_id, human);
    }
}

fn validate_human_jump_line_shape(entity: &TraceEntityId, human: &TraceHuman) {
    let Some(jump_lines) = human.opponent_jump_lines.as_ref() else {
        return;
    };
    let opponents = human.opponents.as_ref().unwrap_or_else(|| {
        panic!(
            "schema-16 {entity:?} records opponent jump lines without the parallel opponent list"
        )
    });
    assert_eq!(
        opponents.len(),
        jump_lines.len(),
        "schema-16 {entity:?} opponent and jump-line arrays differ in length"
    );
}

fn validate_sequence_diagnostic_order(frame: &TraceFrame) {
    let proposals = frame.strike_proposal_events.as_deref().unwrap_or_default();
    let lifecycle = frame
        .sequence_lifecycle_events
        .as_deref()
        .unwrap_or_default();
    for (expected, event) in proposals.iter().enumerate() {
        assert_eq!(
            event.ordinal, expected as u64,
            "schema-16 strike-proposal event ordinals are not contiguous"
        );
    }
    for (expected, event) in lifecycle.iter().enumerate() {
        assert_eq!(
            event.ordinal, expected as u64,
            "schema-16 sequence-lifecycle event ordinals are not contiguous"
        );
    }
    for (expected, event) in frame
        .target_lifecycle_events
        .as_deref()
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        assert_eq!(
            event.ordinal, expected as u64,
            "schema-16 target-lifecycle event ordinals are not contiguous"
        );
    }

    let mut frame_ordinals = proposals
        .iter()
        .map(|event| event.frame_ordinal)
        .chain(lifecycle.iter().map(|event| event.frame_ordinal))
        .chain(
            frame
                .target_lifecycle_events
                .as_deref()
                .unwrap_or_default()
                .iter()
                .filter_map(|event| event.frame_ordinal),
        )
        .collect::<Vec<_>>();
    frame_ordinals.sort_unstable();
    assert_eq!(
        frame_ordinals,
        (0..frame_ordinals.len() as u64).collect::<Vec<_>>(),
        "schema-16 strike/sequence/target frame ordinals are not a single contiguous timeline"
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

fn validate_trace_frame_envelope(schema: u32, frame: &TraceFrame) {
    validate_jump_line_shapes(frame);
    validate_sequence_diagnostic_order(frame);
    match schema {
        12 | 14 | 15 | 16 => assert!(
            trace_frame_envelope_matches(schema, frame),
            "schema-{schema} frame unexpectedly contains schema-13 authoritative state"
        ),
        13 => assert!(
            trace_frame_envelope_matches(schema, frame),
            "schema-13 frame is missing campaign or engine_state"
        ),
        _ => unreachable!("trace schema was validated before frame parsing"),
    }
}

const TRACE_NATIVE_VERSION: u32 = 66;
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
const TRACE_REBLOCK_SOURCE_SUFFIX: &str = ".parity-reblock-source-v66";
const TRACE_REBLOCK_BINDING_SUFFIX: &str = ".parity-reblock-binding-v66.json";
const TRACE_NATIVE_FOOTER_MAGIC: [u8; 16] = *b"RHPRTRACEFOOTER!";
const TRACE_NATIVE_FOOTER_LEN: u64 = 16 + 4 + 8 + 8;
// Full-session JSONL recordings are compressed as a single zstd frame. Some
// encoders select a frame window from the total uncompressed size, so long
// recordings legitimately exceed zstd's conservative 128 MiB decoder default.
// Keep the reader bounded at zstd's platform maximum while accepting those
// valid trace frames.
const TRACE_ZSTD_WINDOW_LOG_MAX: u32 = if usize::BITS >= 64 { 31 } else { 30 };
// Bitcode's dense output makes level 9 a good throughput/size tradeoff. A
// representative min/median/max corpus benchmark made it 12-17x faster than
// level 19 for only 16-19% larger native traces. Long-distance matching was
// neutral at level 19 and made level 9 both slower and 5-8% larger, so the
// native writer deliberately leaves it disabled.
const TRACE_NATIVE_ZSTD_LEVEL: i32 = 9;
const TRACE_NATIVE_LONG_DISTANCE_MATCHING: bool = false;
/// Frames per on-disk block. A schema-16 frame contains a complete Original
/// state envelope, so decoding 1024 at once expanded a roughly 100 MiB
/// bitcode record into 2-3.6 GiB of live Rust allocations. Sixteen frames
/// retain bitcode's columnar packing while bounding the decoded block to a
/// small fraction of one replay engine. Readers accept any block size, so
/// this storage-only change remains version-66 compatible.
const TRACE_NATIVE_BLOCK_RECORDS: usize = 16;
/// Bound the zstd history retained by every replay process. Cross-frame
/// repetition is already captured inside the 16-frame bitcode blocks; a
/// whole-trace 512 MiB-2 GiB window only traded resident memory for a small
/// artifact-size win and prevented one replay lane per CPU core.
const TRACE_NATIVE_WINDOW_LOG: u32 = 25;

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
struct BinaryTraceHeader {
    version: u32,
    source_fingerprint: String,
    trace: TraceHeader,
    rng_prefix: TraceRngPrefix,
}

#[derive(
    Debug,
    Deserialize,
    Serialize,
    bincode::Encode,
    bincode::Decode,
    bitcode::Encode,
    bitcode::Decode,
)]
enum BinaryTraceRecord {
    Frame(TraceFrame),
    End {
        rng_suffix: Option<TraceRngBatch>,
        final_frame: Option<u64>,
        frame_count: Option<u64>,
    },
}

struct BinaryTraceReader {
    path: PathBuf,
    reader: Box<dyn Read>,
    footer: BinaryTraceFooter,
    /// Records of the current block not yet handed out by [`Self::read_record`].
    pending: VecDeque<BinaryTraceRecord>,
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
/// mission-success/interruption snapshot after `PerformHourglass` returns
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

/// Recover the quit-mission message omitted by the current Original trace
/// schema for the retained-frame mission-success envelope.
///
/// `RHEngine::PerformHourglass` can return `RHGAME_LEVEL_SUCCEEDED` without
/// advancing the universal clock only from its leading `mbQuitWon` branch.
/// The UI sets that flag by forwarding `MSG_QUIT_MISSION`, but
/// `RHParity::RecordInputMessage` currently does not record that simple
/// message. Keep the inference restricted to the exact envelope that proves
/// this path; ordinary advancing frames and other terminal codes remain fully
/// authoritative.
// TODO: Record MSG_QUIT_MISSION in Original RHParity, append a matching
// TraceCommand variant under a bumped schema, and remove this schema-16
// legacy-envelope repair after affected corpora have been recaptured.
fn is_legacy_retained_terminal_success(
    schema: u32,
    frame_before: u64,
    frame_after: u64,
    simulation_body_ran: bool,
    game_code: i32,
) -> bool {
    schema == 16
        && frame_before == frame_after
        && !simulation_body_ran
        && game_code == GameCode::LevelSucceeded as i32
}

#[allow(clippy::too_many_arguments)]
fn append_legacy_retained_terminal_success_repair(
    commands: &mut Vec<PlayerCommand>,
    difficulty: robin_engine::player_profile::DifficultyLevel,
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

    // Original MSG_QUIT_MISSION synchronously runs the complete QuitMission
    // campaign/stat rollup and only then arms mbQuitWon for the next retained
    // Hourglass. Preserve that ordering through the existing deterministic
    // command path.
    *already_applied = true;
    commands.push(PlayerCommand::ApplyQuitMissionUpdates {
        exit_code: GameCode::LevelSucceeded,
        difficulty,
    });
    commands.push(PlayerCommand::QuitMissionRequested);
    true
}

fn cross_post_initialize_frame(
    engine: &mut Engine,
    display: &mut HostDisplayState,
    input: &mut InputState,
    assets: &LevelAssets,
    dev: &mut DevState,
) {
    engine
        .advance_frame(
            display,
            input,
            assets,
            dev,
            robin_engine::engine::SimulationFrameInput::no_hourglass().with_post_initialize(true),
        )
        .unwrap_or_else(|error| panic!("admit Original PostInitialize boundary: {error}"));
}

struct Options {
    scan_all: bool,
    no_auto_dump: bool,
    visual: bool,
    trace_path: PathBuf,
    dump: Option<DumpOptions>,
    http_server: Option<u16>,
    start_paused: bool,
    frame_zero_screenshot_dir: Option<PathBuf>,
    bench_encodings: bool,
    convert: bool,
    reblock: bool,
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

struct ActiveHttpStep {
    request: robin_rs::http_server::PendingStep,
    direction: &'static str,
    from_frame: u32,
    remaining: u32,
    requested: u32,
}

struct VisualReplay {
    window: robin_rs::window::GameWindow,
    renderer: Renderer,
    host: Host,
    sprite_images: BTreeMap<u32, (GpuImage, u16, u16)>,
}

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
            .selected_pc_ids()
            .first()
            .and_then(|id| engine.get_entity(*id))
            .or_else(|| {
                engine
                    .entities_with_ids_iter()
                    .find_map(|(_, entity)| entity.is_human().then_some(entity))
            })
            .map(|entity| entity.element_data().position_map())
            .unwrap_or(MapPoint::ZERO);
        self.host.viewport.view_position =
            MapPoint::new((focus.x - 512.0).max(0.0), (focus.y - 319.0).max(0.0));
        self.host.viewport.zoom_factor = 1.0;
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
                let width = self.host.frame_holder.sprite_width(bank_id);
                let height = self.host.frame_holder.sprite_height(bank_id);
                if width == 0 || height == 0 {
                    continue;
                }
                let rgba = if let Some(rgba) = self.host.frame_holder.rgba_data(bank_id) {
                    rgba.to_vec()
                } else {
                    let mut pixels = vec![0_u16; usize::from(width) * usize::from(height)];
                    self.host.frame_holder.uncompress_frame(
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
                sprite_x - self.host.viewport.view_position.x,
                sprite_y - self.host.viewport.view_position.y,
                sprite_x - self.host.viewport.view_position.x + f32::from(*width),
                sprite_y - self.host.viewport.view_position.y + f32::from(*height),
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

fn main() {
    let options = parse_options();
    if options.reblock {
        reblock_native_trace(&options.trace_path);
        return;
    }
    if options.validate_native {
        validate_native_trace(&options.trace_path);
        return;
    }
    if options.convert {
        convert_recording_to_native(&options.trace_path);
        return;
    }
    if options.bench_encodings {
        bench_trace_encodings(&options.trace_path);
        return;
    }
    if options.frame_zero_screenshot_dir.is_some() {
        let exit = robin_rs::window::run_with_game_visibility(
            "Robin Hood — Original parity frame-zero capture",
            1024,
            768,
            options.visual,
            move |mut window| async move {
                capture_full_frame_zero_screenshot(options, &mut window).await
            },
        )
        .unwrap_or_else(|error| panic!("start frame-zero parity capture: {error}"));
        std::process::exit(exit);
    }
    tracing_subscriber::fmt::init();
    if options.visual {
        let visible = options.visual;
        let exit = robin_rs::window::run_with_game_visibility(
            "Robin Hood — Original parity replay",
            1024,
            768,
            visible,
            move |window| async move { run_replay(options, Some(window)) },
        )
        .unwrap_or_else(|error| panic!("start visual parity replay: {error}"));
        std::process::exit(exit);
    }
    std::process::exit(run_replay(options, None));
}

async fn capture_full_frame_zero_screenshot(
    options: Options,
    window: &mut robin_rs::window::GameWindow,
) -> i32 {
    let invocation_dir =
        std::env::current_dir().expect("resolve invocation directory for frame-zero screenshot");
    let trace_path = canonicalize_trace_identity(&options.trace_path);
    let output_dir = options
        .frame_zero_screenshot_dir
        .expect("frame-zero capture lost its output directory");
    let output_dir = if output_dir.is_absolute() {
        output_dir
    } else {
        invocation_dir.join(output_dir)
    };
    let output_path = frame_zero_screenshot_path(&output_dir, &trace_path);

    let native_path = ensure_native_binary_trace(&trace_path);
    let header = read_binary_trace_header(&native_path).trace;
    validate_trace_header(&header);
    let initial_save = decode_and_validate_initial_save(&header);

    let (_launcher_campaign, profiles, application_context) = robin_rs::main_entry::rust_init()
        .unwrap_or_else(|error| panic!("initialize game: {error}"));
    let campaign = restore_campaign(&header.campaign, &profiles);
    let mut game_args = robin_rs::main_entry::try_parse_cli_from([
        "original_parity_replay",
        "--mission",
        header.mission.as_str(),
        "--proto",
        header.proto_level.as_str(),
        "--no-sound",
        "--http-server=0",
        "--rollback-check=false",
    ])
    .unwrap_or_else(|error| panic!("construct frame-zero game arguments: {error}"));
    game_args.mission_start_map_output = Some(output_path.clone());
    game_args.mission_start_map_frame = 0;
    game_args.mission_start_viewport_capture = true;
    game_args.mission_start_legacy_save = initial_save;
    game_args.preserve_forced_mission_campaign = true;
    game_args.fast_forward = true;

    match robin_rs::main_entry::run_rust_game(
        window,
        campaign,
        profiles,
        application_context,
        &game_args,
    )
    .await
    {
        Ok(_) => {
            eprintln!(
                "full frame-zero parity screenshot written to {}",
                output_path.display()
            );
            0
        }
        Err(error) => {
            eprintln!("frame-zero parity screenshot failed: {error}");
            1
        }
    }
}

fn run_replay(options: Options, visual_window: Option<robin_rs::window::GameWindow>) -> i32 {
    let replay_started = Instant::now();
    let scan_all = options.scan_all;
    let no_auto_dump = options.no_auto_dump;
    let trace_path = options.trace_path;
    let http_server = options.http_server;
    let mut manual_pause = options.start_paused;
    let trace_path = canonicalize_trace_identity(&trace_path);
    let native_path = ensure_native_binary_trace(&trace_path);
    let mut dump = options.dump.map(|options| {
        let file = File::create(&options.path)
            .unwrap_or_else(|e| panic!("create diagnostic dump {}: {e}", options.path.display()));
        (options, BufWriter::new(file))
    });
    let cached_header = read_binary_trace_header(&native_path);
    // Normally grow the replay RNG one frame at a time so loading the trace
    // does not decode every large frame twice. Zero-prefix loaded saves need
    // future draws during deterministic reconstruction. The environment
    // override keeps an exact full-preload A/B path for diagnosing either
    // strategy without another build.
    let preload_complete_rng_stream = should_preload_complete_rng_stream(
        cached_header.trace.start_state,
        cached_header.rng_prefix.draws.gameplay_draw_count(),
        std::env::var_os("PARITY_PRELOAD_RNG").is_some(),
    );
    let initial_rng_draws = if preload_complete_rng_stream {
        read_all_rng_draws(&native_path)
    } else {
        simulation_rng_draws(&cached_header.rng_prefix.draws)
    };
    let header = cached_header.trace;
    validate_trace_header(&header);
    validate_trace_start(
        header.start_state,
        header.session_index,
        header.initial_frame,
    );
    let initial_save = decode_and_validate_initial_save(&header);

    if let Ok(dir) = std::env::var("ROBINHOOD_DATA_DIR") {
        std::env::set_current_dir(&dir).expect("chdir to ROBINHOOD_DATA_DIR");
    }
    robin_rs::main_entry::register_language_data_paths_for_tool();

    let mut records = BinaryTraceReader::open(&native_path);
    let stream_header = records.read_header();
    assert_eq!(stream_header.trace.schema, header.schema);
    assert_eq!(stream_header.trace.session_index, header.session_index);
    assert_eq!(stream_header.trace.initial_frame, header.initial_frame);
    assert_eq!(stream_header.trace.mission, header.mission);
    assert_eq!(stream_header.trace.rng_seed, header.rng_seed);
    assert_eq!(
        stream_header.trace.synchronous_pathfinding,
        header.synchronous_pathfinding
    );
    assert!(
        header.synchronous_pathfinding,
        "trace was recorded with asynchronous pathfinding"
    );

    let prefix = stream_header.rng_prefix;
    assert_eq!(prefix.r#type, "rng_prefix");
    assert_eq!(prefix.draws.first_index, 0);
    let prefix_end = prefix.draws.gameplay_draw_count();
    if let Some((options, writer)) = &mut dump {
        write_jsonl_record(
            writer,
            &serde_json::json!({
                "schema": "robin-parity-engine-dump.v1",
                "type": "header",
                "source_trace": trace_path,
                "mission": header.mission,
                "rng_seed": header.rng_seed,
                "frame_range": {
                    "from": options.from_frame,
                    "through": (options.through_frame != u64::MAX)
                        .then_some(options.through_frame),
                },
                "entity_filter": options.entities,
            }),
        );
    }

    assert!(
        initial_rng_draws.len() >= prefix_end,
        "loaded simulation RNG stream is shorter than prefix"
    );
    let rewind_loaded_save_rng =
        header.start_state == TraceStartState::LoadedSave && prefix_end == 0;
    let (mut engine, assets, mut host, background, mission_scb, menu_text) =
        initialize_engine(&header, initial_rng_draws.clone());
    let mut loaded_save_host = None;
    if let Some(initial_save) = initial_save {
        let save = robin_engine::legacy_save::initialized::decode_initialized_v48_save(
            initial_save,
            format!("{}#initial_save", trace_path.display()),
            &engine,
            &assets,
            &mission_scb,
            &robin_engine::legacy_save::body::LegacySaveBodyLimits::default(),
        )
        .unwrap_or_else(|error| panic!("decode schema-12 initial_save body: {error}"));
        eprintln!(
            "decoded schema-12 Original save through byte {} ({} elements, {} dynamic, {} pending paths, {} failed paths)",
            save.end_offset,
            save.element_envelope.records.len(),
            save.element_envelope
                .records
                .iter()
                .filter(|record| matches!(
                    record.resolution,
                    robin_engine::legacy_save::elements::LegacyElementResolution::ConstructDynamic {
                        ..
                    }
                ))
                .count(),
            save.tail.pathfinder.requests.len(),
            save.failed_path_requests.requests.len(),
        );
        if std::env::var_os("PARITY_DEBUG_STAGE_TIMING").is_some() {
            for (index, request) in save.tail.pathfinder.requests.iter().enumerate() {
                eprintln!("parity stage: saved pending path {index}: {request:?}");
            }
            for (index, request) in save.failed_path_requests.requests.iter().enumerate() {
                eprintln!("parity stage: saved failed path {index}: {request:?}");
            }
        }
        loaded_save_host = Some(
            robin_engine::legacy_save::adopt_engine::adopt_known_linux_v48_replay(
                &mut engine,
                &assets,
                &save,
            )
            .unwrap_or_else(|error| panic!("adopt schema-12 initial_save body: {error}")),
        );
        eprintln!("atomically adopted schema-12 Original Linux-v48 save");
    }
    let restored_dormant_macros = apply_legacy_interactive_chain_macro_fallback(
        &trace_path,
        &header,
        prefix_end,
        &mut engine,
        &assets,
    );
    if restored_dormant_macros != 0 {
        eprintln!(
            "restored {restored_dormant_macros} dormant waypoint-macro cursors from the authoritative preceding interactive-session terminal snapshot"
        );
    }
    if let Some(transients) = header.initial_npc_transients.as_deref() {
        apply_initial_npc_transients(&mut engine, transients);
        eprintln!(
            "restored {} explicit schema-16 NPC session-boundary transients",
            transients.len()
        );
    } else if header.schema == 16
        && header.start_state == TraceStartState::LoadedSave
        && header.session_index > 1
        && legacy_loaded_save_retains_process_transients(prefix_end)
    {
        let restored = apply_legacy_segment_visibility_fallback(&mut engine);
        eprintln!(
            "warning: legacy schema-16 segment lacks initial_npc_transients; reconstructed maximal_visibility for {restored} dead/unconscious NPCs"
        );
    } else if header.schema == 16
        && header.start_state == TraceStartState::LoadedSave
        && header.session_index > 1
    {
        eprintln!(
            "warning: legacy schema-16 fresh-engine segment lacks initial_npc_transients; retained constructor-zero maximal_visibility"
        );
    }
    if rewind_loaded_save_rng {
        let setup_draws = engine
            .original_rng_replay_cursor()
            .expect("loaded-save reconstruction lost Original RNG replay");
        engine.replace_original_rng_replay(initial_rng_draws);
        eprintln!("rewound loaded-save RNG after {setup_draws} deterministic construction draws");
    }
    let mut motion_line_parity = MotionLineParity::build(&engine, &header.motion_grid);
    engine.set_external_director_completion_replay(true);
    let mut dev = DevState::new();
    let mut display = HostDisplayState::default();
    let mut input = InputState::default();
    let mut selected_view_element = None;
    if let Some(loaded) = &loaded_save_host {
        loaded.apply_display_to(&mut display);
        loaded.apply_display_to(&mut host.engine_display);
        selected_view_element = loaded.selected_view_element();
        host.selected_view_element = selected_view_element;
        assert!(
            loaded.trajectory_output().clear_preview,
            "Original loaded-save adoption must invalidate trajectory preview"
        );
        let _post_load = loaded.post_load_output();
        // This replay host and input state were constructed afresh, so all
        // explicit Original post-load transient clears already hold.
    }
    let mut visual =
        visual_window.map(|window| VisualReplay::new(window, host, &engine, background));
    assert_eq!(
        engine.original_rng_replay_cursor(),
        Some(prefix_end),
        "Rust mission setup consumed a different number of global RNG draws than the original"
    );
    if std::env::var_os("PARITY_DEBUG_RNG_PREFIX").is_some() {
        for (index, site) in engine
            .original_rng_replay_sites(0..prefix_end)
            .expect("original RNG site history unexpectedly disabled")
            .into_iter()
            .enumerate()
        {
            eprintln!("Rust startup RNG {index}: {site:?}");
        }
    }
    let mut entity_map: Option<EntityMap> = None;
    let mut divergent_frames = 0_u64;
    let mut first_by_field = BTreeMap::<String, (u64, String)>::new();
    let mut gameplay_rng_index = prefix_end;
    let mut active_http_step = None;
    let debug_stage_timing = std::env::var_os("PARITY_DEBUG_STAGE_TIMING").is_some();
    let automatic_dump_enabled = dump.is_none() && !scan_all && !no_auto_dump;
    let mut rolling_dump = VecDeque::<RollingDumpFrame>::new();

    if let Some(port) = http_server {
        robin_rs::http_server::start_global(port)
            .unwrap_or_else(|e| panic!("start parity replay HTTP server: {e}"));
        eprintln!(
            "parity replay HTTP server ready on http://127.0.0.1:{port} (frame {})",
            engine.frame_counter()
        );
    }

    // Original retains path events across the full boundary between frame
    // writes. PostInitialize, sound callbacks, and resolved input can enqueue
    // movement before the next PerformHourglass, so this capture deliberately
    // remains active outside the simulation body.
    // Keep both captures open across the complete recorded-frame boundary.
    // In particular, queued sound completions run before the simulation body
    // and may synchronously invoke authoritative AI visibility checks.
    robin_engine::sight_obstacle::begin_parity_visibility_capture();
    robin_engine::pathfinder::begin_parity_path_capture();

    // Correlate Original RNG callsite return-address offsets with the Rust
    // site labels observed at the same draw indices while the streams still
    // agree. On a cursor mismatch the accumulated map names the Original
    // callsites of the frame, which identifies the draw Rust skipped or
    // invented even when Rust never reached that site.
    let debug_rng_site_map = std::env::var_os("PARITY_DEBUG_RNG_SITE_MAP").is_some();
    let mut rng_site_map: BTreeMap<u32, BTreeSet<String>> = BTreeMap::new();
    let profile_timing = std::env::var_os("PARITY_PROFILE_TIMING").is_some();
    let mut simulation_time = Duration::ZERO;
    let mut comparison_time = Duration::ZERO;
    let mut rng_diagnostic_time = Duration::ZERO;

    let mut line_index = 0_usize;
    let mut trace_timeline = TraceTimeline::new(header.initial_frame);
    let mut legacy_terminal_success_repair_applied = false;
    let terminator = loop {
        let mut frame = match records.read_record() {
            BinaryTraceRecord::Frame(frame) => frame,
            end @ BinaryTraceRecord::End { .. } => break end,
        };
        assert_eq!(
            frame.record_type, "frame",
            "invalid parity frame record type"
        );
        validate_trace_frame_envelope(header.schema, &frame);
        if debug_stage_timing {
            eprintln!(
                "parity stage: loaded original frame {} -> {}",
                frame.frame_before, frame.frame_after
            );
        }
        trace_timeline
            .observe(frame.frame_before, frame.frame_after)
            .unwrap_or_else(|error| panic!("invalid parity frame timeline: {error}"));
        line_index += 1;
        let mut http_frame_commands = robin_engine::player_command::FrameCommands::new();
        if http_server.is_some() {
            loop {
                let drained = drain_headless_http(
                    &mut engine,
                    &mut display,
                    &assets,
                    &mut input,
                    &mut selected_view_element,
                    &mut manual_pause,
                    &mut active_http_step,
                );
                http_frame_commands.commands.extend(drained.commands);
                if !manual_pause || active_http_step.is_some() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        let rng_start = gameplay_rng_index;
        let rng_end = rng_start + frame.rng_draws.gameplay_draw_count();
        gameplay_rng_index = rng_end;
        if !preload_complete_rng_stream {
            engine.append_original_rng_replay(simulation_rng_draws(&frame.rng_draws));
        }
        let capture_commands = automatic_dump_enabled
            || dump
                .as_ref()
                .is_some_and(|(options, _)| options.includes(frame.frame_after));
        let resolved_commands = capture_commands.then(|| {
            serde_json::to_value(&frame.commands).expect("serialize resolved trace commands")
        });
        assert_eq!(
            engine.original_rng_replay_cursor(),
            Some(rng_start),
            "RNG cursor already diverged before original frame {}",
            frame.frame_before
        );
        if scan_all
            && (frame.frame_before.is_multiple_of(10) || (150..=180).contains(&frame.frame_before))
        {
            eprintln!("scanning original frame {}", frame.frame_before);
        }

        // Establish identity from the untouched mission-start state.  Besides
        // being the strongest isomorphism anchor, this lets startup debugging
        // distinguish load-time differences from first-hourglass mutations.
        let map = entity_map.get_or_insert_with(|| EntityMap::build(&engine, &assets, &frame));
        map.refresh_trace_indices(&frame);
        engine.set_original_impossible_action_done_deadlines(
            frame
                .strike_proposal_events
                .as_deref()
                .unwrap_or_default()
                .iter()
                .filter(|event| event.phase == "opponent_inputs")
                .map(|event| {
                    (
                        event.actor_creation_order,
                        event.principal_opponent_creation_order.unwrap_or_else(|| {
                            panic!(
                                "schema-16 frame {} opponent_inputs invocation {} lacks principal_opponent_creation_order",
                                frame.frame_before, event.invocation
                            )
                        }),
                        i16::try_from(event.time_limit.unwrap_or_else(|| {
                            panic!(
                                "schema-16 frame {} opponent_inputs invocation {} lacks time_limit",
                                frame.frame_before, event.invocation
                            )
                        }))
                        .unwrap_or_else(|_| {
                            panic!(
                                "Original strike deadline does not fit SWORD: {:?}",
                                event.time_limit
                            )
                        }),
                    )
                }),
        );
        if debug_stage_timing {
            eprintln!("parity stage: mapped original frame {}", frame.frame_before);
        }
        if frame.frame_before == 0 && std::env::var_os("PARITY_DEBUG_NPC_ORDER").is_some() {
            let reverse: BTreeMap<_, _> = map
                .entities
                .iter()
                .map(|(original, rust)| (*rust, *original))
                .collect();
            eprintln!(
                "Rust NPC iteration mapped to original IDs: {:?}",
                engine
                    .npc_ids()
                    .into_iter()
                    .map(|id| reverse[&id])
                    .collect::<Vec<_>>()
            );
        }
        let debug_startup =
            frame.frame_before == 0 && std::env::var_os("PARITY_DEBUG_STARTUP").is_some();
        if debug_startup {
            print_startup_actors("before Rust frame 1", &engine, &frame, map);
        }
        let mut external_facts = frame
            .director_completions
            .drain(..)
            .map(robin_engine::engine::ExternalFact::DirectorCompletion)
            .collect::<Vec<_>>();
        // `RHGame` records the engine frame before running the host sound
        // manager (`original-code/RHgame.cpp:1879-1915`). Consequently these
        // resolutions belong chronologically before the input commands stored
        // on this following frame record. Consume that sound-only boundary
        // first so a current-frame selection bark cannot jump ahead of an NPC
        // request created by the preceding boundary's SoundIsFinished callback.
        let resolutions = frame
            .resolved_exclamations
            .drain(..)
            .map(|resolved| {
                let _selection_diagnostics = (resolved.selected_variant, resolved.selected_entry);
                robin_engine::sound::ResolvedExclamation {
                    actor_id: map.translate(resolved.actor).index(),
                    identifier: resolved.identifier,
                    exclamation_id: resolved.exclamation_id,
                    duration_frames: resolved.duration_frames,
                }
            })
            .collect();
        external_facts.push(robin_engine::engine::ExternalFact::ReplaySoundBoundary(
            resolutions,
        ));
        let popup_nested_refresh = frame.popup_events.as_deref().is_some_and(|events| {
            events.iter().any(|event| {
                event.stage == "nested_refresh_entry" && event.remove_mouse == Some(true)
            })
        });
        let (commands_before_hourglass, commands_after_hourglass) = split_late_refresh_orientations(
            std::mem::take(&mut frame.commands),
            popup_nested_refresh,
        );
        let mut consumed_drop_ale_route_ordinals = BTreeSet::new();
        let mut consumed_group_move_route_ordinals = BTreeSet::new();
        let mut commands_before_hourglass_resolved = Vec::new();
        for command in commands_before_hourglass {
            if debug_stage_timing {
                eprintln!(
                    "parity stage: converting command before frame {}: {command:?}",
                    frame.frame_after
                );
            }
            let drop_ale_resolution = resolve_schema_sixteen_drop_ale(
                header.schema,
                &command,
                frame.route_construction_events.as_deref(),
                &mut consumed_drop_ale_route_ordinals,
                map,
                schema_sixteen_drop_ale_actor_goal(header.schema, &command, map, &engine),
            );
            let group_move_resolution = resolve_schema_sixteen_group_move_route(
                header.schema,
                &command,
                frame.route_construction_events.as_deref(),
                &mut consumed_group_move_route_ordinals,
                map,
                &assets
                    .legacy_grid_topology
                    .as_ref()
                    .expect("parity replay requires retained Original fast-grid topology")
                    .sectors,
            );
            if let Some(command) = command.into_player_command(
                map,
                &engine,
                drop_ale_resolution,
                group_move_resolution,
            ) {
                commands_before_hourglass_resolved.push(command);
            }
        }
        append_legacy_retained_terminal_success_repair(
            &mut commands_before_hourglass_resolved,
            header.sim_config.difficulty.into(),
            header.schema,
            frame.frame_before,
            frame.frame_after,
            frame.simulation_body_ran,
            frame.game_code,
            &mut legacy_terminal_success_repair_applied,
        );
        let commands_after_hourglass = commands_after_hourglass
            .into_iter()
            .filter_map(|command| {
                let drop_ale_resolution = resolve_schema_sixteen_drop_ale(
                    header.schema,
                    &command,
                    frame.route_construction_events.as_deref(),
                    &mut consumed_drop_ale_route_ordinals,
                    map,
                    schema_sixteen_drop_ale_actor_goal(header.schema, &command, map, &engine),
                );
                let group_move_resolution = resolve_schema_sixteen_group_move_route(
                    header.schema,
                    &command,
                    frame.route_construction_events.as_deref(),
                    &mut consumed_group_move_route_ordinals,
                    map,
                    &assets
                        .legacy_grid_topology
                        .as_ref()
                        .expect("parity replay requires retained Original fast-grid topology")
                        .sectors,
                );
                command.into_player_command(
                    map,
                    &engine,
                    drop_ale_resolution,
                    group_move_resolution,
                )
            })
            .collect::<Vec<_>>();
        if debug_stage_timing {
            eprintln!(
                "parity stage: entering Rust frame {} -> {}",
                frame.frame_before, frame.frame_after
            );
        }
        print_debug_element("before", &engine, &frame);
        robin_engine::movement_diagnostics::begin_parity_movement_capture();
        let simulation_started = Instant::now();
        let tick_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut pre_commands = http_frame_commands
                .commands
                .into_iter()
                .map(robin_engine::engine::SimCommand::from)
                .collect::<Vec<_>>();
            pre_commands.extend(
                commands_before_hourglass_resolved
                    .into_iter()
                    .map(robin_engine::engine::SimCommand::from),
            );
            let frame_input = robin_engine::engine::SimulationFrameInput::new(pre_commands)
                .with_external_facts(external_facts)
                .with_post_commands(
                    commands_after_hourglass
                        .into_iter()
                        .map(robin_engine::engine::SimCommand::from)
                        .collect(),
                )
                .with_simulation_body_allowed(frame.simulation_body_ran);
            engine
                .advance_frame(&mut display, &mut input, &assets, &mut dev, frame_input)
                .unwrap_or_else(|error| {
                    panic!("admit original frame {}: {error}", frame.frame_before)
                })
                .events
                .into_side_effects()
        }));
        if profile_timing {
            simulation_time += simulation_started.elapsed();
        }
        let actual_visibility_queries =
            robin_engine::sight_obstacle::take_parity_visibility_capture();
        // Restart immediately so post-frame work and the next frame's
        // director/input/sound prefix are attributed to that next frame,
        // matching the Original recorder's frame envelope.
        robin_engine::sight_obstacle::begin_parity_visibility_capture();
        let actual_movement_steps =
            robin_engine::movement_diagnostics::take_parity_movement_capture();
        let actual_flight_steps = robin_engine::movement_diagnostics::take_parity_flight_capture();
        let actual_move_box_extractions =
            robin_engine::movement_diagnostics::take_parity_move_box_extractions();
        let late_movement_retranslations =
            robin_engine::movement_diagnostics::take_parity_late_movement_retranslations();
        let actual_path_events = robin_engine::pathfinder::take_parity_path_capture();
        // Restart immediately: the post-frame comparison and one-shot
        // PostInitialize below precede the next recorded frame boundary.
        robin_engine::pathfinder::begin_parity_path_capture();
        let tick_effects = tick_result.unwrap_or_else(|payload| {
            eprintln!(
                "Rust simulation panicked while replaying original frame {} -> {}",
                frame.frame_before, frame.frame_after
            );
            std::panic::resume_unwind(payload);
        });
        if debug_stage_timing {
            eprintln!("parity stage: completed Rust frame {}", frame.frame_after);
        }
        print_debug_element("after", &engine, &frame);
        map.extend_runtime_entities(&engine, &frame);
        record_arrow_publication_before_compare(&engine, &frame, map);
        if debug_stage_timing {
            eprintln!(
                "parity stage: extended runtime identity through frame {}",
                frame.frame_after
            );
        }
        if let Some(visual) = &mut visual
            && !visual.render(&engine)
        {
            eprintln!(
                "visual parity replay closed by user at frame {}",
                engine.frame_counter()
            );
            return 0;
        }
        let actual_rng_end = engine
            .original_rng_replay_cursor()
            .expect("original RNG replay unexpectedly disabled");
        if let Ok(original_index) = std::env::var("PARITY_DEBUG_SOLDIER").map(|value| {
            value
                .parse::<u32>()
                .expect("PARITY_DEBUG_SOLDIER must be a u32")
        }) && frame.frame_after
            <= std::env::var("PARITY_DEBUG_UNTIL")
                .map(|value| {
                    value
                        .parse::<u64>()
                        .expect("PARITY_DEBUG_UNTIL must be a u64")
                })
                .unwrap_or(10)
        {
            let id = map.translate(TraceEntityId {
                kind: TraceEntityKind::Soldier,
                index: original_index,
            });
            let entity = engine.get_entity(id).expect("debug soldier exists");
            let sprite = &entity.element_data().sprite;
            let actor = entity.actor_data().expect("debug soldier 83 is an actor");
            let ai = entity.ai_controller().expect("debug soldier has AI");
            eprintln!(
                "rust frame {} soldier{} pos={:?} goal={:?} increment={:?} dir={:?}/{:?} command={:?} order={:?} last_action={:?} sprite_row={} sprite_frame={}/{} frame_distance={} sprite_motion={:?} execute_init={} last_order={:?} ai={:?}/{:?} chief={:?} patrol={:?} path={:?} history={:?}",
                frame.frame_after,
                original_index,
                entity.element_data().position_map(),
                sprite.position_iface.map_goal(),
                sprite
                    .position_iface
                    .is_increment_map_computed()
                    .then(|| sprite.position_iface.get_increment_map()),
                sprite.position_iface.get_direction(),
                sprite.position_iface.get_direction_goal(),
                engine.actor_command(id),
                engine.actor_order_type(id),
                sprite.last_action,
                sprite.current_row,
                sprite.current_frame,
                sprite.frame_count,
                sprite.current_frame_distance(),
                sprite.last_motion_state,
                actor.execute_order_initialising,
                actor.last_execute_order_id,
                ai.current_state,
                ai.current_substate,
                ai.patrol_chief,
                ai.patrol,
                ai.patrol_path.as_ref().map(|path| (
                    path.hiking_path_index,
                    path.current_waypoint_index,
                    path.last_waypoint_index,
                    path.forward,
                    path.current_waypoint(&assets.hiking_paths)
                )),
                ai.patrol_path.as_ref().map(|path| &path.history),
            );
            if frame.frame_after == 1
                && let Some(path) = ai.patrol_path.as_ref()
            {
                eprintln!(
                    "rust soldier{} route {:?}",
                    original_index,
                    assets.hiking_paths[usize::from(path.hiking_path_index)].waypoints
                );
            }
        }
        if debug_startup {
            print_startup_actors("after Rust frame 1", &engine, &frame, map);
        }

        let comparison_started = Instant::now();
        map.validate_building_sector_mapping(&engine, &frame);
        let mut differences =
            motion_line_parity.apply_changes_and_compare(&engine, &frame.motion_line_changes);
        differences.extend(compare_visibility_queries(
            &frame.visibility_queries,
            &actual_visibility_queries,
        ));
        differences.extend(compare_path_events(
            &frame.path_events,
            &actual_path_events,
            map,
        ));
        differences.extend(compare_frame(
            &engine,
            &assets,
            &menu_text,
            &frame,
            tick_effects.code as i32,
            map,
            &late_movement_retranslations,
        ));
        if profile_timing {
            comparison_time += comparison_started.elapsed();
        }
        if debug_stage_timing {
            eprintln!(
                "parity stage: compared Rust frame {} ({} differences)",
                frame.frame_after,
                differences.len()
            );
        }
        let rng_diagnostic_started = Instant::now();
        let rust_rng_sites = engine
            .original_rng_replay_sites(rng_start..actual_rng_end)
            .expect("original RNG site history unexpectedly disabled");
        let rust_rng_diagnostics = engine
            .original_rng_replay_diagnostics(rng_start..actual_rng_end)
            .expect("original RNG diagnostics unexpectedly disabled");
        if profile_timing {
            rng_diagnostic_time += rng_diagnostic_started.elapsed();
        }
        if let Some((options, writer)) = &mut dump
            && options.includes(frame.frame_after)
        {
            write_engine_dump_frame(
                writer,
                options,
                &engine,
                map,
                &frame,
                resolved_commands
                    .clone()
                    .expect("included diagnostic frame captured its resolved commands"),
                rng_start,
                rng_end,
                actual_rng_end,
                &rust_rng_sites,
                &rust_rng_diagnostics,
                &actual_path_events,
                &actual_visibility_queries,
                &frame.movement_steps,
                &actual_movement_steps,
                &frame.flight_steps,
                &actual_flight_steps,
                &actual_move_box_extractions,
                &differences,
            );
        }
        if automatic_dump_enabled {
            push_rolling_window(
                &mut rolling_dump,
                RollingDumpFrame {
                    engine: engine.diagnostic_snapshot_without_original_rng_replay(),
                    frame_before: frame.frame_before,
                    frame_after: frame.frame_after,
                    selected_pcs: frame.selected_pcs.clone(),
                    rng_draws: frame.rng_draws.clone(),
                    resolved_commands: resolved_commands
                        .expect("automatic diagnostic frame captured its resolved commands"),
                    original_path_events: frame.path_events.clone(),
                    rust_path_events: actual_path_events.clone(),
                    original_visibility_queries: frame.visibility_queries.clone(),
                    rust_visibility_queries: actual_visibility_queries.clone(),
                    original_movement_steps: frame.movement_steps.clone(),
                    rust_movement_steps: actual_movement_steps.clone(),
                    original_flight_steps: frame.flight_steps.clone(),
                    rust_flight_steps: actual_flight_steps.clone(),
                    rust_move_box_extractions: actual_move_box_extractions.clone(),
                    rng_start,
                    expected_rng_end: rng_end,
                    actual_rng_end,
                    rust_rng_sites: rust_rng_sites.clone(),
                    rust_rng_diagnostics: rust_rng_diagnostics.clone(),
                    differences: differences.clone(),
                },
            );
        }
        // Diagnostic: correlate the Original binary's per-draw callsite
        // offsets with the Rust `RngSite` names. Only meaningful while the
        // two streams still agree on the draw count for the frame, which is
        // exactly when the pairing is positional and unambiguous.
        if std::env::var_os("PARITY_DEBUG_RNG_SITE_MAP").is_some() {
            let offsets = frame.rng_draws.gameplay_callsite_offsets();
            eprintln!(
                "RNG_FRAME frame={} start={rng_start} original_offsets={offsets:?} rust_sites={rust_rng_sites:?}",
                frame.frame_before,
            );
            if actual_rng_end == rng_end && offsets.len() == rust_rng_sites.len() {
                for (offset, site) in offsets.iter().zip(rust_rng_sites.iter()) {
                    eprintln!("RNG_SITE_MAP offset={offset} site={site:?}");
                }
            }
        }
        // Preserve the complete divergent frame in --dump-jsonl before
        // stopping on an RNG cursor mismatch. RNG ordering failures are often
        // precisely where the broad engine snapshot is most useful.
        if debug_rng_site_map {
            let offsets = frame.rng_draws.gameplay_callsite_offsets();
            if actual_rng_end == rng_end && offsets.len() == rust_rng_sites.len() {
                for (offset, site) in offsets.iter().copied().zip(rust_rng_sites.iter()) {
                    rng_site_map
                        .entry(offset)
                        .or_default()
                        .insert(format!("{site:?}"));
                }
            } else {
                eprintln!(
                    "original callsite offsets for frame {}: {:?}",
                    frame.frame_before, offsets
                );
                for (index, offset) in offsets.iter().copied().enumerate() {
                    let known = rng_site_map
                        .get(&offset)
                        .map(|sites| sites.iter().cloned().collect::<Vec<_>>().join("|"))
                        .unwrap_or_else(|| "<unseen>".to_string());
                    eprintln!("  original draw {index}: offset {offset} -> {known}");
                }
                for (index, site) in rust_rng_sites.iter().enumerate() {
                    eprintln!("  rust draw {index}: {site:?}");
                }
                for difference in &differences {
                    eprintln!("  state difference: {difference}");
                }
                for (offset, sites) in &rng_site_map {
                    eprintln!(
                        "rng site map: {offset} {}",
                        sites.iter().cloned().collect::<Vec<_>>().join("|")
                    );
                }
            }
        }
        if actual_rng_end != rng_end {
            if automatic_dump_enabled {
                write_automatic_rolling_dump(
                    &rolling_dump,
                    &trace_path,
                    &header,
                    map,
                    frame.frame_after,
                );
            }
            panic!(
                "Rust consumed RNG draws {:?} at sites {rust_rng_sites:?} with script diagnostics {rust_rng_diagnostics:#?} during original frame {}; original ended at draw {rng_end}; Original simulation callsite offsets for the frame: {:?}",
                rng_start..actual_rng_end,
                frame.frame_before,
                frame.rng_draws.gameplay_callsite_offsets(),
            );
        }
        if !differences.is_empty() {
            if let Some(step) = active_http_step.take() {
                step.request.respond_err(format!(
                    "parity divergence after frame {}: {} differences",
                    frame.frame_after,
                    differences.len()
                ));
            }
            divergent_frames += 1;
            for difference in &differences {
                first_by_field
                    .entry(difference_field(difference).to_string())
                    .or_insert_with(|| (frame.frame_after, difference.clone()));
            }
            if scan_all && visual.is_none() {
                // RHGame records the authoritative frame immediately after
                // PerformHourglass, then runs the one-shot PostInitialize
                // hook after refresh/sound. Apply that boundary only after
                // comparing this frame, before advancing to the next one.
                cross_post_initialize_frame(
                    &mut engine,
                    &mut display,
                    &mut input,
                    &assets,
                    &mut dev,
                );
                continue;
            }
            let mut fields = BTreeMap::<&str, usize>::new();
            for difference in &differences {
                let field = difference_field(difference);
                *fields.entry(field).or_default() += 1;
            }
            eprintln!(
                "first parity divergence after frame {} ({} differences; showing up to 40):",
                frame.frame_after,
                differences.len()
            );
            eprintln!("  mismatch counts by logical field: {fields:?}");
            for (field, (_, example)) in &first_by_field {
                eprintln!("  first {field}: {example}");
            }
            for difference in differences.iter().take(40) {
                eprintln!("  {difference}");
            }
            if !frame.path_events.is_empty() || !actual_path_events.is_empty() {
                eprintln!(
                    "  Original path events this frame: {}",
                    serde_json::to_string(&frame.path_events)
                        .expect("serialize Original path-event diagnostics")
                );
                eprintln!(
                    "  Rust path events this frame: {}",
                    serde_json::to_string(&actual_path_events)
                        .expect("serialize Rust path-event diagnostics")
                );
            }
            if let Some(route_events) = &frame.route_construction_events
                && !route_events.is_empty()
            {
                eprintln!(
                    "  Original route-construction events this frame: {}",
                    serde_json::to_string(route_events)
                        .expect("serialize Original route-construction diagnostics")
                );
            }
            print_schema_sixteen_events("popup events", frame.popup_events.as_ref());
            print_schema_sixteen_events("AI forecast events", frame.ai_forecast_events.as_ref());
            print_schema_sixteen_events(
                "alert-formation events",
                frame.alert_formation_events.as_ref(),
            );
            print_schema_sixteen_events(
                "GoTo-authorization events",
                frame.goto_authorization_events.as_ref(),
            );
            print_schema_sixteen_events(
                "strike-proposal events",
                frame.strike_proposal_events.as_ref(),
            );
            print_schema_sixteen_events(
                "sequence-lifecycle events",
                frame.sequence_lifecycle_events.as_ref(),
            );
            print_schema_sixteen_events(
                "target lifecycle events",
                frame.target_lifecycle_events.as_ref(),
            );
            print_schema_sixteen_actor_diagnostics(&frame.elements);
            if automatic_dump_enabled {
                write_automatic_rolling_dump(
                    &rolling_dump,
                    &trace_path,
                    &header,
                    map,
                    frame.frame_after,
                );
            }
            if http_server.is_some() {
                eprintln!(
                    "parity replay halted at frame {}; HTTP inspection remains available",
                    engine.frame_counter()
                );
                serve_halted_http(
                    &mut engine,
                    &mut display,
                    &assets,
                    &mut input,
                    &mut selected_view_element,
                );
            }
            if let Some(visual) = &mut visual {
                eprintln!(
                    "visual parity replay frozen at first divergence; close the window to exit"
                );
                visual.wait_until_closed();
            }
            return 1;
        }
        // Original captures the frame above before its post-refresh
        // PostInitialize hook. The hook's effects belong to the starting
        // state of the next recorded frame, not the frame just compared.
        cross_post_initialize_frame(&mut engine, &mut display, &mut input, &assets, &mut dev);
        if let Some(step) = &mut active_http_step {
            step.remaining -= 1;
            if step.remaining == 0 {
                let step = active_http_step.take().expect("active HTTP step exists");
                step.request.respond_ok(serde_json::json!({
                    "direction": step.direction,
                    "from_frame": step.from_frame,
                    "frame": engine.frame_counter(),
                    "advanced": step.requested,
                    "parity": "matched",
                }));
            }
        }
    };

    if profile_timing {
        eprintln!(
            "parity timing: total={:.3}s simulation={:.3}s comparison={:.3}s rng_diagnostics={:.3}s other={:.3}s",
            replay_started.elapsed().as_secs_f64(),
            simulation_time.as_secs_f64(),
            comparison_time.as_secs_f64(),
            rng_diagnostic_time.as_secs_f64(),
            replay_started
                .elapsed()
                .saturating_sub(simulation_time + comparison_time + rng_diagnostic_time)
                .as_secs_f64(),
        );
    }

    match terminator {
        BinaryTraceRecord::End {
            rng_suffix: Some(_),
            final_frame: Some(final_frame),
            frame_count: Some(frame_count),
        } => {
            records
                .validate_terminator(frame_count, final_frame)
                .unwrap_or_else(|error| {
                    panic!(
                        "native parity trace {} has an invalid terminal record: {error}",
                        native_path.display()
                    )
                });
            assert_eq!(
                frame_count,
                u64::try_from(line_index).expect("parity frame count exceeds u64"),
                "parity terminator frame_count disagrees with the frame stream"
            );
            trace_timeline
                .validate_terminator(frame_count, final_frame)
                .unwrap_or_else(|error| panic!("invalid parity terminator timeline: {error}"));
            assert_eq!(
                u64::from(engine.frame_counter()),
                final_frame,
                "Rust final frame disagrees with the clean parity terminator"
            );
        }
        BinaryTraceRecord::End {
            rng_suffix: None,
            final_frame: None,
            frame_count: None,
        } => panic!("parity trace ended without a clean rng_suffix terminator"),
        BinaryTraceRecord::End { .. } => {
            panic!("native parity trace contains a partially populated terminator")
        }
        BinaryTraceRecord::Frame(_) => unreachable!("replay loop exits only on a terminator"),
    }

    if let Some(step) = active_http_step.take() {
        step.request.respond_err(format!(
            "trace ended at frame {} with {} requested frames still pending",
            engine.frame_counter(),
            step.remaining
        ));
    }
    if divergent_frames == 0 {
        println!("parity trace matched every recorded frame");
        if let Some(visual) = &mut visual {
            eprintln!("visual parity replay finished; close the window to exit");
            visual.wait_until_closed();
        }
        0
    } else {
        println!("logical parity scan: {divergent_frames} divergent frames");
        for (field, (frame, example)) in first_by_field {
            println!("  first {field} divergence after frame {frame}: {example}");
        }
        1
    }
}

fn parse_options() -> Options {
    const USAGE: &str = "usage: original_parity_replay [--scan-all] [--no-auto-dump] [--visual] \
        [--frame-zero-screenshot-dir DIR] \
        [--frame-zero-screenshot-only] \
        [--http-server PORT [--start-paused]] \
        [--dump-jsonl PATH [--dump-from FRAME] [--dump-through FRAME] \
        [--dump-entity KIND:INDEX]...] [--bench-encodings] [--convert] [--reblock] \
        [--validate-native] TRACE.jsonl[.zst]";

    let mut args = std::env::args_os().skip(1);
    let mut bench_encodings = false;
    let mut convert = false;
    let mut reblock = false;
    let mut validate_native = false;
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
            Some("--scan-all") => scan_all = true,
            Some("--bench-encodings") => bench_encodings = true,
            Some("--convert") => convert = true,
            Some("--reblock") => reblock = true,
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
            + usize::from(convert)
            + usize::from(reblock)
            + usize::from(validate_native)
            <= 1,
        "--bench-encodings, --convert, --reblock, and --validate-native are mutually exclusive"
    );
    Options {
        scan_all,
        no_auto_dump,
        visual,
        trace_path,
        http_server,
        start_paused,
        frame_zero_screenshot_dir,
        bench_encodings,
        convert,
        reblock,
        validate_native,
        dump: dump_path.map(|path| DumpOptions {
            path,
            from_frame: dump_from,
            through_frame: dump_through,
            entities: dump_entities,
        }),
    }
}

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
            robin_rs::http_server::StepKind::Forward { n } => {
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
            robin_rs::http_server::StepKind::GoToFrame { target } => {
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

fn trace_content_sha256(trace_path: &Path) -> String {
    let mut source = File::open(trace_path).unwrap_or_else(|error| {
        panic!(
            "open parity trace for fingerprint {}: {error}",
            trace_path.display()
        )
    });
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = source.read(&mut buffer).unwrap_or_else(|error| {
            panic!(
                "read parity trace for fingerprint {}: {error}",
                trace_path.display()
            )
        });
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    let digest = digest.finalize();
    let mut content_sha256 = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut content_sha256, "{byte:02x}").expect("writing to String cannot fail");
    }
    content_sha256
}

fn trace_source_fingerprint(trace_path: &Path) -> String {
    let metadata = std::fs::metadata(trace_path)
        .unwrap_or_else(|error| panic!("stat parity trace {}: {error}", trace_path.display()));
    let modified = metadata
        .modified()
        .expect("parity trace modification time is unavailable")
        .duration_since(std::time::UNIX_EPOCH)
        .expect("parity trace modification time predates Unix epoch")
        .as_nanos();
    let content_sha256 = trace_content_sha256(trace_path);
    format!(
        "native-parity-v{TRACE_NATIVE_VERSION}:length={}:modified={modified}:sha256={content_sha256}",
        metadata.len()
    )
}

fn absolute_trace_path(trace_path: &Path) -> PathBuf {
    absolute_trace_path_from(
        trace_path,
        &std::env::current_dir().expect("read current directory for relative parity trace"),
    )
}

fn absolute_trace_path_from(trace_path: &Path, current_dir: &Path) -> PathBuf {
    if trace_path.is_absolute() {
        return trace_path.to_owned();
    }
    current_dir.join(trace_path)
}

/// Canonicalize the logical trace path even when only its native artifact
/// still exists on disk (a converted recording is deleted, but its
/// `.jsonl.zst` path remains the trace's identity).
fn canonicalize_trace_identity(trace_path: &Path) -> PathBuf {
    if trace_path.exists() {
        return trace_path
            .canonicalize()
            .unwrap_or_else(|error| panic!("canonicalize {}: {error}", trace_path.display()));
    }
    let file_name = trace_path.file_name().unwrap_or_else(|| {
        panic!(
            "parity trace path {} has no file name",
            trace_path.display()
        )
    });
    let parent = match trace_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    parent
        .canonicalize()
        .unwrap_or_else(|error| {
            panic!(
                "canonicalize parity trace directory {}: {error}",
                parent.display()
            )
        })
        .join(file_name)
}

fn native_binary_trace_path(trace_path: &std::path::Path) -> PathBuf {
    let mut native_name = trace_path.as_os_str().to_owned();
    // A plain `.jsonl` capture converts to the same artifact name its
    // compressed spelling would have produced: the `.jsonl.zst` path is the
    // stable trace identity in ledgers, sweep status keys, and completion
    // markers, so skipping the interim zstd recording must not change it.
    if trace_path.as_os_str().to_string_lossy().ends_with(".jsonl") {
        native_name.push(".zst");
    }
    native_name.push(TRACE_NATIVE_SUFFIX);
    PathBuf::from(native_name)
}

fn open_jsonl_trace(trace_path: &std::path::Path) -> Box<dyn BufRead> {
    const ZSTD_MAGIC: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];

    let mut file = File::open(trace_path)
        .unwrap_or_else(|error| panic!("open parity trace {}: {error}", trace_path.display()));
    let mut magic = [0_u8; ZSTD_MAGIC.len()];
    let magic_len = file.read(&mut magic).unwrap_or_else(|error| {
        panic!("read parity trace magic {}: {error}", trace_path.display())
    });
    file.rewind().unwrap_or_else(|error| {
        panic!(
            "rewind parity trace after reading magic {}: {error}",
            trace_path.display()
        )
    });

    if magic_len == ZSTD_MAGIC.len() && magic == ZSTD_MAGIC {
        let mut decoder = zstd::stream::read::Decoder::new(file).unwrap_or_else(|error| {
            panic!(
                "start parity trace decompression {}: {error}",
                trace_path.display()
            )
        });
        decoder
            .window_log_max(TRACE_ZSTD_WINDOW_LOG_MAX)
            .unwrap_or_else(|error| {
                panic!(
                    "configure parity trace decompression {}: {error}",
                    trace_path.display()
                )
            });
        Box::new(BufReader::new(decoder))
    } else {
        Box::new(BufReader::new(file))
    }
}

/// Normalize a trace JSON tree for the cache round-trip audit. Two declared,
/// information-preserving differences between the raw JSONL and the typed
/// representation are erased on BOTH sides so everything else must match
/// exactly:
///
/// * `{"bits": N, "value": F}` float objects lose the redundant decimal
///   rendering `value`; `bits` alone is authoritative.
/// * `null` object entries are removed: serde cannot distinguish an absent
///   optional field from an explicit `null` once re-serialized (both are
///   `None`), and the two fields where Original's `null` is meaningful keep
///   the distinction in their typed `Option<Option<_>>` form. Array elements
///   are never removed.
/// * Empty array/object entries are removed after their children normalize:
///   schema-gated `#[serde(default)]` collection fields parse from an absent
///   key but re-serialize as an empty collection, so the typed schema
///   deliberately identifies the two.
/// * Legacy alert-formation eligibility used `stay_on_post` for the boolean
///   now emitted by Original as `allowed_to_leave_post`. The two spellings
///   share one native slot; only this exact nested key is canonicalized.
fn normalize_trace_json_for_roundtrip(value: &mut serde_json::Value) {
    normalize_trace_json_for_roundtrip_inner(value, false);
}

fn normalize_trace_json_for_roundtrip_inner(
    value: &mut serde_json::Value,
    inside_alert_formation_events: bool,
) {
    match value {
        serde_json::Value::Object(map) => {
            if map.len() == 2
                && map.get("bits").is_some_and(serde_json::Value::is_u64)
                && map.get("value").is_some_and(|value| {
                    // RHParity's FloatState renders non-finite floats as
                    // strings; `bits` alone carries the information.
                    value.is_number()
                        || matches!(value.as_str(), Some("nan" | "infinity" | "-infinity"))
                })
            {
                map.remove("value");
            }
            if inside_alert_formation_events {
                if let Some(serde_json::Value::Object(eligibility)) = map.get_mut("eligibility") {
                    if let Some(stay_on_post) = eligibility.remove("stay_on_post") {
                        assert!(
                            eligibility
                                .insert("allowed_to_leave_post".to_owned(), stay_on_post)
                                .is_none(),
                            "alert eligibility contains both stay_on_post and allowed_to_leave_post"
                        );
                    }
                }
            }
            for (key, child) in map.iter_mut() {
                normalize_trace_json_for_roundtrip_inner(child, key == "alert_formation_events");
            }
            map.retain(|_, child| match child {
                serde_json::Value::Null => false,
                serde_json::Value::Array(items) => !items.is_empty(),
                serde_json::Value::Object(entries) => !entries.is_empty(),
                _ => true,
            });
        }
        serde_json::Value::Array(items) => {
            for item in items {
                normalize_trace_json_for_roundtrip_inner(item, inside_alert_formation_events);
            }
        }
        _ => {}
    }
}

/// First path where the two normalized JSON trees disagree, or `None` when
/// they match. Paths make cache-build failures actionable on multi-hundred-KB
/// trace lines.
fn first_json_difference(
    path: &str,
    original: &serde_json::Value,
    reserialized: &serde_json::Value,
) -> Option<String> {
    use serde_json::Value;
    match (original, reserialized) {
        (Value::Object(original), Value::Object(reserialized)) => {
            for (key, original_child) in original {
                let Some(reserialized_child) = reserialized.get(key) else {
                    return Some(format!(
                        "{path}.{key} is dropped by the typed representation (recorded {original_child})"
                    ));
                };
                if let Some(difference) = first_json_difference(
                    &format!("{path}.{key}"),
                    original_child,
                    reserialized_child,
                ) {
                    return Some(difference);
                }
            }
            reserialized
                .keys()
                .find(|key| !original.contains_key(*key))
                .map(|key| format!("{path}.{key} is invented by the typed representation"))
        }
        (Value::Array(original), Value::Array(reserialized)) => {
            if original.len() != reserialized.len() {
                return Some(format!(
                    "{path} has {} recorded elements but {} typed elements",
                    original.len(),
                    reserialized.len()
                ));
            }
            original.iter().zip(reserialized).enumerate().find_map(
                |(index, (original_child, reserialized_child))| {
                    first_json_difference(
                        &format!("{path}[{index}]"),
                        original_child,
                        reserialized_child,
                    )
                },
            )
        }
        _ if original == reserialized => None,
        _ => Some(format!("{path}: recorded {original} became {reserialized}")),
    }
}

/// Panic unless the typed record re-serializes to the JSON it was parsed
/// from, modulo [`normalize_trace_json_for_roundtrip`]. Running this on every
/// line during cache conversion is what lets the binary cache stand in for
/// the recording: a field the typed schema silently drops or reshapes fails
/// the build instead of becoming data loss.
///
/// Building JSON trees for both sides of every frame is expensive, so
/// [`ensure_native_binary_trace`] audits frame lines on a worker pool
/// ([`spawn_roundtrip_audit_workers`]) while the writer thread streams
/// records into the cache; a failed audit aborts conversion before the
/// temporary cache file is published.
fn verify_trace_line_roundtrip<T: Serialize>(record: &T, line: &str, line_number: usize) {
    let mut original: serde_json::Value = serde_json::from_str(line).unwrap_or_else(|error| {
        panic!("reparse trace line {line_number} for the round-trip audit: {error}")
    });
    let mut reserialized = serde_json::to_value(record).unwrap_or_else(|error| {
        panic!("reserialize trace line {line_number} for the round-trip audit: {error}")
    });
    normalize_trace_json_for_roundtrip(&mut original);
    normalize_trace_json_for_roundtrip(&mut reserialized);
    if let Some(difference) = first_json_difference("$", &original, &reserialized) {
        panic!(
            "trace line {line_number} does not survive the typed cache round trip: {difference}"
        );
    }
}

/// Re-parse and round-trip-audit one trace line (any line after the header
/// and RNG prefix: frames and the rng_suffix terminator).
fn audit_trace_line(line: &str, line_number: usize) {
    if let Some(frame) = parse_trace_frame(line, line_number) {
        verify_trace_line_roundtrip(&frame, line, line_number);
    } else {
        let suffix: TraceRngOnly = serde_json::from_str(line).unwrap_or_else(|error| {
            panic!(
                "reparse RNG suffix on trace line {line_number} for the round-trip audit: {error}"
            )
        });
        verify_trace_line_roundtrip(&suffix, line, line_number);
    }
}

/// Fan trace lines out to audit workers. Returns the sender; drop it to let
/// the workers drain and finish. Worker panics (i.e. audit failures)
/// propagate when the enclosing [`std::thread::scope`] joins.
fn spawn_roundtrip_audit_workers<'scope, 'env>(
    scope: &'scope std::thread::Scope<'scope, 'env>,
) -> std::sync::mpsc::SyncSender<(usize, String)> {
    // Bounded so a fast reader cannot buffer a whole multi-GB trace.
    let (sender, receiver) = std::sync::mpsc::sync_channel::<(usize, String)>(64);
    let receiver = std::sync::Arc::new(std::sync::Mutex::new(receiver));
    let workers = std::thread::available_parallelism()
        .map(|threads| threads.get().saturating_sub(1).clamp(1, 8))
        .unwrap_or(1);
    for _ in 0..workers {
        let receiver = std::sync::Arc::clone(&receiver);
        scope.spawn(move || {
            loop {
                let received = receiver
                    .lock()
                    .expect("audit line channel lock is never poisoned")
                    .recv();
                match received {
                    Ok((line_number, line)) => audit_trace_line(&line, line_number),
                    Err(_) => return,
                }
            }
        });
    }
    sender
}

/// Panic unless the native trace at `native_path` carries the current format
/// version in both its fixed footer and its decoded header. Used when there
/// is no JSONL source to regenerate from, so the only correct responses to a
/// mismatch are migration or restoring the recording — never regeneration.
fn validate_standalone_native_trace(native_path: &Path) {
    let footer = read_binary_trace_footer(native_path).unwrap_or_else(|error| {
        panic!(
            "native parity trace {} has a corrupt or missing fixed footer: {error};              its JSONL source is gone, so restore or migrate the native file",
            native_path.display()
        )
    });
    let header = read_binary_trace_header(native_path);
    assert!(
        footer.version == TRACE_NATIVE_VERSION && header.version == TRACE_NATIVE_VERSION,
        "native parity trace {} is version {} (footer {}) but this runner expects          {TRACE_NATIVE_VERSION}; its JSONL source is gone, so it must be migrated          with a runner that still reads its version",
        native_path.display(),
        header.version,
        footer.version,
    );

    // A conversion may have crashed after unlinking the JSONL source.  In
    // that state the native file is authoritative, so accepting it based on
    // only its header/footer would conceal a corrupt compressed block.  Read
    // the complete record stream before reporting an already-converted
    // source as successful.
    let mut reader = BinaryTraceReader::open(native_path);
    let decoded_header = reader.read_header();
    let mut timeline = TraceTimeline::new(decoded_header.trace.initial_frame);
    loop {
        match reader.read_record() {
            BinaryTraceRecord::Frame(frame) => timeline
                .observe(frame.frame_before, frame.frame_after)
                .unwrap_or_else(|error| {
                    panic!(
                        "standalone native parity trace {} breaks the frame timeline: {error}",
                        native_path.display()
                    )
                }),
            BinaryTraceRecord::End {
                rng_suffix,
                final_frame,
                frame_count,
            } => {
                assert!(
                    rng_suffix.is_some(),
                    "standalone native parity trace {} lost its RNG suffix",
                    native_path.display()
                );
                let final_frame = final_frame.unwrap_or_else(|| {
                    panic!(
                        "standalone native parity trace {} lost its final frame",
                        native_path.display()
                    )
                });
                let frame_count = frame_count.unwrap_or_else(|| {
                    panic!(
                        "standalone native parity trace {} lost its frame count",
                        native_path.display()
                    )
                });
                timeline
                    .validate_terminator(frame_count, final_frame)
                    .unwrap_or_else(|error| {
                        panic!(
                            "standalone native parity trace {} terminator disagrees with its frames: {error}",
                            native_path.display()
                        )
                    });
                reader
                    .validate_terminator(frame_count, final_frame)
                    .unwrap_or_else(|error| {
                        panic!(
                            "standalone native parity trace {} disagrees with its fixed footer: {error}",
                            native_path.display()
                        )
                    });
                break;
            }
        }
    }
}

/// `--convert`: turn a JSONL recording into its native parity trace and
/// delete the recording once losslessness is assured. Safety gates, in
/// order:
///
/// 1. Conversion itself round-trip audits every line (see
///    [`verify_trace_line_roundtrip`]) — an unfaithful typed representation
///    aborts before the native file is published.
/// 2. The published native file is independently re-read from disk: every
///    block must decode, the frame timeline must be contiguous, and the
///    terminator must agree with the fixed footer.
/// 3. The decoded frame count must match the recording's own line count.
///
/// Only then is the JSONL deleted, along with obsolete `.parity-cache-v*`
/// derivations of it. Completion markers (`*.complete`) are left in place —
/// they carry the trace's capture provenance and its identity persists.
fn convert_recording_to_native(trace_path: &Path) {
    // `Path::parent()` is `Some("")` for a bare relative file name. Resolve
    // the input before publishing so cleanup always has a real directory and
    // `--convert replay.jsonl.zst` behaves like `--convert ./replay.jsonl.zst`.
    let trace_path = absolute_trace_path(trace_path);
    let display = trace_path.display();
    assert!(
        !trace_path
            .as_os_str()
            .to_string_lossy()
            .ends_with(TRACE_NATIVE_SUFFIX),
        "{display} already is a native parity trace"
    );
    let native_path = native_binary_trace_path(&trace_path);
    let quarantine_path = conversion_quarantine_path(&trace_path);
    // State classification and recovery are protected by the same stable
    // lock inode as generation and deletion.
    let _generation_lock = lock_native_trace_generation(&native_path);
    reject_conversion_symlink(&trace_path);
    reject_conversion_symlink(&quarantine_path);
    reject_conversion_symlink(&native_path);

    match (trace_path.exists(), quarantine_path.exists()) {
        (true, true) => {
            panic!(
                "conversion conflict: producer recreated {display} while pending quarantine {} exists; preserve both",
                quarantine_path.display()
            );
        }
        (false, true) => {
            assert!(
                native_path.is_file(),
                "pending conversion quarantine {} has no native counterpart {}",
                quarantine_path.display(),
                native_path.display()
            );
            let fingerprint = trace_source_fingerprint(&quarantine_path);
            let verified =
                verify_converted_native_trace(&quarantine_path, &native_path, fingerprint);
            let removed_derived =
                finish_verified_conversion(&trace_path, &quarantine_path, &verified);
            eprintln!(
                "recovered conversion of {display} into {} ({} frames); deleted the pending source and {removed_derived} obsolete derived files",
                native_path.display(),
                verified.decoded_frames
            );
            return;
        }
        (false, false) => {
            assert!(
                native_path.is_file(),
                "recording {display} does not exist and has no native counterpart {}",
                native_path.display()
            );
            validate_standalone_native_trace(&native_path);
            eprintln!(
                "recording {display} was already converted into {}",
                native_path.display()
            );
            return;
        }
        (true, false) => {}
    }
    assert!(trace_path.is_file(), "recording {display} is not a file");
    let source_bytes = std::fs::metadata(&trace_path)
        .expect("stat recording before conversion")
        .len();
    let source_fingerprint = trace_source_fingerprint(&trace_path);
    let native_path =
        ensure_native_binary_trace_locked(&trace_path, &native_path, source_fingerprint.clone());

    // Independent re-read of the published artifact. Keep the proof token in
    // the type flow so the destructive cleanup below cannot move ahead of the
    // complete native readback accidentally.
    let verified = verify_converted_native_trace(&trace_path, &native_path, source_fingerprint);
    move_verified_recording_to_quarantine(&trace_path, &quarantine_path, &verified)
        .unwrap_or_else(|error| panic!("refuse to quarantine converted recording: {error}"));
    let removed_derived = finish_verified_conversion(
        &trace_path,
        &quarantine_path,
        &VerifiedNativeReadback {
            source_path: quarantine_path.clone(),
            ..verified.clone()
        },
    );

    let native_bytes = std::fs::metadata(&native_path)
        .expect("stat native parity trace after conversion")
        .len();
    eprintln!(
        "converted {display} ({:.2} MiB) into {} ({:.2} MiB, {} frames);          deleted the recording and {removed_derived} obsolete derived files",
        source_bytes as f64 / (1024.0 * 1024.0),
        native_path.display(),
        native_bytes as f64 / (1024.0 * 1024.0),
        verified.decoded_frames,
    );
}

#[derive(Clone, Debug)]
struct VerifiedNativeReadback {
    decoded_frames: u64,
    source_path: PathBuf,
    source_fingerprint: String,
}

fn verify_converted_native_trace(
    trace_path: &Path,
    native_path: &Path,
    source_fingerprint: String,
) -> VerifiedNativeReadback {
    let mut source_lines = open_jsonl_trace(trace_path).lines();
    let source_header_line = source_lines
        .next()
        .expect("recording lost its header during native readback")
        .expect("read recording header during native readback");
    let source_trace: TraceHeader = serde_json::from_str(&source_header_line)
        .expect("reparse recording header during native readback");
    let source_prefix_line = source_lines
        .next()
        .expect("recording lost its RNG prefix during native readback")
        .expect("read recording RNG prefix during native readback");
    let source_prefix: TraceRngPrefix = serde_json::from_str(&source_prefix_line)
        .expect("reparse recording RNG prefix during native readback");
    let expected_header = BinaryTraceHeader {
        version: TRACE_NATIVE_VERSION,
        source_fingerprint: source_fingerprint.clone(),
        trace: source_trace,
        rng_prefix: source_prefix,
    };

    let mut reader = BinaryTraceReader::open(native_path);
    let header = reader.read_header();
    assert_eq!(
        header.version,
        TRACE_NATIVE_VERSION,
        "native parity trace {} decodes with the wrong version",
        native_path.display()
    );
    assert_eq!(
        bitcode::encode(&header),
        bitcode::encode(&expected_header),
        "native parity trace {} header differs semantically from its recording",
        native_path.display()
    );
    let mut timeline = TraceTimeline::new(header.trace.initial_frame);
    let mut decoded_frames = 0_u64;
    loop {
        match reader.read_record() {
            BinaryTraceRecord::Frame(frame) => {
                let line_number = decoded_frames + 3;
                let source_line = source_lines
                    .next()
                    .unwrap_or_else(|| {
                        panic!("recording ended before native frame on line {line_number}")
                    })
                    .unwrap_or_else(|error| {
                        panic!("read recording frame on line {line_number}: {error}")
                    });
                let source_frame = parse_trace_frame(&source_line, line_number as usize)
                    .unwrap_or_else(|| {
                        panic!(
                            "recording has its terminator before native frame on line {line_number}"
                        )
                    });
                assert_eq!(
                    bitcode::encode(&frame),
                    bitcode::encode(&source_frame),
                    "native parity frame on line {line_number} differs semantically from its recording"
                );
                timeline
                    .observe(frame.frame_before, frame.frame_after)
                    .unwrap_or_else(|error| {
                        panic!(
                            "native parity trace {} breaks the frame timeline: {error}",
                            native_path.display()
                        )
                    });
                decoded_frames += 1;
            }
            BinaryTraceRecord::End {
                rng_suffix,
                final_frame,
                frame_count,
            } => {
                let line_number = decoded_frames + 3;
                let source_line = source_lines
                    .next()
                    .unwrap_or_else(|| {
                        panic!("recording ended before native terminator on line {line_number}")
                    })
                    .unwrap_or_else(|error| {
                        panic!("read recording terminator on line {line_number}: {error}")
                    });
                let source_end: TraceRngOnly =
                    serde_json::from_str(&source_line).unwrap_or_else(|error| {
                        panic!("parse recording terminator on line {line_number}: {error}")
                    });
                let final_frame = final_frame.expect("native End record lost its final frame");
                let frame_count = frame_count.expect("native End record lost its frame count");
                let rng_suffix = rng_suffix.expect("native End record lost its RNG suffix");
                assert_eq!(
                    bitcode::encode(&rng_suffix),
                    bitcode::encode(&source_end.draws),
                    "native RNG suffix differs semantically from its recording"
                );
                assert_eq!(final_frame, source_end.final_frame);
                assert_eq!(frame_count, source_end.frame_count);
                assert_eq!(frame_count, decoded_frames);
                timeline
                    .validate_terminator(frame_count, final_frame)
                    .unwrap_or_else(|error| {
                        panic!(
                            "native parity trace {} terminator disagrees with its frames: {error}",
                            native_path.display()
                        )
                    });
                reader
                    .validate_terminator(frame_count, final_frame)
                    .unwrap_or_else(|error| {
                        panic!(
                            "native parity trace {} disagrees with its fixed footer: {error}",
                            native_path.display()
                        )
                    });
                break;
            }
        }
    }
    assert!(
        source_lines.next().is_none(),
        "recording {} has data after its native terminator",
        trace_path.display()
    );

    assert_eq!(
        header.source_fingerprint,
        source_fingerprint,
        "native parity trace {} was not built from the source fingerprint held by this conversion",
        native_path.display()
    );

    VerifiedNativeReadback {
        decoded_frames,
        source_path: trace_path.to_owned(),
        source_fingerprint,
    }
}

fn conversion_quarantine_path(trace_path: &Path) -> PathBuf {
    let mut path = trace_path.as_os_str().to_owned();
    path.push(TRACE_CONVERSION_QUARANTINE_SUFFIX);
    PathBuf::from(path)
}

fn reject_conversion_symlink(path: &Path) {
    match std::fs::symlink_metadata(path) {
        Ok(_) => assert!(
            !conversion_path_is_symlink(path),
            "conversion paths must not be symbolic links: {}",
            path.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("inspect conversion path {}: {error}", path.display()),
    }
}

fn conversion_path_is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
}

fn move_verified_recording_to_quarantine(
    trace_path: &Path,
    quarantine_path: &Path,
    verified: &VerifiedNativeReadback,
) -> Result<(), String> {
    if trace_path != verified.source_path {
        return Err(format!(
            "verified source path {} changed to {} before quarantine",
            verified.source_path.display(),
            trace_path.display()
        ));
    }
    let parent = trace_path
        .parent()
        .expect("absolute converted recording always has a parent directory");
    if quarantine_path.exists() {
        return Err(format!(
            "deterministic conversion quarantine {} already exists",
            quarantine_path.display()
        ));
    }
    std::fs::rename(trace_path, &quarantine_path).map_err(|error| {
        format!(
            "atomically quarantine converted recording {} as {}: {error}",
            trace_path.display(),
            quarantine_path.display()
        )
    })?;
    sync_directory(parent, "converted recording quarantine");

    let quarantined_fingerprint = trace_source_fingerprint(&quarantine_path);
    if quarantined_fingerprint == verified.source_fingerprint {
        return Ok(());
    }

    let restoration = match restore_quarantined_recording_no_replace(quarantine_path, trace_path) {
        Ok(()) => {
            std::fs::remove_file(quarantine_path)
                .map_err(|error| format!("remove restored recording quarantine link: {error}"))?;
            sync_directory(parent, "mismatched recording restoration");
            "restored it without overwriting another producer".to_owned()
        }
        Err(error) => {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                format!(
                    "canonical pathname was recreated; preserved mismatched content at {}",
                    quarantine_path.display()
                )
            } else {
                return Err(format!(
                    "recording changed and atomic no-replace restoration {} -> {} failed: {error}; preserved mismatched content at {}",
                    quarantine_path.display(),
                    trace_path.display(),
                    quarantine_path.display()
                ));
            }
        }
    };
    Err(format!(
        "recording {} changed after native readback (expected {:?}, found {:?}); {restoration}",
        trace_path.display(),
        verified.source_fingerprint,
        quarantined_fingerprint
    ))
}

/// Atomically restore a quarantined source without replacing a producer that
/// recreated the canonical pathname. Both paths share a filesystem, so a
/// hard link provides the required no-replace operation.
fn restore_quarantined_recording_no_replace(
    quarantine_path: &Path,
    trace_path: &Path,
) -> std::io::Result<()> {
    std::fs::hard_link(quarantine_path, trace_path)
}

fn finish_verified_conversion(
    trace_path: &Path,
    quarantine_path: &Path,
    verified: &VerifiedNativeReadback,
) -> usize {
    assert_eq!(verified.source_path, quarantine_path);
    assert_eq!(
        trace_source_fingerprint(quarantine_path),
        verified.source_fingerprint,
        "pending conversion source changed after native readback"
    );
    assert!(
        !trace_path.exists(),
        "conversion conflict: producer recreated {} while its verified quarantine still exists",
        trace_path.display(),
    );
    let parent = trace_path
        .parent()
        .expect("absolute converted recording always has a parent directory");
    commit_verified_conversion_files(trace_path, quarantine_path, || {})
        .unwrap_or_else(|error| panic!("conversion committed with a producer conflict: {error}"));
    let mut removed_derived = 0_usize;
    if let Some(name) = trace_path.file_name() {
        let obsolete_prefix = format!("{}.parity-cache-v", name.to_string_lossy());
        let entries = match std::fs::read_dir(parent) {
            Ok(entries) => entries,
            Err(error) => {
                eprintln!("warning: could not list obsolete conversion derivations: {error}");
                return 0;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    eprintln!("warning: could not read obsolete derivation entry: {error}");
                    continue;
                }
            };
            if is_obsolete_native_derivation(&entry.file_name().to_string_lossy(), &obsolete_prefix)
            {
                match std::fs::remove_file(entry.path()) {
                    Ok(()) => removed_derived += 1,
                    Err(error) => eprintln!(
                        "warning: could not delete obsolete derivation {}: {error}",
                        entry.path().display()
                    ),
                }
            }
        }
        if removed_derived > 0 {
            if let Err(error) = File::open(parent).and_then(|directory| directory.sync_all()) {
                eprintln!(
                    "warning: could not sync {} after obsolete converted recording cleanup: {error}",
                    parent.display()
                );
            }
        }
    }
    removed_derived
}

fn commit_verified_conversion_files(
    trace_path: &Path,
    quarantine_path: &Path,
    before_quarantine_unlink: impl FnOnce(),
) -> Result<(), String> {
    let parent = trace_path
        .parent()
        .expect("absolute converted recording always has a parent directory");
    if trace_path.exists() {
        return Err(format!(
            "producer recreated canonical recording {} before quarantine commit",
            trace_path.display()
        ));
    }
    before_quarantine_unlink();
    std::fs::remove_file(quarantine_path).unwrap_or_else(|error| {
        panic!(
            "delete quarantined converted recording {}: {error}",
            quarantine_path.display()
        )
    });
    sync_directory(parent, "converted recording transaction commit");
    if trace_path.exists() {
        return Err(format!(
            "producer recreated canonical recording {} during quarantine commit; new recording was preserved",
            trace_path.display()
        ));
    }
    Ok(())
}

fn is_obsolete_native_derivation(file_name: &str, prefix: &str) -> bool {
    let Some(suffix) = file_name.strip_prefix(prefix) else {
        return false;
    };
    let digit_count = suffix
        .bytes()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    digit_count > 0
        && (digit_count == suffix.len() || suffix.as_bytes().get(digit_count) == Some(&b'.'))
}

fn native_trace_generation_lock_path(native_path: &Path) -> PathBuf {
    let mut lock_name = native_path.as_os_str().to_owned();
    lock_name.push(".lock");
    PathBuf::from(lock_name)
}

fn lock_native_trace_generation(native_path: &Path) -> File {
    let lock_path = native_trace_generation_lock_path(native_path);
    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock_path)
        .unwrap_or_else(|error| {
            panic!(
                "open native parity trace lock {}: {error}",
                lock_path.display()
            )
        });
    lock_file.lock_exclusive().unwrap_or_else(|error| {
        panic!(
            "lock native parity trace generation {}: {error}",
            lock_path.display()
        )
    });
    lock_file
}

fn sync_directory(directory: &Path, operation: &str) {
    File::open(directory)
        .unwrap_or_else(|error| {
            panic!(
                "open directory {} after {operation}: {error}",
                directory.display()
            )
        })
        .sync_all()
        .unwrap_or_else(|error| {
            panic!(
                "sync directory {} after {operation}: {error}",
                directory.display()
            )
        });
}

fn native_reblock_source_path(native_path: &Path) -> PathBuf {
    let mut path = native_path.as_os_str().to_owned();
    path.push(TRACE_REBLOCK_SOURCE_SUFFIX);
    PathBuf::from(path)
}

fn native_reblock_binding_path(native_path: &Path) -> PathBuf {
    let mut path = native_path.as_os_str().to_owned();
    path.push(TRACE_REBLOCK_BINDING_SUFFIX);
    PathBuf::from(path)
}

fn native_reblock_temporary_prefix(native_path: &Path, binding: bool) -> String {
    let canonical_path = native_reblock_canonical_path(native_path);
    #[cfg(unix)]
    let path_bytes = canonical_path.as_os_str().as_bytes();
    #[cfg(not(unix))]
    let path_text = canonical_path.to_string_lossy();
    #[cfg(not(unix))]
    let path_bytes = path_text.as_bytes();
    let path_digest = sha256_hex(path_bytes);
    let kind = if binding { "binding-v66" } else { "v66" };
    format!(".parity-reblock-{kind}-{path_digest}-")
}

fn cleanup_native_reblock_orphans(native_path: &Path) {
    let parent = native_path
        .parent()
        .expect("absolute native trace always has a parent directory");
    let prefixes = [
        native_reblock_temporary_prefix(native_path, false),
        native_reblock_temporary_prefix(native_path, true),
    ];
    let mut removed = 0_usize;
    for entry in std::fs::read_dir(parent).unwrap_or_else(|error| {
        panic!(
            "list native reblock temporary directory {}: {error}",
            parent.display()
        )
    }) {
        let entry = entry.unwrap_or_else(|error| {
            panic!(
                "read native reblock temporary entry in {}: {error}",
                parent.display()
            )
        });
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !prefixes.iter().any(|prefix| name.starts_with(prefix)) {
            continue;
        }
        let metadata = std::fs::symlink_metadata(entry.path()).unwrap_or_else(|error| {
            panic!(
                "inspect native reblock temporary {}: {error}",
                entry.path().display()
            )
        });
        if !metadata.file_type().is_file() {
            eprintln!(
                "warning: preserving non-regular native reblock temporary {}",
                entry.path().display()
            );
            continue;
        }
        std::fs::remove_file(entry.path()).unwrap_or_else(|error| {
            panic!(
                "remove orphaned native reblock temporary {}: {error}",
                entry.path().display()
            )
        });
        removed += 1;
    }
    if removed > 0 {
        sync_directory(parent, "native reblock orphan cleanup");
        eprintln!(
            "removed {removed} orphaned temporary file(s) for {}",
            native_path.display()
        );
    }
}

fn native_reblock_canonical_path(native_path: &Path) -> PathBuf {
    let parent = native_path
        .parent()
        .expect("absolute native trace always has a parent directory")
        .canonicalize()
        .unwrap_or_else(|error| {
            panic!(
                "canonicalize native trace parent {}: {error}",
                native_path.parent().unwrap().display()
            )
        });
    parent.join(
        native_path
            .file_name()
            .expect("native trace path always has a file name"),
    )
}

fn native_reblock_file_identity(path: &Path) -> (u64, String, u64, u64) {
    let metadata = std::fs::metadata(path)
        .unwrap_or_else(|error| panic!("stat native reblock file {}: {error}", path.display()));
    #[cfg(unix)]
    let (device, inode) = (metadata.dev(), metadata.ino());
    #[cfg(not(unix))]
    let (device, inode) = (0, 0);
    (metadata.len(), trace_content_sha256(path), device, inode)
}

fn native_reblock_semantic_identity(path: &Path) -> (u64, u64, String) {
    let footer = read_binary_trace_footer(path)
        .unwrap_or_else(|error| panic!("read native reblock footer {}: {error}", path.display()));
    validate_binary_trace_footer(&footer).unwrap_or_else(|error| {
        panic!("validate native reblock footer {}: {error}", path.display())
    });
    let (frame_count, digest) = digest_and_validate_native_trace(path);
    assert_eq!(frame_count, footer.frame_count);
    (frame_count, footer.final_frame, sha256_hex(&digest))
}

fn create_native_reblock_binding(native_path: &Path) -> NativeReblockBinding {
    let (source_bytes, source_content_sha256, source_device, source_inode) =
        native_reblock_file_identity(native_path);
    let (frame_count, final_frame, source_semantic_sha256) =
        native_reblock_semantic_identity(native_path);
    NativeReblockBinding {
        version: TRACE_NATIVE_VERSION,
        canonical_path: native_reblock_canonical_path(native_path),
        source_content_sha256,
        source_bytes,
        source_semantic_sha256,
        frame_count,
        final_frame,
        #[cfg(unix)]
        source_device,
        #[cfg(unix)]
        source_inode,
    }
}

fn write_native_reblock_binding(path: &Path, binding: &NativeReblockBinding) {
    let parent = path
        .parent()
        .expect("absolute native reblock binding always has a parent");
    let mut temporary = tempfile::Builder::new()
        .prefix(&native_reblock_temporary_prefix(
            Path::new(&binding.canonical_path),
            true,
        ))
        .tempfile_in(parent)
        .unwrap_or_else(|error| {
            panic!(
                "create temporary native reblock binding beside {}: {error}",
                path.display()
            )
        });
    serde_json::to_writer(&mut temporary, binding)
        .unwrap_or_else(|error| panic!("write native reblock binding: {error}"));
    temporary
        .write_all(b"\n")
        .unwrap_or_else(|error| panic!("terminate native reblock binding: {error}"));
    temporary
        .flush()
        .unwrap_or_else(|error| panic!("flush native reblock binding: {error}"));
    temporary
        .as_file()
        .sync_all()
        .unwrap_or_else(|error| panic!("sync native reblock binding: {error}"));
    temporary.persist(path).unwrap_or_else(|error| {
        panic!(
            "atomically publish native reblock binding {}: {}",
            path.display(),
            error.error
        )
    });
    sync_directory(parent, "native reblock binding publication");
}

fn read_native_reblock_binding(path: &Path, native_path: &Path) -> NativeReblockBinding {
    reject_conversion_symlink(path);
    let file = File::open(path)
        .unwrap_or_else(|error| panic!("open native reblock binding {}: {error}", path.display()));
    let binding: NativeReblockBinding = serde_json::from_reader(BufReader::new(file))
        .unwrap_or_else(|error| panic!("read native reblock binding {}: {error}", path.display()));
    assert_eq!(
        binding.version,
        TRACE_NATIVE_VERSION,
        "native reblock binding {} has unsupported version",
        path.display()
    );
    assert_eq!(
        binding.canonical_path,
        native_reblock_canonical_path(native_path),
        "native reblock binding {} belongs to another canonical path",
        path.display()
    );
    binding
}

fn validate_native_reblock_source_binding(source_path: &Path, binding: &NativeReblockBinding) {
    validate_native_reblock_source_file_identity(source_path, binding)
        .unwrap_or_else(|error| panic!("{error}"));
    validate_native_reblock_semantics(source_path, binding, "recovery source");
}

fn validate_native_reblock_source_file_identity(
    source_path: &Path,
    binding: &NativeReblockBinding,
) -> Result<(), String> {
    let (bytes, content_sha256, device, inode) = native_reblock_file_identity(source_path);
    if !native_reblock_file_identity_matches(binding, bytes, &content_sha256, device, inode) {
        return Err(format!(
            "native reblock recovery {} content or inode is not the bound source",
            source_path.display()
        ));
    }
    Ok(())
}

fn native_reblock_file_identity_matches(
    binding: &NativeReblockBinding,
    bytes: u64,
    content_sha256: &str,
    device: u64,
    inode: u64,
) -> bool {
    if bytes != binding.source_bytes || content_sha256 != binding.source_content_sha256 {
        return false;
    }
    #[cfg(unix)]
    {
        device == binding.source_device && inode == binding.source_inode
    }
    #[cfg(not(unix))]
    {
        let _ = (device, inode);
        true
    }
}

fn validate_native_reblock_semantics(path: &Path, binding: &NativeReblockBinding, label: &str) {
    let (frame_count, final_frame, semantic_sha256) = native_reblock_semantic_identity(path);
    assert_eq!(
        frame_count, binding.frame_count,
        "{label} frame count changed"
    );
    assert_eq!(
        final_frame, binding.final_frame,
        "{label} final frame changed"
    );
    assert_eq!(
        semantic_sha256, binding.source_semantic_sha256,
        "{label} is not semantically identical to the bound source"
    );
}

enum NativeReblockPreparation {
    Ready(NativeReblockBinding),
    AlreadyCommitted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativeReblockRecoveryState {
    Fresh,
    AuthenticatedPair,
    BindingOnly,
}

fn classify_native_reblock_recovery_state(
    native_exists: bool,
    source_exists: bool,
    binding_exists: bool,
) -> Result<NativeReblockRecoveryState, String> {
    match (native_exists, source_exists, binding_exists) {
        (true, false, false) => Ok(NativeReblockRecoveryState::Fresh),
        (_, true, true) => Ok(NativeReblockRecoveryState::AuthenticatedPair),
        (true, false, true) => Ok(NativeReblockRecoveryState::BindingOnly),
        (_, true, false) => {
            Err("native reblock recovery source has no authenticated binding".to_owned())
        }
        (false, false, false) => {
            Err("native trace is missing and has no authenticated recovery pair".to_owned())
        }
        (false, false, true) => Err(
            "native reblock binding exists without canonical trace or recovery source".to_owned(),
        ),
    }
}

fn prepare_native_reblock_source(
    native_path: &Path,
    source_path: &Path,
    binding_path: &Path,
) -> NativeReblockPreparation {
    let source_exists = source_path.exists();
    let binding_exists = binding_path.exists();
    let recovery_state = classify_native_reblock_recovery_state(
        native_path.is_file(),
        source_exists,
        binding_exists,
    )
    .unwrap_or_else(|error| {
        panic!(
            "{error}: canonical={} source={} binding={}; preserve every existing path for manual audit",
            native_path.display(),
            source_path.display(),
            binding_path.display()
        )
    });

    if recovery_state == NativeReblockRecoveryState::Fresh {
        let binding = create_native_reblock_binding(native_path);
        write_native_reblock_binding(binding_path, &binding);
        std::fs::hard_link(native_path, source_path).unwrap_or_else(|error| {
            panic!(
                "create authenticated reblock source {} for {}: {error}",
                source_path.display(),
                native_path.display()
            )
        });
        sync_directory(
            native_path.parent().unwrap(),
            "native parity trace reblock source publication",
        );
        validate_native_reblock_source_file_identity(source_path, &binding)
            .unwrap_or_else(|error| panic!("{error}"));
        return NativeReblockPreparation::Ready(binding);
    }

    let binding = read_native_reblock_binding(binding_path, native_path);
    if recovery_state == NativeReblockRecoveryState::BindingOnly {
        let (bytes, content_sha256, device, inode) = native_reblock_file_identity(native_path);
        if native_reblock_file_identity_matches(&binding, bytes, &content_sha256, device, inode) {
            validate_native_reblock_semantics(native_path, &binding, "canonical source");
            std::fs::hard_link(native_path, source_path).unwrap_or_else(|error| {
                panic!(
                    "resume authenticated reblock source publication {}: {error}",
                    source_path.display()
                )
            });
            sync_directory(
                native_path.parent().unwrap(),
                "resumed native parity trace reblock source publication",
            );
            validate_native_reblock_source_file_identity(source_path, &binding)
                .unwrap_or_else(|error| panic!("{error}"));
            return NativeReblockPreparation::Ready(binding);
        }

        // The source link is removed before its binding during commit. A
        // semantically identical but byte-distinct canonical file proves the
        // atomic publication completed and only binding cleanup remains.
        validate_native_reblock_semantics(native_path, &binding, "published canonical trace");
        std::fs::remove_file(binding_path).unwrap_or_else(|error| {
            panic!(
                "finish committed native reblock binding cleanup {}: {error}",
                binding_path.display()
            )
        });
        sync_directory(
            native_path.parent().unwrap(),
            "committed native reblock binding cleanup",
        );
        return NativeReblockPreparation::AlreadyCommitted;
    }

    validate_native_reblock_source_binding(source_path, &binding);
    if native_path.exists() {
        validate_native_reblock_semantics(native_path, &binding, "canonical trace");
    }
    eprintln!(
        "resuming authenticated reblock of {} from {}",
        native_path.display(),
        source_path.display()
    );
    NativeReblockPreparation::Ready(binding)
}

fn requested_native_trace_path(trace_path: &Path) -> PathBuf {
    let requested = absolute_trace_path(trace_path);
    if requested
        .as_os_str()
        .to_string_lossy()
        .ends_with(TRACE_NATIVE_SUFFIX)
    {
        requested
    } else {
        native_binary_trace_path(&requested)
    }
}

fn update_native_semantic_digest<T: bitcode::Encode + ?Sized>(digest: &mut Sha256, value: &T) {
    let encoded = bitcode::encode(value);
    digest.update((encoded.len() as u64).to_le_bytes());
    digest.update(encoded);
}

/// Rewrite an authoritative version-66 native trace into bounded bitcode
/// blocks and a bounded-window zstd frame. The semantic record stream and
/// fixed footer are unchanged.
///
/// A hard-link recovery source is synced before the atomic replacement. If
/// the process crashes at any later point, rerunning `--reblock` reads that
/// original inode and retries; the link is removed only after a complete
/// semantic-digest and timeline readback of the replacement.
fn reblock_native_trace(trace_path: &Path) {
    let native_path = requested_native_trace_path(trace_path);
    let display = native_path.display();
    let parent = native_path
        .parent()
        .expect("absolute native trace always has a parent directory");
    let source_path = native_reblock_source_path(&native_path);
    let binding_path = native_reblock_binding_path(&native_path);
    let _generation_lock = lock_native_trace_generation(&native_path);
    reject_conversion_symlink(&native_path);
    reject_conversion_symlink(&source_path);
    reject_conversion_symlink(&binding_path);
    cleanup_native_reblock_orphans(&native_path);
    let binding = match prepare_native_reblock_source(&native_path, &source_path, &binding_path) {
        NativeReblockPreparation::Ready(binding) => binding,
        NativeReblockPreparation::AlreadyCommitted => {
            eprintln!("native trace reblock was already committed for {display}");
            return;
        }
    };
    let source_bytes = binding.source_bytes;
    let started = Instant::now();
    let mut source = BinaryTraceReader::open(&source_path);
    let footer = source.footer;
    validate_binary_trace_footer(&footer).unwrap_or_else(|error| {
        panic!(
            "native parity trace reblock source {} has an invalid footer: {error}",
            source_path.display()
        )
    });
    let header = source.read_header();
    assert_eq!(
        header.version, TRACE_NATIVE_VERSION,
        "native parity trace reblock source has version {}",
        header.version
    );
    let mut source_digest = Sha256::new();
    update_native_semantic_digest(&mut source_digest, &header);
    let mut timeline = TraceTimeline::new(header.trace.initial_frame);
    let mut frame_count = 0_u64;

    let mut temporary = tempfile::Builder::new()
        .prefix(&native_reblock_temporary_prefix(&native_path, false))
        .tempfile_in(parent)
        .unwrap_or_else(|error| {
            panic!("create temporary reblocked native trace beside {display}: {error}")
        });
    {
        let mut encoder = zstd::stream::write::Encoder::new(
            BufWriter::new(temporary.as_file_mut()),
            TRACE_NATIVE_ZSTD_LEVEL,
        )
        .unwrap_or_else(|error| panic!("start native trace reblock compression: {error}"));
        configure_cache_compression(&mut encoder, None);
        write_binary_record(&mut encoder, &header, "reblocked native trace header");
        let mut block = Vec::with_capacity(TRACE_NATIVE_BLOCK_RECORDS);
        loop {
            let record = source.read_record();
            update_native_semantic_digest(&mut source_digest, &record);
            let terminal = match &record {
                BinaryTraceRecord::Frame(frame) => {
                    timeline
                        .observe(frame.frame_before, frame.frame_after)
                        .unwrap_or_else(|error| {
                            panic!("native trace reblock source timeline is invalid: {error}")
                        });
                    frame_count += 1;
                    None
                }
                BinaryTraceRecord::End {
                    rng_suffix,
                    final_frame,
                    frame_count: terminal_frame_count,
                } => {
                    assert!(
                        rng_suffix.is_some(),
                        "native trace reblock source lost RNG suffix"
                    );
                    Some((
                        terminal_frame_count
                            .expect("native trace reblock source lost terminal frame count"),
                        final_frame.expect("native trace reblock source lost final frame"),
                    ))
                }
            };
            block.push(record);
            if block.len() == TRACE_NATIVE_BLOCK_RECORDS || terminal.is_some() {
                write_binary_record(
                    &mut encoder,
                    block.as_slice(),
                    "reblocked native trace frame block",
                );
                block.clear();
            }
            if let Some((terminal_frame_count, final_frame)) = terminal {
                assert_eq!(terminal_frame_count, frame_count);
                timeline
                    .validate_terminator(terminal_frame_count, final_frame)
                    .unwrap_or_else(|error| {
                        panic!("native trace reblock terminator is invalid: {error}")
                    });
                source
                    .validate_terminator(terminal_frame_count, final_frame)
                    .unwrap_or_else(|error| {
                        panic!("native trace reblock source footer is invalid: {error}")
                    });
                break;
            }
        }
        let mut writer = encoder
            .finish()
            .unwrap_or_else(|error| panic!("finish native trace reblock compression: {error}"));
        write_binary_trace_footer(&mut writer, footer)
            .unwrap_or_else(|error| panic!("write reblocked native trace footer: {error}"));
        writer
            .flush()
            .unwrap_or_else(|error| panic!("flush reblocked native trace: {error}"));
    }
    temporary
        .as_file()
        .sync_all()
        .unwrap_or_else(|error| panic!("sync reblocked native trace: {error}"));
    let expected_digest = source_digest.finalize();
    assert_eq!(
        sha256_hex(&expected_digest),
        binding.source_semantic_sha256,
        "native trace reblock source changed after its authenticated binding was published"
    );
    let (decoded_frames, actual_digest) = digest_and_validate_native_trace(temporary.path());
    assert_eq!(decoded_frames, frame_count);
    assert_eq!(
        actual_digest, expected_digest,
        "temporary reblocked native trace for {display} differs semantically from its recovery source"
    );
    temporary.persist(&native_path).unwrap_or_else(|error| {
        panic!(
            "atomically publish reblocked native trace {display}: {}",
            error.error
        )
    });
    sync_directory(parent, "reblocked native trace publication");
    std::fs::remove_file(&source_path).unwrap_or_else(|error| {
        panic!(
            "remove verified native trace reblock source {}: {error}",
            source_path.display()
        )
    });
    std::fs::remove_file(&binding_path).unwrap_or_else(|error| {
        panic!(
            "remove verified native trace reblock binding {}: {error}",
            binding_path.display()
        )
    });
    sync_directory(parent, "verified native trace reblock commit");
    let output_bytes = std::fs::metadata(&native_path)
        .expect("stat reblocked native parity trace")
        .len();
    eprintln!(
        "reblocked {display}: {frame_count} frames, {:.2} MiB -> {:.2} MiB in {:.1}s ({} records/block, {} MiB zstd window)",
        source_bytes as f64 / (1024.0 * 1024.0),
        output_bytes as f64 / (1024.0 * 1024.0),
        started.elapsed().as_secs_f64(),
        TRACE_NATIVE_BLOCK_RECORDS,
        1_u64 << (TRACE_NATIVE_WINDOW_LOG - 20),
    );
}

/// Read and semantically validate a native trace without running the replay.
/// This is intentionally read-only so a migration operator can compare old
/// and reblocked artifacts by digest, elapsed time, and peak process RSS.
fn validate_native_trace(trace_path: &Path) {
    let native_path = requested_native_trace_path(trace_path);
    let started = Instant::now();
    let (frame_count, digest) = digest_and_validate_native_trace(&native_path);
    eprintln!(
        "validated {}: {frame_count} frames, semantic_sha256={}, {:.1}s",
        native_path.display(),
        sha256_hex(&digest),
        started.elapsed().as_secs_f64(),
    );
}

fn digest_and_validate_native_trace(path: &Path) -> (u64, sha2::digest::Output<Sha256>) {
    let mut reader = BinaryTraceReader::open(path);
    validate_binary_trace_footer(&reader.footer)
        .unwrap_or_else(|error| panic!("reblocked native trace footer is invalid: {error}"));
    let header = reader.read_header();
    assert_eq!(header.version, TRACE_NATIVE_VERSION);
    let mut digest = Sha256::new();
    update_native_semantic_digest(&mut digest, &header);
    let mut timeline = TraceTimeline::new(header.trace.initial_frame);
    let mut decoded_frames = 0_u64;
    loop {
        let record = reader.read_record();
        update_native_semantic_digest(&mut digest, &record);
        match record {
            BinaryTraceRecord::Frame(frame) => {
                timeline
                    .observe(frame.frame_before, frame.frame_after)
                    .unwrap_or_else(|error| panic!("reblocked native trace timeline: {error}"));
                decoded_frames += 1;
            }
            BinaryTraceRecord::End {
                rng_suffix,
                final_frame,
                frame_count,
            } => {
                assert!(
                    rng_suffix.is_some(),
                    "reblocked native trace lost RNG suffix"
                );
                let frame_count = frame_count.expect("reblocked native trace lost frame count");
                let final_frame = final_frame.expect("reblocked native trace lost final frame");
                assert_eq!(frame_count, decoded_frames);
                timeline
                    .validate_terminator(frame_count, final_frame)
                    .unwrap_or_else(|error| panic!("reblocked native trace terminator: {error}"));
                reader
                    .validate_terminator(frame_count, final_frame)
                    .unwrap_or_else(|error| panic!("reblocked native trace footer: {error}"));
                return (decoded_frames, digest.finalize());
            }
        }
    }
}

fn ensure_native_binary_trace(trace_path: &std::path::Path) -> PathBuf {
    if trace_path
        .as_os_str()
        .to_string_lossy()
        .ends_with(TRACE_NATIVE_SUFFIX)
    {
        // The native trace itself was passed; it is the artifact, not a
        // derivation of one.
        validate_standalone_native_trace(trace_path);
        return trace_path.to_owned();
    }
    let native_path = native_binary_trace_path(trace_path);
    if !trace_path.exists() {
        assert!(
            native_path.is_file(),
            "parity trace {} does not exist and has no native counterpart {}",
            trace_path.display(),
            native_path.display()
        );
        // The recording was converted and deleted; the native trace is the
        // authoritative replacement.
        validate_standalone_native_trace(&native_path);
        return native_path;
    }
    let _generation_lock = lock_native_trace_generation(&native_path);
    let fingerprint = trace_source_fingerprint(trace_path);
    ensure_native_binary_trace_locked(trace_path, &native_path, fingerprint)
}

fn ensure_native_binary_trace_locked(
    trace_path: &Path,
    native_path: &Path,
    fingerprint: String,
) -> PathBuf {
    match try_read_binary_trace_header(&native_path) {
        Ok(header)
            if header.version == TRACE_NATIVE_VERSION
                && header.source_fingerprint == fingerprint =>
        {
            let footer = read_binary_trace_footer(&native_path).unwrap_or_else(|error| {
                panic!(
                    "native parity trace {} has a corrupt or missing fixed footer: {error}; remove the derived native file and retry (its JSONL source is still present)",
                    native_path.display()
                )
            });
            validate_binary_trace_footer(&footer).unwrap_or_else(|error| {
                panic!(
                    "native parity trace {} has an invalid fixed footer: {error}; remove the derived native file and retry (its JSONL source is still present)",
                    native_path.display()
                )
            });
            eprintln!("loaded native parity trace {}", native_path.display());
            return native_path.to_owned();
        }
        Ok(header) => eprintln!(
            "rebuilding stale native parity trace {} (version {}, fingerprint {:?})",
            native_path.display(),
            header.version,
            header.source_fingerprint
        ),
        Err(error) if native_path.exists() => panic!(
            "native parity trace {} is unreadable: {error}; remove the derived native file and retry (its JSONL source is still present)",
            native_path.display()
        ),
        Err(_) => eprintln!(
            "building native parity trace {} with bitcode + zstd level {TRACE_NATIVE_ZSTD_LEVEL}",
            native_path.display()
        ),
    }

    let mut lines = open_jsonl_trace(trace_path).lines();
    let header_line = lines
        .next()
        .expect("parity trace has no header")
        .expect("read parity trace header");
    let trace: TraceHeader = serde_json::from_str(&header_line).expect("parse parity trace header");
    verify_trace_line_roundtrip(&trace, &header_line, 1);
    validate_trace_header(&trace);
    let rng_prefix_line = lines
        .next()
        .expect("parity trace has no RNG prefix")
        .expect("read parity RNG prefix");
    let rng_prefix: TraceRngPrefix =
        serde_json::from_str(&rng_prefix_line).expect("parse parity RNG prefix");
    verify_trace_line_roundtrip(&rng_prefix, &rng_prefix_line, 2);
    assert_eq!(
        rng_prefix.r#type, "rng_prefix",
        "invalid RNG prefix record type"
    );
    rng_prefix.draws.validate();
    let header = BinaryTraceHeader {
        version: TRACE_NATIVE_VERSION,
        source_fingerprint: fingerprint,
        trace,
        rng_prefix,
    };

    let parent = match native_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let mut temporary = tempfile::NamedTempFile::new_in(parent).unwrap_or_else(|error| {
        panic!(
            "create temporary native parity trace beside {}: {error}",
            native_path.display()
        )
    });
    let started = std::time::Instant::now();
    let mut frame_count = 0_u64;
    let mut trace_timeline = TraceTimeline::new(header.trace.initial_frame);
    // The closure returns the terminal frame purely so the scope's value
    // documents a completed conversion; the footer consumed it inside.
    let _final_frame = std::thread::scope(|scope| {
        let audit_sender = spawn_roundtrip_audit_workers(scope);
        let mut encoder = zstd::stream::write::Encoder::new(
            BufWriter::new(temporary.as_file_mut()),
            TRACE_NATIVE_ZSTD_LEVEL,
        )
        .unwrap_or_else(|error| panic!("start native parity trace compression: {error}"));
        configure_cache_compression(&mut encoder, expected_native_stream_bytes(trace_path));
        write_binary_record(&mut encoder, &header, "native parity trace header");

        let mut records = lines.enumerate();
        let mut terminal_metadata = None;
        let mut block: Vec<BinaryTraceRecord> = Vec::with_capacity(TRACE_NATIVE_BLOCK_RECORDS);
        while let Some((record_index, line)) = records.next() {
            let line_number = record_index + 3;
            let line = line.unwrap_or_else(|error| {
                panic!("read parity trace record on line {line_number}: {error}")
            });
            audit_sender
                .send((line_number, line.clone()))
                .expect("parity round-trip audit workers stopped early");
            if let Some(frame) = parse_trace_frame(&line, line_number) {
                validate_trace_frame_envelope(header.trace.schema, &frame);
                trace_timeline
                    .observe(frame.frame_before, frame.frame_after)
                    .unwrap_or_else(|error| {
                        panic!("invalid parity frame timeline on line {line_number}: {error}")
                    });
                block.push(BinaryTraceRecord::Frame(frame));
                if block.len() >= TRACE_NATIVE_BLOCK_RECORDS {
                    write_binary_record(&mut encoder, block.as_slice(), "parity trace frame block");
                    block.clear();
                }
                frame_count += 1;
                if frame_count.is_multiple_of(500) {
                    eprintln!("cached {frame_count} parity frames");
                }
            } else {
                let suffix: TraceRngOnly = serde_json::from_str(&line).unwrap_or_else(|error| {
                    panic!("parse RNG suffix on trace line {line_number}: {error}")
                });
                assert_eq!(
                    suffix.record_type, "rng_suffix",
                    "invalid parity terminator record type on line {line_number}"
                );
                suffix.draws.validate();
                trace_timeline
                    .validate_terminator(suffix.frame_count, suffix.final_frame)
                    .unwrap_or_else(|error| {
                        panic!("invalid parity terminator timeline on line {line_number}: {error}")
                    });
                block.push(BinaryTraceRecord::End {
                    rng_suffix: Some(suffix.draws),
                    final_frame: Some(suffix.final_frame),
                    frame_count: Some(suffix.frame_count),
                });
                terminal_metadata = Some((suffix.frame_count, suffix.final_frame));
                if let Some((trailing_index, trailing)) = records.next() {
                    let trailing_line = trailing_index + 3;
                    trailing.unwrap_or_else(|error| {
                        panic!("read trailing parity trace line {trailing_line}: {error}")
                    });
                    panic!(
                        "parity trace has a record after its rng_suffix terminator on line {trailing_line}"
                    );
                }
                break;
            }
        }
        let (terminal_frame_count, terminal_final_frame) = terminal_metadata.unwrap_or_else(|| {
            panic!("parity trace ended without an rng_suffix terminator; refusing to publish the native trace")
        });
        assert_eq!(terminal_frame_count, frame_count);
        let final_frame = terminal_final_frame;
        write_binary_record(
            &mut encoder,
            block.as_slice(),
            "native parity trace final block",
        );
        let mut writer = encoder
            .finish()
            .unwrap_or_else(|error| panic!("finish native parity trace compression: {error}"));
        write_binary_trace_footer(
            &mut writer,
            BinaryTraceFooter {
                version: TRACE_NATIVE_VERSION,
                frame_count,
                final_frame,
            },
        )
        .unwrap_or_else(|error| panic!("write native parity trace fixed footer: {error}"));
        writer
            .flush()
            .unwrap_or_else(|error| panic!("flush native parity trace: {error}"));
        final_frame
    });
    temporary
        .as_file()
        .sync_all()
        .unwrap_or_else(|error| panic!("sync native parity trace: {error}"));
    temporary.persist(&native_path).unwrap_or_else(|error| {
        panic!(
            "persist native parity trace {}: {}",
            native_path.display(),
            error.error
        )
    });
    sync_directory(parent, "native parity trace publication");
    let compressed_bytes = std::fs::metadata(&native_path)
        .expect("stat completed native parity trace")
        .len();
    eprintln!(
        "cached {frame_count} frames in {} ({:.1} MiB, {:.1}s)",
        native_path.display(),
        compressed_bytes as f64 / (1024.0 * 1024.0),
        started.elapsed().as_secs_f64()
    );
    native_path.to_owned()
}

impl BinaryTraceReader {
    fn open(path: &std::path::Path) -> Self {
        let footer = read_binary_trace_footer(path).unwrap_or_else(|error| {
            panic!(
                "read native parity trace fixed footer {}: {error}",
                path.display()
            )
        });
        let file = File::open(path)
            .unwrap_or_else(|error| panic!("open native parity trace {}: {error}", path.display()));
        let compressed_len = file
            .metadata()
            .expect("stat native parity trace before decompression")
            .len()
            .checked_sub(TRACE_NATIVE_FOOTER_LEN)
            .expect("validated native parity trace is shorter than its footer");
        // The fixed footer is outside the zstd stream so it can be checked
        // without decoding a potentially ABI-incompatible file. Bound zstd
        // to the compressed bytes or it treats the footer as another frame.
        let mut decoder = zstd::stream::read::Decoder::new(file.take(compressed_len))
            .unwrap_or_else(|error| {
                panic!(
                    "start native parity trace decompression {}: {error}",
                    path.display()
                )
            });
        decoder
            .window_log_max(TRACE_ZSTD_WINDOW_LOG_MAX)
            .unwrap_or_else(|error| {
                panic!(
                    "configure native parity trace decompression {}: {error}",
                    path.display()
                )
            });
        Self {
            path: path.to_owned(),
            reader: Box::new(decoder),
            footer,
            pending: VecDeque::new(),
        }
    }

    fn read_header(&mut self) -> BinaryTraceHeader {
        read_binary_record(&mut self.reader, "native parity trace header").unwrap_or_else(|error| {
            panic!(
                "read native parity trace header {}: {error}",
                self.path.display()
            )
        })
    }

    fn read_record(&mut self) -> BinaryTraceRecord {
        if let Some(record) = self.pending.pop_front() {
            return record;
        }
        let block: Vec<BinaryTraceRecord> =
            read_binary_record(&mut self.reader, "native parity trace block").unwrap_or_else(
                |error| {
                    panic!(
                        "read native parity trace block {}: {error}",
                        self.path.display()
                    )
                },
            );
        self.pending.extend(block);
        self.pending.pop_front().unwrap_or_else(|| {
            panic!(
                "native parity trace {} contains an empty record block",
                self.path.display()
            )
        })
    }

    fn validate_terminator(&mut self, frame_count: u64, final_frame: u64) -> Result<(), String> {
        if self.footer.frame_count != frame_count || self.footer.final_frame != final_frame {
            return Err(format!(
                "decoded terminator says frame_count={frame_count} final_frame={final_frame}, fixed footer says frame_count={} final_frame={}",
                self.footer.frame_count, self.footer.final_frame
            ));
        }
        if !self.pending.is_empty() {
            return Err(
                "decoded native trace contains records after its first End record in the same block"
                    .to_owned(),
            );
        }
        let mut trailing = [0_u8; 1];
        match self.reader.read(&mut trailing) {
            Ok(0) => Ok(()),
            Ok(_) => {
                Err("decoded native trace contains data after its first End record".to_owned())
            }
            Err(error) => Err(format!(
                "decompress cache after its first End record: {error}"
            )),
        }
    }
}

fn write_binary_trace_footer(
    writer: &mut impl Write,
    footer: BinaryTraceFooter,
) -> std::io::Result<()> {
    writer.write_all(&TRACE_NATIVE_FOOTER_MAGIC)?;
    writer.write_all(&footer.version.to_le_bytes())?;
    writer.write_all(&footer.frame_count.to_le_bytes())?;
    writer.write_all(&footer.final_frame.to_le_bytes())?;
    Ok(())
}

fn read_binary_trace_footer(path: &Path) -> Result<BinaryTraceFooter, String> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let length = file.metadata().map_err(|error| error.to_string())?.len();
    if length < TRACE_NATIVE_FOOTER_LEN {
        return Err(format!(
            "file is {length} bytes, shorter than the {TRACE_NATIVE_FOOTER_LEN}-byte footer"
        ));
    }
    file.seek(SeekFrom::End(
        -i64::try_from(TRACE_NATIVE_FOOTER_LEN).unwrap(),
    ))
    .map_err(|error| error.to_string())?;
    let mut magic = [0_u8; TRACE_NATIVE_FOOTER_MAGIC.len()];
    file.read_exact(&mut magic)
        .map_err(|error| error.to_string())?;
    if magic != TRACE_NATIVE_FOOTER_MAGIC {
        return Err(format!("footer magic is {magic:?}"));
    }
    let mut version = [0_u8; 4];
    let mut frame_count = [0_u8; 8];
    let mut final_frame = [0_u8; 8];
    file.read_exact(&mut version)
        .map_err(|error| error.to_string())?;
    file.read_exact(&mut frame_count)
        .map_err(|error| error.to_string())?;
    file.read_exact(&mut final_frame)
        .map_err(|error| error.to_string())?;
    Ok(BinaryTraceFooter {
        version: u32::from_le_bytes(version),
        frame_count: u64::from_le_bytes(frame_count),
        final_frame: u64::from_le_bytes(final_frame),
    })
}

fn validate_binary_trace_footer(footer: &BinaryTraceFooter) -> Result<(), String> {
    if footer.version != TRACE_NATIVE_VERSION {
        return Err(format!(
            "footer version {} does not match runner version {TRACE_NATIVE_VERSION}",
            footer.version
        ));
    }
    Ok(())
}

fn try_read_binary_trace_header(path: &std::path::Path) -> Result<BinaryTraceHeader, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    let mut decoder = zstd::stream::read::Decoder::new(file).map_err(|error| error.to_string())?;
    decoder
        .window_log_max(TRACE_ZSTD_WINDOW_LOG_MAX)
        .map_err(|error| error.to_string())?;
    read_binary_record(&mut decoder, "native parity trace header")
}

/// A compression window large enough for short encoded streams and otherwise
/// capped at [`TRACE_NATIVE_WINDOW_LOG`]. A wider zstd frame makes every
/// replay decoder retain that history even though parity consumes records
/// strictly once and in order. The bounded window is therefore part of the
/// replay lane's memory contract, not merely an encoder tuning parameter.
fn native_stream_window_log(expected_bytes: Option<u64>) -> u32 {
    let Some(bytes) = expected_bytes else {
        return TRACE_NATIVE_WINDOW_LOG;
    };
    // ceil(log2(bytes)): the smallest window that still spans the estimate.
    let spanning = u64::BITS
        - bytes
            .max(1)
            .checked_next_power_of_two()
            .unwrap_or(u64::MAX)
            .leading_zeros()
        - 1;
    // Below ~1 MiB the window stops being what limits matching, and a stream
    // that outgrows the estimate only loses long-range matches, never data.
    spanning.clamp(20, TRACE_NATIVE_WINDOW_LOG)
}

/// How much bitcode a recording is expected to encode into. Measured traces
/// pack to roughly a fifth of their JSONL; a quarter leaves room for one that
/// packs worse. `None` when the recording's uncompressed size is unknown, and
/// the window then falls back to the maximum.
fn expected_native_stream_bytes(trace_path: &std::path::Path) -> Option<u64> {
    Some(recording_uncompressed_bytes(trace_path)? / 4)
}

/// The uncompressed byte count of a JSONL recording: the file length, or the
/// content size a `.zst` recording declares in its frame header.
fn recording_uncompressed_bytes(trace_path: &std::path::Path) -> Option<u64> {
    let length = std::fs::metadata(trace_path).ok()?.len();
    if !trace_path.as_os_str().to_string_lossy().ends_with(".zst") {
        return Some(length);
    }
    // A zstd frame header is at most 18 bytes.
    let mut header = [0_u8; 18];
    let read = {
        let mut file = File::open(trace_path).ok()?;
        read_up_to(&mut file, &mut header).ok()?
    };
    // An unknown content size is reported as an error or as zero; either way
    // the caller falls back to the widest window rather than guessing small.
    match zstd::zstd_safe::get_frame_content_size(&header[..read]) {
        Ok(Some(size)) if size > 0 => Some(size),
        _ => None,
    }
}

/// Fill `buffer` as far as the reader allows, returning how much was read.
fn read_up_to<R: Read>(reader: &mut R, buffer: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match reader.read(&mut buffer[filled..])? {
            0 => break,
            read => filled += read,
        }
    }
    Ok(filled)
}

fn configure_cache_compression<W: Write>(
    encoder: &mut zstd::stream::write::Encoder<'_, W>,
    expected_bytes: Option<u64>,
) {
    if TRACE_NATIVE_LONG_DISTANCE_MATCHING {
        encoder
            .long_distance_matching(true)
            .unwrap_or_else(|error| panic!("enable cache long-distance matching: {error}"));
    }
    encoder
        .window_log(native_stream_window_log(expected_bytes))
        .unwrap_or_else(|error| panic!("configure cache compression window: {error}"));
}

fn read_binary_trace_header(path: &std::path::Path) -> BinaryTraceHeader {
    try_read_binary_trace_header(path).unwrap_or_else(|error| {
        panic!(
            "read native parity trace header {} after conversion: {error}",
            path.display()
        )
    })
}

fn write_binary_record<T: bitcode::Encode + ?Sized>(
    writer: &mut impl Write,
    value: &T,
    label: &str,
) {
    let encoded = bitcode::encode(value);
    let length = u64::try_from(encoded.len()).expect("binary record length exceeds u64");
    writer
        .write_all(&length.to_le_bytes())
        .unwrap_or_else(|error| panic!("write {label} length: {error}"));
    writer
        .write_all(&encoded)
        .unwrap_or_else(|error| panic!("write {label}: {error}"));
}

fn read_binary_record<T: bitcode::DecodeOwned>(
    reader: &mut dyn Read,
    label: &str,
) -> Result<T, String> {
    const MAX_RECORD_BYTES: u64 = 1024 * 1024 * 1024;
    let mut length_bytes = [0_u8; 8];
    reader
        .read_exact(&mut length_bytes)
        .map_err(|error| format!("read {label} length: {error}"))?;
    let length = u64::from_le_bytes(length_bytes);
    if length > MAX_RECORD_BYTES {
        return Err(format!(
            "{label} length {length} exceeds {MAX_RECORD_BYTES}-byte safety limit"
        ));
    }
    let length = usize::try_from(length)
        .map_err(|_| format!("{label} length cannot be represented on this platform"))?;
    let mut encoded = vec![0_u8; length];
    reader
        .read_exact(&mut encoded)
        .map_err(|error| format!("read {label} payload: {error}"))?;
    bitcode::decode(&encoded).map_err(|error| format!("decode {label}: {error}"))
}

/// Storage experiment for the canonical-trace-format decision: re-encode the
/// cached records of one trace with bincode and bitcode in several layouts
/// and report raw and zstd-compressed sizes. Layouts are compared on the same
/// decoded records, so the only variable is the encoding.
///
/// `PARITY_BENCH_ZSTD_LEVELS` (default `3,19`) and `PARITY_BENCH_BLOCKS`
/// (default `16,64,256`) tune the sweep.
fn bench_trace_encodings(trace_path: &Path) {
    fn env_list(name: &str, default: &str) -> Vec<usize> {
        std::env::var(name)
            .unwrap_or_else(|_| default.to_owned())
            .split(',')
            .map(|item| {
                item.trim()
                    .parse()
                    .unwrap_or_else(|error| panic!("parse {name} item {item:?}: {error}"))
            })
            .collect()
    }

    fn zstd_size(bytes: &[u8], level: i32) -> (usize, Duration) {
        let started = Instant::now();
        let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), level)
            .unwrap_or_else(|error| panic!("start zstd level {level}: {error}"));
        configure_cache_compression(&mut encoder, Some(bytes.len() as u64));
        encoder
            .write_all(bytes)
            .unwrap_or_else(|error| panic!("zstd level {level}: {error}"));
        let compressed = encoder
            .finish()
            .unwrap_or_else(|error| panic!("finish zstd level {level}: {error}"));
        (compressed.len(), started.elapsed())
    }

    fn bincode_record<T: bincode::Encode>(out: &mut Vec<u8>, value: &T) {
        let encoded = bincode::encode_to_vec(value, bincode::config::standard())
            .unwrap_or_else(|error| panic!("bincode: {error}"));
        out.extend_from_slice(&(encoded.len() as u64).to_le_bytes());
        out.extend_from_slice(&encoded);
    }

    fn bitcode_record<T: bitcode::Encode + ?Sized>(out: &mut Vec<u8>, value: &T) {
        let encoded = bitcode::encode(value);
        out.extend_from_slice(&(encoded.len() as u64).to_le_bytes());
        out.extend_from_slice(&encoded);
    }

    let zstd_levels: Vec<i32> = env_list("PARITY_BENCH_ZSTD_LEVELS", "3,19")
        .into_iter()
        .map(|level| i32::try_from(level).expect("zstd level fits i32"))
        .collect();
    let block_sizes = env_list("PARITY_BENCH_BLOCKS", "16,64,256");

    let native_path = ensure_native_binary_trace(trace_path);
    let source_bytes = std::fs::metadata(trace_path)
        .expect("stat source trace")
        .len();
    let cache_bytes = std::fs::metadata(&native_path)
        .expect("stat trace cache")
        .len();

    let started = Instant::now();
    let mut reader = BinaryTraceReader::open(&native_path);
    let header = reader.read_header();
    let mut records = Vec::new();
    loop {
        let record = reader.read_record();
        let is_end = matches!(record, BinaryTraceRecord::End { .. });
        records.push(record);
        if is_end {
            break;
        }
    }
    let frame_count = records.len() - 1;
    eprintln!(
        "loaded {frame_count} frames from {} in {:.1}s",
        native_path.display(),
        started.elapsed().as_secs_f64()
    );

    // Viability probe. bitcode's *serde* backend panics ("type changed") on
    // `TraceJsonValue`, because `#[serde(untagged)]` values of differing
    // shapes share one sequence/map slot. The native derives handle enums
    // properly, so that is what a real switch would use; confirm the native
    // derives round-trip a frame and time a whole-trace decode.
    if let Some(first_frame) = records.first() {
        let encoded = bitcode::encode(first_frame);
        bitcode::decode::<BinaryTraceRecord>(&encoded)
            .expect("bitcode native round-trip of a frame");
        eprintln!("bitcode native round-trip of a frame: ok");
    }

    let mut rows: Vec<(String, Vec<u8>, Duration)> = Vec::new();

    let started = Instant::now();
    let mut out = Vec::new();
    bincode_record(&mut out, &header);
    for record in &records {
        bincode_record(&mut out, record);
    }
    rows.push((
        "bincode per-record (former cache)".into(),
        out,
        started.elapsed(),
    ));

    let started = Instant::now();
    let mut out = Vec::new();
    bitcode_record(&mut out, &header);
    for record in &records {
        bitcode_record(&mut out, record);
    }
    rows.push(("bitcode per-record".into(), out, started.elapsed()));

    for &block in &block_sizes {
        let started = Instant::now();
        let mut out = Vec::new();
        bitcode_record(&mut out, &header);
        for chunk in records.chunks(block) {
            bitcode_record(&mut out, chunk);
        }
        let marker = if block == TRACE_NATIVE_BLOCK_RECORDS {
            " (current cache)"
        } else {
            ""
        };
        rows.push((
            format!("bitcode blocks of {block}{marker}"),
            out,
            started.elapsed(),
        ));
    }

    // bitcode has no `Encode` for references, so the whole-trace layouts
    // encode an owned tuple; both whole-trace decodes are timed for the
    // "one block" discussion (they must materialize every frame).
    let whole = (header, records);

    let started = Instant::now();
    let out =
        bincode::encode_to_vec(&whole, bincode::config::standard()).expect("bincode whole trace");
    let encode_time = started.elapsed();
    let started = Instant::now();
    let (decoded, _): ((BinaryTraceHeader, Vec<BinaryTraceRecord>), usize) =
        bincode::decode_from_slice(&out, bincode::config::standard())
            .expect("bincode decode whole trace");
    eprintln!(
        "bincode whole-trace decode: {} records in {:.2}s",
        decoded.1.len(),
        started.elapsed().as_secs_f64()
    );
    drop(decoded);
    rows.push(("bincode whole-trace".into(), out, encode_time));

    let started = Instant::now();
    let out = bitcode::encode(&whole);
    let encode_time = started.elapsed();
    let started = Instant::now();
    let decoded: (BinaryTraceHeader, Vec<BinaryTraceRecord>) =
        bitcode::decode(&out).expect("bitcode decode whole trace");
    eprintln!(
        "bitcode whole-trace decode: {} records in {:.2}s",
        decoded.1.len(),
        started.elapsed().as_secs_f64()
    );
    drop(decoded);
    rows.push(("bitcode whole-trace".into(), out, encode_time));

    let baseline_raw = rows[0].1.len();
    let mut results: Vec<(String, Duration, usize, Vec<(i32, usize, Duration)>)> = Vec::new();
    for (name, bytes, encode_time) in &rows {
        let mut compressed = Vec::new();
        for &level in &zstd_levels {
            let (size, time) = zstd_size(bytes, level);
            compressed.push((level, size, time));
        }
        eprintln!("measured {name}");
        results.push((name.clone(), *encode_time, bytes.len(), compressed));
    }

    let mib = |bytes: usize| bytes as f64 / (1024.0 * 1024.0);
    println!();
    println!("trace: {} ({} frames)", trace_path.display(), frame_count);
    println!(
        "source jsonl.zst: {:.2} MiB; existing cache (bitcode + zstd {TRACE_NATIVE_ZSTD_LEVEL}): {:.2} MiB",
        mib(source_bytes as usize),
        mib(cache_bytes as usize)
    );
    println!();
    let mut head = format!(
        "| {:<36} | {:>10} | {:>8} |",
        "encoding", "raw MiB", "enc s"
    );
    let mut sep = format!("|{:-<38}|{:->12}|{:->10}|", "", "", "");
    for level in &zstd_levels {
        head.push_str(&format!(
            " {:>13} | {:>8} |",
            format!("zstd-{level} MiB"),
            "zstd s"
        ));
        sep.push_str(&format!("{:->15}|{:->10}|", "", ""));
    }
    println!("{head}");
    println!("{sep}");
    for (name, encode_time, raw, compressed) in &results {
        let mut line = format!(
            "| {:<36} | {:>5.2} {:>4.0}% | {:>8.2} |",
            name,
            mib(*raw),
            100.0 * *raw as f64 / baseline_raw as f64,
            encode_time.as_secs_f64()
        );
        for (index, (_, size, time)) in compressed.iter().enumerate() {
            let baseline = results[0].3[index].1;
            line.push_str(&format!(
                " {:>6.2} {:>5.0}% | {:>8.2} |",
                mib(*size),
                100.0 * *size as f64 / baseline as f64,
                time.as_secs_f64()
            ));
        }
        println!("{line}");
    }
    println!();
    println!(
        "percentages are relative to the current per-record bincode layout at the same zstd level"
    );
}

fn read_all_rng_draws(native_path: &std::path::Path) -> Vec<u32> {
    let mut reader = BinaryTraceReader::open(native_path);
    let header = reader.read_header();
    let mut result = Vec::new();
    let mut original_index = 0_usize;
    let mut frame_count = 0_u64;
    let mut trace_timeline = TraceTimeline::new(header.trace.initial_frame);
    append_simulation_rng_draws(&mut result, &mut original_index, &header.rng_prefix.draws);
    loop {
        match reader.read_record() {
            BinaryTraceRecord::Frame(frame) => {
                trace_timeline
                    .observe(frame.frame_before, frame.frame_after)
                    .unwrap_or_else(|error| {
                        panic!("invalid cached parity frame timeline during RNG pre-scan: {error}")
                    });
                append_simulation_rng_draws(&mut result, &mut original_index, &frame.rng_draws);
                frame_count += 1;
            }
            BinaryTraceRecord::End {
                rng_suffix,
                final_frame,
                frame_count: terminal_frame_count,
            } => {
                if let Some(batch) = rng_suffix {
                    append_simulation_rng_draws(&mut result, &mut original_index, &batch);
                }
                let terminal_frame_count = terminal_frame_count
                    .unwrap_or_else(|| panic!("parity cache RNG pre-scan found incomplete End"));
                let final_frame = final_frame
                    .unwrap_or_else(|| panic!("parity cache RNG pre-scan found incomplete End"));
                assert_eq!(terminal_frame_count, frame_count);
                trace_timeline
                    .validate_terminator(terminal_frame_count, final_frame)
                    .unwrap_or_else(|error| {
                        panic!(
                            "invalid cached parity terminator timeline during RNG pre-scan: {error}"
                        )
                    });
                reader
                    .validate_terminator(terminal_frame_count, final_frame)
                    .unwrap_or_else(|error| {
                        panic!(
                            "native parity trace {} has an invalid terminal record during RNG pre-scan: {error}",
                            native_path.display()
                        )
                    });
                break;
            }
        }
    }
    eprintln!(
        "loaded {} simulation RNG draws from {}",
        result.len(),
        native_path.display()
    );
    result
}

fn append_simulation_rng_draws(
    result: &mut Vec<u32>,
    original_index: &mut usize,
    batch: &TraceRngBatch,
) {
    assert_eq!(batch.first_index, *original_index, "RNG stream has a gap");
    batch.validate();
    *original_index += batch.values.len();
    result.extend(
        batch
            .values
            .iter()
            .copied()
            .zip(batch.domains.iter().copied())
            .filter_map(|(value, domain)| (domain == TraceRngDomain::Simulation).then_some(value)),
    );
}

fn simulation_rng_draws(batch: &TraceRngBatch) -> Vec<u32> {
    batch
        .values
        .iter()
        .copied()
        .zip(batch.domains.iter().copied())
        .filter_map(|(value, domain)| (domain == TraceRngDomain::Simulation).then_some(value))
        .collect()
}

fn should_preload_complete_rng_stream(
    start_state: TraceStartState,
    prefix_draw_count: usize,
    forced: bool,
) -> bool {
    forced || (start_state == TraceStartState::LoadedSave && prefix_draw_count == 0)
}

fn difference_field(difference: &str) -> &str {
    let entity_field = difference
        .split_once(").")
        .and_then(|(_, tail)| tail.split_once(':'))
        .map(|(field, _)| field);
    entity_field
        .or_else(|| {
            difference
                .strip_prefix("frame.")
                .and_then(|tail| tail.split_once(':'))
                .map(|(field, _)| field)
        })
        .unwrap_or("other")
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
    campaign.last_played_mission_indices = trace
        .last_played_mission_indices
        .iter()
        .copied()
        .map(validate_mission_index)
        .collect();
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
                    obstacle: occupant.obstacle,
                })
                .collect(),
            amount: source.amount,
            produced_amount: source.produced_amount,
            max_amount_reached: source.max_amount_reached,
        })
        .collect();

    campaign
}

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
    assets.profile_manager = profiles.clone();
    let mut text_res = robin_assets::resource_manager::ResourceManager::new();
    text_res
        .attach_resource_file("Data/Text/Level.res")
        .expect("load Data/Text/Level.res for Original rescue-PC names");
    (assets.peasant_firstnames, assets.peasant_surnames) =
        robin_rs::game_session::load_peasant_name_pool(&mut text_res);
    assets.fixed_vip_names = robin_rs::game_session::load_fixed_vip_name_map(&mut text_res);
    let _ = text_res.attach_resource_file("Data/Interface/Start.sxt");
    let menu_text = robin_rs::ingame_menu::resources::MenuText::load(&mut text_res);
    let mut host = Host::scratch(1024.0, 768.0);
    host.frame_holder_mut()
        .initialize_sprite_bank(".")
        .expect("initialize sprite bank");
    assets.bank_signature = host.frame_holder.signature();

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
        Arc::new(robin_engine::script_manager::ScriptProgram::from_scb(
            scb.clone(),
        )),
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
    /// Original sparse `RHFastFindGrid::marraySectors` slot to Rust's compact
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
    /// cursor previews construct stack-local `RHElement` projectiles (for
    /// example the temporary arrow in `RHEngine::IsValidTrajectory`), consuming
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

    /// Preserve the patch-aware `pSectorGoal` recorded by PerformGroupMove.
    /// A retained canonical position sector is translated normally. A true
    /// unmapped constructor remains an authoritative route goal in Original's
    /// sparse identity domain: keeping
    /// its number forces the same AppendMoveToSequence gate search instead of
    /// silently substituting Rust's coincident `mpSelectedSector` overlay.
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

fn compare_visibility_queries(
    expected: &[TraceVisibilityQuery],
    actual: &[robin_engine::sight_obstacle::ParityVisibilityQuery],
) -> Vec<String> {
    let mut differences = Vec::new();
    if expected.len() != actual.len() {
        differences.push(format!(
            "frame.visibility_queries.length: original={} rust={}",
            expected.len(),
            actual.len()
        ));
    }
    for (index, (expected, actual)) in expected.iter().zip(actual).enumerate() {
        let actual_origin = actual.origin.map(f32::to_bits);
        let actual_destination = actual.destination.map(f32::to_bits);
        let expected_origin = [
            expected.origin.x.bits,
            expected.origin.y.bits,
            expected.origin.z.bits,
        ];
        let expected_destination = [
            expected.destination.x.bits,
            expected.destination.y.bits,
            expected.destination.z.bits,
        ];
        if expected_origin != actual_origin {
            differences.push(format!(
                "frame.visibility_queries[{index}].origin: original_bits={expected_origin:?} rust_bits={actual_origin:?}"
            ));
        }
        if expected_destination != actual_destination {
            differences.push(format!(
                "frame.visibility_queries[{index}].destination: original_bits={expected_destination:?} rust_bits={actual_destination:?}"
            ));
        }
        if expected.result != actual.result {
            differences.push(format!(
                "frame.visibility_queries[{index}].result: original={} rust={}",
                expected.result, actual.result
            ));
        }
    }
    differences
}

struct TracePathRequestRef<'a> {
    actor: TraceEntityId,
    antagonist: Option<TraceEntityId>,
    layer: u16,
    area: u16,
    source: &'a TracePoint,
    goal: &'a TracePoint,
    half_diagonal_index: u16,
    half_diagonal: &'a TracePoint,
    animation: u32,
    reverse: bool,
    speed: u8,
    tolerance: &'a TraceFloat,
    use_first_point: bool,
}

impl TracePathEvent {
    fn request(&self) -> TracePathRequestRef<'_> {
        match self {
            TracePathEvent::Queued {
                actor,
                antagonist,
                layer,
                area,
                source,
                goal,
                half_diagonal_index,
                half_diagonal,
                animation,
                reverse,
                speed,
                tolerance,
                use_first_point,
            }
            | TracePathEvent::Completed {
                actor,
                antagonist,
                layer,
                area,
                source,
                goal,
                half_diagonal_index,
                half_diagonal,
                animation,
                reverse,
                speed,
                tolerance,
                use_first_point,
                ..
            } => TracePathRequestRef {
                actor: *actor,
                antagonist: *antagonist,
                layer: *layer,
                area: *area,
                source,
                goal,
                half_diagonal_index: *half_diagonal_index,
                half_diagonal,
                animation: *animation,
                reverse: *reverse,
                speed: *speed,
                tolerance,
                use_first_point: *use_first_point,
            },
        }
    }
}

fn compare_path_events(
    expected: &[TracePathEvent],
    actual: &[robin_engine::pathfinder::ParityPathEvent],
    entity_map: &EntityMap,
) -> Vec<String> {
    use robin_engine::pathfinder::ParityPathEvent;

    let mut differences = Vec::new();
    if expected.len() != actual.len() {
        differences.push(format!(
            "frame.path_events.length: original={} rust={}",
            expected.len(),
            actual.len()
        ));
    }
    for (index, (expected, actual)) in expected.iter().zip(actual).enumerate() {
        let expected_phase = match expected {
            TracePathEvent::Queued { .. } => "queued",
            TracePathEvent::Completed { .. } => "completed",
        };
        let (actual_phase, actual_request) = match actual {
            ParityPathEvent::Queued(request) => ("queued", request),
            ParityPathEvent::Completed { request, .. } => ("completed", request),
        };
        if expected_phase != actual_phase {
            differences.push(format!(
                "frame.path_events[{index}].phase: original={expected_phase} rust={actual_phase}"
            ));
        }

        let expected_request = expected.request();
        let prefix = format!("frame.path_events[{index}]");
        macro_rules! compare_path_field {
            ($field:expr, $expected:expr, $actual:expr) => {
                if $expected != $actual {
                    differences.push(format!(
                        "{}.{field}: original={:?} rust={:?}",
                        prefix,
                        $expected,
                        $actual,
                        field = $field
                    ));
                }
            };
        }
        compare_path_field!(
            "actor",
            entity_map.translate(expected_request.actor),
            actual_request.actor
        );
        compare_path_field!(
            "antagonist",
            expected_request
                .antagonist
                .map(|entity| entity_map.translate(entity)),
            actual_request.antagonist
        );
        compare_path_field!("layer", expected_request.layer, actual_request.layer);
        compare_path_field!("area", expected_request.area, actual_request.area);
        compare_path_field!(
            "source.bits",
            [
                expected_request.source.x.bits,
                expected_request.source.y.bits
            ],
            [
                actual_request.source.x.to_bits(),
                actual_request.source.y.to_bits()
            ]
        );
        compare_path_field!(
            "goal.bits",
            [expected_request.goal.x.bits, expected_request.goal.y.bits],
            [
                actual_request.goal.x.to_bits(),
                actual_request.goal.y.to_bits()
            ]
        );
        compare_path_field!(
            "half_diagonal_index",
            expected_request.half_diagonal_index,
            actual_request.half_diagonal_index
        );
        compare_path_field!(
            "half_diagonal.bits",
            [
                expected_request.half_diagonal.x.bits,
                expected_request.half_diagonal.y.bits
            ],
            [
                actual_request.half_diagonal.x.to_bits(),
                actual_request.half_diagonal.y.to_bits()
            ]
        );
        compare_path_field!(
            "animation",
            expected_request.animation,
            actual_request.animation
        );
        compare_path_field!("reverse", expected_request.reverse, actual_request.reverse);
        compare_path_field!("speed", expected_request.speed, actual_request.speed);
        compare_path_field!(
            "tolerance.bits",
            expected_request.tolerance.bits,
            actual_request.tolerance.to_bits()
        );
        compare_path_field!(
            "use_first_point",
            expected_request.use_first_point,
            actual_request.use_first_point
        );

        if let (
            TracePathEvent::Completed {
                valid: expected_valid,
                waypoints: expected_waypoints,
                ..
            },
            ParityPathEvent::Completed {
                valid: actual_valid,
                waypoints: actual_waypoints,
                ..
            },
        ) = (expected, actual)
        {
            compare_path_field!("valid", *expected_valid, *actual_valid);
            compare_path_field!(
                "waypoints.length",
                expected_waypoints.len(),
                actual_waypoints.len()
            );
            for (waypoint, (expected, actual)) in
                expected_waypoints.iter().zip(actual_waypoints).enumerate()
            {
                compare_path_field!(
                    &format!("waypoints[{waypoint}].bits"),
                    [expected.x.bits, expected.y.bits],
                    [actual.x.to_bits(), actual.y.to_bits()]
                );
            }
        }
    }
    differences
}

fn campaign_comparison_value(
    campaign: &robin_engine::campaign::Campaign,
    menu_text: &dyn robin_engine::sherwood_stat::MenuTextLookup,
) -> serde_json::Value {
    let mut value = serde_json::to_value(campaign).expect("serialize campaign parity state");
    let object = value
        .as_object_mut()
        .expect("serialized Campaign must be a JSON object");

    // These fields implement Rust host-side restart checkpoints and do not
    // exist in the Original campaign object. They are verified by Rust's
    // rollback/restart tests rather than cross-engine parity.
    for field in [
        "pre_mission_snapshot",
        "pre_mission_rng_seed",
        "pre_mission_sim_config",
        "pre_mission_was_preselected",
    ] {
        object.remove(field);
    }
    if let Some(characters) = object.get_mut("characters").and_then(|v| v.as_array_mut()) {
        assert_eq!(characters.len(), campaign.characters.len());
        for (character, source) in characters.iter_mut().zip(&campaign.characters) {
            let status = character
                .get_mut("status")
                .and_then(|v| v.as_object_mut())
                .expect("serialized campaign character status must be an object");
            status.insert(
                "name".to_owned(),
                serde_json::Value::String(source.status.display_name(menu_text).into_owned()),
            );
            status.remove("name_override");
        }
    }
    if let Some(sectors) = object
        .get_mut("production_sectors")
        .and_then(|v| v.as_array_mut())
    {
        for sector in sectors {
            let sector = sector
                .as_object_mut()
                .expect("serialized production sector must be an object");
            // Runtime geometry attachments are derived from the loaded level;
            // CampaignSnapshotJson records the mutable production values and
            // occupants that the Original campaign actually serializes.
            sector.remove("script_zone");
            sector.remove("production_points");
        }
    }
    value
}

fn collect_json_differences(
    path: &str,
    expected: &serde_json::Value,
    actual: &serde_json::Value,
    differences: &mut Vec<String>,
) {
    if differences.len() >= 64 || expected == actual {
        return;
    }
    match (expected, actual) {
        (serde_json::Value::Array(expected), serde_json::Value::Array(actual)) => {
            if expected.len() != actual.len() {
                differences.push(format!(
                    "{path}.length: original={} rust={}",
                    expected.len(),
                    actual.len()
                ));
            }
            for (index, (expected, actual)) in expected.iter().zip(actual).enumerate() {
                collect_json_differences(
                    &format!("{path}[{index}]"),
                    expected,
                    actual,
                    differences,
                );
            }
        }
        (serde_json::Value::Object(expected), serde_json::Value::Object(actual)) => {
            for key in expected.keys().chain(actual.keys()) {
                match (expected.get(key), actual.get(key)) {
                    (Some(expected), Some(actual)) => collect_json_differences(
                        &format!("{path}.{key}"),
                        expected,
                        actual,
                        differences,
                    ),
                    (Some(expected), None) => differences.push(format!(
                        "{path}.{key}: original={expected:?} rust=<missing>"
                    )),
                    (None, Some(actual)) => differences
                        .push(format!("{path}.{key}: original=<missing> rust={actual:?}")),
                    (None, None) => unreachable!(),
                }
                if differences.len() >= 64 {
                    break;
                }
            }
            differences.sort();
            differences.dedup();
        }
        _ => differences.push(format!("{path}: original={expected:?} rust={actual:?}")),
    }
}

fn trace_entity_kind_name(name: &str) -> Option<TraceEntityKind> {
    Some(match name {
        "pc" => TraceEntityKind::Pc,
        "soldier" => TraceEntityKind::Soldier,
        "civilian" => TraceEntityKind::Civilian,
        "fx" => TraceEntityKind::Fx,
        "target" => TraceEntityKind::Target,
        "bonus" => TraceEntityKind::Bonus,
        "scroll" => TraceEntityKind::Scroll,
        "projectile" => TraceEntityKind::Projectile,
        "net" => TraceEntityKind::Net,
        _ => return None,
    })
}

fn entity_kind_name(kind: robin_engine::element::EntityIdKind) -> &'static str {
    match kind {
        robin_engine::element::EntityIdKind::Pc => "pc",
        robin_engine::element::EntityIdKind::Soldier => "soldier",
        robin_engine::element::EntityIdKind::Civilian => "civilian",
        robin_engine::element::EntityIdKind::Fx => "fx",
        robin_engine::element::EntityIdKind::Target => "target",
        robin_engine::element::EntityIdKind::Bonus => "bonus",
        robin_engine::element::EntityIdKind::Scroll => "scroll",
        robin_engine::element::EntityIdKind::Projectile => "projectile",
        robin_engine::element::EntityIdKind::Net => "net",
    }
}

/// Replace Original allocation identities in the schema-13 whole-engine
/// snapshots with their Rust-side isomorphic identities before structural
/// comparison. Native VM table indices and sequence ordinals are semantic and
/// intentionally remain untouched.
fn canonicalize_authoritative_snapshot(value: &mut serde_json::Value, entity_map: &EntityMap) {
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                canonicalize_authoritative_snapshot(value, entity_map);
            }
        }
        serde_json::Value::Object(object) => {
            let entity_reference = if object.len() == 2 {
                object
                    .get("kind")
                    .and_then(serde_json::Value::as_str)
                    .and_then(trace_entity_kind_name)
                    .zip(object.get("index").and_then(serde_json::Value::as_u64))
            } else {
                None
            };
            if let Some((kind, index)) = entity_reference {
                let index = u32::try_from(index)
                    .unwrap_or_else(|_| panic!("Original entity index {index} exceeds u32"));
                let id = entity_map.translate(TraceEntityId { kind, index });
                *value = serde_json::json!({
                    "kind": entity_kind_name(id.kind()),
                    "index": id.index(),
                });
                return;
            }

            for (key, child) in object {
                canonicalize_authoritative_snapshot(child, entity_map);
                if matches!(
                    key.as_str(),
                    "area" | "sector" | "sector_goal" | "sector_in" | "sector_out"
                ) && let Some(original) = child.as_i64()
                    && original >= 0
                {
                    let original = u16::try_from(original).unwrap_or_else(|_| {
                        panic!("Original snapshot sector {original} exceeds u16")
                    });
                    *child = entity_map.translate_sector(original).into();
                }
            }
        }
        _ => {}
    }
}

/// Preserve additive schema-13 coverage for older raw recordings. Fields that
/// do not exist in the recording are omitted from the actual projection too;
/// whenever a field is present, comparison remains strict. No expected state
/// is fabricated for frontiers that cannot be reconstructed from old frames.
fn retain_recorded_world_interactable_coverage(
    expected: &serde_json::Value,
    actual: &mut serde_json::Value,
) {
    let (Some(expected), Some(actual)) = (expected.as_object(), actual.as_object_mut()) else {
        return;
    };
    if !expected.contains_key("lifts") {
        actual.remove("lifts");
    }
    for field in ["buildings", "script_zones"] {
        if !expected.contains_key(field) {
            actual.remove(field);
        }
    }

    let Some(expected_doors) = expected.get("doors").and_then(serde_json::Value::as_array) else {
        return;
    };
    let Some(actual_doors) = actual
        .get_mut("doors")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return;
    };
    for (expected_door, actual_door) in expected_doors.iter().zip(actual_doors) {
        let (Some(expected_door), Some(actual_door)) =
            (expected_door.as_object(), actual_door.as_object_mut())
        else {
            continue;
        };
        for field in [
            "locked_pc_after_patch",
            "locked_npc_villain_after_patch",
            "locked_npc_civilian_after_patch",
            "unlockable_after_patch",
        ] {
            if !expected_door.contains_key(field) {
                actual_door.remove(field);
            }
        }
    }
}

/// Remove only the additive v28 order fields that are absent from an older
/// raw sequence snapshot. Presence remains strict, including null/zero values.
fn retain_recorded_order_runtime_coverage(
    expected: &serde_json::Value,
    actual: &mut serde_json::Value,
) {
    if !expected
        .as_object()
        .is_some_and(|object| object.contains_key("next_order_id"))
        && let Some(actual) = actual.as_object_mut()
    {
        actual.remove("next_order_id");
    }

    let Some(expected_sequences) = expected
        .get("sequences")
        .and_then(serde_json::Value::as_array)
    else {
        return;
    };
    let Some(actual_sequences) = actual
        .get_mut("sequences")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return;
    };
    for (expected_sequence, actual_sequence) in
        expected_sequences.iter().zip(actual_sequences.iter_mut())
    {
        let Some(expected_elements) = expected_sequence
            .get("elements")
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        let Some(actual_elements) = actual_sequence
            .get_mut("elements")
            .and_then(serde_json::Value::as_array_mut)
        else {
            continue;
        };
        for (expected_element, actual_element) in
            expected_elements.iter().zip(actual_elements.iter_mut())
        {
            let Some(expected_orders) = expected_element
                .get("orders")
                .and_then(serde_json::Value::as_array)
            else {
                continue;
            };
            let Some(actual_orders) = actual_element
                .get_mut("orders")
                .and_then(serde_json::Value::as_array_mut)
            else {
                continue;
            };
            for (expected_order, actual_order) in
                expected_orders.iter().zip(actual_orders.iter_mut())
            {
                let (Some(expected_order), Some(actual_order)) =
                    (expected_order.as_object(), actual_order.as_object_mut())
                else {
                    continue;
                };
                for field in [
                    "destination_3d",
                    "flight_vector",
                    "apply_transition",
                    "can_fly",
                    "lock_ai",
                    "transition",
                    "id",
                ] {
                    if !expected_order.contains_key(field) {
                        actual_order.remove(field);
                    }
                }
            }
        }
    }
}

/// Remove only additive entity-runtime projections when an older raw
/// recording predates them. Once present, every nested field (including
/// explicit nulls) remains structurally strict.
fn retain_recorded_entity_runtime_coverage(
    expected: &serde_json::Value,
    actual: &mut serde_json::Value,
) {
    if !expected
        .as_object()
        .is_some_and(|object| object.contains_key("subtype"))
        && let Some(actual) = actual.as_object_mut()
    {
        actual.remove("subtype");
    }
    if !expected
        .as_object()
        .is_some_and(|object| object.contains_key("npc_ai"))
        && let Some(actual) = actual.as_object_mut()
    {
        actual.remove("npc_ai");
    }
    if !expected
        .as_object()
        .is_some_and(|object| object.contains_key("human_continuation"))
        && let Some(actual) = actual.as_object_mut()
    {
        actual.remove("human_continuation");
    }
    if !expected
        .as_object()
        .is_some_and(|object| object.contains_key("human_structure"))
        && let Some(actual) = actual.as_object_mut()
    {
        actual.remove("human_structure");
    }
    if !expected
        .as_object()
        .is_some_and(|object| object.contains_key("pc_tail"))
        && let Some(actual) = actual.as_object_mut()
    {
        actual.remove("pc_tail");
    }
    if !expected
        .as_object()
        .is_some_and(|object| object.contains_key("pc_qa"))
        && let Some(actual) = actual.as_object_mut()
    {
        actual.remove("pc_qa");
    }
    if !expected
        .as_object()
        .is_some_and(|object| object.contains_key("pc_interface"))
        && let Some(actual) = actual.as_object_mut()
    {
        actual.remove("pc_interface");
    }
    if !expected
        .as_object()
        .is_some_and(|object| object.contains_key("pc_portrait"))
        && let Some(actual) = actual.as_object_mut()
    {
        actual.remove("pc_portrait");
    }
    if !expected
        .as_object()
        .is_some_and(|object| object.contains_key("pc_core"))
        && let Some(actual) = actual.as_object_mut()
    {
        actual.remove("pc_core");
    }
    if expected
        .get("npc_ai")
        .and_then(serde_json::Value::as_object)
        .is_some_and(|npc_ai| !npc_ai.contains_key("subclass"))
        && let Some(actual_npc_ai) = actual
            .get_mut("npc_ai")
            .and_then(serde_json::Value::as_object_mut)
    {
        actual_npc_ai.remove("subclass");
    }
    if let (Some(expected_npc_ai), Some(actual_npc_ai)) = (
        expected
            .get("npc_ai")
            .and_then(serde_json::Value::as_object),
        actual
            .get_mut("npc_ai")
            .and_then(serde_json::Value::as_object_mut),
    ) {
        for field in [
            "stimulus_queue",
            "object_memory",
            "synchronizing_actors",
            "reconnaissance",
            "path_control",
            "legacy_continuation",
        ] {
            if !expected_npc_ai.contains_key(field) {
                actual_npc_ai.remove(field);
            }
        }
    }
    if expected
        .get("npc_ai")
        .and_then(|npc_ai| npc_ai.get("subclass"))
        .and_then(serde_json::Value::as_object)
        .is_some_and(|subclass| {
            subclass.get("kind").and_then(serde_json::Value::as_str) == Some("enemy")
        })
        && let (Some(expected_subclass), Some(actual_subclass)) = (
            expected
                .get("npc_ai")
                .and_then(|npc_ai| npc_ai.get("subclass"))
                .and_then(serde_json::Value::as_object),
            actual
                .get_mut("npc_ai")
                .and_then(|npc_ai| npc_ai.get_mut("subclass"))
                .and_then(serde_json::Value::as_object_mut),
        )
    {
        for field in [
            "heard_nets",
            "other_seen_ale",
            "search_charly_way",
            "missed_in_action",
            "other_bodies_to_examine",
            "beggars_to_control",
            "them",
            "ambush_point_array_reset",
            "ambush_point_status",
            "my_seek_points",
            "personal_seek_point_1",
            "personal_seek_point_2",
            "seek_center",
            "actual_seek_point",
            "seek_point_view_directions",
            "positions_of_beggars_to_control",
            "seek_flags",
            "seen_dead_body",
            "seeking_charly",
            "forced_next_battle_decision",
            "reset_battle_decision",
            "synchronize_index",
            "initial_view_cone",
            "company_number",
            "left_combat_neighbour",
            "right_combat_neighbour",
            "attentive",
            "will_be_attentive",
            "forced_attentive",
            "guarded_pc",
            "tower_guard",
            "combat_trainer",
            "gather_position",
            "gather_direction",
            "gather_position_instructed",
            "officers_position",
            "previous_state",
            "previous_substate",
            "reported_to_officer",
            "missed_soldier_timer",
            "old_money",
            "other_seen_money",
            "money_fight_enemies",
            "money_fight_victims",
            "archer_behind_me",
            "shield_bearer_before_me",
            "already_seen_bodies",
            "my_line_jump",
            "shield_bearer_direction",
            "phalanx_aborted",
            "changed_to_alert_path",
            "shooting_point",
            "archery_sector",
            "archery_sector_index",
            "archery_point_index",
            "archery_point_increment",
            "enemy_seen_below",
            "enemy_had_this_elevation",
            "known_enemy_strike_commands",
            "last_stimulus_dispatched_to_patrol",
        ] {
            if !expected_subclass.contains_key(field) {
                actual_subclass.remove(field);
            }
        }
    }
}

fn compare_engine_state(
    differences: &mut Vec<String>,
    expected: &TraceEngineState,
    engine: &Engine,
    assets: &LevelAssets,
    menu_text: &dyn robin_engine::sherwood_stat::MenuTextLookup,
    entity_map: &EntityMap,
) {
    let actual = engine.parity_engine_state();
    macro_rules! field {
        ($name:ident) => {
            if expected.$name != actual.$name {
                differences.push(format!(
                    "frame.engine_state.{}: original={:?} rust={:?}",
                    stringify!($name),
                    expected.$name,
                    actual.$name
                ));
            }
        };
    }
    field!(cheat_used_flags);
    field!(next_creation_order);
    field!(chorus_timer);
    field!(force_check);
    field!(men_to_blazon_conversion);
    field!(lock_engine);
    field!(freeze_all);
    field!(locker);
    field!(speed_int);
    field!(mission_won);
    field!(mission_won_first_time);
    field!(quit_won);
    field!(quit_lost);
    field!(quit_interrupted);
    field!(script_globals);

    if let Some(expected_game_ui) = &expected.game_ui {
        collect_json_differences(
            "frame.engine_state.game_ui",
            &expected_game_ui.to_json(),
            &engine.parity_game_ui_state(),
            differences,
        );
    }
    if let Some(expected_controller) = &expected.messenger_controller {
        let expected_controller = expected_controller.to_json();
        let mut actual_controller = engine.parity_messenger_controller_state();
        // Schema v49 introduced this object with only `view_locked`; v50 adds
        // the independently authoritative action. Keep those existing traces
        // usable while making the field strict whenever the recorder emitted
        // it.
        if expected_controller
            .as_object()
            .is_some_and(|controller| !controller.contains_key("selected_action"))
            && let Some(controller) = actual_controller.as_object_mut()
        {
            controller.remove("selected_action");
        }
        collect_json_differences(
            "frame.engine_state.messenger_controller",
            &expected_controller,
            &actual_controller,
            differences,
        );
    }
    if let Some(expected_controller) = &expected.shield_controller {
        let mut expected_controller = expected_controller.to_json();
        canonicalize_authoritative_snapshot(&mut expected_controller, entity_map);
        collect_json_differences(
            "frame.engine_state.shield_controller",
            &expected_controller,
            &engine.parity_shield_controller_state(),
            differences,
        );
    }

    let mut expected_pc_registry = expected.pc_registry.to_json();
    canonicalize_authoritative_snapshot(&mut expected_pc_registry, entity_map);
    let actual_pc_registry = engine.parity_pc_registry_state();
    collect_json_differences(
        "frame.engine_state.pc_registry",
        &expected_pc_registry,
        &actual_pc_registry,
        differences,
    );

    let mut expected_sequences = expected.sequence_manager.to_json();
    canonicalize_authoritative_snapshot(&mut expected_sequences, entity_map);
    let mut actual_sequences = engine.parity_sequence_manager_state();
    retain_recorded_order_runtime_coverage(&expected_sequences, &mut actual_sequences);
    collect_json_differences(
        "frame.engine_state.sequence_manager",
        &expected_sequences,
        &actual_sequences,
        differences,
    );

    let mut expected_script = expected.script_runtime.to_json();
    canonicalize_authoritative_snapshot(&mut expected_script, entity_map);
    let actual_script = engine.parity_script_runtime_state();
    collect_json_differences(
        "frame.engine_state.script_runtime",
        &expected_script,
        &actual_script,
        differences,
    );

    let mut expected_pathfinder = expected.pathfinder.to_json();
    canonicalize_authoritative_snapshot(&mut expected_pathfinder, entity_map);
    let actual_pathfinder = engine.parity_pathfinder_state();
    collect_json_differences(
        "frame.engine_state.pathfinder",
        &expected_pathfinder,
        &actual_pathfinder,
        differences,
    );

    let mut expected_view_radius_cache = expected.view_radius_cache.to_json();
    canonicalize_authoritative_snapshot(&mut expected_view_radius_cache, entity_map);
    let actual_view_radius_cache = engine.parity_view_radius_cache_state(assets);
    collect_json_differences(
        "frame.engine_state.view_radius_cache",
        &expected_view_radius_cache,
        &actual_view_radius_cache,
        differences,
    );

    let expected_sound_sources = expected.sound_sources.to_json();
    let actual_sound_sources = engine.parity_sound_sources_state();
    collect_json_differences(
        "frame.engine_state.sound_sources",
        &expected_sound_sources,
        &actual_sound_sources,
        differences,
    );

    if let Some(expected_frontier) = &expected.sound_completion_frontier {
        let expected_frontier = expected_frontier.to_json();
        let actual_frontier = engine.parity_sound_completion_frontier_state();
        collect_json_differences(
            "frame.engine_state.sound_completion_frontier",
            &expected_frontier,
            &actual_frontier,
            differences,
        );
    }

    let mut expected_ai_global = expected.ai_global.to_json();
    canonicalize_authoritative_snapshot(&mut expected_ai_global, entity_map);
    let actual_ai_global = engine.parity_ai_global_state();
    collect_json_differences(
        "frame.engine_state.ai_global",
        &expected_ai_global,
        &actual_ai_global,
        differences,
    );

    let mut expected_runtime_roots = expected.engine_runtime_roots.to_json();
    canonicalize_authoritative_snapshot(&mut expected_runtime_roots, entity_map);
    let actual_runtime_roots = engine.parity_engine_runtime_roots_state(menu_text);
    collect_json_differences(
        "frame.engine_state.engine_runtime_roots",
        &expected_runtime_roots,
        &actual_runtime_roots,
        differences,
    );

    let mut expected_world_interactables = expected.world_interactables.to_json();
    canonicalize_authoritative_snapshot(&mut expected_world_interactables, entity_map);
    let mut actual_world_interactables = engine.parity_world_interactables_state(assets);
    retain_recorded_world_interactable_coverage(
        &expected_world_interactables,
        &mut actual_world_interactables,
    );
    collect_json_differences(
        "frame.engine_state.world_interactables",
        &expected_world_interactables,
        &actual_world_interactables,
        differences,
    );

    let expected_repulsive_points = expected.repulsive_points.to_json();
    let actual_repulsive_points = engine.parity_repulsive_points_state();
    collect_json_differences(
        "frame.engine_state.repulsive_points",
        &expected_repulsive_points,
        &actual_repulsive_points,
        differences,
    );

    let mut expected_titbit_manager = expected.titbit_manager.to_json();
    canonicalize_authoritative_snapshot(&mut expected_titbit_manager, entity_map);
    let actual_titbit_manager = engine.parity_titbit_manager_state();
    collect_json_differences(
        "frame.engine_state.titbit_manager",
        &expected_titbit_manager,
        &actual_titbit_manager,
        differences,
    );

    if expected.speed.bits != actual.speed.to_bits() {
        differences.push(format!(
            "frame.engine_state.speed: original={} (0x{:08x}) rust={} (0x{:08x})",
            expected.speed.value(),
            expected.speed.bits,
            actual.speed,
            actual.speed.to_bits()
        ));
    }

    let actual_failed = engine.parity_failed_path_requests();
    if expected.failed_path_requests.len() != actual_failed.len() {
        differences.push(format!(
            "frame.engine_state.failed_path_requests.length: original={} rust={}",
            expected.failed_path_requests.len(),
            actual_failed.len()
        ));
    }
    for (index, (expected, actual)) in expected
        .failed_path_requests
        .iter()
        .zip(&actual_failed)
        .enumerate()
    {
        let prefix = format!("frame.engine_state.failed_path_requests[{index}]");
        let request = &actual.request;
        macro_rules! request_field {
            ($name:expr, $left:expr, $right:expr) => {
                if $left != $right {
                    differences.push(format!(
                        "{}.{}: original={:?} rust={:?}",
                        prefix, $name, $left, $right
                    ));
                }
            };
        }
        request_field!("actor", entity_map.translate(expected.actor), request.actor);
        request_field!(
            "antagonist",
            expected.antagonist.map(|id| entity_map.translate(id)),
            request.antagonist
        );
        request_field!("layer", expected.layer, request.layer);
        if !entity_map.sectors_equivalent(expected.area, request.area) {
            differences.push(format!(
                "{prefix}.area: original={} rust={}",
                expected.area, request.area
            ));
        }
        request_field!(
            "source.bits",
            [expected.source.x.bits, expected.source.y.bits],
            [request.source.x.to_bits(), request.source.y.to_bits()]
        );
        request_field!(
            "goal.bits",
            [expected.goal.x.bits, expected.goal.y.bits],
            [request.goal.x.to_bits(), request.goal.y.to_bits()]
        );
        request_field!(
            "half_diagonal_index",
            expected.half_diagonal_index,
            request.half_diagonal_index
        );
        request_field!(
            "half_diagonal.bits",
            [expected.half_diagonal.x.bits, expected.half_diagonal.y.bits],
            [
                request.half_diagonal.x.to_bits(),
                request.half_diagonal.y.to_bits()
            ]
        );
        request_field!("animation", expected.animation, request.animation);
        request_field!("reverse", expected.reverse, request.reverse);
        request_field!("speed", expected.speed, request.speed);
        request_field!(
            "tolerance.bits",
            expected.tolerance.bits,
            request.tolerance.to_bits()
        );
        request_field!(
            "use_first_point",
            expected.use_first_point,
            request.use_first_point
        );
        request_field!("sector", expected.sector, actual.sector);
        request_field!("time", expected.time, actual.time);
    }
}

/// Whether Original exposes indeterminate `RHPositionInterface` old-position
/// storage for a bonus constructed during this replay.
///
/// `RHPositionInterface::RHPositionInterface` does not initialize
/// `mpointOldPosition`/`mpointOldMap`. Both runtime bonus construction paths
/// (`RHElementBonus` and the `RHElementAle` used by DROP_ALE) place the object
/// through `RHElement::CopyPositionMapEtc`, which sets only the current
/// position and never calls `NewMove` (`RHpositioninterface.cpp:50-108`,
/// `RHelement.cpp:287-304`, `RHelementactorpc.cpp:5704-5713`). These stationary
/// objects never acquire a defined old position during their live runtime.
///
/// Save/mission-start bonuses are deliberately excluded: serialization does
/// retain the complete old position, and their creation orders lie below the
/// boundary captured when the initial entity bijection is built. All other
/// fields on runtime bonuses remain authoritative and are still compared.
fn original_runtime_bonus_has_undefined_old_position(
    kind: TraceEntityKind,
    creation_order: u32,
    runtime_creation_order_boundary: u32,
) -> bool {
    kind == TraceEntityKind::Bonus && creation_order >= runtime_creation_order_boundary
}

fn compare_frame(
    engine: &Engine,
    assets: &LevelAssets,
    menu_text: &dyn robin_engine::sherwood_stat::MenuTextLookup,
    frame: &TraceFrame,
    actual_game_code: i32,
    entity_map: &EntityMap,
    late_movement_retranslations: &[EntityId],
) -> Vec<String> {
    let mut differences = Vec::new();

    if let Some(expected_campaign) = &frame.campaign {
        let restored = restore_campaign(expected_campaign, &assets.profile_manager);
        let expected = campaign_comparison_value(&restored, menu_text);
        let actual = campaign_comparison_value(engine.parity_campaign(), menu_text);
        collect_json_differences("frame.campaign", &expected, &actual, &mut differences);
    }
    if let Some(expected) = &frame.engine_state {
        compare_engine_state(
            &mut differences,
            expected,
            engine,
            assets,
            menu_text,
            entity_map,
        );
    }

    if frame.game_code != actual_game_code {
        differences.push(format!(
            "frame.game_code: original={} rust={actual_game_code}",
            frame.game_code
        ));
    }
    let selected: Vec<EntityId> = frame
        .selected_pcs
        .iter()
        .copied()
        .map(|id| entity_map.translate(id))
        .collect();
    if engine.selected_pc_ids() != selected {
        differences.push(format!(
            "selected_pcs: original={selected:?} rust={:?}",
            engine.selected_pc_ids()
        ));
    }

    // Actor state is generally the most actionable parity signal. Report it
    // before the (much larger) background-FX table.
    let mut elements: Vec<_> = frame.elements.iter().collect();
    elements.sort_by_key(|element| {
        let priority = match element.entity_id.kind {
            TraceEntityKind::Pc => 0,
            TraceEntityKind::Soldier => 1,
            TraceEntityKind::Civilian => 2,
            _ => 3,
        };
        (priority, element.entity_id.index)
    });
    for expected in elements {
        // Background FX animation is presentation state. It remains in the
        // trace for a later renderer-parity pass, but does not belong in the
        // first logical gameplay comparison.
        if expected.entity_id.kind == TraceEntityKind::Fx {
            continue;
        }
        let id = entity_map.translate(expected.entity_id);
        // Diff paths name the RUST id, but `--dump-entity`, the trace's
        // `elements[].entity_id.index` and the Original's logs all use the ORIGINAL
        // index, and the two are frequently unequal (e.g. Original pc:171 is
        // Rust Pc(PcId(174))). Four investigations lost hours to that mismatch, so
        // every diff root spells the pairing out.
        let id_label = EntityLabel {
            id,
            original_index: expected.entity_id.index,
        };
        let Some(actual) = engine.get_entity(id) else {
            differences.push(format!("{id_label:?}: missing in Rust entity table"));
            continue;
        };
        let element = actual.element_data();
        assert_eq!(
            expected.ai.is_some(),
            expected.detection.is_some(),
            "Original NPC trace state must contain both ai and detection payloads for {:?}",
            expected.entity_id
        );
        if entity_map.creation_order_is_exact(expected.creation_order) {
            compare(
                &mut differences,
                id,
                "creation_order",
                expected.creation_order,
                engine.original_creation_order(id),
            );
        }
        compare(
            &mut differences,
            id,
            "kind",
            expected.kind,
            trace_kind_for_entity(actual),
        );
        compare(
            &mut differences,
            id,
            "entity_id.kind",
            expected.entity_id.kind,
            expected.kind,
        );
        compare(
            &mut differences,
            id,
            "active",
            expected.active,
            element.active,
        );
        compare(
            &mut differences,
            id,
            "blipped",
            expected.blipped,
            element.blipped,
        );
        // `class_id` is redundant with the concrete kind above. Rust uses
        // that typed kind for dispatch rather than retaining Original's raw
        // numeric RTTI token. `surface_id` is a DrawManager allocation handle;
        // Rust has no corresponding per-entity render surface. Both fields
        // remain deserialized for diagnostics, but neither is logical state.
        if expected.actor.is_some() {
            compare(
                &mut differences,
                id,
                "unreachable",
                expected.unreachable,
                element.unreachable,
            );
            if expected.posture != element.posture as u32 {
                let ai = actual.ai_controller();
                differences.push(format!(
                    "{id:?}.posture: original={} rust={} (rust initial_action={:?} stay_home={:?} likes_to_sit={:?} sector={:?})",
                    expected.posture,
                    element.posture as u32,
                    ai.map(|ai| ai.initial_action),
                    ai.map(|ai| ai.is_stay_at_home),
                    ai.map(|ai| ai.likes_to_sit_around),
                    element.sector(),
                ));
            }
        }
        compare_point(
            &mut differences,
            id,
            "position_map",
            expected.position_map,
            element.position_map(),
        );
        let pi = &element.sprite.position_iface;
        let undefined_runtime_bonus_old_position =
            original_runtime_bonus_has_undefined_old_position(
                expected.kind,
                expected.creation_order,
                entity_map.runtime_creation_order_boundary,
            );
        if !undefined_runtime_bonus_old_position {
            compare_point(
                &mut differences,
                id,
                "old_position_map",
                expected.old_position_map,
                pi.old_map_position(),
            );
        }
        compare_point_with_absolute_tolerance(
            &mut differences,
            id,
            "position_goal_map",
            expected.position_goal_map,
            pi.map_goal(),
            0.011,
        );
        compare_float(
            &mut differences,
            id,
            "elevation",
            expected.elevation,
            pi.get_elevation(),
        );
        if !undefined_runtime_bonus_old_position {
            compare_float(
                &mut differences,
                id,
                "old_elevation",
                expected.old_elevation,
                pi.old_elevation(),
            );
        }
        let increment_map = pi.raw_increment_map();
        compare_point(
            &mut differences,
            id,
            "increment_map",
            expected.increment_map,
            MapPoint::new(increment_map.x, increment_map.y),
        );
        if let Some(expected_increment_map_valid) = expected.increment_map_valid {
            compare(
                &mut differences,
                id,
                "increment_map_valid",
                expected_increment_map_valid,
                pi.is_increment_map_computed(),
            );
        }
        if !undefined_runtime_bonus_old_position {
            let movement_map = element.position_map() - pi.old_map_position();
            compare_point(
                &mut differences,
                id,
                "movement_map",
                expected.movement_map,
                MapPoint::new(movement_map.x, movement_map.y),
            );
        }
        let actual_sector = element.sector().map_or(u16::MAX, |sector| sector.get());
        let mapped_building_sector = expected.sector != actual_sector
            && entity_map.sectors_equivalent(expected.sector, actual_sector);
        compare(
            &mut differences,
            id,
            "layer",
            expected.layer,
            element.layer(),
        );
        compare(
            &mut differences,
            id,
            "layer_goal",
            expected.layer_goal,
            pi.layer_goal().get(),
        );
        if !mapped_building_sector {
            compare(
                &mut differences,
                id,
                "sector",
                expected.sector,
                actual_sector,
            );
        }
        compare(
            &mut differences,
            id,
            "direction",
            expected.direction,
            i16::from(pi.get_direction().as_u8()),
        );
        compare(
            &mut differences,
            id,
            "direction_goal",
            expected.direction_goal,
            i16::from(pi.get_direction_goal().as_u8()),
        );
        if !undefined_runtime_bonus_old_position {
            compare(
                &mut differences,
                id,
                "moving",
                expected.moving,
                pi.is_moving(),
            );
            compare(
                &mut differences,
                id,
                "moving_map",
                expected.moving_map,
                pi.is_moving_map(),
            );
        }
        compare(
            &mut differences,
            id,
            "sprite_row",
            expected.sprite_row,
            element.sprite.current_row,
        );
        compare(
            &mut differences,
            id,
            "sprite_frame",
            expected.sprite_frame,
            element.sprite.current_frame,
        );
        if let Some(expected_frame_count) = expected.sprite_frame_count {
            compare(
                &mut differences,
                id,
                "sprite_frame_count",
                expected_frame_count,
                element.sprite.frame_count,
            );
        }
        if let Some(expected_runtime) = &expected.runtime {
            let mut expected_runtime = expected_runtime.to_json();
            canonicalize_authoritative_snapshot(&mut expected_runtime, entity_map);
            let mut actual_runtime = engine.parity_entity_runtime_state(id, assets);
            retain_recorded_entity_runtime_coverage(&expected_runtime, &mut actual_runtime);
            collect_json_differences(
                &format!("{id_label:?}.runtime"),
                &expected_runtime,
                &actual_runtime,
                &mut differences,
            );
        }
        if let Some(expected_actor) = &expected.actor {
            let actual_actor = actual
                .actor_data()
                .unwrap_or_else(|| panic!("trace reports actor state for non-actor {id:?}"));
            if expected_actor.action_state != actual_actor.action_state as u32 {
                differences.push(format!(
                    "{id:?}.actor.action_state: original={} rust={} (original animation={} command={} motion={}; rust last_action={:?} command={:?} script_class={:?})",
                    expected_actor.action_state,
                    actual_actor.action_state as u32,
                    expected_actor.animation,
                    expected_actor.command,
                    expected_actor.motion_state,
                    element.sprite.last_action,
                    engine.actor_command(id),
                    actual_actor.script_class,
                ));
            }
            compare(
                &mut differences,
                id,
                "actor.wait_time",
                expected_actor.wait_time,
                engine.actor_legacy_wait_time(id),
            );
            // Legacy schema-12 recordings made before Original commit
            // 7243bed9 captured a dangling `RHElementActor::mpOrder` here.
            // `InvalidateMovements` runs after the actor's owner slot, deletes
            // its live orders, and translates replacements. The old pointer's
            // apparent animation then depended on RHOrder allocator reuse and
            // is not logical game state. The replay has no sequence snapshot
            // from which to reconstruct the corrected first order, but Rust's
            // retranslation is independently compared through movement,
            // motion-line, and subsequent-frame actor state. Omit only this
            // explicitly captured post-slot physical representation.
            if !late_movement_retranslations.contains(&id) {
                compare(
                    &mut differences,
                    id,
                    "actor.animation",
                    expected_actor.animation,
                    engine
                        .actor_order_type(id)
                        .unwrap_or(robin_engine::order::OrderType::NonanimationEnd)
                        as u32,
                );
            }
            // `RHElementActorPC::Perform(RHANIMATION_STRANGLING)` leaves its
            // local `motionState` uninitialized while either participant is
            // still turning, and `RHElementActor::Hourglass` copies that raw
            // return into `mmotionState`.  Old schema-12 captures therefore
            // occasionally contain stack bytes here.  Preserve comparison
            // for every value in Original's declared `RHmotionState` enum,
            // including its `RHMOTION_ERROR` member that Rust deliberately
            // cannot produce, but do not turn undefined C++ telemetry into a
            // fabricated Rust gameplay state.
            if original_motion_state_is_defined(expected_actor.motion_state) {
                compare(
                    &mut differences,
                    id,
                    "actor.motion_state",
                    expected_actor.motion_state,
                    actual_actor.continuation.motion_state as u32,
                );
            }
            compare(
                &mut differences,
                id,
                "actor.command",
                command_from_stable_name(&expected_actor.command_name),
                engine.actor_command(id),
            );
            if let Some(expected_direct) = expected_actor.passing_door_directly {
                compare(
                    &mut differences,
                    id,
                    "actor.passing_door_directly",
                    expected_direct,
                    actual_actor.passing_door_directly,
                );
            }
            if let Some(expected_pass) = &expected_actor.active_pass_door {
                let expected_pass_key = expected_pass.as_ref().map(trace_pass_door_key);
                let actual_pass = engine
                    .actor_selected_pass_door(id)
                    .map(|(gate_id, direction)| (u32::from(gate_id.0), direction != 0));
                if !active_pass_door_keys_match(expected_pass.as_ref(), actual_pass) {
                    differences.push(format!(
                        "{id:?}.actor.active_pass_door.(gate_id,direct): original={expected_pass_key:?} rust={actual_pass:?}"
                    ));
                }
            }
            if let Some(Some(expected_sequence)) = &expected_actor.sequence_element {
                compare(
                    &mut differences,
                    id,
                    "actor.sequence_element.command",
                    command_from_stable_name(&expected_sequence.command_name),
                    engine.actor_command(id),
                );
            }
        }
        if let Some(expected_human) = &expected.human {
            let actual_camp = actual.camp();
            let actual_life = match actual {
                Entity::Pc(pc) => pc.pc.life_points,
                Entity::Soldier(soldier) => soldier.npc.life_points,
                Entity::Civilian(civilian) => civilian.npc.life_points,
                _ => panic!("trace reports life_points for non-human {id:?}"),
            };
            compare(
                &mut differences,
                id,
                "life_points",
                expected_human.life_points,
                actual_life,
            );
            compare(
                &mut differences,
                id,
                "dead",
                expected_human.dead,
                actual.is_dead(),
            );
            compare(
                &mut differences,
                id,
                "unconscious",
                expected_human.unconscious,
                actual
                    .human_data()
                    .unwrap_or_else(|| panic!("trace reports human state for non-human {id:?}"))
                    .unconscious,
            );
            compare(
                &mut differences,
                id,
                "human.camp",
                expected_human.camp.as_str(),
                camp_name(actual_camp),
            );
            compare(
                &mut differences,
                id,
                "human.original_camp",
                expected_human.original_camp,
                camp_ordinal(actual_camp),
            );
            compare(
                &mut differences,
                id,
                "human.vip",
                expected_human.vip,
                entity_is_vip(actual, assets),
            );
            compare(
                &mut differences,
                id,
                "human.civilian",
                expected_human.civilian,
                actual.is_civilian(),
            );
            if let Some(expected) = &expected_human.opponents {
                let expected: Vec<EntityId> = expected
                    .iter()
                    .copied()
                    .map(|opponent| entity_map.translate(opponent))
                    .collect();
                let actual = actual
                    .human_data()
                    .unwrap_or_else(|| panic!("trace reports human opponents for non-human {id:?}"))
                    .opponents
                    .ids();
                compare(&mut differences, id, "human.opponents", expected, actual);
            }
            if let Some(expected) = &expected_human.opponent_jump_lines {
                let expected: Vec<Option<[u32; 4]>> = expected
                    .iter()
                    .map(|line| line.as_ref().map(trace_jump_line_bits))
                    .collect();
                let human = actual.human_data().unwrap_or_else(|| {
                    panic!("trace reports human opponent jump lines for non-human {id:?}")
                });
                let actual: Vec<Option<[u32; 4]>> = human
                    .opponents
                    .iter_with_jump_lines()
                    .map(|(_, line_index)| line_index)
                    .map(|line_index| {
                        line_index.map(|line_index| {
                            let line = engine
                                .fast_grid()
                                .level
                                .jump_lines
                                .get(usize::from(line_index))
                                .unwrap_or_else(|| {
                                    panic!(
                                        "human {id:?} opponent jump-line index {line_index} is out of range"
                                    )
                                });
                            runtime_jump_line_bits(line)
                        })
                    })
                    .collect();
                compare(
                    &mut differences,
                    id,
                    "human.opponent_jump_lines",
                    expected,
                    actual,
                );
            }
        }
        if let Some(expected_pc) = &expected.pc {
            use robin_engine::profiles::Action;

            let ammo = &expected_pc.ammo;
            for (field, expected_count, action) in [
                ("ales", ammo.ales, Action::Ale),
                ("apples", ammo.apples, Action::Apple),
                ("arrows", ammo.arrows, Action::Bow),
                ("nets", ammo.nets, Action::Net),
                ("plants", ammo.plants, Action::Heal),
                ("purses", ammo.purses, Action::Purse),
                ("rations", ammo.rations, Action::Eat),
                ("stones", ammo.stones, Action::Stone),
                ("wasp_nests", ammo.wasp_nests, Action::WaspNest),
            ] {
                compare(
                    &mut differences,
                    id,
                    &format!("pc.ammo.{field}"),
                    expected_count,
                    engine.get_pc_ammo_count(id, action),
                );
            }
        }
        if let Some(expected_ai) = &expected.ai {
            let actual_ai = actual
                .ai_controller()
                .unwrap_or_else(|| panic!("trace reports AI state for non-NPC {id:?}"));
            compare(
                &mut differences,
                id,
                "ai.state",
                expected_ai.state,
                actual_ai.current_state as u32,
            );
            compare(
                &mut differences,
                id,
                "ai.substate",
                expected_ai.substate,
                actual_ai.current_substate as u32,
            );
            if let Some(expected) = expected_ai.script_locked {
                compare(
                    &mut differences,
                    id,
                    "ai.script_locked",
                    expected,
                    actual_ai.ai_is_script_locked(),
                );
            }
            if let Some(expected) = expected_ai.locked {
                compare(
                    &mut differences,
                    id,
                    "ai.locked",
                    expected,
                    actual_ai.ai_is_locked(),
                );
            }
            if let Some(expected) = expected_ai.locks {
                compare(
                    &mut differences,
                    id,
                    "ai.locks",
                    expected,
                    actual_ai.locks_flag_field.bits(),
                );
            }
            if let Some(expected) = expected_ai.was_busy {
                compare(
                    &mut differences,
                    id,
                    "ai.was_busy",
                    expected,
                    actual_ai.was_busy,
                );
            }
            if let Some(expected) = expected_ai.very_busy {
                compare(
                    &mut differences,
                    id,
                    "ai.very_busy",
                    expected,
                    engine.is_very_very_busy(id),
                );
            }
            if let Some(expected) = expected_ai.macro_timer_running {
                compare(
                    &mut differences,
                    id,
                    "ai.macro_timer_running",
                    expected,
                    actual_ai.macro_timer_is_running,
                );
            }
            if let Some(expected) = expected_ai.macro_timer_ring {
                compare(
                    &mut differences,
                    id,
                    "ai.macro_timer_ring",
                    expected,
                    actual_ai.when_does_macro_timer_ring,
                );
            }
            // The cursor is only a position while it still lies inside the
            // waypoint block it was authored against. Advancing the patrol path
            // or breaking a macro leaves the cursor behind on the previous
            // waypoint's data, and the Original can only report an offset when
            // its pointer still falls within the current waypoint's block.
            // That is a question of identity, not of content: two waypoints can
            // carry byte-identical macro data, so the retained stream has to be
            // the one taken from the waypoint the path currently stands on.
            let current_waypoint = actual_ai
                .has_patrol_path
                .then_some(actual_ai.patrol_path.as_ref())
                .flatten()
                .map(|path| (path.hiking_path_index, path.current_waypoint_index));
            let actual_macro_cursor = current_waypoint
                .filter(|current| {
                    actual_ai.macro_command_waypoint == Some(*current)
                        && !actual_ai.macro_command.is_empty()
                        && actual_ai.macro_command_offset <= actual_ai.macro_command.len()
                })
                .map(|_| {
                    u16::try_from(actual_ai.macro_command_offset).unwrap_or_else(|_| {
                        panic!(
                            "NPC {id:?} macro cursor {} exceeds Original's UWORD domain",
                            actual_ai.macro_command_offset
                        )
                    })
                });
            if let Some(expected) = expected_ai.macro_cursor {
                compare(
                    &mut differences,
                    id,
                    "ai.macro_cursor",
                    expected,
                    actual_macro_cursor,
                );
            }
            if let Some(expected) = expected_ai.macro_remaining {
                compare(
                    &mut differences,
                    id,
                    "ai.macro_remaining",
                    expected,
                    actual_ai.number_of_remaining_macro_bytes,
                );
            }
            if let Some(expected) = expected_ai.macro_in_progress {
                compare(
                    &mut differences,
                    id,
                    "ai.macro_in_progress",
                    expected,
                    actual_ai.macro_in_progress,
                );
            }
            if let Some(expected) = &expected_ai.list_us {
                let expected: Vec<EntityId> = expected
                    .iter()
                    .copied()
                    .map(|human| entity_map.translate(human))
                    .collect();
                let actual: Vec<EntityId> = actual_ai
                    .list_us
                    .iter()
                    .map(|&handle| {
                        engine.entity_id_for_index(handle).unwrap_or_else(|| {
                            panic!("AI list_us handle {handle} refers to a vacant entity slot")
                        })
                    })
                    .collect();
                compare(&mut differences, id, "ai.list_us", expected, actual);
            }
            if let Some(expected) = &expected_ai.list_them {
                let expected: Vec<EntityId> = expected
                    .iter()
                    .copied()
                    .map(|human| entity_map.translate(human))
                    .collect();
                let actual: Vec<EntityId> = actual
                    .enemy_ai()
                    .map(|enemy| {
                        enemy
                            .list_them
                            .iter()
                            .map(|&handle| {
                                engine.entity_id_for_index(handle).unwrap_or_else(|| {
                                    panic!(
                                        "AI list_them handle {handle} refers to a vacant entity slot"
                                    )
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                compare(&mut differences, id, "ai.list_them", expected, actual);
            }
            if let Some(expected) = &expected_ai.my_line_jump {
                let expected = expected.as_ref().map(trace_jump_line_bits);
                let enemy = actual.enemy_ai().unwrap_or_else(|| {
                    panic!("trace reports my_line_jump for non-enemy entity {id:?}")
                });
                let actual = enemy.my_line_jump.map(|line_index| {
                    let line = engine
                        .fast_grid()
                        .level
                        .jump_lines
                        .get(line_index as usize)
                        .unwrap_or_else(|| {
                            panic!("enemy {id:?} my_line_jump index {line_index} is out of range")
                        });
                    runtime_jump_line_bits(line)
                });
                compare(&mut differences, id, "ai.my_line_jump", expected, actual);
            }
        }
        if let Some(expected_detection) = &expected.detection {
            let npc = actual
                .npc_data()
                .unwrap_or_else(|| panic!("trace reports detection state for non-NPC {id:?}"));
            let controller = actual
                .ai_controller()
                .unwrap_or_else(|| panic!("trace reports detection state for AI-less NPC {id:?}"));
            compare(
                &mut differences,
                id,
                "detection.suspects",
                expected_detection.suspects.as_slice(),
                npc.detection_suspects.as_slice(),
            );
            compare(
                &mut differences,
                id,
                "detection.maximal_suspect",
                expected_detection.maximal_suspect,
                npc.maximal_detection_suspect,
            );
            compare(
                &mut differences,
                id,
                "detection.maximal_visibility",
                expected_detection.maximal_visibility,
                controller.max_visibility,
            );
            compare(
                &mut differences,
                id,
                "detection.view_status",
                expected_detection.view_status,
                npc.eye_status as u8,
            );
            compare(
                &mut differences,
                id,
                "detection.alert_status",
                expected_detection.alert_status,
                controller.view_alert_status as u32,
            );

            let actual_detectables_len = npc.detectable_lists.iter().map(Vec::len).sum::<usize>();
            compare(
                &mut differences,
                id,
                "detection.detectables.length",
                expected_detection.detectables.len(),
                actual_detectables_len,
            );
            if expected_detection.detectables.len() != actual_detectables_len
                && std::env::var_os("PARITY_DEBUG_DETECTABLE_DIFF").is_some()
            {
                // Side-by-side dump of both flattened detectable lists for the
                // NPC whose length diverged. Unlike `PARITY_DEBUG_DETECTABLE_LIST`
                // this needs no frame/creation-order filter: it fires exactly at
                // the first divergent frame, which is where the identity of the
                // added/removed entry (bucket + target) has to be read off.
                eprintln!(
                    "DETDIFF owner={id:?} original_len={} rust_len={}",
                    expected_detection.detectables.len(),
                    actual_detectables_len
                );
                for (i, d) in expected_detection.detectables.iter().enumerate() {
                    eprintln!(
                        "DETDIFF original[{i}] type={} target={:?}->{:?} seen_now={} seen_last={} heard_last={} shadow_now={} shadow_last={} vis={:?}",
                        d.detectable_type,
                        d.target,
                        entity_map.translate(d.target),
                        d.seen_now,
                        d.seen_last_frame,
                        d.heard_last_frame,
                        d.shadow_seen_now,
                        d.shadow_seen_last_frame,
                        d.last_visibility
                    );
                }
                for (i, d) in npc
                    .detectable_lists
                    .iter()
                    .flat_map(|list| list.iter())
                    .enumerate()
                {
                    eprintln!(
                        "DETDIFF rust[{i}] type={} target={:?} seen_now={} seen_last={} heard_last={} shadow_now={} shadow_last={} vis={}",
                        detectable_type_ordinal(d.detectable_type),
                        d.element,
                        d.seen_now,
                        d.seen_last_frame,
                        d.heard_last_frame,
                        d.shadow_seen_now,
                        d.shadow_seen_last_frame,
                        d.last_visibility
                    );
                }
            }
            for (detectable_index, (expected_detectable, actual_detectable)) in expected_detection
                .detectables
                .iter()
                .zip(npc.detectable_lists.iter().flat_map(|list| list.iter()))
                .enumerate()
            {
                compare_indexed(
                    &mut differences,
                    id,
                    "detection.detectables",
                    detectable_index,
                    "type",
                    expected_detectable.detectable_type,
                    detectable_type_ordinal(actual_detectable.detectable_type),
                );
                compare_indexed(
                    &mut differences,
                    id,
                    "detection.detectables",
                    detectable_index,
                    "target",
                    entity_map.translate(expected_detectable.target),
                    actual_detectable.element.unwrap_or_else(|| {
                        panic!("NPC {id:?} detectable {detectable_index} has no target element")
                    }),
                );
                compare_indexed(
                    &mut differences,
                    id,
                    "detection.detectables",
                    detectable_index,
                    "seen_now",
                    expected_detectable.seen_now,
                    actual_detectable.seen_now,
                );
                compare_indexed(
                    &mut differences,
                    id,
                    "detection.detectables",
                    detectable_index,
                    "seen_last_frame",
                    expected_detectable.seen_last_frame,
                    actual_detectable.seen_last_frame,
                );
                compare_indexed(
                    &mut differences,
                    id,
                    "detection.detectables",
                    detectable_index,
                    "heard_last_frame",
                    expected_detectable.heard_last_frame,
                    actual_detectable.heard_last_frame,
                );
                compare_indexed(
                    &mut differences,
                    id,
                    "detection.detectables",
                    detectable_index,
                    "shadow_seen_now",
                    expected_detectable.shadow_seen_now,
                    actual_detectable.shadow_seen_now,
                );
                compare_indexed(
                    &mut differences,
                    id,
                    "detection.detectables",
                    detectable_index,
                    "shadow_seen_last_frame",
                    expected_detectable.shadow_seen_last_frame,
                    actual_detectable.shadow_seen_last_frame,
                );
                compare_float_indexed(
                    &mut differences,
                    id,
                    "detection.detectables",
                    detectable_index,
                    "last_visibility",
                    expected_detectable.last_visibility,
                    actual_detectable.last_visibility,
                );
            }
        }
    }
    differences
}

fn compare<T: std::fmt::Debug + PartialEq>(
    differences: &mut Vec<String>,
    id: EntityId,
    field: &str,
    expected: T,
    actual: T,
) {
    if expected != actual {
        differences.push(format!(
            "{id:?}.{field}: original={expected:?} rust={actual:?}"
        ));
    }
}

fn compare_indexed<T: std::fmt::Debug + PartialEq>(
    differences: &mut Vec<String>,
    id: EntityId,
    collection: &str,
    index: usize,
    field: &str,
    expected: T,
    actual: T,
) {
    if expected != actual {
        differences.push(format!(
            "{id:?}.{collection}[{index}].{field}: original={expected:?} rust={actual:?}"
        ));
    }
}

fn original_motion_state_is_defined(raw: u32) -> bool {
    // Original `RHSprite.h`: DONE, START, IN_PROGRESS, TERMINATED,
    // ABORTED, ERROR.
    raw <= 5
}

fn trace_kind_for_entity(entity: &Entity) -> TraceEntityKind {
    match entity {
        Entity::Pc(_) => TraceEntityKind::Pc,
        Entity::Soldier(_) => TraceEntityKind::Soldier,
        Entity::Civilian(_) => TraceEntityKind::Civilian,
        Entity::Fx(_) => TraceEntityKind::Fx,
        Entity::Target(_) => TraceEntityKind::Target,
        Entity::Bonus(_) => TraceEntityKind::Bonus,
        Entity::Scroll(_) => TraceEntityKind::Scroll,
        Entity::Projectile(_) => TraceEntityKind::Projectile,
        Entity::Net(_) => TraceEntityKind::Net,
    }
}

fn camp_name(camp: robin_engine::element::Camp) -> &'static str {
    match camp {
        robin_engine::element::Camp::Royalists => "royalists",
        robin_engine::element::Camp::Lacklandists => "lacklandists",
        robin_engine::element::Camp::Error => "error",
    }
}

fn camp_ordinal(camp: robin_engine::element::Camp) -> i32 {
    match camp {
        robin_engine::element::Camp::Royalists => 0,
        robin_engine::element::Camp::Lacklandists => 1,
        robin_engine::element::Camp::Error => 2,
    }
}

fn entity_is_vip(entity: &Entity, assets: &LevelAssets) -> bool {
    match entity {
        Entity::Pc(pc) => {
            assets
                .profile_manager
                .get_character(pc.pc.profile_index)
                .unwrap_or_else(|| {
                    panic!(
                        "parity VIP lookup is missing PC character profile {:?}",
                        pc.pc.profile_index
                    )
                })
                .vip
        }
        Entity::Soldier(soldier) => {
            assets
                .profile_manager
                .get_soldier(soldier.soldier.soldier_profile_index)
                .unwrap_or_else(|| {
                    panic!(
                        "parity VIP lookup is missing soldier profile {:?}",
                        soldier.soldier.soldier_profile_index
                    )
                })
                .vip
        }
        Entity::Civilian(civilian) => {
            civilian.civilian.cached_civilian_type == robin_engine::profiles::CivilianType::Vip
        }
        _ => panic!("parity VIP lookup called for non-human entity"),
    }
}

fn detectable_type_ordinal(detectable_type: robin_engine::element::DetectableType) -> u32 {
    match detectable_type {
        robin_engine::element::DetectableType::Enemy => 0,
        robin_engine::element::DetectableType::Body => 1,
        robin_engine::element::DetectableType::Object => 2,
        robin_engine::element::DetectableType::Friend => 3,
        robin_engine::element::DetectableType::MissedFriend => 4,
        robin_engine::element::DetectableType::Beggar => 5,
        robin_engine::element::DetectableType::None => 6,
    }
}

fn compare_float(
    differences: &mut Vec<String>,
    id: EntityId,
    field: &str,
    expected: TraceFloat,
    actual: f32,
) {
    compare_float_with_absolute_tolerance(differences, id, field, expected, actual, 0.0);
}

fn compare_float_with_absolute_tolerance(
    differences: &mut Vec<String>,
    id: EntityId,
    field: &str,
    expected: TraceFloat,
    actual: f32,
    absolute_tolerance: f32,
) {
    let expected_value = expected.value();
    let scale = expected_value.abs().max(actual.abs()).max(1.0);
    let logically_equal = (expected_value.is_nan() && actual.is_nan())
        || (expected_value.is_finite()
            && actual.is_finite()
            && (expected_value - actual).abs() <= (1.0e-5 * scale).max(absolute_tolerance));
    if !logically_equal {
        differences.push(format!(
            "{id:?}.{field}: original={} (0x{:08x}) rust={} (0x{:08x})",
            expected.value(),
            expected.bits,
            actual,
            actual.to_bits()
        ));
    }
}

fn compare_point(
    differences: &mut Vec<String>,
    id: EntityId,
    field: &str,
    expected: TracePoint,
    actual: MapPoint,
) {
    compare_float_component(differences, id, field, "x", expected.x, actual.x, 0.0);
    compare_float_component(differences, id, field, "y", expected.y, actual.y, 0.0);
}

fn compare_point_with_absolute_tolerance(
    differences: &mut Vec<String>,
    id: EntityId,
    field: &str,
    expected: TracePoint,
    actual: MapPoint,
    absolute_tolerance: f32,
) {
    compare_float_component(
        differences,
        id,
        field,
        "x",
        expected.x,
        actual.x,
        absolute_tolerance,
    );
    compare_float_component(
        differences,
        id,
        field,
        "y",
        expected.y,
        actual.y,
        absolute_tolerance,
    );
}

fn compare_float_component(
    differences: &mut Vec<String>,
    id: EntityId,
    field: &str,
    component: &str,
    expected: TraceFloat,
    actual: f32,
    absolute_tolerance: f32,
) {
    let expected_value = expected.value();
    let scale = expected_value.abs().max(actual.abs()).max(1.0);
    let logically_equal = (expected_value.is_nan() && actual.is_nan())
        || (expected_value.is_finite()
            && actual.is_finite()
            && (expected_value - actual).abs() <= (1.0e-5 * scale).max(absolute_tolerance));
    if !logically_equal {
        differences.push(format!(
            "{id:?}.{field}.{component}: original={} (0x{:08x}) rust={} (0x{:08x})",
            expected.value(),
            expected.bits,
            actual,
            actual.to_bits()
        ));
    }
}

fn compare_float_indexed(
    differences: &mut Vec<String>,
    id: EntityId,
    collection: &str,
    index: usize,
    field: &str,
    expected: TraceFloat,
    actual: f32,
) {
    let expected_value = expected.value();
    let scale = expected_value.abs().max(actual.abs()).max(1.0);
    let logically_equal = (expected_value.is_nan() && actual.is_nan())
        || (expected_value.is_finite()
            && actual.is_finite()
            && (expected_value - actual).abs() <= 1.0e-5 * scale);
    if !logically_equal {
        differences.push(format!(
            "{id:?}.{collection}[{index}].{field}: original={} (0x{:08x}) rust={} (0x{:08x})",
            expected.value(),
            expected.bits,
            actual,
            actual.to_bits()
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_motion_state_comparison_rejects_only_undefined_stack_values() {
        for raw in 0..=5 {
            assert!(original_motion_state_is_defined(raw));
        }
        assert!(!original_motion_state_is_defined(6));
        assert!(!original_motion_state_is_defined(4_160_286_488));
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
    fn schema16_npc_boundary_transients_are_typed_and_bounded() {
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
            "Original muwMaximalVisibility is a UWORD and must not be widened silently"
        );
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
        assert!(
            legacy_loaded_save_retains_process_transients(0),
            "in-game loaded-save sessions 5-8 have an empty post-load RNG prefix"
        );
        assert!(
            !legacy_loaded_save_retains_process_transients(76),
            "fresh-engine session 9 records mission-construction draws and must retain constructor zero"
        );
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

    fn minimal_test_native_header(source_fingerprint: &str) -> BinaryTraceHeader {
        BinaryTraceHeader {
            version: TRACE_NATIVE_VERSION,
            source_fingerprint: source_fingerprint.to_owned(),
            trace: TraceHeader {
                record_type: "header".to_owned(),
                mission: "test".to_owned(),
                proto_level: "test".to_owned(),
                rng_seed: 1,
                schema: 16,
                session_index: 1,
                start_state: TraceStartState::MissionStart,
                initial_frame: 0,
                simulation_hz: 25,
                synchronous_pathfinding: true,
                rng_stream: "libc_rand_raw_global_draw_order".to_owned(),
                visibility_queries: "opaque_is_reachable".to_owned(),
                authoritative_state: None,
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
                initial_npc_transients: None,
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
    fn native_level_nine_policy_round_trips_records_and_footer() {
        assert_eq!(TRACE_NATIVE_VERSION, 66);
        assert_eq!(TRACE_NATIVE_ZSTD_LEVEL, 9);
        assert!(!TRACE_NATIVE_LONG_DISTANCE_MATCHING);
        assert_eq!(TRACE_NATIVE_BLOCK_RECORDS, 16);
        assert_eq!(TRACE_NATIVE_WINDOW_LOG, 25);

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
    fn reblock_recovery_source_is_adjacent_and_version_specific() {
        let native = Path::new("dir/replay-001-session-0001.jsonl.zst.parity.bitcode.zst");
        assert_eq!(
            native_reblock_source_path(native),
            PathBuf::from(
                "dir/replay-001-session-0001.jsonl.zst.parity.bitcode.zst.parity-reblock-source-v66"
            )
        );
        assert_eq!(
            native_reblock_binding_path(native),
            PathBuf::from(
                "dir/replay-001-session-0001.jsonl.zst.parity.bitcode.zst.parity-reblock-binding-v66.json"
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
        let unrelated = directory.path().join(".parity-reblock-v66-unrelated");
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
    }

    #[test]
    fn native_reader_keeps_level_nineteen_ldm_version_sixty_six_compatible() {
        let footer = BinaryTraceFooter {
            version: 66,
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
    fn schema16_retained_terminal_success_selects_legacy_quit_repair() {
        assert!(is_legacy_retained_terminal_success(
            16,
            9_602,
            9_602,
            false,
            GameCode::LevelSucceeded as i32,
        ));
    }

    #[test]
    fn legacy_quit_repair_rejects_advancing_body_and_other_schema_or_code() {
        assert!(!is_legacy_retained_terminal_success(
            16,
            9_601,
            9_602,
            false,
            GameCode::LevelSucceeded as i32,
        ));
        for game_code in [
            GameCode::LevelInProgress,
            GameCode::LevelFailed,
            GameCode::LevelInterrupted,
        ] {
            assert!(!is_legacy_retained_terminal_success(
                16,
                9_602,
                9_602,
                false,
                game_code as i32,
            ));
        }
        assert!(!is_legacy_retained_terminal_success(
            16,
            9_602,
            9_602,
            true,
            GameCode::LevelSucceeded as i32,
        ));
        for schema in [15, 17] {
            assert!(!is_legacy_retained_terminal_success(
                schema,
                9_602,
                9_602,
                false,
                GameCode::LevelSucceeded as i32,
            ));
        }
    }

    #[test]
    fn legacy_retained_success_applies_full_quit_rollup_exactly_once() {
        use robin_engine::campaign::{Campaign, CampaignValue};
        use robin_engine::mission::MissionStatus;
        use robin_engine::player_profile::DifficultyLevel;

        let mut campaign = Campaign::default();
        campaign.values[CampaignValue::Score] = 7;
        let mut assets = LevelAssets::default();
        let mut engine = Engine::new_for_test_with_simulation(
            800.0,
            600.0,
            campaign,
            &mut assets,
            0,
            robin_engine::engine::SimConfig {
                script_enabled: false,
                ..robin_engine::engine::SimConfig::default()
            },
        )
        .expect("test engine");
        engine.test_set_mission_flags(false, false, true);
        let mut display = HostDisplayState::default();
        let mut input = InputState::default();
        let mut repaired = false;
        let mut repair_commands = Vec::new();

        assert!(append_legacy_retained_terminal_success_repair(
            &mut repair_commands,
            DifficultyLevel::Medium,
            16,
            9_602,
            9_602,
            false,
            GameCode::LevelSucceeded as i32,
            &mut repaired,
        ));
        assert!(!append_legacy_retained_terminal_success_repair(
            &mut repair_commands,
            DifficultyLevel::Medium,
            16,
            9_602,
            9_602,
            false,
            GameCode::LevelSucceeded as i32,
            &mut repaired,
        ));

        let output = engine
            .advance_frame(
                &mut display,
                &mut input,
                &assets,
                &mut DevState::default(),
                robin_engine::engine::SimulationFrameInput::new(
                    repair_commands
                        .into_iter()
                        .map(robin_engine::engine::SimCommand::from)
                        .collect(),
                )
                .with_simulation_body_allowed(false),
            )
            .expect("repair frame");
        assert_eq!(output.game_code(), GameCode::LevelSucceeded);
        let campaign = engine.into_campaign();
        assert_eq!(campaign.missions[0].status, MissionStatus::Won);
        assert_eq!(campaign.values[CampaignValue::Score], 1_007);
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
    fn movement_and_flight_steps_default_old_frames_and_preserve_exact_operands() {
        let old: TraceFrame = serde_json::from_value(minimal_frame_json()).unwrap();
        assert!(old.movement_steps.is_empty());
        assert!(old.flight_steps.is_empty());

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
    fn trace_sword_seek_distance_defaults_old_records_and_discards_direct_zero() {
        let old: TraceCommand = serde_json::from_value(serde_json::json!({
            "type": "sword_strike",
            "actor": { "kind": "pc", "index": 3 },
            "target": { "kind": "soldier", "index": 7 },
            "original_command": 78,
            "original_command_name": "swordstrike_thrust_a",
            "with_seek": true
        }))
        .expect("old sword trace without seek distance");
        let TraceCommand::SwordStrike { seek_distance, .. } = old else {
            panic!("decoded wrong trace command")
        };
        assert_eq!(seek_distance, None);

        assert_eq!(normalized_trace_sword_seek_distance(false, Some(0.0)), None);
        assert_eq!(
            normalized_trace_sword_seek_distance(true, Some(63.0)),
            Some(63.0)
        );
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
        // Unknown and large streams use the fixed 32 MiB replay-memory budget.
        assert_eq!(native_stream_window_log(None), TRACE_NATIVE_WINDOW_LOG);
        assert_eq!(
            native_stream_window_log(Some(192 * 1024 * 1024)),
            TRACE_NATIVE_WINDOW_LOG
        );
        assert_eq!(
            native_stream_window_log(Some(256 * 1024 * 1024)),
            TRACE_NATIVE_WINDOW_LOG
        );
        assert_eq!(
            native_stream_window_log(Some(256 * 1024 * 1024 + 1)),
            TRACE_NATIVE_WINDOW_LOG
        );
        // Tiny estimates retain the encoder's accepted minimum.
        assert_eq!(native_stream_window_log(Some(0)), 20);
        assert_eq!(native_stream_window_log(Some(1)), 20);
        assert_eq!(
            native_stream_window_log(Some(u64::MAX)),
            TRACE_NATIVE_WINDOW_LOG
        );
        assert_eq!(TRACE_NATIVE_BLOCK_RECORDS, 16);
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
    fn resolved_exclamation_schema_twelve_is_accepted() {
        validate_trace_schema(12);
    }

    #[test]
    fn authoritative_state_schema_thirteen_is_accepted() {
        validate_trace_schema(13);
    }

    #[test]
    fn refused_action_schema_fourteen_is_accepted() {
        validate_trace_schema(14);
    }

    #[test]
    fn door_and_route_diagnostics_schema_fifteen_are_accepted() {
        validate_trace_schema(15);

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
                }
            }
        }))
        .expect("parse schema-15 actor diagnostics");
        assert_eq!(actor.passing_door_directly, Some(true));
        assert_eq!(
            actor
                .active_pass_door
                .as_ref()
                .and_then(Option::as_ref)
                .map(|pass| (pass.gate_id, pass.direction)),
            Some((51, 1))
        );
        let pass = actor
            .active_pass_door
            .as_ref()
            .and_then(Option::as_ref)
            .unwrap();
        assert!(active_pass_door_keys_match(Some(pass), Some((51, true))));
        assert!(!active_pass_door_keys_match(Some(pass), Some((51, false))));
        assert!(!active_pass_door_keys_match(Some(pass), None));
        assert_eq!(
            actor
                .sequence_element
                .as_ref()
                .and_then(Option::as_ref)
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
            .expect("parse schema-15 route-construction diagnostics");
        validate_trace_frame_envelope(15, &frame);
        let route = &frame.route_construction_events.as_ref().unwrap()[0];
        assert_eq!(route.actor.index, 43);
        assert_eq!(route.gates[0].gate_id, 51);
        assert!(!route.gates[0].direct);
    }

    #[test]
    fn schema_sixteen_jump_lines_are_typed_parallel_and_cache_safe() {
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
        validate_human_jump_line_shape(&entity, &human);

        let ai: TraceAi = serde_json::from_value(serde_json::json!({
            "state": 1,
            "substate": 2,
            "my_line_jump": null
        }))
        .expect("parse authoritative null soldier jump line");
        assert!(matches!(ai.my_line_jump, Some(None)));

        let encoded = bitcode::encode(&(human, ai));
        let (cached_human, cached_ai): (TraceHuman, TraceAi) =
            bitcode::decode(&encoded).expect("restore typed jump-line snapshots");
        assert_eq!(cached_human.opponent_jump_lines.as_ref().unwrap().len(), 1);
        assert!(matches!(cached_ai.my_line_jump, Some(None)));

        let malformed_line = serde_json::json!({
            "a": {"x": {"bits": 0}, "y": {"bits": 0}},
            "b": {"x": {"bits": 0}, "y": {"bits": 0}},
            "pointer": 123
        });
        assert!(serde_json::from_value::<TraceJumpLine>(malformed_line).is_err());
    }

    #[test]
    #[should_panic(expected = "opponent and jump-line arrays differ in length")]
    fn schema_sixteen_rejects_misaligned_opponent_jump_lines() {
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
        validate_human_jump_line_shape(&entity, &human);
    }

    #[test]
    fn schema_sixteen_event_window_resets_only_after_record_frame() {
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
        next_ordinal = 0; // RecordFrame serialized and then reset the window.

        emit(
            &mut pending,
            &mut next_ordinal,
            "refresh_after_record_frame",
        );
        // BeginEngineFrame intentionally does not clear or reset event state.
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
    fn schema_sixteen_target_unreached_observations_are_explicitly_nullable() {
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
    fn schema_sixteen_movement_action_is_only_present_when_initialized() {
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
    fn extensible_diagnostics_schema_sixteen_are_cached_and_required() {
        validate_trace_schema(16);

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
        .expect("parse schema-16 actor and sequence diagnostics");
        assert!(actor.position_interface.is_some());
        let sequence = actor.sequence_element.as_ref().unwrap().as_ref().unwrap();
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
                "stay_on_post": true,
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

        let frame: TraceFrame = serde_json::from_value(frame_json)
            .expect("parse extensible schema-16 diagnostic streams");
        validate_trace_frame_envelope(16, &frame);
        assert_eq!(frame.popup_events.as_ref().unwrap().len(), 1);
        assert_eq!(frame.popup_events.as_ref().unwrap()[0].ordinal, Some(0));
        assert_eq!(
            frame.ai_forecast_events.as_ref().unwrap()[0]
                .phase
                .as_deref(),
            Some("resolved")
        );
        assert_eq!(
            frame.alert_formation_events.as_ref().unwrap()[0].ordinal,
            Some(0)
        );
        let authorizations = frame.goto_authorization_events.as_ref().unwrap();
        assert_eq!(authorizations[0].source.as_ref().unwrap().layer, 2);
        assert!(authorizations[0].move_box.is_some());
        assert!(authorizations[1].source.is_none());
        assert!(authorizations[1].move_box.is_none());
        let target = &frame.target_lifecycle_events.as_ref().unwrap()[0];
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
        let proposals = frame.strike_proposal_events.as_ref().unwrap();
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
        let lifecycle = frame.sequence_lifecycle_events.as_ref().unwrap();
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
            frame.route_construction_events.as_ref().unwrap()[0]
                .draft_diagnostics
                .contains_key("ordinal")
        );
        assert!(
            frame.route_construction_events.as_ref().unwrap()[0].gates[0]
                .draft_diagnostics
                .contains_key("score")
        );
        let encoded = bitcode::encode(&frame);
        let cached: TraceFrame = bitcode::decode(&encoded)
            .expect("decode schema-16 frame from native cache representation");
        assert_eq!(cached.alert_formation_events.as_ref().unwrap().len(), 1);
        assert_eq!(
            cached.target_lifecycle_events.as_ref().unwrap()[0].sequence_id,
            Some(701)
        );
        let cached_authorizations = cached.goto_authorization_events.as_ref().unwrap();
        assert_eq!(cached_authorizations[0].source.as_ref().unwrap().layer, 2);
        assert!(cached_authorizations[0].move_box.is_some());
        assert!(cached_authorizations[1].source.is_none());
        assert!(cached_authorizations[1].move_box.is_none());
        assert_eq!(cached.strike_proposal_events.as_ref().unwrap().len(), 5);
        assert_eq!(cached.sequence_lifecycle_events.as_ref().unwrap().len(), 5);
        assert!(
            cached.route_construction_events.as_ref().unwrap()[0]
                .draft_diagnostics
                .contains_key("result")
        );

        let mut incomplete_json = minimal_frame_json();
        incomplete_json["route_construction_events"] = serde_json::json!([]);
        let incomplete: TraceFrame = serde_json::from_value(incomplete_json).unwrap();
        assert!(!trace_frame_envelope_matches(16, &incomplete));

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

        let mut legacy_schema_sixteen = minimal_frame_json();
        legacy_schema_sixteen["route_construction_events"] = serde_json::json!([]);
        legacy_schema_sixteen["popup_events"] = serde_json::json!([]);
        legacy_schema_sixteen["ai_forecast_events"] = serde_json::json!([]);
        legacy_schema_sixteen["alert_formation_events"] = serde_json::json!([]);
        legacy_schema_sixteen["goto_authorization_events"] = serde_json::json!([]);
        let legacy: TraceFrame = serde_json::from_value(legacy_schema_sixteen).unwrap();
        assert!(trace_frame_envelope_matches(16, &legacy));
        assert!(legacy.target_lifecycle_events.is_none());
    }

    #[test]
    fn schema_fourteen_frames_carry_no_authoritative_state() {
        let frame: TraceFrame = serde_json::from_value(minimal_frame_json()).unwrap();
        validate_trace_frame_envelope(14, &frame);
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
            .expect("valid schema-12 initial_save");
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
                Some(Some(18)),
                Some(true),
                &unique,
            )
            .map(|(path, waypoint, offset)| (path.get(), waypoint, offset)),
            Some((0, 0, 18))
        );
        assert!(
            terminal_macro_waypoint_at(
                (353_f32.to_bits(), 1050_f32.to_bits()),
                Some(Some(18)),
                Some(false),
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
                Some(Some(18)),
                Some(true),
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
            .expect("valid Windows i386 schema-12 initial_save");
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
    }

    #[test]
    #[should_panic(expected = "schemas 12 through 16 are supported")]
    fn schema_eleven_is_rejected() {
        validate_trace_schema(11);
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
            "resolved_exclamations": [],
            "rng_draws": {
                "first_index": 0,
                "values": [],
                "callsite_offsets": [],
                "main_thread": [],
                "domains": []
            }
        })
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
            "novel_recorder_field": 123
        }]);
        let line = stray.to_string();
        let frame: TraceFrame = serde_json::from_str(&line).unwrap();
        let panic =
            std::panic::catch_unwind(|| verify_trace_line_roundtrip(&frame, &line, 3)).unwrap_err();
        let message = panic
            .downcast_ref::<String>()
            .expect("round-trip audit panics with a formatted message");
        assert!(
            message.contains("$.elements[0].novel_recorder_field is dropped"),
            "unexpected audit failure message: {message}"
        );
    }

    #[test]
    fn alert_eligibility_round_trip_canonicalizes_legacy_post_key_without_layout_change() {
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

        let legacy_json = serde_json::json!({
            "rank": true,
            "stay_on_post": true,
        });
        let current_json = serde_json::json!({
            "rank": true,
            "allowed_to_leave_post": true,
        });
        let legacy: TraceAlertEligibility = serde_json::from_value(legacy_json.clone()).unwrap();
        let current: TraceAlertEligibility = serde_json::from_value(current_json.clone()).unwrap();
        assert_eq!(legacy.allowed_to_leave_post, Some(true));
        assert_eq!(current.allowed_to_leave_post, Some(true));
        assert_eq!(bitcode::encode(&legacy), bitcode::encode(&current));

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

        let serialized = serde_json::to_value(&legacy).unwrap();
        assert_eq!(serialized["allowed_to_leave_post"], serde_json::json!(true));
        assert!(serialized.get("stay_on_post").is_none());

        for duplicate in [
            r#"{"rank":true,"stay_on_post":true,"allowed_to_leave_post":true}"#,
            r#"{"rank":true,"allowed_to_leave_post":false,"stay_on_post":false}"#,
        ] {
            assert!(
                serde_json::from_str::<TraceAlertEligibility>(duplicate).is_err(),
                "both post keys must be rejected regardless of order or value"
            );
        }

        for eligibility in [legacy_json, current_json] {
            let mut frame_json = minimal_frame_json();
            frame_json["alert_formation_events"] = serde_json::json!([{
                "ordinal": 0,
                "stage": "candidate",
                "invocation": 9,
                "scan_index": 2,
                "candidate": {"kind": "soldier", "index": 8},
                "eligibility": eligibility,
            }]);

            let line = frame_json.to_string();
            let frame: TraceFrame = serde_json::from_str(&line).unwrap();
            verify_trace_line_roundtrip(&frame, &line, 3);
        }

        let mut outside_alert_events = serde_json::json!({
            "eligibility": {"stay_on_post": true},
        });
        normalize_trace_json_for_roundtrip(&mut outside_alert_events);
        assert_eq!(
            outside_alert_events,
            serde_json::json!({"eligibility": {"stay_on_post": true}}),
            "legacy spelling normalization must stay scoped to alert formation events"
        );

        let mut nested_inside_event = serde_json::json!({
            "alert_formation_events": [{
                "nested": {"eligibility": {"stay_on_post": true}},
            }],
        });
        normalize_trace_json_for_roundtrip(&mut nested_inside_event);
        assert_eq!(
            nested_inside_event["alert_formation_events"][0]["nested"]["eligibility"],
            serde_json::json!({"stay_on_post": true}),
            "normalization must only inspect an alert event's direct eligibility field"
        );
    }

    #[test]
    fn schema_twelve_and_thirteen_frame_envelopes_are_distinct() {
        let schema_twelve: TraceFrame = serde_json::from_value(minimal_frame_json()).unwrap();
        assert!(trace_frame_envelope_matches(12, &schema_twelve));
        assert!(!trace_frame_envelope_matches(13, &schema_twelve));

        let mut schema_thirteen_json = minimal_frame_json();
        schema_thirteen_json["campaign"] = serde_json::json!({
            "version": 1,
            "values": [],
            "ares": -1,
            "missions": [],
            "accessible_mission_indices": [],
            "pending_accessible_mission_indices": [],
            "last_mission_index": null,
            "current_mission_index": null,
            "next_mission_index": null,
            "blazon_mission_index": null,
            "last_played_mission_indices": [],
            "last_pseudo_mission_status": 0,
            "last_pseudo_mission_id": 0,
            "characters": [],
            "gang_indices": [],
            "reservist_indices": [],
            "mission_team_indices": [],
            "peasant_names": [],
            "reservists_are_back": false,
            "collected_relics": [],
            "production_sectors": []
        });
        schema_thirteen_json["engine_state"] = serde_json::json!({
            "cheat_used_flags": 0,
            "next_creation_order": 31,
            "chorus_timer": 0,
            "force_check": false,
            "men_to_blazon_conversion": false,
            "pc_registry": [],
            "lock_engine": false,
            "freeze_all": false,
            "locker": false,
            "speed": {"bits": 1065353216},
            "speed_int": 1,
            "mission_won": false,
            "mission_won_first_time": false,
            "quit_won": false,
            "quit_lost": false,
            "quit_interrupted": false,
            "script_globals": [],
            "sequence_manager": {
                "sequences": [], "elements_to_go": [], "actor_current": []
            },
            "script_runtime": {
                "static_words": [], "instances": [], "computed_locations": []
            },
            "pathfinder": {
                "status": "waiting",
                "ignore_next_path": false,
                "number_of_attempts": 1,
                "area_states": [],
                "requests": []
            },
            "view_radius_cache": {"frame": 0, "entries": []},
            "sound_sources": [],
            "sound_completion_frontier": [],
            "ai_global": {
                "stupid_soldiers_cheat": false,
                "seek_points": [],
                "archery_sectors": [],
                "green_alert_soldiers": 0,
                "yellow_alert_soldiers": 0,
                "red_alert_soldiers": 0,
                "overall_alert_status": 0,
                "overall_villain_alert_status": 0,
                "saved_random_seed": 0,
                "forbidden_remarks": [],
                "current_speech_variant": 0
            },
            "engine_runtime_roots": {
                "timer_elements": [],
                "camera_sequence": null,
                "dead_pc": null,
                "mission_stat": {
                    "collected_money": 0,
                    "bonus_money": 0,
                    "soldier_money": 0,
                    "living_soldier_count": 0,
                    "total_soldier_count": 0,
                    "new_peasant_count": 0,
                    "killed_peasant_count": 0,
                    "killed_allied_count": 0,
                    "added_score": 0,
                    "pc_names": []
                },
                "user_locked": false,
                "selection_before_user_lock": [],
                "follow_element": null
            },
            "world_interactables": {
                "patches": [],
                "doors": [],
                "sector_doors": [],
                "lifts": [],
                "buildings": [],
                "script_zones": []
            },
            "repulsive_points": {
                "next_id": 0,
                "points": []
            },
            "titbit_manager": {
                "current_id": 0,
                "titbits": []
            },
            "failed_path_requests": []
        });
        let schema_thirteen: TraceFrame = serde_json::from_value(schema_thirteen_json).unwrap();
        assert!(trace_frame_envelope_matches(13, &schema_thirteen));
        assert!(!trace_frame_envelope_matches(12, &schema_thirteen));
    }

    #[test]
    fn old_raw_world_snapshots_skip_only_absent_additive_v26_state() {
        let expected = serde_json::json!({
            "patches": [],
            "doors": [{ "locked_pc": true }],
            "sector_doors": [],
            "lifts": []
        });
        let mut actual = serde_json::json!({
            "patches": [],
            "doors": [{
                "locked_pc": true,
                "locked_pc_after_patch": false,
                "locked_npc_villain_after_patch": false,
                "locked_npc_civilian_after_patch": false,
                "unlockable_after_patch": false
            }],
            "sector_doors": [],
            "lifts": [],
            "buildings": [{ "occupants": [], "arrow_reserve": false }],
            "script_zones": [{
                "occupants": [],
                "transformed_to_apex": false,
                "max_apex_height": null
            }]
        });

        retain_recorded_world_interactable_coverage(&expected, &mut actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn old_raw_sequence_snapshots_skip_only_absent_additive_v28_order_state() {
        let expected = serde_json::json!({
            "sequences": [{
                "elements": [{
                    "orders": [{ "action": 6, "done": false }]
                }]
            }]
        });
        let mut actual = serde_json::json!({
            "next_order_id": 44,
            "sequences": [{
                "elements": [{
                    "orders": [{
                        "action": 6,
                        "done": false,
                        "destination_3d": {},
                        "flight_vector": {},
                        "apply_transition": false,
                        "can_fly": false,
                        "lock_ai": false,
                        "transition": false,
                        "id": 43
                    }]
                }]
            }]
        });

        retain_recorded_order_runtime_coverage(&expected, &mut actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn old_enemy_snapshots_skip_only_absent_additive_v45_state() {
        let expected = serde_json::json!({
            "npc_ai": {
                "state": 3,
                "subclass": {
                    "kind": "enemy",
                    "frame_when_missed_charly": 17
                }
            }
        });
        let mut actual = serde_json::json!({
            "human_continuation": {
                "already_detectable_body": false,
                "sword_strike_boredom": [0, 0, 0, 0, 0, 0, 0, 0, 0]
            },
            "npc_ai": {
                "state": 3,
                "stimulus_queue": [],
                "object_memory": {
                    "forgotten": [], "desire": null,
                    "checkpoint_charly": null, "synchronize_charly": null
                },
                "synchronizing_actors": [],
                "reconnaissance": {
                    "report_type": 0, "seek_position": {}, "seen_bodies": [],
                    "charly": null, "charly_seen": false
                },
                "path_control": {
                    "stop_before_end": false, "use_max_norm": false, "stop_distance": 0,
                    "status": {
                        "current_waypoint_index": 0, "last_waypoint_index": 0,
                        "forward": true, "hiking_path_index": null, "history": []
                    },
                    "has_patrol_path": false, "macro_cursor": null
                },
                "legacy_continuation": {
                    "remaining_tequila_gulps": 0,
                    "last_hint_actuality": 0,
                    "last_hint_subject": 1,
                    "current_door": null,
                    "looking_for_help_because_enemy_seen": false
                },
                "subclass": {
                    "kind": "enemy",
                    "frame_when_missed_charly": 17,
                    "heard_nets": [],
                    "other_seen_ale": [],
                    "search_charly_way": [],
                    "missed_in_action": [],
                    "other_bodies_to_examine": [],
                    "beggars_to_control": [],
                    "them": [],
                    "ambush_point_array_reset": false,
                    "ambush_point_status": [],
                    "my_seek_points": [],
                    "personal_seek_point_1": null,
                    "personal_seek_point_2": null,
                    "seek_center": {},
                    "actual_seek_point": null,
                    "seek_point_view_directions": [],
                    "positions_of_beggars_to_control": [],
                    "seek_flags": 0,
                    "seen_dead_body": false,
                    "seeking_charly": false,
                    "forced_next_battle_decision": 0,
                    "reset_battle_decision": false,
                    "synchronize_index": 0,
                    "initial_view_cone": 0,
                    "company_number": 0,
                    "left_combat_neighbour": null,
                    "right_combat_neighbour": null,
                    "attentive": false,
                    "will_be_attentive": false,
                    "forced_attentive": false,
                    "guarded_pc": null,
                    "tower_guard": false,
                    "combat_trainer": false,
                    "gather_position": {},
                    "gather_direction": 0,
                    "gather_position_instructed": false,
                    "officers_position": {},
                    "previous_state": 0,
                    "previous_substate": 0,
                    "reported_to_officer": false,
                    "missed_soldier_timer": 0,
                    "old_money": 0,
                    "other_seen_money": [],
                    "money_fight_enemies": [],
                    "money_fight_victims": [],
                    "archer_behind_me": null,
                    "shield_bearer_before_me": null,
                    "already_seen_bodies": [],
                    "my_line_jump": null,
                    "shield_bearer_direction": 0,
                    "phalanx_aborted": false,
                    "changed_to_alert_path": false,
                    "shooting_point": null,
                    "archery_sector": null,
                    "archery_sector_index": 0,
                    "archery_point_index": 0,
                    "archery_point_increment": 0,
                    "enemy_seen_below": false,
                    "enemy_had_this_elevation": 0,
                    "known_enemy_strike_commands": [0, 0, 0],
                    "last_stimulus_dispatched_to_patrol": null
                }
            }
        });

        retain_recorded_entity_runtime_coverage(&expected, &mut actual);
        assert_eq!(actual, expected);

        let expected = serde_json::json!({});
        let mut actual = serde_json::json!({
            "pc_core": { "current_action": 0, "disabled_actions": [false, false, false] }
        });
        retain_recorded_entity_runtime_coverage(&expected, &mut actual);
        assert_eq!(actual, expected);

        let expected = serde_json::json!({});
        let mut actual = serde_json::json!({
            "pc_portrait": { "open": false, "burned": false }
        });
        retain_recorded_entity_runtime_coverage(&expected, &mut actual);
        assert_eq!(actual, expected);

        let expected = serde_json::json!({});
        let mut actual = serde_json::json!({
            "human_structure": { "opponents": [], "pending_shoots": [] }
        });
        retain_recorded_entity_runtime_coverage(&expected, &mut actual);
        assert_eq!(actual, expected);

        let expected = serde_json::json!({});
        let mut actual = serde_json::json!({ "pc_tail": { "carried": null } });
        retain_recorded_entity_runtime_coverage(&expected, &mut actual);
        assert_eq!(actual, expected);

        let expected = serde_json::json!({});
        let mut actual = serde_json::json!({ "pc_qa": [] });
        retain_recorded_entity_runtime_coverage(&expected, &mut actual);
        assert_eq!(actual, expected);

        let expected = serde_json::json!({});
        let mut actual = serde_json::json!({
            "pc_interface": { "playable": true, "displayed": true }
        });
        retain_recorded_entity_runtime_coverage(&expected, &mut actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn simulation_body_marker_is_mandatory() {
        let mut frame_without_marker = minimal_frame_json();
        frame_without_marker
            .as_object_mut()
            .unwrap()
            .remove("simulation_body_ran");

        let error = serde_json::from_value::<TraceFrame>(frame_without_marker)
            .expect_err("schema 12 frames must report whether the simulation body ran");
        assert!(error.to_string().contains("simulation_body_ran"));
    }

    #[test]
    fn schema_nine_path_events_are_typed() {
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
        .expect("parse clean schema-12 terminator");
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
                "wait_time": 25
            },
            "human": {
                "life_points": 60,
                "dead": false,
                "unconscious": false,
                "camp": "lacklandists",
                "original_camp": 1,
                "vip": true,
                "civilian": false,
                "opponents": [{"kind": "pc", "index": 2}]
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
                "list_them": [{"kind": "pc", "index": 2}]
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
            }
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
        assert_eq!(element.sprite_frame_count, Some(u16::MAX));
        let human = element.human.expect("human state");
        assert_eq!(human.camp, "lacklandists");
        assert!(human.vip);
        let ai = element.ai.expect("AI state");
        assert_eq!(ai.macro_cursor, Some(Some(4)));
        assert_eq!(
            ai.list_them.as_deref(),
            Some(
                &[TraceEntityId {
                    kind: TraceEntityKind::Pc,
                    index: 2,
                }][..]
            )
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
    fn additive_ai_diagnostics_are_optional_within_schema_twelve() {
        let ai: TraceAi = serde_json::from_value(serde_json::json!({
            "state": 3,
            "substate": 17,
            "script_locked": false,
            "locked": true,
            "list_us": [],
            "list_them": []
        }))
        .expect("schema-12 AI state predating additive diagnostics remains readable");

        assert_eq!(ai.state, 3);
        assert_eq!(ai.substate, 17);
        assert_eq!(ai.locks, None);
        assert_eq!(ai.was_busy, None);
        assert_eq!(ai.very_busy, None);
        assert_eq!(ai.macro_timer_running, None);
        assert_eq!(ai.macro_timer_ring, None);
        assert_eq!(ai.macro_cursor, None);
        assert_eq!(ai.macro_remaining, None);
        assert_eq!(ai.macro_in_progress, None);
        assert_eq!(ai.list_us, Some(Vec::new()));
        assert_eq!(ai.list_them, Some(Vec::new()));

        let early_schema_twelve: TraceAi = serde_json::from_value(serde_json::json!({
            "state": 3,
            "substate": 17
        }))
        .expect("early schema-12 AI state with only state/substate remains readable");
        assert_eq!(early_schema_twelve.script_locked, None);
        assert_eq!(early_schema_twelve.locked, None);
        assert_eq!(early_schema_twelve.list_us, None);
        assert_eq!(early_schema_twelve.list_them, None);

        let early_human: TraceHuman = serde_json::from_value(serde_json::json!({
            "life_points": 60,
            "dead": false,
            "unconscious": false,
            "camp": "outlaw",
            "original_camp": 0,
            "vip": false,
            "civilian": false
        }))
        .expect("early schema-12 human state without opponents remains readable");
        assert_eq!(early_human.opponents, None);

        let recorded_null_cursor: TraceAi = serde_json::from_value(serde_json::json!({
            "state": 3,
            "substate": 17,
            "script_locked": false,
            "locked": true,
            "macro_cursor": null,
            "list_us": [],
            "list_them": []
        }))
        .expect("recorded null macro cursor is distinct from a missing diagnostic");
        assert_eq!(recorded_null_cursor.macro_cursor, Some(None));
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
                "original_command": 0, "original_command_name": "hit", "with_seek": true}),
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

        let (before, after) = split_late_refresh_orientations(commands, false);

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
        let (before, after) =
            split_late_refresh_orientations(vec![orientation(), orientation()], true);

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

        let (before, after) = split_late_refresh_orientations(vec![command], true);

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
                robin_engine::gate::DoorIndex(0),
                robin_engine::gate::DoorIndex(3),
                robin_engine::gate::DoorIndex(1),
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

    fn group_move_route_map(max_gate: u32) -> EntityMap {
        EntityMap {
            entities: BTreeMap::new(),
            entities_by_creation_order: BTreeMap::new(),
            sectors: BTreeMap::new(),
            sector_indices: BTreeMap::new(),
            gates: (0..=max_gate).map(robin_engine::gate::DoorIndex).collect(),
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
    fn schema_sixteen_group_move_recovers_ordinary_route_over_door_overlay() {
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
            resolve_schema_sixteen_group_move_route(
                16,
                &command,
                Some(&routes),
                &mut consumed,
                &map,
                &sectors,
            ),
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
    fn schema_sixteen_same_sector_group_move_retains_ordinary_goal_kind() {
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
            resolve_schema_sixteen_group_move_route(
                16,
                &command,
                None,
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
    fn schema_sixteen_group_moves_share_frame_routes_in_command_order() {
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

        let first_resolution = resolve_schema_sixteen_group_move_route(
            16,
            &command,
            Some(&routes),
            &mut consumed,
            &map,
            &sectors,
        )
        .unwrap();
        assert_eq!(
            first_resolution.recorded_gate_routes,
            vec![(actor, vec![(53, false)])]
        );
        assert_eq!(consumed, BTreeSet::from([40]));

        let second_resolution = resolve_schema_sixteen_group_move_route(
            16,
            &command,
            Some(&routes),
            &mut consumed,
            &map,
            &sectors,
        )
        .unwrap();
        assert_eq!(
            second_resolution.recorded_gate_routes,
            vec![(actor, vec![(54, true)])]
        );
        assert_eq!(consumed, BTreeSet::from([40, 41]));
    }

    #[test]
    fn schema_sixteen_group_move_recovers_internal_door_branch_from_retained_goal_kind() {
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
            resolve_schema_sixteen_group_move_route(
                16,
                &command,
                Some(&routes),
                &mut consumed,
                &map,
                &sectors,
            ),
            Some(ReplayGroupMoveResolution {
                door_route: true,
                unmapped_goal_search_sector: Some(64),
                recorded_gate_routes: vec![(actor, vec![(53, false)])],
                recorded_failed_gate_routes: Vec::new(),
            })
        );
        let mut legacy_consumed = BTreeSet::new();
        assert_eq!(
            resolve_schema_sixteen_group_move_route(
                15,
                &command,
                Some(&routes),
                &mut legacy_consumed,
                &map,
                &sectors,
            ),
            None
        );
        assert!(legacy_consumed.is_empty());
    }

    #[test]
    fn schema_sixteen_group_move_retains_door_branch_for_failed_empty_route() {
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
            resolve_schema_sixteen_group_move_route(
                16,
                &command,
                Some(&routes),
                &mut consumed,
                &map,
                &sectors,
            ),
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
    fn schema_sixteen_failed_ordinary_group_move_is_authoritative() {
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
            resolve_schema_sixteen_group_move_route(
                16,
                &command,
                Some(&[route]),
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
        let resolution = resolve_schema_sixteen_group_move_route(
            16,
            &command,
            Some(&[route]),
            &mut consumed,
            &map,
            &sectors,
        );

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
            gates: Vec::new(),
            draft_diagnostics: BTreeMap::from([(
                "ordinal".to_owned(),
                TraceJsonValue::from(TraceJsonTree::Unsigned(7)),
            )]),
        }
    }

    fn drop_ale_route_map(actor: TraceEntityId) -> EntityMap {
        let goal_sector_index = robin_engine::fast_find_grid::SectorIndex::new(37).unwrap();
        EntityMap {
            entities: BTreeMap::from([(actor, EntityId::Pc(robin_engine::entity_id::PcId(12)))]),
            entities_by_creation_order: BTreeMap::new(),
            sectors: BTreeMap::from([(148, 55)]),
            sector_indices: BTreeMap::from([(148, goal_sector_index)]),
            gates: Vec::new(),
            runtime_creation_order_boundary: 0,
        }
    }

    #[test]
    fn schema_sixteen_drop_ale_recovers_save067_route_goal() {
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
            resolve_schema_sixteen_drop_ale(
                16,
                &command,
                Some(&routes),
                &mut consumed,
                &drop_ale_route_map(actor),
                None,
            ),
            Some(ReplayDropAleResolution {
                goal: (SectorNumber::new(55), 4),
                goal_sector_index: robin_engine::fast_find_grid::SectorIndex::new(37),
            })
        );
        assert_eq!(consumed, BTreeSet::from([7]));
    }

    #[test]
    fn drop_ale_route_recovery_rejects_nonmatching_point_and_legacy_schema() {
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
            resolve_schema_sixteen_drop_ale(16, &command, Some(&routes), &mut consumed, &map, None,),
            None
        );
        assert_eq!(
            resolve_schema_sixteen_drop_ale(
                15,
                &command,
                Some(&[drop_ale_route_fixture(actor, target)]),
                &mut consumed,
                &map,
                None,
            ),
            None
        );
        assert!(consumed.is_empty());
    }

    #[test]
    #[should_panic(expected = "has no retained Rust position-sector mapping")]
    fn schema_sixteen_drop_ale_rejects_unmapped_authoritative_goal() {
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

        let _ = resolve_schema_sixteen_drop_ale(
            16,
            &command,
            Some(&[drop_ale_route_fixture(actor, target)]),
            &mut BTreeSet::new(),
            &map,
            None,
        );
    }

    #[test]
    fn schema_sixteen_drop_ale_recovers_same_sector_actor_goal_without_route() {
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
        };
        let mut consumed = BTreeSet::new();

        assert_eq!(
            resolve_schema_sixteen_drop_ale(
                16,
                &command,
                None,
                &mut consumed,
                &drop_ale_route_map(actor),
                Some(expected),
            ),
            Some(expected)
        );
        assert!(consumed.is_empty());
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

/// Wraps a Rust entity id so its `Debug` rendering also carries the Original
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
