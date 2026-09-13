//! Original v48 local-AI payload decoding.
//!
//! AI serialization has a common prefix followed by a type-specific payload.
//! The type identity is not present
//! in the stream, so callers must derive it from the owning element class.
//! The common reader exposes that exact payload boundary for diagnostics;
//! [`LegacyLocalAiPayload`] continues through either complete v48 subclass.
//! No decoder scans for a later fingerprint or guesses byte counts.
//! Field declaration order is wire order.

use super::read_helpers::DEFAULT_BULK_LIMIT;
use super::read_helpers::hex16;
use serde::{Deserialize, Serialize};

use crate::legacy_io::{LegacyRead, LegacyReader, LegacyResult};

use super::LegacySaveAbiProfile;
use super::payload_base::{
    LegacyAiElementRef, LegacyElementRef, LegacyLineRef, LegacySectorRef, read_ai_element_ref,
    read_element_ref, read_nullable_u16, read_sector_ref,
};

const AI_FINGERPRINT: [u8; 16] = hex16("02c3dacf6a5a1868e649569740c7fe14");
const POSITION_FINGERPRINT: [u8; 16] = hex16("16171037069ea629a320950e904da3ba");
const STIMULUS_FINGERPRINT: [u8; 16] = hex16("fed36c329a78c5b76534124341270899");
const RECONNAISSANCE_FINGERPRINT: [u8; 16] = hex16("c99555a7400566f2e984f47860cf828b");
const PATH_FINGERPRINT: [u8; 16] = hex16("f2781c304bb147aa1defc89ab1033082");
const HUMANS_LIST_FINGERPRINT: [u8; 16] = hex16("e1edc9e0991a413e5577613783f4333d");
const NPC_LIST_FINGERPRINT: [u8; 16] = hex16("d36d2f762287f69bcb71a45982dde0ca");
const OBJECT_LIST_FINGERPRINT: [u8; 16] = hex16("ee2a8180604e52d16d2844512ba84d7f");
const BONHOMIE_FINGERPRINT: [u8; 16] = hex16("6a8a5ae26b698c516e64d1767753cd9d");
const MALIGNITY_FINGERPRINT: [u8; 16] = hex16("4a5e5b668d2eb6d3b8ec78313111c571");
const SEEK_POINT_ALL_FINGERPRINT: [u8; 16] = hex16("a9b877b827568572a12866cfa54c26ac");
const SEEK_POINT_STATUS_FINGERPRINT: [u8; 16] = hex16("1d8f13888a44ed97abc70ec98d7132a1");
/// Sentinel the Original uses for "no shooting point / archery sector".
const NO_ARCHERY_SECTOR: u16 = 666;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LegacyLocalAiKind {
    /// Friendly AI, used by civilians.
    Friendly,
    /// Hostile AI, used by soldiers.
    Enemy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyLocalAiLimits {
    pub forbidden_remarks: usize,
    pub log_lines: usize,
    pub path_history: usize,
    pub element_lists: usize,
    pub stimulus_queue: usize,
    pub reconnaissance_bodies: usize,
    pub enemy_positions: usize,
    pub ambush_statuses: usize,
    pub seek_point_ids: usize,
    pub seek_directions: usize,
}

impl Default for LegacyLocalAiLimits {
    fn default() -> Self {
        Self {
            forbidden_remarks: DEFAULT_BULK_LIMIT,
            log_lines: DEFAULT_BULK_LIMIT,
            path_history: DEFAULT_BULK_LIMIT,
            element_lists: DEFAULT_BULK_LIMIT,
            stimulus_queue: DEFAULT_BULK_LIMIT,
            reconnaissance_bodies: DEFAULT_BULK_LIMIT,
            enemy_positions: DEFAULT_BULK_LIMIT,
            ambush_statuses: DEFAULT_BULK_LIMIT,
            seek_point_ids: DEFAULT_BULK_LIMIT,
            seek_directions: DEFAULT_BULK_LIMIT,
        }
    }
}

/// Non-self-describing facts supplied by the initialized mission.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyLocalAiDecodeConfig {
    pub kind: Option<LegacyLocalAiKind>,
    pub limits: LegacyLocalAiLimits,
    pub abi_profile: LegacySaveAbiProfile,
}

impl LegacyLocalAiDecodeConfig {
    pub fn for_kind(kind: LegacyLocalAiKind) -> Self {
        Self {
            kind: Some(kind),
            limits: LegacyLocalAiLimits::default(),
            abi_profile: LegacySaveAbiProfile::PortLinuxI386V48,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(fingerprint = POSITION_FINGERPRINT, expected = "AI position fingerprint")]
pub struct LegacyAiPosition {
    pub x: f32,
    pub y: f32,
    pub level: u16,
    pub sector: LegacySectorRef,
}

/// Generic stimulus positions are `x, y, sector, level` on the wire; the
/// noise origin is the exception (see [`read_noise_stimulus_position`]).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
pub struct LegacyStimulusPosition {
    pub x: f32,
    pub y: f32,
    pub sector: LegacySectorRef,
    pub level: u16,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LegacyStimulusInfo {
    None,
    Noise {
        /// The original game first writes the saved-position size. On both audited
        /// 32-bit ABIs this is x, y, an obsolete raw pointer, level, padding.
        raw_x: f32,
        raw_y: f32,
        raw_sector_pointer: u32,
        raw_level: u16,
        raw_padding: u16,
        first_sector: LegacySectorRef,
        origin: LegacyStimulusPosition,
        noise_type: i32,
        volume: u16,
        elevation: u16,
    },
    Position(LegacyStimulusPosition),
    Human(LegacyAiElementRef),
    Hint {
        position: LegacyStimulusPosition,
        teller: LegacyAiElementRef,
        seek_flags: u16,
    },
    Object(LegacyAiElementRef),
    Stolen {
        object: LegacyAiElementRef,
        thief: LegacyAiElementRef,
    },
    Combat {
        enemy_position: LegacyStimulusPosition,
        actor: LegacyAiElementRef,
    },
    DoorCombat {
        delay: u16,
        direction: u16,
        goal: LegacyStimulusPosition,
        adversary: LegacyAiElementRef,
    },
    Index(u16),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyStimulus {
    pub to_whole_patrol: bool,
    pub stimulus_type: i32,
    pub info_type: i32,
    pub owner: LegacyElementRef,
    pub info: LegacyStimulusInfo,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
pub struct LegacyAiPathHistoryEntry {
    #[legacy(name = "position.x")]
    pub position_x: f32,
    #[legacy(name = "position.y")]
    pub position_y: f32,
    pub sector: LegacySectorRef,
    pub level: u16,
    pub direction: u8,
    pub distance: u16,
}

/// Context: the maximum history length.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(ctx = usize, fingerprint = PATH_FINGERPRINT, expected = "path-status fingerprint")]
pub struct LegacyAiPathStatus {
    pub current_waypoint_index: u8,
    pub last_waypoint_index: u8,
    pub forward_movement: bool,
    #[legacy(with = read_nullable_u16)]
    pub hiking_path_index: Option<u16>,
    #[legacy(read = read_ai_path_history(reader, *ctx))]
    pub history: Vec<LegacyAiPathHistoryEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LegacyAiLogLine {
    /// The Linux port's enum serialization writes only the first four
    /// bytes of the log-line aggregate.
    PortLinuxType(i32),
    /// Windows retail writes the complete naturally aligned 12-byte
    /// log-line aggregate.
    RetailWindows {
        log_type: i32,
        info: u16,
        alignment_padding: u16,
        frame: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(
    ctx = LegacyLocalAiLimits,
    fingerprint = RECONNAISSANCE_FINGERPRINT,
    expected = "reconnaissance-report fingerprint"
)]
pub struct LegacyReconnaissanceReport {
    pub report_type: i32,
    #[legacy(name = "seek_position.x")]
    pub seek_position_x: f32,
    #[legacy(name = "seek_position.y")]
    pub seek_position_y: f32,
    #[legacy(name = "seek_position.obsolete_sector_pointer")]
    pub obsolete_sector_pointer: u32,
    #[legacy(name = "seek_position.level")]
    pub seek_position_level: u16,
    #[legacy(name = "seek_position.alignment_padding")]
    pub alignment_padding: u16,
    #[legacy(name = "seek_position.sector")]
    pub seek_position_sector: LegacySectorRef,
    #[legacy(count_u32 = ctx.reconnaissance_bodies)]
    pub seen_bodies: Vec<LegacyElementRef>,
    pub charly: LegacyElementRef,
    pub charly_seen: bool,
}

/// The exact point immediately before the type-specific payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyLocalAiSubclassBoundary {
    pub kind: LegacyLocalAiKind,
    pub byte_offset: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(
    ctx = LegacyLocalAiLimits,
    fingerprint = SEEK_POINT_ALL_FINGERPRINT,
    expected = "seek-point data fingerprint"
)]
pub struct LegacySeekPoint {
    #[legacy(name = "position.x")]
    pub position_x: f32,
    #[legacy(name = "position.y")]
    pub position_y: f32,
    #[legacy(name = "position.level")]
    pub position_level: u16,
    #[legacy(name = "position.sector")]
    pub position_sector: LegacySectorRef,
    pub frame_when_fully_interesting: u32,
    #[legacy(count_u32 = ctx.seek_directions, items)]
    pub directions: Vec<u16>,
    pub last_calculated_interest: u8,
    pub locked: bool,
    /// The payload ends with a second copy of the status.
    #[legacy(
        fingerprint = SEEK_POINT_STATUS_FINGERPRINT,
        fingerprint_name = "status.fingerprint",
        expected = "seek-point fingerprint",
        name = "status.frame_when_fully_interesting"
    )]
    pub repeated_frame_when_fully_interesting: u32,
    #[legacy(name = "status.last_calculated_interest")]
    pub repeated_last_calculated_interest: u8,
    #[legacy(name = "status.locked")]
    pub repeated_locked: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(fingerprint = BONHOMIE_FINGERPRINT, expected = "friendly AI fingerprint")]
pub struct LegacyFriendlyAiTail {
    pub fleeing_seen_enemy_counter: u16,
    pub beggar_dont_talk_counter: u16,
    pub wants_to_talk: bool,
    pub last_talk_partner: LegacyAiElementRef,
    pub can_go_away: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyShootingPointRef {
    pub sector_index: u16,
    pub point_index: u16,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(
    ctx = LegacyLocalAiLimits,
    fingerprint = MALIGNITY_FINGERPRINT,
    expected = "enemy AI fingerprint"
)]
pub struct LegacyEnemyAiTail {
    #[legacy(read = reader.scope("last_stimulus_dispatched_to_patrol", |reader| {
        read_stimulus(reader, ctx.element_lists)
    }))]
    pub last_stimulus_dispatched_to_patrol: LegacyStimulus,
    pub frame_when_missed_charly: u32,
    #[legacy(with = read_ai_ref_list, args(OBJECT_LIST_FINGERPRINT, ctx.element_lists))]
    pub heard_nets: Vec<LegacyAiElementRef>,
    pub frame_when_enemy_detected: u32,
    pub fleeing_seen_enemy_counter: u16,
    #[legacy(with = read_ai_ref_list, args(OBJECT_LIST_FINGERPRINT, ctx.element_lists))]
    pub other_seen_ale: Vec<LegacyAiElementRef>,
    pub pc_gone_away_direction: u16,
    pub detected_something_there: LegacyAiPosition,
    pub missed_pc: LegacyAiElementRef,
    pub last_seek_direction_index: u8,
    pub beggar_to_examine: LegacyAiElementRef,
    pub pc_missed: bool,
    #[legacy(count_u32 = ctx.enemy_positions, items)]
    pub search_charly_way: Vec<LegacyAiPosition>,
    pub current_task_priority: u16,
    pub minimal_task_priority: u16,
    pub new_task_priority: u16,
    pub number_of_different_checkpoints: u8,
    pub delta_sorrow_level: u16,
    #[legacy(with = read_ai_ref_list, args(NPC_LIST_FINGERPRINT, ctx.element_lists))]
    pub missed_in_action: Vec<LegacyAiElementRef>,
    #[legacy(with = read_ai_ref_list, args(HUMANS_LIST_FINGERPRINT, ctx.element_lists))]
    pub other_bodies_to_examine: Vec<LegacyAiElementRef>,
    #[legacy(with = read_ai_ref_list, args(HUMANS_LIST_FINGERPRINT, ctx.element_lists))]
    pub beggars_to_control: Vec<LegacyAiElementRef>,
    pub thirsty: bool,
    pub old_life_points: u8,
    pub initial_life_points: u8,
    #[legacy(with = read_ai_ref_list, args(HUMANS_LIST_FINGERPRINT, ctx.element_lists))]
    pub them: Vec<LegacyAiElementRef>,
    pub old_odds: i16,
    pub position_change_locked_for_test: bool,
    pub ambush_point_array_reset: bool,
    #[legacy(count_u32 = ctx.ambush_statuses, items)]
    pub ambush_point_statuses: Vec<i32>,
    #[legacy(count_u32 = ctx.seek_point_ids, items)]
    pub seek_point_ids: Vec<u32>,
    pub actual_seek_point_id: u32,
    #[legacy(count_u32 = ctx.seek_directions, items)]
    pub seek_point_view_directions_before_personal_points: Vec<u16>,
    #[legacy(with = read_optional_seek_point, args(ctx))]
    pub personal_seek_point_1: Option<LegacySeekPoint>,
    #[legacy(with = read_optional_seek_point, args(ctx))]
    pub personal_seek_point_2: Option<LegacySeekPoint>,
    pub seek_center: LegacyAiPosition,
    #[legacy(count_u32 = ctx.seek_directions, items)]
    pub seek_point_view_directions: Vec<u16>,
    #[legacy(count_u32 = ctx.enemy_positions, items)]
    pub positions_of_beggars_to_control: Vec<LegacyAiPosition>,
    pub seek_flags: u16,
    pub forced_next_battle_decision: i32,
    pub reset_battle_decision: bool,
    pub synchronize_index: u16,
    pub seen_dead_body: bool,
    pub seeking_charly: bool,
    pub initial_view_cone: i32,
    pub repeated_seek_flags: u16,
    pub company_number: u16,
    pub left_combat_neighbour: LegacyAiElementRef,
    pub right_combat_neighbour: LegacyAiElementRef,
    pub attentive: bool,
    pub will_be_attentive: bool,
    pub forced_attentive: bool,
    pub guarded_pc: LegacyAiElementRef,
    pub tower_guard: bool,
    pub combat_trainer: bool,
    pub gather_position: LegacyAiPosition,
    pub gather_direction: u16,
    pub gather_position_instructed: bool,
    pub officers_position: LegacyAiPosition,
    pub previous_state: i32,
    pub previous_substate: i32,
    pub reported_to_officer: bool,
    pub missed_soldier_timer: u16,
    pub old_money: u16,
    #[legacy(with = read_ai_ref_list, args(OBJECT_LIST_FINGERPRINT, ctx.element_lists))]
    pub other_seen_money: Vec<LegacyAiElementRef>,
    #[legacy(with = read_ai_ref_list, args(NPC_LIST_FINGERPRINT, ctx.element_lists))]
    pub money_fight_enemies: Vec<LegacyAiElementRef>,
    #[legacy(with = read_ai_ref_list, args(NPC_LIST_FINGERPRINT, ctx.element_lists))]
    pub money_fight_victims: Vec<LegacyAiElementRef>,
    pub archer_behind_me: LegacyAiElementRef,
    pub shield_bearer_before_me: LegacyAiElementRef,
    #[legacy(with = read_ai_ref_list, args(HUMANS_LIST_FINGERPRINT, ctx.element_lists))]
    pub already_seen_bodies: Vec<LegacyAiElementRef>,
    pub jump_line: LegacyLineRef,
    pub shield_bearer_direction: u16,
    pub phalanx_aborted: bool,
    pub changed_to_alert_path: bool,
    #[legacy(read = read_shooting_point(reader))]
    pub shooting_point: Option<LegacyShootingPointRef>,
    #[legacy(read = read_archery_sector(reader))]
    pub archery_sector: Option<u16>,
    pub archery_sector_index: u16,
    pub archery_point_index: u16,
    pub archery_point_increment: i8,
    pub enemy_seen_below: bool,
    pub enemy_had_this_elevation: u16,
    pub known_enemy_strike_commands: [i32; 3],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LegacyLocalAiTail {
    Friendly(LegacyFriendlyAiTail),
    Enemy(LegacyEnemyAiTail),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyLocalAiPayload {
    pub common: LegacyLocalAiCommon,
    pub tail: LegacyLocalAiTail,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, LegacyRead)]
#[legacy(
    ctx = LegacyLocalAiDecodeConfig,
    fingerprint = AI_FINGERPRINT,
    expected = "local AI fingerprint"
)]
pub struct LegacyLocalAiCommon {
    pub last_goto_destination: LegacyAiPosition,
    pub last_goto_flags: u16,
    pub stuck_counter: u16,
    #[legacy(count_u32 = ctx.limits.forbidden_remarks, items)]
    pub forbidden_remarks: Vec<i32>,
    pub current_remark_flags: u16,
    /// The log-line value uses enum serialization, hence four bytes.
    #[legacy(read = read_ai_log_lines(reader, ctx.abi_profile, ctx.limits.log_lines))]
    pub log_lines: Vec<LegacyAiLogLine>,
    pub owner: LegacyAiElementRef,
    pub current_state: i32,
    pub old_state: i32,
    pub current_substate: i32,
    pub current_music_alert_status: i32,
    pub substate_at_last_timer_launch: i32,
    pub attitude: i32,
    pub blood_alcohol: u8,
    pub initial_action: i32,
    pub number_of_looks: u8,
    pub can_move: bool,
    pub stop_before_end_of_path: bool,
    pub use_max_norm_to_stop_before_end_of_path: bool,
    pub stop_before_end_of_path_distance: u16,
    #[legacy(read = LegacyAiPathStatus::read_field(reader, "path", &ctx.limits.path_history))]
    pub path: LegacyAiPathStatus,
    pub has_patrol_path: bool,
    #[legacy(when = has_patrol_path)]
    pub macro_command_offset: Option<u16>,
    pub remaining_macro_bytes: u16,
    pub macro_in_progress: bool,
    pub primary_target: LegacyAiElementRef,
    pub friend_in_trouble: LegacyAiElementRef,
    pub detected_body: LegacyAiElementRef,
    pub interesting_object: LegacyAiElementRef,
    pub antagonist: LegacyAiElementRef,
    pub last_stimulus_actor: LegacyAiElementRef,
    pub timer_is_running: bool,
    pub timer_ring_frame: u32,
    pub macro_timer_is_running: bool,
    pub macro_timer_ring_frame: u32,
    pub standing_around_timer: u16,
    pub sorrow_level: u16,
    #[legacy(scoped)]
    pub last_stimuli: [i32; 5],
    #[legacy(scoped)]
    pub last_stimulus_multiplicities: [u16; 5],
    pub is_master: bool,
    pub master: LegacyAiElementRef,
    pub seek_position: LegacyAiPosition,
    pub alert_soldiers_point: LegacyAiPosition,
    pub first_try: bool,
    #[legacy(name = "panic_center.x")]
    pub panic_center_x: f32,
    #[legacy(name = "panic_center.y")]
    pub panic_center_y: f32,
    pub lasting_panic_runs: u8,
    pub directed_panic: bool,
    #[legacy(with = read_ai_ref_list, args(HUMANS_LIST_FINGERPRINT, ctx.limits.element_lists))]
    pub us: Vec<LegacyAiElementRef>,
    #[legacy(with = read_ai_ref_list, args(NPC_LIST_FINGERPRINT, ctx.limits.element_lists))]
    pub alerted_us: Vec<LegacyAiElementRef>,
    #[legacy(with = read_ai_ref_list, args(NPC_LIST_FINGERPRINT, ctx.limits.element_lists))]
    pub staying_us: Vec<LegacyAiElementRef>,
    pub could_not_reach_point: bool,
    pub already_on_point: bool,
    pub already_turned: bool,
    pub likes_to_sit_around: bool,
    pub special_action: bool,
    pub remaining_tequila_gulps: u8,
    pub friends_are_alerted: bool,
    pub stay_at_home: bool,
    pub locks_flag_field: u8,
    pub was_busy: bool,
    #[legacy(read = read_stimulus_list(
        reader,
        ctx.limits.stimulus_queue,
        ctx.limits.element_lists,
    ))]
    pub stimulus_queue: Vec<LegacyStimulus>,
    pub script_locked: bool,
    pub remember_events: bool,
    pub leave_house_number: u16,
    pub last_hint_actuality: u32,
    pub last_hint_subject: i32,
    #[legacy(with = read_optional_i16)]
    pub door_index: Option<i16>,
    #[legacy(with = read_ai_ref_list, args(OBJECT_LIST_FINGERPRINT, ctx.limits.element_lists))]
    pub forgotten_objects: Vec<LegacyAiElementRef>,
    pub object_of_desire: LegacyElementRef,
    pub checkpoint_charly: LegacyElementRef,
    pub synchronize_charly: LegacyElementRef,
    pub inside_halt_method: bool,
    pub macro_started_this_frame: bool,
    #[legacy(with = read_ai_ref_list, args(NPC_LIST_FINGERPRINT, ctx.limits.element_lists))]
    pub synchronizing_actors: Vec<LegacyAiElementRef>,
    pub default_path_walking_flags: u16,
    pub looking_for_help_because_enemy_seen: bool,
    pub current_remark: i32,
    /// The next macro-random byte is stored as one byte, but v48 erroneously serializes a 16-bit value
    /// starting at its address. The high byte overlaps the adjacent bool.
    pub next_macro_rand_word: u16,
    #[legacy(value = next_macro_rand_word as u8)]
    pub next_macro_rand: u8,
    #[legacy(value = (next_macro_rand_word >> 8) as u8)]
    pub overlapped_forecast_byte: u8,
    /// Serialized again after the overlapping 16-bit value and therefore authoritative.
    pub next_macro_rand_forecasted: bool,
    pub current_emoticon_type: i32,
    pub emoticon_expiration_date: u32,
    pub emoticon_has_expiration_date: bool,
    #[legacy(read = LegacyReconnaissanceReport::read_field(reader, "reconnaissance", &ctx.limits))]
    pub reconnaissance: LegacyReconnaissanceReport,
    pub knocked_out_in_money_fight: bool,
    pub got_beggar_trick: bool,
    pub patrol_chief: LegacyElementRef,
    #[legacy(with = read_ai_ref_list, args(NPC_LIST_FINGERPRINT, ctx.limits.element_lists))]
    pub patrol: Vec<LegacyAiElementRef>,
    #[legacy(with = read_ai_ref_list, args(NPC_LIST_FINGERPRINT, ctx.limits.element_lists))]
    pub missed_patrol_members: Vec<LegacyAiElementRef>,
    #[legacy(with = read_ai_ref_list, args(NPC_LIST_FINGERPRINT, ctx.limits.element_lists))]
    pub theoretical_patrol: Vec<LegacyAiElementRef>,
    pub patrol_stopped: bool,
    pub patrol_direction: u16,
    #[legacy(read = read_subclass_boundary(reader, ctx))]
    pub subclass: LegacyLocalAiSubclassBoundary,
}

impl LegacyLocalAiCommon {
    pub fn read(
        reader: &mut LegacyReader<'_>,
        config: &LegacyLocalAiDecodeConfig,
    ) -> LegacyResult<Self> {
        // The subclass kind is not self-describing: reject its absence
        // before consuming any bytes.
        local_ai_kind(reader, config)?;
        <Self as LegacyRead<_>>::read(reader, config)
    }
}

fn local_ai_kind(
    reader: &mut LegacyReader<'_>,
    config: &LegacyLocalAiDecodeConfig,
) -> LegacyResult<LegacyLocalAiKind> {
    config.kind.ok_or_else(|| {
        let offset = reader.offset();
        reader.invalid_value(
            offset,
            "kind",
            "missing",
            "caller-supplied Friendly or Enemy local-AI kind",
        )
    })
}

fn read_subclass_boundary(
    reader: &mut LegacyReader<'_>,
    config: &LegacyLocalAiDecodeConfig,
) -> LegacyResult<LegacyLocalAiSubclassBoundary> {
    Ok(LegacyLocalAiSubclassBoundary {
        kind: local_ai_kind(reader, config)?,
        byte_offset: reader.offset(),
    })
}

impl LegacyLocalAiPayload {
    pub fn read(
        reader: &mut LegacyReader<'_>,
        config: &LegacyLocalAiDecodeConfig,
    ) -> LegacyResult<Self> {
        let common = LegacyLocalAiCommon::read(reader, config)?;
        let tail = LegacyLocalAiTail::read(reader, common.subclass, &config.limits)?;
        Ok(Self { common, tail })
    }
}

impl LegacyLocalAiTail {
    pub fn read(
        reader: &mut LegacyReader<'_>,
        boundary: LegacyLocalAiSubclassBoundary,
        limits: &LegacyLocalAiLimits,
    ) -> LegacyResult<Self> {
        let offset = reader.offset();
        if offset != boundary.byte_offset {
            return Err(reader.invalid_value(
                offset,
                "subclass.byte_offset",
                offset,
                "current reader offset equal to the common AI subclass boundary",
            ));
        }
        match boundary.kind {
            LegacyLocalAiKind::Friendly => {
                LegacyFriendlyAiTail::read_field(reader, "friendly", &()).map(Self::Friendly)
            }
            LegacyLocalAiKind::Enemy => {
                LegacyEnemyAiTail::read_field(reader, "enemy", limits).map(Self::Enemy)
            }
        }
    }
}

fn read_shooting_point(
    reader: &mut LegacyReader<'_>,
) -> LegacyResult<Option<LegacyShootingPointRef>> {
    let sector_index = reader.read_u16("shooting_point.sector_index")?;
    if sector_index == NO_ARCHERY_SECTOR {
        return Ok(None);
    }
    Ok(Some(LegacyShootingPointRef {
        sector_index,
        point_index: reader.read_u16("shooting_point.point_index")?,
    }))
}

fn read_archery_sector(reader: &mut LegacyReader<'_>) -> LegacyResult<Option<u16>> {
    let raw = reader.read_u16("archery_sector")?;
    Ok((raw != NO_ARCHERY_SECTOR).then_some(raw))
}

fn read_optional_seek_point(
    reader: &mut LegacyReader<'_>,
    field: &'static str,
    limits: &LegacyLocalAiLimits,
) -> LegacyResult<Option<LegacySeekPoint>> {
    reader.scope(field, |reader| {
        if reader.read_bool("present")? {
            LegacySeekPoint::read(reader, limits).map(Some)
        } else {
            Ok(None)
        }
    })
}

/// `None` for the `-1` null sentinel.
fn read_optional_i16(
    reader: &mut LegacyReader<'_>,
    field: &'static str,
) -> LegacyResult<Option<i16>> {
    let raw = reader.read_i16(field)?;
    Ok((raw != -1).then_some(raw))
}

fn read_ai_log_lines(
    reader: &mut LegacyReader<'_>,
    abi_profile: LegacySaveAbiProfile,
    maximum: usize,
) -> LegacyResult<Vec<LegacyAiLogLine>> {
    reader.scope("log_lines", |reader| {
        let count = reader.read_count_u32("count", maximum)?;
        reader.read_list("items", count, |reader, item| {
            reader.scope(item, |reader| {
                Ok(match abi_profile {
                    LegacySaveAbiProfile::PortLinuxI386V48 => {
                        LegacyAiLogLine::PortLinuxType(reader.read_i32("log_type")?)
                    }
                    LegacySaveAbiProfile::RetailWindowsX86V48 => {
                        LegacyAiLogLine::RetailWindows {
                            log_type: reader.read_i32("log_type")?,
                            info: reader.read_u16("info")?,
                            // The Windows layout inserts two bytes before the 32-bit frame.
                            // They are not behavior, but retaining them keeps
                            // the compatibility decoder lossless.
                            alignment_padding: reader.read_u16("alignment_padding")?,
                            frame: reader.read_u32("frame")?,
                        }
                    }
                })
            })
        })
    })
}

fn read_ai_path_history(
    reader: &mut LegacyReader<'_>,
    maximum_history: usize,
) -> LegacyResult<Vec<LegacyAiPathHistoryEntry>> {
    let count = reader.read_u16("history.count")? as usize;
    ensure_count(reader, "history.count", count, maximum_history)?;
    reader.read_list("history", count, |reader, item| {
        LegacyAiPathHistoryEntry::read_field(reader, item, &())
    })
}

fn read_stimulus_list(
    reader: &mut LegacyReader<'_>,
    maximum: usize,
    maximum_nested_list: usize,
) -> LegacyResult<Vec<LegacyStimulus>> {
    let count = reader.read_count_u32("stimulus_queue.count", maximum)?;
    reader.read_list("stimulus_queue", count, |reader, item| {
        reader.scope(item, |reader| read_stimulus(reader, maximum_nested_list))
    })
}

// Hand-written: the payload shape depends on the preceding `info_type`.
fn read_stimulus(
    reader: &mut LegacyReader<'_>,
    _maximum_nested_list: usize,
) -> LegacyResult<LegacyStimulus> {
    reader.read_signature("fingerprint", STIMULUS_FINGERPRINT, "stimulus fingerprint")?;
    let to_whole_patrol = reader.read_bool("to_whole_patrol")?;
    let stimulus_type = reader.read_i32("stimulus_type")?;
    let info_type = reader.read_i32("info_type")?;
    let owner = read_element_ref(reader, "owner")?;
    let info = match info_type {
        0 => LegacyStimulusInfo::None,
        1 => {
            let raw_x = reader.read_f32("noise.raw_position.x")?;
            let raw_y = reader.read_f32("noise.raw_position.y")?;
            let raw_sector_pointer = reader.read_u32("noise.raw_position.sector_pointer")?;
            let raw_level = reader.read_u16("noise.raw_position.level")?;
            let raw_padding = reader.read_u16("noise.raw_position.padding")?;
            let first_sector = read_sector_ref(reader, "noise.first_sector")?;
            // INFO_NOISE is the sole stimulus position variant whose v22+
            // compatibility payload writes the level before the sector reference.
            // Stimulus serialization; the generic position variants below
            // retain their x, y, sector, level wire order.
            let origin = read_noise_stimulus_position(reader)?;
            let noise_type = reader.read_i32("noise.type")?;
            let volume = reader.read_u16("noise.volume")?;
            let elevation = reader.read_u16("noise.elevation")?;
            LegacyStimulusInfo::Noise {
                raw_x,
                raw_y,
                raw_sector_pointer,
                raw_level,
                raw_padding,
                first_sector,
                origin,
                noise_type,
                volume,
                elevation,
            }
        }
        2 => LegacyStimulusInfo::Position(LegacyStimulusPosition::read_field(
            reader,
            "position",
            &(),
        )?),
        3 => LegacyStimulusInfo::Human(read_ai_element_ref(reader, "human")?),
        4 => LegacyStimulusInfo::Hint {
            position: LegacyStimulusPosition::read_field(reader, "hint.position", &())?,
            teller: read_ai_element_ref(reader, "hint.teller")?,
            seek_flags: reader.read_u16("hint.seek_flags")?,
        },
        5 => LegacyStimulusInfo::Object(read_ai_element_ref(reader, "object")?),
        6 => LegacyStimulusInfo::Stolen {
            object: read_ai_element_ref(reader, "stolen.object")?,
            thief: read_ai_element_ref(reader, "stolen.thief")?,
        },
        7 => LegacyStimulusInfo::Combat {
            enemy_position: LegacyStimulusPosition::read_field(
                reader,
                "combat.enemy_position",
                &(),
            )?,
            actor: read_ai_element_ref(reader, "combat.actor")?,
        },
        8 => {
            let delay = reader.read_u16("door_combat.delay")?;
            let direction = reader.read_u16("door_combat.direction")?;
            let goal = LegacyStimulusPosition::read_field(reader, "door_combat.goal", &())?;
            let adversary = read_ai_element_ref(reader, "door_combat.adversary")?;
            LegacyStimulusInfo::DoorCombat {
                delay,
                direction,
                goal,
                adversary,
            }
        }
        9 => LegacyStimulusInfo::Index(reader.read_u16("index")?),
        value => {
            let offset = reader.offset();
            return Err(reader.invalid_value(
                offset.saturating_sub(4),
                "info_type",
                value,
                "stimulus information type 0 through 9 in a v48 stream",
            ));
        }
    };
    Ok(LegacyStimulus {
        to_whole_patrol,
        stimulus_type,
        info_type,
        owner,
        info,
    })
}

/// The noise origin's wire order is `x, y, level, sector`, unlike the
/// declaration order of [`LegacyStimulusPosition`].
fn read_noise_stimulus_position(
    reader: &mut LegacyReader<'_>,
) -> LegacyResult<LegacyStimulusPosition> {
    reader.scope("noise.origin", |reader| {
        Ok(LegacyStimulusPosition {
            x: reader.read_f32("x")?,
            y: reader.read_f32("y")?,
            level: reader.read_u16("level")?,
            sector: read_sector_ref(reader, "sector")?,
        })
    })
}

fn read_ai_ref_list(
    reader: &mut LegacyReader<'_>,
    field: &'static str,
    fingerprint: [u8; 16],
    maximum: usize,
) -> LegacyResult<Vec<LegacyAiElementRef>> {
    reader.scope(field, |reader| {
        reader.read_signature("fingerprint", fingerprint, "AI element-list fingerprint")?;
        let count = reader.read_count_u32("count", maximum)?;
        reader.read_list("items", count, |reader, item| {
            LegacyAiElementRef::read_field(reader, item, &())
        })
    })
}

fn ensure_count(
    reader: &mut LegacyReader<'_>,
    field: &'static str,
    count: usize,
    maximum: usize,
) -> LegacyResult<()> {
    if count <= maximum {
        Ok(())
    } else {
        let offset = reader.offset().saturating_sub(2);
        Err(reader.invalid_value(
            offset,
            field,
            count,
            "count no greater than caller-supplied limit",
        ))
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    use crate::legacy_save::test_support::with_reader;

    fn u16(bytes: &mut Vec<u8>, value: u16) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn u32(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn i32(bytes: &mut Vec<u8>, value: i32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn f32(bytes: &mut Vec<u8>, value: f32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn position(bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&POSITION_FINGERPRINT);
        f32(bytes, 1.25);
        f32(bytes, -2.5);
        u16(bytes, 3);
        u16(bytes, u16::MAX);
    }

    #[test]
    fn windows_ai_log_lines_consume_complete_aligned_aggregates() {
        let mut bytes = Vec::new();
        u32(&mut bytes, 2);
        i32(&mut bytes, 3);
        u16(&mut bytes, 17);
        u16(&mut bytes, 0xabcd);
        u32(&mut bytes, 120);
        i32(&mut bytes, 7);
        u16(&mut bytes, 110);
        u16(&mut bytes, 0x35e2);
        u32(&mut bytes, 999);

        with_reader(&bytes, |reader| {
            let decoded =
                read_ai_log_lines(reader, LegacySaveAbiProfile::RetailWindowsX86V48, 8).unwrap();
            assert_eq!(
                decoded,
                vec![
                    LegacyAiLogLine::RetailWindows {
                        log_type: 3,
                        info: 17,
                        alignment_padding: 0xabcd,
                        frame: 120,
                    },
                    LegacyAiLogLine::RetailWindows {
                        log_type: 7,
                        info: 110,
                        alignment_padding: 0x35e2,
                        frame: 999,
                    },
                ]
            );
            assert_eq!(reader.offset(), bytes.len() as u64);
        });
    }

    fn ai_list(bytes: &mut Vec<u8>, fingerprint: [u8; 16]) {
        bytes.extend_from_slice(&fingerprint);
        u32(bytes, 0);
    }

    fn minimal_common() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&AI_FINGERPRINT);
        position(&mut bytes);
        u16(&mut bytes, 4);
        u16(&mut bytes, 5);
        u32(&mut bytes, 0); // forbidden remarks
        u16(&mut bytes, 6);
        u32(&mut bytes, 0); // log lines
        u16(&mut bytes, 54_321); // owner
        for value in 10..16 {
            i32(&mut bytes, value);
        }
        bytes.push(17); // blood alcohol
        i32(&mut bytes, 18);
        bytes.push(19); // looks
        bytes.extend_from_slice(&[1, 0, 1]);
        u16(&mut bytes, 20);
        bytes.extend_from_slice(&PATH_FINGERPRINT);
        bytes.extend_from_slice(&[0, 0, 1]);
        u16(&mut bytes, u16::MAX);
        u16(&mut bytes, 0); // path history
        bytes.push(0); // no patrol path
        u16(&mut bytes, 0);
        bytes.push(0);
        for _ in 0..6 {
            u16(&mut bytes, 54_321);
        }
        bytes.push(0);
        u32(&mut bytes, 21);
        bytes.push(0);
        u32(&mut bytes, 22);
        u16(&mut bytes, 23);
        u16(&mut bytes, 24);
        for value in 25..30 {
            i32(&mut bytes, value);
        }
        for value in 30..35 {
            u16(&mut bytes, value);
        }
        bytes.push(0);
        u16(&mut bytes, 54_321);
        position(&mut bytes);
        position(&mut bytes);
        bytes.push(1);
        f32(&mut bytes, 35.0);
        f32(&mut bytes, 36.0);
        bytes.extend_from_slice(&[2, 0]);
        ai_list(&mut bytes, HUMANS_LIST_FINGERPRINT);
        ai_list(&mut bytes, NPC_LIST_FINGERPRINT);
        ai_list(&mut bytes, NPC_LIST_FINGERPRINT);
        bytes.extend_from_slice(&[0, 0, 0, 1, 0]);
        bytes.push(3);
        bytes.extend_from_slice(&[0, 0]);
        bytes.push(4);
        bytes.push(0);
        u32(&mut bytes, 0); // stimuli
        bytes.extend_from_slice(&[0, 1]);
        u16(&mut bytes, 37);
        u32(&mut bytes, 38);
        i32(&mut bytes, 39);
        u16(&mut bytes, u16::MAX); // door
        ai_list(&mut bytes, OBJECT_LIST_FINGERPRINT);
        for _ in 0..3 {
            u32(&mut bytes, u32::MAX);
        }
        bytes.extend_from_slice(&[0, 1]);
        ai_list(&mut bytes, NPC_LIST_FINGERPRINT);
        u16(&mut bytes, 40);
        bytes.push(0);
        i32(&mut bytes, 41);
        u16(&mut bytes, 0xab42); // low byte rand; high byte overlaps bool
        bytes.push(0); // authoritative forecast
        i32(&mut bytes, 43);
        u32(&mut bytes, 44);
        bytes.push(1);
        bytes.extend_from_slice(&RECONNAISSANCE_FINGERPRINT);
        i32(&mut bytes, 0);
        f32(&mut bytes, 45.0);
        f32(&mut bytes, 46.0);
        u32(&mut bytes, 0xdead_beef);
        u16(&mut bytes, 47);
        u16(&mut bytes, 0xd5a4);
        u16(&mut bytes, u16::MAX);
        u32(&mut bytes, 0);
        u32(&mut bytes, u32::MAX);
        bytes.push(0);
        bytes.extend_from_slice(&[0, 1]);
        u32(&mut bytes, u32::MAX);
        ai_list(&mut bytes, NPC_LIST_FINGERPRINT);
        ai_list(&mut bytes, NPC_LIST_FINGERPRINT);
        ai_list(&mut bytes, NPC_LIST_FINGERPRINT);
        bytes.push(0);
        u16(&mut bytes, 48);
        bytes
    }

    fn append_friendly_tail(bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&BONHOMIE_FINGERPRINT);
        u16(bytes, 51);
        u16(bytes, 52);
        bytes.push(1);
        u16(bytes, 53);
        bytes.push(0);
    }

    fn append_none_stimulus(bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&STIMULUS_FINGERPRINT);
        bytes.push(0);
        i32(bytes, 61);
        i32(bytes, 0);
        u32(bytes, u32::MAX);
    }

    #[test]
    fn noise_origin_reads_level_before_sector_without_changing_hint_order() {
        let mut bytes = Vec::new();

        bytes.extend_from_slice(&STIMULUS_FINGERPRINT);
        bytes.push(0);
        i32(&mut bytes, 2);
        i32(&mut bytes, 1);
        u32(&mut bytes, u32::MAX);
        f32(&mut bytes, 10.0);
        f32(&mut bytes, 11.0);
        u32(&mut bytes, 0xdead_beef);
        u16(&mut bytes, 12);
        u16(&mut bytes, 0xabcd);
        u16(&mut bytes, 13);
        f32(&mut bytes, 20.0);
        f32(&mut bytes, 21.0);
        u16(&mut bytes, 22);
        u16(&mut bytes, 23);
        i32(&mut bytes, 24);
        u16(&mut bytes, 25);
        u16(&mut bytes, 26);

        bytes.extend_from_slice(&STIMULUS_FINGERPRINT);
        bytes.push(0);
        i32(&mut bytes, 3);
        i32(&mut bytes, 4);
        u32(&mut bytes, u32::MAX);
        f32(&mut bytes, 30.0);
        f32(&mut bytes, 31.0);
        u16(&mut bytes, 32);
        u16(&mut bytes, 33);
        u16(&mut bytes, u16::MAX);
        u16(&mut bytes, 34);

        with_reader(&bytes, |reader| {
            let noise = read_stimulus(reader, 0).unwrap();
            let LegacyStimulusInfo::Noise { origin, .. } = noise.info else {
                panic!("expected noise stimulus");
            };
            assert_eq!(
                origin,
                LegacyStimulusPosition {
                    x: 20.0,
                    y: 21.0,
                    sector: LegacySectorRef(Some(23)),
                    level: 22,
                }
            );

            let hint = read_stimulus(reader, 0).unwrap();
            let LegacyStimulusInfo::Hint { position, .. } = hint.info else {
                panic!("expected hint stimulus");
            };
            assert_eq!(
                position,
                LegacyStimulusPosition {
                    x: 30.0,
                    y: 31.0,
                    sector: LegacySectorRef(Some(32)),
                    level: 33,
                }
            );
            assert_eq!(reader.offset(), bytes.len() as u64);
        });
    }

    fn append_seek_point(bytes: &mut Vec<u8>, seed: u16) {
        bytes.extend_from_slice(&SEEK_POINT_ALL_FINGERPRINT);
        f32(bytes, seed as f32);
        f32(bytes, -(seed as f32));
        u16(bytes, seed);
        u16(bytes, u16::MAX);
        u32(bytes, u32::from(seed) + 100);
        u32(bytes, 2);
        u16(bytes, seed + 1);
        u16(bytes, seed + 2);
        bytes.push(seed as u8);
        bytes.push(1);
        bytes.extend_from_slice(&SEEK_POINT_STATUS_FINGERPRINT);
        u32(bytes, u32::from(seed) + 200);
        bytes.push(seed as u8 + 3);
        bytes.push(0);
    }

    fn append_enemy_tail(bytes: &mut Vec<u8>, personal_points: bool) {
        bytes.extend_from_slice(&MALIGNITY_FINGERPRINT);
        append_none_stimulus(bytes);
        u32(bytes, 62);
        ai_list(bytes, OBJECT_LIST_FINGERPRINT);
        u32(bytes, 63);
        u16(bytes, 64);
        ai_list(bytes, OBJECT_LIST_FINGERPRINT);
        u16(bytes, 65);
        position(bytes);
        u16(bytes, 54_321);
        bytes.push(1);
        u16(bytes, 54_321);
        bytes.push(0);
        u32(bytes, 0); // search Charly positions
        u16(bytes, 66);
        u16(bytes, 67);
        u16(bytes, 68);
        bytes.push(2);
        u16(bytes, 69);
        ai_list(bytes, NPC_LIST_FINGERPRINT);
        ai_list(bytes, HUMANS_LIST_FINGERPRINT);
        ai_list(bytes, HUMANS_LIST_FINGERPRINT);
        bytes.extend_from_slice(&[0, 70, 71]);
        ai_list(bytes, HUMANS_LIST_FINGERPRINT);
        u16(bytes, (-2_i16) as u16);
        bytes.extend_from_slice(&[0, 1]);
        u32(bytes, 0); // ambush statuses
        u32(bytes, 0); // seek-point IDs
        u32(bytes, 6666);
        u32(bytes, 0); // directions before personal points
        bytes.push(personal_points as u8);
        if personal_points {
            append_seek_point(bytes, 72);
        }
        bytes.push(personal_points as u8);
        if personal_points {
            append_seek_point(bytes, 82);
        }
        position(bytes);
        u32(bytes, 0); // seek directions
        u32(bytes, 0); // beggar positions
        u16(bytes, 90);
        i32(bytes, 91);
        bytes.push(0);
        u16(bytes, 92);
        bytes.extend_from_slice(&[1, 0]);
        i32(bytes, 93);
        u16(bytes, 94);
        u16(bytes, 95);
        u16(bytes, 54_321);
        u16(bytes, 54_321);
        bytes.extend_from_slice(&[1, 0, 1]);
        u16(bytes, 54_321);
        bytes.extend_from_slice(&[0, 1]);
        position(bytes);
        u16(bytes, 96);
        bytes.push(1);
        position(bytes);
        i32(bytes, 97);
        i32(bytes, 98);
        bytes.push(0);
        u16(bytes, 99);
        u16(bytes, 100);
        ai_list(bytes, OBJECT_LIST_FINGERPRINT);
        ai_list(bytes, NPC_LIST_FINGERPRINT);
        ai_list(bytes, NPC_LIST_FINGERPRINT);
        u16(bytes, 54_321);
        u16(bytes, 54_321);
        ai_list(bytes, HUMANS_LIST_FINGERPRINT);
        u16(bytes, u16::MAX);
        u16(bytes, (-1_i16) as u16);
        u16(bytes, 101);
        bytes.extend_from_slice(&[0, 1]);
        u16(bytes, 666);
        u16(bytes, 666);
        u16(bytes, 102);
        u16(bytes, 103);
        bytes.push((-1_i8) as u8);
        bytes.push(0);
        u16(bytes, 104);
        i32(bytes, 105);
        i32(bytes, 106);
        i32(bytes, 107);
    }

    #[test]
    fn decodes_common_prefix_and_preserves_overlap_at_subclass_boundary() {
        let bytes = minimal_common();
        for kind in [LegacyLocalAiKind::Friendly, LegacyLocalAiKind::Enemy] {
            with_reader(&bytes, |reader| {
                let decoded =
                    LegacyLocalAiCommon::read(reader, &LegacyLocalAiDecodeConfig::for_kind(kind))
                        .unwrap();
                assert_eq!(decoded.subclass.kind, kind);
                assert_eq!(decoded.subclass.byte_offset, bytes.len() as u64);
                assert_eq!(decoded.next_macro_rand_word, 0xab42);
                assert_eq!(decoded.next_macro_rand, 0x42);
                assert_eq!(decoded.overlapped_forecast_byte, 0xab);
                assert!(!decoded.next_macro_rand_forecasted);
                assert_eq!(decoded.log_lines, Vec::<LegacyAiLogLine>::new());
            });
        }
    }

    #[test]
    fn requires_non_self_describing_subclass_kind_before_consuming_bytes() {
        let bytes = minimal_common();
        with_reader(&bytes, |reader| {
            let error = LegacyLocalAiCommon::read(
                reader,
                &LegacyLocalAiDecodeConfig {
                    kind: None,
                    limits: LegacyLocalAiLimits::default(),
                    abi_profile: LegacySaveAbiProfile::PortLinuxI386V48,
                },
            )
            .unwrap_err();
            assert_eq!(error.offset, 0);
            assert_eq!(error.field, "kind");
            assert_eq!(reader.offset(), 0);
        });
    }

    #[test]
    fn rejects_signature_count_and_truncation_at_the_exact_field() {
        let mut bad_signature = minimal_common();
        bad_signature[0] ^= 0xff;
        with_reader(&bad_signature, |reader| {
            let error = LegacyLocalAiCommon::read(
                reader,
                &LegacyLocalAiDecodeConfig::for_kind(LegacyLocalAiKind::Friendly),
            )
            .unwrap_err();
            assert_eq!(error.offset, 0);
            assert_eq!(error.field, "fingerprint");
        });

        let mut excessive_count = minimal_common();
        excessive_count[48..52].copy_from_slice(&1_u32.to_le_bytes());
        with_reader(&excessive_count, |reader| {
            let mut config = LegacyLocalAiDecodeConfig::for_kind(LegacyLocalAiKind::Friendly);
            config.limits.forbidden_remarks = 0;
            let error = LegacyLocalAiCommon::read(reader, &config).unwrap_err();
            assert_eq!(error.offset, 48);
            assert_eq!(error.field, "forbidden_remarks.count");
        });

        let truncated = &minimal_common()[..8];
        with_reader(truncated, |reader| {
            let error = LegacyLocalAiCommon::read(
                reader,
                &LegacyLocalAiDecodeConfig::for_kind(LegacyLocalAiKind::Enemy),
            )
            .unwrap_err();
            assert_eq!(error.offset, 0);
            assert_eq!(error.field, "fingerprint");
        });
    }

    #[test]
    fn decodes_complete_friendly_tail() {
        let mut bytes = minimal_common();
        append_friendly_tail(&mut bytes);
        with_reader(&bytes, |reader| {
            let decoded = LegacyLocalAiPayload::read(
                reader,
                &LegacyLocalAiDecodeConfig::for_kind(LegacyLocalAiKind::Friendly),
            )
            .unwrap();
            assert_eq!(reader.offset(), bytes.len() as u64);
            let LegacyLocalAiTail::Friendly(tail) = decoded.tail else {
                panic!("expected friendly tail");
            };
            assert_eq!(tail.fleeing_seen_enemy_counter, 51);
            assert_eq!(tail.last_talk_partner, LegacyAiElementRef(Some(53)));
            assert!(!tail.can_go_away);
        });
    }

    #[test]
    fn decodes_complete_enemy_tail_without_personal_seek_points() {
        let mut bytes = minimal_common();
        append_enemy_tail(&mut bytes, false);
        with_reader(&bytes, |reader| {
            let decoded = LegacyLocalAiPayload::read(
                reader,
                &LegacyLocalAiDecodeConfig::for_kind(LegacyLocalAiKind::Enemy),
            )
            .unwrap();
            assert_eq!(reader.offset(), bytes.len() as u64);
            let LegacyLocalAiTail::Enemy(tail) = decoded.tail else {
                panic!("expected enemy tail");
            };
            assert!(tail.personal_seek_point_1.is_none());
            assert!(tail.personal_seek_point_2.is_none());
            assert_eq!(tail.actual_seek_point_id, 6666);
            assert_eq!(tail.archery_point_increment, -1);
            assert_eq!(tail.known_enemy_strike_commands, [105, 106, 107]);
        });
    }

    #[test]
    fn decodes_complete_enemy_tail_with_both_personal_seek_points() {
        let mut bytes = minimal_common();
        append_enemy_tail(&mut bytes, true);
        with_reader(&bytes, |reader| {
            let decoded = LegacyLocalAiPayload::read(
                reader,
                &LegacyLocalAiDecodeConfig::for_kind(LegacyLocalAiKind::Enemy),
            )
            .unwrap();
            assert_eq!(reader.offset(), bytes.len() as u64);
            let LegacyLocalAiTail::Enemy(tail) = decoded.tail else {
                panic!("expected enemy tail");
            };
            let first = tail.personal_seek_point_1.unwrap();
            let second = tail.personal_seek_point_2.unwrap();
            assert_eq!(first.directions, vec![73, 74]);
            assert_eq!(first.repeated_frame_when_fully_interesting, 272);
            assert_eq!(second.directions, vec![83, 84]);
            assert_eq!(second.repeated_last_calculated_interest, 85);
        });
    }
}
