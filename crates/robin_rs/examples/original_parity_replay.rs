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
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{collections::BTreeMap, collections::BTreeSet, collections::VecDeque};

use base64::Engine as _;
use robin_engine::coordinates::MapPoint;
use robin_engine::coordinates::WorldPoint3D;
use robin_engine::element::{Command, Entity, EntityId, EntityIdKind};
use robin_engine::engine::{DevState, Engine, HostDisplayState, InputState, LevelAssets};
use robin_engine::fast_find_grid::LineIndex;
use robin_engine::graphic_config::TextureScaleMode;
use robin_engine::player_command::PlayerCommand;
use robin_engine::profiles::Action;
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

#[derive(Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
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
    #[serde(default)]
    initial_save: Option<TraceInitialSave>,
}

#[derive(Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
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
    Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, bincode::Encode, bincode::Decode,
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

#[derive(Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
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

#[derive(Clone, Copy, Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
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
    Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, bincode::Encode, bincode::Decode,
)]
#[serde(rename_all = "snake_case")]
enum TraceStartState {
    MissionStart,
    LoadedSave,
}

fn validate_trace_schema(schema: u32) {
    assert!(
        matches!(schema, 12 | 13 | 14),
        "unsupported parity trace schema {schema}; schemas 12, 13 and 14 are supported"
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
        12 | 14 => assert!(
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
}

fn decode_and_validate_initial_save(header: &TraceHeader) -> Option<Vec<u8>> {
    match (
        header.schema,
        header.start_state,
        header.initial_save.as_ref(),
    ) {
        (12 | 13 | 14, TraceStartState::MissionStart, None) => None,
        (12 | 13 | 14, TraceStartState::MissionStart, Some(_)) => {
            panic!("mission_start traces must not contain initial_save")
        }
        (12 | 13 | 14, TraceStartState::LoadedSave, None) => {
            panic!("loaded_save traces require initial_save")
        }
        (12 | 13 | 14, TraceStartState::LoadedSave, Some(initial_save)) => {
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

#[derive(Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
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

#[derive(Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
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

#[derive(Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
struct TraceCampaignCharacter {
    profile_index: u32,
    profile_name: String,
    instanced: bool,
    status: TracePcStatus,
}

#[derive(Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
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

#[derive(Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
struct TraceSkill {
    capacity: u32,
    experience: u32,
}

#[derive(Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
struct TraceProductionSector {
    r#type: u32,
    speed: u16,
    amount: u16,
    produced_amount: u16,
    max_amount_reached: bool,
    occupants: Vec<TraceProductionOccupant>,
}

#[derive(Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
struct TraceProductionOccupant {
    character_index: usize,
    x: TraceFloat,
    y: TraceFloat,
    obstacle: u16,
}

#[derive(Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
struct TraceMotionGrid {
    layers: Vec<TraceMotionLayer>,
}

#[derive(Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
struct TraceMotionLayer {
    layer: u16,
    lines: Vec<TraceMotionLine>,
}

#[derive(Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
struct TraceMotionLine {
    index: u16,
    a: TracePoint,
    b: TracePoint,
    type_mask: i32,
    associated_sector: i16,
    active: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
struct TraceMotionLineChange {
    layer: u16,
    index: u16,
    active: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
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

#[derive(Debug, Clone, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
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
    Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, bincode::Encode, bincode::Decode,
)]
#[serde(rename_all = "snake_case")]
enum TraceRngDomain {
    Simulation,
    Audio,
}

#[derive(Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
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

#[derive(Debug, Clone, Copy, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
struct TraceFloat {
    bits: u32,
}

impl TraceFloat {
    fn value(self) -> f32 {
        f32::from_bits(self.bits)
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
struct TracePoint {
    x: TraceFloat,
    y: TraceFloat,
}

impl From<TracePoint> for MapPoint {
    fn from(value: TracePoint) -> Self {
        Self::new(value.x.value(), value.y.value())
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
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

#[derive(Debug, Serialize, Deserialize, bincode::Encode, bincode::Decode)]
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
        original_command_name: String,
        running: bool,
    },
    LaunchSelfAbility {
        actor: TraceEntityId,
        original_command_name: String,
    },
    LaunchGroundTarget {
        actor: TraceEntityId,
        target: TracePoint3,
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
        original_command_name: String,
        with_seek: bool,
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
    },
    CancelAction {
        #[serde(default)]
        pc: Option<TraceEntityId>,
    },
    OrientActionAt {
        action: TraceAction,
        actor: TraceEntityId,
        mouse_map: TracePoint,
        target: TracePoint3,
    },
    MakePcFast {
        entity: TraceEntityId,
    },
    CrouchDown,
    StandUp,
    // Keep newly supported variants at the end: the native parity cache uses
    // bincode's enum discriminants, so appending preserves existing cache
    // compatibility.
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
        #[serde(default)]
        target: Option<TraceEntityId>,
        reason: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, bincode::Encode, bincode::Decode)]
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
    fn into_player_command(self, entity_map: &EntityMap, engine: &Engine) -> Option<PlayerCommand> {
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
                let goal_override = {
                    let raw_index = usize::try_from(goal_sector).unwrap_or_else(|_| {
                        panic!("group-move has negative sector index {goal_sector}")
                    });
                    // The Original records its heterogeneous fast-grid
                    // sector-array index, which Rust does not retain. Resolve
                    // its isomorphic motion area from the other authoritative
                    // command fields.
                    let containing = engine
                        .fast_grid()
                        .level
                        .sectors
                        .iter()
                        .filter(|sector| {
                            sector.layer == goal_layer && sector.contains_point(destination)
                        })
                        .collect::<Vec<_>>();
                    let has_semantic_overlay = containing
                        .iter()
                        .any(|sector| sector.sector_type.is_door() || sector.sector_type.is_jump())
                        || engine.group_move_door_at(destination).is_some();
                    if has_semantic_overlay {
                        // Door and jump polygons intentionally overlap their
                        // underlying motion areas. A motion-only override
                        // would erase the Original's selected overlay and
                        // change route construction (notably MoveToLine's
                        // nearest jump-line point). Re-enter the ordinary
                        // spatial selection with the recorded point and the
                        // parity-matched live PC reference instead.
                        None
                    } else {
                        let candidates = engine
                            .fast_grid()
                            .level
                            .sectors
                            .iter()
                            .filter(|sector| {
                                sector.layer == goal_layer
                                    && sector.sector_type.is_motion()
                                    && sector.sector_type.is_area()
                                    && sector.contains_point(destination)
                            })
                            .collect::<Vec<_>>();
                        match candidates.as_slice() {
                            [sector] => Some((sector.sector_number, goal_layer)),
                            [] => {
                                match containing.as_slice() {
                                    [sector]
                                        if sector.sector_type.is_door()
                                            || sector.sector_type.is_jump() =>
                                    {
                                        // Preserve semantic overlay selection.
                                        // Re-running the ordinary spatial lookup
                                        // lets perform_group_move use its canonical
                                        // door/jump routing instead of collapsing
                                        // the hit to an underlying motion area.
                                        None
                                    }
                                    _ => panic!(
                                        "group-move raw sector index {raw_index} has no \
                                     motion area at {destination:?} on layer {goal_layer}; containing \
                                     semantic sectors: {:?}",
                                        containing
                                            .iter()
                                            .map(|sector| (
                                                sector.sector_number,
                                                sector.sector_type,
                                                sector.underlying_sector,
                                                sector.door_index,
                                            ))
                                            .collect::<Vec<_>>()
                                    ),
                                }
                            }
                            many => panic!(
                                "group-move raw sector index {raw_index} is ambiguous \
                             at {destination:?} on layer {goal_layer}: {:?}",
                                many.iter()
                                    .map(|sector| sector.sector_number)
                                    .collect::<Vec<_>>(),
                            ),
                        }
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
                }
            }
            Self::LaunchInteraction {
                actor,
                target,
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
                original_command_name,
            } => PlayerCommand::LaunchSelfAbility {
                actor: entity_map.translate(actor),
                command: command_from_stable_name(&original_command_name),
            },
            Self::LaunchGroundTarget {
                actor,
                target,
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
                original_command_name,
                with_seek,
            } => PlayerCommand::SwordStrikeCmd {
                actor: entity_map.translate(actor),
                target: entity_map.translate(target),
                command: command_from_stable_name(&original_command_name),
                with_seek,
            },
            Self::SelectPc { pc, append } => PlayerCommand::SelectPc {
                pc_id: entity_map.translate(pc),
                append,
            },
            Self::UnselectAllPcs => PlayerCommand::UnselectAllPcs,
            Self::StopPc { pc } => PlayerCommand::StopPc {
                pc_id: entity_map.translate(pc),
            },
            Self::SelectAction { pc, action } => PlayerCommand::SelectResolvedAction {
                pc_id: entity_map.translate(pc),
                action: action.into(),
            },
            Self::CancelAction { pc } => match pc {
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
            } => PlayerCommand::DropAleAt {
                actor: entity_map.translate(actor),
                target_pos: target.into(),
                running,
            },
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
        })
    }
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

#[derive(Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
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

#[derive(Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
struct TraceActor {
    action_state: u32,
    animation: u32,
    command: u16,
    command_name: String,
    motion_state: u32,
    wait_time: u32,
}

#[derive(Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
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
}

#[derive(Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
struct TraceElementPc {
    ammo: TraceElementAmmo,
}

#[derive(Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
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

#[derive(Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
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
}

#[derive(Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
struct TraceDetection {
    suspects: Vec<u16>,
    maximal_suspect: u16,
    maximal_visibility: u32,
    view_status: u8,
    alert_status: u32,
    detectables: Vec<TraceDetectable>,
}

#[derive(Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
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

#[derive(Clone, Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
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

#[derive(Clone, Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
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

#[derive(Clone, Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
struct TraceSightObstacleTypes {
    solid: bool,
    opaque: bool,
    projection_area: bool,
    mouse: bool,
    shield: bool,
    show_shadow_polygon: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
struct TraceSightObstacleBox {
    min: TracePoint,
    max: TracePoint,
}

#[derive(Clone, Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
struct TraceSightObstaclePoint {
    x: TraceFloat,
    y: TraceFloat,
    z_top: TraceFloat,
    z_bottom: TraceFloat,
}

#[derive(Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
struct TraceResolvedExclamation {
    actor: TraceEntityId,
    identifier: u32,
    exclamation_id: u16,
    selected_variant: i32,
    selected_entry: Option<u32>,
    duration_frames: u32,
}

#[derive(Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
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
    resolved_exclamations: Vec<TraceResolvedExclamation>,
}

#[derive(Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
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

/// Cache-safe recursive JSON used for high-volume authoritative snapshots.
///
/// `serde_json::Value` deliberately has no native `bincode::Encode`
/// implementation. This equivalent tree keeps schema-13 frame parsing strict
/// without making the native trace cache serialize a JSON string per frame.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, bincode::Decode, bincode::Encode)]
#[serde(untagged)]
enum TraceJsonValue {
    Null(()),
    Bool(bool),
    Unsigned(u64),
    Signed(i64),
    String(String),
    Array(Vec<TraceJsonValue>),
    Object(BTreeMap<String, TraceJsonValue>),
}

impl TraceJsonValue {
    fn to_json(&self) -> serde_json::Value {
        match self {
            Self::Null(()) => serde_json::Value::Null,
            Self::Bool(value) => (*value).into(),
            Self::Unsigned(value) => (*value).into(),
            Self::Signed(value) => (*value).into(),
            Self::String(value) => value.clone().into(),
            Self::Array(values) => values.iter().map(Self::to_json).collect(),
            Self::Object(values) => values
                .iter()
                .map(|(key, value)| (key.clone(), value.to_json()))
                .collect(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
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

fn validate_trace_frame_envelope(schema: u32, frame: &TraceFrame) {
    match schema {
        12 | 14 => assert!(
            frame.campaign.is_none() && frame.engine_state.is_none(),
            "schema-{schema} frame unexpectedly contains schema-13 authoritative state"
        ),
        13 => assert!(
            frame.campaign.is_some() && frame.engine_state.is_some(),
            "schema-13 frame is missing campaign or engine_state"
        ),
        _ => unreachable!("trace schema was validated before frame parsing"),
    }
}

const TRACE_CACHE_VERSION: u32 = 53;
const TRACE_CACHE_SUFFIX: &str = ".parity-cache-v53.native-bincode.zst";
// Full-session JSONL recordings are compressed as a single zstd frame. Some
// encoders select a frame window from the total uncompressed size, so long
// recordings legitimately exceed zstd's conservative 128 MiB decoder default.
// Keep the reader bounded at zstd's platform maximum while accepting those
// valid trace frames.
const TRACE_ZSTD_WINDOW_LOG_MAX: u32 = if usize::BITS >= 64 { 31 } else { 30 };
const TRACE_CACHE_ZSTD_LEVEL: i32 = 0;

#[derive(Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
struct BinaryTraceHeader {
    version: u32,
    source_fingerprint: String,
    trace: TraceHeader,
    rng_prefix: TraceRngPrefix,
}

#[derive(Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
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
    rust_movement_steps: Vec<robin_engine::movement_diagnostics::ParityMovementStep>,
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
    let trace_path = options
        .trace_path
        .canonicalize()
        .unwrap_or_else(|error| panic!("canonicalize parity trace: {error}"));
    let output_dir = options
        .frame_zero_screenshot_dir
        .expect("frame-zero capture lost its output directory");
    let output_dir = if output_dir.is_absolute() {
        output_dir
    } else {
        invocation_dir.join(output_dir)
    };
    let output_path = frame_zero_screenshot_path(&output_dir, &trace_path);

    let cache_path = ensure_binary_trace_cache(&trace_path);
    let header = read_binary_trace_header(&cache_path).trace;
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
    let scan_all = options.scan_all;
    let no_auto_dump = options.no_auto_dump;
    let trace_path = options.trace_path;
    let http_server = options.http_server;
    let mut manual_pause = options.start_paused;
    let trace_path = trace_path
        .canonicalize()
        .unwrap_or_else(|e| panic!("canonicalize {}: {e}", trace_path.display()));
    let cache_path = ensure_binary_trace_cache(&trace_path);
    let mut dump = options.dump.map(|options| {
        let file = File::create(&options.path)
            .unwrap_or_else(|e| panic!("create diagnostic dump {}: {e}", options.path.display()));
        (options, BufWriter::new(file))
    });
    let cached_header = read_binary_trace_header(&cache_path);
    let header = cached_header.trace;
    validate_trace_header(&header);
    validate_trace_start(
        header.start_state,
        header.session_index,
        header.initial_frame,
    );
    let initial_save = decode_and_validate_initial_save(&header);
    let all_rng_draws = read_all_rng_draws(&cache_path);

    if let Ok(dir) = std::env::var("ROBINHOOD_DATA_DIR") {
        std::env::set_current_dir(&dir).expect("chdir to ROBINHOOD_DATA_DIR");
    }
    robin_rs::main_entry::register_language_data_paths_for_tool();

    let mut records = BinaryTraceReader::open(&cache_path);
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
        all_rng_draws.len() >= prefix_end,
        "RNG pre-scan is shorter than prefix"
    );
    let rewind_loaded_save_rng =
        header.start_state == TraceStartState::LoadedSave && prefix_end == 0;
    let (mut engine, assets, mut host, background, mission_scb, menu_text) =
        initialize_engine(&header, all_rng_draws.clone());
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
    if rewind_loaded_save_rng {
        let setup_draws = engine
            .original_rng_replay_cursor()
            .expect("loaded-save reconstruction lost Original RNG replay");
        engine.replace_original_rng_replay(all_rng_draws);
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

    let mut line_index = 0_usize;
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
        assert_eq!(frame.frame_before, header.initial_frame + line_index as u64);
        assert_eq!(frame.frame_after, frame.frame_before + 1);
        line_index += 1;
        if http_server.is_some() {
            loop {
                drain_headless_http(
                    &mut engine,
                    &mut display,
                    &assets,
                    &mut input,
                    &mut selected_view_element,
                    &mut manual_pause,
                    &mut active_http_step,
                );
                if !manual_pause || active_http_step.is_some() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        let rng_start = gameplay_rng_index;
        let rng_end = rng_start + frame.rng_draws.gameplay_draw_count();
        gameplay_rng_index = rng_end;
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
        for completion in frame.director_completions.drain(..) {
            engine
                .apply_external_director_completion(completion, &mut display, &assets)
                .unwrap_or_else(|error| {
                    panic!(
                        "apply original director completion before frame {}: {error}",
                        frame.frame_before
                    )
                });
        }
        for command in frame.commands.drain(..) {
            if debug_stage_timing {
                eprintln!(
                    "parity stage: converting command before frame {}: {command:?}",
                    frame.frame_after
                );
            }
            if let Some(command) = command.into_player_command(map, &engine) {
                if debug_stage_timing {
                    eprintln!(
                        "parity stage: applying command before frame {}: {command:?}",
                        frame.frame_after
                    );
                }
                engine.apply_command(&mut display, &mut input, &assets, &command);
                if debug_stage_timing {
                    eprintln!(
                        "parity stage: applied command before frame {}: {command:?}",
                        frame.frame_after
                    );
                }
            }
        }
        // Original's Sound Hourglass ran after the preceding engine frame.
        // Apply its ordered, concrete Pass-1 speech resolutions now, before
        // the next simulation body begins. Audio RNG remains diagnostic only.
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
        engine.queue_resolved_exclamations(resolutions);
        if debug_stage_timing {
            eprintln!(
                "parity stage: entering Rust frame {} -> {}",
                frame.frame_before, frame.frame_after
            );
        }
        print_debug_element("before", &engine, &frame);
        robin_engine::movement_diagnostics::begin_parity_movement_capture();
        let tick_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            engine.perform_hourglass_with_body_gate(
                &mut display,
                &assets,
                &mut dev,
                frame.simulation_body_ran,
            )
        }));
        let actual_visibility_queries =
            robin_engine::sight_obstacle::take_parity_visibility_capture();
        // Restart immediately so post-frame work and the next frame's
        // director/input/sound prefix are attributed to that next frame,
        // matching the Original recorder's frame envelope.
        robin_engine::sight_obstacle::begin_parity_visibility_capture();
        let actual_movement_steps =
            robin_engine::movement_diagnostics::take_parity_movement_capture();
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
        if debug_stage_timing {
            eprintln!(
                "parity stage: compared Rust frame {} ({} differences)",
                frame.frame_after,
                differences.len()
            );
        }
        let rust_rng_sites = engine
            .original_rng_replay_sites(rng_start..actual_rng_end)
            .expect("original RNG site history unexpectedly disabled");
        let rust_rng_diagnostics = engine
            .original_rng_replay_diagnostics(rng_start..actual_rng_end)
            .expect("original RNG diagnostics unexpectedly disabled");
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
                &actual_movement_steps,
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
                    rust_movement_steps: actual_movement_steps.clone(),
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
        // Preserve the complete divergent frame in --dump-jsonl before
        // stopping on an RNG cursor mismatch. RNG ordering failures are often
        // precisely where the broad engine snapshot is most useful.
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
                let _ = engine.perform_post_initialize(&mut display, &assets);
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
        let _ = engine.perform_post_initialize(&mut display, &assets);
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

    match terminator {
        BinaryTraceRecord::End {
            rng_suffix: Some(_),
            final_frame: Some(final_frame),
            frame_count: Some(frame_count),
        } => {
            assert_eq!(
                frame_count,
                u64::try_from(line_index).expect("parity frame count exceeds u64"),
                "parity terminator frame_count disagrees with the frame stream"
            );
            assert_eq!(
                final_frame,
                header.initial_frame + frame_count,
                "parity terminator final_frame disagrees with its initial frame and frame count"
            );
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
            panic!("parity trace cache contains a partially populated terminator")
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
        [--dump-entity KIND:INDEX]...] TRACE.jsonl[.zst]";

    let mut args = std::env::args_os().skip(1);
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
    Options {
        scan_all,
        no_auto_dump,
        visual,
        trace_path,
        http_server,
        start_paused,
        frame_zero_screenshot_dir,
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
) {
    robin_rs::http_server::drain_global_headless(
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
}

fn serve_halted_http(
    engine: &mut Engine,
    display: &mut HostDisplayState,
    assets: &LevelAssets,
    input: &mut InputState,
    selected_view_element: &mut Option<EntityId>,
) -> ! {
    loop {
        robin_rs::http_server::drain_global_headless(
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
    rust_movement_steps: &[robin_engine::movement_diagnostics::ParityMovementStep],
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
        rust_movement_steps,
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
    rust_movement_steps: &[robin_engine::movement_diagnostics::ParityMovementStep],
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
        "rust_movement_steps": rust_movement_steps,
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
            &frame.rust_movement_steps,
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

fn trace_source_fingerprint(trace_path: &std::path::Path) -> String {
    let metadata = std::fs::metadata(trace_path)
        .unwrap_or_else(|error| panic!("stat parity trace {}: {error}", trace_path.display()));
    let modified = metadata
        .modified()
        .expect("parity trace modification time is unavailable")
        .duration_since(std::time::UNIX_EPOCH)
        .expect("parity trace modification time predates Unix epoch")
        .as_nanos();
    format!(
        "parity-cache-v{TRACE_CACHE_VERSION}:length={}:modified={modified}",
        metadata.len()
    )
}

fn binary_trace_cache_path(trace_path: &std::path::Path) -> PathBuf {
    let mut cache_name = trace_path.as_os_str().to_owned();
    cache_name.push(TRACE_CACHE_SUFFIX);
    PathBuf::from(cache_name)
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

fn ensure_binary_trace_cache(trace_path: &std::path::Path) -> PathBuf {
    let cache_path = binary_trace_cache_path(trace_path);
    let fingerprint = trace_source_fingerprint(trace_path);
    match try_read_binary_trace_header(&cache_path) {
        Ok(header)
            if header.version == TRACE_CACHE_VERSION
                && header.source_fingerprint == fingerprint =>
        {
            eprintln!("loaded parity trace cache {}", cache_path.display());
            return cache_path;
        }
        Ok(header) => eprintln!(
            "rebuilding stale parity trace cache {} (version {}, fingerprint {:?})",
            cache_path.display(),
            header.version,
            header.source_fingerprint
        ),
        Err(error) if cache_path.exists() => eprintln!(
            "rebuilding unreadable parity trace cache {}: {error}",
            cache_path.display()
        ),
        Err(_) => eprintln!(
            "building parity trace cache {} with bincode + zstd level {TRACE_CACHE_ZSTD_LEVEL}",
            cache_path.display()
        ),
    }

    let mut lines = open_jsonl_trace(trace_path).lines();
    let trace: TraceHeader = serde_json::from_str(
        &lines
            .next()
            .expect("parity trace has no header")
            .expect("read parity trace header"),
    )
    .expect("parse parity trace header");
    validate_trace_header(&trace);
    let rng_prefix: TraceRngPrefix = serde_json::from_str(
        &lines
            .next()
            .expect("parity trace has no RNG prefix")
            .expect("read parity RNG prefix"),
    )
    .expect("parse parity RNG prefix");
    assert_eq!(
        rng_prefix.r#type, "rng_prefix",
        "invalid RNG prefix record type"
    );
    rng_prefix.draws.validate();
    let header = BinaryTraceHeader {
        version: TRACE_CACHE_VERSION,
        source_fingerprint: fingerprint,
        trace,
        rng_prefix,
    };

    let parent = cache_path
        .parent()
        .expect("parity trace cache path has no parent");
    let mut temporary = tempfile::NamedTempFile::new_in(parent).unwrap_or_else(|error| {
        panic!(
            "create temporary parity trace cache beside {}: {error}",
            cache_path.display()
        )
    });
    let started = std::time::Instant::now();
    let mut frame_count = 0_u64;
    {
        let mut encoder = zstd::stream::write::Encoder::new(
            BufWriter::new(temporary.as_file_mut()),
            TRACE_CACHE_ZSTD_LEVEL,
        )
        .unwrap_or_else(|error| panic!("start parity trace cache compression: {error}"));
        write_binary_record(&mut encoder, &header, "parity trace cache header");

        let mut wrote_end = false;
        for (record_index, line) in lines.enumerate() {
            let line_number = record_index + 3;
            let line = line.unwrap_or_else(|error| {
                panic!("read parity trace record on line {line_number}: {error}")
            });
            if let Some(frame) = parse_trace_frame(&line, line_number) {
                validate_trace_frame_envelope(header.trace.schema, &frame);
                write_binary_record(
                    &mut encoder,
                    &BinaryTraceRecord::Frame(frame),
                    "parity trace frame",
                );
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
                assert_eq!(
                    suffix.frame_count, frame_count,
                    "parity terminator frame_count disagrees with the JSONL frame stream"
                );
                assert_eq!(
                    suffix.final_frame,
                    header.trace.initial_frame + frame_count,
                    "parity terminator final_frame disagrees with its initial frame and frame count"
                );
                write_binary_record(
                    &mut encoder,
                    &BinaryTraceRecord::End {
                        rng_suffix: Some(suffix.draws),
                        final_frame: Some(suffix.final_frame),
                        frame_count: Some(suffix.frame_count),
                    },
                    "parity trace cache terminator",
                );
                wrote_end = true;
                break;
            }
        }
        if !wrote_end {
            write_binary_record(
                &mut encoder,
                &BinaryTraceRecord::End {
                    rng_suffix: None,
                    final_frame: None,
                    frame_count: None,
                },
                "parity trace cache terminator",
            );
        }
        let mut writer = encoder
            .finish()
            .unwrap_or_else(|error| panic!("finish parity trace cache compression: {error}"));
        writer
            .flush()
            .unwrap_or_else(|error| panic!("flush parity trace cache: {error}"));
    }
    temporary
        .as_file()
        .sync_all()
        .unwrap_or_else(|error| panic!("sync parity trace cache: {error}"));
    temporary.persist(&cache_path).unwrap_or_else(|error| {
        panic!(
            "persist parity trace cache {}: {}",
            cache_path.display(),
            error.error
        )
    });
    let compressed_bytes = std::fs::metadata(&cache_path)
        .expect("stat completed parity trace cache")
        .len();
    eprintln!(
        "cached {frame_count} frames in {} ({:.1} MiB, {:.1}s)",
        cache_path.display(),
        compressed_bytes as f64 / (1024.0 * 1024.0),
        started.elapsed().as_secs_f64()
    );
    cache_path
}

impl BinaryTraceReader {
    fn open(path: &std::path::Path) -> Self {
        let file = File::open(path)
            .unwrap_or_else(|error| panic!("open parity trace cache {}: {error}", path.display()));
        let decoder = zstd::stream::read::Decoder::new(file).unwrap_or_else(|error| {
            panic!(
                "start parity trace cache decompression {}: {error}",
                path.display()
            )
        });
        Self {
            path: path.to_owned(),
            reader: Box::new(decoder),
        }
    }

    fn read_header(&mut self) -> BinaryTraceHeader {
        read_binary_record(&mut self.reader, "parity trace cache header").unwrap_or_else(|error| {
            panic!(
                "read parity trace cache header {}: {error}",
                self.path.display()
            )
        })
    }

    fn read_record(&mut self) -> BinaryTraceRecord {
        read_binary_record(&mut self.reader, "parity trace cache record").unwrap_or_else(|error| {
            panic!(
                "read parity trace cache record {}: {error}",
                self.path.display()
            )
        })
    }
}

fn try_read_binary_trace_header(path: &std::path::Path) -> Result<BinaryTraceHeader, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    let mut decoder = zstd::stream::read::Decoder::new(file).map_err(|error| error.to_string())?;
    read_binary_record(&mut decoder, "parity trace cache header")
}

fn read_binary_trace_header(path: &std::path::Path) -> BinaryTraceHeader {
    try_read_binary_trace_header(path).unwrap_or_else(|error| {
        panic!(
            "read parity trace cache header {} after conversion: {error}",
            path.display()
        )
    })
}

fn write_binary_record<T: bincode::Encode>(writer: &mut impl Write, value: &T, label: &str) {
    let encoded = bincode::encode_to_vec(value, bincode::config::standard())
        .unwrap_or_else(|error| panic!("encode {label}: {error}"));
    let length = u64::try_from(encoded.len()).expect("binary record length exceeds u64");
    writer
        .write_all(&length.to_le_bytes())
        .unwrap_or_else(|error| panic!("write {label} length: {error}"));
    writer
        .write_all(&encoded)
        .unwrap_or_else(|error| panic!("write {label}: {error}"));
}

fn read_binary_record<T: bincode::Decode<()>>(
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
    let (value, consumed) = bincode::decode_from_slice(&encoded, bincode::config::standard())
        .map_err(|error| format!("decode {label}: {error}"))?;
    if consumed != encoded.len() {
        return Err(format!(
            "decode {label}: consumed {consumed} of {} bytes",
            encoded.len()
        ));
    }
    Ok(value)
}

fn read_all_rng_draws(cache_path: &std::path::Path) -> Vec<u32> {
    let mut reader = BinaryTraceReader::open(cache_path);
    let header = reader.read_header();
    let mut result = Vec::new();
    let mut original_index = 0_usize;
    append_simulation_rng_draws(&mut result, &mut original_index, &header.rng_prefix.draws);
    loop {
        match reader.read_record() {
            BinaryTraceRecord::Frame(frame) => {
                append_simulation_rng_draws(&mut result, &mut original_index, &frame.rng_draws);
            }
            BinaryTraceRecord::End { rng_suffix, .. } => {
                if let Some(batch) = rng_suffix {
                    append_simulation_rng_draws(&mut result, &mut original_index, &batch);
                }
                break;
            }
        }
    }
    eprintln!(
        "loaded {} simulation RNG draws from {}",
        result.len(),
        cache_path.display()
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
        let mut reverse_sectors = BTreeMap::new();
        for (original, runtime) in retained.position_sector_numbers.iter().enumerate() {
            let Some(runtime) = runtime else {
                continue;
            };
            let original = u16::try_from(original)
                .expect("Original sparse sector slot exceeds its u16 identity domain");
            let runtime =
                u16::try_from(*runtime).expect("Rust canonical sector number is negative");
            assert!(
                sectors.insert(original, runtime).is_none(),
                "Original sparse sector slot {original} was mapped twice"
            );
            if let Some(previous) = reverse_sectors.insert(runtime, original) {
                panic!(
                    "Rust canonical sector {runtime} maps to both Original sparse slots \
                     {previous} and {original}"
                );
            }
        }
        Self {
            entities: result,
            entities_by_creation_order,
            sectors,
        }
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

    fn sectors_equivalent(&self, original: u16, rust: u16) -> bool {
        original == rust || self.sectors.get(&original) == Some(&rust)
    }

    fn translate_sector(&self, original: u16) -> u16 {
        self.sectors.get(&original).copied().unwrap_or(original)
    }
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
        "{label} frame {} {:?} order={:?} installed={:?} last_processed_order={} actor_motion={:?} sprite_motion={:?} last_action={:?} row={} frame={}/{} command={:?} execute_init={} last_execute_order={:?}",
        frame.frame_after,
        id,
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
        let Some(actual) = engine.get_entity(id) else {
            differences.push(format!("{id:?}: missing in Rust entity table"));
            continue;
        };
        let element = actual.element_data();
        assert_eq!(
            expected.ai.is_some(),
            expected.detection.is_some(),
            "Original NPC trace state must contain both ai and detection payloads for {:?}",
            expected.entity_id
        );
        compare(
            &mut differences,
            id,
            "creation_order",
            expected.creation_order,
            engine.original_creation_order(id),
        );
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
        compare_point(
            &mut differences,
            id,
            "old_position_map",
            expected.old_position_map,
            pi.old_map_position(),
        );
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
        compare_float(
            &mut differences,
            id,
            "old_elevation",
            expected.old_elevation,
            pi.old_elevation(),
        );
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
        let movement_map = element.position_map() - pi.old_map_position();
        compare_point(
            &mut differences,
            id,
            "movement_map",
            expected.movement_map,
            MapPoint::new(movement_map.x, movement_map.y),
        );
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
                &format!("{id:?}.runtime"),
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
            compare(
                &mut differences,
                id,
                "actor.motion_state",
                expected_actor.motion_state,
                actual_actor.continuation.motion_state as u32,
            );
            compare(
                &mut differences,
                id,
                "actor.command",
                command_from_stable_name(&expected_actor.command_name),
                engine.actor_command(id),
            );
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
                    .clone();
                compare(&mut differences, id, "human.opponents", expected, actual);
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
            let actual_macro_cursor = (actual_ai.patrol_path.is_some()
                && !actual_ai.macro_command.is_empty())
            .then(|| {
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

            let actual_detectables = npc
                .detectable_lists
                .iter()
                .flat_map(|list| list.iter())
                .collect::<Vec<_>>();
            compare(
                &mut differences,
                id,
                "detection.detectables.length",
                expected_detection.detectables.len(),
                actual_detectables.len(),
            );
            for (detectable_index, (expected_detectable, actual_detectable)) in expected_detection
                .detectables
                .iter()
                .zip(actual_detectables)
                .enumerate()
            {
                let field =
                    |name: &str| format!("detection.detectables[{detectable_index}].{name}");
                compare(
                    &mut differences,
                    id,
                    &field("type"),
                    expected_detectable.detectable_type,
                    detectable_type_ordinal(actual_detectable.detectable_type),
                );
                compare(
                    &mut differences,
                    id,
                    &field("target"),
                    entity_map.translate(expected_detectable.target),
                    actual_detectable.element.unwrap_or_else(|| {
                        panic!("NPC {id:?} detectable {detectable_index} has no target element")
                    }),
                );
                compare(
                    &mut differences,
                    id,
                    &field("seen_now"),
                    expected_detectable.seen_now,
                    actual_detectable.seen_now,
                );
                compare(
                    &mut differences,
                    id,
                    &field("seen_last_frame"),
                    expected_detectable.seen_last_frame,
                    actual_detectable.seen_last_frame,
                );
                compare(
                    &mut differences,
                    id,
                    &field("heard_last_frame"),
                    expected_detectable.heard_last_frame,
                    actual_detectable.heard_last_frame,
                );
                compare(
                    &mut differences,
                    id,
                    &field("shadow_seen_now"),
                    expected_detectable.shadow_seen_now,
                    actual_detectable.shadow_seen_now,
                );
                compare(
                    &mut differences,
                    id,
                    &field("shadow_seen_last_frame"),
                    expected_detectable.shadow_seen_last_frame,
                    actual_detectable.shadow_seen_last_frame,
                );
                compare_float(
                    &mut differences,
                    id,
                    &field("last_visibility"),
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
    compare_float(differences, id, &format!("{field}.x"), expected.x, actual.x);
    compare_float(differences, id, &format!("{field}.y"), expected.y, actual.y);
}

fn compare_point_with_absolute_tolerance(
    differences: &mut Vec<String>,
    id: EntityId,
    field: &str,
    expected: TracePoint,
    actual: MapPoint,
    absolute_tolerance: f32,
) {
    compare_float_with_absolute_tolerance(
        differences,
        id,
        &format!("{field}.x"),
        expected.x,
        actual.x,
        absolute_tolerance,
    );
    compare_float_with_absolute_tolerance(
        differences,
        id,
        &format!("{field}.y"),
        expected.y,
        actual.y,
        absolute_tolerance,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_cache_suffix_tracks_binary_header_version() {
        assert!(
            TRACE_CACHE_SUFFIX.contains(&format!("-v{TRACE_CACHE_VERSION}.")),
            "native parity-cache suffix {TRACE_CACHE_SUFFIX:?} does not identify header version {TRACE_CACHE_VERSION}"
        );
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
    #[should_panic(expected = "schemas 12, 13 and 14 are supported")]
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
    fn schema_twelve_and_thirteen_frame_envelopes_are_distinct() {
        let schema_twelve: TraceFrame = serde_json::from_value(minimal_frame_json()).unwrap();
        validate_trace_frame_envelope(12, &schema_twelve);
        assert!(
            std::panic::catch_unwind(|| validate_trace_frame_envelope(13, &schema_twelve)).is_err()
        );

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
        validate_trace_frame_envelope(13, &schema_thirteen);
        assert!(
            std::panic::catch_unwind(|| validate_trace_frame_envelope(12, &schema_thirteen))
                .is_err()
        );
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
            "original_action": 0
        }))
        .expect("parse Original global action cancellation");
        assert!(matches!(command, TraceCommand::CancelAction { pc: None }));
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
    fn native_bincode_cache_handles_heterogeneous_command_variants() {
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
        };

        map.refresh_trace_index(shifted_trace_id, 158);

        assert_eq!(map.translate(shifted_trace_id), rust_id);
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
