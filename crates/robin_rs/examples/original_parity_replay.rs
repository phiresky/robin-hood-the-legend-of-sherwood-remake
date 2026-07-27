//! Replay a JSONL parity trace produced by the original C++ game.
//!
//! This intentionally accepts the neutral, resolved-command schema emitted by
//! `original-code/RHParity.cpp`, rather than the Rust-native replay schema.
//!
//! Usage:
//!   ROBINHOOD_DATA_DIR=datadirs/demo_leicester_linux \
//!     cargo run --example original_parity_replay -- \
//!       original-code/parity-traces/original-demo-baseline.jsonl

use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::ops::Range;
use std::path::PathBuf;
use std::process::Command as ProcessCommand;
use std::sync::Arc;
use std::{collections::BTreeMap, collections::BTreeSet, collections::VecDeque};

use robin_engine::coordinates::MapPoint;
use robin_engine::coordinates::WorldPoint3D;
use robin_engine::element::{Command, Entity, EntityId, EntityIdKind};
use robin_engine::engine::{DevState, Engine, HostDisplayState, InputState, LevelAssets};
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

#[derive(Debug, Deserialize)]
struct TraceHeader {
    mission: String,
    proto_level: String,
    rng_seed: u64,
    schema: u32,
    synchronous_pathfinding: bool,
    campaign: TraceCampaign,
}

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
struct TraceCampaignCharacter {
    profile_index: u32,
    profile_name: String,
    instanced: bool,
    status: TracePcStatus,
}

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
struct TraceSkill {
    capacity: u32,
    experience: u32,
}

#[derive(Debug, Deserialize)]
struct TraceProductionSector {
    r#type: u32,
    speed: u16,
    amount: u16,
    produced_amount: u16,
    max_amount_reached: bool,
    occupants: Vec<TraceProductionOccupant>,
}

#[derive(Debug, Deserialize)]
struct TraceProductionOccupant {
    character_index: usize,
    x: TraceFloat,
    y: TraceFloat,
    obstacle: u16,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct TraceRngBatch {
    first_index: usize,
    values: Vec<u32>,
    callsite_offsets: Vec<u32>,
    #[serde(default)]
    domains: Vec<TraceRngDomain>,
}

impl TraceRngBatch {
    fn gameplay_draw_count(&self, schema: u32, classifier: &RngDomainClassifier) -> usize {
        assert_eq!(self.values.len(), self.callsite_offsets.len());
        if self.values.is_empty() {
            assert!(
                self.domains.is_empty(),
                "empty RNG batch unexpectedly contains draw domains"
            );
            return 0;
        }
        if !self.domains.is_empty() {
            assert_eq!(
                self.values.len(),
                self.domains.len(),
                "RNG domain stream has a different length than its values"
            );
            return self
                .domains
                .iter()
                .filter(|domain| **domain == TraceRngDomain::Simulation)
                .count();
        }
        assert_eq!(
            schema, 2,
            "schema {schema} RNG batches must include stable draw domains"
        );
        self.callsite_offsets
            .iter()
            .filter(|offset| classifier.is_gameplay(**offset))
            .count()
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TraceRngDomain {
    Simulation,
    Audio,
}

#[derive(Debug, Default)]
struct RngDomainClassifier {
    legacy_audio_symbol_ranges: Vec<Range<u32>>,
}

impl RngDomainClassifier {
    fn for_schema(schema: u32) -> Self {
        if schema != 2 {
            return Self::default();
        }
        let Some(binary) = std::env::var_os("ROBIN_ORIGINAL_BINARY") else {
            return Self::default();
        };
        let binary = PathBuf::from(binary)
            .canonicalize()
            .unwrap_or_else(|error| panic!("canonicalize ROBIN_ORIGINAL_BINARY: {error}"));
        let output = ProcessCommand::new("nm")
            .args(["-S", "--defined-only"])
            .arg(&binary)
            .output()
            .unwrap_or_else(|error| panic!("inspect {} with nm: {error}", binary.display()));
        assert!(
            output.status.success(),
            "nm failed while inspecting legacy Original binary {}: {}",
            binary.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        let classifier = Self::from_nm_output(&String::from_utf8_lossy(&output.stdout));
        assert!(
            !classifier.legacy_audio_symbol_ranges.is_empty(),
            "no RHSound/RHSoundCache symbols found in legacy Original binary {}",
            binary.display()
        );
        eprintln!(
            "classifying schema-2 audio RNG from {} ({} symbol ranges)",
            binary.display(),
            classifier.legacy_audio_symbol_ranges.len()
        );
        classifier
    }

    fn from_nm_output(output: &str) -> Self {
        let legacy_audio_symbol_ranges = output
            .lines()
            .filter_map(|line| {
                let mut fields = line.split_whitespace();
                let start = u64::from_str_radix(fields.next()?, 16).ok()?;
                let size = u64::from_str_radix(fields.next()?, 16).ok()?;
                let _symbol_type = fields.next()?;
                let name = fields.next()?;
                if !(name.starts_with("_ZN7RHSound") || name.starts_with("_ZN12RHSoundCache")) {
                    return None;
                }
                let end = start.checked_add(size)?;
                Some(
                    u32::try_from(start).expect("audio symbol address exceeds trace offset range")
                        ..u32::try_from(end).expect("audio symbol end exceeds trace offset range"),
                )
            })
            .collect();
        Self {
            legacy_audio_symbol_ranges,
        }
    }

    /// Old schema-2 recordings predate stable RNG domains. The fixed offsets
    /// preserve the first baseline trace, while symbol ranges make other
    /// captured Original builds relocatable when ROBIN_ORIGINAL_BINARY names
    /// the exact executable used for recording.
    fn is_gameplay(&self, offset: u32) -> bool {
        if matches!(
            offset,
            3_227_602 | 3_230_325 | 3_296_383 | 3_305_409 | 3_305_465 | 3_305_909 | 3_306_159
        ) {
            return false;
        }
        !self
            .legacy_audio_symbol_ranges
            .iter()
            .any(|range| range.contains(&offset))
    }
}

#[derive(Debug, Deserialize)]
struct TraceRngPrefix {
    #[allow(dead_code)]
    r#type: String,
    draws: TraceRngBatch,
}

#[derive(Debug, Deserialize)]
struct TraceRngOnly {
    #[serde(default)]
    draws: Option<TraceRngBatch>,
    #[serde(default)]
    rng_draws: Option<TraceRngBatch>,
}

#[derive(Debug, Deserialize, Serialize)]
struct RngReplayCache {
    fingerprint: String,
    values: Vec<u32>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
struct TraceEntityId {
    kind: TraceEntityKind,
    index: u32,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
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

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
struct TraceFloat {
    bits: u32,
}

impl TraceFloat {
    fn value(self) -> f32 {
        f32::from_bits(self.bits)
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
struct TracePoint {
    x: TraceFloat,
    y: TraceFloat,
}

impl From<TracePoint> for MapPoint {
    fn from(value: TracePoint) -> Self {
        Self::new(value.x.value(), value.y.value())
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
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

#[derive(Debug, Serialize, Deserialize)]
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
        original_command: u16,
        #[serde(default)]
        original_command_name: Option<String>,
        running: bool,
    },
    LaunchSelfAbility {
        actor: TraceEntityId,
        original_command: u16,
        #[serde(default)]
        original_command_name: Option<String>,
    },
    SwordStrike {
        actor: TraceEntityId,
        target: TraceEntityId,
        original_command: u16,
        #[serde(default)]
        original_command_name: Option<String>,
        with_seek: bool,
    },
    SelectPc {
        pc: TraceEntityId,
        append: bool,
    },
    UnselectAllPcs,
    SelectAction {
        pc: TraceEntityId,
        #[serde(default)]
        action: Option<TraceAction>,
        #[serde(default)]
        original_action: Option<u32>,
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
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
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
    fn into_player_command(self, entity_map: &EntityMap, schema: u32) -> Option<PlayerCommand> {
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
            } => PlayerCommand::GroupMove {
                actors: actors
                    .into_iter()
                    .map(|id| entity_map.translate(id))
                    .collect(),
                destination: destination.into(),
                running,
                show_marker,
                goal_override: Some((SectorNumber::new(goal_sector), goal_layer)),
            },
            Self::LaunchInteraction {
                actor,
                target,
                original_command,
                original_command_name,
                running,
            } => PlayerCommand::LaunchInteraction {
                actor: entity_map.translate(actor),
                target: entity_map.translate(target),
                command: decode_original_command(
                    original_command,
                    original_command_name.as_deref(),
                    schema,
                ),
                running,
            },
            Self::LaunchSelfAbility {
                actor,
                original_command,
                original_command_name,
            } => PlayerCommand::LaunchSelfAbility {
                actor: entity_map.translate(actor),
                command: decode_original_command(
                    original_command,
                    original_command_name.as_deref(),
                    schema,
                ),
            },
            Self::SwordStrike {
                actor,
                target,
                original_command,
                original_command_name,
                with_seek,
            } => PlayerCommand::SwordStrikeCmd {
                actor: entity_map.translate(actor),
                target: entity_map.translate(target),
                command: decode_original_command(
                    original_command,
                    original_command_name.as_deref(),
                    schema,
                ),
                with_seek,
            },
            Self::SelectPc { pc, append } => PlayerCommand::SelectPc {
                pc_id: entity_map.translate(pc),
                append,
            },
            Self::UnselectAllPcs => PlayerCommand::UnselectAllPcs,
            Self::SelectAction {
                pc,
                action,
                original_action,
            } => {
                let action = match (action, original_action) {
                    (Some(action), _) => action.into(),
                    (None, Some(original_action)) if schema == 2 => {
                        Action::try_from(original_action).unwrap_or_else(|_| {
                            panic!("unsupported original RHaction value {original_action}")
                        })
                    }
                    (None, _) => {
                        panic!("schema {schema} select_action lacks a stable semantic action")
                    }
                };
                PlayerCommand::SelectResolvedAction {
                    pc_id: entity_map.translate(pc),
                    action,
                }
            }
            Self::OrientActionAt {
                action,
                original_action: _,
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
        })
    }
}

fn decode_original_command(value: u16, name: Option<&str>, schema: u32) -> Command {
    match name {
        Some(name) => command_from_stable_name(name),
        None if schema == 2 => original_command_to_rust(value),
        None => panic!("schema {schema} RHcommand {value} lacks a stable semantic name"),
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

/// Convert only legacy values whose semantic names have been verified against
/// `original-code/RHCommand.h`. Never cast enum ordinals: the two enums have
/// deliberately diverged (notably original `ROLL` and Rust `Jump`).
fn original_command_to_rust(value: u16) -> Command {
    match value {
        52 => Command::ParrySword,
        59 => Command::SwordstrikeThrustA,
        60 => Command::SwordstrikeThrustB,
        61 => Command::SwordstrikeThrustC,
        62 => Command::SwordstrikeThrustD,
        63 => Command::SwordstrikeThrustE,
        64 => Command::SwordstrikeThrustF,
        65 => Command::SwordstrikeThrustG,
        66 => Command::SwordstrikeThrustH,
        67 => Command::SwordstrikeThrustI,
        75 => Command::WakeUp,
        86 => Command::HitCmd,
        _ => panic!("unsupported original RHcommand value {value}"),
    }
}

/// Map the active legacy sequence command by semantic name. This is separate
/// from resolved player-command conversion above: trace frames contain many
/// engine-internal commands, and Rust inserted/removed enum variants over
/// time, so ordinal casts would report false parity after the first skew.
fn original_active_command_to_rust(value: u16) -> Command {
    match value {
        19 => Command::PassDoor,
        22 => Command::MoveOk,
        23 => Command::MoveWaiting,
        26 => Command::Turn,
        27 => Command::TurnElement,
        28 => Command::TurnFast,
        29 => Command::EquipBow,
        33 => Command::LowerBow,
        34 => Command::RaiseBow,
        35 => Command::ShootBow,
        39 => Command::ReceiveSwordDamage,
        40 => Command::ReceiveArrowDamage,
        42 => Command::ReceiveHitDamage,
        50 => Command::EnterSwordfight,
        51 => Command::QuitSwordfight,
        52 => Command::ParrySword,
        54 => Command::StopParrySword,
        55 => Command::SwordstrikeSmalltalkLeft,
        56 => Command::SwordstrikeSmalltalkRight,
        57 => Command::ParrySmalltalkLeft,
        58 => Command::ParrySmalltalkRight,
        59 => Command::SwordstrikeThrustA,
        60 => Command::SwordstrikeThrustB,
        61 => Command::SwordstrikeThrustC,
        62 => Command::SwordstrikeThrustD,
        63 => Command::SwordstrikeThrustE,
        64 => Command::SwordstrikeThrustF,
        65 => Command::SwordstrikeThrustG,
        66 => Command::SwordstrikeThrustH,
        67 => Command::SwordstrikeThrustI,
        70 => Command::Provoke,
        75 => Command::WakeUp,
        83 => Command::JumpCmd,
        86 => Command::HitCmd,
        87 => Command::HealCmd,
        91 => Command::ThrowWaspNest,
        99 => Command::EnterListen,
        117 => Command::WhistleCmd,
        119 => Command::Point,
        122 => Command::SitDown,
        131 => Command::EnterAttentiveMode,
        132 => Command::LeaveAttentiveMode,
        133 => Command::LeaveAttentiveModeOfficer,
        134 => Command::LeanOut,
        135 => Command::LookLeft,
        136 => Command::LookRight,
        140 => Command::GatherSoldiers,
        160 => Command::WaitTimer,
        161 => Command::Wait,
        164 => Command::PlayAnim,
        165 => Command::PlayAnimLoop,
        _ => panic!("unsupported active original RHcommand value {value}"),
    }
}

#[derive(Debug, Deserialize)]
struct TraceElement {
    entity_id: TraceEntityId,
    creation_order: u32,
    active: bool,
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
    ai: Option<TraceAi>,
}

#[derive(Debug, Deserialize)]
struct TraceActor {
    action_state: u32,
    animation: u32,
    command: u16,
    #[serde(default)]
    command_name: Option<String>,
    motion_state: u32,
    wait_time: u32,
}

#[derive(Debug, Deserialize)]
struct TraceHuman {
    life_points: i16,
    dead: bool,
}

#[derive(Debug, Deserialize)]
struct TraceAi {
    state: u32,
    substate: u32,
}

#[derive(Debug, Deserialize)]
struct TraceFrame {
    frame_before: u64,
    frame_after: u64,
    commands: Vec<TraceCommand>,
    selected_pcs: Vec<TraceEntityId>,
    elements: Vec<TraceElement>,
    rng_draws: TraceRngBatch,
}

struct Options {
    scan_all: bool,
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
    rng_start: usize,
    expected_rng_end: usize,
    actual_rng_end: usize,
    differences: Vec<String>,
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
    let trace_path = options.trace_path;
    let http_server = options.http_server;
    let mut manual_pause = options.start_paused;
    let trace_path = trace_path
        .canonicalize()
        .unwrap_or_else(|e| panic!("canonicalize {}: {e}", trace_path.display()));
    let mut dump = options.dump.map(|options| {
        let file = File::create(&options.path)
            .unwrap_or_else(|e| panic!("create diagnostic dump {}: {e}", options.path.display()));
        (options, BufWriter::new(file))
    });
    let header = read_trace_header(&trace_path);
    assert_eq!(
        header.schema, 4,
        "unsupported parity trace schema {}; campaign-complete schema 4 is required",
        header.schema
    );
    let rng_domain_classifier = RngDomainClassifier::for_schema(header.schema);
    let all_rng_draws = read_all_rng_draws(&trace_path, header.schema, &rng_domain_classifier);

    if let Ok(dir) = std::env::var("ROBINHOOD_DATA_DIR") {
        std::env::set_current_dir(&dir).expect("chdir to ROBINHOOD_DATA_DIR");
    }

    let file = File::open(&trace_path)
        .unwrap_or_else(|e| panic!("open parity trace {}: {e}", trace_path.display()));
    let mut lines = BufReader::new(file).lines();
    let stream_header: TraceHeader = serde_json::from_str(
        &lines
            .next()
            .expect("parity trace has no header")
            .expect("read parity trace header"),
    )
    .expect("parse parity trace header");
    assert_eq!(stream_header.schema, header.schema);
    assert_eq!(stream_header.mission, header.mission);
    assert_eq!(stream_header.rng_seed, header.rng_seed);
    assert_eq!(
        stream_header.synchronous_pathfinding,
        header.synchronous_pathfinding
    );
    assert!(
        header.synchronous_pathfinding,
        "trace was recorded with asynchronous pathfinding"
    );

    let prefix: TraceRngPrefix = serde_json::from_str(
        &lines
            .next()
            .expect("schema 2 parity trace has no RNG prefix")
            .expect("read parity RNG prefix"),
    )
    .expect("parse parity RNG prefix");
    assert_eq!(prefix.r#type, "rng_prefix");
    assert_eq!(prefix.draws.first_index, 0);
    let prefix_end = prefix
        .draws
        .gameplay_draw_count(header.schema, &rng_domain_classifier);
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
    let (mut engine, assets, host, background) = initialize_engine(&header, all_rng_draws);
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
    let automatic_dump_enabled = dump.is_none() && !scan_all;
    let mut rolling_dump = VecDeque::<RollingDumpFrame>::new();

    if let Some(port) = http_server {
        robin_rs::http_server::start_global(port)
            .unwrap_or_else(|e| panic!("start parity replay HTTP server: {e}"));
        eprintln!(
            "parity replay HTTP server ready on http://127.0.0.1:{port} (frame {})",
            engine.frame_counter()
        );
    }

    for (line_index, line) in lines.enumerate() {
        let mut frame: TraceFrame = serde_json::from_str(&line.expect("read parity trace frame"))
            .unwrap_or_else(|e| panic!("parse trace line {}: {e}", line_index + 2));
        assert_eq!(frame.frame_before, line_index as u64);
        assert_eq!(frame.frame_after, frame.frame_before + 1);
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
        let rng_end = rng_start
            + frame
                .rng_draws
                .gameplay_draw_count(header.schema, &rng_domain_classifier);
        gameplay_rng_index = rng_end;
        let resolved_commands =
            serde_json::to_value(&frame.commands).expect("serialize resolved trace commands");
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
        for command in frame.commands.drain(..) {
            if let Some(command) = command.into_player_command(map, header.schema) {
                engine.apply_command(&mut display, &mut input, &assets, &command);
            }
        }
        let tick_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            engine.perform_hourglass(&mut display, &assets, &mut dev)
        }));
        if let Err(payload) = tick_result {
            eprintln!(
                "Rust simulation panicked while replaying original frame {} -> {}",
                frame.frame_before, frame.frame_after
            );
            std::panic::resume_unwind(payload);
        }
        let _ = engine.perform_post_initialize(&mut display, &assets);
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

        let differences = compare_frame(&engine, &frame, map, header.schema);
        if let Some((options, writer)) = &mut dump
            && options.includes(frame.frame_after)
        {
            write_engine_dump_frame(
                writer,
                options,
                &engine,
                map,
                &frame,
                resolved_commands.clone(),
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
                    resolved_commands,
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
    const USAGE: &str = "usage: original_parity_replay [--scan-all] [--visual] \
        [--http-server PORT [--start-paused]] \
        [--dump-jsonl PATH [--dump-from FRAME] [--dump-through FRAME] \
        [--dump-entity KIND:INDEX]...] TRACE.jsonl";

    let mut args = std::env::args_os().skip(1);
    let mut scan_all = false;
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

fn read_trace_header(trace_path: &std::path::Path) -> TraceHeader {
    let file = File::open(trace_path)
        .unwrap_or_else(|e| panic!("open parity trace {}: {e}", trace_path.display()));
    let line = BufReader::new(file)
        .lines()
        .next()
        .expect("parity trace has no header")
        .expect("read parity trace header");
    serde_json::from_str(&line).expect("parse parity trace header")
}

fn read_all_rng_draws(
    trace_path: &std::path::Path,
    schema: u32,
    classifier: &RngDomainClassifier,
) -> Vec<u32> {
    let metadata = std::fs::metadata(trace_path)
        .unwrap_or_else(|error| panic!("stat parity trace {}: {error}", trace_path.display()));
    let modified = metadata
        .modified()
        .expect("parity trace modification time is unavailable")
        .duration_since(std::time::UNIX_EPOCH)
        .expect("parity trace modification time predates Unix epoch")
        .as_nanos();
    let fingerprint = format!(
        "v1:schema={schema}:length={}:modified={modified}:audio_ranges={:?}",
        metadata.len(),
        classifier.legacy_audio_symbol_ranges
    );
    let mut cache_name = trace_path.as_os_str().to_owned();
    cache_name.push(".rng-cache.json");
    let cache_path = PathBuf::from(cache_name);
    if let Ok(file) = File::open(&cache_path)
        && let Ok(cache) = serde_json::from_reader::<_, RngReplayCache>(BufReader::new(file))
        && cache.fingerprint == fingerprint
    {
        eprintln!(
            "loaded {} simulation RNG draws from {}",
            cache.values.len(),
            cache_path.display()
        );
        return cache.values;
    }

    let file = File::open(trace_path)
        .unwrap_or_else(|e| panic!("open parity trace {}: {e}", trace_path.display()));
    let mut result = Vec::new();
    let mut original_index = 0;
    for (line_index, line) in BufReader::new(file).lines().enumerate().skip(1) {
        let record: TraceRngOnly = serde_json::from_str(&line.expect("read parity RNG record"))
            .unwrap_or_else(|e| panic!("parse RNG fields on trace line {}: {e}", line_index + 1));
        if let Some(batch) = record.draws.or(record.rng_draws) {
            assert_eq!(batch.first_index, original_index, "RNG stream has a gap");
            assert_eq!(batch.values.len(), batch.callsite_offsets.len());
            original_index += batch.values.len();
            if batch.values.is_empty() {
                assert!(
                    batch.domains.is_empty(),
                    "empty RNG batch unexpectedly contains draw domains"
                );
                continue;
            }
            if !batch.domains.is_empty() {
                assert_eq!(
                    batch.values.len(),
                    batch.domains.len(),
                    "RNG domain stream has a different length than its values"
                );
                result.extend(batch.values.into_iter().zip(batch.domains).filter_map(
                    |(value, domain)| (domain == TraceRngDomain::Simulation).then_some(value),
                ));
            } else {
                assert_eq!(
                    schema, 2,
                    "schema {schema} RNG batches must include stable draw domains"
                );
                result.extend(
                    batch
                        .values
                        .into_iter()
                        .zip(batch.callsite_offsets)
                        .filter_map(|(value, offset)| {
                            classifier.is_gameplay(offset).then_some(value)
                        }),
                );
            }
        }
    }
    let cache = RngReplayCache {
        fingerprint,
        values: result.clone(),
    };
    let file = File::create(&cache_path)
        .unwrap_or_else(|error| panic!("create RNG cache {}: {error}", cache_path.display()));
    serde_json::to_writer(BufWriter::new(file), &cache)
        .unwrap_or_else(|error| panic!("write RNG cache {}: {error}", cache_path.display()));
    eprintln!(
        "cached {} simulation RNG draws in {}",
        result.len(),
        cache_path.display()
    );
    result
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
        sim_config: robin_engine::engine::SimConfig {
            synchronous_pathfinding: true,
            ..Default::default()
        },
    })
    .expect("initialize engine");
    (engine, assets, host, background)
}

struct EntityMap {
    entities: BTreeMap<TraceEntityId, EntityId>,
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
        Self {
            entities: result,
            building_sectors,
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

fn compare_frame(
    engine: &Engine,
    frame: &TraceFrame,
    entity_map: &EntityMap,
    schema: u32,
) -> Vec<String> {
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
        if !mapped_building_sector {
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
                match expected_actor.command_name.as_deref() {
                    Some(name) => command_from_stable_name(name),
                    None if schema == 2 => original_active_command_to_rust(expected_actor.command),
                    None => panic!(
                        "schema {schema} actor RHcommand {} lacks a stable semantic name",
                        expected_actor.command
                    ),
                },
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

    #[test]
    fn stable_rng_domains_do_not_depend_on_callsite_offsets() {
        let batch = TraceRngBatch {
            first_index: 0,
            values: vec![1, 2],
            callsite_offsets: vec![3_305_465, 123],
            domains: vec![TraceRngDomain::Simulation, TraceRngDomain::Audio],
        };
        assert_eq!(
            batch.gameplay_draw_count(3, &RngDomainClassifier::default()),
            1
        );
    }

    #[test]
    fn schema_two_audio_callsite_can_be_resolved_from_symbols() {
        let classifier = RngDomainClassifier::from_nm_output(
            "00327400 00000040 T _ZN7RHSound9HourglassEv\n\
             00400000 00000020 T _ZN8NotAudio4DrawEv\n",
        );
        assert!(!classifier.is_gameplay(0x0032_7410));
        assert!(classifier.is_gameplay(0x0040_0010));
    }

    #[test]
    fn original_schema_two_baseline_offsets_remain_supported() {
        assert!(!RngDomainClassifier::default().is_gameplay(3_305_465));
        assert!(RngDomainClassifier::default().is_gameplay(1));
    }

    #[test]
    fn original_equip_bow_command_maps_by_semantic_name() {
        assert_eq!(original_active_command_to_rust(29), Command::EquipBow);
        assert_eq!(Action::from(TraceAction::Bow), Action::Bow);
        assert_eq!(command_from_stable_name("raise_bow"), Command::RaiseBow);
        assert_eq!(command_from_stable_name("jump"), Command::JumpCmd);
        assert_eq!(command_from_stable_name("roll"), Command::Jump);
    }

    #[test]
    fn schema_three_resolved_orientation_is_bit_exact() {
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
