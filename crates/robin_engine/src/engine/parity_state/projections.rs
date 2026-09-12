//! Stable parity JSON schemas. These are projections, never restored runtime state.

use super::{ParityEntityReference, ParityFloat};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub(super) struct Point2 {
    pub x: ParityFloat,
    pub y: ParityFloat,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Point3 {
    pub x: ParityFloat,
    pub y: ParityFloat,
    pub z: ParityFloat,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Bounds2 {
    pub min: Point2,
    pub max: Point2,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Door {
    pub kind: String,
    pub sector_out: i16,
    pub sector_in: i16,
    pub layer_out: u16,
    pub layer_in: u16,
    pub point_out: Point2,
    pub point_in: Point2,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Obstacle {
    pub kind: String,
    pub index: usize,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Position {
    pub computed_position: u8,
    pub computed_increment: u8,
    pub material: u32,
    pub posture: u32,
    pub old_posture: u32,
    pub direction: i16,
    pub direction_goal: i16,
    pub slow_turn_count: u8,
    pub direction_count: i8,
    pub layer: Option<u16>,
    pub layer_goal: Option<u16>,
    pub tolerance: ParityFloat,
    pub directional_tolerance: bool,
    pub accumulate_movement_map: bool,
    pub anti_collision_on: bool,
    pub goal_next_valid: bool,
    pub deviated: bool,
    pub door_direction: bool,
    pub reversed_movement: bool,
    pub blocked_count: u16,
    pub radius: ParityFloat,
    pub emergency_lying_box: bool,
    pub sector: Option<i16>,
    pub sector_goal: Option<i16>,
    pub door: Option<Door>,
    pub obstacle: Option<Obstacle>,
    pub target: Option<ParityEntityReference>,
    pub world: Point3,
    pub map: Point2,
    pub sprite: Point2,
    pub old_world: Point3,
    pub old_map: Point2,
    pub old_sprite: Point2,
    pub goal_map: Point2,
    pub goal_next_map: Point2,
    pub goal_world: Point3,
    pub increment: Point3,
    pub increment_map: Point2,
    pub accumulated_movement_map: Point2,
    pub forecasted_movement: Point3,
    pub move_box: Option<Bounds2>,
    pub blocked_box: Option<Bounds2>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct AnimationReplacement {
    pub from: u32,
    pub to: u32,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Sprite {
    pub row: u16,
    pub frame: u16,
    pub frame_count: u16,
    pub flight_countdown: u16,
    pub width: u16,
    pub height: u16,
    pub last_action: u32,
    pub last_processed_order_id: u32,
    pub masked: bool,
    pub alternate_profile: bool,
    pub action_done_frame: u16,
    pub action_done_counter: u16,
    pub last_sound_id: u16,
    pub behind_display_order_reference: bool,
    pub display_order_reference: Option<ParityEntityReference>,
    pub replacements: Vec<AnimationReplacement>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct SoundSource {
    pub kind: u8,
    pub id: u32,
    pub global: bool,
    pub inner_distance: u16,
    pub outer_distance: u16,
    pub noise_covering_distance: u16,
    pub inner_volume: u16,
    pub outer_volume: u16,
    pub shape: Vec<Point2>,
    pub altitude: u8,
    pub min_delay: u16,
    pub max_delay: u16,
    pub delay_stepping: u16,
    pub timer: u16,
    pub active: bool,
    pub ambience_enabled: bool,
}

#[derive(Serialize, Deserialize)]
pub(super) struct SeekPointStatus {
    pub frame_when_full_interest: u32,
    pub last_calculated_interest: u8,
    pub locked: bool,
}

#[derive(Serialize, Deserialize)]
pub(super) struct ArcherySector {
    pub num_owners: u16,
    pub point_owners: Vec<Option<ParityEntityReference>>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct ForbiddenRemark {
    pub remark: u32,
    pub flags: u16,
    pub speech_id: u32,
    pub guy_index: u16,
    pub bad_guy: bool,
    pub forbidden_till_frame: u32,
}

#[derive(Serialize, Deserialize)]
pub(super) struct GlobalAi {
    pub stupid_soldiers_cheat: bool,
    pub seek_points: Vec<SeekPointStatus>,
    pub archery_sectors: Vec<ArcherySector>,
    pub green_alert_soldiers: u16,
    pub yellow_alert_soldiers: u16,
    pub red_alert_soldiers: u16,
    pub overall_alert_status: u32,
    pub overall_villain_alert_status: u32,
    pub saved_random_seed: i64,
    pub forbidden_remarks: Vec<ForbiddenRemark>,
    pub current_speech_variant: u16,
}

/// Optional component projections are omitted, unlike optional references inside
/// components, which are emitted as explicit nulls. Keep these policies distinct.
#[derive(Serialize, Deserialize)]
pub(super) struct EntityRuntime<'a> {
    pub position: Position,
    pub sprite: Sprite,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtype: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub npc_ai: Option<NpcAi<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub human_continuation: Option<super::human_projections::HumanContinuation<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub human_structure: Option<super::human_projections::HumanStructure>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pc_tail: Option<super::human_projections::PcTail>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pc_core: Option<super::human_projections::PcCore<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pc_qa: Option<Vec<super::human_projections::PcQa>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pc_interface: Option<super::human_projections::PcInterface>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pc_portrait: Option<super::human_projections::PcPortrait>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct FloatBits {
    pub bits: u32,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Point3Bits {
    pub x: FloatBits,
    pub y: FloatBits,
    pub z: FloatBits,
}

#[derive(Serialize, Deserialize)]
pub(super) struct ShieldController {
    pub is_protected: bool,
    pub protected_pc: Option<ParityEntityReference>,
    pub danger_point: Point3Bits,
}

#[derive(Serialize, Deserialize)]
pub(super) struct SoundCompletion {
    pub source_index: u32,
    pub finish_frame: u32,
}

#[derive(Serialize, Deserialize)]
pub(super) struct AiPosition {
    pub map: Point2,
    pub sector: Option<i16>,
    pub layer: u16,
}

#[derive(Serialize, Deserialize)]
pub(super) struct NoiseOrigin {
    pub map: Point2,
    pub sector: Option<i16>,
    pub layer: Option<u16>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum StimulusInfo {
    None,
    Noise {
        origin: NoiseOrigin,
        noise_type: u32,
        volume: u16,
        elevation: u16,
    },
    Position {
        position: AiPosition,
    },
    Human {
        entity: ParityEntityReference,
    },
    Hint {
        position: AiPosition,
        teller: ParityEntityReference,
        seek_flags: u16,
    },
    Object {
        entity: ParityEntityReference,
    },
    Stolen {
        object: ParityEntityReference,
        thief: ParityEntityReference,
    },
    Combat {
        actor: ParityEntityReference,
        enemy_position: AiPosition,
    },
    DoorCombat {
        delay: u16,
        direction: u16,
        goal: AiPosition,
        adversary: Option<ParityEntityReference>,
    },
    Index {
        value: u16,
    },
}

#[derive(Serialize, Deserialize)]
pub(super) struct Stimulus {
    pub stimulus_type: u32,
    pub info_type: u8,
    pub owner: Option<ParityEntityReference>,
    pub to_whole_patrol: bool,
    pub info: StimulusInfo,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Line {
    pub a: Point2,
    pub b: Point2,
}

#[derive(Serialize, Deserialize)]
pub(super) struct SeekPoint<'a> {
    pub position: AiPosition,
    pub frame_when_full_interest: u32,
    pub directions: std::borrow::Cow<'a, [u16]>,
    pub last_calculated_interest: u8,
    pub locked: bool,
}

#[derive(Serialize, Deserialize)]
pub(super) struct PathHistory {
    pub position: AiPosition,
    pub direction: u8,
    pub distance: u16,
}

#[derive(Serialize, Deserialize)]
pub(super) struct PatrolPathStatus {
    pub current_waypoint_index: u8,
    pub last_waypoint_index: u8,
    pub forward: bool,
    pub hiking_path_index: Option<u16>,
    pub history: Vec<PathHistory>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct ShootingPoint {
    pub sector_index: u16,
    pub point_index: u16,
}

#[derive(Serialize, Deserialize)]
pub(super) struct FriendlyAi {
    pub kind: String,
    pub fleeing_seen_enemy_counter: u16,
    pub beggar_dont_talk_counter: u16,
    pub wants_to_talk: bool,
    pub last_talk_partner: Option<ParityEntityReference>,
    pub can_go_away: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub(super) enum NpcSubclass<'a> {
    Friendly(FriendlyAi),
    Enemy(EnemyAi<'a>),
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct NpcAiLastGoto {
    pub destination: AiPosition,
    pub flags: u16,
    pub stuck_counter: u16,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct NpcAiPathControl {
    pub stop_before_end: bool,
    pub use_max_norm: bool,
    pub stop_distance: u16,
    pub status: PatrolPathStatus,
    pub has_patrol_path: bool,
    pub macro_cursor: Option<usize>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct NpcAiMacro {
    pub remaining_bytes: u16,
    pub in_progress: bool,
    pub started_this_frame: bool,
    pub next_rand: u8,
    pub next_rand_forecasted: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct NpcAiTargets {
    pub primary: Option<ParityEntityReference>,
    pub friend_in_trouble: Option<ParityEntityReference>,
    pub detected_body: Option<ParityEntityReference>,
    pub interesting_object: Option<ParityEntityReference>,
    pub antagonist: Option<ParityEntityReference>,
    pub last_stimulus_actor: Option<ParityEntityReference>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct NpcAiTimers {
    pub running: bool,
    pub ring: u32,
    pub macro_running: bool,
    pub macro_ring: u32,
    pub standing_around: u16,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct NpcAiGroup {
    pub is_master: bool,
    pub master: Option<ParityEntityReference>,
    pub us: Vec<ParityEntityReference>,
    pub alerted_us: Vec<ParityEntityReference>,
    pub staying_us: Vec<ParityEntityReference>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct NpcAiPanic {
    pub center: Point2,
    pub lasting_runs: u8,
    pub directed: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct NpcAiMovementFailures {
    pub could_not_reach: bool,
    pub already_on_point: bool,
    pub already_turned: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct NpcAiLegacyContinuation {
    pub remaining_tequila_gulps: u8,
    pub last_hint_actuality: u32,
    pub last_hint_subject: u32,
    pub current_door: Option<Door>,
    pub looking_for_help_because_enemy_seen: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct NpcAiObjectMemory {
    pub forgotten: Vec<ParityEntityReference>,
    pub desire: Option<ParityEntityReference>,
    pub checkpoint_charly: Option<ParityEntityReference>,
    pub synchronize_charly: Option<ParityEntityReference>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct NpcAiEmoticon {
    pub r#type: u32,
    pub expiration: u32,
    pub has_expiration: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct NpcAiReconnaissance {
    pub report_type: u32,
    pub seek_position: AiPosition,
    pub seen_bodies: Vec<ParityEntityReference>,
    pub charly: Option<ParityEntityReference>,
    pub charly_seen: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct NpcAiPatrol {
    pub chief: Option<ParityEntityReference>,
    pub active: Vec<ParityEntityReference>,
    pub missed: Vec<ParityEntityReference>,
    pub theoretical: Vec<ParityEntityReference>,
    pub stopped: bool,
    pub direction: u16,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct NpcAi<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subclass: Option<NpcSubclass<'a>>,
    pub last_goto: NpcAiLastGoto,
    pub forbidden_remarks: std::borrow::Cow<'a, [u32]>,
    pub current_remark_flags: u16,
    pub owner: Option<ParityEntityReference>,
    pub state: u32,
    pub old_state: i32,
    pub substate: u32,
    pub music_alert: u32,
    pub timer_launch_substate: u32,
    pub attitude: u32,
    pub blood_alcohol: u8,
    pub initial_action: u32,
    pub number_of_looks: u8,
    pub can_move: bool,
    pub path_control: NpcAiPathControl,
    pub r#macro: NpcAiMacro,
    pub targets: NpcAiTargets,
    pub timers: NpcAiTimers,
    pub sorrow: u16,
    pub last_stimuli: [u32; 5],
    pub last_stimulus_multiplicities: [u16; 5],
    pub group: NpcAiGroup,
    pub seek_position: AiPosition,
    pub alert_soldiers_point: AiPosition,
    pub first_try: bool,
    pub panic: NpcAiPanic,
    pub movement_failures: NpcAiMovementFailures,
    pub likes_to_sit: bool,
    pub special_action: bool,
    pub friends_alerted: bool,
    pub stay_at_home: bool,
    pub locks: u8,
    pub was_busy: bool,
    pub stimulus_queue: Vec<Stimulus>,
    pub script_locked: bool,
    pub remember_events: bool,
    pub leave_house_number: u16,
    pub legacy_continuation: NpcAiLegacyContinuation,
    pub object_memory: NpcAiObjectMemory,
    pub inside_halt: bool,
    pub synchronizing_actors: Vec<ParityEntityReference>,
    pub default_path_flags: u16,
    pub current_remark: u32,
    pub emoticon: NpcAiEmoticon,
    pub knocked_out_in_money_fight: bool,
    pub got_beggar_trick: bool,
    pub reconnaissance: NpcAiReconnaissance,
    pub patrol: NpcAiPatrol,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct EnemyAiTaskPriorities {
    pub current: u16,
    pub minimal: u16,
    pub new: u16,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct EnemyAi<'a> {
    pub kind: String,
    pub frame_when_missed_charly: u32,
    pub frame_when_enemy_detected: u32,
    pub fleeing_seen_enemy_counter: u16,
    pub pc_gone_direction: u16,
    pub detected_something_there: AiPosition,
    pub missed_pc: Option<ParityEntityReference>,
    pub last_seek_direction_index: u8,
    pub beggar_to_examine: Option<ParityEntityReference>,
    pub pc_missed: bool,
    pub task_priorities: EnemyAiTaskPriorities,
    pub different_checkpoints: u8,
    pub delta_sorrow: u16,
    pub thirsty: bool,
    pub old_life_points: u8,
    pub initial_life_points: u8,
    pub old_odds: i16,
    pub position_change_locked_for_test: bool,
    pub heard_nets: Vec<ParityEntityReference>,
    pub other_seen_ale: Vec<ParityEntityReference>,
    pub search_charly_way: Vec<AiPosition>,
    pub missed_in_action: Vec<ParityEntityReference>,
    pub other_bodies_to_examine: Vec<ParityEntityReference>,
    pub beggars_to_control: Vec<ParityEntityReference>,
    pub them: Vec<ParityEntityReference>,
    pub ambush_point_array_reset: bool,
    pub ambush_point_status: Vec<u32>,
    pub my_seek_points: std::borrow::Cow<'a, [u16]>,
    pub personal_seek_point_1: Option<SeekPoint<'a>>,
    pub personal_seek_point_2: Option<SeekPoint<'a>>,
    pub seek_center: AiPosition,
    pub actual_seek_point: Option<u16>,
    pub seek_point_view_directions: std::borrow::Cow<'a, [u16]>,
    pub positions_of_beggars_to_control: Vec<AiPosition>,
    pub seek_flags: u16,
    pub seen_dead_body: bool,
    pub seeking_charly: bool,
    pub forced_next_battle_decision: u32,
    pub reset_battle_decision: bool,
    pub synchronize_index: u16,
    pub initial_view_cone: u32,
    pub company_number: u16,
    pub left_combat_neighbour: Option<ParityEntityReference>,
    pub right_combat_neighbour: Option<ParityEntityReference>,
    pub attentive: bool,
    pub will_be_attentive: bool,
    pub forced_attentive: bool,
    pub guarded_pc: Option<ParityEntityReference>,
    pub tower_guard: bool,
    pub combat_trainer: bool,
    pub gather_position: AiPosition,
    pub gather_direction: u16,
    pub gather_position_instructed: bool,
    pub officers_position: AiPosition,
    pub previous_state: i32,
    pub previous_substate: i32,
    pub reported_to_officer: bool,
    pub missed_soldier_timer: u16,
    pub old_money: u16,
    pub other_seen_money: Vec<ParityEntityReference>,
    pub money_fight_enemies: Vec<ParityEntityReference>,
    pub money_fight_victims: Vec<ParityEntityReference>,
    pub archer_behind_me: Option<ParityEntityReference>,
    pub shield_bearer_before_me: Option<ParityEntityReference>,
    pub already_seen_bodies: Vec<ParityEntityReference>,
    pub my_line_jump: Option<Line>,
    pub shield_bearer_direction: u16,
    pub phalanx_aborted: bool,
    pub changed_to_alert_path: bool,
    pub shooting_point: Option<ShootingPoint>,
    pub archery_sector: Option<u16>,
    pub archery_sector_index: u16,
    pub archery_point_index: u16,
    pub archery_point_increment: i8,
    pub enemy_seen_below: bool,
    pub enemy_had_this_elevation: u16,
    pub known_enemy_strike_commands: [i32; 3],
    pub last_stimulus_dispatched_to_patrol: Option<Stimulus>,
}
