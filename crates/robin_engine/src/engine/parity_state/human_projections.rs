//! Fixed human and playable-character diagnostic records. Optional references
//! serialize as null; omission is reserved for absent outer entity components.
use super::{
    ParityEntityReference, ParityFloat,
    projections::{Line, Point2, Point3},
};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

#[derive(Serialize, Deserialize)]
pub(super) struct HumanContinuation<'a> {
    pub already_detectable_body: bool,
    pub concussion_healing_timeout: u16,
    pub tiredness: u16,
    pub concussion: u16,
    pub parry_counter: u16,
    pub detectable_list_index: u16,
    pub invulnerable: bool,
    pub last_motion_was_step_back: bool,
    pub smalltalk_initiative: bool,
    pub received_smalltalk_initiative: bool,
    pub smalltalk_hint: u32,
    pub smalltalk_hint_opponent: Option<ParityEntityReference>,
    pub relative_fighting_ability: u16,
    pub hollow_man: bool,
    pub killed_by_accident: bool,
    pub stuck_under_nets_counter: u16,
    pub sword_strike_boredom: Cow<'a, [u16]>,
    pub carrier: Option<ParityEntityReference>,
    pub small_repulsive_radius: bool,
    pub hulk: Hulk,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Hulk {
    pub running: u32,
    pub time: u32,
    pub level: u16,
    pub direction: bool,
    pub speed: ParityFloat,
}

#[derive(Serialize, Deserialize)]
pub(super) struct HumanStructure {
    pub opponents: Vec<Opponent>,
    pub repulsive_point: RepulsivePoint,
    pub building: Option<i16>,
    pub shield: Shield,
    pub sword_sweep: SwordSweep,
    pub pending_shoots: Vec<SequenceReference>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Opponent {
    pub entity: ParityEntityReference,
    pub jump_line: Option<Line>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct RepulsivePoint {
    pub position: Point2,
    pub concave: bool,
    pub limit_left: Point2,
    pub limit_right: Point2,
    pub action_radius: ParityFloat,
    pub force_a: ParityFloat,
    pub force_b: ParityFloat,
    pub radius: ParityFloat,
    pub id: u32,
    pub affects_pcs: bool,
    pub affects_soldiers: bool,
    pub affects_civilians: bool,
    pub affects_animals: bool,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Shield {
    pub points: Vec<ShieldPoint>,
    pub top_plane: Plane,
    pub bottom_plane: Plane,
    pub box_3d: [ParityFloat; 6],
    pub ground_box: BoundingBox,
    pub screen_box: BoundingBox,
    pub on_ground: bool,
}

#[derive(Serialize, Deserialize)]
pub(super) struct ShieldPoint {
    pub obstacle: [ParityFloat; 4],
    pub polygon: Point2,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Plane {
    pub a: Point3,
    pub b: Point3,
    pub normal: Point3,
    pub origin: Point3,
    pub u: Point3,
    pub v: Point3,
    pub az: ParityFloat,
    pub bz: ParityFloat,
    pub dz: ParityFloat,
    pub d: ParityFloat,
}

#[derive(Serialize, Deserialize)]
pub(super) struct BoundingBox {
    pub top_left: Point2,
    pub bottom_right: Point2,
    pub bounds_are_set: bool,
}

#[derive(Serialize, Deserialize)]
pub(super) struct SwordSweep {
    pub victims: Vec<ParityEntityReference>,
    pub initial_angle: ParityFloat,
    pub current_angle: ParityFloat,
    pub final_angle: ParityFloat,
}

#[derive(Serialize, Deserialize)]
pub(super) struct SequenceReference {
    pub sequence: usize,
    pub element: usize,
}

#[derive(Serialize, Deserialize)]
pub(super) struct PcCore<'a> {
    pub work_icon: u32,
    pub campaign_description_index: u32,
    pub playable: bool,
    pub beam_me_index: i16,
    pub already_selected: bool,
    pub belt_seen: bool,
    pub feet_seen: bool,
    pub head_seen: bool,
    pub immortal: bool,
    pub fried_psykokwack: bool,
    pub list_index: u8,
    pub teleport_counter: u16,
    pub current_action: u32,
    pub saved_action: u32,
    pub disabled_actions: Cow<'a, [bool]>,
    pub disabled_actions_temp: Cow<'a, [bool]>,
    pub position_before_teleport: Point2,
}

#[derive(Serialize, Deserialize)]
pub(super) struct PcQa {
    pub special_count: u16,
    pub quickito: u32,
    pub titbit: Option<u32>,
    pub button: u16,
    pub interactor: Option<ParityEntityReference>,
    pub action_size: Option<usize>,
    pub seek_size: Option<usize>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct PcInterface {
    pub playable: bool,
    pub displayed: bool,
}

#[derive(Serialize, Deserialize)]
pub(super) struct PcPortrait {
    pub quantities: [u16; 3],
    pub two_buttons_mode: bool,
    pub displayed: bool,
    pub burned: bool,
    pub open: bool,
    pub life_level: ParityFloat,
    pub trumpet_enabled: bool,
    pub quick_icons: Vec<QuickIcon>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct QuickIcon {
    pub titbit: Option<u32>,
    pub running: bool,
}

#[derive(Serialize, Deserialize)]
pub(super) struct PcTail {
    pub carried: Option<ParityEntityReference>,
    pub carried_posture: u32,
    pub shield_danger_point: Point3,
    pub shield_protected: Option<ParityEntityReference>,
    pub shield_protector: Option<ParityEntityReference>,
    pub guard: Option<ParityEntityReference>,
    pub time_till_reinforcement: u32,
    pub last_ammo_dropping_position: Point2,
    pub last_dropped_ammo: Option<ParityEntityReference>,
    pub update_last_dropped_ammo: bool,
    pub last_dropping_direction: u8,
}
