//! Original v48 local-AI payload decoding.
//!
//! `RHArtificialIntelligence::SerializeThisAI` writes a common prefix and then
//! calls a virtual subclass serializer. The subclass identity is not present
//! in the stream, so callers must derive it from the owning element class.
//! The common reader exposes that exact virtual-call boundary for diagnostics;
//! [`LegacyLocalAiPayload`] continues through either complete v48 subclass.
//! No decoder scans for a later fingerprint or guesses byte counts.

use serde::{Deserialize, Serialize};

use crate::legacy_io::{LegacyReader, LegacyResult};

use super::LegacySaveAbiProfile;
use super::payload_base::{
    LegacyAiElementRef, LegacyElementRef, LegacyLineRef, LegacySectorRef, read_ai_element_ref,
    read_element_ref, read_line_ref, read_sector_ref,
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

const fn hex16(value: &str) -> [u8; 16] {
    let bytes = value.as_bytes();
    let mut result = [0; 16];
    let mut index = 0;
    while index < 16 {
        result[index] = (hex_nibble(bytes[index * 2]) << 4) | hex_nibble(bytes[index * 2 + 1]);
        index += 1;
    }
    result
}

const fn hex_nibble(value: u8) -> u8 {
    match value {
        b'0'..=b'9' => value - b'0',
        b'a'..=b'f' => value - b'a' + 10,
        _ => panic!("invalid fingerprint hex"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LegacyLocalAiKind {
    /// `RHArtificialBonhomie`, used by civilians.
    Friendly,
    /// `RHArtificialMalignity`, used by soldiers.
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
            forbidden_remarks: 65_535,
            log_lines: 65_535,
            path_history: 65_535,
            element_lists: 65_535,
            stimulus_queue: 65_535,
            reconnaissance_bodies: 65_535,
            enemy_positions: 65_535,
            ambush_statuses: 65_535,
            seek_point_ids: 65_535,
            seek_directions: 65_535,
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

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyAiPosition {
    pub x: f32,
    pub y: f32,
    pub level: u16,
    pub sector: LegacySectorRef,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
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
        /// The original first writes `sizeof(RHposition)`. On both audited
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyAiPathHistoryEntry {
    pub position_x: f32,
    pub position_y: f32,
    pub sector: LegacySectorRef,
    pub level: u16,
    pub direction: u8,
    pub distance: u16,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyAiPathStatus {
    pub current_waypoint_index: u8,
    pub last_waypoint_index: u8,
    pub forward_movement: bool,
    pub hiking_path_index: Option<u16>,
    pub history: Vec<LegacyAiPathHistoryEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LegacyAiLogLine {
    /// The Linux port's `CHECKENUM(newLogLine)` writes only the first four
    /// bytes of the `RHlogLine` aggregate.
    PortLinuxType(i32),
    /// Windows retail writes the complete naturally aligned 12-byte
    /// `RHlogLine` aggregate.
    RetailWindows {
        log_type: i32,
        info: u16,
        alignment_padding: u16,
        frame: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyReconnaissanceReport {
    pub report_type: i32,
    pub seek_position_x: f32,
    pub seek_position_y: f32,
    pub obsolete_sector_pointer: u32,
    pub seek_position_level: u16,
    pub alignment_padding: u16,
    pub seek_position_sector: LegacySectorRef,
    pub seen_bodies: Vec<LegacyElementRef>,
    pub charly: LegacyElementRef,
    pub charly_seen: bool,
}

/// The exact point immediately before the virtual subclass serializer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyLocalAiSubclassBoundary {
    pub kind: LegacyLocalAiKind,
    pub byte_offset: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacySeekPoint {
    pub position_x: f32,
    pub position_y: f32,
    pub position_level: u16,
    pub position_sector: LegacySectorRef,
    pub frame_when_fully_interesting: u32,
    pub directions: Vec<u16>,
    pub last_calculated_interest: u8,
    pub locked: bool,
    /// `SerializeAllData` ends by serializing the status a second time.
    pub repeated_frame_when_fully_interesting: u32,
    pub repeated_last_calculated_interest: u8,
    pub repeated_locked: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyEnemyAiTail {
    pub last_stimulus_dispatched_to_patrol: LegacyStimulus,
    pub frame_when_missed_charly: u32,
    pub heard_nets: Vec<LegacyAiElementRef>,
    pub frame_when_enemy_detected: u32,
    pub fleeing_seen_enemy_counter: u16,
    pub other_seen_ale: Vec<LegacyAiElementRef>,
    pub pc_gone_away_direction: u16,
    pub detected_something_there: LegacyAiPosition,
    pub missed_pc: LegacyAiElementRef,
    pub last_seek_direction_index: u8,
    pub beggar_to_examine: LegacyAiElementRef,
    pub pc_missed: bool,
    pub search_charly_way: Vec<LegacyAiPosition>,
    pub current_task_priority: u16,
    pub minimal_task_priority: u16,
    pub new_task_priority: u16,
    pub number_of_different_checkpoints: u8,
    pub delta_sorrow_level: u16,
    pub missed_in_action: Vec<LegacyAiElementRef>,
    pub other_bodies_to_examine: Vec<LegacyAiElementRef>,
    pub beggars_to_control: Vec<LegacyAiElementRef>,
    pub thirsty: bool,
    pub old_life_points: u8,
    pub initial_life_points: u8,
    pub them: Vec<LegacyAiElementRef>,
    pub old_odds: i16,
    pub position_change_locked_for_test: bool,
    pub ambush_point_array_reset: bool,
    pub ambush_point_statuses: Vec<i32>,
    pub seek_point_ids: Vec<u32>,
    pub actual_seek_point_id: u32,
    pub seek_point_view_directions_before_personal_points: Vec<u16>,
    pub personal_seek_point_1: Option<LegacySeekPoint>,
    pub personal_seek_point_2: Option<LegacySeekPoint>,
    pub seek_center: LegacyAiPosition,
    pub seek_point_view_directions: Vec<u16>,
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
    pub other_seen_money: Vec<LegacyAiElementRef>,
    pub money_fight_enemies: Vec<LegacyAiElementRef>,
    pub money_fight_victims: Vec<LegacyAiElementRef>,
    pub archer_behind_me: LegacyAiElementRef,
    pub shield_bearer_before_me: LegacyAiElementRef,
    pub already_seen_bodies: Vec<LegacyAiElementRef>,
    pub jump_line: LegacyLineRef,
    pub shield_bearer_direction: u16,
    pub phalanx_aborted: bool,
    pub changed_to_alert_path: bool,
    pub shooting_point: Option<LegacyShootingPointRef>,
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyLocalAiCommon {
    pub last_goto_destination: LegacyAiPosition,
    pub last_goto_flags: u16,
    pub stuck_counter: u16,
    pub forbidden_remarks: Vec<i32>,
    pub current_remark_flags: u16,
    /// `RHlogLine` is serialized through `CHECKENUM`, hence four bytes.
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
    pub path: LegacyAiPathStatus,
    pub has_patrol_path: bool,
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
    pub last_stimuli: [i32; 5],
    pub last_stimulus_multiplicities: [u16; 5],
    pub is_master: bool,
    pub master: LegacyAiElementRef,
    pub seek_position: LegacyAiPosition,
    pub alert_soldiers_point: LegacyAiPosition,
    pub first_try: bool,
    pub panic_center_x: f32,
    pub panic_center_y: f32,
    pub lasting_panic_runs: u8,
    pub directed_panic: bool,
    pub us: Vec<LegacyAiElementRef>,
    pub alerted_us: Vec<LegacyAiElementRef>,
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
    pub stimulus_queue: Vec<LegacyStimulus>,
    pub script_locked: bool,
    pub remember_events: bool,
    pub leave_house_number: u16,
    pub last_hint_actuality: u32,
    pub last_hint_subject: i32,
    pub door_index: Option<i16>,
    pub forgotten_objects: Vec<LegacyAiElementRef>,
    pub object_of_desire: LegacyElementRef,
    pub checkpoint_charly: LegacyElementRef,
    pub synchronize_charly: LegacyElementRef,
    pub inside_halt_method: bool,
    pub macro_started_this_frame: bool,
    pub synchronizing_actors: Vec<LegacyAiElementRef>,
    pub default_path_walking_flags: u16,
    pub looking_for_help_because_enemy_seen: bool,
    pub current_remark: i32,
    /// `mubNextMacroRand` is a UBYTE, but v48 erroneously serializes a UWORD
    /// starting at its address. The high byte overlaps the adjacent bool.
    pub next_macro_rand_word: u16,
    pub next_macro_rand: u8,
    pub overlapped_forecast_byte: u8,
    /// Serialized again after the overlapping UWORD and therefore authoritative.
    pub next_macro_rand_forecasted: bool,
    pub current_emoticon_type: i32,
    pub emoticon_expiration_date: u32,
    pub emoticon_has_expiration_date: bool,
    pub reconnaissance: LegacyReconnaissanceReport,
    pub knocked_out_in_money_fight: bool,
    pub got_beggar_trick: bool,
    pub patrol_chief: LegacyElementRef,
    pub patrol: Vec<LegacyAiElementRef>,
    pub missed_patrol_members: Vec<LegacyAiElementRef>,
    pub theoretical_patrol: Vec<LegacyAiElementRef>,
    pub patrol_stopped: bool,
    pub patrol_direction: u16,
    pub subclass: LegacyLocalAiSubclassBoundary,
}

impl LegacyLocalAiCommon {
    pub fn read(
        reader: &mut LegacyReader<'_>,
        config: &LegacyLocalAiDecodeConfig,
    ) -> LegacyResult<Self> {
        let kind = config.kind.ok_or_else(|| {
            let offset = reader.offset();
            reader.invalid_value(
                offset,
                "kind",
                "missing",
                "caller-supplied Friendly or Enemy local-AI kind",
            )
        })?;
        reader.read_signature(
            "fingerprint",
            AI_FINGERPRINT,
            "RHArtificialIntelligenceLocalAI fingerprint",
        )?;

        let last_goto_destination = read_ai_position(reader, "last_goto_destination")?;
        let last_goto_flags = reader.read_u16("last_goto_flags")?;
        let stuck_counter = reader.read_u16("stuck_counter")?;
        let forbidden_remarks =
            read_i32_list(reader, "forbidden_remarks", config.limits.forbidden_remarks)?;
        let current_remark_flags = reader.read_u16("current_remark_flags")?;
        let log_lines = read_ai_log_lines(reader, config.abi_profile, config.limits.log_lines)?;
        let owner = read_ai_element_ref(reader, "owner")?;

        let current_state = reader.read_i32("current_state")?;
        let old_state = reader.read_i32("old_state")?;
        let current_substate = reader.read_i32("current_substate")?;
        let current_music_alert_status = reader.read_i32("current_music_alert_status")?;
        let substate_at_last_timer_launch = reader.read_i32("substate_at_last_timer_launch")?;
        let attitude = reader.read_i32("attitude")?;
        let blood_alcohol = reader.read_u8("blood_alcohol")?;
        let initial_action = reader.read_i32("initial_action")?;
        let number_of_looks = reader.read_u8("number_of_looks")?;
        let can_move = reader.read_bool("can_move")?;
        let stop_before_end_of_path = reader.read_bool("stop_before_end_of_path")?;
        let use_max_norm_to_stop_before_end_of_path =
            reader.read_bool("use_max_norm_to_stop_before_end_of_path")?;
        let stop_before_end_of_path_distance =
            reader.read_u16("stop_before_end_of_path_distance")?;
        let path = read_path_status(reader, config.limits.path_history)?;
        let has_patrol_path = reader.read_bool("has_patrol_path")?;
        let macro_command_offset = has_patrol_path
            .then(|| reader.read_u16("macro_command_offset"))
            .transpose()?;
        let remaining_macro_bytes = reader.read_u16("remaining_macro_bytes")?;
        let macro_in_progress = reader.read_bool("macro_in_progress")?;

        let primary_target = read_ai_element_ref(reader, "primary_target")?;
        let friend_in_trouble = read_ai_element_ref(reader, "friend_in_trouble")?;
        let detected_body = read_ai_element_ref(reader, "detected_body")?;
        let interesting_object = read_ai_element_ref(reader, "interesting_object")?;
        let antagonist = read_ai_element_ref(reader, "antagonist")?;
        let last_stimulus_actor = read_ai_element_ref(reader, "last_stimulus_actor")?;
        let timer_is_running = reader.read_bool("timer_is_running")?;
        let timer_ring_frame = reader.read_u32("timer_ring_frame")?;
        let macro_timer_is_running = reader.read_bool("macro_timer_is_running")?;
        let macro_timer_ring_frame = reader.read_u32("macro_timer_ring_frame")?;
        let standing_around_timer = reader.read_u16("standing_around_timer")?;
        let sorrow_level = reader.read_u16("sorrow_level")?;
        let last_stimuli = read_i32_array(reader, "last_stimuli")?;
        let last_stimulus_multiplicities = read_u16_array(reader, "last_stimulus_multiplicities")?;
        let is_master = reader.read_bool("is_master")?;
        let master = read_ai_element_ref(reader, "master")?;
        let seek_position = read_ai_position(reader, "seek_position")?;
        let alert_soldiers_point = read_ai_position(reader, "alert_soldiers_point")?;
        let first_try = reader.read_bool("first_try")?;
        let panic_center_x = reader.read_f32("panic_center.x")?;
        let panic_center_y = reader.read_f32("panic_center.y")?;
        let lasting_panic_runs = reader.read_u8("lasting_panic_runs")?;
        let directed_panic = reader.read_bool("directed_panic")?;
        let us = read_ai_ref_list(
            reader,
            "us",
            HUMANS_LIST_FINGERPRINT,
            config.limits.element_lists,
        )?;
        let alerted_us = read_ai_ref_list(
            reader,
            "alerted_us",
            NPC_LIST_FINGERPRINT,
            config.limits.element_lists,
        )?;
        let staying_us = read_ai_ref_list(
            reader,
            "staying_us",
            NPC_LIST_FINGERPRINT,
            config.limits.element_lists,
        )?;
        let could_not_reach_point = reader.read_bool("could_not_reach_point")?;
        let already_on_point = reader.read_bool("already_on_point")?;
        let already_turned = reader.read_bool("already_turned")?;
        let likes_to_sit_around = reader.read_bool("likes_to_sit_around")?;
        let special_action = reader.read_bool("special_action")?;
        let remaining_tequila_gulps = reader.read_u8("remaining_tequila_gulps")?;
        let friends_are_alerted = reader.read_bool("friends_are_alerted")?;
        let stay_at_home = reader.read_bool("stay_at_home")?;
        let locks_flag_field = reader.read_u8("locks_flag_field")?;
        let was_busy = reader.read_bool("was_busy")?;
        let stimulus_queue = read_stimulus_list(
            reader,
            config.limits.stimulus_queue,
            config.limits.element_lists,
        )?;
        let script_locked = reader.read_bool("script_locked")?;
        let remember_events = reader.read_bool("remember_events")?;
        let leave_house_number = reader.read_u16("leave_house_number")?;
        let last_hint_actuality = reader.read_u32("last_hint_actuality")?;
        let last_hint_subject = reader.read_i32("last_hint_subject")?;
        let raw_door_index = reader.read_i16("door_index")?;
        let door_index = (raw_door_index != -1).then_some(raw_door_index);
        let forgotten_objects = read_ai_ref_list(
            reader,
            "forgotten_objects",
            OBJECT_LIST_FINGERPRINT,
            config.limits.element_lists,
        )?;
        let object_of_desire = read_element_ref(reader, "object_of_desire")?;
        let checkpoint_charly = read_element_ref(reader, "checkpoint_charly")?;
        let synchronize_charly = read_element_ref(reader, "synchronize_charly")?;
        let inside_halt_method = reader.read_bool("inside_halt_method")?;
        let macro_started_this_frame = reader.read_bool("macro_started_this_frame")?;
        let synchronizing_actors = read_ai_ref_list(
            reader,
            "synchronizing_actors",
            NPC_LIST_FINGERPRINT,
            config.limits.element_lists,
        )?;
        let default_path_walking_flags = reader.read_u16("default_path_walking_flags")?;
        let looking_for_help_because_enemy_seen =
            reader.read_bool("looking_for_help_because_enemy_seen")?;
        let current_remark = reader.read_i32("current_remark")?;
        let next_macro_rand_word = reader.read_u16("next_macro_rand_word")?;
        let next_macro_rand = next_macro_rand_word as u8;
        let overlapped_forecast_byte = (next_macro_rand_word >> 8) as u8;
        let next_macro_rand_forecasted = reader.read_bool("next_macro_rand_forecasted")?;
        let current_emoticon_type = reader.read_i32("current_emoticon_type")?;
        let emoticon_expiration_date = reader.read_u32("emoticon_expiration_date")?;
        let emoticon_has_expiration_date = reader.read_bool("emoticon_has_expiration_date")?;
        let reconnaissance = read_reconnaissance(reader, config.limits.reconnaissance_bodies)?;
        let knocked_out_in_money_fight = reader.read_bool("knocked_out_in_money_fight")?;
        let got_beggar_trick = reader.read_bool("got_beggar_trick")?;
        let patrol_chief = read_element_ref(reader, "patrol_chief")?;
        let patrol = read_ai_ref_list(
            reader,
            "patrol",
            NPC_LIST_FINGERPRINT,
            config.limits.element_lists,
        )?;
        let missed_patrol_members = read_ai_ref_list(
            reader,
            "missed_patrol_members",
            NPC_LIST_FINGERPRINT,
            config.limits.element_lists,
        )?;
        let theoretical_patrol = read_ai_ref_list(
            reader,
            "theoretical_patrol",
            NPC_LIST_FINGERPRINT,
            config.limits.element_lists,
        )?;
        let patrol_stopped = reader.read_bool("patrol_stopped")?;
        let patrol_direction = reader.read_u16("patrol_direction")?;
        let subclass = LegacyLocalAiSubclassBoundary {
            kind,
            byte_offset: reader.offset(),
        };

        Ok(Self {
            last_goto_destination,
            last_goto_flags,
            stuck_counter,
            forbidden_remarks,
            current_remark_flags,
            log_lines,
            owner,
            current_state,
            old_state,
            current_substate,
            current_music_alert_status,
            substate_at_last_timer_launch,
            attitude,
            blood_alcohol,
            initial_action,
            number_of_looks,
            can_move,
            stop_before_end_of_path,
            use_max_norm_to_stop_before_end_of_path,
            stop_before_end_of_path_distance,
            path,
            has_patrol_path,
            macro_command_offset,
            remaining_macro_bytes,
            macro_in_progress,
            primary_target,
            friend_in_trouble,
            detected_body,
            interesting_object,
            antagonist,
            last_stimulus_actor,
            timer_is_running,
            timer_ring_frame,
            macro_timer_is_running,
            macro_timer_ring_frame,
            standing_around_timer,
            sorrow_level,
            last_stimuli,
            last_stimulus_multiplicities,
            is_master,
            master,
            seek_position,
            alert_soldiers_point,
            first_try,
            panic_center_x,
            panic_center_y,
            lasting_panic_runs,
            directed_panic,
            us,
            alerted_us,
            staying_us,
            could_not_reach_point,
            already_on_point,
            already_turned,
            likes_to_sit_around,
            special_action,
            remaining_tequila_gulps,
            friends_are_alerted,
            stay_at_home,
            locks_flag_field,
            was_busy,
            stimulus_queue,
            script_locked,
            remember_events,
            leave_house_number,
            last_hint_actuality,
            last_hint_subject,
            door_index,
            forgotten_objects,
            object_of_desire,
            checkpoint_charly,
            synchronize_charly,
            inside_halt_method,
            macro_started_this_frame,
            synchronizing_actors,
            default_path_walking_flags,
            looking_for_help_because_enemy_seen,
            current_remark,
            next_macro_rand_word,
            next_macro_rand,
            overlapped_forecast_byte,
            next_macro_rand_forecasted,
            current_emoticon_type,
            emoticon_expiration_date,
            emoticon_has_expiration_date,
            reconnaissance,
            knocked_out_in_money_fight,
            got_beggar_trick,
            patrol_chief,
            patrol,
            missed_patrol_members,
            theoretical_patrol,
            patrol_stopped,
            patrol_direction,
            subclass,
        })
    }
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
                read_friendly_tail(reader).map(LegacyLocalAiTail::Friendly)
            }
            LegacyLocalAiKind::Enemy => {
                read_enemy_tail(reader, limits).map(LegacyLocalAiTail::Enemy)
            }
        }
    }
}

fn read_friendly_tail(reader: &mut LegacyReader<'_>) -> LegacyResult<LegacyFriendlyAiTail> {
    reader.scope("friendly", |reader| {
        reader.read_signature(
            "fingerprint",
            BONHOMIE_FINGERPRINT,
            "RHArtificialBonhomie fingerprint",
        )?;
        Ok(LegacyFriendlyAiTail {
            fleeing_seen_enemy_counter: reader.read_u16("fleeing_seen_enemy_counter")?,
            beggar_dont_talk_counter: reader.read_u16("beggar_dont_talk_counter")?,
            wants_to_talk: reader.read_bool("wants_to_talk")?,
            last_talk_partner: read_ai_element_ref(reader, "last_talk_partner")?,
            can_go_away: reader.read_bool("can_go_away")?,
        })
    })
}

fn read_enemy_tail(
    reader: &mut LegacyReader<'_>,
    limits: &LegacyLocalAiLimits,
) -> LegacyResult<LegacyEnemyAiTail> {
    reader.scope("enemy", |reader| {
        reader.read_signature(
            "fingerprint",
            MALIGNITY_FINGERPRINT,
            "RHArtificialMalignity fingerprint",
        )?;
        let last_stimulus_dispatched_to_patrol = reader
            .scope("last_stimulus_dispatched_to_patrol", |reader| {
                read_stimulus(reader, limits.element_lists)
            })?;
        let frame_when_missed_charly = reader.read_u32("frame_when_missed_charly")?;
        let heard_nets = read_ai_ref_list(
            reader,
            "heard_nets",
            OBJECT_LIST_FINGERPRINT,
            limits.element_lists,
        )?;
        let frame_when_enemy_detected = reader.read_u32("frame_when_enemy_detected")?;
        let fleeing_seen_enemy_counter = reader.read_u16("fleeing_seen_enemy_counter")?;
        let other_seen_ale = read_ai_ref_list(
            reader,
            "other_seen_ale",
            OBJECT_LIST_FINGERPRINT,
            limits.element_lists,
        )?;
        let pc_gone_away_direction = reader.read_u16("pc_gone_away_direction")?;
        let detected_something_there = read_ai_position(reader, "detected_something_there")?;
        let missed_pc = read_ai_element_ref(reader, "missed_pc")?;
        let last_seek_direction_index = reader.read_u8("last_seek_direction_index")?;
        let beggar_to_examine = read_ai_element_ref(reader, "beggar_to_examine")?;
        let pc_missed = reader.read_bool("pc_missed")?;
        let search_charly_way =
            read_ai_position_list(reader, "search_charly_way", limits.enemy_positions)?;
        let current_task_priority = reader.read_u16("current_task_priority")?;
        let minimal_task_priority = reader.read_u16("minimal_task_priority")?;
        let new_task_priority = reader.read_u16("new_task_priority")?;
        let number_of_different_checkpoints = reader.read_u8("number_of_different_checkpoints")?;
        let delta_sorrow_level = reader.read_u16("delta_sorrow_level")?;
        let missed_in_action = read_ai_ref_list(
            reader,
            "missed_in_action",
            NPC_LIST_FINGERPRINT,
            limits.element_lists,
        )?;
        let other_bodies_to_examine = read_ai_ref_list(
            reader,
            "other_bodies_to_examine",
            HUMANS_LIST_FINGERPRINT,
            limits.element_lists,
        )?;
        let beggars_to_control = read_ai_ref_list(
            reader,
            "beggars_to_control",
            HUMANS_LIST_FINGERPRINT,
            limits.element_lists,
        )?;
        let thirsty = reader.read_bool("thirsty")?;
        let old_life_points = reader.read_u8("old_life_points")?;
        let initial_life_points = reader.read_u8("initial_life_points")?;
        let them = read_ai_ref_list(
            reader,
            "them",
            HUMANS_LIST_FINGERPRINT,
            limits.element_lists,
        )?;
        let old_odds = reader.read_i16("old_odds")?;
        let position_change_locked_for_test =
            reader.read_bool("position_change_locked_for_test")?;
        let ambush_point_array_reset = reader.read_bool("ambush_point_array_reset")?;
        let ambush_point_statuses =
            read_i32_list(reader, "ambush_point_statuses", limits.ambush_statuses)?;
        let seek_point_ids = read_u32_list(reader, "seek_point_ids", limits.seek_point_ids)?;
        let actual_seek_point_id = reader.read_u32("actual_seek_point_id")?;
        let seek_point_view_directions_before_personal_points = read_u16_list(
            reader,
            "seek_point_view_directions_before_personal_points",
            limits.seek_directions,
        )?;
        let personal_seek_point_1 =
            read_optional_seek_point(reader, "personal_seek_point_1", limits)?;
        let personal_seek_point_2 =
            read_optional_seek_point(reader, "personal_seek_point_2", limits)?;
        let seek_center = read_ai_position(reader, "seek_center")?;
        let seek_point_view_directions =
            read_u16_list(reader, "seek_point_view_directions", limits.seek_directions)?;
        let positions_of_beggars_to_control = read_ai_position_list(
            reader,
            "positions_of_beggars_to_control",
            limits.enemy_positions,
        )?;
        let seek_flags = reader.read_u16("seek_flags")?;
        let forced_next_battle_decision = reader.read_i32("forced_next_battle_decision")?;
        let reset_battle_decision = reader.read_bool("reset_battle_decision")?;
        let synchronize_index = reader.read_u16("synchronize_index")?;
        let seen_dead_body = reader.read_bool("seen_dead_body")?;
        let seeking_charly = reader.read_bool("seeking_charly")?;
        let initial_view_cone = reader.read_i32("initial_view_cone")?;
        let repeated_seek_flags = reader.read_u16("repeated_seek_flags")?;
        let company_number = reader.read_u16("company_number")?;
        let left_combat_neighbour = read_ai_element_ref(reader, "left_combat_neighbour")?;
        let right_combat_neighbour = read_ai_element_ref(reader, "right_combat_neighbour")?;
        let attentive = reader.read_bool("attentive")?;
        let will_be_attentive = reader.read_bool("will_be_attentive")?;
        let forced_attentive = reader.read_bool("forced_attentive")?;
        let guarded_pc = read_ai_element_ref(reader, "guarded_pc")?;
        let tower_guard = reader.read_bool("tower_guard")?;
        let combat_trainer = reader.read_bool("combat_trainer")?;
        let gather_position = read_ai_position(reader, "gather_position")?;
        let gather_direction = reader.read_u16("gather_direction")?;
        let gather_position_instructed = reader.read_bool("gather_position_instructed")?;
        let officers_position = read_ai_position(reader, "officers_position")?;
        let previous_state = reader.read_i32("previous_state")?;
        let previous_substate = reader.read_i32("previous_substate")?;
        let reported_to_officer = reader.read_bool("reported_to_officer")?;
        let missed_soldier_timer = reader.read_u16("missed_soldier_timer")?;
        let old_money = reader.read_u16("old_money")?;
        let other_seen_money = read_ai_ref_list(
            reader,
            "other_seen_money",
            OBJECT_LIST_FINGERPRINT,
            limits.element_lists,
        )?;
        let money_fight_enemies = read_ai_ref_list(
            reader,
            "money_fight_enemies",
            NPC_LIST_FINGERPRINT,
            limits.element_lists,
        )?;
        let money_fight_victims = read_ai_ref_list(
            reader,
            "money_fight_victims",
            NPC_LIST_FINGERPRINT,
            limits.element_lists,
        )?;
        let archer_behind_me = read_ai_element_ref(reader, "archer_behind_me")?;
        let shield_bearer_before_me = read_ai_element_ref(reader, "shield_bearer_before_me")?;
        let already_seen_bodies = read_ai_ref_list(
            reader,
            "already_seen_bodies",
            HUMANS_LIST_FINGERPRINT,
            limits.element_lists,
        )?;
        let jump_line = read_line_ref(reader, "jump_line")?;
        let shield_bearer_direction = reader.read_u16("shield_bearer_direction")?;
        let phalanx_aborted = reader.read_bool("phalanx_aborted")?;
        let changed_to_alert_path = reader.read_bool("changed_to_alert_path")?;
        let shooting_sector = reader.read_u16("shooting_point.sector_index")?;
        let shooting_point = if shooting_sector == 666 {
            None
        } else {
            Some(LegacyShootingPointRef {
                sector_index: shooting_sector,
                point_index: reader.read_u16("shooting_point.point_index")?,
            })
        };
        let raw_archery_sector = reader.read_u16("archery_sector")?;
        let archery_sector = (raw_archery_sector != 666).then_some(raw_archery_sector);
        let archery_sector_index = reader.read_u16("archery_sector_index")?;
        let archery_point_index = reader.read_u16("archery_point_index")?;
        let archery_point_increment = reader.read_i8("archery_point_increment")?;
        let enemy_seen_below = reader.read_bool("enemy_seen_below")?;
        let enemy_had_this_elevation = reader.read_u16("enemy_had_this_elevation")?;
        let known_enemy_strike_commands = [
            reader.read_i32("known_enemy_strike_commands[0]")?,
            reader.read_i32("known_enemy_strike_commands[1]")?,
            reader.read_i32("known_enemy_strike_commands[2]")?,
        ];

        Ok(LegacyEnemyAiTail {
            last_stimulus_dispatched_to_patrol,
            frame_when_missed_charly,
            heard_nets,
            frame_when_enemy_detected,
            fleeing_seen_enemy_counter,
            other_seen_ale,
            pc_gone_away_direction,
            detected_something_there,
            missed_pc,
            last_seek_direction_index,
            beggar_to_examine,
            pc_missed,
            search_charly_way,
            current_task_priority,
            minimal_task_priority,
            new_task_priority,
            number_of_different_checkpoints,
            delta_sorrow_level,
            missed_in_action,
            other_bodies_to_examine,
            beggars_to_control,
            thirsty,
            old_life_points,
            initial_life_points,
            them,
            old_odds,
            position_change_locked_for_test,
            ambush_point_array_reset,
            ambush_point_statuses,
            seek_point_ids,
            actual_seek_point_id,
            seek_point_view_directions_before_personal_points,
            personal_seek_point_1,
            personal_seek_point_2,
            seek_center,
            seek_point_view_directions,
            positions_of_beggars_to_control,
            seek_flags,
            forced_next_battle_decision,
            reset_battle_decision,
            synchronize_index,
            seen_dead_body,
            seeking_charly,
            initial_view_cone,
            repeated_seek_flags,
            company_number,
            left_combat_neighbour,
            right_combat_neighbour,
            attentive,
            will_be_attentive,
            forced_attentive,
            guarded_pc,
            tower_guard,
            combat_trainer,
            gather_position,
            gather_direction,
            gather_position_instructed,
            officers_position,
            previous_state,
            previous_substate,
            reported_to_officer,
            missed_soldier_timer,
            old_money,
            other_seen_money,
            money_fight_enemies,
            money_fight_victims,
            archer_behind_me,
            shield_bearer_before_me,
            already_seen_bodies,
            jump_line,
            shield_bearer_direction,
            phalanx_aborted,
            changed_to_alert_path,
            shooting_point,
            archery_sector,
            archery_sector_index,
            archery_point_index,
            archery_point_increment,
            enemy_seen_below,
            enemy_had_this_elevation,
            known_enemy_strike_commands,
        })
    })
}

fn read_optional_seek_point(
    reader: &mut LegacyReader<'_>,
    field: &'static str,
    limits: &LegacyLocalAiLimits,
) -> LegacyResult<Option<LegacySeekPoint>> {
    reader.scope(field, |reader| {
        if reader.read_bool("present")? {
            read_seek_point(reader, limits).map(Some)
        } else {
            Ok(None)
        }
    })
}

fn read_seek_point(
    reader: &mut LegacyReader<'_>,
    limits: &LegacyLocalAiLimits,
) -> LegacyResult<LegacySeekPoint> {
    reader.read_signature(
        "fingerprint",
        SEEK_POINT_ALL_FINGERPRINT,
        "RHSeekPoint::SerializeAllData fingerprint",
    )?;
    let position_x = reader.read_f32("position.x")?;
    let position_y = reader.read_f32("position.y")?;
    let position_level = reader.read_u16("position.level")?;
    let position_sector = read_sector_ref(reader, "position.sector")?;
    let frame_when_fully_interesting = reader.read_u32("frame_when_fully_interesting")?;
    let directions = read_u16_list(reader, "directions", limits.seek_directions)?;
    let last_calculated_interest = reader.read_u8("last_calculated_interest")?;
    let locked = reader.read_bool("locked")?;
    reader.read_signature(
        "status.fingerprint",
        SEEK_POINT_STATUS_FINGERPRINT,
        "RHSeekPoint fingerprint",
    )?;
    let repeated_frame_when_fully_interesting =
        reader.read_u32("status.frame_when_fully_interesting")?;
    let repeated_last_calculated_interest = reader.read_u8("status.last_calculated_interest")?;
    let repeated_locked = reader.read_bool("status.locked")?;
    Ok(LegacySeekPoint {
        position_x,
        position_y,
        position_level,
        position_sector,
        frame_when_fully_interesting,
        directions,
        last_calculated_interest,
        locked,
        repeated_frame_when_fully_interesting,
        repeated_last_calculated_interest,
        repeated_locked,
    })
}

fn read_ai_position_list(
    reader: &mut LegacyReader<'_>,
    field: &'static str,
    maximum: usize,
) -> LegacyResult<Vec<LegacyAiPosition>> {
    reader.scope(field, |reader| {
        let count = reader.read_count_u32("count", maximum)?;
        let mut values = reserve(reader, "items", count)?;
        for index in 0..count {
            values.push(read_ai_position(reader, format!("items[{index}]"))?);
        }
        Ok(values)
    })
}

fn read_u16_list(
    reader: &mut LegacyReader<'_>,
    field: &'static str,
    maximum: usize,
) -> LegacyResult<Vec<u16>> {
    reader.scope(field, |reader| {
        let count = reader.read_count_u32("count", maximum)?;
        let mut values = reserve(reader, "items", count)?;
        for index in 0..count {
            values.push(reader.read_u16(format_args!("items[{index}]"))?);
        }
        Ok(values)
    })
}

fn read_u32_list(
    reader: &mut LegacyReader<'_>,
    field: &'static str,
    maximum: usize,
) -> LegacyResult<Vec<u32>> {
    reader.scope(field, |reader| {
        let count = reader.read_count_u32("count", maximum)?;
        let mut values = reserve(reader, "items", count)?;
        for index in 0..count {
            values.push(reader.read_u32(format_args!("items[{index}]"))?);
        }
        Ok(values)
    })
}

fn read_ai_log_lines(
    reader: &mut LegacyReader<'_>,
    abi_profile: LegacySaveAbiProfile,
    maximum: usize,
) -> LegacyResult<Vec<LegacyAiLogLine>> {
    reader.scope("log_lines", |reader| {
        let count = reader.read_count_u32("count", maximum)?;
        let mut values = reserve(reader, "items", count)?;
        for index in 0..count {
            values.push(reader.scope(format!("items[{index}]"), |reader| {
                Ok(match abi_profile {
                    LegacySaveAbiProfile::PortLinuxI386V48 => {
                        LegacyAiLogLine::PortLinuxType(reader.read_i32("log_type")?)
                    }
                    LegacySaveAbiProfile::RetailWindowsX86V48 => {
                        LegacyAiLogLine::RetailWindows {
                            log_type: reader.read_i32("log_type")?,
                            info: reader.read_u16("info")?,
                            // MSVC inserts two bytes before the ULONG frame.
                            // They are not behavior, but retaining them keeps
                            // the compatibility decoder lossless.
                            alignment_padding: reader.read_u16("alignment_padding")?,
                            frame: reader.read_u32("frame")?,
                        }
                    }
                })
            })?);
        }
        Ok(values)
    })
}

fn read_ai_position(
    reader: &mut LegacyReader<'_>,
    field: impl Into<String>,
) -> LegacyResult<LegacyAiPosition> {
    reader.scope(field, |reader| {
        reader.read_signature(
            "fingerprint",
            POSITION_FINGERPRINT,
            "RHArtificialIntelligencePosition fingerprint",
        )?;
        Ok(LegacyAiPosition {
            x: reader.read_f32("x")?,
            y: reader.read_f32("y")?,
            level: reader.read_u16("level")?,
            sector: read_sector_ref(reader, "sector")?,
        })
    })
}

fn read_path_status(
    reader: &mut LegacyReader<'_>,
    maximum_history: usize,
) -> LegacyResult<LegacyAiPathStatus> {
    reader.scope("path", |reader| {
        reader.read_signature(
            "fingerprint",
            PATH_FINGERPRINT,
            "RHPath::SerializeStatus fingerprint",
        )?;
        let current_waypoint_index = reader.read_u8("current_waypoint_index")?;
        let last_waypoint_index = reader.read_u8("last_waypoint_index")?;
        let forward_movement = reader.read_bool("forward_movement")?;
        let raw_hiking_path = reader.read_u16("hiking_path_index")?;
        let hiking_path_index = (raw_hiking_path != u16::MAX).then_some(raw_hiking_path);
        let count = reader.read_u16("history.count")? as usize;
        ensure_count(reader, "history.count", count, maximum_history)?;
        let mut history = reserve(reader, "history", count)?;
        for index in 0..count {
            history.push(reader.scope(format!("history[{index}]"), |reader| {
                Ok(LegacyAiPathHistoryEntry {
                    position_x: reader.read_f32("position.x")?,
                    position_y: reader.read_f32("position.y")?,
                    sector: read_sector_ref(reader, "sector")?,
                    level: reader.read_u16("level")?,
                    direction: reader.read_u8("direction")?,
                    distance: reader.read_u16("distance")?,
                })
            })?);
        }
        Ok(LegacyAiPathStatus {
            current_waypoint_index,
            last_waypoint_index,
            forward_movement,
            hiking_path_index,
            history,
        })
    })
}

fn read_stimulus_list(
    reader: &mut LegacyReader<'_>,
    maximum: usize,
    maximum_nested_list: usize,
) -> LegacyResult<Vec<LegacyStimulus>> {
    let count = reader.read_count_u32("stimulus_queue.count", maximum)?;
    let mut values = reserve(reader, "stimulus_queue", count)?;
    for index in 0..count {
        values.push(reader.scope(format!("stimulus_queue[{index}]"), |reader| {
            read_stimulus(reader, maximum_nested_list)
        })?);
    }
    Ok(values)
}

fn read_stimulus(
    reader: &mut LegacyReader<'_>,
    _maximum_nested_list: usize,
) -> LegacyResult<LegacyStimulus> {
    reader.read_signature(
        "fingerprint",
        STIMULUS_FINGERPRINT,
        "RHStimulus fingerprint",
    )?;
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
            let origin = read_stimulus_position(reader, "noise.origin")?;
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
        2 => LegacyStimulusInfo::Position(read_stimulus_position(reader, "position")?),
        3 => LegacyStimulusInfo::Human(read_ai_element_ref(reader, "human")?),
        4 => LegacyStimulusInfo::Hint {
            position: read_stimulus_position(reader, "hint.position")?,
            teller: read_ai_element_ref(reader, "hint.teller")?,
            seek_flags: reader.read_u16("hint.seek_flags")?,
        },
        5 => LegacyStimulusInfo::Object(read_ai_element_ref(reader, "object")?),
        6 => LegacyStimulusInfo::Stolen {
            object: read_ai_element_ref(reader, "stolen.object")?,
            thief: read_ai_element_ref(reader, "stolen.thief")?,
        },
        7 => LegacyStimulusInfo::Combat {
            enemy_position: read_stimulus_position(reader, "combat.enemy_position")?,
            actor: read_ai_element_ref(reader, "combat.actor")?,
        },
        8 => {
            let delay = reader.read_u16("door_combat.delay")?;
            let direction = reader.read_u16("door_combat.direction")?;
            let goal = read_stimulus_position(reader, "door_combat.goal")?;
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
                "RHstimulusInfoType 0 through 9 in a v48 stream",
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

fn read_stimulus_position(
    reader: &mut LegacyReader<'_>,
    field: &'static str,
) -> LegacyResult<LegacyStimulusPosition> {
    reader.scope(field, |reader| {
        Ok(LegacyStimulusPosition {
            x: reader.read_f32("x")?,
            y: reader.read_f32("y")?,
            sector: read_sector_ref(reader, "sector")?,
            level: reader.read_u16("level")?,
        })
    })
}

fn read_reconnaissance(
    reader: &mut LegacyReader<'_>,
    maximum_bodies: usize,
) -> LegacyResult<LegacyReconnaissanceReport> {
    reader.scope("reconnaissance", |reader| {
        reader.read_signature(
            "fingerprint",
            RECONNAISSANCE_FINGERPRINT,
            "RHReconnaissanceReport fingerprint",
        )?;
        let report_type = reader.read_i32("report_type")?;
        let seek_position_x = reader.read_f32("seek_position.x")?;
        let seek_position_y = reader.read_f32("seek_position.y")?;
        let obsolete_sector_pointer = reader.read_u32("seek_position.obsolete_sector_pointer")?;
        let seek_position_level = reader.read_u16("seek_position.level")?;
        let alignment_padding = reader.read_u16("seek_position.alignment_padding")?;
        let seek_position_sector = read_sector_ref(reader, "seek_position.sector")?;
        let count = reader.read_count_u32("seen_bodies.count", maximum_bodies)?;
        let mut seen_bodies = reserve(reader, "seen_bodies", count)?;
        for index in 0..count {
            seen_bodies.push(read_element_ref(
                reader,
                format_args!("seen_bodies[{index}]"),
            )?);
        }
        let charly = read_element_ref(reader, "charly")?;
        let charly_seen = reader.read_bool("charly_seen")?;
        Ok(LegacyReconnaissanceReport {
            report_type,
            seek_position_x,
            seek_position_y,
            obsolete_sector_pointer,
            seek_position_level,
            alignment_padding,
            seek_position_sector,
            seen_bodies,
            charly,
            charly_seen,
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
        let mut values = reserve(reader, "items", count)?;
        for index in 0..count {
            values.push(read_ai_element_ref(reader, format_args!("items[{index}]"))?);
        }
        Ok(values)
    })
}

fn read_i32_list(
    reader: &mut LegacyReader<'_>,
    field: &'static str,
    maximum: usize,
) -> LegacyResult<Vec<i32>> {
    reader.scope(field, |reader| {
        let count = reader.read_count_u32("count", maximum)?;
        let mut values = reserve(reader, "items", count)?;
        for index in 0..count {
            values.push(reader.read_i32(format_args!("items[{index}]"))?);
        }
        Ok(values)
    })
}

fn read_i32_array(reader: &mut LegacyReader<'_>, field: &'static str) -> LegacyResult<[i32; 5]> {
    reader.scope(field, |reader| {
        Ok([
            reader.read_i32("[0]")?,
            reader.read_i32("[1]")?,
            reader.read_i32("[2]")?,
            reader.read_i32("[3]")?,
            reader.read_i32("[4]")?,
        ])
    })
}

fn read_u16_array(reader: &mut LegacyReader<'_>, field: &'static str) -> LegacyResult<[u16; 5]> {
    reader.scope(field, |reader| {
        Ok([
            reader.read_u16("[0]")?,
            reader.read_u16("[1]")?,
            reader.read_u16("[2]")?,
            reader.read_u16("[3]")?,
            reader.read_u16("[4]")?,
        ])
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

fn reserve<T>(
    reader: &mut LegacyReader<'_>,
    field: &'static str,
    count: usize,
) -> LegacyResult<Vec<T>> {
    let offset = reader.offset();
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| reader.allocation_error(offset, field, count))?;
    Ok(values)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;
    use crate::sbfile::{SB_FILE_READ, SbFile};

    fn with_reader<T>(bytes: &[u8], read: impl FnOnce(&mut LegacyReader<'_>) -> T) -> T {
        let mut fixture = NamedTempFile::new().unwrap();
        fixture.write_all(bytes).unwrap();
        fixture.flush().unwrap();
        let path = fixture.path().to_string_lossy();
        let mut file = SbFile::open(&path, SB_FILE_READ).unwrap();
        read(&mut LegacyReader::new(&mut file))
    }

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
