//! Replay a JSONL parity trace produced by the original C++ game.
//!
//! This intentionally accepts the neutral, resolved-command schema emitted by
//! `original-code/RHParity.cpp`, rather than the Rust-native replay schema.
//!
//! Usage:
//!   ROBINHOOD_DATA_DIR=datadirs/demo_leicester_linux \
//!     cargo run --example original_parity_replay -- \
//!       original-code/parity-traces/original-demo-baseline.jsonl

use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::PathBuf;
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
struct TraceHeader {
    mission: String,
    proto_level: String,
    rng_seed: u64,
    schema: u32,
    session_index: u32,
    start_state: TraceStartState,
    initial_frame: u64,
    synchronous_pathfinding: bool,
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
        matches!(schema, 10 | 11),
        "unsupported parity trace schema {schema}; complete resolved-input schema 10 or 11 is required"
    );
}

fn decode_and_validate_initial_save(header: &TraceHeader) -> Option<Vec<u8>> {
    match (
        header.schema,
        header.start_state,
        header.initial_save.as_ref(),
    ) {
        // Schema 10 remains temporarily supported as an oracle while the v48
        // RHSG body importer is developed. It reconstructs loaded saves from
        // campaign state and therefore cannot guarantee exact mid-mission state.
        (10, _, None) => None,
        (10, _, Some(_)) => {
            panic!("schema 10 must not contain the schema-11 initial_save envelope")
        }
        (11, TraceStartState::MissionStart, None) => None,
        (11, TraceStartState::MissionStart, Some(_)) => {
            panic!("schema-11 mission_start traces must not contain initial_save")
        }
        (11, TraceStartState::LoadedSave, None) => {
            panic!("schema-11 loaded_save traces require initial_save")
        }
        (11, TraceStartState::LoadedSave, Some(initial_save)) => {
            let mission_index = header
                .campaign
                .current_mission_index
                .expect("schema-11 loaded_save campaign has no current mission");
            let mission = header.campaign.missions.get(mission_index).unwrap_or_else(|| {
                panic!(
                    "schema-11 loaded_save current mission index {mission_index} is out of range"
                )
            });
            Some(
                initial_save
                    .decode_and_validate(mission.profile_id)
                    .unwrap_or_else(|error| panic!("invalid schema-11 initial_save: {error}")),
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
        // schema-10 campaign/config/RNG prefix and the ordinary mission
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
struct TraceRngBatch {
    first_index: usize,
    values: Vec<u32>,
    callsite_offsets: Vec<u32>,
    domains: Vec<TraceRngDomain>,
}

impl TraceRngBatch {
    fn gameplay_draw_count(&self) -> usize {
        assert_eq!(self.values.len(), self.callsite_offsets.len());
        if self.values.is_empty() {
            assert!(
                self.domains.is_empty(),
                "empty RNG batch unexpectedly contains draw domains"
            );
            return 0;
        }
        assert_eq!(
            self.values.len(),
            self.domains.len(),
            "RNG domain stream has a different length than its values"
        );
        self.domains
            .iter()
            .filter(|domain| **domain == TraceRngDomain::Simulation)
            .count()
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
struct TraceRngPrefix {
    #[allow(dead_code)]
    r#type: String,
    draws: TraceRngBatch,
}

#[derive(Debug, Deserialize, Serialize)]
struct TraceRngOnly {
    #[serde(default)]
    draws: Option<TraceRngBatch>,
    #[serde(default)]
    rng_draws: Option<TraceRngBatch>,
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
    active: bool,
    blipped: bool,
    unreachable: bool,
    posture: u32,
    position_map: TracePoint,
    position_goal_map: TracePoint,
    elevation: TraceFloat,
    layer: u16,
    sector: u16,
    direction: i16,
    direction_goal: i16,
    #[serde(default)]
    actor: Option<TraceActor>,
    #[serde(default)]
    human: Option<TraceHuman>,
    #[serde(default)]
    pc: Option<TraceElementPc>,
    #[serde(default)]
    ai: Option<TraceAi>,
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

#[derive(Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
struct TraceAi {
    state: u32,
    substate: u32,
}

#[derive(Debug, Deserialize, Serialize, bincode::Encode, bincode::Decode)]
struct TraceFrame {
    frame_before: u64,
    frame_after: u64,
    simulation_body_ran: bool,
    commands: Vec<TraceCommand>,
    director_completions: Vec<robin_engine::engine::DirectorCompletion>,
    selected_pcs: Vec<TraceEntityId>,
    elements: Vec<TraceElement>,
    rng_draws: TraceRngBatch,
    motion_line_changes: Vec<TraceMotionLineChange>,
    path_events: Vec<TracePathEvent>,
}

const TRACE_CACHE_VERSION: u32 = 4;
const TRACE_CACHE_SUFFIX: &str = ".parity-cache-v4.native-bincode.zst";
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
    End { rng_suffix: Option<TraceRngBatch> },
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
    path_events: Vec<TracePathEvent>,
    rng_start: usize,
    expected_rng_end: usize,
    actual_rng_end: usize,
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
    fn render(&mut self, engine: &Engine) -> bool {
        let _events = self.window.poll_events();
        if self.window.close_requested {
            return false;
        }

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
    tracing_subscriber::fmt::init();

    let options = parse_options();
    if options.visual {
        let exit = robin_rs::window::run_with_game(
            "Robin Hood — Original parity replay",
            1024,
            768,
            move |window| async move { run_replay(options, Some(window)) },
        )
        .unwrap_or_else(|error| panic!("start visual parity replay: {error}"));
        std::process::exit(exit);
    }
    std::process::exit(run_replay(options, None));
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
    validate_trace_schema(header.schema);
    validate_trace_start(
        header.start_state,
        header.session_index,
        header.initial_frame,
    );
    let _initial_save = decode_and_validate_initial_save(&header);
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
    let (mut engine, assets, host, background) = initialize_engine(&header, all_rng_draws.clone());
    if rewind_loaded_save_rng {
        let setup_draws = engine
            .original_rng_replay_cursor()
            .expect("loaded-save reconstruction lost Original RNG replay");
        engine.replace_original_rng_replay(all_rng_draws);
        eprintln!("rewound loaded-save RNG after {setup_draws} deterministic construction draws");
    }
    let mut motion_line_parity = MotionLineParity::build(&engine, &header.motion_grid);
    engine.set_external_director_completion_replay(true);
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
    let mut dev = DevState::new();
    let mut display = HostDisplayState::default();
    let mut input = InputState::default();
    let mut selected_view_element = None;
    let mut entity_map: Option<EntityMap> = None;
    let mut divergent_frames = 0_u64;
    let mut first_by_field = BTreeMap::<String, (u64, String)>::new();
    let mut gameplay_rng_index = prefix_end;
    let mut active_http_step = None;
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

    let mut line_index = 0_usize;
    loop {
        let mut frame = match records.read_record() {
            BinaryTraceRecord::Frame(frame) => frame,
            BinaryTraceRecord::End { .. } => break,
        };
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
        let map = entity_map.get_or_insert_with(|| EntityMap::build(&engine, &frame));
        map.refresh_trace_indices(&frame);
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
            if let Some(command) = command.into_player_command(map, &engine) {
                engine.apply_command(&mut display, &mut input, &assets, &command);
            }
        }
        let tick_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            engine.perform_hourglass_with_body_gate(
                &mut display,
                &assets,
                &mut dev,
                frame.simulation_body_ran,
            )
        }));
        if let Err(payload) = tick_result {
            eprintln!(
                "Rust simulation panicked while replaying original frame {} -> {}",
                frame.frame_before, frame.frame_after
            );
            std::panic::resume_unwind(payload);
        }
        map.extend_runtime_entities(&engine, &frame);
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

        map.refresh_building_sector_mapping(&engine, &frame);
        let mut differences =
            motion_line_parity.apply_changes_and_compare(&engine, &frame.motion_line_changes);
        differences.extend(compare_frame(&engine, &frame, map));
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
                    path_events: frame.path_events.clone(),
                    rng_start,
                    expected_rng_end: rng_end,
                    actual_rng_end,
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
            let sites = engine
                .original_rng_replay_sites(rng_start..actual_rng_end)
                .expect("original RNG site history unexpectedly disabled");
            panic!(
                "Rust consumed RNG draws {:?} at sites {sites:?} during original frame {}; original ended at draw {rng_end}",
                rng_start..actual_rng_end,
                frame.frame_before
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
            if !frame.path_events.is_empty() {
                eprintln!(
                    "  Original path events this frame: {}",
                    serde_json::to_string(&frame.path_events)
                        .expect("serialize Original path-event diagnostics")
                );
                // TODO(schema-9 path parity): compare these with an ordered
                // Rust path request/completion event stream once the engine
                // exposes one. Engine state alone cannot recover queued and
                // completed call boundaries without changing the engine.
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
        [--http-server PORT [--start-paused]] \
        [--dump-jsonl PATH [--dump-from FRAME] [--dump-through FRAME] \
        [--dump-entity KIND:INDEX]...] TRACE.jsonl";

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
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--scan-all") => scan_all = true,
            Some("--no-auto-dump") => no_auto_dump = true,
            Some("--visual") => visual = true,
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
    Options {
        scan_all,
        no_auto_dump,
        visual,
        trace_path,
        http_server,
        start_paused,
        dump: dump_path.map(|path| DumpOptions {
            path,
            from_frame: dump_from,
            through_frame: dump_through,
            entities: dump_entities,
        }),
    }
}

fn parse_trace_frame(line: &str, line_number: usize) -> Option<TraceFrame> {
    match serde_json::from_str(line) {
        Ok(frame) => Some(frame),
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
    differences: &[String],
) {
    let diagnostic_engine = engine.diagnostic_snapshot_without_original_rng_replay();
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
        resolved_commands,
        rng_start,
        expected_rng_end,
        actual_rng_end,
        differences,
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
    path_events: &[TracePathEvent],
    resolved_commands: serde_json::Value,
    rng_start: usize,
    expected_rng_end: usize,
    actual_rng_end: usize,
    differences: &[String],
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
    write_jsonl_record(
        writer,
        &serde_json::json!({
            "schema": "robin-parity-engine-dump.v1",
            "type": "frame",
            "frame_before": frame_before,
            "frame_after": frame_after,
            "input": {
                "resolved_commands": resolved_commands,
                "selected_pcs": selected_pcs,
            },
            "original_path_events": path_events,
            "rng": {
                "cursor_before": rng_start,
                "expected_cursor_after": expected_rng_end,
                "actual_cursor_after": actual_rng_end,
                "original_frame_draws": rng_draws,
                "engine_original_replay_stream_omitted": true,
            },
            "entity_mapping": mapped_entities,
            "parity_differences": differences,
            "engine": engine_value,
        }),
    );
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
    let temporary = tempfile::Builder::new()
        .prefix(&prefix)
        .suffix(".jsonl")
        .tempfile()
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
            &frame.path_events,
            frame.resolved_commands.clone(),
            frame.rng_start,
            frame.expected_rng_end,
            frame.actual_rng_end,
            &frame.differences,
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

    let file = File::open(trace_path)
        .unwrap_or_else(|error| panic!("open parity trace {}: {error}", trace_path.display()));
    let mut lines = BufReader::new(file).lines();
    let trace: TraceHeader = serde_json::from_str(
        &lines
            .next()
            .expect("parity trace has no header")
            .expect("read parity trace header"),
    )
    .expect("parse parity trace header");
    let rng_prefix: TraceRngPrefix = serde_json::from_str(
        &lines
            .next()
            .expect("parity trace has no RNG prefix")
            .expect("read parity RNG prefix"),
    )
    .expect("parse parity RNG prefix");
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
                write_binary_record(
                    &mut encoder,
                    &BinaryTraceRecord::End {
                        rng_suffix: suffix.draws.or(suffix.rng_draws),
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
                &BinaryTraceRecord::End { rng_suffix: None },
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
            BinaryTraceRecord::End { rng_suffix } => {
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
    assert_eq!(batch.values.len(), batch.callsite_offsets.len());
    *original_index += batch.values.len();
    if batch.values.is_empty() {
        assert!(
            batch.domains.is_empty(),
            "empty RNG batch unexpectedly contains draw domains"
        );
        return;
    }
    assert_eq!(
        batch.values.len(),
        batch.domains.len(),
        "RNG domain stream has a different length than its values"
    );
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
    difference
        .split_once(").")
        .and_then(|(_, tail)| tail.split_once(':'))
        .map_or("other", |(field, _)| field)
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
) {
    let mut pm = robin_engine::profiles::ProfileManager::new();
    let mut cpf = robin_engine::sbfile::SbFile::open(
        "Data/Configuration/profile.cpf",
        robin_engine::sbfile::SB_FILE_READ,
    )
    .expect("open profile.cpf");
    pm.load_all_legacy_cpf(&mut cpf).expect("parse profile.cpf");

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
        Arc::new(robin_engine::script_manager::ScriptProgram::from_scb(scb)),
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
    (engine, assets, host, background)
}

struct EntityMap {
    /// Per-frame Original array index to Rust entity. Original array indices
    /// can shift after a physical removal, so this is a view rather than the
    /// durable identity registry.
    entities: BTreeMap<TraceEntityId, EntityId>,
    /// Original's immutable per-engine construction serial is the durable
    /// identity anchor for refreshing the per-frame raw-index view.
    entities_by_creation_order: BTreeMap<u32, EntityId>,
    /// Original and Rust allocate synthetic building-sector numbers from
    /// different registries. Pair them by their shared first-gate position
    /// and inactive occupant sets rather than treating the raw numbers as
    /// gameplay identity.
    building_sectors: BTreeMap<u16, u16>,
}

impl EntityMap {
    /// Build a deterministic one-to-one correspondence without consulting raw
    /// table indices. Entity kind and initial map position are the primary
    /// labels. Creation order and Rust table order only break ties between
    /// otherwise indistinguishable colocated entities.
    fn build(engine: &Engine, frame: &TraceFrame) -> Self {
        let mut originals: Vec<_> = frame.elements.iter().collect();
        originals.sort_by_key(|element| (element.creation_order, element.entity_id));
        let rust_entities: Vec<_> = engine
            .entities_with_ids_iter()
            .map(|(id, entity)| {
                let element = entity.element_data();
                (
                    id,
                    entity.entity_id_kind(),
                    element.position_map(),
                    element.active,
                    element.posture as u32,
                )
            })
            .collect();
        assert_eq!(
            originals.len(),
            rust_entities.len(),
            "entity tables have different cardinality"
        );

        let mut used = BTreeSet::new();
        let mut result = BTreeMap::new();
        for original in originals {
            let expected_kind = EntityIdKind::from(original.entity_id.kind);
            let expected_position: MapPoint = original.position_map.into();
            let mut candidates: Vec<_> = rust_entities
                .iter()
                .filter(|(id, kind, _, _, _)| *kind == expected_kind && !used.contains(id))
                .collect();
            candidates.sort_by_key(|(id, _, position, active, posture)| {
                let exact = position.x.to_bits() == expected_position.x.to_bits()
                    && position.y.to_bits() == expected_position.y.to_bits();
                let dx = f64::from(position.x) - f64::from(expected_position.x);
                let dy = f64::from(position.y) - f64::from(expected_position.y);
                (
                    (!exact) as u8,
                    exact && *active != original.active,
                    exact && *posture != original.posture,
                    (dx * dx + dy * dy).to_bits(),
                    id.index(),
                )
            });
            let (rust_id, _, _, _, _) = candidates.first().unwrap_or_else(|| {
                panic!(
                    "no unused Rust {:?} for original {:?}",
                    expected_kind, original.entity_id
                )
            });
            used.insert(*rust_id);
            result.insert(original.entity_id, *rust_id);
        }
        let mut building_sectors = BTreeMap::new();
        let mut reverse_building_sectors = BTreeMap::new();
        for original in frame
            .elements
            .iter()
            .filter(|element| element.actor.is_some() && !element.active)
        {
            let rust_id = result[&original.entity_id];
            let actual = engine
                .get_entity(rust_id)
                .unwrap_or_else(|| panic!("mapped entity {rust_id:?} vanished"));
            let actual_element = actual.element_data();
            let actual_sector = actual_element.sector().map(|sector| sector.get());
            let expected_position: MapPoint = original.position_map.into();
            let actual_position = actual_element.position_map();
            let same_position = expected_position.x.to_bits() == actual_position.x.to_bits()
                && expected_position.y.to_bits() == actual_position.y.to_bits();
            let Some(actual_sector) = actual_sector else {
                continue;
            };
            if same_position && !actual_element.active && original.sector != actual_sector {
                if let Some(previous) = building_sectors.insert(original.sector, actual_sector) {
                    assert_eq!(
                        previous, actual_sector,
                        "original building sector {} maps to multiple Rust sectors",
                        original.sector
                    );
                }
                if let Some(previous) =
                    reverse_building_sectors.insert(actual_sector, original.sector)
                {
                    assert_eq!(
                        previous, original.sector,
                        "Rust building sector {actual_sector} maps to multiple original sectors"
                    );
                }
            }
        }
        let entities_by_creation_order = frame
            .elements
            .iter()
            .map(|element| {
                (
                    element.creation_order,
                    *result
                        .get(&element.entity_id)
                        .expect("startup entity has an isomorphic mapping"),
                )
            })
            .collect();
        Self {
            entities: result,
            entities_by_creation_order,
            building_sectors,
        }
    }

    fn refresh_trace_indices(&mut self, frame: &TraceFrame) {
        for original in &frame.elements {
            self.refresh_trace_index(original.entity_id, original.creation_order);
        }
    }

    /// Learn synthetic Original↔Rust building-sector identities when an actor
    /// first becomes a hidden building occupant. Neither engine's raw sector
    /// allocation is stable across implementations, and some buildings have
    /// no occupant at mission start from which `build` could infer the pair.
    fn refresh_building_sector_mapping(&mut self, engine: &Engine, frame: &TraceFrame) {
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
            if let Some(previous) = self.building_sectors.get(&original.sector) {
                assert_eq!(
                    *previous, actual_sector,
                    "original building sector {} maps to multiple Rust sectors",
                    original.sector
                );
                continue;
            }
            if let Some((&other_original, _)) = self
                .building_sectors
                .iter()
                .find(|(_, rust_sector)| **rust_sector == actual_sector)
            {
                panic!(
                    "Rust building sector {actual_sector} maps to both Original sectors \
                     {other_original} and {}",
                    original.sector
                );
            }
            self.building_sectors.insert(original.sector, actual_sector);
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

    /// Extend the mission-start bijection for entities created while replaying
    /// the mission. Raw table IDs are allocation details in both engines, so
    /// pair new entities by the same logical labels as startup mapping and use
    /// each engine's creation order only to break otherwise indistinguishable
    /// ties.
    fn extend_runtime_entities(&mut self, engine: &Engine, frame: &TraceFrame) {
        self.refresh_trace_indices(frame);
        let mut originals: Vec<_> = frame
            .elements
            .iter()
            .filter(|element| {
                !self
                    .entities_by_creation_order
                    .contains_key(&element.creation_order)
            })
            .collect();
        originals.sort_by_key(|element| (element.creation_order, element.entity_id));

        let mut used: BTreeSet<_> = self.entities_by_creation_order.values().copied().collect();
        let rust_entities: Vec<_> = engine
            .entities_with_ids_iter()
            .filter(|(id, _)| !used.contains(id))
            .map(|(id, entity)| {
                let element = entity.element_data();
                (
                    id,
                    entity.entity_id_kind(),
                    element.position_map(),
                    element.active,
                    element.posture as u32,
                )
            })
            .collect();
        assert_eq!(
            originals.len(),
            rust_entities.len(),
            "runtime entity tables gained different numbers of unmapped entities"
        );

        for original in originals {
            let expected_kind = EntityIdKind::from(original.entity_id.kind);
            let expected_position: MapPoint = original.position_map.into();
            let mut candidates: Vec<_> = rust_entities
                .iter()
                .filter(|(id, kind, _, _, _)| *kind == expected_kind && !used.contains(id))
                .collect();
            candidates.sort_by_key(|(id, _, position, active, posture)| {
                let exact = position.x.to_bits() == expected_position.x.to_bits()
                    && position.y.to_bits() == expected_position.y.to_bits();
                let dx = f64::from(position.x) - f64::from(expected_position.x);
                let dy = f64::from(position.y) - f64::from(expected_position.y);
                (
                    (!exact) as u8,
                    exact && *active != original.active,
                    exact && *posture != original.posture,
                    (dx * dx + dy * dy).to_bits(),
                    id.index(),
                )
            });
            let (rust_id, _, _, _, _) = candidates.first().unwrap_or_else(|| {
                panic!(
                    "no unused runtime Rust {:?} for original {:?} created at order {}",
                    expected_kind, original.entity_id, original.creation_order
                )
            });
            used.insert(*rust_id);
            self.entities.insert(original.entity_id, *rust_id);
            assert!(
                self.entities_by_creation_order
                    .insert(original.creation_order, *rust_id)
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
        original == rust || self.building_sectors.get(&original) == Some(&rust)
    }
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

fn compare_frame(engine: &Engine, frame: &TraceFrame, entity_map: &EntityMap) -> Vec<String> {
    let mut differences = Vec::new();
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
    // before the (much larger) background-FX table without excluding any
    // entity from the exact comparison.
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
            compare(
                &mut differences,
                id,
                "actor.command",
                command_from_stable_name(&expected_actor.command_name),
                engine.actor_command(id),
            );
        }
        if let Some(expected_human) = &expected.human {
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
    fn transitional_schema_ten_and_current_schema_eleven_are_accepted() {
        validate_trace_schema(10);
        validate_trace_schema(11);
    }

    #[test]
    fn initial_save_decodes_and_matches_its_rhsg_envelope() {
        let save = valid_initial_save();
        let decoded = save
            .decode_and_validate(16_723)
            .expect("valid schema-11 initial_save");
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
            .expect("valid Windows i386 schema-11 initial_save");
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
    #[should_panic(expected = "schema 9; complete resolved-input schema 10 or 11 is required")]
    fn schema_nine_is_rejected() {
        validate_trace_schema(9);
    }

    #[test]
    fn simulation_body_marker_is_mandatory() {
        let frame_without_marker = serde_json::json!({
            "frame_before": 0,
            "frame_after": 1,
            "commands": [],
            "director_completions": [],
            "selected_pcs": [],
            "elements": [],
            "motion_line_changes": [],
            "path_events": [],
            "rng_draws": {
                "first_index": 0,
                "values": [],
                "callsite_offsets": [],
                "domains": []
            }
        });

        let error = serde_json::from_value::<TraceFrame>(frame_without_marker)
            .expect_err("schema 8 frames must report whether the simulation body ran");
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
            domains: vec![TraceRngDomain::Simulation, TraceRngDomain::Audio],
        };
        assert_eq!(batch.gameplay_draw_count(), 1);
    }

    #[test]
    fn original_commands_map_by_semantic_name() {
        assert_eq!(Action::from(TraceAction::Bow), Action::Bow);
        assert_eq!(command_from_stable_name("raise_bow"), Command::RaiseBow);
        assert_eq!(command_from_stable_name("jump"), Command::JumpCmd);
        assert_eq!(command_from_stable_name("roll"), Command::Jump);
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
            building_sectors: BTreeMap::new(),
        };

        map.refresh_trace_index(shifted_trace_id, 158);

        assert_eq!(map.translate(shifted_trace_id), rust_id);
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
