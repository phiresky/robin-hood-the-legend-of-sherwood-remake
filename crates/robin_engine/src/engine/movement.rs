//! Movement ticking, pathfinding dispatch, and order processing.

use super::*;
use crate::coordinates::{MapBBox, MapPoint, MapVec};
use crate::element::EntityId;
use crate::entities::EntitySlots;
use crate::movement::ActiveMovement;
use crate::order::OrderType;
use crate::position_interface::vector_to_sector_0_to_15;
use crate::sprite::{FrameProgression, MotionMethod, MotionOrderContext, MotionState};

/// Per-owner lift translation snapshot consumed by movement Execute's live
/// animation derivation. Covers the lift cases of
/// `DetermineMovementAnimation`.
#[derive(Debug, Clone, Copy)]
enum LiftAnimContext {
    /// Upright posture in a lift sector.  Upwards and downwards animations
    /// are asserted equal for upright posture, so a single mapping covers
    /// both directions.
    Upright(crate::sector::LiftType),
    /// On-ladder / on-wall posture in a ladder or wall lift sector.  The
    /// per-frame upwards-vs-downwards pick comes from the dot product of
    /// the ladder vector (low point minus high point) with the actor's
    /// movement vector.  `ladder_dx` / `ladder_dy` is that ladder vector
    /// in map coordinates.
    OnClimb {
        lift_type: crate::sector::LiftType,
        lift_direction: i16,
        ladder_dx: f32,
        ladder_dy: f32,
    },
}

/// Apply the lift-sector portion of Original
/// `RHElementActor::DetermineMovementAnimation`.
///
/// The current actor sector is authoritative. In particular, an actor leaving
/// a lift translates its movement action before the door callback changes the
/// sector, while an actor approaching the lift from outside does not.
pub(super) fn determine_lift_movement_animation_for(
    entity: &crate::element::Entity,
    fast_grid: &crate::fast_find_grid::FastFindGrid,
    posture_after: crate::element::Posture,
    action: OrderType,
    destination: MapPoint,
) -> OrderType {
    use crate::element::Posture;

    let elem = entity.element_data();
    let posture = if posture_after == Posture::Undefined {
        elem.posture
    } else {
        posture_after
    };
    let Some(sector_handle) = elem.sector() else {
        return action;
    };
    let sector_number = crate::sector::SectorNumber::new(i16::from(sector_handle));
    let Some(sector) = fast_grid
        .level
        .sector_number_map
        .get(&sector_number)
        .and_then(|&index| fast_grid.level.sectors.get(index))
    else {
        return action;
    };
    let Some(lift_type) = sector.lift_type else {
        return action;
    };

    match posture {
        Posture::Upright => lift_type.translate_upright_action(action),
        Posture::OnWall | Posture::OnLadder => {
            if !matches!(
                (posture, lift_type),
                (Posture::OnWall, crate::sector::LiftType::Wall)
                    | (Posture::OnLadder, crate::sector::LiftType::Ladder)
            ) {
                tracing::warn!(
                    ?posture,
                    ?lift_type,
                    sector = %sector.sector_number,
                    "DetermineMovementAnimation: climb posture does not match lift sector"
                );
                return action;
            }
            let low = sector.low_exit_point.unwrap_or_else(|| {
                panic!(
                    "DetermineMovementAnimation: lift sector {} missing low exit point",
                    sector.sector_number
                )
            });
            let high = sector.high_exit_point.unwrap_or_else(|| {
                panic!(
                    "DetermineMovementAnimation: lift sector {} missing high exit point",
                    sector.sector_number
                )
            });
            let position = elem.position_map();
            let ladder_dx = low.x - high.x;
            let ladder_dy = low.y - high.y;
            let movement_dx = destination.x - position.x;
            let movement_dy = destination.y - position.y;
            let going_down = ladder_dx * movement_dx + ladder_dy * movement_dy >= 0.0;
            lift_type.translate_climb_action(action, going_down)
        }
        _ => action,
    }
}

#[cfg(test)]
mod line_crossing_eligibility_tests {
    use super::actor_line_crossing_eligible;
    use crate::element::Posture;

    #[test]
    fn wall_and_ladder_climbers_still_check_elevation_lines() {
        assert!(actor_line_crossing_eligible(Posture::OnWall, false, true));
        assert!(actor_line_crossing_eligible(Posture::OnLadder, false, true));
        assert!(!actor_line_crossing_eligible(Posture::Flying, false, true));
        assert!(!actor_line_crossing_eligible(Posture::OnWall, true, true));
        assert!(!actor_line_crossing_eligible(Posture::OnWall, false, false));
    }
}

/// Mobile geometry sampled at one actor's live creation-order slot.
///
/// Unlike the other immutable movement preparation, this must not escape the
/// owner boundary: an actor before a mobile sees its previous position and an
/// actor after the mobile sees the geometry translated by that master's
/// `Hourglass`.
struct LiveMobileGeometry {
    mobile_lines_by_layer: std::collections::BTreeMap<u16, Vec<crate::fast_find_grid::GridLine>>,
    mobile_points_by_layer: std::collections::BTreeMap<u16, Vec<crate::repulsive::RepulsivePoint>>,
    mobile_polygons_by_layer:
        std::collections::BTreeMap<u16, Vec<Vec<crate::coordinates::MapPoint>>>,
}

#[cfg(test)]
thread_local! {
    static LAST_MOBILE_CROSSING_INCREMENT: std::cell::Cell<Option<MapVec>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(super) fn take_last_mobile_crossing_increment() -> Option<MapVec> {
    LAST_MOBILE_CROSSING_INCREMENT.with(|increment| increment.take())
}

#[derive(Clone, Copy, Debug)]
pub(super) struct MovementOwnerSelection {
    pub seq_id: crate::sequence::SequenceId,
    pub elem_idx: usize,
    pub order_id: std::num::NonZeroU32,
}

fn order_uses_distance_motion(order: OrderType) -> bool {
    matches!(
        order,
        OrderType::WalkingUpright
            | OrderType::WalkingCrouched
            | OrderType::WalkingAlerted
            | OrderType::RunningUpright
            | OrderType::WalkingWithSword
            | OrderType::RunningWithSword
            | OrderType::WalkingStairs
            | OrderType::WalkingStairsAlerted
            | OrderType::RunningStairs
            | OrderType::WalkingSword
            | OrderType::WalkingBackwardsSword
            | OrderType::StrafingRightSword
            | OrderType::StrafingLeftSword
            | OrderType::WalkingShield
            | OrderType::WalkingBackwardsShield
            | OrderType::StrafingRightShield
            | OrderType::StrafingLeftShield
            | OrderType::WalkingWithCorpse
            | OrderType::WalkingCarryingOnShoulders
            | OrderType::ClimbingWallUp
            | OrderType::ClimbingWallDown
            | OrderType::ClimbingWallUpFast
            | OrderType::ClimbingWallDownFast
            | OrderType::ClimbingLadderUp
            | OrderType::ClimbingLadderDown
            | OrderType::ClimbingLadderUpAlerted
            | OrderType::ClimbingLadderDownAlerted
            | OrderType::ClimbingLadderUpFast
            | OrderType::ClimbingLadderDownFast
    )
}

/// Match `RHSprite::PerformMotion`: scale the sprite-frame distance by the
/// movement element's speed factor before applying the turn slowdown and its
/// minimum useful step. Direct transition orders call `PerformMotion` without
/// the element speed factor, while seek transitions route through
/// `RHElementActor::PerformSeek`, which passes the element factor explicitly.
fn scaled_motion_distance(
    frame_distance: f32,
    speed_factor: f32,
    apply_speed_factor: bool,
    direction_differs_from_goal: bool,
) -> f32 {
    let mut distance = frame_distance
        * if apply_speed_factor {
            speed_factor
        } else {
            1.0
        };
    if direction_differs_from_goal && distance > 0.0 {
        distance *= 0.6;
        if distance < 0.7 {
            distance = 0.7;
        }
    }
    distance
}

fn climb_lift_type(action: OrderType) -> Option<crate::sector::LiftType> {
    use crate::sector::LiftType;

    match action {
        OrderType::TransitionWaitingUprightClimbingWallUp
        | OrderType::ClimbingWallUp
        | OrderType::ClimbingWallDown
        | OrderType::TransitionClimbingWallUpWaitingCrouched
        | OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel
        | OrderType::TransitionWaitingCrouchedClimbingWallDown
        | OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel
        | OrderType::TransitionClimbingWallDownWaitingUpright
        | OrderType::ClimbingWallUpFast
        | OrderType::ClimbingWallDownFast => Some(LiftType::Wall),
        OrderType::TransitionWaitingUprightClimbingLadderUp
        | OrderType::TransitionWaitingUprightClimbingLadderUpAlerted
        | OrderType::TransitionClimbingLadderUpWaitingCrouched
        | OrderType::TransitionClimbingLadderUpWaitingUprightAlerted
        | OrderType::TransitionWaitingCrouchedClimbingLadderDown
        | OrderType::TransitionWaitingUprightClimbingLadderDownAlerted
        | OrderType::TransitionClimbingLadderDownWaitingUpright
        | OrderType::TransitionClimbingLadderDownWaitingUprightAlerted
        | OrderType::ClimbingLadderUp
        | OrderType::ClimbingLadderDown
        | OrderType::ClimbingLadderUpFast
        | OrderType::ClimbingLadderDownFast => Some(LiftType::Ladder),
        _ => None,
    }
}

fn is_fast_climb_action(action: OrderType) -> bool {
    matches!(
        action,
        OrderType::RunningStairs
            | OrderType::ClimbingWallUpFast
            | OrderType::ClimbingWallDownFast
            | OrderType::ClimbingLadderUpFast
            | OrderType::ClimbingLadderDownFast
    )
}

fn is_authored_climb_action(action: OrderType) -> bool {
    matches!(
        action,
        OrderType::ClimbingWallUp
            | OrderType::ClimbingWallDown
            | OrderType::ClimbingWallUpFast
            | OrderType::ClimbingWallDownFast
            | OrderType::ClimbingLadderUp
            | OrderType::ClimbingLadderDown
            | OrderType::ClimbingLadderUpAlerted
            | OrderType::ClimbingLadderDownAlerted
            | OrderType::ClimbingLadderUpFast
            | OrderType::ClimbingLadderDownFast
    )
}

fn sprite_motion_order_for_nonanimation(order: OrderType) -> OrderType {
    match order {
        // legacy implementation RHNONANIMATION_CLIMBING_*_FAST tokens are dispatch /
        // pathfinder speed tokens. RHElementActor handles them by
        // playing the normal climb animation row with RHMOTIONMETHOD_RUN.
        OrderType::RunningStairs => OrderType::WalkingStairs,
        OrderType::ClimbingWallUpFast => OrderType::ClimbingWallUp,
        OrderType::ClimbingWallDownFast => OrderType::ClimbingWallDown,
        OrderType::ClimbingLadderUpFast => OrderType::ClimbingLadderUp,
        OrderType::ClimbingLadderDownFast => OrderType::ClimbingLadderDown,
        other => other,
    }
}

fn rider_charge_point_in_quad(point: MapPoint, quad: [(f32, f32); 4]) -> bool {
    fn cross(ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
        ax * by - ay * bx
    }
    let mut positive = false;
    let mut negative = false;
    for index in 0..4 {
        let (x1, y1) = quad[index];
        let (x2, y2) = quad[(index + 1) % 4];
        let value = cross(x2 - x1, y2 - y1, point.x - x1, point.y - y1);
        positive |= value > 0.0;
        negative |= value < 0.0;
    }
    !(positive && negative)
}

fn is_galopp_decision_frame(current_frame: u16, frame_count: u16) -> bool {
    assert!(
        frame_count > 0,
        "selected RunningUpright rider-charge animation has no frames"
    );
    // Original compares WORD values in an arithmetic expression, so C++
    // promotes them to signed int. In particular, a one-frame animation's
    // midpoint expression is -1 rather than an unsigned underflow.
    let current_frame = i32::from(current_frame);
    let frame_count = i32::from(frame_count);
    current_frame == frame_count / 2 - 1 || current_frame == frame_count - 1
}

pub(super) fn door_click_polygon_at(doors: &[crate::gate::Door], click: MapPoint) -> Option<u32> {
    doors
        .iter()
        .enumerate()
        .find(|(_, door)| door.is_door() && door.click_polygon_contains(click.x, click.y))
        .map(|(idx, _)| idx as u32)
}

fn is_sword_movement_nonanimation(order: OrderType) -> bool {
    matches!(
        order,
        OrderType::WalkingWithSword | OrderType::RunningWithSword
    )
}

/// Match `RHElementActorHuman::DetermineMovementAnimation`: these are logical
/// dispatch tokens consumed by the Human Execute override, not sprite rows.
/// `FaceOpponent` chooses the concrete forward/backward/strafe sword animation
/// later, so sprite animation availability must not gate this rewrite.
fn sword_movement_dispatch_action(order: OrderType) -> OrderType {
    match order {
        OrderType::WalkingUpright | OrderType::WalkingWithCorpse => OrderType::WalkingWithSword,
        OrderType::RunningUpright => OrderType::RunningWithSword,
        _ => order,
    }
}

/// Match `SBGeoVector2D::Angle(vDisplacement, vDirection)` as used by
/// `RHElementActorHuman::FaceOpponent`: the signed angle is measured from the
/// movement displacement to the actor's facing vector.
fn combat_movement_angle(move_angle: f32, facing_angle: f32) -> f32 {
    let mut angle = facing_angle - move_angle;
    if angle < 0.0 {
        angle += 2.0 * std::f32::consts::PI;
    }
    if angle >= 2.0 * std::f32::consts::PI {
        angle -= 2.0 * std::f32::consts::PI;
    }
    angle
}

fn combat_directional_animation(
    action_state: crate::element::ActionState,
    angle: f32,
) -> OrderType {
    let unit = std::f32::consts::FRAC_PI_4;
    match action_state {
        crate::element::ActionState::MovingShield => {
            if angle < unit || angle >= 7.0 * unit {
                OrderType::WalkingShield
            } else if angle < 3.0 * unit {
                OrderType::StrafingRightShield
            } else if angle < 5.0 * unit {
                OrderType::WalkingBackwardsShield
            } else {
                OrderType::StrafingLeftShield
            }
        }
        _ => {
            if angle < unit || angle >= 7.0 * unit {
                OrderType::WalkingSword
            } else if angle < 3.0 * unit {
                OrderType::StrafingRightSword
            } else if angle < 5.0 * unit {
                OrderType::WalkingBackwardsSword
            } else {
                OrderType::StrafingLeftSword
            }
        }
    }
}

fn movement_execute_state_effect(
    order: OrderType,
    motion: MotionState,
) -> Option<(crate::element::Posture, crate::element::ActionState)> {
    use crate::element::{ActionState as AS, Posture as P};
    use crate::order::OrderType as OT;
    use crate::sprite::MotionState as MS;

    match (order, motion) {
        (
            OT::TransitionWalkingUprightWaitingUpright
            | OT::TransitionRunningUprightWaitingUpright
            | OT::TransitionWaitingUprightWalkingUpright
            | OT::TransitionSpecialWaitingUpright,
            MS::Done | MS::Terminated,
        ) => Some((P::Upright, AS::Waiting)),
        (OT::TransitionWaitingUprightSpecial, MS::Done | MS::Terminated) => {
            Some((P::Leisure, AS::Waiting))
        }
        (OT::TransitionWaitingUprightBoredWaitingUpright, MS::Done | MS::Terminated) => {
            Some((P::Upright, AS::Waiting))
        }
        (OT::TransitionWaitingUprightWaitingUprightBored, MS::Done | MS::Terminated) => {
            Some((P::Upright, AS::Bored))
        }
        (
            OT::TransitionCrouchingUp
            | OT::TransitionSittingWaitingUpright
            | OT::TransitionLeaningOutWaitingAlerted
            | OT::LoweringShield,
            MS::Done | MS::Terminated,
        ) => Some((P::Upright, AS::Waiting)),
        (OT::TransitionCrouchingDown, MS::Done | MS::Terminated) => {
            Some((P::Crouched, AS::Waiting))
        }
        (
            OT::TransitionWaitingUprightRunningUpright | OT::TransitionWalkingUprightRunningUpright,
            MS::Done | MS::Terminated,
        ) => Some((P::Upright, AS::MovingFast)),
        (OT::TransitionRunningUprightWalkingUpright, MS::Done | MS::Terminated) => {
            Some((P::Upright, AS::Moving))
        }
        (OT::WalkingUpright | OT::WalkingAlerted | OT::WalkingCrouched, MS::Start) => {
            Some((P::Upright, AS::Moving))
        }
        (OT::RunningUpright, MS::Start) => Some((P::Upright, AS::MovingFast)),
        (OT::WalkingWithSword, MS::Start) => Some((P::Upright, AS::MovingSword)),
        (OT::RunningWithSword, MS::Start) => Some((P::Upright, AS::MovingFastSword)),
        (OT::ClimbingWallUp | OT::ClimbingWallDown, MS::Start) => Some((P::OnWall, AS::Moving)),
        (
            OT::TransitionWaitingUprightClimbingLadderUp
            | OT::TransitionWaitingUprightClimbingLadderUpAlerted,
            MS::Done | MS::Terminated,
        ) => Some((P::OnLadder, AS::Moving)),
        _ => None,
    }
}

fn take_transition_distance_first_execute(transition_distance_continuation: &mut bool) -> bool {
    std::mem::take(transition_distance_continuation)
}

fn take_deferred_movement_state_start(deferred_movement_state_start: &mut bool) -> bool {
    std::mem::take(deferred_movement_state_start)
}

fn actor_line_crossing_eligible(
    posture: crate::element::Posture,
    human_is_carried: bool,
    inside_map: bool,
) -> bool {
    posture != crate::element::Posture::Flying && !human_is_carried && inside_map
}

#[inline]
fn stationary_motion_waits(speed: f32, tolerance_arrival: bool, distance: f32) -> bool {
    speed <= 0.0 && !tolerance_arrival && distance > f32::EPSILON
}

/// Motion state observed by the Original Execute arm after `PerformSeek`.
///
/// Entity-target `PerformSeek` consumes non-terminal sprite results and returns
/// `IN_PROGRESS`; point seeks return the raw result. This matters because the
/// caller's Execute switch must not observe either a raw `START` or `DONE`
/// while the seek wrapper remains active. Running upright is deliberately
/// excluded: its Original Execute arm sets `MOVING_FAST` unconditionally after
/// `PerformSeek`, irrespective of the returned motion state.
fn movement_execute_visible_motion(
    order: OrderType,
    motion: MotionState,
    reaches_goal_this_step: bool,
    entity_target_seek: bool,
) -> MotionState {
    if reaches_goal_this_step && matches!(motion, MotionState::Start) {
        return MotionState::Terminated;
    }
    if entity_target_seek
        && !matches!(motion, MotionState::Terminated)
        && !matches!(order, OrderType::RunningUpright)
    {
        return MotionState::InProgress;
    }
    motion
}

/// Original `mulWaitTime--` uses an unsigned 32-bit counter. A stationary
/// entity seek deliberately wraps zero to `UINT_MAX`; the signed refresh gate
/// then continues to regard the wrapped values as elapsed.
#[inline]
fn age_seek_refresh_wait(wait: u32) -> u32 {
    wait.wrapping_sub(1)
}

/// Number of times the Original actor `Execute` arm invokes `PerformSeek` for
/// one execution of this movement order when `RHMOVE_SEEK` is set.
///
/// The flag alone is not enough: authored wall and ladder orders retain it
/// while their Execute arms call `PerformMotion` directly. Conversely,
/// `RHNONANIMATION_RUNNING_STAIRS` literally calls `PerformSeek` twice.
#[inline]
fn perform_seek_calls_per_execute(order: OrderType) -> u32 {
    match order {
        OrderType::TransitionWalkingUprightWaitingUpright
        | OrderType::TransitionRunningUprightWaitingUpright
        | OrderType::TransitionWaitingUprightWalkingUpright
        | OrderType::TransitionWaitingUprightRunningUpright
        | OrderType::TransitionWalkingUprightRunningUpright
        | OrderType::TransitionRunningUprightWalkingUpright
        | OrderType::TransitionWaitingCrouchedWalkingCrouched
        | OrderType::TransitionWalkingCrouchedWaitingCrouched
        | OrderType::TransitionWalkingUprightWalkingCrouched
        | OrderType::TransitionWalkingCrouchedWalkingUpright
        | OrderType::TransitionRunningUprightWalkingCrouched
        | OrderType::TransitionWalkingCrouchedRunningUpright
        | OrderType::WalkingUpright
        | OrderType::RunningUpright
        | OrderType::WalkingCrouched
        | OrderType::WalkingAlerted
        | OrderType::WalkingStairs
        | OrderType::WalkingStairsAlerted
        | OrderType::WalkingCarryingOnShoulders
        | OrderType::WalkingWithCorpse
        | OrderType::WalkingWithSword
        | OrderType::RunningWithSword
        | OrderType::WalkingWithShield => 1,
        OrderType::RunningStairs => 2,
        _ => 0,
    }
}

fn original_final_path_metadata(
    raw_waypoint_count: usize,
    tolerance: f32,
    antagonist: Option<EntityId>,
) -> (f32, Option<EntityId>) {
    if raw_waypoint_count > 1 {
        (tolerance, antagonist)
    } else {
        (0.0, None)
    }
}

fn is_in_place_movement_transition(order: OrderType) -> bool {
    matches!(
        order,
        OrderType::TransitionWaitingUprightSpecial
            | OrderType::TransitionSpecialWaitingUpright
            | OrderType::TransitionWaitingUprightBoredWaitingUpright
            | OrderType::TransitionWaitingUprightWaitingUprightBored
            | OrderType::TransitionCrouchingUp
            | OrderType::TransitionCrouchingDown
            | OrderType::TransitionSittingWaitingUpright
            | OrderType::TransitionLeaningOutWaitingAlerted
            | OrderType::TransitionClimbingWallDownWaitingUpright
            | OrderType::StandingUp
            | OrderType::StandingUpSword
            | OrderType::StandingUpBow
            | OrderType::LoweringShield
    )
}

/// Result of [`EngineInner::advance_door_pass`].
///
/// Outcomes from draining the order list after a walk step terminates.
#[derive(Debug, Clone)]
pub(super) enum DoorPassAdvance {
    /// No active door pass existed when the state machine was asked to
    /// advance. This is a caller bug or a stale animation callback; it
    /// must not be treated as a completed pass.
    NoActive,
    /// A new `Walk` step is ready — the caller must push a walking
    /// order onto the actor's current sequence element to install the
    /// destination.  Movement tick resumes once the order is queued.
    Continue {
        destination: MapPoint,
        action: OrderType,
        reverse: bool,
        compute_direction: bool,
        /// Walk-step tolerance copied from the source
        /// [`DoorPassStep::Walk`].  Populated for the ladder/wall
        /// translators and `0.0` for stairs/building/default.
        tolerance: f32,
    },
    /// A `Transition` step was popped — the caller must push the
    /// included [`crate::order::Order`] onto the actor's current
    /// sequence element and *not* clear `active_door_pass` or signal
    /// arrival.  Door-pass advancement resumes when the transition
    /// animation completes (via [`crate::order::OrderCompletion::ResumeDoorPass`]).
    Paused {
        transition_order: crate::order::Order,
    },
    /// A non-animation `PassingDoor` action point is ready. It must be
    /// installed as the next real actor order so it consumes its own owner
    /// slot, just like the Original order chain.
    ActionPoint { order: crate::order::Order },
    /// No more steps remain; the door pass is complete and the caller
    /// should tear down path / active-movement state.
    Done {
        completed: Option<(crate::gate::DoorIndex, bool)>,
    },
}

// ─── Group-move formation helper ─────────────────────────────────────

/// Compute per-character destination points for a "mercenary"-style group
/// move around `click_point`.
///
/// The group's centroid is calculated, then each character's destination
/// is its current position translated so that the centroid lands on the
/// click point — preserving the relative formation of the group.
///
/// Returns a vector with the same length as `pc_positions`, each entry
/// being the destination for the PC at the matching index.  Returns an
/// empty vector if `pc_positions` is empty.
pub(crate) fn mercenary_formation_destinations(
    pc_positions: &[MapPoint],
    click_point: MapPoint,
) -> Vec<MapPoint> {
    if pc_positions.is_empty() {
        return Vec::new();
    }

    let n = pc_positions.len() as f32;
    let cx = pc_positions.iter().map(|p| p.x).sum::<f32>() / n;
    let cy = pc_positions.iter().map(|p| p.y).sum::<f32>() / n;

    pc_positions
        .iter()
        .map(|p| MapPoint::new(p.x - cx + click_point.x, p.y - cy + click_point.y))
        .collect()
}

/// Shape of the goal passed to [`EngineInner::build_gate_movement_sequence`].
///
/// Unifies the three goal flavours (point, door, line) into a single
/// builder; the function switches on this enum to pick the right
/// trailing-step shape.
#[derive(Debug, Clone, Copy)]
pub(crate) enum GoalShape {
    /// Point-goal. The actor walks to this map point after the last gate,
    /// retaining the caller's arrival tolerance (notably for AI `GoNear`).
    Point { point: MapPoint, tolerance: f32 },
    /// Entity-target seek goal.  The trailing MOVE keeps the target
    /// element, SEEK flag, and tolerance so arrival uses the same
    /// live target-distance predicate as a plain `Command::Seek`.
    Seek {
        point: MapPoint,
        target: EntityId,
        tolerance: f32,
    },
    /// Direct entity-target route built by `RHElementTarget::MouseClicked`.
    /// Unlike `Seek`, the target pointer is retained on each ordinary MOVE
    /// but `RHMOVE_SEEK` is not set and the actor's seek-refresh state is not
    /// touched.
    Target {
        point: MapPoint,
        target: EntityId,
        tolerance: f32,
    },
    /// Door-goal.  The gate path's final element is the goal door
    /// itself.  `far_side_point` describes the point the actor lands
    /// at after passing through.  When the far-side sector is a
    /// building, a `CHANGE_POSITION` teleport is emitted.
    Door {
        /// Index of the goal door in `self.script_domains.interactables.doors`.
        door_index: crate::gate::DoorIndex,
        /// The approach point (near side of the goal door).
        far_side_point: MapPoint,
        /// Far-side layer.
        far_side_layer: u16,
        /// True iff the goal-sector (far side) is a building.  When
        /// true the trailing step is a `CHANGE_POSITION` teleport after
        /// a random wait, not a plain walk to the far-side point.
        far_side_is_building: bool,
    },
    /// Line-goal.  The final MOVE uses the line's midpoint as its
    /// waypoint and carries `MoveFlags::LINE` + the line id so the
    /// actor's arrival check snaps to line tolerance.
    Line {
        /// Index of the goal line in `fast_grid.level.jump_lines`.
        line_index: crate::jump_line::JumpLineIndex,
        /// Midpoint of the line.  Used as the path target point during
        /// gate routing.
        midpoint: MapPoint,
        /// Arrival tolerance passed to the final line move.
        tolerance: f32,
    },
}

/// Timeout queue entry for a Move/Seek element whose pathfind failed.
/// When the pathfinder returns no path, the request is stamped with the
/// current universal frame counter and pushed onto this list.  After
/// 100 frames the element transitions to `Impossible` (and, for PCs,
/// the "unable to do something" speech line fires).
///
/// This is **not** a retry queue — the path is not re-dispatched during
/// the 100-frame window.  The element sits waiting (no orders, so the
/// actor's idle animation drives) until either the pathfinder produces
/// a result via external state changes, the element is cancelled (halt /
/// postpone), or the timeout elapses.
#[derive(
    Debug, Clone, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash,
)]
pub(crate) struct FailedPathRequest {
    pub(crate) owner: EntityId,
    pub(crate) seq_id: crate::sequence::SequenceId,
    pub(crate) elem_idx: usize,
    /// Universal frame counter at failure time.  Ages out at
    /// `first_fail_frame + 100`.
    pub(crate) first_fail_frame: u32,
    /// Exact `RHpathRequest` payload retained for save/replay parity.
    ///
    /// Runtime failures produced by an actual A* request and all imported v48
    /// failures populate this. A few Rust-only dispatch failures occur before
    /// an Original request could be constructed and therefore use `None`.
    /// TODO(path-dispatch): remove those synthetic timeout entries or give
    /// them an explicit non-Original state instead of sharing this queue.
    pub(crate) authoritative_request: Option<PendingPathRequest>,
}

impl FailedPathRequest {
    pub(crate) fn from_pending(request: PendingPathRequest, first_fail_frame: u32) -> Self {
        Self {
            owner: request.owner,
            seq_id: request.seq_id,
            elem_idx: request.elem_idx,
            first_fail_frame,
            authoritative_request: Some(request),
        }
    }

    pub(crate) fn synthetic(
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        first_fail_frame: u32,
    ) -> Self {
        Self {
            owner,
            seq_id,
            elem_idx,
            first_fail_frame,
            authoritative_request: None,
        }
    }
}

/// Snapshot of one legacy `RHpathRequest` waiting for A*.
///
/// Direct / straight moves never enter this queue. Requests that do need A*
/// snapshot their dispatch inputs here, then [`MovementContext`] resolves at
/// most one request at the original `RHEngine::ProcessPathRequests` point per
/// frame.
#[derive(
    Debug, Clone, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash,
)]
pub(crate) struct PendingPathRequest {
    /// This request was decoded from an Original v48 pending-path FIFO.
    ///
    /// Its movement element owns the exact serialized pre-path order queue,
    /// so completion must reuse the saved last waiting order in place.
    #[serde(default)]
    pub(crate) restored_from_v48: bool,
    pub(crate) owner: EntityId,
    pub(crate) seq_id: crate::sequence::SequenceId,
    pub(crate) elem_idx: usize,
    pub(crate) source: MapPoint,
    pub(crate) dest: MapPoint,
    pub(crate) layer: u16,
    /// Original `RHpathRequest::uwArea`; despite the name this is the actor's
    /// sector number and is converted to a graph-area index during A*.
    pub(crate) sector: u16,
    /// Exact serialized `RHpathRequest::uwSector`. Original request creation
    /// does not initialize this member and pathfinding never reads it, but v48
    /// saves nevertheless contain it.
    pub(crate) legacy_sector: u16,
    pub(crate) half_diagonal_idx: u16,
    pub(crate) use_first_point: bool,
    pub(crate) move_action: OrderType,
    pub(crate) speed: crate::pathfinder::PathFinderSpeed,
    pub(crate) reverse: bool,
    pub(crate) tolerance: f32,
    pub(crate) antagonist: Option<EntityId>,
    pub(crate) is_pass_door: bool,
    pub(crate) elem_flags: crate::sequence::MoveFlags,
    pub(crate) sword_movement_context: bool,
    pub(crate) is_fast: bool,
}

#[derive(
    Debug, Clone, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash,
)]
struct ProcessedPathRequest {
    request: PendingPathRequest,
    waypoints: Option<Vec<MapPoint>>,
}

/// Legacy path-request ordering plus the pathfinder's in-flight result.
///
/// `RHPathFinder::AddPathRequest` leaves queues of length zero or one alone.
/// From length two onward it stably sorts by speed, except that the in-flight
/// entry cannot be displaced. The original WAITING branch starts work but
/// returns no result; a later READY call delivers it and starts the next
/// request. `in_flight` preserves that one-call latency.
#[derive(
    Debug, Clone, Default, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash,
)]
pub(crate) struct PendingPathRequestQueue {
    waiting: Vec<PendingPathRequest>,
    in_flight: Option<ProcessedPathRequest>,
}

impl PendingPathRequestQueue {
    /// Restore the exact post-save FIFO. The Original writer excludes an
    /// ignored/in-flight head and writes the remaining list in order, so every
    /// deserialized request is waiting and no completion result is present.
    pub(crate) fn restore_v48_waiting(waiting: Vec<PendingPathRequest>) -> Self {
        Self {
            waiting,
            in_flight: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn v48_waiting(&self) -> &[PendingPathRequest] {
        &self.waiting
    }

    pub(crate) fn has_in_flight(&self) -> bool {
        self.in_flight.is_some()
    }

    fn enqueue(&mut self, request: PendingPathRequest) {
        let total = self.waiting.len() + usize::from(self.in_flight.is_some());
        if total < 2 {
            self.waiting.push(request);
            return;
        }

        // With no in-flight request, waiting[0] is the original list's
        // special first entry and is compared only after index 1. Once a
        // request is in flight, every waiting entry is priority-sortable.
        let first_sortable = usize::from(self.in_flight.is_none());
        let speed = request.speed as u8;
        if let Some(index) = (first_sortable..self.waiting.len())
            .rev()
            .find(|&index| self.waiting[index].speed as u8 <= speed)
        {
            self.waiting.insert(index + 1, request);
        } else {
            self.waiting.insert(0, request);
        }
    }

    fn take_completed(&mut self) -> Option<ProcessedPathRequest> {
        self.in_flight.take()
    }

    fn pop_to_start(&mut self) -> Option<PendingPathRequest> {
        (!self.waiting.is_empty()).then(|| self.waiting.remove(0))
    }

    fn set_in_flight(&mut self, request: PendingPathRequest, waypoints: Option<Vec<MapPoint>>) {
        debug_assert!(self.in_flight.is_none());
        self.in_flight = Some(ProcessedPathRequest { request, waypoints });
    }

    pub(super) fn retain_not_owned_by(&mut self, owner: EntityId) {
        self.waiting.retain(|request| request.owner != owner);
        if self
            .in_flight
            .as_ref()
            .is_some_and(|processed| processed.request.owner == owner)
        {
            self.in_flight = None;
        }
    }

    /// Mirror `RHPathFinder::CancelPathRequest`: cancelling the list head
    /// marks its eventual result stale instead of removing it, while later
    /// requests for the same actor are deleted immediately. The retained head
    /// still occupies one `ProcessPathRequests` result slot.
    pub(super) fn cancel_for_owner(&mut self, owner: EntityId) {
        if self.in_flight.is_some() {
            self.waiting.retain(|request| request.owner != owner);
            return;
        }
        let mut index = 1;
        while index < self.waiting.len() {
            if self.waiting[index].owner == owner {
                self.waiting.remove(index);
            } else {
                index += 1;
            }
        }
    }

    pub(super) fn clear(&mut self) {
        self.waiting.clear();
        self.in_flight = None;
    }
}

/// Disjoint owner borrows for the once-per-frame path scheduling barrier.
///
/// This context deliberately cannot reach scripts, campaign, player state, or
/// feedback. It only advances the pathfinder queue and classifies expired
/// failures; the root tick performs the cross-owner consequences (path order
/// installation and hero speech) immediately after each returned item.
pub(super) struct MovementContext<'a> {
    frame_counter: u32,
    world: &'a mut WorldState,
    orders: &'a mut OrderRuntime,
}

pub(super) enum CompletedPathWork {
    Ready {
        request: PendingPathRequest,
        waypoints: Vec<MapPoint>,
    },
    Failed(PendingPathRequest),
}

pub(super) struct ExpiredPathWork {
    pub(super) request: FailedPathRequest,
    pub(super) owner_is_pc: bool,
    pub(super) age: u32,
}

impl<'a> MovementContext<'a> {
    pub(super) fn new(
        frame_counter: u32,
        world: &'a mut WorldState,
        orders: &'a mut OrderRuntime,
    ) -> Self {
        Self {
            frame_counter,
            world,
            orders,
        }
    }

    /// Take the one result made ready by the previous scheduling call.
    /// Stale results are discarded exactly where the old root helper discarded
    /// them; no later request is completed at this barrier.
    pub(super) fn take_completed(&mut self) -> Option<CompletedPathWork> {
        let processed = self.orders.pending_path_requests.take_completed()?;
        let request = processed.request;
        let still_live = self
            .orders
            .sequence_manager
            .get_element(request.seq_id, request.elem_idx)
            .is_some_and(|elem| {
                elem.owner == Some(request.owner)
                    && elem.state == crate::sequence::SequenceState::InProgress
                    && elem.command == crate::element::Command::MoveWaiting
            });
        if !still_live {
            return None;
        }

        Some(match processed.waypoints {
            Some(waypoints) => CompletedPathWork::Ready { request, waypoints },
            None => CompletedPathWork::Failed(request),
        })
    }

    /// Start at most one queued request after the previous result has been
    /// applied by the root coordinator. Rust computes A* synchronously, but the
    /// result remains parked until the next frame's scheduling barrier.
    pub(super) fn start_next(&mut self, assets: &LevelAssets) {
        let Some(request) = self.orders.pending_path_requests.pop_to_start() else {
            return;
        };
        let still_live = self
            .orders
            .sequence_manager
            .get_element(request.seq_id, request.elem_idx)
            .is_some_and(|elem| {
                elem.owner == Some(request.owner)
                    && elem.state == crate::sequence::SequenceState::InProgress
                    && elem.command == crate::element::Command::MoveWaiting
            });
        if !still_live {
            return;
        }

        let waypoints = self.world.pathfinder.find_path(
            assets.pathfinder_graph.as_ref(),
            &self.world.fast_grid,
            request.layer,
            request.sector,
            request.half_diagonal_idx,
            request.source,
            request.dest,
            request.use_first_point,
        );
        self.orders
            .pending_path_requests
            .set_in_flight(request, waypoints);
    }

    /// Remove stale failures and return expired live entries in their stored
    /// order. Cross-owner effects are intentionally left to the tick
    /// coordinator so hero speech still precedes `element_impossible` for each
    /// request.
    pub(super) fn take_expired_failures(&mut self) -> Vec<ExpiredPathWork> {
        let mut still_waiting = Vec::new();
        let mut expired = Vec::new();
        for request in std::mem::take(&mut self.orders.failed_path_requests) {
            let still_live = self
                .orders
                .sequence_manager
                .get_element(request.seq_id, request.elem_idx)
                .is_some_and(|element| {
                    element.owner == Some(request.owner)
                        && element.state == crate::sequence::SequenceState::InProgress
                        && element.command == crate::element::Command::MoveWaiting
                });
            if !still_live {
                continue;
            }

            let age = self.frame_counter.saturating_sub(request.first_fail_frame);
            if age <= 100 {
                still_waiting.push(request);
                continue;
            }

            let owner_is_pc = self
                .world
                .entities
                .get(request.owner)
                .is_some_and(Entity::is_pc);
            expired.push(ExpiredPathWork {
                request,
                owner_is_pc,
                age,
            });
        }
        self.orders.failed_path_requests = still_waiting;
        expired
    }
}

/// Outcome of [`EngineInner::try_dispatch_move_path`], the unified
/// pathfind-and-populate pipeline invoked from the hourglass Move
/// dispatch.
#[derive(Debug)]
pub(crate) enum MovePathOutcome {
    /// Path found, orders populated, actor's `active_movement` + action
    /// state set, element transitioned to `InProgress`.  Caller has
    /// nothing left to do.
    Success,
    /// The move requires A* and has entered the legacy one-completion-per-
    /// frame request queue.
    Pending,
    /// Dispatch could not submit the move (for example, source extraction
    /// failed). The caller applies the existing failure handling.
    Failed,
    /// The entity slot is empty or the element vanished mid-dispatch.
    /// Caller should mark the element `Impossible`.
    ActorGone,
}

impl GoalShape {
    /// The point used for pathfinding / the final MOVE's destination.
    pub(crate) fn goal_point(&self) -> MapPoint {
        match *self {
            GoalShape::Point { point, .. } => point,
            GoalShape::Seek { point, .. } => point,
            GoalShape::Target { point, .. } => point,
            GoalShape::Door { far_side_point, .. } => far_side_point,
            GoalShape::Line { midpoint, .. } => midpoint,
        }
    }
}

/// Source adaptation when an actor is currently straddling a gate.
///
/// When the actor's current door is non-null, the path source is
/// rewritten to the gate's far-side point / sector / layer based on the
/// actor's door direction.
///
/// Returns `None` when the actor is not in a gate (callers should use
/// the raw `position_map` / `sector` / `layer`).
pub(crate) fn adapt_source_to_current_door(
    doors: &[crate::gate::Door],
    door_handle: crate::position_interface::DoorHandle,
    door_direction: bool,
) -> Option<(MapPoint, u16, u16)> {
    if door_handle.is_null() {
        return None;
    }
    let door = doors.get(door_handle.0 as usize)?;
    // door_direction true → use the "in" side of the door as the
    // source; false → use the "out" side.
    if door_direction {
        Some((door.point_in, u16::from(door.sector_in), door.layer_in))
    } else {
        Some((door.point_out, u16::from(door.sector_out), door.layer_out))
    }
}

/// Legacy `GetDoor()` source state for route construction.
///
/// Rust keeps an executing translated pass in `ActorData` rather than always
/// mirroring it into `PositionInterface`. Prefer that live pass so commands
/// issued while crossing a door start from the committed far side.
pub(crate) fn current_door_for_route_source(
    entity: &crate::element::Entity,
) -> (crate::position_interface::DoorHandle, bool) {
    entity
        .actor_data()
        .and_then(|actor| actor.active_door_pass.as_ref())
        .map(|pass| {
            (
                crate::position_interface::DoorHandle(pass.door_index.0),
                pass.position_direct,
            )
        })
        .unwrap_or_else(|| {
            let position = entity.position_iface();
            (position.get_door(), position.get_door_direction())
        })
}

/// Radius for circular dispatch (one third of [`GROUP_LIMIT_MAX`]).
const CIRCULAR_DISPATCH_RADIUS: f32 = 60.0;

/// Maximum centroid-to-member distance for mercenary formation to apply.
/// When any member is farther than this from the centroid, fall back to
/// circular dispatch.
const GROUP_LIMIT_MAX: f32 = 180.0;

fn force_sword_movement_for_sequence(seq: &mut crate::sequence::Sequence) {
    for elem in &mut seq.elements {
        if let crate::sequence::SequenceElementData::Movement { flags, .. } = &mut elem.data {
            *flags |= crate::sequence::MoveFlags::FORCE_SWORD_MOVEMENT;
        }
    }
}

/// Compute per-character destination points using circular distribution.
///
/// The circular dispatch fallback when the group is too spread out for
/// the mercenary formation.
///
/// Characters are arranged in a circle around `click_point`. Each
/// unassigned character picks the nearest available slot; when multiple
/// characters want the same slot, the one farthest from the click gets it
/// (the "worst placed" heuristic). The loop repeats until all characters
/// are assigned.
pub(crate) fn circular_dispatch_destinations(
    pc_positions: &[MapPoint],
    click_point: MapPoint,
) -> Vec<MapPoint> {
    let n = pc_positions.len();
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![click_point];
    }

    // Generate candidate positions in a circle around click_point.
    // Each candidate is `(0, -CIRCULAR_DISPATCH_RADIUS)` rotated by
    // `(i * TWO_PI / n)`.
    let candidates: Vec<MapPoint> = (0..n)
        .map(|i| {
            let angle = i as f32 * std::f32::consts::TAU / n as f32;
            MapPoint::new(
                click_point.x + angle.sin() * CIRCULAR_DISPATCH_RADIUS,
                click_point.y - angle.cos() * CIRCULAR_DISPATCH_RADIUS,
            )
        })
        .collect();

    let mut result = vec![click_point; n];
    let mut assigned = vec![false; n];
    let mut candidate_taken = vec![false; candidates.len()];

    // Iterative assignment with conflict resolution.
    loop {
        // Each unassigned character picks its nearest untaken candidate.
        // Store (character_idx, sq_dist) per candidate.
        let mut claims: Vec<Vec<(usize, f32)>> = vec![Vec::new(); candidates.len()];

        for (ci, &pos) in pc_positions.iter().enumerate() {
            if assigned[ci] {
                continue;
            }
            let mut best_k = None;
            let mut best_d = f32::INFINITY;
            for (ki, &cand) in candidates.iter().enumerate() {
                if candidate_taken[ki] {
                    continue;
                }
                let dx = pos.x - cand.x;
                let dy = pos.y - cand.y;
                let d = dx * dx + dy * dy;
                if d < best_d {
                    best_d = d;
                    best_k = Some(ki);
                }
            }
            if let Some(ki) = best_k {
                claims[ki].push((ci, best_d));
            }
        }

        let mut any_assigned = false;
        for (ki, claimants) in claims.iter().enumerate() {
            match claimants.len() {
                0 => {}
                1 => {
                    let (ci, _) = claimants[0];
                    result[ci] = candidates[ki];
                    assigned[ci] = true;
                    candidate_taken[ki] = true;
                    any_assigned = true;
                }
                _ => {
                    // Multiple characters want this candidate.
                    // Give it to the "worst-placed" claimant — the one
                    // whose distance to the contested slot is largest
                    // (per-claimant distance to the slot, not distance
                    // to the click point).
                    let worst = claimants
                        .iter()
                        .max_by(|(_, da), (_, db)| {
                            da.partial_cmp(db).unwrap_or(std::cmp::Ordering::Equal)
                        })
                        .unwrap()
                        .0;
                    result[worst] = candidates[ki];
                    assigned[worst] = true;
                    candidate_taken[ki] = true;
                    any_assigned = true;
                }
            }
        }

        if !any_assigned || assigned.iter().all(|&a| a) {
            break;
        }
    }

    // Any unassigned characters get the click point itself.
    result
}

/// Build the line-jump click-sequence shape:
///
/// 1. move to the selected source jump-line,
/// 2. execute the jump command from source to associated destination,
/// 3. move from the landing side to the original clicked point.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_line_jump_click_sequence(
    owner: EntityId,
    action: OrderType,
    source_line_idx: crate::jump_line::JumpLineIndex,
    source_line: &crate::jump_line::JumpLine,
    destination_line_idx: crate::jump_line::JumpLineIndex,
    click_point: MapPoint,
    click_layer: u16,
    speed_factor: f32,
) -> crate::sequence::Sequence {
    use crate::element::Command;
    use crate::sequence::{
        Field, FieldValue, MoveFlags, Sequence, SequenceElement, SequenceElementData,
    };

    let mut seq = Sequence::new();

    let mut move_to_line = SequenceElement::new_movement(1, Command::Move, Some(owner), action);
    move_to_line.data = SequenceElementData::Movement {
        destination: source_line.get_middle_point(),
        layer: source_line.layer,
        sector: None,
        gate_id: None,
        line_id: Some(source_line_idx),
        element: None,
        flags: MoveFlags::LINE | MoveFlags::TO_JUMP,
        tolerance: 0.0,
        direction: 0,
        action,
        speed_factor,
        post_seek_sequence: None,
    };
    seq.append_element(move_to_line);

    let mut jump = SequenceElement::new_generic(2, Command::JumpCmd, Some(owner));
    jump.set_property(Field::JumplineSource, FieldValue::LineId(source_line_idx));
    jump.set_property(
        Field::JumplineDestination,
        FieldValue::LineId(destination_line_idx),
    );
    seq.append_element(jump);

    let mut final_move = SequenceElement::new_movement(3, Command::Move, Some(owner), action);
    final_move.data = SequenceElementData::Movement {
        destination: click_point,
        layer: click_layer,
        sector: None,
        gate_id: None,
        line_id: None,
        element: None,
        // Original appends the post-jump click tail with no movement flags.
        // In particular it is a normal-priority move, so a later click may
        // interrupt it while the actor is still finishing this route.
        flags: MoveFlags::empty(),
        tolerance: 0.0,
        direction: 0,
        action,
        speed_factor,
        post_seek_sequence: None,
    };
    seq.append_element(final_move);

    seq
}

impl EngineInner {
    /// Rebuild Rust's derived active-movement latch after loading an Original
    /// save. Original keeps the executing movement in `mpSequenceElement`;
    /// Rust additionally caches its sequence identity for owner-local
    /// movement and anti-collision work.
    pub(crate) fn restore_loaded_active_movements(&mut self) {
        let active =
            self.orders
                .sequence_manager
                .sequences_iter()
                .flat_map(|sequence| {
                    sequence.elements.iter().enumerate().filter_map(
                        move |(element_index, element)| {
                            (element.state == crate::sequence::SequenceState::InProgress
                                && element.data.is_movement())
                            .then_some((element.owner?, sequence.id, element_index))
                        },
                    )
                })
                .collect::<Vec<_>>();
        let mut owners = std::collections::BTreeSet::new();
        for (owner, sequence_id, element_index) in active {
            assert!(
                owners.insert(owner),
                "loaded actor {owner:?} owns multiple in-progress movement elements"
            );
            self.world
                .entities
                .get_mut(owner)
                .and_then(Entity::actor_data_mut)
                .unwrap_or_else(|| {
                    panic!("loaded movement owner {owner:?} is missing required actor state")
                })
                .active_movement = ActiveMovement::new(sequence_id, element_index);
        }
    }

    /// Consume one queued `RHElement` position update at the beginning of
    /// this actor's Hourglass, before order selection.
    ///
    /// Original gives the map-space queue priority when both flags are set;
    /// the world-space queue remains armed for the next actor frame. The
    /// resulting teleport still traverses Actor::CheckForLineCrossing.
    pub(super) fn apply_delayed_actor_position(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
    ) {
        let (old_pos, new_pos, layer, posture, is_carried, is_pc, is_human) = {
            let Some(entity) = self.world.entities.get_mut(entity_id) else {
                panic!("delayed-position owner {entity_id:?} disappeared before Actor::Hourglass");
            };
            let posture = entity.element_data().posture;
            let is_carried = entity
                .human_data()
                .is_some_and(|human| human.carrier.is_some());
            let is_pc = entity.is_pc();
            let is_human = entity.is_human();
            let Some((old_pos, new_pos, layer)) =
                entity.element_data_mut().apply_next_delayed_position()
            else {
                return;
            };
            (
                old_pos, new_pos, layer, posture, is_carried, is_pc, is_human,
            )
        };

        if !actor_line_crossing_eligible(
            posture,
            is_carried,
            self.world.fast_grid.level.map_bbox.contains_point(new_pos),
        ) {
            return;
        }

        let crossed = self.check_for_line_crossing(assets, entity_id, old_pos, new_pos, layer);
        if crossed {
            if is_human {
                self.update_roll_after_crossing(assets, entity_id);
            }
            let compute_direction = self
                .orders
                .sequence_manager
                .current_order_for_actor(entity_id)
                .map(|(_, _, order)| order.compute_direction);
            if let Some(compute_direction) = compute_direction
                && let Some(entity) = self.world.entities.get_mut(entity_id)
            {
                entity
                    .position_iface_mut()
                    .compute_increment_all(compute_direction);
            }
        }
        if is_pc {
            self.check_for_patch_line_crossing(sim, assets, entity_id, old_pos, new_pos, layer);
        }
        self.check_for_script_line_crossing(sim, assets, entity_id, old_pos, new_pos, layer);
        self.check_for_sound_line_crossing(assets, entity_id, old_pos, new_pos, layer);
    }

    /// Match `RHSectorBuilding::IsAuthorized()` for gate pathfinding.
    ///
    /// The original initializes every building's maximum occupancy to
    /// `u16::MAX`; the occupant list remains live and is still consulted.
    pub(super) fn building_sector_is_authorized(
        &self,
        sector_number: crate::sector::SectorNumber,
    ) -> bool {
        let sector = self
            .grid_sector_by_number(sector_number)
            .unwrap_or_else(|| panic!("building door references missing sector {sector_number}"));
        let occupant_count = if let Some(building_index) = sector.building_index {
            self.script_domains
                .buildings
                .occupants
                .get(usize::from(building_index.get()))
                .unwrap_or_else(|| {
                    panic!(
                        "building sector {sector_number} references missing building {}",
                        building_index.get()
                    )
                })
                .len()
        } else {
            // TODO(original-parity): attach every door-authored building
            // sector to canonical BuildingState during level loading. A few
            // loaded sectors lack the attachment; count their live actors by
            // sector rather than fabricating an empty building.
            self.world
                .entities
                .actors()
                .filter(|(_, entity)| {
                    entity
                        .element_data()
                        .sector()
                        .is_some_and(|sector| u16::from(sector) == u16::from(sector_number))
                })
                .count()
        };
        occupant_count < usize::from(u16::MAX)
    }

    fn live_mobile_geometry(&self) -> LiveMobileGeometry {
        let mut prepared = LiveMobileGeometry {
            mobile_lines_by_layer: std::collections::BTreeMap::new(),
            mobile_points_by_layer: std::collections::BTreeMap::new(),
            mobile_polygons_by_layer: std::collections::BTreeMap::new(),
        };
        for mobile in &self.world.mobile_elements {
            if !mobile.active {
                continue;
            }
            prepared
                .mobile_lines_by_layer
                .entry(mobile.layer)
                .or_default()
                .extend(mobile.repulsive_lines());
            prepared
                .mobile_points_by_layer
                .entry(mobile.layer)
                .or_default()
                .extend(mobile.repulsive_points());
            prepared
                .mobile_polygons_by_layer
                .entry(mobile.layer)
                .or_default()
                .push(mobile.motion_polygon.clone());
        }
        prepared
    }

    #[cfg(test)]
    pub(super) fn first_live_mobile_polygon_point(
        &self,
        layer: u16,
    ) -> crate::coordinates::MapPoint {
        self.live_mobile_geometry()
            .mobile_polygons_by_layer
            .get(&layer)
            .and_then(|polygons| polygons.first())
            .and_then(|polygon| polygon.first())
            .copied()
            .unwrap_or_else(|| panic!("no live mobile polygon point on layer {layer}"))
    }

    /// Execute the Original's `RHNONANIMATION_RIDER_CHARGING` arm inside its
    /// rider's creation-ordered movement slot. Returns true only when the live
    /// selected movement order was exactly `RiderCharging` and consumed the
    /// slot; stale state is cleared for every other live-order shape.
    fn tick_rider_charge_owner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &crate::engine::LevelAssets,
        rider_id: EntityId,
        frozen_all: bool,
    ) -> bool {
        use crate::element::{ActionState, ActiveRiderCharge, Posture};
        use crate::weapons::SwordStrike;

        let selected = self
            .orders
            .sequence_manager
            .current_element_for_actor(rider_id);
        let live = selected.and_then(|(seq_id, elem_idx)| {
            let element = self.orders.sequence_manager.get_element(seq_id, elem_idx)?;
            if !element.data.is_movement() {
                return None;
            }
            Some((
                seq_id,
                elem_idx,
                element.current_order()?.clone(),
                element.next_order().cloned(),
                element.speed_factor(),
            ))
        });
        let is_live_charge = live
            .as_ref()
            .is_some_and(|(_, _, order, _, _)| order.order_type == OrderType::RiderCharging);
        if !is_live_charge {
            if let Some(entity) = self.world.entities.get_mut(rider_id)
                && let Some(actor) = entity.actor_data_mut()
            {
                actor.active_rider_charge = None;
                actor.last_executed_rider_charge_order_id = None;
            }
            return false;
        }
        let (seq_id, elem_idx, order, next_order, speed_factor) = live.unwrap();

        let rider = self
            .world
            .entities
            .get(rider_id)
            .unwrap_or_else(|| panic!("live rider-charge owner {rider_id:?} disappeared"));
        let soldier = rider
            .soldier_data()
            .unwrap_or_else(|| panic!("RiderCharging owner {rider_id:?} is not a soldier"));
        assert!(
            soldier.rider,
            "RiderCharging owner {rider_id:?} is not a rider"
        );
        let weapon_profile_id =
            super::melee::get_hth_weapon_id_full(rider, &assets.profile_manager).unwrap_or_else(
                || panic!("rider {rider_id:?} has no hand-to-hand weapon profile id"),
            );
        assets
            .profile_manager
            .get_hth_weapon(weapon_profile_id)
            .unwrap_or_else(|| {
                panic!(
                    "rider {rider_id:?} references missing hand-to-hand weapon profile {weapon_profile_id}"
                )
            });
        let transition_frames = rider
            .sprite()
            .num_frames_for_anim(OrderType::TransitionCharging);
        assert!(
            rider.sprite().has_animation(OrderType::TransitionCharging) && transition_frames > 0,
            "rider {rider_id:?} is missing TransitionCharging animation"
        );

        // ExecuteRiderCharge samples these before Turn and PerformMotion on
        // every call. The same sample drives initialization and this frame's
        // narrow hit polygon.
        let (origin, sampled_layer, sampled_direction, forward, sidewards) = {
            let elem = rider.element_data();
            let direction = elem.direction();
            let [fx, fy] = crate::position_interface::sector_to_vector_iso(direction);
            let [sx, sy] = crate::position_interface::sector_to_vector_iso((direction + 4) & 15);
            (
                elem.position_map(),
                elem.layer(),
                direction,
                (fx, fy),
                (sx, sy),
            )
        };

        // RHElementActor's new-order identity is distinct from RHSprite's
        // processed-motion identity. FrozenAll executes this charge pass but
        // deliberately leaves sprite motion initialization pending.
        let needs_initialization = rider
            .actor_data()
            .expect("RiderCharging soldier must have actor data")
            .last_executed_rider_charge_order_id
            != Some(order.order_id);
        if needs_initialization {
            let initial_quad = [
                (
                    origin.x - 20.0 * forward.0 - 20.0 * sidewards.0,
                    origin.y - 20.0 * forward.1 - 20.0 * sidewards.1,
                ),
                (
                    origin.x + 180.0 * forward.0 - 20.0 * sidewards.0,
                    origin.y + 180.0 * forward.1 - 20.0 * sidewards.1,
                ),
                (
                    origin.x + 180.0 * forward.0 + 80.0 * sidewards.0,
                    origin.y + 180.0 * forward.1 + 80.0 * sidewards.1,
                ),
                (
                    origin.x - 20.0 * forward.0 + 80.0 * sidewards.0,
                    origin.y - 20.0 * forward.1 + 80.0 * sidewards.1,
                ),
            ];
            let obstacles = crate::sight_obstacle::ObstacleList {
                static_obstacles: assets.static_sight_obstacles.as_slice(),
                dynamic_obstacles: &self.world.dynamic_sight_obstacles,
                static_active: &self.world.static_sight_obstacle_active,
            };
            let mut pending_victims = Vec::new();
            for (victim_id, victim) in self.world.entities.humans() {
                let victim_id: EntityId = victim_id.into();
                if super::melee::is_possible_sword_strike_victim(
                    &self.world.entities,
                    rider_id,
                    victim,
                    victim_id,
                    &assets.profile_manager,
                    &self.world.fast_grid,
                    obstacles,
                ) && victim.element_data().layer() == sampled_layer
                    && rider_charge_point_in_quad(
                        victim.element_data().position_map(),
                        initial_quad,
                    )
                {
                    pending_victims.push(victim_id);
                }
            }
            self.world.entities[rider_id]
                .as_mut()
                .expect("rider remained present during charge initialization")
                .actor_data_mut()
                .expect("RiderCharging soldier must have actor data")
                .active_rider_charge = Some(ActiveRiderCharge { pending_victims });
        }

        let goal = MapPoint::new(order.target_x, order.target_y);
        let next_destination_same_action = next_order
            .filter(|next| next.order_type == order.order_type)
            .map(|next| MapPoint::new(next.target_x, next.target_y));
        let motion_context = MotionOrderContext {
            order_id: order.order_id,
            destination: goal,
            reverse: order.reverse,
            tolerance: order.tolerance,
            directional_tolerance: false,
            compute_direction: order.compute_direction,
            next_destination_same_action,
        };
        let (motion_state, actual_frame) = {
            let entity = self.world.entities[rider_id]
                .as_mut()
                .expect("rider remained present before charge motion");
            let elem = entity.element_data_mut();
            let dx = goal.x - elem.position_map().x;
            let dy = goal.y - elem.position_map().y;
            if order.compute_direction && dx * dx + dy * dy > 0.01 {
                let direction = vector_to_sector_0_to_15(dx, dy);
                elem.set_direction_goal(if order.reverse {
                    direction ^ 8
                } else {
                    direction
                });
            }
            elem.sprite.position_iface.turn();
            let (state, distance) = if frozen_all {
                // FrozenAll short-circuits RHSprite::PerformMotion before it
                // changes row/frame/order state. ExecuteRiderCharge continues
                // around that call and uses the sprite's existing live frame.
                (MotionState::InProgress, 0.0)
            } else {
                elem.sprite.perform_motion(
                    sim,
                    Some(motion_context),
                    OrderType::TransitionCharging,
                    elem.direction() as u16,
                    FrameProgression::Default,
                    false,
                    MotionMethod::Run,
                    false,
                )
            };
            if distance != 0.0 {
                elem.sprite
                    .position_iface
                    .update_position_map_scaled(distance * speed_factor);
                elem.update_grid_cell();
            }
            (state, elem.sprite.current_frame)
        };
        let last_frame = actual_frame == transition_frames - 1;
        if matches!(motion_state, MotionState::Start) {
            let entity = self.world.entities[rider_id]
                .as_mut()
                .expect("rider remained present after charge motion");
            assert_eq!(
                entity.element_data().posture,
                Posture::Upright,
                "rider charge must start upright"
            );
            let actor = entity
                .actor_data_mut()
                .expect("RiderCharging soldier must have actor data");
            actor.action_state = ActionState::MovingFast;
            entity.element_data_mut().posture = Posture::Upright;
        }

        let back_length = (5.0 * f32::from(actual_frame)).min(50.0);
        let back = (-back_length * forward.0, -back_length * forward.1);
        let front = if last_frame { 15.0 } else { 0.0 };
        let hit_quad = [
            (origin.x + back.0, origin.y + back.1),
            (origin.x + front * forward.0, origin.y + front * forward.1),
            (
                origin.x + front * forward.0 + 60.0 * sidewards.0,
                origin.y + front * forward.1 + 60.0 * sidewards.1,
            ),
            (
                origin.x + back.0 + 60.0 * sidewards.0,
                origin.y + back.1 + 60.0 * sidewards.1,
            ),
        ];

        // Copy only the IDs to release the charge borrow. Each hit is removed
        // before launching its damage element, and queue_sword_damage
        // completes synchronously before the next candidate is inspected.
        let candidates = self.world.entities[rider_id]
            .as_ref()
            .expect("rider remained present before charge damage")
            .actor_data()
            .expect("RiderCharging soldier must have actor data")
            .active_rider_charge
            .as_ref()
            .expect("live RiderCharging order must retain active charge state")
            .pending_victims
            .clone();
        for victim_id in candidates {
            let Some(victim) = self.world.entities.get(victim_id) else {
                // Original pointers cannot become holes independently; Rust
                // entity removal can. Retain the pending ID so this is visible
                // state rather than silently fabricating a resolved hit.
                continue;
            };
            if victim.element_data().layer() != sampled_layer
                || !rider_charge_point_in_quad(victim.element_data().position_map(), hit_quad)
            {
                continue;
            }
            self.world.entities[rider_id]
                .as_mut()
                .expect("rider remained present while resolving charge hit")
                .actor_data_mut()
                .expect("RiderCharging soldier must have actor data")
                .active_rider_charge
                .as_mut()
                .expect("active charge must remain installed through synchronous damage")
                .pending_victims
                .retain(|pending| *pending != victim_id);
            self.queue_sword_damage(
                sim,
                assets,
                victim_id,
                rider_id,
                SwordStrike::Charge,
                weapon_profile_id,
            );
        }

        if last_frame {
            // Rewrite only the same live order identity sampled above. Damage
            // can interrupt or replace it synchronously; never mutate a newer
            // order in that case.
            let still_same = self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .and_then(|element| element.current_order())
                .is_some_and(|current| {
                    current.order_type == OrderType::RiderCharging
                        && current.order_id == order.order_id
                });
            if still_same {
                let fresh_id = self.orders.allocate_order_id();
                let current = self
                    .orders
                    .sequence_manager
                    .get_element_mut(seq_id, elem_idx)
                    .and_then(|element| element.orders.front_mut())
                    .expect("validated rider charge order disappeared before rewrite");
                current.order_type = OrderType::RunningUpright;
                current.order_id = fresh_id;
            }
            self.world.entities[rider_id]
                .as_mut()
                .expect("rider remained present at charge completion")
                .actor_data_mut()
                .expect("RiderCharging soldier must have actor data")
                .active_rider_charge = None;
        }

        // RHElementActor::Hourglass clears mbNewOrder after Execute even when
        // FrozenAll prevented RHSprite::PerformMotion from initializing. Stamp
        // only the actor-level identity. A synchronously installed fresh order
        // therefore still initializes on its next owner slot.
        self.world.entities[rider_id]
            .as_mut()
            .expect("rider remained present after charge execute")
            .actor_data_mut()
            .expect("RiderCharging soldier must have actor data")
            .last_executed_rider_charge_order_id = Some(order.order_id);

        tracing::trace!(
            ?rider_id,
            ?sampled_direction,
            actual_frame,
            last_frame,
            frozen_all,
            "executed rider charge in owner movement slot"
        );
        true
    }

    pub(super) fn lift_endpoint_points(
        &self,
        sector_number: crate::sector::SectorNumber,
    ) -> (MapPoint, MapPoint) {
        let sector = self
            .grid_sector_by_number(sector_number)
            .expect("DetermineMovementAnimation: missing lift sector");
        let low = sector.low_exit_point.unwrap_or_else(|| {
            panic!("DetermineMovementAnimation: lift sector {sector_number} missing low exit point")
        });
        let high = sector.high_exit_point.unwrap_or_else(|| {
            panic!(
                "DetermineMovementAnimation: lift sector {sector_number} missing high exit point"
            )
        });
        (low, high)
    }

    fn determine_lift_movement_animation(
        &self,
        owner: EntityId,
        posture_after: crate::element::Posture,
        action: OrderType,
        destination: MapPoint,
    ) -> OrderType {
        let Some(entity) = self.world.entities.get(owner) else {
            return action;
        };
        determine_lift_movement_animation_for(
            entity,
            &self.world.fast_grid,
            posture_after,
            action,
            destination,
        )
    }

    pub(crate) fn apply_sword_movement_start_initiative_transfer(&mut self, entity_id: EntityId) {
        let principal_id = self
            .get_entity(entity_id)
            .and_then(|e| e.human_data())
            .and_then(|h| h.opponents.first().copied());

        if let Some(entity) = self.world.entities.get_mut(entity_id)
            && let Some(human) = entity.human_data_mut()
        {
            human.smalltalk_initiative = false;
        }

        let Some(principal_id) = principal_id else {
            return;
        };
        let is_mutual = self
            .get_entity(principal_id)
            .and_then(|e| e.human_data())
            .and_then(|h| h.opponents.first().copied())
            .map(|opp| opp == entity_id)
            .unwrap_or(false);
        if !is_mutual {
            return;
        }

        if let Some(entity) = self.world.entities.get_mut(principal_id)
            && let Some(human) = entity.human_data_mut()
        {
            human.smalltalk_initiative = true;
            human.received_smalltalk_initiative = true;
        }
    }

    fn maybe_provoke_after_sword_movement_terminated(
        &mut self,
        assets: &crate::engine::LevelAssets,
        entity_id: EntityId,
    ) {
        let principal_id = self
            .get_entity(entity_id)
            .and_then(|e| e.human_data())
            .and_then(|h| h.opponents.first().copied());
        let Some(principal_id) = principal_id else {
            return;
        };

        let is_mutual = self
            .get_entity(principal_id)
            .and_then(|e| e.human_data())
            .and_then(|h| h.opponents.first().copied())
            .map(|opp| opp == entity_id)
            .unwrap_or(false);
        if !is_mutual {
            return;
        }

        let Some(me) = self.get_entity(entity_id) else {
            return;
        };
        let Some(opponent) = self.get_entity(principal_id) else {
            return;
        };
        let me_pos = me.element_data().position();
        let opponent_pos = opponent.element_data().position();
        let dx = me_pos.x - opponent_pos.x;
        let dy = me_pos.y - opponent_pos.y;
        let dz = me_pos.z - opponent_pos.z;
        let distance = (dx * dx + dy * dy + dz * dz).sqrt();

        let Some(me_weapon) =
            crate::engine::melee::get_hth_weapon_id_full(me, &assets.profile_manager)
                .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
        else {
            return;
        };
        let Some(opponent_weapon) =
            crate::engine::melee::get_hth_weapon_id_full(opponent, &assets.profile_manager)
                .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
        else {
            return;
        };

        let between = |weapon: &crate::profiles::HtHWeaponProfile| {
            let lo = weapon.distance[crate::weapons::WeaponDistance::Maximal as usize] as f32;
            let hi = weapon.distance[crate::weapons::WeaponDistance::Uber as usize] as f32;
            lo < distance && distance <= hi
        };
        tracing::trace!(
            ?entity_id,
            ?principal_id,
            distance,
            my_maximal = me_weapon.distance[crate::weapons::WeaponDistance::Maximal as usize],
            my_uber = me_weapon.distance[crate::weapons::WeaponDistance::Uber as usize],
            opponent_maximal =
                opponent_weapon.distance[crate::weapons::WeaponDistance::Maximal as usize],
            opponent_uber = opponent_weapon.distance[crate::weapons::WeaponDistance::Uber as usize],
            "checking sword-movement termination Provoke"
        );
        if between(me_weapon) && between(opponent_weapon) {
            self.launch_element(crate::sequence::SequenceElement::new(
                1,
                crate::element::Command::Provoke,
                Some(entity_id),
            ));
        }
    }

    // ─── Order system ─────────────────────────────────────────────

    /// Snap a click/formation-slot point to the nearest authorized
    /// (walkable) position for a unit of the given size.
    ///
    /// Returns the adjusted point, or `None` if no walkable spot can be
    /// found near the click. Builds a move-box-sized bbox around the
    /// candidate point, pushes it away from any motion lines that would
    /// otherwise block the unit, then returns the box center.
    ///
    /// Without this snap, clicks that land on dynamic elements like
    /// drawbridges (whose surface lies just outside the static motion-area
    /// polygon) or even slightly inside an obstacle's bbox fail
    /// `object_position_authorized` and the pathfinder refuses to build
    /// a path — so the click appears to do nothing.
    ///
    /// `reference` is used as the "push toward" anchor — typically the
    /// raw click point passed alongside the per-PC formation slot.
    ///
    /// Callers must skip this snap when the click hits a Door/Drawbridge
    /// sector — the cross-sector gate A* path routes the PC through the
    /// door's entry point, which is the only walkable approach when the
    /// door sector itself isn't a motion area (e.g. a raised drawbridge).
    pub fn snap_click_to_walkable(
        &self,
        candidate: MapPoint,
        reference: MapPoint,
        layer: u16,
        half_diagonal_idx: usize,
    ) -> Option<MapPoint> {
        let hd = self
            .world
            .fast_grid
            .level
            .move_box_half_diagonals
            .get(half_diagonal_idx)
            .copied()?;
        let mut bbox = MapBBox::from_corners(
            MapPoint::new(candidate.x - hd.x, candidate.y - hd.y),
            MapPoint::new(candidate.x + hd.x, candidate.y + hd.y),
        );
        if self
            .world
            .fast_grid
            .find_authorized_position_toward(&mut bbox, reference, layer)
        {
            Some(bbox.center())
        } else {
            None
        }
    }

    /// Issue movement orders for a group of selected PCs around a single
    /// click point.
    ///
    /// Uses the "mercenary" formation: each PC keeps its position
    /// relative to the group centroid and walks to the corresponding
    /// offset around `click_point`.  The marker for each PC is placed
    /// at *its own* resolved destination, not at the raw click point.
    ///
    /// Each per-PC formation slot is then snapped to a walkable spot via
    /// [`EngineInner::snap_click_to_walkable`].  This is what allows
    /// clicks on drawbridges and other dynamic elements to actually move
    /// PCs onto them — the raw click often lands just outside the
    /// walkable polygon, and the snap pulls it back inside.
    ///
    /// Uses mercenary formation for compact groups and circular dispatch
    /// for spread-out groups.
    pub(crate) fn perform_group_move(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        pc_ids: &[EntityId],
        click_point: MapPoint,
        run: bool,
        show_marker: bool,
        goal_override: Option<(crate::sector::SectorNumber, u16)>,
    ) {
        if pc_ids.is_empty() {
            return;
        }

        // Preemption is handled downstream by `arbitrate_instruct`:
        // every same-sector PC gets a fresh `Command::Move` sequence
        // element launched via `launch_element` below, which reaches
        // `InstructOwner` on the next hourglass and drives the standard
        // priority-arbitration cascade.  A pending scroll/object pickup
        // (Seek + queued Take at `Normal`) vs a new Move at `Normal`
        // resolves to `InterruptCurrent`, which cleanly tears down both
        // the seek and its post-seek Take via the `NEXT_LEVEL` cascade.
        // Earlier fixes tried to short-circuit this with explicit
        // `stop_owner` calls, but `stop_owner` on a movement element
        // keeps the element InProgress "for transition", which left the
        // stale seek hanging when the same-sector shortcut was
        // direct-pathfinder rather than a proper Move element.

        // Collect each PC's effective route-source position, layer, and
        // sector. While a non-interruptible door pass is active, a newly
        // issued move cannot begin until that pass reaches its committed far
        // side. Original input dispatch observes that committed door side
        // when `PerformGroupMove` calls `AppendMoveToSequence`; using the
        // actor's still-visible near-side sector here would incorrectly
        // classify a return click as a same-sector Move and lose the reverse
        // gate traversal before the command is postponed.
        let positions: Vec<(EntityId, MapPoint, u16, u16)> = pc_ids
            .iter()
            .filter_map(|&pc_id| {
                self.get_entity(pc_id).map(|e| {
                    let elem = e.element_data();
                    let (position, sector, layer) = {
                        let (door_handle, door_direction) = current_door_for_route_source(e);
                        adapt_source_to_current_door(
                            &self.script_domains.interactables.doors,
                            door_handle,
                            door_direction,
                        )
                        .unwrap_or_else(|| {
                            (
                                elem.position_map(),
                                elem.sector().map(u16::from).unwrap_or(0),
                                elem.layer(),
                            )
                        })
                    };
                    (pc_id, position, layer, sector)
                })
            })
            .collect();
        if positions.is_empty() {
            return;
        }

        let src_layer = positions[0].2;
        let reference = positions[0].1;

        // ── Unified sector hit-test ──
        //
        // Top-down layer search to set the selected sector / layer /
        // valid-for-move flags.  When `goal_override` is set, skip the
        // spatial query: host-side hover already selected the authoritative
        // sector/layer.  This mirrors C++ where `update_mouse` mutates
        // `mpSelectedSector` / `muwSelectedLayer` before
        // `PerformGroupMove` reads them.
        let (
            goal_sector,
            effective_click,
            effective_layer,
            is_valid,
            is_door_click,
            is_jump_click,
            clicked_jump_sector_idx,
            jump_underlying_sector,
            clicked_door_index,
        ) = if let Some((override_sector, override_layer)) = goal_override {
            // Explicit host-selected sector path: route to
            // `(override_sector, override_layer)` unconditionally.  The
            // waypoint becomes the destination without snapping; per-PC
            // routing below still calls `snap_click_to_walkable` at
            // `effective_layer` to find a valid landing position.  The
            // host only sends motion-sector overrides, so door / jump
            // shortcuts stay false.  `is_valid = true` keeps the
            // simple-move branch reachable when the PC is already in the
            // selected sector.
            (
                Some(override_sector),
                click_point,
                override_layer,
                true,
                false,
                false,
                None,
                None,
                None,
            )
        } else {
            let hit = self
                .world
                .fast_grid
                .get_sector_screen(click_point, reference);
            let is_valid = hit.is_valid_for_move(&self.world.fast_grid);

            // ── Door/Drawbridge click shortcut ──
            //
            // When the click hits a door sector, bypass the walkability
            // snap on formation slots.  Per-PC routing must also skip
            // `snap_click_to_walkable` so the destination stays in the
            // door sector and the gate-A* path routes through the
            // door's entry point (the door sector itself is not a
            // motion area).
            let is_door_click_sector = hit
                .sector_idx
                .and_then(|i| self.world.fast_grid.level.sectors.get(usize::from(i)))
                .is_some_and(|s| s.sector_type.is_door());
            let is_jump_click = hit
                .sector_idx
                .and_then(|i| self.world.fast_grid.level.sectors.get(usize::from(i)))
                .is_some_and(|s| s.sector_type.is_jump());
            let jump_underlying_sector = hit
                .sector_idx
                .and_then(|i| self.world.fast_grid.level.sectors.get(usize::from(i)))
                .filter(|s| s.sector_type.is_jump())
                .and_then(|s| s.underlying_sector)
                .and_then(|i| {
                    self.world
                        .fast_grid
                        .level
                        .sectors
                        .get(usize::from(i))
                        .map(|s| (s.sector_number, s.layer))
                });

            // Door index of the clicked door sector, if any.  Used to
            // route the per-PC gate search via `find_path_to_door` and
            // emit a `GoalShape::Door` terminal element.
            let clicked_sector_door_index: Option<u32> = hit
                .sector_idx
                .and_then(|i| self.world.fast_grid.level.sectors.get(usize::from(i)))
                .and_then(|s| s.door_index);
            let clicked_polygon_door_index = self.scripts.mission.as_ref().and_then(|_| {
                door_click_polygon_at(&self.script_domains.interactables.doors, click_point)
            });
            let clicked_door_index = clicked_sector_door_index.or(clicked_polygon_door_index);
            let is_door_click = is_door_click_sector || clicked_door_index.is_some();

            let (effective_click, effective_layer) = if is_valid || is_jump_click {
                (click_point, hit.layer)
            } else {
                let snapped = self.snap_to_nearest_walkable(assets, click_point, src_layer);
                (snapped.unwrap_or(click_point), src_layer)
            };
            (
                hit.sector,
                effective_click,
                effective_layer,
                is_valid,
                is_door_click,
                is_jump_click,
                if is_jump_click { hit.sector_idx } else { None },
                jump_underlying_sector,
                clicked_door_index,
            )
        };

        // ── Compute formation slots around the click point ──
        //
        // If the group is compact enough, use mercenary formation
        // (preserve relative positions).  Otherwise use circular
        // dispatch (arrange in a circle around click).
        let pc_positions: Vec<MapPoint> = positions.iter().map(|(_, p, _, _)| *p).collect();
        let dests = {
            let n = pc_positions.len() as f32;
            let cx = pc_positions.iter().map(|p| p.x).sum::<f32>() / n;
            let cy = pc_positions.iter().map(|p| p.y).sum::<f32>() / n;
            let max_sq_dist = pc_positions
                .iter()
                .map(|p| {
                    let dx = p.x - cx;
                    let dy = p.y - cy;
                    dx * dx + dy * dy
                })
                .fold(0.0f32, f32::max);
            if max_sq_dist <= GROUP_LIMIT_MAX * GROUP_LIMIT_MAX {
                mercenary_formation_destinations(&pc_positions, effective_click)
            } else {
                // Snap each circular candidate to a walkable position
                // before the assignment algorithm.  Skip the snap on
                // door clicks so the raw formation slot feeds into the
                // cross-sector gate-A* path below.
                let raw = circular_dispatch_destinations(&pc_positions, effective_click);
                if is_door_click {
                    raw
                } else {
                    raw.into_iter()
                        .map(|c| {
                            self.snap_click_to_walkable(c, effective_click, effective_layer, 0)
                                .unwrap_or(c)
                        })
                        .collect()
                }
            }
        };

        // ── Per-PC routing ──
        // For each PC, decide between:
        //   1. Same-sector: simple MOVE
        //   2. Cross-sector (door/lift): gate-A* sequence
        for ((pc_id, _, pc_src_layer, src_sector), dest) in positions.iter().zip(dests.iter()) {
            let mut pc_goal_sector = goal_sector;
            let mut pc_effective_layer = effective_layer;
            if is_jump_click {
                // PerformGroupMove authorizes each formation slot before
                // PerformMove tests whether the selected jump is usable.
                // Keep the raw click through the jump-sector hit test, then
                // apply that same move-box authorization here; the coarse
                // nearest-walkable fallback is not equivalent near a jump
                // landing boundary.
                let Some(resolved_jump_dest) =
                    self.snap_click_to_walkable(*dest, effective_click, pc_effective_layer, 0)
                else {
                    self.hero_speaking(
                        assets,
                        *pc_id,
                        crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
                    );
                    continue;
                };
                let pc_pos = positions
                    .iter()
                    .find(|(id, _, _, _)| *id == *pc_id)
                    .map(|(_, p, _, _)| *p)
                    .unwrap_or(*dest);
                let source_line_idx = self
                    .get_nearest_jumpable_jump_line(
                        *pc_id,
                        u32::from(clicked_jump_sector_idx.unwrap_or_else(|| {
                            panic!("jump click missing selected jump sector index")
                        })),
                        pc_pos,
                        resolved_jump_dest,
                        false,
                        jump_underlying_sector.map(|(sector, _)| u16::from(sector)),
                    )
                    .and_then(crate::jump_line::JumpLineIndex::new);
                if let Some(source_line_idx) = source_line_idx {
                    let Some(source_line) = self
                        .world
                        .fast_grid
                        .level
                        .jump_lines
                        .get(usize::from(source_line_idx))
                        .cloned()
                    else {
                        panic!("line-jump source line {source_line_idx} is missing");
                    };
                    let Some(destination_line_idx) = source_line
                        .associated_line_index
                        .and_then(crate::jump_line::JumpLineIndex::new)
                    else {
                        panic!("line-jump source line {source_line_idx} has no associated line");
                    };
                    if self
                        .world
                        .fast_grid
                        .level
                        .jump_lines
                        .get(usize::from(destination_line_idx))
                        .is_none()
                    {
                        panic!(
                            "line-jump destination line {destination_line_idx} for source {source_line_idx} is missing"
                        );
                    }

                    let is_swordfighting = self
                        .get_entity(*pc_id)
                        .and_then(|e| e.human_data())
                        .map(|h| !h.opponents.is_empty())
                        .unwrap_or(false);
                    let action = if is_swordfighting {
                        let want = if run {
                            OrderType::RunningWithSword
                        } else {
                            OrderType::WalkingWithSword
                        };
                        let has_sword_row = self
                            .get_entity(*pc_id)
                            .map(|e| e.sprite().has_animation(want))
                            .unwrap_or(false);
                        if has_sword_row {
                            want
                        } else if run {
                            OrderType::RunningUpright
                        } else {
                            OrderType::WalkingUpright
                        }
                    } else if run {
                        OrderType::RunningUpright
                    } else {
                        match self.get_entity(*pc_id).map(|e| e.element_data().posture) {
                            Some(crate::element::Posture::Crouched) => OrderType::WalkingCrouched,
                            _ => OrderType::WalkingUpright,
                        }
                    };

                    let mut seq = build_line_jump_click_sequence(
                        *pc_id,
                        action,
                        source_line_idx,
                        &source_line,
                        destination_line_idx,
                        resolved_jump_dest,
                        pc_effective_layer,
                        1.0,
                    );
                    if is_swordfighting {
                        force_sword_movement_for_sequence(&mut seq);
                    }
                    let speak = crate::sequence::SequenceElement::new(
                        4,
                        crate::element::Command::SpeakHeroReachDestination,
                        Some(*pc_id),
                    );
                    seq.append_element(speak);
                    self.append_posture_recovery(*pc_id, &mut seq);
                    self.launch_sequence(seq);
                    if show_marker && !is_door_click {
                        self.feedback.ground_mark.add_mark(
                            resolved_jump_dest.x,
                            resolved_jump_dest.y,
                            pc_effective_layer,
                        );
                    }
                    continue;
                } else if let Some((underlying_sector, underlying_layer)) = jump_underlying_sector {
                    pc_goal_sector = Some(underlying_sector);
                    pc_effective_layer = underlying_layer;
                    tracing::debug!(
                        actor = ?pc_id,
                        click_x = effective_click.x,
                        click_y = effective_click.y,
                        sector = %underlying_sector,
                        layer = underlying_layer,
                        "jump-sector click has no executable jump line; falling back to underlying motion sector"
                    );
                } else {
                    tracing::warn!(
                        actor = ?pc_id,
                        click_x = effective_click.x,
                        click_y = effective_click.y,
                        "jump-sector click has no executable jump line and no underlying motion sector"
                    );
                    self.hero_speaking(
                        assets,
                        *pc_id,
                        crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
                    );
                    continue;
                };
            }

            // Same-sector or unknown goal sector: simple move
            if !is_door_click
                && (!is_valid
                    || pc_goal_sector.is_none()
                    || pc_goal_sector.is_some_and(|goal| {
                        u16::from(goal) == *src_sector && pc_effective_layer == *pc_src_layer
                    }))
            {
                // Door clicks skip the walkable snap entirely.
                let snap_res = if is_door_click {
                    Some(*dest)
                } else {
                    self.snap_click_to_walkable(*dest, effective_click, pc_effective_layer, 0)
                };
                let snapped = match snap_res {
                    Some(pt) => pt,
                    None => {
                        // FindAuthorizedPosition failure on the
                        // mercenary/same-sector path fires
                        // HERO_UNABLE_TO_DO_SOMETHING and skips the
                        // move for this PC.
                        self.hero_speaking(
                            assets,
                            *pc_id,
                            crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
                        );
                        continue;
                    }
                };
                // Launch a Move sequence element.  Going through the
                // sequence pipeline — rather than a direct
                // `pathfinder.add_request` shortcut — means the element
                // hits `arbitrate_instruct` when it transitions
                // Todo → InProgress next hourglass.  Any pending Seek +
                // post-seek Take (from a prior scroll-pickup click) at
                // Normal priority is interrupted by the new Normal Move
                // via the NEXT_LEVEL cascade, cleanly tearing down the
                // pickup so it doesn't replay at the new destination.
                let is_swordfighting = self
                    .get_entity(*pc_id)
                    .and_then(|e| e.human_data())
                    .map(|h| !h.opponents.is_empty())
                    .unwrap_or(false);
                let action = if is_swordfighting {
                    // Sword-variant pick with PC-fallback.  When the
                    // determined sword animation isn't in the actor's
                    // sprite profile, fall back to the plain upright
                    // variant.  PC sprites (e.g. "Robin des bois") ship
                    // without `WalkingWithSword` / `RunningWithSword`
                    // rows — the dev-mode assert compiles out in
                    // release, leaving the upright fallback as the
                    // shipping behaviour.
                    let want = if run {
                        OrderType::RunningWithSword
                    } else {
                        OrderType::WalkingWithSword
                    };
                    let has_sword_row = self
                        .get_entity(*pc_id)
                        .map(|e| e.sprite().has_animation(want))
                        .unwrap_or(false);
                    if has_sword_row {
                        want
                    } else if run {
                        OrderType::RunningUpright
                    } else {
                        OrderType::WalkingUpright
                    }
                } else if run {
                    OrderType::RunningUpright
                } else {
                    match self.get_entity(*pc_id).map(|e| e.element_data().posture) {
                        Some(crate::element::Posture::Crouched) => OrderType::WalkingCrouched,
                        _ => OrderType::WalkingUpright,
                    }
                };
                let mut move_elem = crate::sequence::SequenceElement::new_movement(
                    1,
                    crate::element::Command::Move,
                    Some(*pc_id),
                    action,
                );
                if let crate::sequence::SequenceElementData::Movement {
                    destination,
                    layer,
                    flags,
                    ..
                } = &mut move_elem.data
                {
                    *destination = snapped;
                    *layer = pc_effective_layer;
                    if is_swordfighting {
                        *flags |= crate::sequence::MoveFlags::FORCE_SWORD_MOVEMENT;
                    }
                }

                // Append a `SpeakHeroReachDestination` element after
                // the move and cap the sequence with any
                // posture-cleanup sub-elements the PC needs (re-equip
                // bow, re-crouch, re-enter HelpingClimb / Beggar,
                // demote trailing ShootBow to ShootBowOnce).  The PC's
                // `Instruct` override terminates the Speak element on
                // dispatch and queues the HERO_DONE_COMMAND bark
                // (handled by `arbitrate_instruct`).
                let speak = crate::sequence::SequenceElement::new(
                    1,
                    crate::element::Command::SpeakHeroReachDestination,
                    Some(*pc_id),
                );
                let mut seq = crate::sequence::Sequence::new();
                seq.append_element(move_elem);
                seq.append_element(speak);
                self.append_posture_recovery(*pc_id, &mut seq);
                self.launch_sequence(seq);
                if show_marker && !is_door_click {
                    self.feedback
                        .ground_mark
                        .add_mark(snapped.x, snapped.y, pc_effective_layer);
                }
                continue;
            }

            if pc_goal_sector.is_none() && !is_door_click {
                tracing::warn!("skipping cross-sector move without resolved goal sector");
                continue;
            };

            // PerformGroupMove resolves every formation slot through
            // FindAuthorizedPosition before it builds a per-PC gate route.
            // This is also required for a single PC clicking a lift: the
            // authored click can be shifted slightly so the upright move box
            // fits inside the narrow wall/ladder rail.
            let resolved_dest = if is_door_click {
                *dest
            } else {
                let Some(resolved) =
                    self.snap_click_to_walkable(*dest, effective_click, pc_effective_layer, 0)
                else {
                    self.hero_speaking(
                        assets,
                        *pc_id,
                        crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
                    );
                    continue;
                };
                resolved
            };

            // Cross-sector: try gate A*
            let pc_pos_raw = positions
                .iter()
                .find(|(id, _, _, _)| *id == *pc_id)
                .map(|(_, p, _, _)| *p)
                .unwrap_or(*dest);

            // Source adaptation: if the PC is currently straddling a
            // gate, use the gate's far-side point / sector as the path
            // source.  Without this, the pathfinder starts from inside
            // the door sector, which is not a motion area and yields no
            // valid seed gates.
            let (door_handle, door_direction) = self
                .get_entity(*pc_id)
                .map(current_door_for_route_source)
                .unwrap_or((crate::position_interface::DoorHandle::NULL, false));
            let (pc_pos, path_src_sector, _path_src_layer) = {
                let adapted = self.scripts.mission.as_ref().and_then(|_| {
                    adapt_source_to_current_door(
                        &self.script_domains.interactables.doors,
                        door_handle,
                        door_direction,
                    )
                });
                match adapted {
                    Some((adj, sector, layer)) => (adj, sector, layer),
                    None => (pc_pos_raw, *src_sector, *pc_src_layer),
                }
            };

            // Door-click routing: when the click lands on a door
            // sector with a known `door_index`, use
            // `find_path_to_door` and `GoalShape::Door` so the trailing
            // emission walks the PC up to the door's near-side (and
            // CHANGE_POSITION-teleports into buildings, turns the PC to
            // face the lock for lockpicks, etc.).
            let door_goal = if is_door_click {
                clicked_door_index
            } else {
                None
            };

            // PC authorisation for the gate A*.  Click-to-move never
            // sets `MoveFlags::MAP`, so `allow_leave_map = false` here.
            let pc_auth = self.get_entity(*pc_id).map(|e| e.actor_auth_info());
            let level = self.world.fast_grid.level.clone();
            let door_goal_info = door_goal.and_then(|door_idx| {
                self.scripts.mission.as_ref().and_then(|_| {
                    let path = crate::gate::find_path_into_door(
                        &self.script_domains.interactables.doors,
                        (pc_pos.x, pc_pos.y),
                        path_src_sector,
                        crate::gate::DoorIndex(door_idx),
                        pc_auth.as_ref(),
                        false,
                        &|sector| self.building_sector_is_authorized(sector),
                        &|sector| {
                            level
                                .sectors
                                .iter()
                                .find(|candidate| candidate.sector_number == sector)
                                .and_then(|candidate| candidate.lift_type)
                        },
                    )?;
                    let terminal = path
                        .last()
                        .copied()
                        .expect("path into a door must contain the goal door");
                    assert_eq!(
                        terminal.door_index,
                        crate::gate::DoorIndex(door_idx),
                        "path into door {door_idx} ended at {}",
                        terminal.door_index
                    );
                    let door = self
                        .script_domains
                        .interactables
                        .doors
                        .get(usize::from(terminal.door_index))
                        .expect("terminal door path index must resolve");
                    let (point, sector, layer) = if terminal.direct {
                        (door.point_out, door.sector_out, door.layer_out)
                    } else {
                        (door.point_in, door.sector_in, door.layer_in)
                    };
                    Some((door_idx, path, (point.x, point.y), u16::from(sector), layer))
                })
            });

            let door_far_side_is_building = door_goal_info.as_ref().map(|(_, _, _, sector, _)| {
                self.grid_sector_by_number(crate::sector::SectorNumber::new(*sector as i16))
                    .map(|gs| gs.sector_type.is_building())
                    .unwrap_or(false)
            });

            let path = if door_goal_info.is_some() {
                door_goal_info.as_ref().map(|(_, p, _, _, _)| p.clone())
            } else {
                let Some(goal_sector) = pc_goal_sector else {
                    tracing::warn!("skipping gate path without resolved goal sector");
                    continue;
                };
                let level = self.world.fast_grid.level.clone();
                self.scripts.mission.as_ref().and_then(|_| {
                    crate::gate::find_path_gates(
                        &self.script_domains.interactables.doors,
                        (pc_pos.x, pc_pos.y),
                        path_src_sector,
                        (resolved_dest.x, resolved_dest.y),
                        u16::from(goal_sector),
                        pc_auth.as_ref(),
                        false,
                        &|sector| {
                            level
                                .sectors
                                .iter()
                                .find(|candidate| candidate.sector_number == sector)
                                .and_then(|candidate| candidate.lift_type)
                        },
                    )
                })
            };

            match path {
                Some(gate_steps) => {
                    tracing::info!(
                        "Gate A* from sector {} to sector {}: {} gates{}",
                        src_sector,
                        pc_goal_sector.map(u16::from).unwrap_or(*src_sector),
                        gate_steps.len(),
                        if door_goal.is_some() {
                            " (door goal)"
                        } else {
                            ""
                        },
                    );
                    let goal_shape = if let Some((door_idx, _, pt, _sector, layer)) = door_goal_info
                    {
                        GoalShape::Door {
                            door_index: crate::gate::DoorIndex(door_idx),
                            far_side_point: MapPoint::new(pt.0, pt.1),
                            far_side_layer: layer,
                            far_side_is_building: door_far_side_is_building.unwrap_or(false),
                        }
                    } else {
                        GoalShape::Point {
                            point: resolved_dest,
                            tolerance: 0.0,
                        }
                    };
                    let _ = self.build_gate_movement_sequence(
                        sim,
                        *pc_id,
                        gate_steps,
                        goal_shape,
                        pc_effective_layer,
                        if run {
                            OrderType::RunningUpright
                        } else {
                            OrderType::WalkingUpright
                        },
                        door_goal.is_none(),
                        1.0,
                        if self
                            .get_entity(*pc_id)
                            .and_then(|e| e.human_data())
                            .map(|h| !h.opponents.is_empty())
                            .unwrap_or(false)
                        {
                            crate::sequence::MoveFlags::FORCE_SWORD_MOVEMENT
                        } else {
                            crate::sequence::MoveFlags::empty()
                        },
                        Vec::new(),
                        Vec::new(),
                        true,
                        true,
                    );
                    if show_marker && !is_door_click {
                        self.feedback.ground_mark.add_mark(
                            resolved_dest.x,
                            resolved_dest.y,
                            pc_effective_layer,
                        );
                    }
                }
                None => {
                    // RHSequence::AppendMoveToSequence reports an
                    // unreachable cross-sector destination and returns
                    // without appending a direct MOVE when gate routing
                    // fails.
                    self.hero_speaking(
                        assets,
                        *pc_id,
                        crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
                    );
                }
            }
        }

        // At the tail of group-move, if the click happened during
        // macro recording the messenger forwards `StopRecordingMacro`.
        // Routing through the messenger keeps the downstream
        // bookkeeping (QA HUD reset, macro-slot commit) consistent
        // with other stop points.
        if self.is_recording_macro() {
            self.orders.messenger.send(crate::messenger::Message::pc(
                crate::messenger::PcMessage::StopRecordingMacro,
                None,
            ));
        }
    }

    /// Search concentric rings for the nearest point inside a walkable
    /// motion area polygon on the given layer. Used when a click lands
    /// outside all sectors.
    fn snap_to_nearest_walkable(
        &self,
        assets: &LevelAssets,
        click: MapPoint,
        layer: u16,
    ) -> Option<MapPoint> {
        for radius_step in 1..=20u32 {
            let r = radius_step as f32 * 10.0;
            for dir in 0..16u32 {
                let angle = dir as f32 * std::f32::consts::FRAC_PI_8;
                let candidate = MapPoint::new(click.x + angle.sin() * r, click.y - angle.cos() * r);
                if assets
                    .pathfinder_graph
                    .find_area_at_point(layer as usize, candidate)
                    .is_some()
                {
                    return Some(candidate);
                }
            }
        }
        None
    }

    /// Build a movement sequence that traverses a gate path from
    /// `find_path_gates` and ends at `goal` on `goal_layer`.
    ///
    /// Unifies the three goal shapes (point, door, line) — callers
    /// pick via [`GoalShape`].
    ///
    /// For each gate in the path the emitted sub-elements depend on
    /// the gate's type:
    ///
    /// * **Jump gates** emit a single `Jump` element carrying the
    ///   source / destination `JumpLine` indices.  The tick handler
    ///   consumes those via [`EngineInner::start_jump`].
    /// * **Building doors** (previous sector `is_building()` true)
    ///   emit `WaitTimer(50)` (skipped on the first gate),
    ///   `WaitTimer(rand & 15 + rand & 15)`, and `ChangePosition` to
    ///   the gate's outside point.  The wait + teleport drives the
    ///   "actor walks inside the building and re-emerges" illusion.
    /// * **Regular doors** emit `Move` to the gate's entry point
    ///   followed by `AssertPosition` that the actor reached it.
    ///
    /// After the approach sub-elements, the door itself is crossed:
    ///
    /// * A **locked PC door** that the PC can pick (`unlockable` +
    ///   `has_lockpick`) emits `Turn` toward the lock then
    ///   `UnlockDoor` and *returns* — the door is expected to re-issue
    ///   the move once the lockpick animation terminates.
    /// * Ladder-lift sectors interpose a `WaitFreeLift` before
    ///   `PassDoor` so the climber waits for the ladder to free up.
    /// * All other doors emit `PassDoor` + `AssertPosition`.
    ///
    /// Trailing emission depends on [`GoalShape`]:
    ///
    /// * **Point goal** — emit a plain `Move` to the goal point
    ///   unless the last gate dropped the actor into a building
    ///   sector.  Skipped entirely when `move_after_last_door` is
    ///   `false` (the "walk up to the door" variant).
    /// * **Door goal** — emit the building CHANGE_POSITION or plain
    ///   MOVE to the far-side point of the goal door, then optionally
    ///   TURN + UNLOCK_DOOR for PC-lockable goal doors.
    /// * **Line goal** — emit a plain `Move` to the line's midpoint
    ///   carrying `MoveFlags::LINE` and the line id so the actor's
    ///   arrival check snaps to line tolerance.  Intermediate gate
    ///   moves never carry `MoveFlags::LINE`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build_gate_movement_sequence(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        entity_id: EntityId,
        gate_path: Vec<crate::gate::GatePathStep>,
        goal: GoalShape,
        goal_layer: u16,
        base_action: OrderType,
        move_after_last_door: bool,
        speed_factor: f32,
        initial_flags: crate::sequence::MoveFlags,
        prefix_elements: Vec<crate::sequence::SequenceElement>,
        tail_elements: Vec<crate::sequence::SequenceElement>,
        append_arrival_speech: bool,
        append_recovery: bool,
    ) -> Option<crate::sequence::SequenceId> {
        use crate::element::Command;
        use crate::sequence::{
            Field, FieldValue, MoveFlags, Sequence, SequenceElement, SequenceElementData,
        };

        // Determine first jump gate.  Every gate *before* the first
        // jump gets the `TO_JUMP` flag so its movement element sets
        // the actor up for the jump.  Only the Point and Door goal
        // variants apply this flag-mutation; the Line variant passes
        // the input flags to every gate unmodified, so suppress the
        // OR for `GoalShape::Line`.
        let apply_to_jump = !matches!(goal, GoalShape::Line { .. });
        let first_jump: Option<usize> = if apply_to_jump {
            gate_path.iter().enumerate().find_map(|(i, step)| {
                let is_jump = self
                    .scripts
                    .mission
                    .as_ref()
                    .and_then(|_| {
                        self.script_domains
                            .interactables
                            .doors
                            .get(usize::from(step.door_index))
                    })
                    .map(|d| d.is_jump())
                    .unwrap_or(false);
                if is_jump { Some(i) } else { None }
            })
        } else {
            None
        };

        let flags_at = |gate_idx: usize| -> MoveFlags {
            match first_jump {
                Some(j) if gate_idx <= j => initial_flags | MoveFlags::TO_JUMP,
                _ => initial_flags,
            }
        };

        // Snapshot the canonical gate data in one short borrow so the main
        // loop can call grid and sequence helpers on `self` without fighting
        // the borrow checker.
        #[derive(Clone, Copy)]
        struct GateShot {
            door_index: crate::gate::DoorIndex,
            direct: bool,
            // Geometry used by the emitted sub-elements.
            entry: MapPoint,
            exit: MapPoint,
            entry_layer: u16,
            exit_layer: u16,
            // Where the actor ends up *after* crossing.
            new_sector: u16,
            // Gate typing.
            is_jump: bool,
            jump_line_src: Option<crate::jump_line::JumpLineIndex>,
            jump_line_dst: Option<crate::jump_line::JumpLineIndex>,
            // Door typing (only meaningful when !is_jump).
            is_locked_pc_unlockable: bool,
            // Original RHsequence.cpp keeps the caller's action on
            // gate approach, WAIT_FREE_LIFT, PASS_DOOR, and post-pass
            // asserts.  Door-specific GetAction1/2 calls exist in
            // original-code but are commented out at execution time.
            entry_action: OrderType,
            door_action: OrderType,
        }

        let (gate_shots, starting_sector) = {
            if self.scripts.mission.is_none() {
                return None;
            }
            // Starting sector = the sector on the "old" side of the
            // first gate.  Needed to decide whether the actor's
            // current location is a building (→ first gate uses the
            // CHANGE_POSITION branch) or not.
            let start_sector = gate_path
                .first()
                .and_then(|first| {
                    self.script_domains
                        .interactables
                        .doors
                        .get(usize::from(first.door_index))
                })
                .map(|d| {
                    if gate_path[0].direct {
                        u16::from(d.sector_out)
                    } else {
                        u16::from(d.sector_in)
                    }
                });
            let shots: Vec<GateShot> = gate_path
                .iter()
                .filter_map(|step| {
                    let door = self
                        .script_domains
                        .interactables
                        .doors
                        .get(usize::from(step.door_index))?;
                    let (entry, exit, entry_layer, exit_layer, new_sector) = if step.direct {
                        (
                            door.point_out,
                            door.point_in,
                            door.layer_out,
                            door.layer_in,
                            u16::from(door.sector_in),
                        )
                    } else {
                        (
                            door.point_in,
                            door.point_out,
                            door.layer_in,
                            door.layer_out,
                            u16::from(door.sector_out),
                        )
                    };
                    let is_jump = door.is_jump();
                    let (jump_src, jump_dst) = if is_jump {
                        let (s, d) = if step.direct {
                            (door.jump_line_out, door.jump_line_in)
                        } else {
                            (door.jump_line_in, door.jump_line_out)
                        };
                        (
                            s.and_then(crate::jump_line::JumpLineIndex::new),
                            d.and_then(crate::jump_line::JumpLineIndex::new),
                        )
                    } else {
                        (None, None)
                    };
                    let is_locked_pc_unlockable = !is_jump && door.locked_pc && door.unlockable;
                    let (entry_action, door_action) = (base_action, base_action);
                    Some(GateShot {
                        door_index: step.door_index,
                        direct: step.direct,
                        entry,
                        exit,
                        entry_layer,
                        exit_layer,
                        new_sector,
                        is_jump,
                        jump_line_src: jump_src,
                        jump_line_dst: jump_dst,
                        is_locked_pc_unlockable,
                        entry_action,
                        door_action,
                    })
                })
                .collect();
            (shots, start_sector)
        }; // host borrow dropped here

        // An empty gate path is still a valid route when source adaptation
        // (for example, an actor currently straddling a door) places the
        // effective source in the goal sector. Keep building the sequence
        // so its trailing direct MOVE is emitted.

        // Does the entity have the lockpick contextual action?
        // Needed to choose the lockpick sub-element branch.
        let has_lockpick = self
            .get_entity(entity_id)
            .map(|e| e.actor_auth_info().has_lockpick)
            .unwrap_or(false);

        // Resolve sector → is_building via the fast grid.  Returns
        // false for unknown sectors (treat motion areas as
        // non-building).
        let is_building_sector = |this: &Self, sector: u16| -> bool {
            this.grid_sector_by_number(crate::sector::SectorNumber::new(sector as i16))
                .map(|gs| gs.sector_type.is_building())
                .unwrap_or(false)
        };

        let is_ladder_lift = |this: &Self, sector: u16| -> bool {
            this.grid_sector_by_number(crate::sector::SectorNumber::new(sector as i16))
                .and_then(|gs| gs.lift_type)
                .map(|lt| lt == crate::sector::LiftType::Ladder)
                .unwrap_or(false)
        };

        let mut seq = Sequence::new();
        let mut level: u16 = 1;

        for mut elem in prefix_elements {
            elem.command_level = level;
            seq.append_element(elem);
            level += 1;
        }

        // Track the "previous" sector so each gate knows what it's
        // coming *from*.  After the first gate, this is the
        // previous gate's `new_sector`.
        let mut prev_sector: Option<u16> = starting_sector;

        // Cross-sector source-sector sanity assert.  When the goal
        // sector differs from the source, prepend an `AssertPosition`
        // against the source sector so the actor's location is
        // re-validated right before the gate walk begins; if the actor
        // was nudged out between scheduling and dispatch the sequence
        // aborts gracefully instead of following a stale path.  This
        // unified builder is only invoked for cross-sector traversals
        // (callers handle same-sector inline), so emit unconditionally.
        if let Some(src_sector) = starting_sector {
            let mut leading_ap = SequenceElement::new_movement(
                level,
                Command::AssertPosition,
                Some(entity_id),
                base_action,
            );
            leading_ap.data = SequenceElementData::Movement {
                destination: crate::coordinates::MapPoint::default(),
                layer: 0,
                sector: crate::position_interface::SectorHandle::new(src_sector),
                gate_id: None,
                line_id: None,
                element: Some(entity_id),
                flags: MoveFlags::empty(),
                // Original leading cross-sector ASSERT_POSITION uses
                // the constructor default tolerance (0).  Gate-entry
                // and post-pass asserts explicitly pass 10.
                tolerance: 0.0,
                direction: 0,
                action: base_action,
                speed_factor,
                post_seek_sequence: None,
            };
            seq.append_element(leading_ap);
            level += 1;
        }

        let entity_goal = match goal {
            GoalShape::Seek {
                target, tolerance, ..
            } => Some((target, tolerance, true)),
            GoalShape::Target {
                target, tolerance, ..
            } => Some((target, tolerance, false)),
            _ => None,
        };

        // Goal-point used for the trailing MOVE (if any).  For Point
        // goals this is the caller's point; for Line goals it's the
        // line's midpoint; for Door goals it's the approach point on
        // the near side of the goal door.
        let goal_point = goal.goal_point();

        // Tracks whether a lockpick branch on an intermediate gate
        // terminated the sequence early.
        let mut ended_early = false;

        // Element count captured after the leading AssertPosition (if
        // any) and used by the building-source branch to skip the
        // 50-frame WaitTimer on the first gate's emission.
        let first_gate_element_count = seq.elements.len();

        for (gate_idx, shot) in gate_shots.iter().enumerate() {
            let gate_flags = flags_at(gate_idx);
            // -------- Gate approach branch --------
            //
            // Doors and jumps both first move to the gate's source
            // point (or CHANGE_POSITION out of a building), matching
            // original AppendMoveToSequence.  The door-vs-jump split
            // happens after this approach.
            let old_is_building = prev_sector
                .map(|s| is_building_sector(self, s))
                .unwrap_or(false);

            // Original sequence construction uses the caller's action
            // for both approach and door-pass sub-elements.
            let entry_action = shot.entry_action;
            let door_action = shot.door_action;

            if old_is_building {
                // When the previous sector is a building, the actor
                // "walks inside" by waiting out a timer then
                // teleporting to the gate's outside point.  Two
                // WaitTimer elements: the 50-frame one is only added
                // when there was already a prior gate-emitted element
                // (so the very first gate skips it).
                let wait_command = if matches!(goal, GoalShape::Line { .. }) {
                    Command::Wait
                } else {
                    Command::WaitTimer
                };
                if seq.elements.len() != first_gate_element_count {
                    let mut w = SequenceElement::new_generic(level, wait_command, Some(entity_id));
                    w.set_property(Field::Timer, FieldValue::Integer(50));
                    seq.append_element(w);
                    level += 1;
                }
                // Original: `RHSequence::AppendMoveToSequence` in
                // `original-code/RHsequence.cpp:484` sums two `rand() & 15`
                // draws for this building-exit wait.
                let r: u32 = crate::sim_rng::u32(
                    sim,
                    crate::sim_rng::RngSite::RuntimeBuildingExitWait,
                    0..16,
                ) + crate::sim_rng::u32(
                    sim,
                    crate::sim_rng::RngSite::RuntimeBuildingExitWait,
                    0..16,
                );
                let mut w = SequenceElement::new_generic(level, wait_command, Some(entity_id));
                w.set_property(Field::Timer, FieldValue::Integer(r));
                seq.append_element(w);
                level += 1;

                // CHANGE_POSITION — instant teleport to the gate's
                // "outside" point (the `entry` in our direction).
                // Compute a 0..15 direction from (exit - entry) so
                // the sprite is facing the exit.  We stuff that into
                // the element's direction field for the tick handler
                // to apply.
                let dx = shot.exit.x - shot.entry.x;
                let dy = shot.exit.y - shot.entry.y;
                let dir = crate::position_interface::vector_to_sector_0_to_15(dx, dy);
                let mut cp = SequenceElement::new_movement(
                    level,
                    Command::ChangePosition,
                    Some(entity_id),
                    entry_action,
                );
                cp.data = SequenceElementData::Movement {
                    destination: shot.entry,
                    layer: shot.entry_layer,
                    // Assert actor is still in the building sector
                    // before teleporting.  Building teleport is an
                    // in-sector position change, not a door-pass, so
                    // no gate ref is attached.
                    sector: prev_sector.and_then(crate::position_interface::SectorHandle::new),
                    gate_id: None,
                    line_id: None,
                    element: None,
                    flags: gate_flags,
                    tolerance: 0.0,
                    direction: dir,
                    action: entry_action,
                    speed_factor,
                    post_seek_sequence: None,
                };
                seq.append_element(cp);
                level += 1;
            } else {
                // MOVE to gate entry point on the source side.
                let gate_seek_target = entity_goal.map(|(target, _, _)| target);
                let mut m = SequenceElement::new_movement(
                    level,
                    Command::Move,
                    Some(entity_id),
                    entry_action,
                );
                m.data = SequenceElementData::Movement {
                    destination: shot.entry,
                    layer: 0,
                    sector: None,
                    // Original gate-approach MOVE uses the plain
                    // point+victim constructor and does not SetGate;
                    // only WAIT_FREE_LIFT/PASS_DOOR carry the gate.
                    gate_id: None,
                    line_id: None,
                    element: gate_seek_target,
                    flags: gate_flags,
                    // Original AppendMoveToSequence passes the
                    // seek victim through to gate-approach moves but
                    // uses tolerance 0; fTolerance belongs to the
                    // final goal/seek move.
                    tolerance: 0.0,
                    direction: 0,
                    action: entry_action,
                    speed_factor,
                    post_seek_sequence: None,
                };
                seq.append_element(m);
                level += 1;

                // ASSERT_POSITION that the actor actually reached
                // the gate.  Tolerance is 10.
                let mut ap = SequenceElement::new_movement(
                    level,
                    Command::AssertPosition,
                    Some(entity_id),
                    entry_action,
                );
                ap.data = SequenceElementData::Movement {
                    destination: shot.entry,
                    layer: 0,
                    sector: None,
                    gate_id: None,
                    line_id: None,
                    element: Some(entity_id),
                    flags: MoveFlags::empty(),
                    tolerance: 10.0,
                    direction: 0,
                    action: entry_action,
                    speed_factor,
                    post_seek_sequence: None,
                };
                seq.append_element(ap);
                level += 1;
            }

            // -------- Jump gate branch --------
            //
            // After the approach/assert above, a jump gate emits a
            // single `Jump` generic element carrying the source and
            // destination jump-line indices.  The tick handler
            // consumes these in `start_jump`.
            if shot.is_jump {
                let (src, dst) = match (shot.jump_line_src, shot.jump_line_dst) {
                    (Some(s), Some(d)) => (s, d),
                    _ => {
                        tracing::warn!(
                            gate = %shot.door_index,
                            "Jump gate missing jump_line indices; skipping jump element"
                        );
                        prev_sector = Some(shot.new_sector);
                        continue;
                    }
                };
                let mut jump_elem =
                    SequenceElement::new_generic(level, Command::JumpCmd, Some(entity_id));
                jump_elem.set_property(Field::JumplineSource, FieldValue::LineId(src));
                jump_elem.set_property(Field::JumplineDestination, FieldValue::LineId(dst));
                seq.append_element(jump_elem);
                level += 1;
                prev_sector = Some(shot.new_sector);
                continue;
            }

            // -------- Lockpick branch --------
            //
            // When the door is PC-locked and the PC has the lockpick
            // action, the sequence terminates after TURN + UNLOCK_DOOR
            // — the unlock animation flips `locked_pc` off and the
            // caller re-issues the move command to resume the path.
            if shot.is_locked_pc_unlockable && has_lockpick {
                // TURN toward the gate entry (point_in for direct,
                // point_out for indirect), so the sprite faces the
                // lock while picking it.
                let camera_pt = if shot.direct { shot.exit } else { shot.entry };
                let mut turn = SequenceElement::new_generic(level, Command::Turn, Some(entity_id));
                turn.set_property(
                    Field::CameraPoint,
                    FieldValue::GeoPoint2D {
                        x: camera_pt.x,
                        y: camera_pt.y,
                    },
                );
                seq.append_element(turn);
                level += 1;

                // UNLOCK_DOOR — the tick handler reads the door id
                // from `Field::Door` and picks UnlockingDoor vs
                // UnlockingTrap from the door table on its own.
                let mut unlock =
                    SequenceElement::new_generic(level, Command::UnlockDoor, Some(entity_id));
                unlock.set_property(Field::Door, FieldValue::DoorId(shot.door_index));
                seq.append_element(unlock);
                level += 1;

                // Early return — the lockpick animation will re-issue
                // the move once it terminates.
                ended_early = true;
                break;
            }

            // -------- Ladder lift wait --------
            if is_ladder_lift(self, shot.new_sector) {
                let mut wait = SequenceElement::new_movement(
                    level,
                    Command::WaitFreeLift,
                    Some(entity_id),
                    door_action,
                );
                wait.data = SequenceElementData::Movement {
                    destination: crate::coordinates::MapPoint::default(),
                    layer: 0,
                    sector: crate::position_interface::SectorHandle::new(shot.new_sector),
                    gate_id: Some(shot.door_index),
                    line_id: None,
                    element: None,
                    flags: MoveFlags::empty(),
                    tolerance: 0.0,
                    direction: 0,
                    action: door_action,
                    speed_factor,
                    post_seek_sequence: None,
                };
                seq.append_element(wait);
                level += 1;
            }

            // -------- PASS_DOOR + post-pass assert --------
            let mut pass = SequenceElement::new_movement(
                level,
                Command::PassDoor,
                Some(entity_id),
                door_action,
            );
            pass.data = SequenceElementData::Movement {
                destination: shot.exit,
                layer: shot.exit_layer,
                sector: None,
                gate_id: Some(shot.door_index),
                line_id: None,
                element: None,
                // Original PASS_DOOR constructor uses default flags
                // and only attaches the gate via SetGate.
                flags: MoveFlags::empty(),
                tolerance: 0.0,
                direction: 0,
                action: door_action,
                speed_factor,
                post_seek_sequence: None,
            };
            seq.append_element(pass);
            level += 1;

            // ASSERT_POSITION that the actor reached the exit point.
            let mut ap = SequenceElement::new_movement(
                level,
                Command::AssertPosition,
                Some(entity_id),
                door_action,
            );
            ap.data = SequenceElementData::Movement {
                destination: shot.exit,
                layer: 0,
                sector: None,
                gate_id: None,
                line_id: None,
                element: Some(entity_id),
                flags: MoveFlags::empty(),
                tolerance: 10.0,
                direction: 0,
                action: door_action,
                speed_factor,
                post_seek_sequence: None,
            };
            seq.append_element(ap);
            level += 1;

            prev_sector = Some(shot.new_sector);
        }

        // Clear TO_JUMP once we're past the last jump gate — the
        // trailing MOVE uses `initial_flags` unmodified.
        let trailing_flags = initial_flags;

        // Trailing emission.  Three goal shapes, three branches:
        //
        // * Point: emit MOVE to `goal_point`, subject to
        //   `move_after_last_door` and the building-sector
        //   short-circuit.
        // * Door: handle the goal-door's approach / CHANGE_POSITION
        //   into building / PC-lockpick tail.
        // * Line: emit MOVE with `MoveFlags::LINE` + `line_id` so
        //   arrival snaps to the line's tolerance window.
        if !ended_early {
            let last_into_building = prev_sector
                .map(|s| is_building_sector(self, s))
                .unwrap_or(false);

            match goal {
                GoalShape::Point { .. } | GoalShape::Seek { .. } | GoalShape::Target { .. } => {
                    if move_after_last_door && !last_into_building {
                        let (seek_target, seek_tolerance, seek_flags) = match goal {
                            GoalShape::Seek {
                                target, tolerance, ..
                            } => (Some(target), tolerance, trailing_flags | MoveFlags::SEEK),
                            GoalShape::Target {
                                target, tolerance, ..
                            } => (Some(target), tolerance, trailing_flags),
                            GoalShape::Point { tolerance, .. } => (None, tolerance, trailing_flags),
                            _ => unreachable!("point/entity trailing branch"),
                        };
                        let mut final_move = SequenceElement::new_movement(
                            level,
                            Command::Move,
                            Some(entity_id),
                            base_action,
                        );
                        final_move.data = SequenceElementData::Movement {
                            destination: goal_point,
                            layer: goal_layer,
                            sector: None,
                            gate_id: None,
                            line_id: None,
                            element: seek_target,
                            flags: seek_flags,
                            tolerance: seek_tolerance,
                            direction: 0,
                            action: base_action,
                            speed_factor,
                            post_seek_sequence: None,
                        };
                        seq.append_element(final_move);
                        level += 1;
                    }

                    // When SEEK is set and the last gate landed us
                    // inside a building sector, emit a trailing MOVE
                    // back to the last gate's `point_in` so the actor
                    // doesn't get stuck at the interior teleport point.
                    if last_into_building
                        && initial_flags.contains(MoveFlags::SEEK)
                        && let Some(last_shot) = gate_shots.last()
                    {
                        let (seek_target, seek_tolerance, seek_flags) = entity_goal
                            .map(|(target, tolerance, is_seek)| {
                                (
                                    Some(target),
                                    tolerance,
                                    if is_seek {
                                        trailing_flags | MoveFlags::SEEK
                                    } else {
                                        trailing_flags
                                    },
                                )
                            })
                            .unwrap_or((None, 0.0, trailing_flags));
                        let point_in = {
                            self.scripts
                                .mission
                                .as_ref()
                                .and_then(|_| {
                                    self.script_domains
                                        .interactables
                                        .doors
                                        .get(usize::from(last_shot.door_index))
                                })
                                .map(|d| d.point_in)
                                .unwrap_or(last_shot.exit)
                        };
                        let mut seek_move = SequenceElement::new_movement(
                            level,
                            Command::Move,
                            Some(entity_id),
                            base_action,
                        );
                        seek_move.data = SequenceElementData::Movement {
                            destination: point_in,
                            layer: goal_layer,
                            sector: None,
                            gate_id: None,
                            line_id: None,
                            element: seek_target,
                            flags: seek_flags,
                            tolerance: seek_tolerance,
                            direction: 0,
                            action: base_action,
                            speed_factor,
                            post_seek_sequence: None,
                        };
                        seq.append_element(seek_move);
                    }
                }
                GoalShape::Line {
                    line_index,
                    tolerance,
                    ..
                } => {
                    // Emit `Move` to the line goal with
                    // `MoveFlags::LINE` and the line id.  When the
                    // last gate landed in a building, bail out without
                    // emitting.
                    if !last_into_building {
                        let mut final_move = SequenceElement::new_movement(
                            level,
                            Command::Move,
                            Some(entity_id),
                            base_action,
                        );
                        final_move.data = SequenceElementData::Movement {
                            destination: goal_point,
                            layer: goal_layer,
                            sector: None,
                            gate_id: None,
                            line_id: Some(line_index),
                            element: None,
                            flags: trailing_flags | MoveFlags::LINE,
                            tolerance,
                            direction: 0,
                            action: base_action,
                            speed_factor,
                            post_seek_sequence: None,
                        };
                        seq.append_element(final_move);
                    }
                }
                GoalShape::Door {
                    door_index,
                    far_side_point,
                    far_side_layer,
                    far_side_is_building,
                } => {
                    // Hoist the goal door's PC-lockable lookup so the
                    // trailing lockpick tail fires regardless of which
                    // trailing branch was taken.  The lockpick tail is
                    // unconditional on the goal door's PC-locked flag,
                    // not on which branch (building vs non-building)
                    // was selected.
                    let goal_door_pc_lockable = {
                        self.scripts
                            .mission
                            .as_ref()
                            .and_then(|_| {
                                self.script_domains
                                    .interactables
                                    .doors
                                    .get(usize::from(door_index))
                            })
                            .map(|d| d.locked_pc && d.unlockable)
                            .unwrap_or(false)
                    };
                    if !move_after_last_door {
                        // "Stop at the door" variant — caller set
                        // `move_after_last_door=false` to skip the
                        // trailing MOVE.  The gate-path includes the
                        // goal door as the last gate, so the loop
                        // already emitted approach + PASS_DOOR for it.
                        // Nothing to emit here.
                    } else if far_side_is_building {
                        // Random 0..30 frames wait + CHANGE_POSITION
                        // teleport into the building interior. Original:
                        // `original-code/RHsequence.cpp:905`. The
                        // direction stuffed on the element is the
                        // door's `point_out - point_in` sector-index.
                        let r: u32 = crate::sim_rng::u32(
                            sim,
                            crate::sim_rng::RngSite::RuntimeBuildingExitWait,
                            0..16,
                        ) + crate::sim_rng::u32(
                            sim,
                            crate::sim_rng::RngSite::RuntimeBuildingExitWait,
                            0..16,
                        );
                        let mut wait = SequenceElement::new_generic(
                            level,
                            Command::WaitTimer,
                            Some(entity_id),
                        );
                        wait.set_property(Field::Timer, FieldValue::Integer(r));
                        seq.append_element(wait);
                        level += 1;

                        let (dx, dy) = {
                            let d = self.scripts.mission.as_ref().and_then(|_| {
                                self.script_domains
                                    .interactables
                                    .doors
                                    .get(usize::from(door_index))
                            });
                            match d {
                                Some(d) => {
                                    (d.point_out.x - d.point_in.x, d.point_out.y - d.point_in.y)
                                }
                                None => (0.0, 0.0),
                            }
                        };
                        let dir = vector_to_sector_0_to_15(dx, dy);
                        let mut cp = SequenceElement::new_movement(
                            level,
                            Command::ChangePosition,
                            Some(entity_id),
                            base_action,
                        );
                        cp.data = SequenceElementData::Movement {
                            destination: far_side_point,
                            layer: far_side_layer,
                            sector: prev_sector
                                .and_then(crate::position_interface::SectorHandle::new),
                            // No gate ref on the building-interior
                            // CHANGE_POSITION (it is an in-sector
                            // teleport).
                            gate_id: None,
                            line_id: None,
                            element: None,
                            flags: trailing_flags,
                            tolerance: 0.0,
                            direction: dir,
                            action: base_action,
                            speed_factor,
                            post_seek_sequence: None,
                        };
                        seq.append_element(cp);
                        level += 1;
                    } else {
                        // Plain MOVE to the goal door's far-side
                        // point.  No `last_into_building` guard here —
                        // the trailing MOVE fires unconditionally for
                        // non-building goal doors.
                        let mut final_move = SequenceElement::new_movement(
                            level,
                            Command::Move,
                            Some(entity_id),
                            base_action,
                        );
                        final_move.data = SequenceElementData::Movement {
                            destination: far_side_point,
                            layer: far_side_layer,
                            sector: None,
                            // Original AppendMoveToDoorToSequence's
                            // trailing goal move is a plain MOVE to
                            // ptGoal; it does not call SetGate.
                            gate_id: None,
                            line_id: None,
                            element: None,
                            flags: trailing_flags,
                            tolerance: 0.0,
                            direction: 0,
                            action: base_action,
                            speed_factor,
                            post_seek_sequence: None,
                        };
                        seq.append_element(final_move);
                        level += 1;
                    }

                    // After the trailing MOVE / CHANGE_POSITION, if
                    // the goal door is PC-lockable and the actor has
                    // lockpick, emit TURN toward the lock +
                    // UNLOCK_DOOR.  The goal door is *not* included in
                    // `gate_path` for the door-goal case
                    // (`find_path_to_door` pops it), so the in-loop
                    // lockpick branch didn't fire for it — this is
                    // where the "walk up to door and pick it" finale
                    // is emitted.
                    if goal_door_pc_lockable && has_lockpick {
                        let (cam_pt, direct) = {
                            let d = self.scripts.mission.as_ref().and_then(|_| {
                                self.script_domains
                                    .interactables
                                    .doors
                                    .get(usize::from(door_index))
                            });
                            // Use the path-direction the gate was
                            // approached in.  When the goal door was
                            // excluded from `gate_path` the caller
                            // signals that direction implicitly via
                            // `far_side_point` — it matches the
                            // door's near-side endpoint for the
                            // approach side.  Recover the direction
                            // by comparing endpoints.
                            let direct = d
                                .map(|d| {
                                    let dx = far_side_point.x - d.point_out.x;
                                    let dy = far_side_point.y - d.point_out.y;
                                    (dx * dx + dy * dy) < 1e-4
                                })
                                .unwrap_or(true);
                            let cam = d
                                .map(|d| if direct { d.point_in } else { d.point_out })
                                .unwrap_or(far_side_point);
                            (cam, direct)
                        };
                        let _ = direct;
                        let mut turn =
                            SequenceElement::new_generic(level, Command::Turn, Some(entity_id));
                        turn.set_property(
                            Field::CameraPoint,
                            FieldValue::GeoPoint2D {
                                x: cam_pt.x,
                                y: cam_pt.y,
                            },
                        );
                        seq.append_element(turn);
                        level += 1;

                        let mut unlock = SequenceElement::new_generic(
                            level,
                            Command::UnlockDoor,
                            Some(entity_id),
                        );
                        unlock.set_property(Field::Door, FieldValue::DoorId(door_index));
                        seq.append_element(unlock);
                    }
                }
            }
        }

        for mut elem in tail_elements {
            elem.command_level = level;
            seq.append_element(elem);
            level += 1;
        }

        // Append a `SpeakHeroReachDestination` element at the tail of
        // the gate-movement sequence so the PC barks the "I have
        // arrived" line once the destination is reached.  Dispatched
        // at the same command_level as the last real element; the
        // PC's `Instruct` override terminates it on dispatch and
        // queues `HeroDoneCommand` via `arbitrate_instruct`.
        if append_arrival_speech && !seq.is_empty() {
            let speak_level = seq.last().map(|e| e.command_level).unwrap_or(level);
            let speak = SequenceElement::new(
                speak_level,
                Command::SpeakHeroReachDestination,
                Some(entity_id),
            );
            seq.append_element(speak);
        }

        // Append posture-recovery sub-elements right after the Speak
        // element so a PC mid-bow-aim / crouched / helping-climb /
        // simulating-beggar ends the order in a neutral posture
        // instead of frozen in their pre-move state.  Only fires for
        // PCs; `append_posture_recovery` bails on non-PC entities.
        if append_recovery {
            self.append_posture_recovery(entity_id, &mut seq);
        }

        let seq_id = self.launch_sequence(seq);
        tracing::trace!(
            entity = ?entity_id,
            ?seq_id,
            gates = gate_path.len(),
            early = ended_early,
            goal = ?goal,
            move_after_last_door,
            "Launched gate-traversal movement sequence"
        );

        // Destination markers are emitted by player group-move callers
        // only; AI/pathfinding callers use this helper without dropping
        // a ground mark.
        Some(seq_id)
    }

    /// Append posture-cleanup sub-elements at the tail of a PC move
    /// sequence so the PC ends the order in a neutral posture rather
    /// than frozen in the pre-move state.
    ///
    /// Covers:
    ///
    /// * **Shoot-bow drain** — if the sequence currently ends with a
    ///   `Command::ShootBow` element *and* the PC is no longer aiming,
    ///   demote that trailing element to `Command::ShootBowOnce` so the
    ///   queued shot fires exactly once before the walk resumes.
    /// * **Upright + bow-aim** → append `EQUIP_BOW` (re-arms the bow so
    ///   the aim state is re-entered after the walk).
    /// * **Crouched + last command ≠ CrouchUp** → append `CROUCH_DOWN`
    ///   so the PC re-crouches at the destination.
    /// * **HelpingToClimb** → append `ENTER_HELPING_CLIMB`.
    /// * **SimulatingBeggar** → append `ENTER_BEGGAR`.
    ///
    /// When the input sequence ends in SEEK, recovery is appended to
    /// that movement element's post-seek sub-sequence so it fires only
    /// on successful seek completion, not on seek abort.
    pub(crate) fn append_posture_recovery(
        &self,
        pc_id: EntityId,
        sequence: &mut crate::sequence::Sequence,
    ) {
        use crate::element::Command;
        let Some(entity) = self.get_entity(pc_id) else {
            return;
        };
        if !entity.is_pc() {
            return;
        }
        let posture = entity.element_data().posture;
        let Some(actor) = entity.actor_data() else {
            return;
        };
        let action_state = actor.action_state;

        // Drill into the SEEK element's post-seek sub-sequence when
        // the inbound sequence ends with `Command::Seek`; allocate a
        // fresh sub-sequence if none was attached yet, then append the
        // recovery command to it.  This way recovery fires only on
        // successful seek completion, not on seek abort.
        let target_sequence: &mut crate::sequence::Sequence = if sequence
            .last()
            .is_some_and(|last| last.command == Command::Seek)
        {
            let last_elem = sequence
                .elements
                .last_mut()
                .expect("Sequence::last() returned Some above");
            if let crate::sequence::SequenceElementData::Movement {
                post_seek_sequence, ..
            } = &mut last_elem.data
            {
                post_seek_sequence
                    .get_or_insert_with(|| Box::new(crate::sequence::Sequence::new()))
                    .as_mut()
            } else {
                sequence
            }
        } else {
            sequence
        };

        let (level, last_command) = match target_sequence.last() {
            None => (1u16, None),
            Some(last) => (last.command_level.saturating_add(1), Some(last.command)),
        };

        // "Shoot once then stop".
        if last_command == Some(Command::ShootBow) && !action_state.is_bow() {
            if let Some(last_mut) = target_sequence.elements.last_mut() {
                last_mut.command = Command::ShootBowOnce;
            }
            return;
        }

        match posture {
            crate::element::Posture::Upright if action_state.is_bow() => {
                target_sequence.append_element(crate::sequence::SequenceElement::new(
                    level,
                    Command::EquipBow,
                    Some(pc_id),
                ));
            }
            crate::element::Posture::Crouched if last_command != Some(Command::CrouchUp) => {
                target_sequence.append_element(crate::sequence::SequenceElement::new(
                    level,
                    Command::CrouchDown,
                    Some(pc_id),
                ));
            }
            crate::element::Posture::HelpingToClimb => {
                target_sequence.append_element(crate::sequence::SequenceElement::new(
                    level,
                    Command::EnterHelpingClimb,
                    Some(pc_id),
                ));
            }
            crate::element::Posture::SimulatingBeggar => {
                target_sequence.append_element(crate::sequence::SequenceElement::new(
                    level,
                    Command::EnterBeggar,
                    Some(pc_id),
                ));
            }
            _ => {}
        }
    }

    /// Enqueue an AI-initiated Move intent for this actor.
    ///
    /// Per-actor dedup: only one pending request per actor exists in
    /// the queue at any time — a later call for the same `entity_id`
    /// overwrites the earlier entry.  The actual Move element launch
    /// happens in `drain_pending_move_requests` at a deterministic
    /// point in the hourglass.
    ///
    /// This queue absorbs high-frequency AI re-fires (patrol macro-
    /// GoTo, pursuit re-pathfind) that would otherwise each spawn a
    /// fresh `Command::Move` element and `InterruptCurrent` the
    /// previous one at the same Normal priority, preventing the actor
    /// from ever completing a startup transition or making waypoint
    /// progress.
    ///
    /// Once drained, the Move element is launched via the sequence
    /// pipeline (`launch_element_for_owner` → `arbitrate_instruct` →
    /// `InstructOwner` dispatch), giving the move:
    /// * Priority arbitration — the element can be postponed behind
    ///   an in-flight `ENTER_ATTENTIVE_MODE`
    ///   (`PostponeEverythingButInjuries`) so the alerted-pose
    ///   transition finishes before the move starts.
    /// * System #16 — failed-path-impossible actually reaches the
    ///   owner via the Move's `element_impossible` condolation.
    /// * `post_process_path` on path arrival (see `tick.rs` Move
    ///   dispatch) inserts the startup-transition animation via the
    ///   normal pipeline.
    ///
    /// Run the AI `GoTo` pre-flight gates for an AI movement intent.
    /// Returns `true` if the intent should proceed to launch, `false`
    /// if it was rejected (in which case `couldnt_reachpoint` has been
    /// set on the AI controller and the caller should drop
    /// the intent).
    ///
    /// `intent.find_accessible` runs
    /// `FastFindGrid::find_authorized_position` against the actor's
    /// `MoveBox + (target_x, target_y)` and rewrites the intent target
    /// to the snapped centre on success.
    ///
    /// `intent.ask_obstacle` runs
    /// `FastFindGrid::is_straight_movement_authorized` from the
    /// actor's current position to the destination.  Only meaningful
    /// for straight moves (gated on `compute_direction == false`).
    fn preflight_ai_goto(
        &mut self,
        entity_id: EntityId,
        intent: &mut crate::order::AiOrderIntent,
    ) -> bool {
        // Upper-bound check.  `AiController::go_to` already rejects
        // `target_x <= 0 || target_y <= 0` before pushing the intent;
        // the engine drain owns the `>= GetLevelSize()` half because
        // `level_size` lives on the shared cutscene camera, not on
        // `AiContext`.
        let level_w = self.feedback.cutscene_camera.level_size.x;
        let level_h = self.feedback.cutscene_camera.level_size.y;
        if level_w > 0.0 && intent.target_x >= level_w
            || level_h > 0.0 && intent.target_y >= level_h
        {
            self.set_ai_couldnt_reachpoint(entity_id);
            return false;
        }

        if !intent.find_accessible && !intent.ask_obstacle {
            return true;
        }

        let (move_box, layer, position) = {
            let Some(entity) = self.get_entity(entity_id) else {
                return true;
            };
            let pi = entity.position_iface();
            let pm = pi.map_position();
            (*pi.get_move_box(), entity.element_data().layer(), pm)
        };

        // Snap destination to the nearest authorised position when
        // `find_accessible` is set.  Translate the move box to the
        // requested destination and ask the grid.  On success rewrite
        // the intent target to the box centre.
        if intent.find_accessible {
            let dest = MapPoint::new(intent.target_x, intent.target_y);
            let mut bbox = if move_box.is_somewhere() {
                MapBBox::from_corners(
                    MapPoint::new(move_box.x_min() + dest.x, move_box.y_min() + dest.y),
                    MapPoint::new(move_box.x_max() + dest.x, move_box.y_max() + dest.y),
                )
            } else {
                MapBBox::new()
            };
            if !self
                .world
                .fast_grid
                .find_authorized_position(&mut bbox, layer)
            {
                self.set_ai_couldnt_reachpoint(entity_id);
                return false;
            }
            let centre = bbox.center();
            intent.target_x = centre.x;
            intent.target_y = centre.y;
        }

        // Pre-flight straight movement.  Only meaningful for straight
        // moves (gated on `compute_direction == false`); when
        // `ask_obstacle` is set without straight-mode the check is
        // silently skipped rather than asserting.
        if intent.ask_obstacle && !intent.compute_direction {
            let dest = MapPoint::new(intent.target_x, intent.target_y);
            if !self
                .world
                .fast_grid
                .is_straight_movement_authorized(position, dest, layer, &move_box)
            {
                self.set_ai_couldnt_reachpoint(entity_id);
                return false;
            }
        }

        true
    }

    /// Set `AiController::couldnt_reachpoint = true` on the entity, used
    /// by the GoTo pre-flight gates to surface a same-frame failure to
    /// the AI's stuck-retry / fallback logic.
    fn set_ai_couldnt_reachpoint(&mut self, entity_id: EntityId) {
        let Some(entity) = self.world.entities.get_mut(entity_id) else {
            return;
        };
        if let Some(ai) = entity.ai_controller_mut() {
            ai.couldnt_reachpoint = true;
        }
    }

    fn launch_ai_move(&mut self, entity_id: EntityId, intent: &crate::order::AiOrderIntent) {
        // The AI think loop can legitimately emit two distinct `GoTo`
        // intents for the same actor in one tick (e.g. a SEEK retarget
        // dispatched immediately after a macro-GoTo).  `halt_actor`
        // tears down the prior Move element cleanly; here we just keep
        // the latest intent — "last intent wins", since the second
        // Halt invalidates anything the first request queued.
        let replaced = self
            .orders
            .pending_move_requests
            .iter()
            .any(|(eid, _)| *eid == entity_id);
        if replaced {
            self.orders
                .pending_move_requests
                .retain(|(eid, _)| *eid != entity_id);
            tracing::trace!(
                entity = ?entity_id,
                "launch_ai_move: replacing prior pending Move (AI re-issued GoTo this tick)"
            );
        }
        self.orders
            .pending_move_requests
            .push((entity_id, intent.clone()));
    }

    /// Drain the pending-move-request queue and launch a Move
    /// sequence element for each.  Runs once per tick from the
    /// hourglass pipeline.  Determinism: requests drain in FIFO order
    /// of enqueue (a `Vec` with `retain`+`push` on launch preserves
    /// this).
    pub(super) fn drain_pending_move_requests(&mut self, sim: &crate::sim_rng::SimulationContext) {
        let requests = std::mem::take(&mut self.orders.pending_move_requests);
        for (entity_id, intent) in requests {
            let _ = self.do_launch_ai_move(sim, entity_id, &intent);
        }
    }

    /// Launch only one owner's pending AI Move at a synchronous owner
    /// boundary. Requests belonging to other creation slots retain their FIFO
    /// positions for the normal tick drain.
    pub(super) fn drain_pending_move_requests_for_owner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        owner: EntityId,
    ) -> Vec<crate::sequence::SequenceId> {
        let requests = std::mem::take(&mut self.orders.pending_move_requests);
        let mut owner_requests = Vec::new();
        let mut remaining = Vec::with_capacity(requests.len());
        for request @ (entity_id, _) in requests {
            if entity_id == owner {
                owner_requests.push(request);
            } else {
                remaining.push(request);
            }
        }
        self.orders.pending_move_requests = remaining;
        owner_requests
            .into_iter()
            .filter_map(|(_, intent)| self.do_launch_ai_move(sim, owner, &intent))
            .collect()
    }

    /// Actually build and launch the Move sequence element for an AI
    /// intent.  Split out from `launch_ai_move` so the enqueue side
    /// can be cheap (push into a Vec) and the heavier work (resolve
    /// entity state, build element, run arbitration + path) only
    /// happens once per actor per tick at drain time.
    fn do_launch_ai_move(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        entity_id: EntityId,
        intent: &crate::order::AiOrderIntent,
    ) -> Option<crate::sequence::SequenceId> {
        let dest = crate::coordinates::MapPoint {
            x: intent.target_x,
            y: intent.target_y,
        };
        let (raw_source, raw_source_layer, raw_source_sector, door_handle, door_direction) = {
            let Some(entity) = self.get_entity(entity_id) else {
                tracing::warn!("do_launch_ai_move: entity {:?} not found", entity_id);
                return None;
            };
            let ed = entity.element_data();
            let (door_handle, door_direction) = current_door_for_route_source(entity);
            (
                ed.position_map(),
                ed.layer(),
                ed.sector(),
                door_handle,
                door_direction,
            )
        };
        // RHSequence::AppendMoveToSequence adapts a source that is currently
        // crossing a gate to the committed far side before comparing sectors
        // or searching the gate graph.
        let (source, source_layer, source_sector) = self
            .scripts
            .mission
            .as_ref()
            .and_then(|_| {
                adapt_source_to_current_door(
                    &self.script_domains.interactables.doors,
                    door_handle,
                    door_direction,
                )
            })
            .map(|(point, sector, layer)| {
                (
                    point,
                    layer,
                    crate::position_interface::SectorHandle::new(sector),
                )
            })
            .unwrap_or((raw_source, raw_source_layer, raw_source_sector));
        let goal_layer = intent.target_layer.unwrap_or(source_layer);
        let goal_sector = intent.target_sector.or(source_sector);
        let move_flags =
            crate::sequence::MoveFlags::from_bits_truncate(u32::from(intent.move_flags));

        let action = intent.order_type;
        let crosses_topology = goal_layer != source_layer || goal_sector != source_sector;
        if crosses_topology {
            let Some(source_sector) = source_sector else {
                tracing::warn!(?entity_id, "cross-sector AI GoTo has no source sector");
                self.set_ai_couldnt_reachpoint(entity_id);
                return None;
            };
            let Some(goal_sector) = goal_sector else {
                tracing::warn!(?entity_id, "cross-sector AI GoTo has no destination sector");
                self.set_ai_couldnt_reachpoint(entity_id);
                return None;
            };
            let auth = self
                .get_entity(entity_id)
                .map(|entity| entity.actor_auth_info());
            let level = self.world.fast_grid.level.clone();
            // AppendMoveToSequence treats a door sector as a door-identity
            // goal. A sector-only FindPathGates search cannot represent that
            // terminal condition because a door sector is not an ordinary
            // motion area.
            let door_goal = self
                .grid_sector_by_number(crate::sector::SectorNumber::new(
                    u16::from(goal_sector) as i16
                ))
                .filter(|sector| sector.sector_type.is_door())
                .and_then(|sector| sector.door_index)
                .map(crate::gate::DoorIndex);
            let gate_path = self.scripts.mission.as_ref().and_then(|_| {
                if let Some(door_index) = door_goal {
                    crate::gate::find_path_into_door(
                        &self.script_domains.interactables.doors,
                        (source.x, source.y),
                        u16::from(source_sector),
                        door_index,
                        auth.as_ref(),
                        move_flags.contains(crate::sequence::MoveFlags::MAP),
                        &|sector| self.building_sector_is_authorized(sector),
                        &|sector| {
                            level
                                .sectors
                                .iter()
                                .find(|candidate| candidate.sector_number == sector)
                                .and_then(|candidate| candidate.lift_type)
                        },
                    )
                } else {
                    crate::gate::find_path_gates(
                        &self.script_domains.interactables.doors,
                        (source.x, source.y),
                        u16::from(source_sector),
                        (dest.x, dest.y),
                        u16::from(goal_sector),
                        auth.as_ref(),
                        move_flags.contains(crate::sequence::MoveFlags::MAP),
                        &|sector| {
                            level
                                .sectors
                                .iter()
                                .find(|candidate| candidate.sector_number == sector)
                                .and_then(|candidate| candidate.lift_type)
                        },
                    )
                }
            });
            let Some(gate_path) = gate_path else {
                tracing::warn!(
                    ?entity_id,
                    source_sector = u16::from(source_sector),
                    source_layer,
                    goal_sector = u16::from(goal_sector),
                    goal_layer,
                    "cross-sector AI GoTo has no gate route"
                );
                self.set_ai_couldnt_reachpoint(entity_id);
                return None;
            };
            let prefix = if intent.quit_swordfight_before_move {
                vec![crate::sequence::SequenceElement::new(
                    1,
                    crate::element::Command::QuitSwordfight,
                    Some(entity_id),
                )]
            } else {
                Vec::new()
            };
            let tail = self.ai_special_action_tail(entity_id, intent);
            let goal = door_goal.map_or(
                GoalShape::Point {
                    point: dest,
                    tolerance: intent.tolerance,
                },
                |door_index| GoalShape::Door {
                    door_index,
                    // These fields only serve the move-after-last-door
                    // variant. AppendMoveToSequence sets that false for a
                    // door-sector goal because the gate path is inclusive.
                    far_side_point: dest,
                    far_side_layer: goal_layer,
                    far_side_is_building: false,
                },
            );
            return self.build_gate_movement_sequence(
                sim,
                entity_id,
                gate_path,
                goal,
                goal_layer,
                action,
                door_goal.is_none(),
                intent.speed_factor,
                move_flags,
                prefix,
                tail,
                false,
                false,
            );
        }

        let move_level = if intent.quit_swordfight_before_move {
            2
        } else {
            1
        };
        let mut elem = crate::sequence::SequenceElement::new_movement(
            move_level,
            crate::element::Command::Move,
            Some(entity_id),
            action,
        );
        elem.retained_movement_goal = intent.retained_movement_goal;
        if let crate::sequence::SequenceElementData::Movement {
            destination,
            layer: elem_layer,
            sector: elem_sector,
            flags,
            tolerance,
            element,
            speed_factor,
            ..
        } = &mut elem.data
        {
            *destination = dest;
            *elem_layer = goal_layer;
            *elem_sector = goal_sector;
            *flags = move_flags;
            *tolerance = intent.tolerance;
            *element = intent.antagonist;
            *speed_factor = intent.speed_factor;
        }

        // Promotion creates the Original sequence-manager work item but does
        // not run the owner's Instruct yet. Normal frame and patrol drains
        // leave it queued until SequenceManager::Hourglass; script-native and
        // condolence call sites that require re-entrant dispatch explicitly
        // take this exact deferred action immediately after this returns.
        let mut sequence = crate::sequence::Sequence::new();
        if intent.quit_swordfight_before_move {
            sequence.append_element(crate::sequence::SequenceElement::new(
                1,
                crate::element::Command::QuitSwordfight,
                Some(entity_id),
            ));
        }
        sequence.append_element(elem);
        for mut tail in self.ai_special_action_tail(entity_id, intent) {
            tail.command_level = sequence
                .last()
                .map_or(1, |element| element.command_level.saturating_add(1));
            sequence.append_element(tail);
        }
        let sequence_id = self.launch_sequence(sequence);

        tracing::trace!(
            entity = ?entity_id,
            dest_x = dest.x,
            dest_y = dest.y,
            ?action,
            move_flags = intent.move_flags,
            "AI movement launched via sequence element"
        );
        Some(sequence_id)
    }

    /// Build the exact tail authored by
    /// `RHArtificialIntelligence::GoTo(..., GOTO_SPECIAL_ACTION)`.
    ///
    /// These are part of the movement sequence, not follow-up AI work. That
    /// distinction keeps the Move from being the last real action: its
    /// condolence must not emit EventReachPoint until the final SitDown /
    /// EnterLeisure element terminates.
    fn ai_special_action_tail(
        &self,
        entity_id: EntityId,
        intent: &crate::order::AiOrderIntent,
    ) -> Vec<crate::sequence::SequenceElement> {
        if !intent.append_special_action_tail {
            return Vec::new();
        }
        let ai = self
            .world
            .entities
            .get(entity_id)
            .and_then(|entity| entity.ai_controller())
            .unwrap_or_else(|| {
                panic!("GOTO_SPECIAL_ACTION movement owner {entity_id:?} lost its AI controller")
            });
        let direction = ai.initial_view_direction;

        let mut turn = crate::sequence::SequenceElement::new_generic(
            0,
            crate::element::Command::Turn,
            Some(entity_id),
        );
        turn.set_property(
            crate::sequence::Field::Direction,
            crate::sequence::FieldValue::Integer(u32::from(direction)),
        );
        let posture_command = if ai.special_action {
            crate::element::Command::EnterLeisure
        } else {
            crate::element::Command::SitDown
        };
        vec![
            turn,
            crate::sequence::SequenceElement::new(0, posture_command, Some(entity_id)),
        ]
    }

    /// Execute the selected movement arm for one live actor owner.
    ///
    /// Mutable inputs are sampled at this owner's legacy slot. Movement and
    /// its Execute-arm callbacks, completion, and condolence continuation are
    /// applied synchronously before this function returns; no owner result
    /// vectors escape to a later global dispatch pass.
    fn turn_globally_frozen_climb_owner(
        &mut self,
        owner: EntityId,
        selected: MovementOwnerSelection,
    ) {
        let order_action = self
            .orders
            .sequence_manager
            .get_element(selected.seq_id, selected.elem_idx)
            .and_then(|element| element.current_order())
            .filter(|order| order.order_id == selected.order_id)
            .map(|order| order.order_type)
            .expect("globally frozen movement owner lost its selected order");
        let (action, door_index, current_sector, execute_order_initialising) = self
            .world
            .entities
            .get(owner)
            .and_then(|entity| {
                let actor = entity.actor_data()?;
                Some((
                    actor
                        .active_door_pass
                        .as_ref()
                        .map_or(order_action, |pass| pass.current_action),
                    actor.active_door_pass.as_ref().map(|pass| pass.door_index),
                    entity.element_data().sector(),
                    actor.execute_order_initialising,
                ))
            })
            .unwrap_or_else(|| panic!("globally frozen movement owner {owner:?} is not an actor"));
        let Some(expected_lift_type) = climb_lift_type(action) else {
            return;
        };

        let sector_number = if let Some(door_index) = door_index {
            self.script_domains
                .interactables
                .doors
                .get(usize::from(door_index))
                .unwrap_or_else(|| {
                    panic!(
                        "globally frozen climb owner {owner:?} references missing door {door_index}"
                    )
                })
                .sector_in
        } else {
            let sector = current_sector.unwrap_or_else(|| {
                panic!("globally frozen climb owner {owner:?} has no lift sector")
            });
            crate::sector::SectorNumber::new(i16::from(sector))
        };
        let lift = self
            .grid_sector_by_number(sector_number)
            .unwrap_or_else(|| {
                panic!(
                    "globally frozen climb owner {owner:?} references missing lift sector {sector_number}"
                )
            });
        assert_eq!(
            lift.lift_type,
            Some(expected_lift_type),
            "globally frozen climb owner {owner:?} action {action:?} requires {expected_lift_type:?}, found {:?}",
            lift.lift_type
        );
        let direction = if action == OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel {
            (lift.lift_direction + 8) & 15
        } else {
            lift.lift_direction
        };
        let turns = if is_fast_climb_action(action) { 2 } else { 1 };
        let entity = self
            .world
            .entities
            .get_mut(owner)
            .expect("globally frozen climb owner disappeared after canonical lookup");
        if execute_order_initialising {
            entity.element_data_mut().set_direction_goal(direction);
        }
        for _ in 0..turns {
            entity.element_data_mut().sprite.position_iface.turn();
        }
    }

    /// Mirror the Human Execute guard on the logical sword-movement
    /// non-animations. A stale sword move can still reach its owner slot after
    /// the actor's final opponent has gone away; unless the movement was
    /// explicitly forced, Original aborts that element and submits one
    /// QuitSwordfight command before it ever calls FaceOpponent/PerformMotion.
    fn abort_orphaned_sword_movement(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        selected: MovementOwnerSelection,
    ) -> bool {
        let should_abort = self
            .orders
            .sequence_manager
            .get_element(selected.seq_id, selected.elem_idx)
            .filter(|element| {
                element.owner == Some(owner)
                    && element.data.is_movement()
                    && !element.priority.is_non_interruptable()
            })
            .and_then(|element| {
                let order = element.current_order()?;
                if order.order_id != selected.order_id
                    || !matches!(
                        order.order_type,
                        OrderType::WalkingWithSword | OrderType::RunningWithSword
                    )
                {
                    return None;
                }
                let flags = match element.data {
                    crate::sequence::SequenceElementData::Movement { flags, .. } => flags,
                    _ => unreachable!("movement element changed data kind during sword guard"),
                };
                Some(!flags.contains(crate::sequence::MoveFlags::FORCE_SWORD_MOVEMENT))
            })
            .unwrap_or(false)
            && self
                .world
                .entities
                .get(owner)
                .and_then(|entity| entity.human_data())
                .is_some_and(|human| human.opponents.is_empty());
        if !should_abort {
            return false;
        }

        // A sword movement may have been postponed by an explicit
        // QuitSwordfight and selected again only after the lowering animation
        // changed the actor back to an ordinary action state. Original
        // retranslates that surviving movement as upright work. Rust retains
        // the already-built order across postponement, so perform the
        // equivalent rewrite here before the orphan guard decides whether the
        // actor still needs to quit.
        let fight_already_lowered = self
            .world
            .entities
            .get(owner)
            .and_then(|entity| entity.actor_data())
            .is_some_and(|actor| !actor.action_state.is_sword());
        if fight_already_lowered {
            let element = self
                .orders
                .sequence_manager
                .get_element_mut(selected.seq_id, selected.elem_idx)
                .unwrap_or_else(|| {
                    panic!(
                        "lowered sword movement owner {owner:?} lost selected element \
                         ({:?}, {})",
                        selected.seq_id, selected.elem_idx
                    )
                });
            let order = element.orders.front_mut().unwrap_or_else(|| {
                panic!(
                    "lowered sword movement owner {owner:?} has no selected order in \
                     ({:?}, {})",
                    selected.seq_id, selected.elem_idx
                )
            });
            let upright_action = match order.order_type {
                OrderType::WalkingWithSword => OrderType::WalkingUpright,
                OrderType::RunningWithSword => OrderType::RunningUpright,
                other => {
                    panic!("lowered sword movement owner {owner:?} has unexpected order {other:?}")
                }
            };
            order.order_type = upright_action;
            let crate::sequence::SequenceElementData::Movement { action, .. } = &mut element.data
            else {
                unreachable!("selected sword movement changed data kind during upright rewrite")
            };
            *action = upright_action;
            element.action_state_after_transition = crate::element::ActionState::Waiting;
            return false;
        }

        // Retire and clean the old movement first. This ensures the
        // replacement QuitSwordfight cannot be postponed behind the very
        // element whose Execute arm just rejected itself.
        self.stop_owner_active_mechanics(owner);
        self.orders
            .sequence_manager
            .element_impossible(selected.seq_id, selected.elem_idx);
        self.dispatch_condolations_for_owner_boundary(sim, owner, assets);
        self.drain_script_synchronous_actions(sim, assets, &mut Vec::new())
            .unwrap_or_else(|error| {
                panic!(
                    "orphaned sword movement owner {owner:?} failed to retire rejected movement: {error:?}"
                )
            });
        let lowering_id = self.orders.allocate_order_id();
        self.launch_single_order_sequence_stamped_ex(
            owner,
            crate::element::Command::QuitSwordfight,
            crate::order::Order::new(OrderType::TransitionLoweringSword, 0.0, 0.0, lowering_id),
            false,
        );

        // Original immediately sends EVENT_QUIT_SWORDFIGHT from this guard;
        // PCs intentionally have no AI receiver.
        self.dispatch_ai_stimulus(
            owner,
            crate::ai::Stimulus::new(crate::ai::StimulusType::EventQuitSwordfight),
        );
        true
    }

    pub(super) fn tick_entity_movement_owner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &crate::engine::LevelAssets,
        owner: EntityId,
        selected: Option<MovementOwnerSelection>,
    ) {
        let Some(selected) = selected else { return };
        let selected_is_live = self
            .orders
            .sequence_manager
            .get_element(selected.seq_id, selected.elem_idx)
            .filter(|element| element.owner == Some(owner) && element.data.is_movement())
            .and_then(|element| element.current_order())
            .is_some_and(|order| order.order_id == selected.order_id);
        if !selected_is_live {
            return;
        }
        if self
            .world
            .entities
            .get(owner)
            .and_then(|entity| entity.actor_data())
            .is_some_and(|actor| actor.execution_frozen)
        {
            return;
        }
        let selected_command = self
            .orders
            .sequence_manager
            .current_element_for_actor(owner)
            .and_then(|(seq_id, elem_idx)| {
                self.orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .map(|element| element.command)
            });
        if matches!(
            selected_command,
            Some(crate::element::Command::WaitTimer | crate::element::Command::WaitFreeLift)
        ) {
            return;
        }
        if self.abort_orphaned_sword_movement(sim, assets, owner, selected) {
            // LaunchSequenceElement registers the replacement for the later
            // sequence-manager phase. It must not execute at this actor
            // boundary: Original exposes QuitSwordfight as the current
            // command for one frame before its lowering order starts.
            return;
        }
        if self.actors_frozen() {
            self.turn_globally_frozen_climb_owner(owner, selected);
            // FrozenAll suppresses Sprite::PerformMotion but not the Execute
            // work before it: climb Turn() above and both rider-specific
            // Soldier arms remain live. RiderCharging performs its polygon
            // work or RunningUpright samples that frozen frame and may Think.
            let consumed_charge = self.tick_rider_charge_owner(sim, assets, owner, true);
            if !consumed_charge && self.selected_galopp_decision_frame(owner, selected) {
                self.dispatch_galopp_loop_event(sim, assets, owner);
            }
            return;
        }

        // Sample mutable mobile geometry only now, at this actor's Original
        // entity slot. Preparing it once before the live owner walk freezes
        // every actor onto the same side of intervening mobile masters.
        let prepared = self.live_mobile_geometry();

        // Pre-pass: collect principal opponent positions for
        // combat-moving entities.  During sword/shield movement,
        // FaceOpponent / FaceDangerPoint overrides the entity's facing
        // direction toward their opponent instead of the movement
        // direction, and selects directional animations
        // (forward/backward/strafe) based on the angle between
        // movement and facing.
        let mut combat_face_targets = EntitySlots::filled(self.world.entities.len(), None);
        for (actor_id, entity) in self
            .world
            .entities
            .actors()
            .filter(|(id, _)| EntityId::from(*id) == owner)
        {
            let actor = entity
                .actor_data()
                .expect("entities.actors() yielded non-actor entity");
            let is_shield_moving = matches!(
                actor.action_state,
                crate::element::ActionState::MovingShield
            );
            // Shield bearers face the stored danger point.
            // Sword fighters face their principal opponent.
            if is_shield_moving && let Some(pt) = actor.shield_face_point {
                combat_face_targets[actor_id] = Some(pt);
                continue;
            }
            // Shield bearer with no danger point stored: face *away*
            // from the protected ally.  Encode this as a target equal
            // to `2 * self_pos - ally_pos` so the downstream
            // `vector_to_sector_0_to_15(target - self)` math aims the
            // shield-bearer away from the ally.
            if is_shield_moving
                && let Some(protected_id) = entity.pc_data().and_then(|pc| pc.shield_protected)
                && let Some(ally) = self.world.entities.get(protected_id)
            {
                let self_pos = entity.element_data().position_map();
                let ally_pos = ally.element_data().position_map();
                combat_face_targets[actor_id] = Some(crate::coordinates::MapPoint {
                    x: 2.0 * self_pos.x - ally_pos.x,
                    y: 2.0 * self_pos.y - ally_pos.y,
                });
                continue;
            }
            // FaceOpponent dispatch for sword movement:
            //   swordfighting → principal opponent's ground position
            //   else if soldier → primary target's ground position
            //   else            → return WALKING_SWORD without facing change
            //
            // Build this even before `action_state` flips to MovingSword;
            // forced sword movement can still be represented only by the
            // movement element's FORCE_SWORD_MOVEMENT flag at this point.
            //
            // The non-soldier, non-swordfighting branch leaves facing
            // untouched. We mirror this by storing the entity's own
            // position as a sentinel so the main loop's combat-face block
            // becomes a no-op (`fdx*fdx + fdy*fdy > 0.01` fails) while
            // still suppressing the movement-direction facing fallback.
            let is_swordfighting = entity
                .human_data()
                .map(|human| !human.opponents.is_empty())
                .unwrap_or(false);
            let opp_id_opt: Option<EntityId> = if is_swordfighting {
                // Principal opponent = first in opponent list.
                entity
                    .human_data()
                    .and_then(|h| h.opponents.first())
                    .copied()
            } else if entity.is_soldier() {
                // GetPrimaryTarget — soldier's AI-picked priority target,
                // which can differ from opponents[0].
                entity
                    .ai_controller()
                    .map(|c| EntityId::Pc(crate::entity_id::PcId(c.primary_target)))
                    .filter(|id| id.index() != 0)
            } else {
                None
            };

            if let Some(opp_id) = opp_id_opt
                && let Some(opp) = self.world.entities.get(opp_id)
            {
                combat_face_targets[actor_id] = Some(opp.element_data().position_map());
            } else {
                // Sentinel: face self → no rotation, no movement-direction
                // fallback (the "return WALKING_SWORD without changing
                // facing" branch).
                combat_face_targets[actor_id] = Some(entity.element_data().position_map());
            }
        }

        // Pre-pass: look up the current sequence-element speed factor
        // for every entity with an active movement
        // (`distance *= speed_factor` during the per-frame motion
        // update). Pre-computed here so the main loop can borrow
        // `self.world.entities` mutably while consulting
        // `self.orders.sequence_manager` for the factor.
        let mut speed_factors = EntitySlots::filled(self.world.entities.len(), 1.0);
        // Per-entity final-waypoint tolerance snapshot for the arrival
        // check.  The seek-arrival predicate is:
        //
        //   target.sector == self.sector                     (same-sector)
        //   && dist_sq < seek_distance^2 * 1.1025            (5% margin)
        //
        // where `dist` is the vector from the actor to the target's
        // current position (or its current-row hotspot under
        // `USE_POINT`), with Y stretched by the inverse aspect ratio
        // when `DIRECTIONAL_TOLERANCE` is set.
        //
        // `target_is_actor` lets the main loop set the shield-follower
        // speed factor: when self is in `MovingShield` action state
        // and the seek target is an actor, the speed factor becomes
        // 1.0 / 1.5 / 2.0 depending on range.
        #[derive(Clone, Copy, Default)]
        struct FinalTol {
            tol: f32,
            directional: bool,
            target_is_actor: bool,
            /// Entity target resolved when the movement frame starts. Its
            /// position, sector, and current-row hotspot are sampled again at
            /// this actor's creation-order slot, after earlier actors have
            /// committed their movement.
            target_id: Option<EntityId>,
            use_point: bool,
            /// Shield seeks compare actor position to the movement
            /// element's computed shield destination, not to the
            /// protected PC's live position.
            shield_destination: Option<MapPoint>,
            /// Snapshot of `ActorData::last_seek_target_position` —
            /// the target position stamped at seek launch / refresh.
            /// Used by the transition-animation refresh check to
            /// detect mid-walk target drift before the transition arm
            /// runs.
            last_seek_target_position: MapPoint,
            /// Whether the actor has a `post_seek_sequence` attached.
            /// Lifts the `is_final_waypoint` gate on tolerance arrival
            /// for mid-path arrivals: the seek's same-sector +
            /// tolerance predicate runs every tick, not just at the
            /// final waypoint.  When the target wanders into range
            /// mid-route, the seek terminates early and the post-seek
            /// sequence fires.  Without a post-seek sequence to
            /// consume the arrival, the order_pop fall-through would
            /// drop intermediate waypoints and leave the actor
            /// stranded — so guard intermediate-tick arrival on this
            /// flag.
            has_post_seek: bool,
        }
        let mut final_tolerances =
            EntitySlots::filled(self.world.entities.len(), FinalTol::default());
        let mut point_seek_post_sectors = EntitySlots::filled(self.world.entities.len(), None);
        let mut sword_movement_starts: Vec<EntityId> = Vec::new();
        let mut sword_movement_terminations: Vec<EntityId> = Vec::new();
        for (actor_id, entity) in self
            .world
            .entities
            .actors()
            .filter(|(id, _)| EntityId::from(*id) == owner)
        {
            let Some(actor) = entity.actor_data() else {
                continue;
            };
            let (seq_id, elem_idx) = (selected.seq_id, selected.elem_idx);
            if let Some(elem) = self.orders.sequence_manager.get_element(seq_id, elem_idx) {
                speed_factors[actor_id] = elem.speed_factor();
                if let crate::sequence::SequenceElementData::Movement {
                    flags,
                    tolerance: _,
                    element: target_elem,
                    destination,
                    sector,
                    ..
                } = &elem.data
                {
                    if flags.contains(crate::sequence::MoveFlags::SEEK) {
                        // The per-tick seek-arrival predicate (and its
                        // FROZEN-wait sibling) is a SEEK-only
                        // mechanism.  Non-seek `GoNear`-style
                        // stop-distances are enforced earlier, by
                        // `insert_transition_end` adding the element's
                        // tolerance to the end-transition shift
                        // (`distance_remaining + tolerance`), which
                        // truncates the walking phase before order
                        // emission.  Gating here keeps the FinalTol
                        // snapshot meaningful only for true seeks, so
                        // the downstream tolerance-arrival check can
                        // rely on the creation-slot target observation /
                        // `shield_destination` being live. Gate-approach
                        // legs of an entity seek deliberately carry zero
                        // tolerance, but remain PerformSeek owners and must
                        // still age the shared refresh countdown.
                        let directional =
                            flags.contains(crate::sequence::MoveFlags::DIRECTIONAL_TOLERANCE);
                        let use_point = flags.contains(crate::sequence::MoveFlags::USE_POINT);
                        let seek_shield = flags.contains(crate::sequence::MoveFlags::SEEK_SHIELD);
                        let (resolved_target_id, target_is_actor) =
                            match target_elem.and_then(|id| self.get_entity(id)) {
                                Some(t) => (*target_elem, t.actor_data().is_some()),
                                // SEEK without antagonist = seek-to-point
                                // mode.  Skip the dist-vs-tolerance
                                // check; arrival is detected by motion
                                // termination + same-sector match.
                                // Falls through to the standard
                                // `dist <= speed` final-waypoint
                                // arrival when there is no post-seek
                                // sequence. Leaving target_id None
                                // signals the consumer to skip the
                                // entity-target seek-distance check.
                                None => {
                                    if actor.post_seek_sequence.is_some() {
                                        point_seek_post_sectors[actor_id] = *sector;
                                    }
                                    (None, false)
                                }
                            };
                        // Skip the FinalTol snapshot entirely for
                        // seek-to-point + non-shield (target_id is
                        // None and there's no shield destination), so
                        // the seek-arrival predicate doesn't fire.
                        if resolved_target_id.is_some() || seek_shield {
                            // Original keeps an interaction following an
                            // entity Seek in `mpPostSeekSequence`. Gate-route
                            // construction in Rust currently represents that
                            // continuation as later elements of the same
                            // sequence. Treat either representation as a live
                            // post-seek handoff: PerformSeek returns before
                            // aging `mulWaitTime` when tolerance is reached.
                            let has_post_seek = actor.post_seek_sequence.is_some()
                                || self
                                    .orders
                                    .sequence_manager
                                    .get_sequence(seq_id)
                                    .is_some_and(|sequence| elem_idx + 1 < sequence.elements.len());
                            final_tolerances[actor_id] = FinalTol {
                                // `mfSeekDistance` remains the unadapted
                                // interaction radius.  RefreshSeek may halve
                                // the concrete movement element's tolerance
                                // while chasing a moving target, but
                                // PerformSeek never uses that adapted path
                                // tolerance for its live-target arrival test.
                                tol: actor.seek_distance,
                                directional,
                                target_is_actor,
                                target_id: resolved_target_id,
                                use_point,
                                shield_destination: seek_shield.then_some(*destination),
                                last_seek_target_position: actor.last_seek_target_position,
                                has_post_seek,
                            };
                        }
                    }
                }
            }
        }

        // Pre-pass: drive the per-tick `TurnDrunken()` turn for every
        // drunken soldier. `TurnDrunken()` picks
        // between `TurnSlow(2)` and `TurnVerySlow()` (delay 5) so the
        // soldier's facing lags behind the movement vector.  This
        // must run before the main loop because the per-tick turn
        // advances `position_iface` (a mutable borrow that would
        // conflict with `entity.element_data_mut()`).
        for (_, soldier) in self
            .world
            .entities
            .soldiers_mut()
            .filter(|(id, _)| EntityId::from(*id) == owner)
        {
            let is_drunk = soldier
                .npc
                .ai_brain
                .base()
                .map(|b| b.blood_alcohol > 0)
                .unwrap_or(false);
            if !is_drunk {
                continue;
            }
            // Compute the movement goal vector.  Skip entities without
            // an active movement path — idle drunk soldiers don't
            // wobble.  Goal is read from the actor's Move element's
            // current order (authoritative path source).
            let actor = &soldier.actor;
            let Some(_) = actor.active_movement.sequence_id else {
                continue;
            };
            let Some(order) = self
                .orders
                .sequence_manager
                .get_element(selected.seq_id, selected.elem_idx)
                .and_then(|element| element.current_order())
                .filter(|order| order.order_id == selected.order_id)
            else {
                continue;
            };
            let goal = MapPoint::new(order.target_x, order.target_y);
            let pos = soldier.element.position_map();
            let dx = goal.x - pos.x;
            let dy = goal.y - pos.y;
            if dx * dx + dy * dy < 0.01 {
                continue;
            }
            let goal_sector = vector_to_sector_0_to_15(dx, dy);
            // Gate the facing-from-movement-vector goal update on
            // the order's compute_direction flag.  When the order
            // pushes a fixed facing (compute_direction = false), keep
            // the goal direction the caller set and only run the slow
            // turn — `TurnDrunken` reads the direction goal but never
            // writes it.
            let order_compute_direction = order.compute_direction;
            {
                let pi = &mut soldier.element.sprite.position_iface;
                let current_dir = pi.get_direction();
                let goal_for_turn = if order_compute_direction {
                    pi.set_direction(crate::position_interface::Direction::from_raw(
                        goal_sector as i32,
                    ));
                    goal_sector as u16
                } else {
                    u16::from(pi.get_direction_goal())
                };
                let very_slow = crate::engine::soldier_helpers::turn_drunken_is_very_slow(
                    u16::from(current_dir),
                    goal_for_turn,
                );
                if very_slow {
                    pi.turn_very_slow();
                } else {
                    pi.turn_slow(2);
                }
            }
        }

        // Pre-pass: per-entity current-sector lift translation, for
        // the lift branches of the movement-animation derivation.
        // When a moving actor is in a lift sector, the per-frame
        // walk/run animation is overridden by the lift's upwards /
        // downwards action mapping:
        //   * Upright posture: lift type rewrites the action; upwards
        //     and downwards animations are equal for upright, so we
        //     always use the upwards mapping.
        //   * OnLadder / OnWall: pick upwards vs downwards by
        //     dot-producting the ladder vector (`pt_low - pt_high`)
        //     with the movement vector — non-negative means moving
        //     down.  The high / low exit points are the in-side
        //     points of the lift's highest and lowest doors.
        //
        // Pre-computed here so the main loop can borrow `self.world.entities`
        // mutably without touching `self.world.fast_grid` or the door table.
        let mut lift_translations = EntitySlots::filled(self.world.entities.len(), None);
        let mut door_pass_climb_directions = EntitySlots::filled(self.world.entities.len(), None);
        for (actor_id, entity) in self
            .world
            .entities
            .actors()
            .filter(|(id, _)| EntityId::from(*id) == owner)
        {
            let posture = entity.element_data().posture;
            let door_pass_action = entity
                .actor_data()
                .and_then(|actor| actor.active_door_pass.as_ref())
                .map(|dp| dp.current_action);
            let Some(sector) = entity.element_data().sector() else {
                continue;
            };
            let Some(gs) =
                self.grid_sector_by_number(crate::sector::SectorNumber::new(i16::from(sector)))
            else {
                continue;
            };
            if let Some(action) = door_pass_action
                && let Some(expected) = climb_lift_type(action)
            {
                door_pass_climb_directions[actor_id] = entity
                    .actor_data()
                    .and_then(|actor| actor.active_door_pass.as_ref())
                    .map(|dp| {
                        self.script_domains
                            .interactables
                            .doors
                            .get(usize::from(dp.door_index))
                            .unwrap_or_else(|| {
                                panic!(
                                    "door-pass climb owner {actor_id:?} references missing door {}",
                                    dp.door_index
                                )
                            })
                            .sector_in
                    })
                    .map(|sector_in| {
                        self.grid_sector_by_number(crate::sector::SectorNumber::new(i16::from(
                            sector_in,
                        )))
                        .unwrap_or_else(|| {
                            panic!(
                                "door-pass climb owner {actor_id:?} references missing lift sector {sector_in}"
                            )
                        })
                    })
                    .map(|sector| {
                        assert_eq!(
                            sector.lift_type,
                            Some(expected),
                            "door-pass climb owner {actor_id:?} action {action:?} requires {expected:?}, found {:?}",
                            sector.lift_type
                        );
                        sector.lift_direction
                    });
            }
            let Some(lt) = gs.lift_type else { continue };
            match posture {
                crate::element::Posture::Upright => {
                    lift_translations[actor_id] = Some(LiftAnimContext::Upright(lt));
                }
                crate::element::Posture::OnLadder | crate::element::Posture::OnWall
                    if matches!(
                        (posture, lt, door_pass_action),
                        (
                            crate::element::Posture::OnWall,
                            crate::sector::LiftType::Wall,
                            _
                        ) | (
                            crate::element::Posture::OnLadder,
                            crate::sector::LiftType::Ladder,
                            _
                        )
                    ) =>
                {
                    let (pt_low, pt_high) = self.lift_endpoint_points(gs.sector_number);
                    let ladder_dx = pt_low.x - pt_high.x;
                    let ladder_dy = pt_low.y - pt_high.y;
                    lift_translations[actor_id] = Some(LiftAnimContext::OnClimb {
                        lift_type: lt,
                        lift_direction: gs.lift_direction,
                        ladder_dx,
                        ladder_dy,
                    });
                }
                _ => {}
            }
            if lift_translations[actor_id].is_none()
                && matches!(
                    (lt, door_pass_action),
                    (
                        crate::sector::LiftType::Wall,
                        Some(
                            OrderType::ClimbingWallUp
                                | OrderType::ClimbingWallDown
                                | OrderType::ClimbingWallUpFast
                                | OrderType::ClimbingWallDownFast
                        )
                    ) | (
                        crate::sector::LiftType::Ladder,
                        Some(
                            OrderType::ClimbingLadderUp
                                | OrderType::ClimbingLadderDown
                                | OrderType::ClimbingLadderUpFast
                                | OrderType::ClimbingLadderDownFast
                        )
                    )
                )
            {
                let (pt_low, pt_high) = self.lift_endpoint_points(gs.sector_number);
                lift_translations[actor_id] = Some(LiftAnimContext::OnClimb {
                    lift_type: lt,
                    lift_direction: gs.lift_direction,
                    ladder_dx: pt_low.x - pt_high.x,
                    ladder_dy: pt_low.y - pt_high.y,
                });
            }
        }

        // Pre-pass: snapshot every actor's position / layer / sector /
        // posture / repulsive-point contribution for the
        // anti-collision disturbing-actor lookup.  Captured once per
        // tick so the mutable main loop can read neighbour state
        // without a second borrow, matching the deterministic
        // start-of-tick view the replay system relies on.
        // Mutable — each entity's post-move position is written back
        // so later entities in the same tick see the serial
        // "already-moved" view: each actor's anti-collision lookup
        // reads live positions from earlier-processed actors.
        let mut anti_snapshots = super::anti_collision::snapshot_all(
            &self.world.entities,
            &self.orders.sequence_manager,
            &assets.profile_manager,
        );

        // Collect movement results that need sequence manager notification.
        // We can't call sequence_manager while iterating entities mutably.
        // Door-pass triggers to execute after the movement loop (need &mut self).
        let mut door_triggers: Vec<(EntityId, crate::gate::DoorIndex, bool, u8)> = Vec::new();
        // Door-pass Transition orders to push onto the actor's current
        // sequence element after the loop closes (needs sequence_manager).
        let mut transition_pushes: Vec<(crate::sequence::SequenceId, usize, crate::order::Order)> =
            Vec::new();
        // Pending `DoorPassStep::Select` hulk requests — processed after the
        // loop since they mutate both the carrier and its carried target.
        let mut select_triggers: Vec<(EntityId, f32)> = Vec::new();
        let mut completed_door_passes: Vec<(EntityId, crate::gate::DoorIndex, bool)> = Vec::new();
        // Rider entities whose running animation hit the charge
        // decision frames while carrying RIDER_CHARGE.
        let mut galopp_event = false;
        // Movement elements whose sprite motion returned the blocked-
        // abort signal and must be marked Impossible after the entity
        // borrow ends.
        let mut blocked_impossible: Vec<(crate::sequence::SequenceId, usize)> = Vec::new();
        let mut door_pass_transition_start_effects: Vec<EntityId> = Vec::new();
        let mut door_pass_transition_done_effects: Vec<EntityId> = Vec::new();
        let mut door_pass_transition_completion_effects: Vec<EntityId> = Vec::new();
        let mut post_seek_arrivals: Vec<(EntityId, crate::sequence::SequenceId, usize)> =
            Vec::new();
        // Elevation-line crossings detected during this tick. Dispatched
        // after the entity loop so `check_for_line_crossing` can borrow
        // `self` for the fast-grid query and obstacle swap.
        // Each entry is `(entity_id, old_pos, new_pos, layer)` in projected
        // map coordinates. Geometry queries convert at the call boundary.
        let mut line_cross_checks: Vec<(EntityId, MapPoint, MapPoint, u16)> = Vec::new();
        // Patch-line (`LINE_PATCH`) crossings detected for PC actors.
        // Dispatched after the entity loop so
        // `check_for_patch_line_crossing` can borrow `self` for the
        // per-patch `enter`/`leave`/`apply` state mutation and effect
        // processing.  Covers the `LINE_PATCH` PC-only crossing arm.
        let mut patch_cross_checks: Vec<(EntityId, MapPoint, MapPoint, u16)> = Vec::new();
        // Sound-line (`LINE_SOUND`) crossings detected for any actor.
        // Dispatched after the entity loop so
        // `check_for_sound_line_crossing` can borrow `self` for the
        // material refresh.  Covers the `LINE_SOUND` crossing arm
        // (single-line and multi-line).  Unlike LINE_PATCH this fires
        // for every actor (PC, NPC, soldier) — the SOUND arm is not
        // gated on PC.
        let mut sound_cross_checks: Vec<(EntityId, MapPoint, MapPoint, u16)> = Vec::new();
        // Script-sector callbacks are line-crossing effects in the Original,
        // not a global per-frame polygon reconciliation.
        let mut script_cross_checks: Vec<(EntityId, MapPoint, MapPoint, u16)> = Vec::new();
        // Seek elements whose end-of-walk arrival put a
        // transition-to-waiting animation in line as the next order
        // while the live target had drifted beyond
        // `transition_distance + seek_distance × 1.05`.  Seek refresh
        // aborts the queued transition arm and rebuilds the path; the
        // Rust port queues these for the existing `tick_refresh_seeks`
        // machinery to handle right after the per-tick movement loop,
        // before the sequence manager re-dispatches.  Each entry is
        // `(owner, seq_id, elem_idx)`.
        let mut transition_seek_refreshes: Vec<(EntityId, crate::sequence::SequenceId, usize)> =
            Vec::new();
        // Waypoint arrivals (both intermediate and final) — each
        // triggers one `do_next_order` call on the actor's Move
        // element to pop the walking order that represented that
        // waypoint.  Each waypoint is its own order on the actor's
        // movement order list, and the engine pops them as the actor
        // crosses them.  Collected here and processed after the entity
        // loop so the `do_next_order` call can borrow `self` mutably.
        let mut order_pops: Vec<(crate::sequence::SequenceId, usize)> = Vec::new();

        // Water-splash titbit emissions queued from the walk branch.
        // Drained after the entity loop so `titbit_manager.add_titbit`
        // can borrow `&mut self` without colliding with the active
        // entity borrow.
        let mut water_splash_emits: Vec<(EntityId, crate::coordinates::WorldPoint3D, u16)> =
            Vec::new();
        let mut movement_state_effects: Vec<(
            EntityId,
            crate::element::Posture,
            crate::element::ActionState,
        )> = Vec::new();
        // PC movement actions actually dispatched this frame.  The original
        // RHElementActorPC performs action-specific side effects from inside
        // the matching Execute arm, so posture alone is not a substitute for
        // this per-frame execution record.
        let mut executed_pc_movement_actions: Vec<(EntityId, OrderType)> = Vec::new();
        let mut executed_sword_movement = false;

        // Iterate a stable creation-order ID list instead of holding one
        // mutable iterator borrow across the whole pass. This lets each actor
        // sample its SEEK target directly from the entity table immediately
        // before its own movement. Mutations by an earlier-created actor are
        // therefore visible, while a later-created target still exposes its
        // pre-movement state, matching RHEngine's virtual Hourglass loop.
        let movement_actor_ids: Vec<_> = self
            .world
            .entities
            .actors()
            .filter(|(id, _)| EntityId::from(*id) == owner)
            .map(|(id, _)| id)
            .collect();
        'actors: for actor_id in movement_actor_ids {
            let entity_id = actor_id.into();
            if self.tick_rider_charge_owner(sim, assets, entity_id, false) {
                continue;
            }
            let ft = final_tolerances[actor_id];
            let live_seek_target = ft.target_id.and_then(|target_id| {
                self.world.entities.get(target_id).map(|target| {
                    let target_data = target.element_data();
                    let target_position = target_data.position_map();
                    let use_point = if ft.use_point {
                        target
                            .cxx_current_point_map()
                            .filter(|point| *point != target_position)
                    } else {
                        None
                    };
                    (target_position, target_data.sector(), use_point)
                })
            });
            let seek_tolerance_reached = |position: MapPoint, self_sector| {
                if ft.tol <= 0.0 {
                    return false;
                }
                let target_sector = live_seek_target.and_then(|(_, sector, _)| sector);
                if target_sector.is_some() && self_sector != target_sector {
                    false
                } else {
                    let target_center = ft
                        .shield_destination
                        .or(live_seek_target.map(|(position, _, _)| position))
                        .expect(
                            "SEEK FinalTol must have shield_destination or a live target position",
                        );
                    let target = live_seek_target
                        .and_then(|(_, _, point)| point)
                        .unwrap_or(target_center);
                    let (dx_use, dy_use) = (target.x - position.x, target.y - position.y);
                    let dy_effective = if ft.directional {
                        const INVERSE_ASPECT_RATIO: f32 = 1.743_446_8;
                        dy_use * INVERSE_ASPECT_RATIO
                    } else {
                        dy_use
                    };
                    let dist_sq = dx_use * dx_use + dy_effective * dy_effective;
                    dist_sq < ft.tol * ft.tol * 1.1025
                }
            };
            let entity = self
                .world
                .entities
                .get_mut(entity_id)
                .expect("movement actor ID collected from entity table must remain present");
            let is_pc = entity.is_pc();
            let human_is_carried = entity
                .human_data()
                .is_some_and(|human| human.carrier.is_some());
            // Check swordfight status before mutable borrows — needed at
            // movement completion to preserve WaitingSword (idle state
            // is derived from the action state machine, not hardcoded
            // Waiting).
            let is_swordfighting = entity
                .human_data()
                .map(|h| !h.opponents.is_empty())
                .unwrap_or(false);

            // Extract movement data from actor (scoped borrow).
            //
            // The walk goal is read from the current order's
            // destination on the actor's active Move element —
            // accessed via `SequenceManager::current_order_for_actor`.
            // `path_waypoints` is kept as a mirror for legacy bolt-ons
            // (drunken wobble, abilities, debug overlays) but is no
            // longer the authoritative path source in the hot loop.
            let (
                goal,
                action_state,
                order_id,
                door_pass_anim,
                is_final_waypoint,
                order_action,
                move_seq_id,
                move_elem_idx,
                active_move_flags,
                order_tolerance,
                mut order_compute_direction,
                order_reverse,
                transition_distance_continuation,
                deferred_movement_state_start,
                next_destination_same_action,
                legacy_serialized_order_chain,
            ) = {
                let actor = match entity.actor_data_mut() {
                    Some(a) => a,
                    None => continue,
                };
                let has_moving_state = actor.action_state.is_moving()
                    || actor.action_state == crate::element::ActionState::MovingSword
                    || actor.action_state == crate::element::ActionState::MovingFastSword
                    || actor.action_state == crate::element::ActionState::MovingShield;
                // Read goal from the current **movement** element's
                // front order on the Move / PassDoor / Seek element.
                //
                // We explicitly filter by element data type instead
                // of using `current_order_for_actor` directly: another
                // element type (`Turn`, `Generic` animation, …) may
                // have become InProgress concurrently — e.g. a Turn
                // launched at `SequencePriority::Turn` while the Move
                // is still in flight.  Its front order has no
                // destination (`Turning` orders are (0,0)), so using
                // it as a goal would make the actor walk toward the
                // map origin.  Hold a pointer to the *movement*
                // element specifically by picking the InProgress
                // element whose data is a `Movement`.
                let move_elem = self
                    .orders
                    .sequence_manager
                    .get_element(selected.seq_id, selected.elem_idx)
                    .filter(|element| {
                        element.owner == Some(entity_id)
                            && element.state == crate::sequence::SequenceState::InProgress
                            && element.data.is_movement()
                            && element
                                .current_order()
                                .is_some_and(|order| order.order_id == selected.order_id)
                    })
                    .map(|_| (selected.seq_id, selected.elem_idx));
                let Some((seq_id, elem_idx)) = move_elem else {
                    if !has_moving_state {
                        continue;
                    }
                    // No active Move element (element terminated or
                    // was never active) — drop out of the moving
                    // state back to Waiting.
                    let restore_anti_collision = {
                        let restore_anti_collision = actor.active_door_pass.is_some();
                        if restore_anti_collision {
                            tracing::warn!(
                                entity = ?entity_id,
                                "DoorPass: clearing stale active pass after movement element disappeared"
                            );
                            actor.active_door_pass = None;
                        }
                        actor.action_state = if is_swordfighting || actor.action_state.is_sword() {
                            crate::element::ActionState::WaitingSword
                        } else {
                            crate::element::ActionState::Waiting
                        };
                        actor.active_movement.clear();
                        restore_anti_collision
                    };
                    if restore_anti_collision {
                        entity.position_iface_mut().set_anti_collision_on(true);
                    }
                    continue;
                };
                if !has_moving_state
                    && self
                        .orders
                        .sequence_manager
                        .current_element_for_actor(actor_id)
                        != Some((seq_id, elem_idx))
                {
                    // A parallel movement element can remain in progress
                    // while a higher-priority non-movement element owns the
                    // actor. Only bootstrap a non-moving actor when this Move
                    // is its selected current element.
                    continue;
                }
                let Some(order) = self
                    .orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .and_then(|e| e.current_order())
                else {
                    continue;
                };
                let goal = MapPoint::new(order.target_x, order.target_y);
                let order_id = Some(order.order_id);
                let order_action = order.order_type;
                let order_tolerance = order.tolerance;
                let order_compute_direction = order.compute_direction;
                let order_reverse = order.reverse;
                let transition_distance_continuation = order.transition_distance_continuation;
                let deferred_movement_state_start = order.deferred_movement_state_start;
                let next_destination_same_action = self
                    .orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .and_then(|e| e.next_order())
                    .filter(|next| next.order_type == order_action)
                    .map(|next| MapPoint::new(next.target_x, next.target_y));
                let active_move_flags = self
                    .orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .and_then(|e| match &e.data {
                        crate::sequence::SequenceElementData::Movement { flags, .. } => {
                            Some(*flags)
                        }
                        _ => None,
                    })
                    .unwrap_or(crate::sequence::MoveFlags::empty());
                let legacy_serialized_order_chain = self
                    .orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .is_some_and(|element| element.legacy_v48.is_some());

                // Selecting a door-pass Walk successor is not the same as
                // executing it.  Restore the movement state only when that
                // concrete order reaches its owner slot; PassingDoor and
                // transition completion retain their preceding state for the
                // remainder of the tick in Original.
                if order_uses_distance_motion(order_action)
                    && actor.active_door_pass.as_ref().is_some_and(|pass| {
                        pass.current_action == order_action && pass.saved_action_state.is_some()
                    })
                {
                    let saved = actor
                        .active_door_pass
                        .as_mut()
                        .expect("checked active door pass")
                        .saved_action_state
                        .take()
                        .expect("checked saved door-pass action state");
                    actor.action_state = saved;
                }

                // Is this the literal last order in the queue?  The
                // Movement element's `tolerance` applies to the final
                // arrival (tolerance applies only on the last order),
                // so we must only allow `tolerance_arrival`
                // to short-circuit when *no* orders remain behind the
                // current one — including end-transition orders spliced
                // in by `insert_transition_end`, which still carry the
                // actual destination as their target.  A prior version
                // of this check counted "last walk-style order", which
                // made the penultimate walking order inserted by
                // `insert_transition_end` look final and triggered an
                // instant tolerance arrival the moment the start
                // transition popped — the actor teleported past the
                // walking phase, played the stop transition in place
                // and never covered any ground.
                let is_final_waypoint = self
                    .orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .map(|e| e.orders.len() <= 1)
                    .unwrap_or(true);
                // Use the animation from the active door-pass Walk step.
                let door_pass_anim: Option<OrderType> =
                    actor.active_door_pass.as_ref().map(|dp| dp.current_action);

                (
                    goal,
                    actor.action_state,
                    order_id,
                    door_pass_anim,
                    is_final_waypoint,
                    order_action,
                    seq_id,
                    elem_idx,
                    active_move_flags,
                    order_tolerance,
                    order_compute_direction,
                    order_reverse,
                    transition_distance_continuation,
                    deferred_movement_state_start,
                    next_destination_same_action,
                    legacy_serialized_order_chain,
                )
            };

            if order_action == OrderType::Freezing {
                // `MOVE_WAITING` carries a temporary FREEZING order while
                // the pathfinder owns the request.  The original
                // RHElementActor::Execute arm returns IN_PROGRESS without
                // touching the sprite; this token has no destination-backed
                // motion state to initialize or validate.
                continue;
            }

            if order_action == OrderType::PassingDoor {
                let eid = entity_id;
                if entity
                    .actor_data()
                    .expect("door-pass action point owner is not an actor")
                    .active_door_pass
                    .is_none()
                {
                    assert!(
                        legacy_serialized_order_chain,
                        "runtime door-pass action point {order_action:?} for {eid:?} lost its active pass"
                    );
                    // Original saves the complete translated order queue,
                    // PositionInterface door pointer/direction, and the
                    // actor's direct flag. It has no separate ActiveDoorPass.
                    // Execute that authoritative queue directly: the first
                    // PassingDoor consumes the saved door and changes sector;
                    // a later one sees NULL and merely restores
                    // anti-collision.
                    let door = entity.position_iface().get_door();
                    if door.is_null() {
                        entity.position_iface_mut().set_anti_collision_on(true);
                    } else {
                        let direct = entity.position_iface().get_door_direction();
                        door_triggers.push((eid, crate::gate::DoorIndex::from(door.0), direct, 0));
                        entity.position_iface_mut().clear_door();
                    }
                    self.orders.messenger.send(crate::messenger::Message::new(
                        crate::messenger::MessageType::Simple(
                            crate::messenger::SimpleMessage::Stature,
                        ),
                    ));
                    order_pops.push((move_seq_id, move_elem_idx));
                    continue;
                }
                let actor = entity
                    .actor_data_mut()
                    .expect("door-pass action point owner is not an actor");
                let dp = actor.active_door_pass.as_mut().unwrap_or_else(|| {
                    panic!("door-pass action point {order_action:?} for {eid:?} has no active pass")
                });
                let trigger_num = dp.triggers_fired;
                dp.triggers_fired += 1;
                door_triggers.push((eid, dp.door_index, dp.direct, trigger_num));

                let advance = Self::advance_door_pass(
                    actor,
                    eid,
                    goal,
                    &mut door_triggers,
                    &mut select_triggers,
                    &mut self.orders.next_order_id,
                );
                match advance {
                    DoorPassAdvance::Continue {
                        destination,
                        action,
                        reverse,
                        compute_direction,
                        tolerance,
                    } => {
                        let order_id = crate::order::alloc_order_id(&mut self.orders.next_order_id);
                        let mut order = crate::order::Order::new(
                            action,
                            destination.x,
                            destination.y,
                            order_id,
                        );
                        order.reverse = reverse;
                        order.compute_direction = compute_direction;
                        order.tolerance = tolerance;
                        transition_pushes.push((move_seq_id, move_elem_idx, order));
                    }
                    DoorPassAdvance::Paused { transition_order } => {
                        transition_pushes.push((move_seq_id, move_elem_idx, transition_order));
                    }
                    DoorPassAdvance::ActionPoint { order } => {
                        transition_pushes.push((move_seq_id, move_elem_idx, order));
                    }
                    DoorPassAdvance::Done { completed } => {
                        if let Some((door_index, direct)) = completed {
                            completed_door_passes.push((eid, door_index, direct));
                        }
                        actor.clear_path();
                        actor.active_movement.clear();
                        actor.active_door_pass = None;
                    }
                    DoorPassAdvance::NoActive => {
                        panic!(
                            "door-pass action point {order_action:?} for {eid:?} lost its active pass"
                        );
                    }
                }
                self.orders.messenger.send(crate::messenger::Message::new(
                    crate::messenger::MessageType::Simple(crate::messenger::SimpleMessage::Stature),
                ));
                order_pops.push((move_seq_id, move_elem_idx));
                continue;
            }

            let tolerance_arrival = seek_tolerance_reached(
                entity.element_data().position_map(),
                entity.element_data().sector(),
            );
            // Original ages this countdown only inside `PerformSeek`, and
            // dispatches there by the current animation arm rather than by
            // RHMOVE_SEEK alone. Cross-sector wall/ladder orders therefore
            // retain the flag but freeze this counter while their Execute
            // arms call PerformMotion directly. Keep the actual decrement
            // ahead of transition and zero-motion early returns; a successful
            // pre-motion post-seek arrival is the one path that returns before
            // it.
            let perform_seek_calls = perform_seek_calls_per_execute(order_action);
            if ft.target_id.is_some()
                && active_move_flags.contains(crate::sequence::MoveFlags::SEEK)
                && perform_seek_calls != 0
                && !(tolerance_arrival && ft.has_post_seek)
            {
                let actor = entity.actor_data_mut().expect("movement owner is actor");
                let wait_before = actor.seek_refresh_wait;
                for _ in 0..perform_seek_calls {
                    actor.seek_refresh_wait = age_seek_refresh_wait(actor.seek_refresh_wait);
                }
                // Original performs this aging directly on the overloaded
                // `mulWaitTime` member. Keep the Rust ordinary-wait copy in
                // sync while seek owns that legacy scalar so every possible
                // exit (post-seek interaction, a following Move, interruption
                // or cancellation) retains the last wrapped value.
                actor.wait_time = actor.seek_refresh_wait;
                tracing::trace!(
                    entity = ?entity_id,
                    wait_before,
                    wait_after = actor.seek_refresh_wait,
                    perform_seek_calls,
                    tolerance_arrival,
                    has_post_seek = ft.has_post_seek,
                    "entity-target PerformSeek aged refresh countdown"
                );
            }

            let soldier_attentive = matches!(entity, crate::element::Entity::Soldier(_))
                && entity.enemy_ai().is_some_and(|enemy| enemy.attentive);
            let execute_order_initialising = entity
                .actor_data()
                .expect("movement owner lost actor initialization state")
                .execute_order_initialising;
            if execute_order_initialising && is_authored_climb_action(order_action) {
                // Every climb Execute arm calls SetDirection(lift direction)
                // and clears the selected order's bComputeDirection during
                // initialization. Without the clear, PerformMotion
                // immediately replaces that lift-facing goal with the
                // destination vector. This is observable when a save resumes
                // with mbNewOrder set on an already-running climb.
                let order = self
                    .orders
                    .sequence_manager
                    .get_element_mut(move_seq_id, move_elem_idx)
                    .and_then(|element| element.orders.front_mut())
                    .filter(|order| Some(order.order_id) == order_id)
                    .unwrap_or_else(|| {
                        panic!(
                            "initializing climb owner {entity_id:?} lost selected order {order_id:?}"
                        )
                    });
                order.compute_direction = false;
                order_compute_direction = false;
            }
            let elem = entity.element_data_mut();
            let dx = goal.x - elem.position_map().x;
            let dy = goal.y - elem.position_map().y;
            let dist = (dx * dx + dy * dy).sqrt();
            // Combat movement: face opponent, select directional
            // animation.  `compute_direction=false` (don't auto-face
            // movement direction), face toward opponent, pick
            // forward/backward/strafe animation based on angle between
            // movement vector and facing vector.
            let combat_target = combat_face_targets[actor_id];
            let door_pass_sword_nonanimation =
                door_pass_anim.is_some_and(is_sword_movement_nonanimation);
            let order_sword_nonanimation = is_sword_movement_nonanimation(order_action);
            let forced_sword_motion =
                active_move_flags.contains(crate::sequence::MoveFlags::FORCE_SWORD_MOVEMENT);
            let is_sword_motion = matches!(
                action_state,
                crate::element::ActionState::MovingSword
                    | crate::element::ActionState::MovingFastSword
            ) || door_pass_sword_nonanimation
                || order_sword_nonanimation
                || forced_sword_motion;
            let is_shield_motion =
                matches!(action_state, crate::element::ActionState::MovingShield);
            let is_combat = (is_shield_motion && combat_target.is_some()) || is_sword_motion;
            if is_combat {
                // Face opponent instead of movement direction.  Use
                // `set_direction_goal` + per-frame `turn()` rather
                // than instantly snapping facing, so the facing
                // rotates one step per frame toward the opponent.
                if let Some(opp_pos) = combat_target {
                    let fdx = opp_pos.x - elem.position_map().x;
                    let fdy = opp_pos.y - elem.position_map().y;
                    if fdx * fdx + fdy * fdy > 0.01 {
                        elem.set_direction_goal(
                            crate::position_interface::vector_to_sector_0_to_15_iso(fdx, fdy),
                        );
                    }
                }
            }
            // Ordinary movement does not recompute facing from the remaining
            // map-space goal every tick. Original ComputeIncrementAll stamps
            // the goal once from its normalized 3D increment (including the
            // live ground plane), then returns early while that increment
            // remains valid. PerformMotion below owns that initialization;
            // anti-collision and path-boundary code explicitly invalidate and
            // rebuild it when the trajectory actually changes.

            // Choose animation based on action state and movement angle.
            let anim = if let Some(dp_anim) =
                door_pass_anim.filter(|anim| !is_sword_movement_nonanimation(*anim))
            {
                // PassDoor supplies the current translated movement step, but
                // Soldier::Execute still dispatches that logical action
                // through its attentive-animation override. In particular,
                // an attentive WalkingUpright door step plays
                // WalkingAlerted and therefore uses its distinct frame
                // distances.
                super::animation::soldier_movement_animation(
                    dp_anim,
                    soldier_attentive,
                    action_state,
                )
            } else if is_combat {
                if is_sword_motion && combat_target.is_none() {
                    // Plain WALKING_SWORD when a non-soldier is forced
                    // through sword movement without an active
                    // opponent.  The `WalkingWithSword` /
                    // `RunningWithSword` values are non-animations and
                    // must never be sent directly to the per-frame
                    // motion update.
                    if elem.sprite.has_animation(OrderType::WalkingSword) {
                        OrderType::WalkingSword
                    } else {
                        order_action
                    }
                } else {
                    // Compute angle between movement direction and
                    // facing direction, normalised to [0, 2π).
                    // UNIT = π/4 (45°).  8-sector mapping:
                    //   [0, π/4) or [7π/4, 2π) → forward
                    //   [π/4, 3π/4)             → strafe right
                    //   [3π/4, 5π/4)            → backward
                    //   [5π/4, 7π/4)            → strafe left
                    let facing_angle = if let Some(opp_pos) = combat_target {
                        let fdx = opp_pos.x - elem.position_map().x;
                        let fdy = opp_pos.y - elem.position_map().y;
                        if fdx * fdx + fdy * fdy > 0.01 {
                            fdy.atan2(fdx)
                        } else {
                            (elem.direction() as f32) * std::f32::consts::PI / 8.0
                        }
                    } else {
                        (elem.direction() as f32) * std::f32::consts::PI / 8.0
                    };
                    let move_angle = dy.atan2(dx);
                    let angle = combat_movement_angle(move_angle, facing_angle);
                    // MovingSword and MovingFastSword both use the
                    // directional walking/strafing sword animations — the
                    // `fast` flag is ignored when selecting the animation.
                    // Running in combat is implemented by playing the walking
                    // animation under `MotionMethod::Fast`.
                    let sword_anim = combat_directional_animation(action_state, angle);
                    if elem.sprite.has_animation(sword_anim) {
                        sword_anim
                    } else {
                        order_action
                    }
                }
            } else {
                // Animation comes from the current order's type —
                // dispatch is on `order.action`.  Order types get
                // rewritten by `MakeFast` / `MakeSlow` / `MakeUpright`
                // / `MakeCrouched`, so reading the order directly is
                // how a mid-movement speed change propagates to the
                // sprite.  Falls back to an action_state-derived base
                // only when the order type isn't a movement animation
                // (shouldn't happen for a Move element but is
                // defensive).
                let base = match order_action {
                    OrderType::WalkingUpright
                    | OrderType::WalkingCrouched
                    | OrderType::WalkingAlerted
                    | OrderType::RunningUpright
                    | OrderType::TransitionWalkingUprightRunningUpright
                    | OrderType::TransitionRunningUprightWalkingUpright
                    | OrderType::TransitionWaitingUprightWalkingUpright
                    | OrderType::TransitionWalkingUprightWaitingUpright
                    | OrderType::TransitionWaitingUprightRunningUpright
                    | OrderType::TransitionRunningUprightWaitingUpright
                    | OrderType::TransitionWalkingCrouchedWalkingUpright
                    | OrderType::TransitionWalkingUprightWalkingCrouched
                    | OrderType::TransitionWalkingCrouchedRunningUpright
                    | OrderType::TransitionRunningUprightWalkingCrouched
                    | OrderType::TransitionWaitingCrouchedWalkingCrouched
                    | OrderType::TransitionWalkingCrouchedWaitingCrouched
                    | OrderType::TransitionWaitingUprightSpecial
                    | OrderType::TransitionSpecialWaitingUpright
                    | OrderType::TransitionWaitingUprightBoredWaitingUpright
                    | OrderType::TransitionWaitingUprightWaitingUprightBored
                    | OrderType::TransitionCrouchingUp
                    | OrderType::TransitionCrouchingDown
                    | OrderType::TransitionSittingWaitingUpright
                    | OrderType::TransitionLeaningOutWaitingAlerted
                    | OrderType::LoweringShield
                    | OrderType::WalkingStairs
                    | OrderType::RunningStairs
                    | OrderType::ClimbingWallUp
                    | OrderType::ClimbingWallDown
                    | OrderType::ClimbingWallUpFast
                    | OrderType::ClimbingWallDownFast
                    | OrderType::ClimbingLadderUp
                    | OrderType::ClimbingLadderDown
                    | OrderType::ClimbingLadderUpAlerted
                    | OrderType::ClimbingLadderDownAlerted
                    | OrderType::ClimbingLadderUpFast
                    | OrderType::ClimbingLadderDownFast => order_action,
                    _ => match action_state {
                        crate::element::ActionState::MovingFast => OrderType::RunningUpright,
                        _ => OrderType::WalkingUpright,
                    },
                };
                // `RHElementActorSoldier::Execute` keeps the authored order
                // unchanged but plays the corresponding alerted sprite
                // animation while the soldier is attentive.  This applies to
                // movement orders too: the alerted start/stop transitions have
                // different frame delays, and WalkingAlerted has different
                // per-frame distances.  Apply the substitution before lift
                // translation so ladder/wall selection also sees the effective
                // upright animation, as in the Original dispatch chain.
                let base = super::animation::soldier_movement_animation(
                    base,
                    soldier_attentive,
                    action_state,
                );
                // DetermineMovementAnimation translates the movement
                // element's primary distance-producing action while it is
                // instructed. PostProcessPath runs afterwards and may insert
                // explicit start/end transition orders; Execute dispatches
                // those transition actions literally even when the actor is
                // standing in a live lift sector. Applying the lift map to a
                // transition here would, for example, turn a walk-to-run
                // transition on stairs back into WalkingStairs.
                //
                // For ordinary distance motion, upright posture takes the
                // upwards mapping unconditionally; on-ladder / on-wall
                // posture chooses upwards vs downwards by dot-producting the
                // ladder vector (`pt_low - pt_high`) with the movement
                // vector. Snapshotted in `lift_translations` so we don't have
                // to re-borrow the grid or door table mid-loop.
                if !order_uses_distance_motion(order_action) || is_authored_climb_action(base) {
                    // DetermineMovementAnimation rewrites the movement
                    // element once when it is instructed. Every path order
                    // retains that authored climb direction, even if a later
                    // waypoint briefly bends the other way.
                    base
                } else {
                    match lift_translations[actor_id] {
                        Some(LiftAnimContext::Upright(lt)) => lt.translate_upright_action(base),
                        Some(LiftAnimContext::OnClimb {
                            lift_type,
                            lift_direction: _,
                            ladder_dx,
                            ladder_dy,
                        }) => {
                            let going_down = ladder_dx * dx + ladder_dy * dy >= 0.0;
                            lift_type.translate_climb_action(base, going_down)
                        }
                        None => base,
                    }
                }
            };
            // Advance sprite animation and get per-frame distance.
            // PerformMotion sets `row = conversion[anim] + direction`,
            // increments the frame, then reads `GetDistance(row,
            // frame)` only when `frame_count == 0` (the first tick of
            // a new animation frame).  Between frames the distance is
            // 0, so entities move in discrete steps synced to the
            // animation.
            //
            // Motion methods:
            //   Walk / Run: normal frame distance * speed_factor
            //   Fast: double frame rate + double distance (only used
            //     for RUNNING_WITH_SWORD in combat, NOT for normal
            //     running)
            // Normal running uses Run, which is identical to Walk in
            // distance calculation — only the animation differs.  The
            // running animation's per-frame distances in the sprite
            // data are already larger than walking distances.
            //
            // The per-frame sprite distance is scaled by the active
            // sequence element's speed factor.  PC-issued moves use
            // 1.0; shield-following and the AI patrol/approach paths
            // set variable factors.
            //
            // Shield-follower speed adjust: when a PC in MovingShield
            // action state is seeking an actor target (the shield
            // holder), the sequence element's speed factor is
            // rewritten per tick to close gaps quickly and slow down
            // when near.
            //   dist² < 25  → 1.0
            //   dist² < 100 → 1.5
            //   else        → 2.0
            // We override the captured value so `current_frame_distance
            // * speed_factor` sees the adjusted value this tick.  The
            // captured value is reread from the element next tick.
            {
                let ft = final_tolerances[actor_id];
                if ft.tol > 0.0
                    && ft.target_is_actor
                    && matches!(action_state, crate::element::ActionState::MovingShield)
                {
                    let (sdx, sdy) = ft
                        .shield_destination
                        .or(live_seek_target.map(|(position, _, _)| position))
                        .map(|p| (p.x - elem.position_map().x, p.y - elem.position_map().y))
                        .unwrap_or((dx, dy));
                    let dist_sq = sdx * sdx + sdy * sdy;
                    speed_factors[actor_id] = if dist_sq < 25.0 {
                        1.0
                    } else if dist_sq < 100.0 {
                        1.5
                    } else {
                        2.0
                    };
                }
            }
            let speed_factor = speed_factors[actor_id];
            // Dispatch by order action: transition-animation orders
            // route to `MotionMethod::TillLastFrame`, while walking /
            // running orders route to `MotionMethod::Walk` (or
            // `MotionMethod::Fast` for RUNNING_WITH_SWORD).  The
            // TillLastFrame branch advances the order on animation
            // loop (`Terminated`) rather than on position arrival,
            // which is the right semantics for zero-distance pose
            // changes whose destination is already the actor's current
            // position.
            // Distance-producing movement animations use Walk/Fast.
            // Everything else (transitions, posture-changes, misc)
            // dispatched via tick_move maps to TillLastFrame.
            let is_movement_anim = order_uses_distance_motion(order_action);
            let is_transition_anim = !is_movement_anim;
            // Execute's transition arms have two distinct C++ call paths.
            // Ordinary transitions call PerformMotion directly and retain its
            // default factor of 1. Seek transitions call PerformSeek, which
            // forwards the movement element's speed factor to PerformMotion.
            let apply_speed_factor =
                !is_transition_anim || active_move_flags.contains(crate::sequence::MoveFlags::SEEK);
            // RHElementActorHuman::Execute selects FAST solely from the
            // current logical movement token. The actor can still be in
            // MOVING_FAST_SWORD when a newly selected WALKING_WITH_SWORD
            // order starts; carrying that old state into the method choice
            // would execute the walking order twice before its START side
            // effect changes the state to MOVING_SWORD.
            let fast_sword_motion = order_action == OrderType::RunningWithSword
                || door_pass_anim == Some(OrderType::RunningWithSword);
            // Fast stairs/ladder/wall actions are non-animation dispatch
            // tokens: the Original executes the ordinary sprite motion
            // twice. Lift
            // translation above may therefore turn an already-authored fast
            // token into its ordinary sprite action; retain the dispatch
            // semantics from the sequence order itself.
            let fast_climb_motion =
                is_fast_climb_action(order_action) || is_fast_climb_action(anim);
            let motion_method = if is_transition_anim {
                MotionMethod::TillLastFrame
            } else if fast_sword_motion {
                MotionMethod::Fast
            } else {
                MotionMethod::Walk
            };
            if let Some(LiftAnimContext::OnClimb {
                lift_type,
                lift_direction,
                ..
            }) = lift_translations[actor_id]
            {
                match (anim, lift_type) {
                    (
                        OrderType::ClimbingWallUp
                        | OrderType::ClimbingWallDown
                        | OrderType::ClimbingWallUpFast
                        | OrderType::ClimbingWallDownFast,
                        crate::sector::LiftType::Wall,
                    )
                    | (
                        OrderType::ClimbingLadderUp
                        | OrderType::ClimbingLadderDown
                        | OrderType::ClimbingLadderUpFast
                        | OrderType::ClimbingLadderDownFast,
                        crate::sector::LiftType::Ladder,
                    ) => elem.set_direction_goal(lift_direction),
                    _ => {}
                }
            }
            match (anim, door_pass_anim) {
                (
                    OrderType::ClimbingWallUp
                    | OrderType::ClimbingWallDown
                    | OrderType::ClimbingWallUpFast
                    | OrderType::ClimbingWallDownFast,
                    Some(_),
                ) => {
                    elem.posture = crate::element::Posture::OnWall;
                }
                (
                    OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel
                    | OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel,
                    Some(_),
                ) if execute_order_initialising => {
                    elem.posture = crate::element::Posture::Flying;
                }
                (
                    OrderType::TransitionClimbingWallUpWaitingCrouched
                    | OrderType::TransitionClimbingWallDownWaitingUpright,
                    Some(_),
                ) => {
                    elem.posture = crate::element::Posture::OnWall;
                }
                (
                    OrderType::ClimbingLadderUp
                    | OrderType::ClimbingLadderDown
                    | OrderType::ClimbingLadderUpFast
                    | OrderType::ClimbingLadderDownFast,
                    Some(_),
                ) => {
                    elem.posture = crate::element::Posture::OnLadder;
                }
                _ => {}
            }
            if execute_order_initialising
                && let Some(climb_dir) = door_pass_climb_directions[actor_id]
            {
                let dir = if matches!(
                    (anim, elem.posture),
                    (
                        OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel,
                        crate::element::Posture::Flying
                    )
                ) {
                    (climb_dir + 8) & 15
                } else {
                    climb_dir
                };
                elem.set_direction_goal(dir);
            }

            let motion_order = order_id.map(|order_id| MotionOrderContext {
                order_id,
                destination: goal,
                reverse: order_reverse,
                tolerance: order_tolerance,
                directional_tolerance: active_move_flags
                    .contains(crate::sequence::MoveFlags::DIRECTIONAL_TOLERANCE),
                compute_direction: order_compute_direction,
                next_destination_same_action,
            });

            if let Some(motion_order) = motion_order
                && let Some(mismatch) = elem.sprite.motion_order_state_mismatch(motion_order)
            {
                panic!(
                    "movement order state invariant failed for entity {entity_id:?}, order {order_action:?}, id {}: {mismatch:?}",
                    motion_order.order_id
                );
            }

            // Fast stairs/ladder/wall Execute is two literal
            // Turn/PerformMotion pairs in Original. The second pair is
            // skipped when the first
            // motion terminates; folding it into MotionMethod::Fast would
            // over-rotate on that terminal tick and cannot expose the first
            // call's termination barrier.
            // Original short-circuits a newly initialized non-transition
            // motion only for exact `pointDestination2D == GetPositionMap()`.
            // A near-target continuation must still run PerformMotion so its
            // ordinary arrival path snaps and retires it in this owner slot.
            let dest_already_at_pos =
                motion_method != MotionMethod::TillLastFrame && elem.position_map() == goal;
            let sprite = &mut elem.sprite;
            // FaceOpponent / FaceDangerPoint calls Turn before PerformSeek
            // tests its entity-target tolerance. Preserve that turn on the
            // successful pre-motion branch; the ordinary branch below already
            // performs the effective per-frame turn before PerformMotion.
            if tolerance_arrival && is_combat {
                let _ = sprite.position_iface.turn();
            }
            // Entity-target PerformSeek returns from its successful
            // pre-motion tolerance branch without calling PerformMotion.
            // Besides avoiding displacement, this preserves the prior sprite
            // action and suppresses START-owned side effects such as combat
            // initiative transfer. When StartPostSeekSequence succeeds the
            // wrapper returns TERMINATED, however; the surrounding Execute
            // arm must still observe that result so a pending movement-end
            // transition applies its terminal posture/action-state effect
            // before the interaction is instructed.
            let (mut motion_state, mut frame_dist_raw) = if tolerance_arrival {
                (
                    if ft.has_post_seek {
                        MotionState::Terminated
                    } else {
                        MotionState::InProgress
                    },
                    0.0,
                )
            } else {
                // Entity-target PerformSeek tests its successful tolerance
                // branch before the ordinary Turn/PerformMotion block. Do
                // not advance anti-vibration turning on a terminal tolerance
                // sample whose post-seek sequence is taking over.
                let _ = sprite.position_iface.turn();
                sprite.perform_motion(
                    sim,
                    motion_order,
                    sprite_motion_order_for_nonanimation(anim),
                    u16::from(sprite.position_iface.get_direction().as_u8()),
                    FrameProgression::Default,
                    false,
                    motion_method,
                    dest_already_at_pos,
                )
            };
            let first_frame_dist_raw = frame_dist_raw;
            let first_direction_differs_from_goal =
                sprite.position_iface.get_direction() != sprite.position_iface.get_direction_goal();
            // Fast climb Execute contains two literal PerformMotion calls, but
            // returns immediately when the first one reaches the order goal.
            // Project that first call through the same anti-collision query
            // used by the committed movement below.  Deferring all position
            // work until after both sprite calls otherwise advances the
            // animation counter once too often on a terminal first call; the
            // next climb order can then move one simulation frame early.
            let first_fast_call_terminates = if !tolerance_arrival
                && fast_climb_motion
                && motion_state != MotionState::Terminated
            {
                let first_speed = scaled_motion_distance(
                    first_frame_dist_raw,
                    speed_factor,
                    apply_speed_factor,
                    first_direction_differs_from_goal,
                );
                if first_speed == 0.0 {
                    false
                } else {
                    let mut projected = sprite.position_iface.clone();
                    let increment = projected.get_increment_map();
                    let anti_on = projected.is_anti_collision_on();
                    let (dx_step, dy_step) = if anti_on
                        && let Some(mover_snap) =
                            anti_snapshots.get(actor_id).and_then(|slot| slot.as_ref())
                    {
                        let move_box = *projected.get_move_box();
                        let half_diagonal = projected.get_half_diagonal();
                        let mut state = super::anti_collision::AntiCollisionState {
                            pi: &mut projected,
                            move_box,
                            half_diagonal,
                            goal_map: goal,
                        };
                        super::anti_collision::apply_anti_collision_step(
                            mover_snap,
                            anti_snapshots.as_slice(),
                            &self.ai.global.repulsive_points,
                            prepared
                                .mobile_points_by_layer
                                .get(&mover_snap.layer)
                                .map(Vec::as_slice)
                                .unwrap_or(&[]),
                            prepared
                                .mobile_lines_by_layer
                                .get(&mover_snap.layer)
                                .map(Vec::as_slice)
                                .unwrap_or(&[]),
                            prepared
                                .mobile_polygons_by_layer
                                .get(&mover_snap.layer)
                                .map(Vec::as_slice)
                                .unwrap_or(&[]),
                            Some(&self.world.fast_grid),
                            Some(&mut state),
                            increment.x,
                            increment.y,
                            first_speed,
                            true,
                        )
                    } else {
                        (increment.x * first_speed, increment.y * first_speed)
                    };
                    let mut projected_position = projected.map_position();
                    projected_position.x += dx_step;
                    projected_position.y += dy_step;
                    projected.set_map_position(projected_position);
                    projected.is_goal_reached(&self.world.fast_grid, None)
                }
            } else {
                false
            };
            let mut second_frame_dist_raw = None;
            if !tolerance_arrival
                && fast_climb_motion
                && motion_state != MotionState::Terminated
                && !first_fast_call_terminates
            {
                let _ = sprite.position_iface.turn();
                let (second_state, second_distance) = sprite.perform_motion(
                    sim,
                    motion_order,
                    sprite_motion_order_for_nonanimation(anim),
                    u16::from(sprite.position_iface.get_direction().as_u8()),
                    FrameProgression::Default,
                    false,
                    MotionMethod::Walk,
                    dest_already_at_pos,
                );
                motion_state = second_state;
                second_frame_dist_raw = Some(second_distance);
                frame_dist_raw += second_distance;
            }
            executed_sword_movement = is_sword_motion;
            if is_pc {
                executed_pc_movement_actions.push((entity_id, order_action));
            }
            if door_pass_anim.is_some()
                && matches!(motion_state, MotionState::Start)
                && matches!(
                    anim,
                    OrderType::TransitionClimbingLadderUpWaitingCrouched
                        | OrderType::TransitionClimbingLadderUpWaitingUprightAlerted
                )
            {
                door_pass_transition_start_effects.push(entity_id);
            }
            if door_pass_anim.is_some()
                && matches!(motion_state, MotionState::Done)
                && matches!(
                    anim,
                    OrderType::TransitionWaitingUprightClimbingWallUp
                        | OrderType::TransitionClimbingWallUpWaitingCrouched
                        | OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel
                        | OrderType::TransitionWaitingCrouchedClimbingWallDown
                        | OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel
                        | OrderType::TransitionClimbingWallDownWaitingUpright
                        | OrderType::TransitionClimbingLadderUpWaitingCrouched
                        | OrderType::TransitionClimbingLadderUpWaitingUprightAlerted
                )
            {
                door_pass_transition_done_effects.push(entity_id);
            }
            if active_move_flags.contains(crate::sequence::MoveFlags::RIDER_CHARGE)
                && anim == OrderType::RunningUpright
            {
                let frame_count = sprite.num_frames_for_anim(OrderType::RunningUpright);
                let cur = sprite.current_frame;
                if is_galopp_decision_frame(cur, frame_count) {
                    assert_eq!(
                        entity_id, owner,
                        "owner-local rider Execute collected a gallop callback for another actor"
                    );
                    galopp_event = true;
                }
            }
            // `PerformMotion` applies the sequence speed factor before its
            // turn slowdown and 0.7-unit minimum. The order is observable:
            // a slow patrol member with raw distance 2 and factor ~0.58 is
            // clamped to exactly 0.7 after the 0.6 multiplier, rather than
            // scaling an already-clamped 0.7 back below the minimum.
            //
            // PerformMotion initializes a new order's direction goal after
            // the caller's Turn() above. The slowdown test in Original
            // happens later and reads the now-live direction/goal pair, so it
            // applies even though the pre-initialization Turn was a no-op.
            let direction_differs_from_goal =
                sprite.position_iface.get_direction() != sprite.position_iface.get_direction_goal();
            // Direct transition Execute arms call `PerformMotion(...,
            // RHMOTIONMETHOD_TILL_LAST_FRAME)` without a speed factor. Seek
            // transitions instead route through PerformSeek and do pass it.
            let speed = if let Some(second_distance) = second_frame_dist_raw {
                // The fast stairs/ladder/wall arms contain two literal
                // PerformMotion calls. Each call applies its own turning
                // slowdown using the direction reached by the immediately
                // preceding Turn(), so a first call that is still rotating
                // must not inherit the second call's newly aligned state.
                scaled_motion_distance(
                    first_frame_dist_raw,
                    speed_factor,
                    apply_speed_factor,
                    first_direction_differs_from_goal,
                ) + scaled_motion_distance(
                    second_distance,
                    speed_factor,
                    apply_speed_factor,
                    direction_differs_from_goal,
                )
            } else {
                scaled_motion_distance(
                    frame_dist_raw,
                    speed_factor,
                    apply_speed_factor,
                    direction_differs_from_goal,
                )
            };
            // PerformMotion applies the distance before returning its motion
            // state. A fresh walking order that reaches its goal on that same
            // invocation returns TERMINATED, not START, so the walking
            // Execute arm does not enter the Moving action state. Our
            // position update is staged below; fold that imminent arrival
            // into the state-effect result now.
            let entity_target_seek = active_move_flags.contains(crate::sequence::MoveFlags::SEEK)
                && ft.target_id.is_some();
            // With anti-collision disabled the committed step is exactly
            // `increment * speed`. Original PerformMotion applies that step,
            // then tests the order's ordinary tolerance before returning to
            // Execute. Account for an imminent tolerance arrival here so a
            // fresh walking order that terminates on its first call does not
            // expose the START-only Moving state. Door-pass approach points
            // intentionally rely on this (their distance is often exactly
            // the authored tolerance).
            let reaches_order_tolerance_this_step = !is_transition_anim
                && speed > 0.0
                && order_tolerance > 0.0
                && !sprite.position_iface.is_anti_collision_on()
                && dist <= speed + order_tolerance;
            let state_effect_motion = movement_execute_visible_motion(
                order_action,
                motion_state,
                !is_transition_anim && (dist <= speed || reaches_order_tolerance_this_step),
                entity_target_seek,
            );
            let deferred_movement_state_start_due = if deferred_movement_state_start {
                let current_order = self
                    .orders
                    .sequence_manager
                    .get_element_mut(move_seq_id, move_elem_idx)
                    .and_then(|element| element.orders.front_mut())
                    .unwrap_or_else(|| {
                        panic!(
                            "deferred movement-state successor for {entity_id:?} disappeared during execution"
                        )
                    });
                assert_eq!(
                    Some(current_order.order_id),
                    order_id,
                    "deferred movement-state successor changed identity during execution"
                );
                assert!(
                    take_deferred_movement_state_start(
                        &mut current_order.deferred_movement_state_start
                    ),
                    "deferred movement-state successor marker was already consumed"
                );
                true
            } else {
                false
            };
            // The initiative handoff belongs to the Human Execute START arm,
            // so it observes entity-target PerformSeek's wrapper result just
            // like posture/action-state changes do. A raw sprite START hidden
            // as IN_PROGRESS by PerformSeek must not transfer initiative.
            if matches!(state_effect_motion, MotionState::Start) && is_sword_motion {
                sword_movement_starts.push(entity_id);
            }
            tracing::trace!(
                entity = ?entity_id,
                frame = self.control.frame_counter,
                ?order_action,
                ?motion_state,
                ?state_effect_motion,
                action_state = ?action_state,
                sprite_frame = sprite.current_frame,
                sprite_counter = sprite.frame_count,
                sprite_num_frames = sprite.num_frames_for_row(sprite.current_row),
                sprite_wait = sprite.wait_time(sprite.current_row, sprite.current_frame),
                frame_distance_raw = frame_dist_raw,
                speed_factor,
                effective_distance = speed,
                remaining_distance = dist,
                "movement Execute result"
            );
            // Transition motion can still change from InProgress to
            // Terminated in the TILL_LAST_FRAME arrival handling below.
            // Original applies the Execute switch's state side effect after
            // PerformMotion returns that final result, before Proceed rewrites
            // the diagnostic motion for a successor order.
            let transition_distance_first_execute_due = if transition_distance_continuation {
                let element = self
                    .orders
                    .sequence_manager
                    .get_element_mut(move_seq_id, move_elem_idx)
                    .unwrap_or_else(|| {
                        panic!(
                            "transition-distance continuation for {entity_id:?} disappeared during its first execution"
                        )
                    });
                let current_order = element.orders.front_mut().unwrap_or_else(|| {
                    panic!(
                        "transition-distance continuation for {entity_id:?} lost its current order during its first execution"
                    )
                });
                assert_eq!(
                    Some(current_order.order_id),
                    order_id,
                    "transition-distance continuation changed identity during its first execution"
                );
                take_transition_distance_first_execute(
                    &mut current_order.transition_distance_continuation,
                )
            } else {
                false
            };
            let suppress_transition_continuation_start = transition_distance_first_execute_due
                && matches!(state_effect_motion, MotionState::Start);
            if !is_transition_anim
                && !suppress_transition_continuation_start
                // A deferred PC successor deliberately postpones this
                // START-only state effect until after order completion has
                // decided whether the authored walking order survived.  The
                // guarded handoff below owns that one-shot side effect.
                && !deferred_movement_state_start_due
                // PerformMotion commits the physical step before returning.
                // A fresh order can therefore reach its goal and return
                // TERMINATED instead of exposing START to Execute. Defer all
                // START-only effects until the committed step has decided
                // whether this exact order survives.
                && !matches!(state_effect_motion, MotionState::Start)
                && let Some((posture, action_state)) =
                    movement_execute_state_effect(order_action, state_effect_motion)
            {
                movement_state_effects.push((entity_id, posture, action_state));
            }
            if is_transition_anim
                && tolerance_arrival
                && let Some((posture, action_state)) =
                    movement_execute_state_effect(order_action, state_effect_motion)
            {
                // PerformSeek's successful pre-motion post-seek branch skips
                // the transition sprite call, but its TERMINATED result still
                // returns through the surrounding transition Execute arm.
                movement_state_effects.push((entity_id, posture, action_state));
            }

            if door_pass_anim.is_some()
                && matches!(
                    anim,
                    OrderType::ClimbingWallUp
                        | OrderType::ClimbingWallDown
                        | OrderType::ClimbingWallUpFast
                        | OrderType::ClimbingWallDownFast
                        | OrderType::TransitionWaitingUprightClimbingWallUp
                        | OrderType::TransitionClimbingWallUpWaitingCrouched
                        | OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel
                        | OrderType::TransitionWaitingCrouchedClimbingWallDown
                        | OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel
                        | OrderType::TransitionClimbingWallDownWaitingUpright
                )
            {
                let goal_dir = elem.sprite.position_iface.get_direction_goal().as_u8();
                tracing::debug!(
                    entity = ?entity_id,
                    ?anim,
                    posture = ?elem.posture,
                    action_state = ?action_state,
                    dir = elem.direction(),
                    goal_dir,
                    row = elem.sprite.current_row,
                    frame = elem.sprite.current_frame,
                    pos_x = elem.position_map().x,
                    pos_y = elem.position_map().y,
                    pos3_y = elem.position().y,
                    pos3_z = elem.position().z,
                    speed,
                    dist,
                    "DoorPass movement state"
                );
            }

            // TillLastFrame branch: transition animations advance via
            // the animation-loop `Terminated` edge, not via position
            // arrival.  Still update position by the sprite's
            // per-frame distance along the vector toward the order's
            // target — end-of-run transitions carry ~26 units of
            // distance and must actually move the actor to reach the
            // goal (without this advance, soldiers stop at the
            // running-phase endpoint and never close the final ~26u
            // gap, leaving them outside sword_range forever and unable
            // to trigger begin_swordfight). C++ routes every nonzero
            // transition distance through UpdatePositionAntiCollision, so
            // transition displacement must also participate in elevation,
            // patch, and sound boundary crossing.
            //
            // C++ seeds `PositionInterface` at the start of every new
            // sprite motion order and moves transition animations via
            // `UpdatePositionMap(fDistance)`, so this branch uses the
            // same precomputed map increment instead of a separate
            // dx/dy step.
            // Entity-target PerformSeek checks its live tolerance before it
            // dispatches the current sprite order.  An already-in-range seek
            // therefore bypasses transition execution and enters the shared
            // post-seek/frozen arrival tail below.
            if is_transition_anim && !tolerance_arrival {
                let transition_has_map_target = goal.x != 0.0 || goal.y != 0.0;
                if !transition_has_map_target && !is_in_place_movement_transition(order_action) {
                    panic!(
                        "movement transition {:?} for entity {:?} has zero map target; refusing to treat (0,0) as an implicit destination",
                        order_action, entity_id
                    );
                }
                // A movement transition can legitimately target the actor's
                // exact current point (for example the generated
                // Waiting→Walking pose at the end of a combat sequence).
                // PerformMotion still advances that animation, but the zero
                // goal vector contributes no map displacement.  In
                // particular, do not feed a stale pre-order increment into
                // anti-collision: ComputeIncrementAll deliberately preserves
                // the stored vector when the new vector is zero.
                let transition_has_distance =
                    transition_has_map_target && speed > 0.0 && dist > f32::EPSILON;
                let transition_crossing_start = transition_has_distance.then(|| {
                    let old_pos = entity.element_data().position_map();
                    let layer = entity.element_data().layer();
                    let eligible = actor_line_crossing_eligible(
                        entity.element_data().posture,
                        human_is_carried,
                        self.world.fast_grid.level.map_bbox.contains_point(old_pos),
                    );
                    (old_pos, layer, eligible)
                });
                if transition_has_distance {
                    // Match GetIncrementMap(): PerformMotion seeded this
                    // normalized vector when the order began and reuses it
                    // unchanged until anti-collision explicitly rebuilds it.
                    let increment = entity.position_iface().get_increment_map();
                    let nx = increment.x;
                    let ny = increment.y;
                    let anti_on = entity.position_iface().is_anti_collision_on();
                    let goal_map = crate::coordinates::MapPoint::new(goal.x, goal.y);
                    let (move_box, half_diagonal) = {
                        let pi = entity.position_iface();
                        (*pi.get_move_box(), pi.get_half_diagonal())
                    };
                    let (dx_step, dy_step, deviated, recovered_from_deviation) =
                        if let Some(mover_snap) =
                            anti_snapshots.get(actor_id).and_then(|slot| slot.as_ref())
                        {
                            let pi = entity.position_iface_mut();
                            let was_deviated = pi.is_deviated();
                            let mut state = super::anti_collision::AntiCollisionState {
                                pi,
                                move_box,
                                half_diagonal,
                                goal_map,
                            };
                            let (dx_step, dy_step) =
                                super::anti_collision::apply_anti_collision_step(
                                    mover_snap,
                                    anti_snapshots.as_slice(),
                                    &self.ai.global.repulsive_points,
                                    prepared
                                        .mobile_points_by_layer
                                        .get(&mover_snap.layer)
                                        .map(Vec::as_slice)
                                        .unwrap_or(&[]),
                                    prepared
                                        .mobile_lines_by_layer
                                        .get(&mover_snap.layer)
                                        .map(Vec::as_slice)
                                        .unwrap_or(&[]),
                                    prepared
                                        .mobile_polygons_by_layer
                                        .get(&mover_snap.layer)
                                        .map(Vec::as_slice)
                                        .unwrap_or(&[]),
                                    Some(&self.world.fast_grid),
                                    Some(&mut state),
                                    nx,
                                    ny,
                                    speed,
                                    anti_on,
                                );
                            (
                                dx_step,
                                dy_step,
                                state.pi.is_deviated(),
                                was_deviated && !state.pi.is_deviated(),
                            )
                        } else {
                            (nx * speed, ny * speed, false, false)
                        };
                    let elem = entity.element_data_mut();
                    if deviated && (dx_step != 0.0 || dy_step != 0.0) {
                        let raw = vector_to_sector_0_to_15(dx_step, dy_step);
                        elem.set_direction_goal(if order_reverse { raw ^ 8 } else { raw });
                    }
                    let mut position = elem.position_map();
                    position.x += dx_step;
                    position.y += dy_step;
                    elem.set_position_map(position);
                    if deviated && (dx_step != 0.0 || dy_step != 0.0) {
                        elem.sprite.position_iface.reset_increment_computed();
                        elem.sprite.position_iface.compute_increment_all(false);
                    } else if recovered_from_deviation {
                        // Original rebuilds the trajectory even when this
                        // animation frame contributes no movement.
                        elem.sprite.position_iface.reset_increment_computed();
                        elem.sprite.position_iface.compute_increment_all(true);
                    }
                    elem.update_grid_cell();
                }
                // TILL_LAST_FRAME still performs the ordinary arrival check
                // after every nonzero transition step. Reaching the target
                // zeros both increments and snaps an undeviated zero-tolerance
                // actor, but the transition keeps playing until its animation
                // loops unless the next order uses the same animation.
                let transition_goal_reached = entity
                    .position_iface()
                    .is_goal_reached(&self.world.fast_grid, None);
                let transition_increment_nonzero = {
                    let increment = entity.position_iface().get_increment_map();
                    increment.x != 0.0 || increment.y != 0.0
                };
                if transition_goal_reached && speed != 0.0 && transition_increment_nonzero {
                    let should_snap =
                        !entity.position_iface().is_deviated() && order_tolerance == 0.0;
                    entity.position_iface_mut().zero_all_increments();
                    if should_snap {
                        entity.element_data_mut().set_position_map(goal);
                    }
                    if next_destination_same_action.is_some() {
                        motion_state = MotionState::Terminated;
                    }
                }
                // Actor::Hourglass runs CheckForLineCrossing after Execute
                // returns, so use the final post-arrival position here. A
                // TillLastFrame step may overshoot and snap back to its goal;
                // the discarded overshoot must not trigger a boundary.
                if let Some((old_pos, layer, eligible)) = transition_crossing_start
                    && eligible
                {
                    let new_pos = entity.element_data().position_map();
                    if self.world.fast_grid.level.map_bbox.contains_point(new_pos) {
                        line_cross_checks.push((entity_id, old_pos, new_pos, layer));
                        if is_pc {
                            patch_cross_checks.push((entity_id, old_pos, new_pos, layer));
                        }
                        script_cross_checks.push((entity_id, old_pos, new_pos, layer));
                        sound_cross_checks.push((entity_id, old_pos, new_pos, layer));
                    }
                }
                let transition_effect_motion = movement_execute_visible_motion(
                    order_action,
                    motion_state,
                    false,
                    entity_target_seek,
                );
                if let Some((posture, next_action_state)) =
                    movement_execute_state_effect(order_action, transition_effect_motion)
                {
                    // A speed-transition completion establishes the live
                    // walking/running state itself. Do not let an older state
                    // saved by a preceding door transition overwrite it when
                    // the generated continuation order executes next tick.
                    if next_action_state.is_moving()
                        && let Some(pass) = entity
                            .actor_data_mut()
                            .and_then(|actor| actor.active_door_pass.as_mut())
                    {
                        pass.saved_action_state = None;
                    }
                    movement_state_effects.push((entity_id, posture, next_action_state));
                }
                let door_transition_state_effect_due =
                    matches!(motion_state, MotionState::Terminated)
                        || matches!(motion_state, MotionState::Done)
                            && matches!(
                                anim,
                                OrderType::TransitionClimbingLadderDownWaitingUpright
                                    | OrderType::TransitionClimbingLadderDownWaitingUprightAlerted
                            );
                if door_pass_anim.is_some()
                    && door_transition_state_effect_due
                    && matches!(
                        anim,
                        OrderType::TransitionWaitingUprightClimbingWallUp
                            | OrderType::TransitionWaitingCrouchedClimbingLadderDown
                            | OrderType::TransitionClimbingLadderDownWaitingUpright
                            | OrderType::TransitionClimbingLadderDownWaitingUprightAlerted
                            | OrderType::TransitionClimbingWallUpWaitingCrouched
                            | OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel
                            | OrderType::TransitionWaitingCrouchedClimbingWallDown
                            | OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel
                            | OrderType::TransitionClimbingWallDownWaitingUpright
                    )
                {
                    door_pass_transition_completion_effects.push(entity_id);
                }
                if matches!(motion_state, MotionState::Terminated) {
                    // TillLastFrame can exhaust its animation before its
                    // distance target is reached (notably the short
                    // Waiting→Walking startup transition). The Original does
                    // not discard that remaining distance: it copies the
                    // current order at the first following animation change,
                    // changes the copy to that next animation, then retires
                    // the exhausted transition. This keeps the copied order's
                    // old target as a one-tick continuation.
                    if !transition_goal_reached {
                        // Original's movement element already contains the
                        // whole PassDoor route. Rust keeps the untranslated
                        // tail on ActiveDoorPass, so the next distinct
                        // destination animation may live there rather than in
                        // `element.orders`.
                        let lazy_next_animation = entity
                            .actor_data()
                            .and_then(|actor| actor.active_door_pass.as_ref())
                            .and_then(|pass| {
                                pass.steps.iter().find_map(|step| match step {
                                    crate::element::DoorPassStep::Walk {
                                        destination,
                                        action,
                                        ..
                                    } if *destination != MapPoint::ZERO
                                        && *action != order_action =>
                                    {
                                        Some(*action)
                                    }
                                    _ => None,
                                })
                            });
                        let next_order_id = &mut self.orders.next_order_id;
                        let mut continuation_door_action = None;
                        if let Some(element) = self
                            .orders
                            .sequence_manager
                            .get_element_mut(move_seq_id, move_elem_idx)
                        {
                            let current_action = element
                                .orders
                                .front()
                                .expect("terminated movement transition lost its current order")
                                .order_type;
                            let next_animation = element
                                .orders
                                .iter()
                                .enumerate()
                                .skip(1)
                                .find(|(_, order)| {
                                    order.order_type != current_action
                                        && (order.target_x != 0.0 || order.target_y != 0.0)
                                })
                                .map(|(index, order)| (index, order.order_type));
                            let next_animation = next_animation.or_else(|| {
                                lazy_next_animation
                                    .map(|animation| (element.orders.len(), animation))
                            });
                            if let Some((insertion, animation)) = next_animation {
                                let mut continuation = element.orders.front().unwrap().clone();
                                continuation.order_type = animation;
                                // PerformMotion(TILL_LAST_FRAME) can exhaust
                                // the transition animation before reaching its
                                // distance target. Original inserts this
                                // changed-animation copy from inside that same
                                // PerformMotion call. Defer the copy's START
                                // state effect until we know whether the copy
                                // actually survives its first Execute.
                                continuation.transition_distance_continuation = true;
                                continuation.reseed_id(crate::order::alloc_order_id(next_order_id));
                                continuation_door_action = Some((animation, continuation.reverse));
                                element.insert_order(insertion, continuation);
                                if is_pc
                                    && let Some(authored_successor) =
                                        element.orders.get_mut(insertion + 1)
                                {
                                    assert_eq!(
                                        authored_successor.order_type, animation,
                                        "transition-distance continuation must precede the authored order whose animation it continues"
                                    );
                                    authored_successor.deferred_movement_state_start = true;
                                }
                            } else {
                                element.orders.truncate(1);
                            }
                        }
                        // Original stores the complete translated door route
                        // in the movement element, so changing to this copied
                        // successor changes the one authoritative current
                        // action. Rust keeps the untranslated route tail in a
                        // parallel ActiveDoorPass. Keep its animation mirror
                        // in lockstep with the concrete continuation order:
                        // lift handling and the next Execute slot both consult
                        // it before dispatching sprite motion.
                        if let Some((animation, reverse)) = continuation_door_action
                            && let Some(pass) = entity
                                .actor_data_mut()
                                .and_then(|actor| actor.active_door_pass.as_mut())
                        {
                            pass.current_action = animation;
                            pass.current_reverse = reverse;
                        }
                    }
                    let eid = entity_id;
                    if is_final_waypoint
                        && is_sword_motion
                        && let Some(human) = entity.human_data_mut()
                    {
                        human.last_motion_was_step_back_in_combat = active_move_flags
                            .contains(crate::sequence::MoveFlags::STEP_BACK_IN_COMBAT);
                    }
                    let actor = entity.actor_data_mut().expect("actor-only branch");
                    // Pop via the element we actually dispatched (`move_seq_id` /
                    // `move_elem_idx`), not `actor.active_movement.sequence_id`
                    // — the latter can be stale/None when the Move element was
                    // launched by the AI without setting active_movement
                    // (soldier chase paths).
                    order_pops.push((move_seq_id, move_elem_idx));
                    // Last order of the Move element just completed — flip
                    // back to Waiting and clear the active movement.
                    // Matches the `DoorPassAdvance::Done` arm below but for
                    // the transition-terminated path.
                    if is_final_waypoint {
                        let mut clear_completed_movement_goal = false;
                        let advance = if actor.active_door_pass.is_some() {
                            Self::advance_door_pass(
                                actor,
                                eid,
                                goal,
                                &mut door_triggers,
                                &mut select_triggers,
                                &mut self.orders.next_order_id,
                            )
                        } else {
                            DoorPassAdvance::Done { completed: None }
                        };
                        match advance {
                            DoorPassAdvance::Continue {
                                destination,
                                action,
                                reverse,
                                compute_direction,
                                tolerance,
                            } => {
                                let order_id =
                                    crate::order::alloc_order_id(&mut self.orders.next_order_id);
                                let mut order = crate::order::Order::new(
                                    action,
                                    destination.x,
                                    destination.y,
                                    order_id,
                                );
                                order.reverse = reverse;
                                order.compute_direction = compute_direction;
                                order.tolerance = tolerance;
                                transition_pushes.push((move_seq_id, move_elem_idx, order));
                            }
                            DoorPassAdvance::Paused { transition_order } => {
                                transition_pushes.push((
                                    move_seq_id,
                                    move_elem_idx,
                                    transition_order,
                                ));
                            }
                            DoorPassAdvance::ActionPoint { order } => {
                                transition_pushes.push((move_seq_id, move_elem_idx, order));
                            }
                            DoorPassAdvance::Done { completed } => {
                                if let Some((door_index, direct)) = completed {
                                    completed_door_passes.push((eid, door_index, direct));
                                }
                                actor.clear_path();
                                actor.action_state =
                                    if is_swordfighting || actor.action_state.is_sword() {
                                        crate::element::ActionState::WaitingSword
                                    } else {
                                        crate::element::ActionState::Waiting
                                    };
                                actor.active_movement.clear();
                                actor.active_door_pass = None;
                                clear_completed_movement_goal = true;
                            }
                            DoorPassAdvance::NoActive => {
                                tracing::warn!(
                                    entity = ?eid,
                                    "DoorPass: transition-terminated movement lost active pass"
                                );
                            }
                        }
                        if clear_completed_movement_goal {
                            // Actor::SendCondolationCard runs synchronously
                            // when this retained stop transition completes and
                            // clears the selected movement's sprite goal before
                            // NPC callbacks can select replacement work.
                            entity
                                .position_iface_mut()
                                .set_map_goal(crate::coordinates::MapPoint::ZERO);
                        }
                    }
                }
                continue;
            }

            // Zero-distance animation ticks are still real PerformSeek /
            // PerformMotion calls. The pre-motion tolerance branch and an
            // ordinary order whose destination already equals the actor's
            // position both complete without sprite displacement. In
            // particular, a freshly initialized exact-position walk returns
            // TERMINATED in Original on that first Execute. Only defer a
            // genuinely stationary motion that has not reached its goal.
            if stationary_motion_waits(speed, tolerance_arrival, dist) {
                if let Some((posture, next_action_state)) =
                    movement_execute_state_effect(order_action, state_effect_motion)
                {
                    movement_state_effects.push((entity_id, posture, next_action_state));
                }
                continue;
            }

            tracing::trace!(
                "tick_move: entity={:?} pos=({:.0},{:.0}) goal=({:.0},{:.0}) speed={speed:.1} action={:?} state={:?}",
                entity_id,
                elem.position_map().x,
                elem.position_map().y,
                goal.x,
                goal.y,
                anim,
                action_state,
            );

            // Snapshot the pre-move position + layer + posture so we
            // can run the elevation-line-crossing check after the
            // position is updated. Original excludes flying actors and
            // humans with a live carrier; wall/ladder climbers and actors
            // carrying somebody else remain eligible.
            let old_pos = elem.position_map();
            let entity_layer = elem.layer();
            let entity_posture = elem.posture;
            let eligible_for_crossing = actor_line_crossing_eligible(
                entity_posture,
                human_is_carried,
                self.world.fast_grid.level.map_bbox.contains_point(old_pos),
            );

            // Seek-arrival predicate:
            //
            //   - dist_sq = squared distance (target - pos), with Y
            //     stretched by the inverse aspect ratio (≈1.7434)
            //     when DIRECTIONAL_TOLERANCE is set (used for net
            //     pickup).
            //   - Arrive iff target.sector == self.sector AND
            //     dist_sq < tolerance² × 1.1025 (the "5% tolerance"
            //     margin baked into the squared comparison).
            //
            // The check runs every tick (not just the last waypoint),
            // so a moving target that wanders into range mid-route
            // ends the seek immediately and the post-seek sequence
            // fires.  The pre-pass only populates `FinalTol` for
            // SEEK-flagged movements with a resolvable target (entity
            // or shield destination), so `ft.tol > 0` is the
            // live-seek gate; non-seek elements skip this branch
            // entirely and fall through to the standard `dist <=
            // speed` arrival.  USE_POINT samples the target's current
            // hotspot; SEEK_SHIELD uses the movement element
            // destination.
            let ft = final_tolerances[actor_id];
            let mut point_seek_post_arrival = is_final_waypoint
                && dist <= speed
                && point_seek_post_sectors[actor_id]
                    .map(|seek_sector| elem.sector() == Some(seek_sector))
                    .unwrap_or(false);
            // FROZEN stand-still wait.  When the seek arrival
            // predicate fires at an intermediate waypoint and there
            // is no `post_seek_sequence` to consume the arrival, the
            // actor freezes its sprite frame in place near the target
            // until either the target moves out of tolerance
            // (next-tick `tick_refresh_seeks` detects drift and
            // rebuilds the path) or a post-seek is later attached.
            // We honour this by simply skipping the per-tick movement
            // step (no order pop, no position update, no sprite
            // advance) so the actor's position + orders persist for
            // the next tick to re-evaluate.
            //
            // This branch only fires for entity-target seeks without
            // a queued post-seek interaction (e.g. AI follow seeks
            // built outside `apply_interaction_with_seek`).  The
            // common PC interaction path always carries a post-seek
            // and routes through the `start_post_seek` branch below
            // instead.
            let frozen_seek_wait = tolerance_arrival && !is_final_waypoint && !ft.has_post_seek;
            if frozen_seek_wait {
                tracing::trace!(
                    entity = ?entity_id,
                    "tick_move: FROZEN seek wait (target in range, no post-seek, mid-path)",
                );
                continue;
            }

            // Original entity-target `PerformSeek` samples its live-target
            // tolerance before `PerformMotion`. If already in range it takes
            // the frozen/post-seek arm without committing a movement step.
            // If this frame's step merely crosses into range, that new
            // distance is not sampled until the next actor Hourglass.
            //
            // Ordinary waypoint arrival is different: PerformMotion commits
            // through UpdatePositionAntiCollision and then asks
            // IsGoalReached. Re-enter the shared tail after that commit while
            // retaining (not recomputing) the pre-motion seek predicate.
            let mut post_step_arrival = dist <= f32::EPSILON || tolerance_arrival;
            'arrival: loop {
                if post_step_arrival {
                    // Original PerformMotion/PerformSeek returns TERMINATED
                    // after committing the step which reaches the goal. Rust
                    // stages geometry after the sprite call, so its raw
                    // motion state can still be DONE here. Queue the Human
                    // Execute termination callback at the authoritative
                    // arrival boundary; it owns the range-based Provoke
                    // launched after sword movement.
                    if is_sword_motion {
                        sword_movement_terminations.push(entity_id);
                    }
                    // Reached waypoint — snap to it and advance
                    if !tolerance_arrival
                        && order_tolerance == 0.0
                        && !entity.position_iface().is_deviated()
                    {
                        entity
                            .element_data_mut()
                            .set_position_map(crate::coordinates::MapPoint {
                                x: goal.x,
                                y: goal.y,
                            });
                    }
                    let eid = entity_id;

                    // Transition-animation refresh.  When this arrival
                    // pop will leave a transition-to-waiting order as the
                    // next (final) order in the queue AND the seek target
                    // has drifted since the last refresh AND the new
                    // distance is greater than `(transition_distance +
                    // seek_distance) * 1.05`, abort the queued transition
                    // and call RefreshSeek immediately so the actor
                    // doesn't play a transition that lands too far from
                    // the (moved) target.  Computed here using the
                    // still-live `elem` borrow (sprite + position) before
                    // the actor mut borrow takes over — NLL would
                    // otherwise reject the second borrow.
                    let transition_refresh_target: Option<(
                        crate::sequence::SequenceId,
                        usize,
                        OrderType,
                        MapPoint,
                        f32,
                    )> = if !is_final_waypoint
                        && !tolerance_arrival
                        && ft.tol > 0.0
                        && let Some(target_now) = live_seek_target.map(|(position, _, _)| position)
                    {
                        let last = ft.last_seek_target_position;
                        let drifted = (target_now.x - last.x).abs() > 0.01
                            || (target_now.y - last.y).abs() > 0.01;
                        if drifted {
                            let seq_id = move_seq_id;
                            let elem_idx = move_elem_idx;
                            let next_anim = self
                                .orders
                                .sequence_manager
                                .get_element(seq_id, elem_idx)
                                .and_then(|e| e.orders.get(1).map(|o| o.order_type));
                            if matches!(
                                next_anim,
                                Some(OrderType::TransitionRunningUprightWaitingUpright)
                                    | Some(OrderType::TransitionWalkingUprightWaitingUpright)
                                    | Some(OrderType::TransitionWalkingCrouchedWaitingCrouched)
                            ) {
                                let next_anim = next_anim.unwrap();
                                let pos = entity.element_data().position_map();
                                let dx = target_now.x - pos.x;
                                let dy = target_now.y - pos.y;
                                let dy_eff = if ft.directional {
                                    const INVERSE_ASPECT_RATIO: f32 = 1.743_446_8;
                                    dy * INVERSE_ASPECT_RATIO
                                } else {
                                    dy
                                };
                                let sq = dx * dx + dy_eff * dy_eff;
                                let trans_dist = if entity.sprite().has_animation(next_anim) {
                                    entity.sprite().distance_for_animation(next_anim) as f32
                                } else {
                                    0.0
                                };
                                let raw = trans_dist + ft.tol;
                                let threshold = raw * 1.05;
                                if sq > threshold * threshold {
                                    Some((seq_id, elem_idx, next_anim, target_now, sq.sqrt()))
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    if let Some((seq_id, elem_idx, _next_anim, _target, _dist)) =
                        transition_refresh_target
                    {
                        transition_seek_refreshes.push((eid, seq_id, elem_idx));
                        tracing::trace!(
                            ?eid,
                            ?_next_anim,
                            target_x = _target.x,
                            target_y = _target.y,
                            new_dist = _dist,
                            "tick_move: transition-animation refresh fired (target drifted beyond transition+seek_dist)",
                        );
                        continue 'actors;
                    }

                    // A final concrete waypoint is only the position at
                    // which the target was observed when this Seek was
                    // built.  When the walking order terminates, Original
                    // PerformSeek validates that stale waypoint against the
                    // live target before it may hand off to the post-seek
                    // action:
                    //
                    //   same sector
                    //   && (target has not moved || live target is in range)
                    //
                    // If that check fails, RefreshSeek replaces the movement
                    // immediately and the exhausted old order must not reach
                    // generic `do_next_order` (which would launch the Hit /
                    // interaction tail unconditionally).
                    let movement_is_last_sequence_element = self
                        .orders
                        .sequence_manager
                        .get_sequence(move_seq_id)
                        .map(|sequence| move_elem_idx + 1 >= sequence.elements.len())
                        .unwrap_or(false);
                    let final_entity_seek_arrival = if is_final_waypoint
                        && movement_is_last_sequence_element
                        && ft.target_id.is_some()
                    {
                        live_seek_target.map(|(target_position, target_sector, _)| {
                            let same_sector = target_sector.is_some()
                                && target_sector == entity.element_data().sector();
                            let target_unchanged = target_position == ft.last_seek_target_position;
                            same_sector && (target_unchanged || tolerance_arrival)
                        })
                    } else {
                        None
                    };
                    if final_entity_seek_arrival == Some(false) {
                        transition_seek_refreshes.push((eid, move_seq_id, move_elem_idx));
                        tracing::trace!(
                            ?eid,
                            "tick_move: final seek waypoint is stale; refreshing against live target",
                        );
                        continue 'actors;
                    }

                    let actor = entity.actor_data_mut().unwrap();
                    // The post-seek sequence fires whenever the seek
                    // arrival predicate is true and a post-seek sequence
                    // is attached — no final-waypoint gate.  The
                    // `tolerance_arrival` guard above already enforces the
                    // post-seek requirement for intermediate waypoints, so
                    // reaching this point with both flags set is the
                    // "terminate the seek and launch the post-seek" path.
                    let start_post_seek = (tolerance_arrival
                        || point_seek_post_arrival
                        || final_entity_seek_arrival == Some(true))
                        && actor.post_seek_sequence.is_some();
                    let start_post_seek = if start_post_seek && actor.active_door_pass.is_some() {
                        tracing::warn!(
                            entity = ?eid,
                            "DoorPass: suppressing post-seek teardown during active pass"
                        );
                        false
                    } else {
                        start_post_seek
                    };

                    // Waypoint reached — queue a `do_next_order` pop on
                    // the actor's Move element.
                    if start_post_seek {
                        post_seek_arrivals.push((eid, move_seq_id, move_elem_idx));
                    } else {
                        order_pops.push((move_seq_id, move_elem_idx));
                    }

                    if start_post_seek {
                        actor.clear_path();
                        // Original StartPostSeekSequence terminates the seek
                        // and launches the interaction without rewriting the
                        // actor state. The interaction's generated transition
                        // owns any later Moving→Waiting change.
                        actor.active_movement.clear();
                        actor.active_door_pass = None;
                        if is_sword_motion && let Some(human) = entity.human_data_mut() {
                            human.last_motion_was_step_back_in_combat = active_move_flags
                                .contains(crate::sequence::MoveFlags::STEP_BACK_IN_COMBAT);
                        }
                        continue 'actors;
                    }

                    // With no post-seek tail, the successful final
                    // entity-target arrival remains inside PerformSeek.  It
                    // arms an immediate refresh check and returns InProgress
                    // instead of consuming the final order.
                    if final_entity_seek_arrival == Some(true) {
                        actor.seek_refresh_wait = 0;
                        continue 'actors;
                    }

                    if is_final_waypoint {
                        // All waypoints for current walk step consumed.
                        // Check if we have more door-pass steps.
                        let advance = if actor.active_door_pass.is_some() {
                            Self::advance_door_pass(
                                actor,
                                eid,
                                goal,
                                &mut door_triggers,
                                &mut select_triggers,
                                &mut self.orders.next_order_id,
                            )
                        } else {
                            DoorPassAdvance::Done { completed: None }
                        };

                        match advance {
                            DoorPassAdvance::Continue {
                                destination,
                                action,
                                reverse,
                                compute_direction,
                                tolerance,
                            } => {
                                // Push a walking order for the new Walk
                                // step onto the actor's current sequence
                                // element, to be installed after the
                                // entity loop closes (same deferred
                                // mechanism as Transition steps).
                                let order_id =
                                    crate::order::alloc_order_id(&mut self.orders.next_order_id);
                                let mut order = crate::order::Order::new(
                                    action,
                                    destination.x,
                                    destination.y,
                                    order_id,
                                );
                                order.reverse = reverse;
                                order.compute_direction = compute_direction;
                                order.tolerance = tolerance;
                                transition_pushes.push((move_seq_id, move_elem_idx, order));
                            }
                            DoorPassAdvance::Paused { transition_order } => {
                                // Transition animation queued — push the
                                // order onto the actor's current sequence
                                // element after the loop closes.
                                transition_pushes.push((
                                    move_seq_id,
                                    move_elem_idx,
                                    transition_order,
                                ));
                            }
                            DoorPassAdvance::ActionPoint { order } => {
                                transition_pushes.push((move_seq_id, move_elem_idx, order));
                            }
                            DoorPassAdvance::Done { completed } => {
                                if let Some((door_index, direct)) = completed {
                                    completed_door_passes.push((eid, door_index, direct));
                                }
                                // Final waypoint's do_next_order pop was
                                // already collected above when
                                // `path_waypoint_index` advanced past the
                                // end of the list; that pop will either
                                // drain the Move element entirely
                                // (triggering `element_terminated` +
                                // `ensure_wait_element` internally) or
                                // leave an end-transition order as the
                                // new current, which the animation driver
                                // will play next tick.
                                actor.clear_path();
                                // Keep the movement action state until an
                                // optional end transition actually finishes.
                                // RHElementActor's walking Execute arm leaves
                                // MOVING unchanged on RHMOTION_TERMINATED; the
                                // transition-to-waiting arm performs the state
                                // change itself.  With NO_TRANSITIONS there is
                                // deliberately no waiting-state rewrite here.
                                actor.active_movement.clear();
                                actor.active_door_pass = None;
                                if is_sword_motion && let Some(human) = entity.human_data_mut() {
                                    human.last_motion_was_step_back_in_combat = active_move_flags
                                        .contains(crate::sequence::MoveFlags::STEP_BACK_IN_COMBAT);
                                }
                            }
                            DoorPassAdvance::NoActive => {
                                tracing::warn!(
                                    entity = ?eid,
                                    "DoorPass: final waypoint reached but active pass was already gone"
                                );
                            }
                        }
                    }
                    break 'arrival;
                } else {
                    // Move toward waypoint.
                    //
                    // Actor-vs-actor anti-collision: deviate around other
                    // actors' repulsive zones before committing the step.
                    // Runs between the motion advance and the position
                    // commit, gated on the mover's `anti_collision_on`
                    // flag — the flag stays `true` by default so this is
                    // active for every normal walk.
                    // `PerformMotion` advances with PositionInterface's
                    // cached `GetIncrementMap()`. It does not renormalize the
                    // remaining goal vector each frame; that cache is rebuilt
                    // only when a new order starts or anti-collision changes
                    // deviation state. Recomputing here introduced tiny drift
                    // into patrol-chief history and eventually flipped exact
                    // transition-arrival dot products.
                    let cached_increment = entity.position_iface().get_increment_map();
                    let nx = cached_increment.x;
                    let ny = cached_increment.y;
                    let anti_on = entity.position_iface().is_anti_collision_on();
                    // Pull transient anti-collision context from position_iface
                    // (move box, half-diagonal) + the current path goal.  The
                    // persistent state (deviated / blocked_count / box_blocked /
                    // radius) lives on the actor's PI directly now.
                    let (
                        dx_step,
                        dy_step,
                        deviated,
                        recovered_from_deviation,
                        rebuild_after_deviation,
                    ) = if anti_on
                        && let Some(mover_snap) =
                            anti_snapshots.get(actor_id).and_then(|slot| slot.as_ref())
                    {
                        let goal_map = crate::coordinates::MapPoint::new(goal.x, goal.y);
                        let (move_box, half_diagonal) = {
                            let pi = entity.position_iface();
                            (*pi.get_move_box(), pi.get_half_diagonal())
                        };
                        let pi = entity.position_iface_mut();
                        let was_deviated = pi.is_deviated();
                        let mut state = super::anti_collision::AntiCollisionState {
                            pi,
                            move_box,
                            half_diagonal,
                            goal_map,
                        };
                        let (dx_step, dy_step) = super::anti_collision::apply_anti_collision_step(
                            mover_snap,
                            anti_snapshots.as_slice(),
                            &self.ai.global.repulsive_points,
                            prepared
                                .mobile_points_by_layer
                                .get(&mover_snap.layer)
                                .map(Vec::as_slice)
                                .unwrap_or(&[]),
                            prepared
                                .mobile_lines_by_layer
                                .get(&mover_snap.layer)
                                .map(Vec::as_slice)
                                .unwrap_or(&[]),
                            prepared
                                .mobile_polygons_by_layer
                                .get(&mover_snap.layer)
                                .map(Vec::as_slice)
                                .unwrap_or(&[]),
                            Some(&self.world.fast_grid),
                            Some(&mut state),
                            nx,
                            ny,
                            speed,
                            anti_on,
                        );
                        (
                            dx_step,
                            dy_step,
                            state.pi.is_deviated(),
                            was_deviated && !state.pi.is_deviated(),
                            // A successfully committed deviation expands the
                            // blocked box, resets the counter, and Original
                            // rebuilds the cached increment. Its
                            // blocked-count break-through path instead uses
                            // MoveMap and deliberately retains the old cache.
                            state.pi.is_deviated() && state.pi.blocked_count == 0,
                        )
                    } else {
                        (nx * speed, ny * speed, false, false, false)
                    };
                    let new_pos_x;
                    let new_pos_y;
                    {
                        let elem = entity.element_data_mut();
                        if deviated && (dx_step != 0.0 || dy_step != 0.0) {
                            // `UpdatePositionAntiCollision` faces along the
                            // committed deviation, then invalidates and
                            // reconstructs the cached increment from the new
                            // position to the original goal.  The direction is
                            // deliberately retained by ComputeIncrementAll(false).
                            let raw = vector_to_sector_0_to_15(dx_step, dy_step);
                            elem.set_direction_goal(if order_reverse { raw ^ 8 } else { raw });
                        }
                        let mut pm = elem.position_map();
                        pm.x += dx_step;
                        pm.y += dy_step;
                        elem.set_position_map(pm);
                        if rebuild_after_deviation && (dx_step != 0.0 || dy_step != 0.0) {
                            elem.sprite.position_iface.reset_increment_computed();
                            elem.sprite.position_iface.compute_increment_all(false);
                        } else if recovered_from_deviation {
                            // Original's no-new-deviation recovery branch commits
                            // the (possibly zero-length) step, clears
                            // `IsDeviated`, and rebuilds the increment with
                            // direction computation enabled.
                            elem.sprite.position_iface.reset_increment_computed();
                            elem.sprite.position_iface.compute_increment_all(true);
                        }
                        new_pos_x = pm.x;
                        new_pos_y = pm.y;
                    }

                    // Water splash titbit emission.  Every walk tick
                    // where `speed > 2` and the actor's cached material
                    // is water, the sprite's splatter counter ticks up;
                    // on `>= 2` a water particle is added at the actor's
                    // 3D position and the counter resets.  Cosmetic but
                    // observable — actors crossing a stream kick up
                    // splash titbits.
                    {
                        let elem = entity.element_data_mut();
                        if speed > 2.0 && elem.material() == crate::element::GameMaterial::Water {
                            if elem.sprite.splitch_count >= 2 {
                                elem.sprite.splitch_count = 0;
                                let pos = elem.position();
                                let layer = elem.layer();
                                water_splash_emits.push((
                                    entity_id,
                                    crate::coordinates::WorldPoint3D {
                                        x: pos.x,
                                        y: pos.y,
                                        z: pos.z,
                                    },
                                    layer,
                                ));
                            } else {
                                elem.sprite.splitch_count =
                                    elem.sprite.splitch_count.saturating_add(1);
                            }
                        }
                    }

                    // When the blocked counter trips, the motion aborts
                    // and the backing sequence element is marked
                    // Impossible.
                    let movement_aborted = entity.position_iface().is_blocked();
                    if movement_aborted {
                        let actor = entity.actor_data_mut().expect("actor-only branch");
                        if let Some(seq_id) = actor.active_movement.sequence_id {
                            blocked_impossible.push((seq_id, actor.active_movement.element_index));
                        }
                        let restore_anti_collision = {
                            let restore_anti_collision = actor.active_door_pass.is_some();
                            if restore_anti_collision {
                                tracing::warn!(
                                    entity = ?entity_id,
                                    "DoorPass: movement blocked; clearing active pass with aborted movement"
                                );
                                actor.active_door_pass = None;
                            }
                            actor.clear_path();
                            actor.action_state =
                                if is_swordfighting || actor.action_state.is_sword() {
                                    crate::element::ActionState::WaitingSword
                                } else {
                                    crate::element::ActionState::Waiting
                                };
                            actor.active_movement.clear();
                            restore_anti_collision
                        };
                        if restore_anti_collision {
                            entity.position_iface_mut().set_anti_collision_on(true);
                        }
                        entity.position_iface_mut().reset_box_blocked();
                    }

                    // Sync the just-moved position back into the snapshot
                    // so later actors in this tick see the serial
                    // "already-moved" position of this one.  Without this
                    // two actors heading for the same cell both see each
                    // other at the *old* position and can still overlap.
                    if let Some(snap) = anti_snapshots
                        .get_mut(actor_id)
                        .and_then(|slot| slot.as_mut())
                    {
                        let new_pos = MapPoint::new(new_pos_x, new_pos_y);
                        super::anti_collision::sync_snapshot_after_move(
                            snap,
                            new_pos,
                            MapVec::new(dx_step, dy_step),
                        );
                    }

                    if movement_aborted {
                        break 'arrival;
                    }

                    // `UpdatePositionAntiCollision` has now committed the
                    // ordinary frame and rebuilt the increment when deviation
                    // changed.  This is the exact point where Original calls
                    // `RHPositionInterface::IsGoalReached`.
                    let movement_goal_reached = entity
                        .position_iface()
                        .is_goal_reached(&self.world.fast_grid, None);
                    point_seek_post_arrival = is_final_waypoint
                        && movement_goal_reached
                        && point_seek_post_sectors[actor_id]
                            .map(|seek_sector| entity.element_data().sector() == Some(seek_sector))
                            .unwrap_or(false);
                    post_step_arrival = movement_goal_reached || tolerance_arrival;
                    if post_step_arrival {
                        continue 'arrival;
                    }
                    break 'arrival;
                }
            }

            // Queue an elevation-line-cross check for this tick. The
            // actual fast-grid query + obstacle swap runs after the
            // loop, since `check_for_line_crossing` needs `&mut self`.
            //
            // Also queue a patch-line-cross check for PC actors —
            // LINE_PATCH handling is gated to PCs only.
            let new_pos = entity.element_data().position_map();
            let new_position_in_bounds =
                self.world.fast_grid.level.map_bbox.contains_point(new_pos);
            tracing::trace!(
                target: "robin_engine::elevation_crossing",
                ?entity_id,
                eligible_for_crossing,
                new_position_in_bounds,
                posture = ?entity_posture,
                human_is_carried,
                layer = entity_layer,
                old_x = old_pos.x,
                old_y = old_pos.y,
                new_x = new_pos.x,
                new_y = new_pos.y,
                "considered queuing elevation crossing"
            );
            if eligible_for_crossing {
                if new_position_in_bounds {
                    line_cross_checks.push((entity_id, old_pos, new_pos, entity_layer));
                    if entity.is_pc() {
                        patch_cross_checks.push((entity_id, old_pos, new_pos, entity_layer));
                    }
                    script_cross_checks.push((entity_id, old_pos, new_pos, entity_layer));
                    // LINE_SOUND crossing is not gated on PC — every
                    // moving actor refreshes its `material` on
                    // crossing a sound-material boundary so footstep
                    // sound playback picks the right per-frame
                    // material.
                    sound_cross_checks.push((entity_id, old_pos, new_pos, entity_layer));
                }
            }
            // Order pops are drained after all actors so the current order is
            // still physically at the front here. Treat an already-queued
            // pop as a completed Execute when deciding whether a deferred
            // START survives this actor slot.
            let current_order_will_advance = order_pops
                .iter()
                .any(|&(seq_id, elem_idx)| seq_id == move_seq_id && elem_idx == move_elem_idx);
            // Ordinary walking START effects have the same survival rule as
            // generated transition-distance and deferred PC successors.
            // Original PerformMotion moves first and only then returns its
            // final motion state to Execute; when anti-collision deviation
            // lands inside the goal predicate on that first call, Execute
            // observes TERMINATED and must not briefly enter Moving.
            if matches!(state_effect_motion, MotionState::Start)
                && !deferred_movement_state_start_due
                && !transition_distance_first_execute_due
                && !current_order_will_advance
                && self
                    .orders
                    .sequence_manager
                    .get_element(move_seq_id, move_elem_idx)
                    .and_then(|element| element.orders.front())
                    .is_some_and(|order| Some(order.order_id) == order_id)
                && let Some((posture, next_action_state)) =
                    movement_execute_state_effect(order_action, MotionState::Start)
            {
                movement_state_effects.push((entity_id, posture, next_action_state));
            }
            // The authored successor owns the deferred movement START only
            // if it remains current after this Execute. A very short
            // successor can complete and hand off to its stop transition in
            // the same call; Original retains Waiting in that case.
            if deferred_movement_state_start_due
                && !current_order_will_advance
                && self
                    .orders
                    .sequence_manager
                    .get_element(move_seq_id, move_elem_idx)
                    .and_then(|element| element.orders.front())
                    .is_some_and(|order| Some(order.order_id) == order_id)
                && let Some((posture, next_action_state)) =
                    movement_execute_state_effect(order_action, MotionState::Start)
            {
                if is_sword_motion {
                    sword_movement_starts.push(entity_id);
                }
                movement_state_effects.push((entity_id, posture, next_action_state));
            }
            // A generated transition-distance copy reports START when first
            // booked, but its movement state is authoritative only if that
            // copied order remains current after the Execute. A short copy
            // may satisfy its arrival predicate and hand off in the same
            // call; Original retains the transition's Waiting state for that
            // frame. This survival rule applies to PCs too; their separate
            // deferred-successor marker covers the later authored order.
            if transition_distance_first_execute_due
                && !current_order_will_advance
                && self
                    .orders
                    .sequence_manager
                    .get_element(move_seq_id, move_elem_idx)
                    .and_then(|element| element.orders.front())
                    .is_some_and(|order| Some(order.order_id) == order_id)
                && let Some((posture, next_action_state)) =
                    movement_execute_state_effect(order_action, MotionState::Start)
            {
                if is_sword_motion {
                    sword_movement_starts.push(entity_id);
                }
                movement_state_effects.push((entity_id, posture, next_action_state));
            }
        }

        for entity_id in sword_movement_starts {
            self.apply_sword_movement_start_initiative_transfer(entity_id);
        }
        for (entity_id, posture, action_state) in movement_state_effects {
            if let Some(entity) = self.get_entity_mut(entity_id) {
                entity.set_posture(posture);
                if let Some(actor) = entity.actor_data_mut() {
                    actor.action_state = action_state;
                }
            }
        }
        for entity_id in door_pass_transition_start_effects {
            self.apply_door_pass_transition_start_side_effects(assets, entity_id);
        }
        for entity_id in door_pass_transition_done_effects {
            self.apply_door_pass_transition_done_side_effects(assets, entity_id);
        }
        for entity_id in door_pass_transition_completion_effects {
            self.apply_door_pass_transition_completion_side_effects(assets, entity_id);
        }
        // Dispatch elevation-line crossings detected during the loop.
        // Runs as a post-pass after the per-actor movement update.
        // When a human actor crosses an elevation line, we also fire
        // `UpdateRoll` so any in-progress Rolling combat_anim can
        // re-aim its flight at the new obstacle's slope.
        for (entity_id, old_pos, new_pos, layer) in line_cross_checks {
            let crossed = self.check_for_line_crossing(assets, entity_id, old_pos, new_pos, layer);
            if crossed {
                let is_human = self
                    .get_entity(entity_id)
                    .map(|e| e.is_human())
                    .unwrap_or(false);
                if is_human {
                    self.update_roll_after_crossing(assets, entity_id);
                }
                let compute_direction = self
                    .orders
                    .sequence_manager
                    .in_progress_element_for_actor_matching(entity_id, |e| e.data.is_movement())
                    .and_then(|(seq_id, elem_idx)| {
                        self.orders
                            .sequence_manager
                            .get_element(seq_id, elem_idx)
                            .and_then(|e| e.current_order())
                    })
                    .map(|order| order.compute_direction);
                if let Some(compute_direction) = compute_direction
                    && let Some(entity) = self.get_entity_mut(entity_id)
                {
                    entity
                        .position_iface_mut()
                        .compute_increment_all(compute_direction);
                }
            }
        }

        // Dispatch patch-line (LINE_PATCH) crossings for PCs.  On
        // crossing a LINE_PATCH line, route the PC's new position
        // through the patch's Enter/Leave + auto-Apply flow.
        for (entity_id, old_pos, new_pos, layer) in patch_cross_checks {
            self.check_for_patch_line_crossing(sim, assets, entity_id, old_pos, new_pos, layer);
        }
        for (entity_id, old_pos, new_pos, layer) in script_cross_checks {
            self.check_for_script_line_crossing(sim, assets, entity_id, old_pos, new_pos, layer);
        }

        // Dispatch transition-animation seek refreshes detected
        // during the per-tick movement loop.  Fires `RefreshSeek`
        // immediately when a queued transition-to-waiting order would
        // land too far from a moved target.  Same machinery as
        // `tick_refresh_seeks` — re-resolve the seek destination,
        // build a fresh single-element seek sequence, and re-launch
        // via `relaunch_seek_replacement`.  Runs before the LINE_SOUND
        // dispatch so the relaunched seek's first dispatch tick
        // already sees the freshly-built grid line state.
        for (owner, seq_id, elem_idx) in transition_seek_refreshes {
            // Re-read the seek element's flags / target / tolerance /
            // action because the dispatch above might have mutated
            // state on adjacent elements.  When the element no longer
            // looks like an entity-target seek, skip silently.
            let snapshot = self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .and_then(|e| match &e.data {
                    crate::sequence::SequenceElementData::Movement {
                        flags,
                        element,
                        tolerance,
                        action,
                        ..
                    } => element.map(|t| (*flags, t, *tolerance, *action)),
                    _ => None,
                });
            if let Some((flags, target, tolerance, action)) = snapshot {
                let new_target_pos = self
                    .get_entity(target)
                    .map(|e| e.element_data().position_map())
                    .unwrap_or_default();
                // The transition-arrival refresh arms SetMovingActionState
                // before calling RefreshSeek, just like the ordinary
                // PerformSeek target-drift branch.
                if let Some(actor) = self
                    .get_entity_mut(owner)
                    .and_then(|entity| entity.actor_data_mut())
                {
                    actor.action_state = actor.action_state.set_moving(false, false);
                }
                self.apply_seek_refresh(
                    sim,
                    assets,
                    owner,
                    seq_id,
                    elem_idx,
                    target,
                    action,
                    flags,
                    tolerance,
                    new_target_pos,
                );
            }
        }

        // Dispatch sound-line (LINE_SOUND) crossings for every actor.
        // On crossing a LINE_SOUND line, refresh the actor's
        // `material` from the new SECTOR_SOUND polygon containment
        // (or fall back to the obstacle / default material).  Drives
        // footstep sound playback parity.
        for (entity_id, old_pos, new_pos, layer) in sound_cross_checks {
            self.check_for_sound_line_crossing(assets, entity_id, old_pos, new_pos, layer);
        }

        // These calls are inside the Human/PC sword movement Execute arms,
        // after PerformMotion and before base Actor completion/DoNextOrder.
        if executed_sword_movement {
            self.quit_swordfight_with_far_opponents(sim, assets, owner);
            if matches!(owner, EntityId::Pc(_)) {
                let pinch_abort = self.world.entities.get(owner).and_then(|entity| {
                    entity.actor_data()?;
                    if !entity.position_iface().is_moving_map()
                        || !crate::engine::melee::enemies_are_blocking_my_movement(
                            &self.world.entities,
                            owner,
                        )
                    {
                        return None;
                    }
                    Some((selected.seq_id, selected.elem_idx))
                });
                if let Some((seq_id, elem_idx)) = pinch_abort {
                    self.orders
                        .sequence_manager
                        .element_impossible(seq_id, elem_idx);
                }
            }
        }

        // Execute pending door-pass triggers (PassingDoor steps).
        // These need &mut self for layer/sector changes and building callbacks.
        for (entity_id, door_index, direct, trigger_num) in door_triggers {
            self.execute_pass_door(sim, assets, entity_id, door_index, direct, trigger_num);
        }
        for (entity_id, door_index, direct) in completed_door_passes {
            tracing::debug!(
                entity = ?entity_id,
                door = %door_index,
                direct,
                "DoorPass: completed"
            );
            self.apply_completed_door_pass_lift_entry_state(entity_id, door_index, direct);
        }

        // Push queued door-pass Transition orders onto each actor's
        // current sequence element.  The current order list — the
        // transition order blocks subsequent orders until its sprite
        // animation completes.
        for (seq_id, elem_idx, order) in transition_pushes {
            self.orders
                .sequence_manager
                .push_order_on(seq_id, elem_idx, order);
        }

        // Fire pending Select hulk flashes.
        for (entity_id, speed) in select_triggers {
            self.apply_select_hulk(entity_id, speed);
        }

        for (entity_id, seq_id, elem_idx) in post_seek_arrivals {
            self.start_post_seek_sequence(entity_id, Some((seq_id, elem_idx)));
        }

        // These are derived Execute-arm tails in Original, so they close
        // after PerformMotion but before base Actor completion/DoNextOrder.
        self.tick_shouldered_carry_ceiling(assets, &executed_pc_movement_actions);
        if galopp_event {
            self.dispatch_galopp_loop_event(sim, assets, owner);
        }
        self.drain_script_synchronous_actions(sim, assets, &mut Vec::new())
            .unwrap_or_else(|error| {
                panic!(
                    "movement owner {owner:?} failed to drain synchronous Execute-arm callback work: {error:?}"
                )
            });

        // Drain collected waypoint pops against each actor's Move
        // element.  One pop per waypoint-arrival (both intermediate
        // and final).  When the final pop empties the order queue,
        // `do_next_order` internally calls `element_terminated` +
        // `ensure_wait_element` to transition the sequence element to
        // Terminated on queue exhaustion.  When an end-transition
        // order was spliced in by `post_process_path`, the final
        // walking pop leaves the end-transition as the new current
        // and the animation driver plays it; its own `do_next_order`
        // on completion then terminates the element.
        for (seq_id, elem_idx) in order_pops {
            self.do_next_order(seq_id, elem_idx);
        }
        // Original LaunchSequenceElement registers the Provoke from inside
        // Execute, but its Instruct arbitration runs only after the terminal
        // movement order has advanced. Launching before `do_next_order`
        // incorrectly compares the Wait-priority Provoke against the still
        // InProgress movement and abandons it.
        for entity_id in sword_movement_terminations {
            self.maybe_provoke_after_sword_movement_terminated(assets, entity_id);
        }

        // Drain water-splash titbit emissions queued from the walk
        // branch.  Emits a water particle at the actor's 3D position
        // with no element supplier.
        for (_eid, position, layer) in water_splash_emits {
            self.feedback.titbit_manager.add_titbit(
                position,
                layer,
                crate::titbit::TitbitKind::Water,
                crate::titbit::ElementHandle::INVALID,
                0,
                crate::titbit::ElementHandle::INVALID,
                false,
                crate::titbit::INVALID_ID,
                true, // display_titbits_enabled — config plumbing not threaded through this site yet
                None,
                None,
            );
        }
        for (seq_id, elem_idx) in blocked_impossible {
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
        }
        let selected_terminal = self
            .orders
            .sequence_manager
            .get_element(selected.seq_id, selected.elem_idx)
            .is_some_and(|element| {
                matches!(
                    element.state,
                    crate::sequence::SequenceState::Terminated
                        | crate::sequence::SequenceState::Impossible
                        | crate::sequence::SequenceState::Interrupted
                )
            });
        if selected_terminal {
            // Termination may synchronously instruct a cross-postponed
            // movement successor, but Actor::Hourglass has already made
            // and executed its one entry-latched order choice for this
            // owner slot. The successor becomes observable immediately and
            // executes at the actor's next Hourglass; never recurse into a
            // second Execute in the same slot.
            let _ = self.dispatch_condolations_for_owner_boundary(sim, owner, assets);
        }

        self.drain_script_synchronous_actions(sim, assets, &mut Vec::new())
            .unwrap_or_else(|error| {
                panic!(
                    "movement owner {owner:?} failed to drain synchronous callback work: {error:?}"
                )
            });
    }

    fn selected_galopp_decision_frame(
        &self,
        owner: EntityId,
        selected: MovementOwnerSelection,
    ) -> bool {
        let element = self
            .orders
            .sequence_manager
            .get_element(selected.seq_id, selected.elem_idx)
            .unwrap_or_else(|| {
                panic!("selected gallop movement element disappeared for {owner:?}")
            });
        let order = element
            .current_order()
            .unwrap_or_else(|| panic!("selected gallop movement order disappeared for {owner:?}"));
        if element.owner != Some(owner)
            || order.order_id != selected.order_id
            || order.order_type != OrderType::RunningUpright
        {
            return false;
        }
        let flags = match element.data {
            crate::sequence::SequenceElementData::Movement { flags, .. } => flags,
            _ => panic!("selected gallop owner {owner:?} no longer has a movement element"),
        };
        if !flags.contains(crate::sequence::MoveFlags::RIDER_CHARGE) {
            return false;
        }
        let sprite = self
            .world
            .entities
            .get(owner)
            .unwrap_or_else(|| panic!("selected gallop owner {owner:?} disappeared"))
            .sprite();
        is_galopp_decision_frame(
            sprite.current_frame,
            sprite.num_frames_for_anim(OrderType::RunningUpright),
        )
    }

    /// Test-only compatibility wrapper. Production movement is owned by the
    /// live legacy-slot Actor coordinator and never batches callback results.
    #[cfg(test)]
    pub(super) fn tick_entity_movement(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &crate::engine::LevelAssets,
    ) {
        let owners: Vec<EntityId> = self
            .world
            .entities
            .actors()
            .map(|(id, _)| id.into())
            .collect();
        for owner in owners {
            let selected = self
                .orders
                .sequence_manager
                .current_order_for_actor(owner)
                .and_then(|(seq_id, elem_idx, order)| {
                    self.orders
                        .sequence_manager
                        .get_element(seq_id, elem_idx)
                        .filter(|element| element.data.is_movement())
                        .map(|_| MovementOwnerSelection {
                            seq_id,
                            elem_idx,
                            order_id: order.order_id,
                        })
                });
            self.tick_entity_movement_owner(sim, assets, owner, selected);
        }
    }

    /// Advance through door-pass steps after a walk step completes.
    ///
    /// Pops one translated motion/door sub-order. `PassingDoor` action
    /// points are returned as real orders instead of being drained in the
    /// predecessor's completion slot: Original `Hourglass` executes one
    /// current order and only then advances to its successor. `Select`
    /// retains its generic-animation callback plumbing for now.
    ///
    /// See [`DoorPassAdvance`] for return semantics.
    pub(super) fn advance_door_pass(
        actor: &mut crate::element::ActorData,
        entity_id: EntityId,
        transition_destination: MapPoint,
        _door_triggers: &mut Vec<(EntityId, crate::gate::DoorIndex, bool, u8)>,
        _select_triggers: &mut Vec<(EntityId, f32)>,
        next_order_id: &mut u32,
    ) -> DoorPassAdvance {
        let dp = match actor.active_door_pass.as_mut() {
            Some(dp) => dp,
            None => {
                tracing::warn!(
                    entity = ?entity_id,
                    "DoorPass: advance requested without active pass"
                );
                return DoorPassAdvance::NoActive;
            }
        };
        let step = match dp.steps.pop_front() {
            Some(s) => s,
            None => {
                let completed = Some((dp.door_index, dp.direct));
                actor.active_door_pass = None;
                return DoorPassAdvance::Done { completed };
            }
        };

        match step {
            crate::element::DoorPassStep::PassingDoor => {
                let order_id = crate::order::alloc_order_id(next_order_id);
                let order = crate::order::Order::new(OrderType::PassingDoor, 0.0, 0.0, order_id);
                DoorPassAdvance::ActionPoint { order }
            }
            crate::element::DoorPassStep::Select { speed } => {
                // Original translates SELECT into a real non-animation order:
                // it is promoted after the preceding walk, executes the Human
                // hulk side effect in its own actor slot, then resumes the
                // remaining door chain. Skipping it advances PASSING_DOOR and
                // its topology swap by one frame.
                let order_id = crate::order::alloc_order_id(next_order_id);
                let mut order = crate::order::Order::new(OrderType::Select, 0.0, 0.0, order_id);
                order.compute_direction = true;
                order.tolerance = speed;
                order.completion = crate::order::OrderCompletion::ResumeDoorPass;
                DoorPassAdvance::ActionPoint { order }
            }
            crate::element::DoorPassStep::Transition { action, reverse } => {
                // The transition order sits at the front of the
                // order queue and blocks subsequent orders until
                // its sprite animation completes.  We build the
                // transition order here and hand it back to the
                // caller, who pushes it onto the actor's current
                // sequence element.  `ResumeDoorPass` completion
                // re-enters this function when the animation
                // finishes.
                //
                // Save the walking action state for the post-transition
                // walk.  Merely materializing the successor must not change
                // it yet: Original does not execute the transition until its
                // own Hourglass slot starts on the following tick.
                let saved = actor.action_state;
                actor.clear_path();
                if let Some(dp) = actor.active_door_pass.as_mut() {
                    dp.saved_action_state = Some(saved);
                    dp.current_action = action;
                    dp.current_reverse = reverse;
                }
                let order_id = crate::order::alloc_order_id(next_order_id);
                let mut order = crate::order::Order::new(
                    action,
                    transition_destination.x,
                    transition_destination.y,
                    order_id,
                );
                order.reverse = reverse;
                order.compute_direction = false;
                order.completion = crate::order::OrderCompletion::ResumeDoorPass;
                tracing::debug!(
                    entity = ?entity_id,
                    ?action,
                    reverse,
                    "DoorPass: pausing for Transition animation"
                );
                DoorPassAdvance::Paused {
                    transition_order: order,
                }
            }
            crate::element::DoorPassStep::Walk {
                destination,
                action,
                reverse,
                compute_direction,
                tolerance,
            } => {
                // The walk animation itself comes from `current_action`
                // (read by tick_entity_movement via `door_pass_anim`).  Keep
                // the saved pre-transition state until this new order is
                // actually dispatched on the following owner tick.
                if let Some(dp) = actor.active_door_pass.as_mut() {
                    dp.current_action = action;
                    dp.current_reverse = reverse;
                }
                // Hand the Walk destination back to the caller —
                // advance_door_pass doesn't have sequence_manager
                // access, so it can't push the walking order
                // directly onto the PassDoor element.  The caller
                // (tick_entity_movement's post-loop door-pass
                // dispatch) does the order push.
                DoorPassAdvance::Continue {
                    destination,
                    action,
                    reverse,
                    compute_direction,
                    tolerance,
                }
            }
        }
    }

    /// Runtime detector for Shape 1 contract violations — logs a warning
    /// (and fires a `debug_assert!`) when a movement intent is drained
    /// while the actor is still in a "waiting" substate that relies on
    /// an exit event the halt-teardown will suppress.
    ///
    /// The Shape 1 wrappers (`EnemyAi::go_to` et al.) force callers to
    /// commit a new substate before queuing a movement — if the current
    /// substate is still in the wedge-prone set at drain time, either a
    /// new caller bypassed the wrapper via `ai.base.go_to(...)` or an
    /// external code path queued the intent on the actor's behalf
    /// without a corresponding `set_state`.  In either case the halt
    /// below will swallow the exit event and leave the AI stranded.
    fn check_shape1_contract(&self, entity_id: EntityId) {
        let Some(entity) = self.world.entities.get(entity_id) else {
            return;
        };
        let Some(ai) = entity.ai_controller() else {
            return;
        };
        use crate::ai::Substate;
        let wedge_prone = matches!(
            ai.current_substate,
            Substate::AttackingSwordfightParade
                | Substate::AttackingReactiontime
                | Substate::AttackingReactiontimeTurning
                | Substate::AttackingReactiontimeBending
        );
        if wedge_prone {
            tracing::warn!(
                entity = entity_id.index(),
                substate = ?ai.current_substate,
                "Shape 1 violation: movement intent drained while actor is in a \
                 wedge-prone substate — halt-teardown will swallow the exit event. \
                 Likely cause: a caller bypassed EnemyAi::go_to / ai.base.go_to, or \
                 queued a movement intent without calling set_state first."
            );
            debug_assert!(
                !wedge_prone,
                "Shape 1 violation at entity {} in substate {:?}",
                entity_id.index(),
                ai.current_substate
            );
        }
    }

    /// Each tick, AI controllers may produce movement/action orders.
    /// This method drains them and submits corresponding path requests.
    ///
    /// `AiController::pending_halt` (set by `stop_all` / `FaceTo`) is
    /// drained inside [`Self::launch_pending_orders_for_npc`] so the
    /// halt happens on the same call stack as the new element launch,
    /// `StopAll` / `FaceTo` halt the actor inline.
    pub(super) fn process_pending_ai_orders(&mut self) {
        let npc_ids: Vec<_> = self.world.entities.npc_ids().collect();
        for npc_id in npc_ids {
            self.launch_pending_orders_for_npc(npc_id);
        }
    }

    /// Per-NPC half of [`Self::process_pending_ai_orders`] — drains one
    /// NPC's `pending_orders` queue and launches the corresponding
    /// movement / turn / generic sequences.  Called both from the
    /// top-of-tick global pass and from the per-NPC synchronous drain
    /// in [`EngineInner::dispatch_think_with_drain`] so `Face` / `GoTo`
    /// etc. take effect inside the same call stack as the handler that
    /// issued them — `Face` / `GoTo` launch the sequence inline.
    pub(super) fn launch_pending_orders_for_npc(&mut self, entity_id: EntityId) {
        self.launch_pending_orders_for_npc_mode(entity_id, false);
    }

    pub(super) fn launch_pending_orders_for_npc_mode(
        &mut self,
        entity_id: EntityId,
        defer_turn_instruction: bool,
    ) {
        self.launch_pending_orders_for_npc_mode_after_halt(
            entity_id,
            defer_turn_instruction,
            false,
        );
    }

    pub(super) fn launch_pending_orders_for_npc_mode_after_halt(
        &mut self,
        entity_id: EntityId,
        defer_turn_instruction: bool,
        halt_already_applied: bool,
    ) {
        // `StopAll` halts the actor inline before subsequent work,
        // and `FaceTo` / `GoTo` do the same on their own.  The Halt
        // is deferred to this drain (via `pending_halt`) so it runs
        // on the same tick as the pending-order launch.  Honor the
        // flag here — before launching new orders — so the
        // `Stop(Preference)` cascade interrupts any in-progress
        // sequence element (e.g. a yellow-? Turn mid-`bored-exit`)
        // and the new element launched below starts from a clean
        // slate.  `halt_actor` brackets the stop with
        // `inside_halt_method=true` so condolations queued by the
        // interrupt are tagged `from_halt` and don't fire
        // `Think(EventDone)`.
        let (has_pending_orders, take_halt) = {
            let Some(entity) = self.world.entities.get_mut(entity_id) else {
                return;
            };
            let Some(ai) = entity.ai_controller_mut() else {
                return;
            };
            let halt = ai.outbox.actor.halt;
            ai.outbox.actor.halt = false;
            (ai.has_pending_orders(), halt)
        };
        if take_halt {
            self.halt_actor(entity_id);
        }
        let had_explicit_halt = halt_already_applied || take_halt;
        if !has_pending_orders {
            return;
        }
        let intents: Vec<crate::order::AiOrderIntent> = {
            let Some(entity) = self.world.entities.get_mut(entity_id) else {
                return;
            };
            let Some(ai) = entity.ai_controller_mut() else {
                return;
            };
            ai.take_pending_orders()
        };
        // CoordinatePatrol may emit a Move and a Turn together; that compound
        // owner boundary was already synchronous and its relative arbitration
        // must remain intact. Every standalone FaceTo in a deferred owner
        // boundary remains registered until SequenceManager::Hourglass,
        // regardless of the actor's walking speed/state. Restricting this to
        // MovingFast let ordinary walking actors execute the newly exposed
        // exit transition one owner slot too early.
        let defer_standalone_turn = defer_turn_instruction
            && !intents.iter().any(|intent| {
                matches!(
                    intent.order_type,
                    OrderType::WalkingUpright
                        | OrderType::RunningUpright
                        | OrderType::WalkingCrouched
                        | OrderType::WalkingAlerted
                        | OrderType::RiderCharging
                )
            });

        for intent in intents {
            match intent.order_type {
                OrderType::WalkingUpright
                | OrderType::RunningUpright
                | OrderType::WalkingCrouched
                | OrderType::WalkingAlerted
                | OrderType::RiderCharging => {
                    let was_computing_path = self
                        .orders
                        .sequence_manager
                        .current_element_for_actor(entity_id)
                        .and_then(|(sequence_id, element_index)| {
                            self.orders
                                .sequence_manager
                                .get_element(sequence_id, element_index)
                        })
                        .is_some_and(|element| {
                            element.command == crate::element::Command::MoveWaiting
                        });
                    // `find_accessible` / `ask_obstacle` pre-flight
                    // gates.  Run them before the halt so a failure
                    // leaves the outgoing sequence in place rather
                    // than tearing it down only to abandon the new
                    // move.
                    let mut intent = intent;
                    if !self.preflight_ai_goto(entity_id, &mut intent) {
                        continue;
                    }
                    // Original Halt preserves a selected movement long enough
                    // for the replacement Move to become mpSequenceElement.
                    // Only then is the old movement interrupted, so its
                    // SendCondolationCard no longer owns the selected element
                    // and cannot clear PositionGoalMap. Preserve that cached
                    // transition goal across Rust's eager halt; launching the
                    // replacement below remains free to overwrite it when a
                    // path is available immediately.
                    //
                    // An explicit StopAll before this GoTo has already
                    // performed the selected-element cleanup and must not
                    // resurrect the old destination.
                    let retained_movement_goal = (!had_explicit_halt)
                        .then(|| {
                            self.orders
                                .sequence_manager
                                .current_element_for_actor(entity_id)
                                .and_then(|(seq, idx)| {
                                    self.orders.sequence_manager.get_element(seq, idx)
                                })
                                .is_some_and(|element| element.data.is_movement())
                        })
                        .filter(|selected_is_movement| *selected_is_movement)
                        .and_then(|_| {
                            self.world
                                .entities
                                .get(entity_id)
                                .map(|entity| entity.position_iface().map_goal())
                        });
                    intent.retained_movement_goal = retained_movement_goal;
                    // The Original source spells this pre-launch gate as
                    // `uwFlags & GOTO_NOHALT == 0`. C/C++ precedence parses
                    // it as `uwFlags & (GOTO_NOHALT == 0)`, which is always
                    // zero: ordinary GoTo never halts here. Keep explicit
                    // StopAll/Halt effects at their own call sites instead of
                    // "fixing" the legacy bug.
                    self.launch_ai_move(entity_id, &intent);

                    // GoTo has a separate, effective tail check after
                    // launching its sequence: an actor whose old movement is
                    // still waiting on the pathfinder is halted. The halt
                    // also cancels the just-registered replacement, matching
                    // StopNotYetLaunchedSequenceElements.
                    if was_computing_path {
                        self.check_shape1_contract(entity_id);
                        self.halt_actor(entity_id);
                    }
                }
                OrderType::Turning => {
                    let turn_command = if intent.fast_turn {
                        crate::element::Command::TurnFast
                    } else {
                        crate::element::Command::Turn
                    };
                    // A SetState callback can synchronously register an
                    // attentive-mode transition, then a re-entrant
                    // EventReachPoint can register FaceTo before the manager
                    // hourglass gets to either element. Original arbitration
                    // postpones the Turn without translating it, so its
                    // direction goal remains untouched until the attentive
                    // transition finishes.
                    let attentive_transition_blocks_turn = self
                        .orders
                        .sequence_manager
                        .current_element_for_actor(entity_id)
                        .and_then(|(seq, idx)| self.orders.sequence_manager.get_element(seq, idx))
                        .is_some_and(|element| {
                            matches!(
                                element.command,
                                crate::element::Command::EnterAttentiveMode
                                    | crate::element::Command::LeaveAttentiveMode
                                    | crate::element::Command::LeaveAttentiveModeOfficer
                            )
                        })
                        || [
                            crate::element::Command::EnterAttentiveMode,
                            crate::element::Command::LeaveAttentiveMode,
                            crate::element::Command::LeaveAttentiveModeOfficer,
                        ]
                        .into_iter()
                        .any(|command| {
                            self.orders
                                .sequence_manager
                                .element_is_about_to_be_launched(entity_id, command)
                        });
                    let current_movement = self
                        .orders
                        .sequence_manager
                        .current_element_for_actor(entity_id)
                        .and_then(|(seq, idx)| self.orders.sequence_manager.get_element(seq, idx))
                        .filter(|element| element.data.is_movement());
                    if let Some(current_movement) = current_movement {
                        let interrupts_retained_stop_transition =
                            current_movement.orders.front().is_some_and(|order| {
                                matches!(
                                    order.order_type,
                                    OrderType::TransitionWalkingUprightWaitingUpright
                                        | OrderType::TransitionRunningUprightWaitingUpright
                                        | OrderType::TransitionWalkingCrouchedWaitingCrouched
                                )
                            });
                        // FaceTo interrupts the movement and installs its
                        // transition early enough for this actor slot, while
                        // retaining the movement goal cached by the sprite.
                        // A standalone FaceTo halts a live movement only to
                        // rotate the actor and therefore retains its cached
                        // destination. An explicit StopAll/Halt preceding the
                        // Face has already cleared the selected movement goal,
                        // so sampling here naturally retains zero instead of
                        // resurrecting the old destination.
                        // A second Halt against an already-retained
                        // transition-to-waiting is different: it interrupts
                        // that selected element outright, so Actor's
                        // condolence clears the goal before the Turn is
                        // instructed.
                        let retained_goal = (!had_explicit_halt
                            && !interrupts_retained_stop_transition)
                            .then(|| {
                                self.world
                                    .entities
                                    .get(entity_id)
                                    .map(|entity| entity.position_iface().map_goal())
                            })
                            .flatten();
                        self.halt_actor(entity_id);
                        if interrupts_retained_stop_transition
                            && let Some(entity) = self.world.entities.get_mut(entity_id)
                        {
                            entity
                                .position_iface_mut()
                                .set_map_goal(crate::coordinates::MapPoint::ZERO);
                        }
                        let direction = intent.explicit_direction.or_else(|| {
                            self.world.entities.get(entity_id).map(|entity| {
                                let position = entity.element_data().position_map();
                                crate::position_interface::vector_to_sector_0_to_15_iso(
                                    intent.target_x - position.x,
                                    intent.target_y - position.y,
                                )
                            })
                        });
                        if defer_standalone_turn
                            || intent.defer_instruction
                            || attentive_transition_blocks_turn
                        {
                            self.launch_turn_sequence_deferred_no_transitions(
                                entity_id,
                                turn_command,
                                direction,
                                intent.target_x,
                                intent.target_y,
                                retained_goal,
                            );
                        } else {
                            let mut intent = intent;
                            intent.defer_initial_turn_step = self.control.frame_counter == 0;
                            let order = intent.stamp(self.orders.allocate_order_id());
                            let (_, instructed) = self
                                .launch_single_order_sequence_stamped_ex_configured(
                                    entity_id,
                                    turn_command,
                                    order,
                                    true,
                                    |element| {
                                        if let Some(direction) = direction {
                                            element.set_property(
                                                crate::sequence::Field::Direction,
                                                crate::sequence::FieldValue::Integer(
                                                    direction as u32,
                                                ),
                                            );
                                        }
                                    },
                                );
                            if instructed
                                && let (Some(direction), Some(entity)) =
                                    (direction, self.world.entities.get_mut(entity_id))
                            {
                                entity.element_data_mut().set_direction_goal(direction);
                            }
                        }
                        if let (Some(goal), Some(entity)) =
                            (retained_goal, self.world.entities.get_mut(entity_id))
                        {
                            entity.position_iface_mut().set_map_goal(goal);
                        }
                    } else {
                        let mut intent = intent;
                        intent.defer_initial_turn_step = self.control.frame_counter == 0;
                        if !intent.no_halt {
                            self.halt_actor(entity_id);
                        }
                        if defer_standalone_turn
                            || intent.defer_instruction
                            || attentive_transition_blocks_turn
                        {
                            self.launch_turn_sequence_deferred_no_transitions(
                                entity_id,
                                turn_command,
                                intent.explicit_direction,
                                intent.target_x,
                                intent.target_y,
                                None,
                            );
                        } else {
                            // Original Face resolves positional targets during
                            // FaceTo and synchronous Turn instruction writes
                            // the direction goal before LaunchSequence
                            // returns. This owner-local drain can run after
                            // the global turn phase, so install both authored
                            // and positional goals here rather than waiting
                            // for next frame's command dispatch.
                            let direction = intent.explicit_direction.or_else(|| {
                                self.world.entities.get(entity_id).map(|entity| {
                                    let position = entity.element_data().position_map();
                                    crate::position_interface::vector_to_sector_0_to_15_iso(
                                        intent.target_x - position.x,
                                        intent.target_y - position.y,
                                    )
                                })
                            });
                            let order = intent.stamp(self.orders.allocate_order_id());
                            let (_, instructed) = self
                                .launch_single_order_sequence_stamped_ex_configured(
                                    entity_id,
                                    turn_command,
                                    order,
                                    true,
                                    |element| {
                                        if let Some(direction) = direction {
                                            element.set_property(
                                                crate::sequence::Field::Direction,
                                                crate::sequence::FieldValue::Integer(
                                                    direction as u32,
                                                ),
                                            );
                                        }
                                    },
                                );
                            if instructed
                                && let (Some(direction), Some(entity)) =
                                    (direction, self.world.entities.get_mut(entity_id))
                            {
                                entity.element_data_mut().set_direction_goal(direction);
                            }
                        }
                    }
                }
                _ => {
                    // Other order types go on their own single-order
                    // sequence for the animation driver to pick up.
                    let order = intent.stamp(self.orders.allocate_order_id());
                    self.launch_single_order_sequence_stamped(
                        entity_id,
                        crate::element::Command::Generic,
                        order,
                    );
                }
            }
        }
    }

    // ─── Elevation-line crossing ──────────────────────────────────

    /// Find a projection-area sight obstacle on `layer` whose
    /// screen-space plane contains `pos`.
    ///
    /// Used by the elevation-line emergency fallbacks: iterate plane
    /// sectors in the spatial bucket at `(pos, layer 0)`, then keep
    /// the one whose attached sight obstacle's layer matches and whose
    /// screen-space sector plane contains the position.  We don't carry
    /// a plane-sector registry yet — but every plane sector wraps a
    /// single projection-area obstacle, so iterating projection-area
    /// obstacles directly gives the same answer.
    pub(super) fn find_plane_obstacle_at(
        &self,
        assets: &LevelAssets,
        layer: u16,
        pos: MapPoint,
    ) -> Option<u16> {
        self.find_plane_obstacle_split(assets, layer, pos, pos)
    }

    /// Asymmetric variant used by the second-emergency probe in
    /// `cross_elevation_line`.  The bounding-box check is evaluated
    /// at the 2-units-ahead probe but the polygon containment check
    /// is at the actor's *current* map position.  In a band where the
    /// probe has left the current polygon but the actor has not, the
    /// old polygon is accepted.  Use `bbox_at` = probe and
    /// `polygon_at` = current map position to capture that.
    fn find_plane_obstacle_split(
        &self,
        assets: &LevelAssets,
        layer: u16,
        bbox_at: MapPoint,
        polygon_at: MapPoint,
    ) -> Option<u16> {
        for (oi, obs) in self.sight_obstacles(assets).iter_indexed() {
            if !obs.is_projection_area() {
                continue;
            }
            if obs.layer != layer {
                continue;
            }
            if !obs.box_projection.contains_point(bbox_at) {
                continue;
            }
            if !obs.contains_point_projection(polygon_at) {
                continue;
            }
            return Some(oi as u16);
        }
        None
    }

    fn crossed_elevation_obstacle(
        current: Option<u16>,
        left: Option<u16>,
        right: Option<u16>,
    ) -> Option<Option<u16>> {
        if current == left {
            Some(right)
        } else if current == right {
            Some(left)
        } else {
            None
        }
    }

    /// Elevation-bond crossing for a shipped mobile master. The C++ mobile
    /// owns the obstacle pointer, then propagates it to every masked child.
    /// Its multi-line branch accidentally iterates a zero counter; preserve
    /// that release behavior and only cross when this tick sees exactly one
    /// unique, non-origin elevation line.
    pub(super) fn check_mobile_line_crossing(&mut self, assets: &LevelAssets, mobile_index: usize) {
        let (old_pos, new_pos, layer, increment, current) = {
            let mobile = self
                .world
                .mobile_elements
                .get(mobile_index)
                .unwrap_or_else(|| panic!("missing mobile {mobile_index}"));
            (
                mobile.old_position,
                mobile.position,
                mobile.layer,
                mobile.increment,
                mobile.obstacle,
            )
        };
        #[cfg(test)]
        LAST_MOBILE_CROSSING_INCREMENT.with(|observed| observed.set(Some(increment)));
        if old_pos == new_pos {
            return;
        }

        let mut indices = self
            .world
            .fast_grid
            .get_crossing_elevation_line_indices(layer, old_pos, new_pos);
        indices.dedup_by(|left_idx, right_idx| {
            let left = &self.world.fast_grid.level.lines[usize::from(*left_idx)];
            let right = &self.world.fast_grid.level.lines[usize::from(*right_idx)];
            (left.a == right.a && left.b == right.b) || (left.a == right.b && left.b == right.a)
        });
        indices.retain(|idx| {
            let line = &self.world.fast_grid.level.lines[usize::from(*idx)];
            let vector = line.b - line.a;
            let from_a = old_pos - line.a;
            vector.x * from_a.y - vector.y * from_a.x != 0.0
        });
        if indices.len() != 1 {
            return;
        }

        let line = &self.world.fast_grid.level.lines[usize::from(indices[0])];
        let mut next = Self::crossed_elevation_obstacle(
            current,
            line.left_obstacle_index,
            line.right_obstacle_index,
        )
        .flatten();
        if current != line.left_obstacle_index && current != line.right_obstacle_index {
            next = self.find_plane_obstacle_at(assets, layer, new_pos);
            if next.is_none() && increment != MapVec::ZERO {
                let probe = new_pos + increment.scale(2.0);
                next = self.find_plane_obstacle_split(assets, layer, probe, new_pos);
            }
            if next.is_none() {
                tracing::debug!(
                    mobile_index,
                    ?current,
                    left = ?line.left_obstacle_index,
                    right = ?line.right_obstacle_index,
                    "mobile crossed an illegal elevation bond with no projection-area fallback"
                );
                return;
            }
        }

        let sprite_ids = {
            let mobile = &mut self.world.mobile_elements[mobile_index];
            mobile.obstacle = next;
            mobile.sprite_ids.clone()
        };
        for sprite_id in sprite_ids {
            self.set_obstacle_and_material(assets, sprite_id, next);
        }
    }

    fn expand_move_box_for_command_extraction(bbox: MapBBox) -> MapBBox {
        if bbox.is_somewhere() {
            MapBBox::from_coords(
                bbox.x_min() - 0.5,
                bbox.y_min() - 0.5,
                bbox.x_max() + 0.5,
                bbox.y_max() + 0.5,
            )
        } else {
            bbox
        }
    }

    /// Swap an actor's sight-obstacle pointer to the opposite side of
    /// an elevation line it just crossed and update the footstep
    /// material + 3D plane projection.
    ///
    /// Given the line's stored left/right obstacle indices, flip the
    /// actor's `obstacle_index` to the other side.  The new obstacle
    /// is then routed through `set_obstacle_and_material` so the
    /// actor picks up the new top-plane and footstep material
    /// immediately.  Finally the sprite is reprojected from the new
    /// map position onto the new plane.
    ///
    /// When the actor's current obstacle matches neither side
    /// ("illegal bond crossing"), two emergency fallbacks run: walk
    /// the plane-sector registry at the actor's position for a
    /// containing plane, then if that misses retry at `pos + 2 *
    /// increment_map`.  Both are reproduced via
    /// [`Self::find_plane_obstacle_at`].
    ///
    /// `new_pos` is the actor's post-move map position. `increment_map`
    /// is a unit vector in the movement direction (used by the second
    /// emergency probe).
    pub(super) fn cross_elevation_line(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        line_idx: crate::fast_find_grid::LineIndex,
        new_pos: MapPoint,
        increment_map: MapVec,
    ) {
        let line = match self.world.fast_grid.level.lines.get(usize::from(line_idx)) {
            Some(l) if l.is_elevation => l,
            _ => return,
        };
        let left = line.left_obstacle_index;
        let right = line.right_obstacle_index;

        let (current, layer) = match self.world.entities.get(entity_id) {
            Some(e) => (
                e.element_data().obstacle_index().map(u16::from),
                e.element_data().layer(),
            ),
            None => return,
        };

        let mut next: Option<u16>;
        let mut found = true;

        if let Some(crossed) = Self::crossed_elevation_obstacle(current, left, right) {
            // legacy implementation compares raw obstacle pointers here, so NULL is a
            // valid side of an elevation line and must cross to the
            // opposite side instead of falling into the emergency path.
            next = crossed;
        } else {
            // "VERBOTEN: Illegal bond crossing" — current obstacle
            // matches neither side.  Walk projection-area obstacles
            // for one containing the actor's current position on its
            // layer.
            tracing::debug!(
                entity = ?entity_id,
                ?current,
                ?left,
                ?right,
                "cross_elevation_line: obstacle pointer doesn't match either side (illegal bond crossing)"
            );
            next = self.find_plane_obstacle_at(assets, layer, new_pos);
            if next.is_none() {
                // "STRENG VERBOTEN" — second emergency: probe two
                // map units ahead in the movement direction.  Gated
                // on a real direction (non-zero `increment_map`) —
                // when `check_for_line_crossing` early-returns on a
                // zero-length step the probe never reaches us, but
                // if a future caller wires this with an unfilled
                // increment we skip the second emergency rather than
                // probing in the wrong direction.
                let increment_computed =
                    increment_map.x.abs() > 1e-9 || increment_map.y.abs() > 1e-9;
                if increment_computed {
                    let probe = MapPoint::new(
                        new_pos.x + 2.0 * increment_map.x,
                        new_pos.y + 2.0 * increment_map.y,
                    );
                    tracing::debug!(
                        entity = ?entity_id,
                        "cross_elevation_line: second emergency, probing 2 units ahead at ({:.1}, {:.1})",
                        probe.x,
                        probe.y,
                    );
                    // Asymmetric predicate: bbox at the probe point,
                    // polygon containment at the actor's current
                    // (post-move) position.
                    next = self.find_plane_obstacle_split(assets, layer, probe, new_pos);
                }
                if next.is_none() {
                    // "ABSOLUT VERBOTEN" — give up; leave the actor's
                    // obstacle alone and skip the reprojection.
                    tracing::debug!(
                        entity = ?entity_id,
                        "cross_elevation_line: no projection area found at ({:.1}, {:.1})",
                        new_pos.x,
                        new_pos.y,
                    );
                    found = false;
                }
            }
        }

        if !found {
            return;
        }

        // Apply the new obstacle: updates element_data.obstacle_index,
        // element_data.material (footstep sounds), and the actor's
        // PositionInterface (obstacle, top-plane, material).
        self.set_obstacle_and_material(assets, entity_id, next);

        // Reproject the sprite onto the new plane.  Per-frame
        // movement updates `element_data.position_map` directly
        // without touching `position_iface`, so seed
        // `position_iface.position_map` from the freshly moved
        // position before recomputing 3D.
        if let Some(entity) = self.get_entity_mut(entity_id) {
            let pi = entity.position_iface_mut();
            pi.set_map_position(crate::coordinates::MapPoint {
                x: new_pos.x,
                y: new_pos.y,
            });
        }
    }

    /// Per-tick line-crossing dispatch for a moving actor.
    ///
    /// Restricted to elevation-line crossings.  For each elevation
    /// line the actor's `(old_pos, new_pos)` segment crosses on its
    /// current layer, we swap the actor's obstacle pointer via
    /// `cross_elevation_line`.  When multiple elevation lines are
    /// crossed in one tick, they are bubble-sorted by obstacle
    /// continuity so consecutive `cross_elevation_line` calls walk an
    /// actual chain of adjacent obstacles.
    ///
    /// Returns `true` if any elevation line was crossed — callers can
    /// use that to fire the human-specific `UpdateRoll` follow-up.
    pub(super) fn check_for_line_crossing(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        old_pos: MapPoint,
        new_pos: MapPoint,
        layer: u16,
    ) -> bool {
        // Early-out: exact same position means no crossing at all.
        if (old_pos.x - new_pos.x).abs() < 1e-4 && (old_pos.y - new_pos.y).abs() < 1e-4 {
            return false;
        }

        let mut indices = self
            .world
            .fast_grid
            .get_crossing_elevation_line_indices(layer, old_pos, new_pos);
        tracing::trace!(
            target: "robin_engine::elevation_crossing",
            ?entity_id,
            layer,
            old_x = old_pos.x,
            old_y = old_pos.y,
            new_x = new_pos.x,
            new_y = new_pos.y,
            crossing_count = indices.len(),
            "queried elevation crossings"
        );
        if indices.is_empty() {
            return false;
        }

        // Read the actor's current obstacle — used as the seed for the
        // sort when multiple lines are crossed.
        let mut current_obstacle = match self.world.entities.get(entity_id) {
            Some(e) => e.element_data().obstacle_index().map(u16::from),
            None => return false,
        };

        // Bubble-sort elevation lines by obstacle continuity.  Each
        // iteration picks the next line whose left or right side
        // matches the running `current_obstacle`, swaps it into
        // place, and advances the running obstacle.  If no line
        // matches we stop sorting — later indices will still be
        // dispatched in whatever order they came out of the grid.
        let n = indices.len();
        if n > 1 {
            for i in 0..n.saturating_sub(1) {
                let mut matched = false;
                for j in i..n {
                    let line = match self
                        .world
                        .fast_grid
                        .level
                        .lines
                        .get(usize::from(indices[j]))
                    {
                        Some(l) => l,
                        None => continue,
                    };
                    if line.left_obstacle_index == current_obstacle {
                        current_obstacle = line.right_obstacle_index;
                        indices.swap(i, j);
                        matched = true;
                        break;
                    }
                    if line.right_obstacle_index == current_obstacle {
                        current_obstacle = line.left_obstacle_index;
                        indices.swap(i, j);
                        matched = true;
                        break;
                    }
                }
                if !matched {
                    break;
                }
            }
        }

        // Compute the unit movement vector for the second-emergency
        // probe inside `cross_elevation_line`.
        let increment_map = {
            let dx = new_pos.x - old_pos.x;
            let dy = new_pos.y - old_pos.y;
            let len = (dx * dx + dy * dy).sqrt();
            if len > 1e-6 {
                MapVec::new(dx / len, dy / len)
            } else {
                MapVec::ZERO
            }
        };

        // Dispatch the swaps in order.
        for &idx in &indices {
            self.cross_elevation_line(assets, entity_id, idx, new_pos, increment_map);
        }

        true
    }

    /// Per-tick `LINE_PATCH` crossing dispatch for a PC.
    ///
    /// On crossing a LINE_PATCH line:
    ///
    /// ```text
    ///   if patch is active:
    ///       if patch.apply_sector contains GetPositionMap():
    ///           patch.Enter(actor)
    ///           if !patch.is_applied: patch.Apply()
    ///       else:
    ///           patch.Leave(actor)
    ///           if patch.is_applied && patch.any_occupant().is_none():
    ///               patch.Apply()
    /// ```
    ///
    /// Uses the PC's `new_pos` as the post-move probe.  `inside` means
    /// the PC just entered the apply polygon, `outside` means the PC
    /// just left it.  Patch state machine, FX entity, sight obstacles,
    /// grid sectors, and door rights are updated via
    /// `process_patch_effects`.
    pub(super) fn check_for_patch_line_crossing(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
        old_pos: MapPoint,
        new_pos: MapPoint,
        layer: u16,
    ) {
        if (old_pos.x - new_pos.x).abs() < 1e-4 && (old_pos.y - new_pos.y).abs() < 1e-4 {
            return;
        }

        let indices = self
            .world
            .fast_grid
            .get_crossing_patch_line_indices(layer, old_pos, new_pos);
        if indices.is_empty() {
            return;
        }

        let occupant = crate::patch::OccupantId(entity_id.index());

        // `Patch::enter` / `leave` recurse onto the actor's carried
        // entity when the actor is a PC and is currently carrying
        // someone.  Resolve that here once (same entity for every
        // crossed patch this tick) so each per-patch Enter/Leave can
        // mirror the recursion.  `patch_cross_checks` only collects
        // PCs.
        let carried_occupant = self
            .get_entity(entity_id)
            .and_then(|e| match e {
                crate::element::Entity::Pc(pc) => pc.pc.carried,
                _ => None,
            })
            .map(|cid| crate::patch::OccupantId(cid.index()));

        // Collect unique patches crossed this frame — one PC step can
        // intersect multiple boundary edges of the same apply polygon
        // (e.g. clipping a corner), and only the net Enter/Leave
        // decision matters, which is independent of edge count.
        let mut seen: Vec<crate::patch::PatchIndex> = Vec::new();
        for &line_idx in &indices {
            let Some(line) = self.world.fast_grid.level.lines.get(usize::from(line_idx)) else {
                continue;
            };
            let Some(patch_index) = line.patch_index else {
                continue;
            };
            if seen.contains(&patch_index) {
                continue;
            }
            seen.push(patch_index);
        }

        for patch_index in seen {
            // Snapshot the apply-sector polygon test result + active
            // state + applied state + occupant emptiness *before*
            // mutating the patch, so the action decision keeps the
            // strict order: is_active → is_inside → Enter/Leave →
            // conditional Apply.
            let patch_usize = patch_index.get() as usize;

            let (is_active, apply_sector_idx) = {
                if self.scripts.mission.is_none() {
                    return;
                }
                let Some(patch) = self.script_domains.interactables.patches.get(patch_usize) else {
                    continue;
                };
                (patch.is_active(), patch.apply_sector_index)
            };
            if !is_active {
                continue;
            }
            let apply_sector_idx = match apply_sector_idx {
                Some(i) => i,
                None => {
                    // No apply polygon declared — treat as "always
                    // outside" safety net and log once.
                    tracing::warn!(
                        patch = %patch_index,
                        "LINE_PATCH crossing on patch with no apply sector — skipping",
                    );
                    continue;
                }
            };
            let Some(apply_sector) = self
                .world
                .fast_grid
                .level
                .sectors
                .get(apply_sector_idx as usize)
            else {
                continue;
            };
            let inside_apply = apply_sector.contains_point(new_pos);

            let effects = {
                if self.scripts.mission.is_none() {
                    return;
                }
                let Some(patch) = self
                    .script_domains
                    .interactables
                    .patches
                    .get_mut(patch_usize)
                else {
                    continue;
                };
                if inside_apply {
                    // Entering the apply region.  `patch.enter`
                    // already logs and bails when the occupant is
                    // already in the list, so repeated single-tick
                    // crossings don't double-insert.
                    patch.enter(occupant);
                    // Carried-actor recursion: runs unconditionally
                    // after the warn/insert branch.
                    if let Some(carried) = carried_occupant {
                        patch.enter(carried);
                    }
                    if !patch.is_applied() {
                        patch.apply()
                    } else {
                        Vec::new()
                    }
                } else {
                    patch.leave(occupant);
                    // Carried-actor recursion: runs unconditionally
                    // after the find/remove branch.
                    if let Some(carried) = carried_occupant {
                        patch.leave(carried);
                    }
                    if patch.is_applied() && patch.any_occupant().is_none() {
                        patch.apply()
                    } else {
                        Vec::new()
                    }
                }
            };

            if !effects.is_empty() {
                self.process_patch_effects(sim, assets, patch_index, effects);
            }
        }
    }

    /// Per-tick LINE_SOUND crossing dispatch for a moving actor.
    ///
    /// When the actor's `(old_pos, new_pos)` segment crosses one or
    /// more active LINE_SOUND grid lines on its current layer,
    /// refresh `actor.material` from the new ground material via
    /// [`MaterialSectors::material_at`] (which combines the
    /// "is-inside material polygon" test with the obstacle /
    /// default-material fallback in a single call).
    ///
    /// Updates both `ElementData::material` (read by footstep sound
    /// playback) and the actor's `PositionInterface` material so
    /// subsequent reads match.
    pub(super) fn check_for_sound_line_crossing(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        old_pos: MapPoint,
        new_pos: MapPoint,
        layer: u16,
    ) {
        if (old_pos.x - new_pos.x).abs() < 1e-4 && (old_pos.y - new_pos.y).abs() < 1e-4 {
            return;
        }

        let indices = self
            .world
            .fast_grid
            .get_crossing_sound_line_indices(layer, old_pos, new_pos);
        if indices.is_empty() {
            return;
        }

        let obstacle_material = self
            .get_entity(entity_id)
            .and_then(|e| e.element_data().obstacle_index())
            .map(|handle| {
                let idx: usize = handle.into();
                self.sight_obstacles(assets)
                    .get(idx)
                    .unwrap_or_else(|| {
                        panic!(
                            "entity {} references missing sight obstacle {idx}",
                            entity_id.index()
                        )
                    })
                    .material
            })
            .map(|raw| crate::element::GameMaterial::from_u32(raw as u32))
            .unwrap_or(assets.material_sectors.default_material);

        // Original sorts non-elevation crossings by intersection distance
        // from the old actor position, then processes each crossed line's
        // exact owning material sector. The last processed line wins.
        let movement = crate::geo2d::segment(old_pos.to_geo(), new_pos.to_geo());
        let mut crossed: Vec<(f32, crate::fast_find_grid::LineIndex)> = indices
            .into_iter()
            .filter_map(|line_index| {
                let line = &self.world.fast_grid.level.lines[usize::from(line_index)];
                let old_dx = old_pos.x - line.a.x;
                let old_dy = old_pos.y - line.a.y;
                let line_dx = line.b.x - line.a.x;
                let line_dy = line.b.y - line.a.y;
                // Original removes crossings whose old position lies exactly
                // on the line to avoid processing the same boundary twice.
                if line_dx * old_dy - line_dy * old_dx == 0.0 {
                    return None;
                }
                let point = crate::geo2d::segment_intersection(movement, line.segment()).point()?;
                let dx = point.x - old_pos.x;
                let dy = point.y - old_pos.y;
                Some((dx * dx + dy * dy, line_index))
            })
            .collect();
        crossed.sort_by(|(left, _), (right, _)| left.total_cmp(right));
        let crossing_count = crossed.len();

        let mut new_material = None;
        for (_, line_index) in crossed {
            let line = &self.world.fast_grid.level.lines[usize::from(line_index)];
            let raw_index = line.sound_material_sector_index.unwrap_or_else(|| {
                panic!("LINE_SOUND {line_index:?} has no owning material sector")
            });
            let sector = assets
                .all_material_sectors
                .get(usize::from(raw_index))
                .and_then(Option::as_ref)
                .unwrap_or_else(|| {
                    panic!(
                        "LINE_SOUND {line_index:?} references missing material sector {raw_index}"
                    )
                });
            new_material = Some(if sector.contains(new_pos) {
                sector.material
            } else {
                obstacle_material
            });
        }
        let Some(new_material) = new_material else {
            return;
        };

        if let Some(entity) = self.get_entity_mut(entity_id) {
            let prev = entity.element_data().material();
            if prev != new_material {
                entity.element_data_mut().set_material(new_material);
                let pi = entity.position_iface_mut();
                pi.set_material(new_material);
                tracing::trace!(
                    ?entity_id,
                    ?prev,
                    ?new_material,
                    crossings = crossing_count,
                    "check_for_sound_line_crossing: refreshed material"
                );
            }
        }
    }

    /// Prepare a Move / Seek sequence element for dispatch.
    ///
    /// Direct moves populate their orders immediately. A*-requiring moves
    /// snapshot a [`PendingPathRequest`], transition to `MoveWaiting`, and
    /// complete later through [`EngineInner::process_next_path_request`].
    pub(crate) fn try_dispatch_move_path(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        _assets: &LevelAssets,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        dest: MapPoint,
        mut move_action: OrderType,
    ) -> MovePathOutcome {
        // Swap walking/running into the sword variant when the actor
        // is already in a sword action state — but only under two
        // gates:
        //   1. The post-transition posture is Upright — the swap is
        //      skipped for non-upright post-transition postures (e.g.
        //      CarryingCorpse, HelpingToClimb, ...).
        //   2. The action-state-after-transition is a sword state.
        // Read both from the SequenceElement rather than the live
        // entity state so a Move queued with a post-transition sword
        // state (e.g. launched from a posture/action transition that
        // hasn't applied yet) uses the intended post-transition
        // values.
        //
        // WalkingWithSword / RunningWithSword are logical non-animation
        // dispatch tokens. The Human Execute override resolves them through
        // FaceOpponent to a concrete forward/backward/strafe sword row.
        let (posture_after, mut action_after) = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .map(|e| (e.posture_after_transition, e.action_state_after_transition))
            .unwrap_or_default();
        let elem_flags = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .and_then(|e| match &e.data {
                crate::sequence::SequenceElementData::Movement { flags, .. } => Some(*flags),
                _ => None,
            })
            .unwrap_or(crate::sequence::MoveFlags::empty());
        let is_fast = elem_flags.contains(crate::sequence::MoveFlags::FAST);
        // STEP_BACK_IN_COMBAT describes how an authored sword move was
        // selected; unlike FORCE_SWORD_MOVEMENT it does not keep the move in
        // sword form after the actor has lowered the weapon.
        let force_sword_movement =
            elem_flags.contains(crate::sequence::MoveFlags::FORCE_SWORD_MOVEMENT);
        if force_sword_movement && posture_after == crate::element::Posture::Upright {
            action_after = if is_fast || matches!(move_action, OrderType::RunningUpright) {
                crate::element::ActionState::MovingFastSword
            } else {
                crate::element::ActionState::MovingSword
            };
            if let Some(elem) = self
                .orders
                .sequence_manager
                .get_element_mut(seq_id, elem_idx)
            {
                elem.action_state_after_transition = action_after;
            }
        }
        let sword_movement_context = (posture_after == crate::element::Posture::Upright
            && action_after.is_sword())
            || force_sword_movement;
        if sword_movement_context {
            move_action = sword_movement_dispatch_action(move_action);
        }
        // PC shield-action arm: a shield-wielding PC on Upright
        // ground rewrites the movement element's stored `action`:
        //   WALKING_UPRIGHT / WALKING_WITH_CORPSE → WALKING_WITH_SHIELD
        //   WALKING_WITH_SHIELD                     → already set, no-op
        //   RUNNING_UPRIGHT                         → no
        //                                             running-with-shield
        //                                             anim, leave the
        //                                             upright variant
        //   default                                 → warn (would
        //                                             assert in dev).
        // Gated on PC because soldier/civilian shield holders fall
        // through to the upright animation.  Skip when the sprite
        // lacks the shield row (mirrors the sword PC fallback above).
        let owner_is_pc = self.world.entities.get(owner).is_some_and(|e| e.is_pc());
        if owner_is_pc
            && posture_after == crate::element::Posture::Upright
            && action_after.is_shield()
        {
            let want = match move_action {
                OrderType::WalkingUpright | OrderType::WalkingWithCorpse => {
                    Some(OrderType::WalkingWithShield)
                }
                OrderType::WalkingWithShield => None,
                OrderType::RunningUpright => None,
                _ => {
                    tracing::warn!(
                        ?owner,
                        ?move_action,
                        "DetermineMovementAnimation: shield action_state with \
                         unrecognised movement action",
                    );
                    None
                }
            };
            if let Some(want) = want
                && let Some(entity) = self.world.entities.get(owner)
                && entity.sprite().has_animation(want)
            {
                move_action = want;
            }
        }
        // Posture forces: non-Upright postures rewrite the action
        // regardless of the action-state inner switch.
        // CARRYING_CORPSE and CROUCHED are pure rewrites;
        // CARRYING_ON_SHOULDERS additionally sets `MoveFlags::REVERSED`
        // on the element flags.  The corpse-lost guard
        // (`WalkingWithCorpse → WalkingUpright`) closes the case where
        // a postponed Move retained `WalkingWithCorpse` after the
        // corpse target was lost — apply that under the Upright arm.
        let mut want_reverse_flag = false;
        match posture_after {
            crate::element::Posture::CarryingCorpse => {
                if let Some(entity) = self.world.entities.get(owner)
                    && entity.sprite().has_animation(OrderType::WalkingWithCorpse)
                {
                    move_action = OrderType::WalkingWithCorpse;
                }
            }
            crate::element::Posture::Crouched => {
                if let Some(entity) = self.world.entities.get(owner)
                    && entity.sprite().has_animation(OrderType::WalkingCrouched)
                {
                    move_action = OrderType::WalkingCrouched;
                }
            }
            crate::element::Posture::CarryingOnShoulders => {
                if let Some(entity) = self.world.entities.get(owner)
                    && entity
                        .sprite()
                        .has_animation(OrderType::WalkingCarryingOnShoulders)
                {
                    move_action = OrderType::WalkingCarryingOnShoulders;
                }
                want_reverse_flag = true;
            }
            crate::element::Posture::Upright => {
                // Inner action-state switch (non-lift Upright): for
                // action states in {Waiting, Bored, Moving,
                // MovingFast, *Bow*, Sleeping, Listening}, normalise
                // STAIRS / CLIMBING_* / CARRYING_ON_SHOULDERS /
                // CROUCHED inbound actions to WalkingUpright or
                // RunningUpright per `is_fast`.  WALKING_STAIRS always
                // normalises to WALKING_UPRIGHT regardless of speed.
                // The PC sword-variant guard is already handled by
                // the upstream sword-variant block + sprite-fallback
                // above.
                let owner_sector = self
                    .world
                    .entities
                    .get(owner)
                    .and_then(|e| e.element_data().sector());
                let on_lift = self.sector_is_lift(owner_sector);
                let inner_arm = matches!(
                    action_after,
                    crate::element::ActionState::Waiting
                        | crate::element::ActionState::Bored
                        | crate::element::ActionState::Moving
                        | crate::element::ActionState::MovingFast
                        | crate::element::ActionState::Sleeping
                        | crate::element::ActionState::Listening
                ) || action_after.is_bow();
                if !on_lift && inner_arm {
                    let walk_or_run = if is_fast {
                        OrderType::RunningUpright
                    } else {
                        OrderType::WalkingUpright
                    };
                    move_action = match move_action {
                        // Pass-through.
                        OrderType::WalkingUpright
                        | OrderType::RunningUpright
                        | OrderType::RiderCharging => move_action,
                        // Stairs always → walking upright.
                        OrderType::WalkingStairs => OrderType::WalkingUpright,
                        // Climbing / carry-on-shoulders → walk/run upright.
                        OrderType::ClimbingWallUp
                        | OrderType::ClimbingWallDown
                        | OrderType::ClimbingLadderUp
                        | OrderType::ClimbingLadderDown
                        | OrderType::ClimbingLadderUpFast
                        | OrderType::ClimbingLadderDownFast
                        | OrderType::ClimbingWallUpFast
                        | OrderType::ClimbingWallDownFast
                        | OrderType::WalkingCarryingOnShoulders => walk_or_run,
                        // Crouched → walk/run upright.
                        OrderType::WalkingCrouched => walk_or_run,
                        // Default arm: leave `move_action` as-is for
                        // any non-listed type (sword/shield variants
                        // are already covered by the upstream blocks).
                        other => other,
                    };
                }
                // Corpse-lost guard.
                if move_action == OrderType::WalkingWithCorpse {
                    move_action = OrderType::WalkingUpright;
                }
            }
            _ => {}
        }
        move_action =
            self.determine_lift_movement_animation(owner, posture_after, move_action, dest);
        // Write the rewritten action back onto the movement sequence
        // element so downstream consumers (refresh-seek, post-process,
        // NPC AI re-reads) see it.  Apply both the action rewrite and
        // the CARRYING_ON_SHOULDERS REVERSED-flag mutation here.
        if let Some(elem) = self
            .orders
            .sequence_manager
            .get_element_mut(seq_id, elem_idx)
            && let crate::sequence::SequenceElementData::Movement { flags, action, .. } =
                &mut elem.data
        {
            *action = move_action;
            if want_reverse_flag {
                *flags |= crate::sequence::MoveFlags::REVERSED;
            }
            if elem.posture_after_transition == crate::element::Posture::Undefined
                && let Some(entity) = self.world.entities.get(owner)
            {
                elem.posture_after_transition = entity.element_data().posture;
            }
        }

        // Read entity position / layer / sector / pathfinder index +
        // current move box + half diagonal (half diagonal drives the
        // thick-reachability pre-check below).
        let (
            mut source,
            entity_layer,
            entity_sector,
            pf_idx,
            mut move_box_map,
            half_diagonal,
            actor_passing_door,
        ) = {
            let entity = match self.world.entities.get(owner) {
                Some(e) => e,
                _ => return MovePathOutcome::ActorGone,
            };
            let elem = entity.element_data();
            let pi = entity.position_iface();
            let pf_idx = {
                let i = pi.get_pathfinder_index();
                if i == u16::MAX { 0 } else { i }
            };
            (
                elem.position_map(),
                elem.layer(),
                elem.sector().map(u16::from).unwrap_or(0),
                pf_idx,
                *pi.get_move_box_map(),
                pi.get_half_diagonal(),
                entity
                    .actor_data()
                    .is_some_and(|actor| actor.active_door_pass.is_some()),
            )
        };

        // legacy implementation RHElementActor::InstructOwner(RHCOMMAND_MOVE) first tries
        // to extract an unauthorized actor from motion obstacles by
        // expanding GetMoveBoxMap() by 0.5 on every side, snapping the
        // actor to the recovered box center, and recomputing position.
        // The later RHPathFinder::AddPathRequest extraction still runs
        // on the unexpanded move box when pathfinding is needed.
        if !self
            .world
            .fast_grid
            .is_position_authorized(&move_box_map, entity_layer)
        {
            let mut box_element = Self::expand_move_box_for_command_extraction(move_box_map);
            if self
                .world
                .fast_grid
                .find_authorized_position(&mut box_element, entity_layer)
            {
                let center = box_element.center();
                source = MapPoint::new(center.x, center.y);
                if let Some(entity) = self.get_entity_mut(owner) {
                    entity.position_iface_mut().set_map_position(source);
                    let elem = entity.element_data_mut();
                    elem.set_position_map(source);
                    elem.update_grid_cell();
                    move_box_map = *entity.position_iface().get_move_box_map();
                }
            }
        }

        // Before queuing a path request, if the move is flagged
        // MAP / STRAIGHT / LINE, or the source→dest segment is
        // thick-reachable, skip the pathfinder entirely and emit a
        // single direct order.  The pathfinder is never invoked when
        // a straight line suffices.
        //
        // LINE is included because C++ routes `AppendMoveToLineToSequence`
        // through gates while building the sequence; the final
        // `RHMOVE_LINE` element is then a direct move to the target line
        // in the already-selected sector.
        //
        // Without this pre-check, short clicks that are directly
        // walkable still hit A*, which can route the actor through
        // source-adjacent graph nodes (extra waypoints around
        // `PassAroundLastNode`) and produce the "keeps moving old
        // direction briefly" click-walk regression.
        let move_flags = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .and_then(|e| match &e.data {
                crate::sequence::SequenceElementData::Movement { flags, .. } => Some(*flags),
                _ => None,
            })
            .unwrap_or(crate::sequence::MoveFlags::empty());
        let is_pass_door = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .is_some_and(|e| e.command == crate::element::Command::PassDoor);
        let source_is_lift_rail = self
            .grid_sector_by_number(crate::sector::SectorNumber::new(entity_sector as i16))
            .and_then(|gs| gs.lift_type)
            .is_some_and(|lt| {
                matches!(
                    lt,
                    crate::sector::LiftType::Wall | crate::sector::LiftType::Ladder
                )
            });
        let straight_ok = move_flags.contains(crate::sequence::MoveFlags::MAP)
            || move_flags.contains(crate::sequence::MoveFlags::STRAIGHT)
            || move_flags.contains(crate::sequence::MoveFlags::LINE)
            || is_pass_door
            || actor_passing_door
            || self
                .world
                .fast_grid
                .is_reachable_thick(source, dest, entity_layer, half_diagonal);

        // Before submitting a path request, check whether the actor's
        // move box is in an authorized position.  This mirrors legacy implementation
        // `RHPathFinder::AddPathRequest`; direct MAP / STRAIGHT /
        // thick-reachable moves do not enter the pathfinder and do
        // not run this extraction gate.
        //
        // If extraction is needed, call `find_authorized_position` to
        // mutate the box to a nearby valid spot, set
        // `use_first_point = true`, and snap the request source to the
        // recovered box centre.  When extraction fails, stop the actor
        // and `Wait` it.
        //
        // Without this snap the downstream strict source-authorization
        // check rejects every candidate and the actor is permanently
        // stuck — A* can't seed.  An earlier fallback only handles
        // the inverse case (source authorized but corridor too thin);
        // this handles "actor must always stay on an authorized
        // position" by pre-snapping the request source.
        let mut use_first_point = false;
        let skip_source_extraction = is_pass_door || actor_passing_door || source_is_lift_rail;
        if !straight_ok
            && !skip_source_extraction
            && !self
                .world
                .fast_grid
                .is_position_authorized(&move_box_map, entity_layer)
        {
            let mut box_element = move_box_map;
            if !self
                .world
                .fast_grid
                .find_authorized_position(&mut box_element, entity_layer)
            {
                // Extraction failed; stop the actor and bail.  Route
                // through `stop_owner` (which clears active sequences
                // and pending path requests for this owner) and
                // launch a `Wait` sequence element at `Wait` priority.
                tracing::warn!(
                    actor = ?owner,
                    src_x = source.x,
                    src_y = source.y,
                    layer = entity_layer,
                    "try_dispatch_move_path: actor cannot be extracted from obstacle (Stop + Wait)",
                );
                self.stop_owner(owner, crate::sequence::SequencePriority::Wait);
                let mut wait_elem = crate::sequence::SequenceElement::new(
                    1,
                    crate::element::Command::Wait,
                    Some(owner),
                );
                wait_elem.priority = crate::sequence::SequencePriority::Wait;
                let mut seq = crate::sequence::Sequence::new();
                seq.append_element(wait_elem);
                self.launch_sequence(seq);
                return MovePathOutcome::Failed;
            }
            let center = box_element.center();
            tracing::info!(
                actor = ?owner,
                old_src_x = source.x,
                old_src_y = source.y,
                new_src_x = center.x,
                new_src_y = center.y,
                "try_dispatch_move_path: extracted source from obstacle (use_first_point=true)",
            );
            source = MapPoint::new(center.x, center.y);
            use_first_point = true;
        }

        let request = PendingPathRequest {
            restored_from_v48: false,
            owner,
            seq_id,
            elem_idx,
            source,
            dest,
            layer: entity_layer,
            sector: entity_sector,
            // Original leaves `uwSector` uninitialized and never reads it.
            // Rust initializes the otherwise dormant serialized member.
            legacy_sector: 0,
            half_diagonal_idx: pf_idx,
            use_first_point,
            move_action,
            speed: if owner_is_pc {
                crate::pathfinder::PathFinderSpeed::Fast
            } else {
                crate::pathfinder::PathFinderSpeed::Medium
            },
            reverse: elem_flags.contains(crate::sequence::MoveFlags::REVERSED),
            tolerance: self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .and_then(|element| match &element.data {
                    crate::sequence::SequenceElementData::Movement { tolerance, .. } => {
                        Some(*tolerance)
                    }
                    _ => None,
                })
                .unwrap_or(0.0),
            antagonist: self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .and_then(|element| match &element.data {
                    crate::sequence::SequenceElementData::Movement { element, .. }
                        if !elem_flags.contains(crate::sequence::MoveFlags::SEEK)
                            || !elem_flags.contains(crate::sequence::MoveFlags::USE_POINT) =>
                    {
                        Some(*element)
                    }
                    crate::sequence::SequenceElementData::Movement { .. } => Some(None),
                    _ => None,
                })
                .flatten(),
            is_pass_door,
            elem_flags,
            sword_movement_context,
            is_fast,
        };

        // `RHElementActor::InstructOwner` completes direct / straight moves
        // immediately, but converts only A*-requiring moves to MOVE_WAITING
        // and queues an `RHpathRequest`.
        if !straight_ok {
            let mut retained_movement_goal = None;
            if let Some(elem) = self
                .orders
                .sequence_manager
                .get_element_mut(seq_id, elem_idx)
            {
                retained_movement_goal = elem.retained_movement_goal;
                elem.command = crate::element::Command::MoveWaiting;
                elem.push_order(crate::order::Order::new(
                    OrderType::Freezing,
                    source.x,
                    source.y,
                    crate::order::alloc_order_id(&mut self.orders.next_order_id),
                ));
            }
            self.orders
                .sequence_manager
                .element_in_progress(seq_id, elem_idx);
            if let Some(goal) = retained_movement_goal
                && let Some(entity) = self.world.entities.get_mut(owner)
                && entity.position_iface().map_goal() == MapPoint::ZERO
            {
                // A pending replacement owns the actor now, but has no
                // concrete waypoint with which to initialize the sprite.
                // Restore the outgoing movement's cached goal only when
                // eager Rust cleanup already erased it. The replacement can
                // be queued before the outgoing actor slot and instructed
                // afterward; in that interval the live movement may advance
                // to another waypoint. Original leaves that newer sprite
                // goal untouched because the interrupted element is no
                // longer selected.
                entity.position_iface_mut().set_map_goal(goal);
            }
            self.orders.pending_path_requests.enqueue(request);
            return MovePathOutcome::Pending;
        }

        self.finish_move_path(sim, request, vec![source, dest])
    }

    pub(super) fn finish_move_path(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        request: PendingPathRequest,
        mut waypoints: Vec<MapPoint>,
    ) -> MovePathOutcome {
        let PendingPathRequest {
            restored_from_v48,
            owner,
            seq_id,
            elem_idx,
            source,
            dest: _,
            layer: entity_layer,
            sector: _,
            legacy_sector: _,
            half_diagonal_idx: _,
            use_first_point,
            move_action,
            speed: _,
            reverse,
            tolerance,
            antagonist,
            is_pass_door,
            elem_flags,
            sword_movement_context,
            is_fast: _,
        } = request;

        // Drunken-soldier path deviation.  Only applies to upright
        // walking/running animations and not to PassDoor commands.
        let is_movement_anim = matches!(
            move_action,
            OrderType::WalkingUpright | OrderType::RunningUpright
        );
        if is_movement_anim && !is_pass_door {
            let blood_alcohol = self
                .world
                .entities
                .get(owner)
                .and_then(|e| e.npc_data())
                .and_then(|n| n.ai_brain.base())
                .map(|b| b.blood_alcohol)
                .unwrap_or(0);
            if blood_alcohol > 0 {
                let (half_diag, move_box) = self
                    .world
                    .entities
                    .get(owner)
                    .map(|e| e.position_iface())
                    .map(|pi| (pi.get_half_diagonal(), *pi.get_move_box()))
                    .unwrap_or_default();
                waypoints = crate::engine::tick::apply_drunken_path_deviation(
                    sim,
                    waypoints,
                    source,
                    blood_alcohol,
                    move_action == OrderType::RunningUpright,
                    entity_layer,
                    &move_box,
                    half_diag,
                    &self.world.fast_grid,
                );
            }
        }

        tracing::trace!(
            actor = ?owner,
            ?seq_id,
            elem_idx,
            wp = waypoints.len(),
            ?move_action,
            ?elem_flags,
            sword_movement_context,
            "try_dispatch_move_path: dispatched {} waypoints to actor",
            waypoints.len(),
        );

        // Build one walking/running order per waypoint.  The final
        // order carries the element's tolerance + antagonist, and
        // every order carries the element's reverse flag.
        //
        // The `antagonist`: when SEEK+USE_POINT the target element is
        // *not* carried on the move (the seek is to a hotspot, not to
        // the antagonist itself); otherwise the movement element's
        // `element` (antagonist) rides along on the final order so
        // downstream consumers (touch-on-Done etc.) can resolve it.
        // ProcessPathRequests only stamps tolerance and antagonist when the
        // raw C++ path contains more than one point. Decide this before
        // removing a leading source waypoint: raw [source, goal] emits one
        // order with metadata, while a direct raw [goal] order keeps the
        // RHOrder constructor defaults.
        let (final_order_tolerance, final_order_antagonist) =
            original_final_path_metadata(waypoints.len(), tolerance, antagonist);

        // `use_first_point` handling: the emission loop starts at
        // index 0 if set, otherwise 1.
        //
        // * `use_first_point == false` — the normal case where the
        //   source was already authorized.  `path[0]` IS the actor's
        //   current position (the pathfinder returns
        //   `[source, ..., goal]` for graph paths), so skip it to
        //   avoid a zero-length first order.  Direct paths return
        //   just `[goal]` (len == 1) and the skip doesn't apply.
        //
        // * `use_first_point == true` — set above when the source
        //   had to be extracted from an obstacle.  `path[0]` is the
        //   snapped source, NOT the actor's current position; keep
        //   it as the first waypoint so the actor walks back to safe
        //   ground before continuing.  (For direct paths this is a
        //   no-op: `[goal]` stays a single waypoint and the actor
        //   walks straight to goal — anti-collision handles the small
        //   obstacle clip on that first leg.)
        if !use_first_point && waypoints.len() > 1 {
            let first = waypoints[0];
            if (first.x - source.x).abs() < f32::EPSILON
                && (first.y - source.y).abs() < f32::EPSILON
            {
                waypoints.remove(0);
            }
        }
        {
            let next_order_id = &mut self.orders.next_order_id;
            if let Some(elem) = self
                .orders
                .sequence_manager
                .get_element_mut(seq_id, elem_idx)
            {
                // Fresh Rust movement elements retain their generated
                // transition prefix through `num_transition_orders`. A
                // restored Original MOVE_WAITING instead owns the exact
                // serialized pre-path queue, whose last waiting order must be
                // reused in place when the saved request completes.
                // ProcessPathRequests marks a resolved movement element as
                // MOVE_OK before installing its path orders. The command is
                // observable by actor execution and condolation logic; it is
                // not merely a pathfinder implementation detail.
                if !is_pass_door {
                    elem.command = crate::element::Command::MoveOk;
                }
                crate::movement::build_orders_from_path(
                    elem,
                    &waypoints,
                    move_action,
                    final_order_tolerance,
                    reverse,
                    final_order_antagonist,
                    next_order_id,
                    restored_from_v48,
                );
            }
        }

        // Splice startup / end transitions into the order queue
        // based on the actor's posture + action state.
        let had_launch_transition = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .is_some_and(|e| e.num_transition_orders > 0 && e.orders.front().is_some());
        let inserted_path_start_transition = self.post_process_path(seq_id, elem_idx);
        // `generate_transition` owns the serialized leading-transition span;
        // `post_process_path` reports its own startup insertion directly.
        // This also covers a short path whose walking order is relabelled in
        // place (no queue-length delta), exactly as C++
        // `InsertTransitionStart` does.
        let have_start_transition = had_launch_transition || inserted_path_start_transition;

        // Update actor state. When a startup transition was prepended,
        // leave `action_state` alone so the transition's `MS::Done`
        // handler flips it on completion. Sword movement without a startup
        // transition also stays WaitingSword until `PerformMotion` returns
        // START in the actor's later execution slot, matching
        // RHElementActorHuman::Execute.
        if let Some(entity) = self.world.entities.get_mut(owner)
            && let Some(actor) = entity.actor_data_mut()
        {
            if !have_start_transition && !sword_movement_context {
                actor.action_state = match move_action {
                    OrderType::RunningUpright => crate::element::ActionState::MovingFast,
                    _ => crate::element::ActionState::Moving,
                };
            }
            actor.active_movement = ActiveMovement::new(seq_id, elem_idx);
            // The outer SEEK translation or RefreshSeek owns the target
            // snapshot and TIME_SEEK_REFRESH assignment. AppendMoveToSequence
            // creates one or more concrete MOVE|SEEK elements without
            // resampling either value, so a route through a door retains the
            // original target reference and accumulated countdown.
            // Mirror the original actor lifecycle flag once the movement
            // element promotes to InProgress.
            actor.sequence_element_started = true;
        }

        // Transition element to InProgress.
        self.orders
            .sequence_manager
            .element_in_progress(seq_id, elem_idx);

        MovePathOutcome::Success
    }
}

#[cfg(test)]
mod orphaned_sword_movement_tests {
    use super::*;
    use crate::element::{
        ActionState, ActorData, ActorPc, Command, ElementData, ElementKind, Entity, HumanData,
        PcData, Posture,
    };
    use crate::order::Order;
    use crate::sequence::{
        MoveFlags, SequenceElement, SequenceElementData, SequencePriority, SequenceState,
    };
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    fn install_sword_movement(
        force: bool,
    ) -> (
        EngineInner,
        EntityId,
        crate::sequence::SequenceId,
        std::num::NonZeroU32,
        MapPoint,
    ) {
        let mut engine = EngineInner::new();
        let start = MapPoint::new(100.0, 100.0);
        let destination = MapPoint::new(140.0, 100.0);
        let mut element = ElementData {
            kind: ElementKind::ActorPc,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        element.set_position_map(start);

        let script = SpriteScript {
            action_id: OrderType::WalkingSword as u16,
            action_done: 0,
            average_speed: 10.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 10,
            frame_ids: vec![1],
            delays: vec![0],
            distances: vec![10],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
        };
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[OrderType::WalkingSword as usize] = 0;
        element.sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script; 16]),
            std::sync::Arc::new(conversion),
        );
        element.sprite.position_iface.set_anti_collision_on(false);
        element.set_position_map(start);

        let owner = engine.add_entity(Entity::Pc(ActorPc {
            element,
            actor: ActorData {
                action_state: ActionState::MovingSword,
                ..ActorData::default()
            },
            human: HumanData::default(),
            pc: PcData {
                life_points: 50,
                ..PcData::default()
            },
        }));

        let order_id = engine.orders.allocate_order_id();
        let mut movement = SequenceElement::new_movement(
            1,
            Command::Move,
            Some(owner),
            OrderType::WalkingWithSword,
        );
        movement.priority = SequencePriority::Normal;
        movement.orders.push_back(Order::new(
            OrderType::WalkingWithSword,
            destination.x,
            destination.y,
            order_id,
        ));
        let SequenceElementData::Movement { flags, .. } = &mut movement.data else {
            unreachable!("new_movement must create movement data")
        };
        if force {
            *flags |= MoveFlags::FORCE_SWORD_MOVEMENT;
        }
        let sequence = engine.orders.sequence_manager.launch_element(movement);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);
        engine
            .get_entity_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .active_movement = ActiveMovement::new(sequence, 0);

        (engine, owner, sequence, order_id, start)
    }

    #[test]
    fn nonforced_sword_movement_without_opponents_aborts_before_motion_and_quits_once() {
        let (mut engine, owner, movement_sequence, order_id, start) = install_sword_movement(false);

        engine.tick_entity_movement(&crate::sim_rng::test_context(), &LevelAssets::new());

        let owner_entity = engine.get_entity(owner).unwrap();
        assert_eq!(
            owner_entity.element_data().position_map(),
            start,
            "the rejected sword movement must not reach PerformMotion"
        );
        assert_ne!(
            owner_entity.element_data().sprite.last_processed_order_id,
            order_id.get(),
            "the rejected order must not initialize the sprite"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(movement_sequence, 0)
                .unwrap()
                .state,
            SequenceState::Impossible
        );
        let quit_count = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .filter(|element| {
                element.owner == Some(owner) && element.command == Command::QuitSwordfight
            })
            .count();
        assert_eq!(
            quit_count, 1,
            "one rejected Execute invocation must launch exactly one QuitSwordfight"
        );
    }

    #[test]
    fn forced_sword_movement_without_opponents_still_performs_motion() {
        let (mut engine, owner, movement_sequence, order_id, start) = install_sword_movement(true);

        engine.tick_entity_movement(&crate::sim_rng::test_context(), &LevelAssets::new());

        let owner_entity = engine.get_entity(owner).unwrap();
        assert_ne!(
            owner_entity.element_data().position_map(),
            start,
            "FORCE_SWORD_MOVEMENT must retain the movement Execute path"
        );
        assert_eq!(
            owner_entity.element_data().sprite.last_processed_order_id,
            order_id.get()
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(movement_sequence, 0)
                .unwrap()
                .state,
            SequenceState::InProgress
        );
        assert!(
            engine
                .orders
                .sequence_manager
                .sequences_iter()
                .flat_map(|sequence| sequence.elements.iter())
                .all(|element| {
                    element.owner != Some(owner) || element.command != Command::QuitSwordfight
                }),
            "forced movement must not launch QuitSwordfight"
        );
    }
}

#[cfg(test)]
mod movement_transition_state_tests {
    use super::*;
    use crate::element::{
        ActionState, ActorData, ActorPc, Command, ElementData, ElementKind, Entity, HumanData,
        PcData, Posture,
    };
    use crate::order::Order;
    use crate::sequence::{SequenceElement, SequencePriority};
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    #[test]
    fn same_action_transition_arrival_applies_terminated_state_before_advancing() {
        let mut engine = EngineInner::new();
        let start = MapPoint::new(100.0, 100.0);
        let first_destination = MapPoint::new(101.5, 100.0);
        let second_destination = MapPoint::new(110.0, 100.0);
        let transition = OrderType::TransitionWaitingUprightRunningUpright;

        let script = SpriteScript {
            action_id: transition as u16,
            action_done: 2,
            average_speed: 1.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 4,
            frame_ids: vec![1, 2, 3, 4],
            delays: vec![0; 4],
            distances: vec![1; 4],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 4],
            sound_ids: vec![0; 4],
        };
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[transition as usize] = 0;

        let mut element = ElementData {
            kind: ElementKind::ActorPc,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        element.sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script; 16]),
            std::sync::Arc::new(conversion),
        );
        element
            .sprite
            .position_iface
            .set_move_box(crate::coordinates::MoveBox::from_coords(
                -4.0, -4.0, 4.0, 4.0,
            ));
        element.sprite.position_iface.set_anti_collision_on(false);
        element.set_position_map(start);

        let owner = engine.add_entity(Entity::Pc(ActorPc {
            element,
            actor: ActorData {
                action_state: ActionState::Waiting,
                ..ActorData::default()
            },
            human: HumanData::default(),
            pc: PcData::default(),
        }));

        let first_order_id = engine.orders.allocate_order_id();
        let second_order_id = engine.orders.allocate_order_id();
        let mut movement = SequenceElement::new_movement(
            1,
            Command::MoveOk,
            Some(owner),
            OrderType::RunningUpright,
        );
        movement.priority = SequencePriority::Normal;
        movement.orders.push_back(Order::new(
            transition,
            first_destination.x,
            first_destination.y,
            first_order_id,
        ));
        movement.orders.push_back(Order::new(
            transition,
            second_destination.x,
            second_destination.y,
            second_order_id,
        ));
        let sequence = engine.orders.sequence_manager.launch_element(movement);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);
        engine
            .get_entity_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .active_movement = ActiveMovement::new(sequence, 0);

        engine.tick_entity_movement(&crate::sim_rng::test_context(), &LevelAssets::new());
        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .action_state,
            ActionState::Waiting,
            "a nonterminal startup-transition tick must retain Waiting"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .unwrap()
                .current_order()
                .unwrap()
                .order_id,
            first_order_id
        );

        engine.tick_entity_movement(&crate::sim_rng::test_context(), &LevelAssets::new());
        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .action_state,
            ActionState::Waiting,
            "the transition remains nonterminal while its turning slowdown leaves the goal ahead"
        );

        engine.tick_entity_movement(&crate::sim_rng::test_context(), &LevelAssets::new());

        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .action_state,
            ActionState::MovingFast,
            "arrival changed InProgress to Terminated after PerformMotion, so the transition Execute side effect must observe the final state"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .unwrap()
                .current_order()
                .unwrap()
                .order_id,
            second_order_id,
            "the terminated transition must advance to its same-action successor"
        );
    }
}

#[cfg(test)]
mod path_request_timing_tests {
    use super::*;
    use crate::entity_id::{PcId, SoldierId};

    fn request(owner: EntityId, speed: crate::pathfinder::PathFinderSpeed) -> PendingPathRequest {
        PendingPathRequest {
            restored_from_v48: false,
            owner,
            seq_id: crate::sequence::SequenceId(1),
            elem_idx: 0,
            source: MapPoint::new(10.0, 10.0),
            dest: MapPoint::new(20.0, 20.0),
            layer: 0,
            sector: 0,
            legacy_sector: 0,
            half_diagonal_idx: 0,
            use_first_point: false,
            move_action: OrderType::WalkingUpright,
            speed,
            reverse: false,
            tolerance: 0.0,
            antagonist: None,
            is_pass_door: false,
            elem_flags: crate::sequence::MoveFlags::empty(),
            sword_movement_context: false,
            is_fast: false,
        }
    }

    fn advance_fake_frame(queue: &mut PendingPathRequestQueue) -> Option<EntityId> {
        let completed = queue
            .take_completed()
            .map(|processed| processed.request.owner);
        if let Some(request) = queue.pop_to_start() {
            queue.set_in_flight(request, Some(Vec::new()));
        }
        completed
    }

    #[test]
    fn original_two_request_special_case_keeps_first_request_first() {
        let npc = EntityId::Soldier(SoldierId(1));
        let pc = EntityId::Pc(PcId(2));
        let mut queue = PendingPathRequestQueue::default();

        // AddPathRequest appends unconditionally while fewer than two entries
        // exist, so this later FAST PC does not overtake the first MEDIUM NPC.
        queue.enqueue(request(npc, crate::pathfinder::PathFinderSpeed::Medium));
        queue.enqueue(request(pc, crate::pathfinder::PathFinderSpeed::Fast));

        assert_eq!(
            advance_fake_frame(&mut queue),
            None,
            "frame 1 only starts NPC"
        );
        assert_eq!(advance_fake_frame(&mut queue), Some(npc), "frame 2");
        assert_eq!(advance_fake_frame(&mut queue), Some(pc), "frame 3");
        assert_eq!(advance_fake_frame(&mut queue), None, "frame 4 is empty");
    }

    #[test]
    fn resolves_one_per_frame_in_original_priority_and_in_flight_order() {
        let npc_1 = EntityId::Soldier(SoldierId(1));
        let npc_2 = EntityId::Soldier(SoldierId(2));
        let pc_1 = EntityId::Pc(PcId(3));
        let pc_2 = EntityId::Pc(PcId(4));
        let mut queue = PendingPathRequestQueue::default();

        queue.enqueue(request(npc_1, crate::pathfinder::PathFinderSpeed::Medium));
        queue.enqueue(request(npc_2, crate::pathfinder::PathFinderSpeed::Medium));
        queue.enqueue(request(pc_1, crate::pathfinder::PathFinderSpeed::Fast));

        let mut completed_by_frame = Vec::new();
        assert_eq!(advance_fake_frame(&mut queue), None, "frame 1 starts pc_1");
        completed_by_frame.push((2, advance_fake_frame(&mut queue).unwrap()));

        // Frame 2 returns pc_1 and starts npc_1. A new FAST request can
        // overtake queued npc_2, but cannot displace in-flight npc_1.
        queue.enqueue(request(pc_2, crate::pathfinder::PathFinderSpeed::Fast));
        completed_by_frame.push((3, advance_fake_frame(&mut queue).unwrap()));
        completed_by_frame.push((4, advance_fake_frame(&mut queue).unwrap()));
        completed_by_frame.push((5, advance_fake_frame(&mut queue).unwrap()));

        assert_eq!(
            completed_by_frame,
            vec![(2, pc_1), (3, npc_1), (4, pc_2), (5, npc_2)]
        );
        assert_eq!(advance_fake_frame(&mut queue), None, "frame 6 is empty");
    }

    #[test]
    fn movement_context_expires_live_failure_only_after_100_frames() {
        let owner = EntityId::Pc(PcId(7));
        let mut world = WorldState::new();
        let mut orders = OrderRuntime::new();
        let mut element = crate::sequence::SequenceElement::new(
            1,
            crate::element::Command::MoveWaiting,
            Some(owner),
        );
        element.priority = crate::sequence::SequencePriority::Normal;
        let sequence_id = orders.sequence_manager.launch_element(element);
        let element = orders
            .sequence_manager
            .get_element_mut(sequence_id, 0)
            .expect("launched movement element");
        element.state = crate::sequence::SequenceState::InProgress;
        element.command = crate::element::Command::MoveWaiting;
        orders
            .failed_path_requests
            .push(FailedPathRequest::synthetic(owner, sequence_id, 0, 10));

        let at_boundary =
            MovementContext::new(110, &mut world, &mut orders).take_expired_failures();
        assert!(at_boundary.is_empty());
        assert_eq!(orders.failed_path_requests.len(), 1);

        let after_boundary =
            MovementContext::new(111, &mut world, &mut orders).take_expired_failures();
        assert_eq!(after_boundary.len(), 1);
        assert_eq!(after_boundary[0].request.owner, owner);
        assert_eq!(after_boundary[0].age, 101);
        assert!(
            !after_boundary[0].owner_is_pc,
            "missing entity is not fabricated"
        );
        assert!(orders.failed_path_requests.is_empty());
    }
}

#[cfg(test)]
mod line_jump_tests {
    use super::*;
    use crate::element::Command;
    use crate::sequence::{Field, FieldValue, MoveFlags, SequenceElementData};

    #[test]
    fn line_jump_click_sequence_moves_to_line_then_jumps_then_moves_to_click() {
        let owner = EntityId::Pc(crate::entity_id::PcId(7));
        let source_idx = crate::jump_line::JumpLineIndex::new(2).unwrap();
        let dest_idx = crate::jump_line::JumpLineIndex::new(3).unwrap();
        let mut source_line = crate::jump_line::JumpLine::new(
            crate::coordinates::map_pt(10.0, 20.0),
            crate::coordinates::map_pt(30.0, 20.0),
            0.0,
            0.0,
        );
        source_line.layer = 4;

        let seq = build_line_jump_click_sequence(
            owner,
            OrderType::RunningUpright,
            source_idx,
            &source_line,
            dest_idx,
            crate::coordinates::map_pt(90.0, 120.0),
            5,
            1.0,
        );

        assert_eq!(seq.elements.len(), 3);
        assert_eq!(seq.elements[0].command, Command::Move);
        match &seq.elements[0].data {
            SequenceElementData::Movement {
                destination,
                layer,
                line_id,
                flags,
                ..
            } => {
                assert_eq!((destination.x, destination.y), (20.0, 20.0));
                assert_eq!(*layer, 4);
                assert_eq!(*line_id, Some(source_idx));
                assert!(flags.contains(MoveFlags::LINE));
                assert!(flags.contains(MoveFlags::TO_JUMP));
            }
            other => panic!("expected movement element, got {other:?}"),
        }

        assert_eq!(seq.elements[1].command, Command::JumpCmd);
        assert!(matches!(
            seq.elements[1].get_property(Field::JumplineSource),
            Some(FieldValue::LineId(idx)) if *idx == source_idx
        ));
        assert!(matches!(
            seq.elements[1].get_property(Field::JumplineDestination),
            Some(FieldValue::LineId(idx)) if *idx == dest_idx
        ));

        assert_eq!(seq.elements[2].command, Command::Move);
        match &seq.elements[2].data {
            SequenceElementData::Movement {
                destination,
                layer,
                flags,
                line_id,
                ..
            } => {
                assert_eq!((destination.x, destination.y), (90.0, 120.0));
                assert_eq!(*layer, 5);
                assert!(flags.is_empty());
                assert_eq!(*line_id, None);
            }
            other => panic!("expected final movement element, got {other:?}"),
        }
    }

    #[test]
    fn force_sword_movement_marks_all_movement_elements() {
        let owner = EntityId::Pc(crate::entity_id::PcId(7));
        let source_idx = crate::jump_line::JumpLineIndex::new(2).unwrap();
        let dest_idx = crate::jump_line::JumpLineIndex::new(3).unwrap();
        let source_line = crate::jump_line::JumpLine::new(
            crate::coordinates::map_pt(10.0, 20.0),
            crate::coordinates::map_pt(30.0, 20.0),
            0.0,
            0.0,
        );

        let mut seq = build_line_jump_click_sequence(
            owner,
            OrderType::WalkingUpright,
            source_idx,
            &source_line,
            dest_idx,
            crate::coordinates::map_pt(90.0, 120.0),
            5,
            1.0,
        );

        force_sword_movement_for_sequence(&mut seq);

        let movement_flags: Vec<_> = seq
            .elements
            .iter()
            .filter_map(|elem| match &elem.data {
                SequenceElementData::Movement { flags, .. } => Some(*flags),
                _ => None,
            })
            .collect();

        assert_eq!(movement_flags.len(), 2);
        assert!(
            movement_flags
                .iter()
                .all(|flags| flags.contains(MoveFlags::FORCE_SWORD_MOVEMENT))
        );
    }

    #[test]
    fn running_with_sword_uses_distance_motion() {
        assert_eq!(
            sword_movement_dispatch_action(OrderType::WalkingUpright),
            OrderType::WalkingWithSword
        );
        assert_eq!(
            sword_movement_dispatch_action(OrderType::WalkingWithCorpse),
            OrderType::WalkingWithSword
        );
        assert_eq!(
            sword_movement_dispatch_action(OrderType::RunningUpright),
            OrderType::RunningWithSword
        );
        assert!(order_uses_distance_motion(OrderType::RunningWithSword));
        assert!(order_uses_distance_motion(OrderType::WalkingWithSword));
        assert!(order_uses_distance_motion(OrderType::WalkingSword));
        assert!(!order_uses_distance_motion(
            OrderType::TransitionRunningUprightWaitingUpright
        ));
        assert!(!order_uses_distance_motion(
            OrderType::TransitionSpecialWaitingUpright
        ));
    }

    #[test]
    fn turn_minimum_is_applied_after_movement_speed_factor() {
        let distance = scaled_motion_distance(2.0, 0.582_163_33, true, true);
        assert_eq!(
            distance, 0.7,
            "Original scales 2.0 by the patrol factor, then applies the 0.6 turn slowdown and 0.7 minimum"
        );
        assert_eq!(
            scaled_motion_distance(2.0, 0.582_163_33, true, false),
            1.164_326_7
        );
        assert_eq!(
            scaled_motion_distance(2.0, 0.25, false, false),
            2.0,
            "transition motion does not use the movement element's speed factor"
        );
    }

    #[test]
    fn movement_execute_state_effects_match_transition_execute() {
        use crate::element::{ActionState, Posture};
        use crate::sprite::MotionState;

        assert_eq!(
            movement_execute_state_effect(
                OrderType::TransitionSpecialWaitingUpright,
                MotionState::Done
            ),
            Some((Posture::Upright, ActionState::Waiting))
        );
        assert_eq!(
            movement_execute_state_effect(
                OrderType::TransitionWaitingUprightWalkingUpright,
                MotionState::Terminated
            ),
            Some((Posture::Upright, ActionState::Waiting))
        );
        assert_eq!(
            movement_execute_state_effect(OrderType::WalkingUpright, MotionState::Start),
            Some((Posture::Upright, ActionState::Moving))
        );
        assert_eq!(
            movement_execute_state_effect(
                OrderType::TransitionWaitingUprightBoredWaitingUpright,
                MotionState::Done
            ),
            Some((Posture::Upright, ActionState::Waiting))
        );
        assert_eq!(
            movement_execute_state_effect(
                OrderType::TransitionWaitingUprightWaitingUprightBored,
                MotionState::Done
            ),
            Some((Posture::Upright, ActionState::Bored))
        );
        assert_eq!(
            movement_execute_state_effect(OrderType::TransitionCrouchingDown, MotionState::Done),
            Some((Posture::Crouched, ActionState::Waiting))
        );
        assert_eq!(
            movement_execute_state_effect(
                OrderType::TransitionLeaningOutWaitingAlerted,
                MotionState::Done
            ),
            Some((Posture::Upright, ActionState::Waiting))
        );
    }

    #[test]
    fn transition_distance_first_execute_is_consumed_once() {
        let mut continuation = true;

        assert!(take_transition_distance_first_execute(&mut continuation));
        assert!(!continuation);
        assert!(!take_transition_distance_first_execute(&mut continuation));

        let mut continuation = true;
        assert!(take_transition_distance_first_execute(&mut continuation));
        assert!(
            !continuation,
            "the marker belongs to the first Execute slot, even when that slot hides START"
        );
        assert!(!take_transition_distance_first_execute(&mut continuation));
    }

    #[test]
    fn deferred_movement_state_start_promotes_only_the_successor_handoff() {
        let mut deferred = true;

        assert!(take_deferred_movement_state_start(&mut deferred));
        assert!(!deferred);
        assert!(
            !take_deferred_movement_state_start(&mut deferred),
            "the synthetic START is a one-shot order handoff"
        );
    }

    #[test]
    fn entity_seek_hides_initial_sword_motion_start_from_execute() {
        use crate::sprite::MotionState;

        assert_eq!(
            movement_execute_visible_motion(
                OrderType::RunningWithSword,
                MotionState::Start,
                false,
                true,
            ),
            MotionState::InProgress,
            "entity-target PerformSeek returns IN_PROGRESS around the sprite's START"
        );
        assert_eq!(
            movement_execute_state_effect(
                OrderType::RunningWithSword,
                movement_execute_visible_motion(
                    OrderType::RunningWithSword,
                    MotionState::Start,
                    false,
                    true,
                ),
            ),
            None,
            "the Human Execute switch must retain WaitingSword"
        );
        assert_eq!(
            movement_execute_visible_motion(
                OrderType::RunningWithSword,
                MotionState::Start,
                false,
                false,
            ),
            MotionState::Start,
            "point and ordinary movement expose the sprite's START"
        );
        assert_eq!(
            movement_execute_visible_motion(
                OrderType::RunningUpright,
                MotionState::Start,
                false,
                true,
            ),
            MotionState::Start,
            "running upright sets MovingFast after PerformSeek unconditionally"
        );
    }

    #[test]
    fn exact_goal_motion_does_not_wait_for_nonzero_animation_speed() {
        assert!(
            !stationary_motion_waits(0.0, false, 0.0),
            "an exact-position walk must reach the shared arrival tail on its first Execute"
        );
        assert!(
            stationary_motion_waits(0.0, false, 1.0),
            "a stationary motion away from its goal must remain current"
        );
        assert!(
            !stationary_motion_waits(0.0, true, 1.0),
            "a pre-motion seek-tolerance arrival must complete without displacement"
        );
    }

    #[test]
    fn entity_seek_hides_transition_done_until_wrapper_termination() {
        use crate::element::ActionState;
        use crate::sprite::MotionState;

        let visible = movement_execute_visible_motion(
            OrderType::TransitionRunningUprightWalkingUpright,
            MotionState::Done,
            false,
            true,
        );
        assert_eq!(visible, MotionState::InProgress);
        assert_eq!(
            movement_execute_state_effect(
                OrderType::TransitionRunningUprightWalkingUpright,
                visible,
            ),
            None,
            "raw sprite DONE must not change a live entity seek from MovingFast to Moving"
        );
        assert_eq!(
            movement_execute_visible_motion(
                OrderType::TransitionRunningUprightWalkingUpright,
                MotionState::Terminated,
                false,
                true,
            ),
            MotionState::Terminated,
            "the Execute switch must observe the seek wrapper's terminal result"
        );
        assert_eq!(
            movement_execute_visible_motion(
                OrderType::TransitionRunningUprightWalkingUpright,
                MotionState::Done,
                false,
                false,
            ),
            MotionState::Done,
            "ordinary and point movement expose the sprite's DONE result"
        );
        assert_eq!(
            movement_execute_state_effect(
                OrderType::TransitionRunningUprightWalkingUpright,
                MotionState::Terminated,
            ),
            Some((crate::element::Posture::Upright, ActionState::Moving))
        );
    }

    #[test]
    fn entity_seek_refresh_countdown_preserves_original_unsigned_wrap() {
        assert_eq!(age_seek_refresh_wait(25), 24);
        assert_eq!(age_seek_refresh_wait(0), u32::MAX);
        assert_eq!(age_seek_refresh_wait(u32::MAX), u32::MAX - 1);
    }

    #[test]
    fn final_path_metadata_depends_on_raw_waypoint_count() {
        let antagonist = Some(EntityId::Pc(crate::entity_id::PcId(9)));

        assert_eq!(
            original_final_path_metadata(1, 28.9, antagonist),
            (0.0, None)
        );
        assert_eq!(
            original_final_path_metadata(2, 28.9, antagonist),
            (28.9, antagonist)
        );
    }

    #[test]
    fn only_explicit_in_place_movement_transitions_accept_zero_target() {
        assert!(is_in_place_movement_transition(
            OrderType::TransitionSpecialWaitingUpright
        ));
        assert!(is_in_place_movement_transition(
            OrderType::TransitionWaitingUprightSpecial
        ));
        assert!(is_in_place_movement_transition(
            OrderType::TransitionWaitingUprightBoredWaitingUpright
        ));
        assert!(is_in_place_movement_transition(
            OrderType::TransitionWaitingUprightWaitingUprightBored
        ));
        assert!(is_in_place_movement_transition(
            OrderType::TransitionCrouchingUp
        ));
        assert!(is_in_place_movement_transition(
            OrderType::TransitionCrouchingDown
        ));
        assert!(is_in_place_movement_transition(
            OrderType::TransitionSittingWaitingUpright
        ));
        assert!(is_in_place_movement_transition(
            OrderType::TransitionLeaningOutWaitingAlerted
        ));
        assert!(is_in_place_movement_transition(
            OrderType::TransitionClimbingWallDownWaitingUpright
        ));
        assert!(is_in_place_movement_transition(OrderType::StandingUp));
        assert!(is_in_place_movement_transition(OrderType::StandingUpSword));
        assert!(is_in_place_movement_transition(OrderType::StandingUpBow));
        assert!(is_in_place_movement_transition(OrderType::LoweringShield));
        assert!(!is_in_place_movement_transition(
            OrderType::TransitionWaitingUprightWalkingUpright
        ));
        assert!(!is_in_place_movement_transition(
            OrderType::TransitionWalkingUprightWaitingUpright
        ));
    }

    #[test]
    fn elevation_crossing_matches_null_obstacle_side() {
        assert_eq!(
            EngineInner::crossed_elevation_obstacle(None, None, Some(50)),
            Some(Some(50))
        );
        assert_eq!(
            EngineInner::crossed_elevation_obstacle(Some(50), None, Some(50)),
            Some(None)
        );
        assert_eq!(
            EngineInner::crossed_elevation_obstacle(Some(49), Some(49), Some(50)),
            Some(Some(50))
        );
        assert_eq!(
            EngineInner::crossed_elevation_obstacle(Some(99), Some(49), Some(50)),
            None
        );
    }

    #[test]
    fn command_extraction_expands_move_box_like_original() {
        let bbox = MapBBox::from_coords(10.0, 20.0, 30.0, 40.0);
        let expanded = EngineInner::expand_move_box_for_command_extraction(bbox);
        assert_eq!(expanded.x_min(), 9.5);
        assert_eq!(expanded.y_min(), 19.5);
        assert_eq!(expanded.x_max(), 30.5);
        assert_eq!(expanded.y_max(), 40.5);
    }

    #[test]
    fn face_opponent_uses_original_displacement_to_facing_angle_sign() {
        use crate::element::ActionState;

        let right = combat_movement_angle(0.0, std::f32::consts::FRAC_PI_2);
        assert_eq!(
            combat_directional_animation(ActionState::MovingSword, right),
            OrderType::StrafingRightSword,
            "Angle(eastward displacement, northward facing) is +90 degrees"
        );

        let left = combat_movement_angle(0.0, -std::f32::consts::FRAC_PI_2);
        assert_eq!(
            combat_directional_animation(ActionState::MovingSword, left),
            OrderType::StrafingLeftSword,
            "reversing the facing vector selects the opposite strafe"
        );
    }

    #[test]
    fn face_opponent_direction_uses_isometric_map_aspect() {
        use crate::position_interface::{
            ASPECT_RATIO, vector_to_sector_0_to_15, vector_to_sector_0_to_15_iso,
        };

        let bare = vector_to_sector_0_to_15(1.0, ASPECT_RATIO);
        let isometric = vector_to_sector_0_to_15_iso(1.0, ASPECT_RATIO);
        assert_eq!(
            isometric, 6,
            "isometric stretch restores a 45-degree vector"
        );
        assert_ne!(
            bare, isometric,
            "raw map-space binning must not replace GetSector0to15(ASPECT_RATIO)"
        );
    }
}
