//! Per-frame movement inputs captured for parity divergence diagnostics.
//!
//! This is deliberately thread-local and opt-in, like the path and visibility
//! parity captures. Normal gameplay pays only one boolean branch per committed
//! motion step, and the capture is not simulation or save state.

use std::cell::RefCell;

use serde::Serialize;

use crate::coordinates::{MapBBox, MapPoint, MapVec, WorldPoint3D, WorldVec3D};
use crate::entity_id::EntityId;

thread_local! {
    static CAPTURE: RefCell<Option<Vec<ParityMovementStep>>> = const { RefCell::new(None) };
    static FLIGHT_CAPTURE: RefCell<Option<Vec<ParityFlightStep>>> = const { RefCell::new(None) };
    static MOVE_BOX_EXTRACTIONS: RefCell<Option<Vec<ParityMoveBoxExtraction>>> = const { RefCell::new(None) };
    static LATE_RETRANSLATIONS: RefCell<Option<Vec<EntityId>>> = const { RefCell::new(None) };
}

/// A float paired with its exact IEEE-754 representation. The decimal value
/// is convenient to read while `bits` makes one-ULP differences unambiguous.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct ParityFloat {
    pub value: f32,
    pub bits: u32,
}

impl From<f32> for ParityFloat {
    fn from(value: f32) -> Self {
        Self {
            value,
            bits: value.to_bits(),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct ParityPoint {
    pub x: ParityFloat,
    pub y: ParityFloat,
}

impl From<MapPoint> for ParityPoint {
    fn from(value: MapPoint) -> Self {
        Self {
            x: value.x.into(),
            y: value.y.into(),
        }
    }
}

impl From<MapVec> for ParityPoint {
    fn from(value: MapVec) -> Self {
        Self {
            x: value.x.into(),
            y: value.y.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct ParityPoint3 {
    pub x: ParityFloat,
    pub y: ParityFloat,
    pub z: ParityFloat,
}

impl From<WorldPoint3D> for ParityPoint3 {
    fn from(value: WorldPoint3D) -> Self {
        Self {
            x: value.x.into(),
            y: value.y.into(),
            z: value.z.into(),
        }
    }
}

impl From<WorldVec3D> for ParityPoint3 {
    fn from(value: WorldVec3D) -> Self {
        Self {
            x: value.x.into(),
            y: value.y.into(),
            z: value.z.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct ParityBox {
    pub min: Option<ParityPoint>,
    pub max: Option<ParityPoint>,
}

impl From<MapBBox> for ParityBox {
    fn from(value: MapBBox) -> Self {
        if value.is_somewhere() {
            Self {
                min: Some(MapPoint::new(value.x_min(), value.y_min()).into()),
                max: Some(MapPoint::new(value.x_max(), value.y_max()).into()),
            }
        } else {
            Self {
                min: None,
                max: None,
            }
        }
    }
}

/// Exact inputs and output of RHCOMMAND_MOVE's unauthorized-actor extraction.
/// This correction runs before ordinary PerformMotion and therefore needs its
/// own record rather than appearing as a movement step.
#[derive(Clone, Debug, Serialize)]
pub struct ParityMoveBoxExtraction {
    pub entity: EntityId,
    pub layer: u16,
    pub sector: Option<u16>,
    pub pathfinder_area: u16,
    pub original_box: ParityBox,
    pub expanded_box: ParityBox,
    pub expanded_motion_lines: Vec<u32>,
    pub authorized: bool,
    pub authorized_box: ParityBox,
    pub authorized_motion_lines: Vec<u32>,
    pub authorized_center: Option<ParityPoint>,
    pub corrected_position: Option<ParityPoint>,
}

/// Exact inputs and result of one distance-producing `PerformMotion` commit.
#[derive(Clone, Debug, Serialize)]
pub struct ParityMovementStep {
    pub entity: EntityId,
    pub order_action: String,
    pub animation: String,
    pub motion_method: String,
    pub pre_position: ParityPoint,
    pub goal: ParityPoint,
    pub cached_increment: ParityPoint,
    pub frame_distance_raw: ParityFloat,
    pub speed_factor: ParityFloat,
    pub speed_factor_applied: bool,
    pub direction_differs_from_goal: bool,
    pub effective_distance: ParityFloat,
    pub anti_collision_on: bool,
    pub deviated_before: bool,
    pub blocked_count_before: u16,
    pub requested_delta: ParityPoint,
    pub raw_committed_delta: ParityPoint,
    pub committed_delta: ParityPoint,
    pub post_position: ParityPoint,
    pub deviated_after: bool,
    pub blocked_count_after: u16,
    pub goal_reached_after_commit: bool,
}

/// Exact state around one Rust analogue of `RHSprite::PerformFlight`.
#[derive(Clone, Debug, Serialize)]
pub struct ParityFlightStep {
    pub entity: EntityId,
    pub phase: String,
    pub geometry: String,
    pub order_id: Option<u32>,
    pub order_type: Option<String>,
    pub frames_remaining_before: u16,
    pub frames_remaining_after: Option<u16>,
    pub entry_position: ParityPoint3,
    pub entry_position_map: ParityPoint,
    pub old_position: ParityPoint3,
    pub old_position_map: ParityPoint,
    pub goal: ParityPoint3,
    pub cached_increment: ParityPoint3,
    pub applied_increment: ParityPoint3,
    pub raw_post_position: ParityPoint3,
    pub raw_post_position_map: ParityPoint,
    pub motion_state: String,
    pub post_position: ParityPoint3,
    pub post_position_map: ParityPoint,
    pub snapped_to_goal: bool,
}

/// Start capturing movement commits on the current thread.
pub fn begin_parity_movement_capture() {
    CAPTURE.with(|capture| *capture.borrow_mut() = Some(Vec::new()));
    FLIGHT_CAPTURE.with(|capture| *capture.borrow_mut() = Some(Vec::new()));
    MOVE_BOX_EXTRACTIONS.with(|capture| *capture.borrow_mut() = Some(Vec::new()));
    LATE_RETRANSLATIONS.with(|capture| *capture.borrow_mut() = Some(Vec::new()));
}

pub fn parity_movement_capture_active() -> bool {
    CAPTURE.with(|capture| capture.borrow().is_some())
}

pub fn record_parity_move_box_extraction(extraction: ParityMoveBoxExtraction) {
    MOVE_BOX_EXTRACTIONS.with(|capture| {
        if let Some(extractions) = capture.borrow_mut().as_mut() {
            extractions.push(extraction);
        }
    });
}

/// Record a live movement whose orders were rebuilt after its actor's owner
/// slot by a patch/pathfinder-state transition.
pub fn record_parity_late_movement_retranslation(entity: EntityId) {
    LATE_RETRANSLATIONS.with(|capture| {
        if let Some(entities) = capture.borrow_mut().as_mut() {
            entities.push(entity);
        }
    });
}

/// Record one movement commit when parity capture is active.
pub fn record_parity_movement_step(step: ParityMovementStep) {
    CAPTURE.with(|capture| {
        if let Some(steps) = capture.borrow_mut().as_mut() {
            steps.push(step);
        }
    });
}

pub fn record_parity_flight_step(step: ParityFlightStep) {
    FLIGHT_CAPTURE.with(|capture| {
        if let Some(steps) = capture.borrow_mut().as_mut() {
            steps.push(step);
        }
    });
}

/// Finish and return the current thread's movement capture.
pub fn take_parity_movement_capture() -> Vec<ParityMovementStep> {
    CAPTURE.with(|capture| capture.borrow_mut().take().unwrap_or_default())
}

pub fn take_parity_flight_capture() -> Vec<ParityFlightStep> {
    FLIGHT_CAPTURE.with(|capture| capture.borrow_mut().take().unwrap_or_default())
}

pub fn take_parity_move_box_extractions() -> Vec<ParityMoveBoxExtraction> {
    MOVE_BOX_EXTRACTIONS.with(|capture| capture.borrow_mut().take().unwrap_or_default())
}

pub fn take_parity_late_movement_retranslations() -> Vec<EntityId> {
    LATE_RETRANSLATIONS.with(|capture| capture.borrow_mut().take().unwrap_or_default())
}
