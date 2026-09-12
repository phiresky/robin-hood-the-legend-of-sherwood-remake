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
