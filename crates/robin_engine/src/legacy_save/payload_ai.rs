//! Original v48 local-AI payload decoding.
//!
//! `RHArtificialIntelligence::SerializeThisAI` writes a common prefix and then
//! calls a virtual subclass serializer. The subclass identity is not present
//! in the stream, so callers must derive it from the owning element class.
//! This module deliberately stops at that exact virtual-call boundary until
//! the complete malignity tail (including personal seek-point bodies) is
//! available; it never scans for a later fingerprint or guesses byte counts.

use serde::{Deserialize, Serialize};

use crate::legacy_io::{LegacyReader, LegacyResult};

use super::payload_base::{
    LegacyAiElementRef, LegacyElementRef, LegacySectorRef, read_ai_element_ref, read_element_ref,
    read_sector_ref,
};

const AI_FINGERPRINT: [u8; 16] = hex16("02c3dacf6a5a1868e649569740c7fe14");
const POSITION_FINGERPRINT: [u8; 16] = hex16("16171037069ea629a320950e904da3ba");
const STIMULUS_FINGERPRINT: [u8; 16] = hex16("fed36c329a78c5b76534124341270899");
const RECONNAISSANCE_FINGERPRINT: [u8; 16] = hex16("c99555a7400566f2e984f47860cf828b");
const PATH_FINGERPRINT: [u8; 16] = hex16("f2781c304bb147aa1defc89ab1033082");
const HUMANS_LIST_FINGERPRINT: [u8; 16] = hex16("e1edc9e0991a413e5577613783f4333d");
const NPC_LIST_FINGERPRINT: [u8; 16] = hex16("d36d2f762287f69bcb71a45982dde0ca");
const OBJECT_LIST_FINGERPRINT: [u8; 16] = hex16("ee2a8180604e52d16d2844512ba84d7f");

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
        }
    }
}

/// Non-self-describing facts supplied by the initialized mission.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyLocalAiDecodeConfig {
    pub kind: Option<LegacyLocalAiKind>,
    pub limits: LegacyLocalAiLimits,
}

impl LegacyLocalAiDecodeConfig {
    pub fn for_kind(kind: LegacyLocalAiKind) -> Self {
        Self {
            kind: Some(kind),
            limits: LegacyLocalAiLimits::default(),
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
pub struct LegacyLocalAiCommon {
    pub last_goto_destination: LegacyAiPosition,
    pub last_goto_flags: u16,
    pub stuck_counter: u16,
    pub forbidden_remarks: Vec<i32>,
    pub current_remark_flags: u16,
    /// `RHlogLine` is serialized through `CHECKENUM`, hence four bytes.
    pub log_lines: Vec<i32>,
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
        let log_lines = read_i32_list(reader, "log_lines", config.limits.log_lines)?;
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

fn read_ai_position(
    reader: &mut LegacyReader<'_>,
    field: &'static str,
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

// Remaining exact subclass grammars at `LegacyLocalAiSubclassBoundary`:
//
// Friendly (`RHArtificialBonhomie::SerializeExactlyThisAI`, v48):
// fingerprint; fleeing-seen-enemy counter u16; beggar-don't-talk counter
// u16; wants-to-talk bool; AI-local last-talk-partner u16; can-go-away bool.
//
// Enemy (`RHArtificialMalignity::SerializeExactlyThisAI`, v48):
// fingerprint; last dispatched stimulus; missed-Charly frame u32; object-ref
// list of heard nets; enemy-detected frame u32; fleeing counter u16;
// other-seen-ale object-ref list; gone-away direction u16; AI position;
// missed-PC AI ref; last seek direction u8; beggar AI ref; PC-missed bool;
// search-Charly AI-position list; three u16 priorities; checkpoint count u8;
// delta sorrow u16; missed-in-action NPC list; bodies and beggars human
// lists; thirsty bool; two life-point u8s; opponents human list; old odds
// i16; position-change lock bool; ambush reset bool; u32 ambush-status list;
// u32 seek-point-ID list and actual ID; u32 u16-direction list; two optional
// `RHSeekPoint::SerializeAllData` bodies; seek-center AI position; another
// u32/u16 direction list; beggar-control AI-position list; seek flags u16;
// forced battle decision i32; reset-decision bool; synchronize index u16;
// seen-dead-body and seeking-Charly bools; initial view cone i32; repeated
// seek flags u16; company number u16; left/right combat-neighbour AI refs;
// three attentive bools; guarded-PC AI ref; tower-guard and trainer bools;
// gather AI position, gather direction u16 and instructed bool; officer AI
// position; previous state/substate i32s; reported bool; missed-soldier timer
// and old money u16s; other-money object list; money-enemy and victim NPC
// lists; archer/shield-bearer AI refs; already-seen-bodies human list; line
// reference (u16 layer + i16 index); shield direction u16; phalanx-aborted and
// alert-path-changed bools; optional shooting-point encoded as sector u16
// (666 null) plus point u16; archery-sector u16 (666 null); archery sector and
// point indices u16; signed point increment i8; enemy-below bool; enemy
// elevation u16; and three known-enemy-strike command i32s. The optional
// seek-point serializer dependencies are why the decoder stops before the
// subclass today.

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
                assert_eq!(decoded.log_lines, Vec::<i32>::new());
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
}
