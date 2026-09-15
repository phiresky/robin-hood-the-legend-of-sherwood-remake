//! Movement ticking, pathfinding dispatch, and order processing.

pub(super) use super::door_pass::DoorPassAdvance;
use super::door_pass::{
    completed_door_pass_to_commit, discard_lazy_door_pass_following_orders,
    door_pass_eager_posture, door_pass_sprite_animation_override, insert_door_pass_successor,
    materialize_door_action_point_prefix, synchronize_selected_door_pass_walk_action,
};
use super::*;
use crate::coordinates::{MapBBox, MapPoint, MapVec};
#[cfg(test)]
use crate::element::ActiveDoorPass;
use crate::element::EntityId;
use crate::order::OrderType;
use crate::position_interface::vector_to_sector_0_to_15;
use crate::sprite::{FrameProgression, MotionMethod, MotionOrderContext, MotionState};

mod combat_motion;
mod diagnostics;
mod door_traversal;
pub(crate) use door_traversal::GateRouteRequest;
mod elevation;
mod formation;
mod path_scheduling;
mod rider_charge;
mod routing;
// Phase methods of `tick_one_movement_actor`. The file lives next to
// `movement.rs` (not in `movement/`), but is a child module so it can use
// the movement-private helpers without widening their visibility.
#[path = "movement_step.rs"]
mod movement_step;
use movement_step::MovementStepCtx;

pub(in crate::engine) use formation::PlannedRecordedGroupMoveOutcome;

use combat_motion::{
    combat_directional_animation, combat_movement_angle, executes_shield_movement_action,
    executes_sword_movement_action, is_sword_motion_context, is_sword_movement_nonanimation,
    sword_movement_dispatch_action,
};
use rider_charge::{is_galopp_decision_frame, rider_charge_point_in_quad};

use diagnostics::debug_post_seek_handoff_enabled;

/// Per-owner lift translation snapshot consumed by movement Execute's live
/// animation derivation. Covers the lift cases of
/// movement-animation selection.
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

/// Direct movement dispatch predicate. Line movement changes post-processing
/// of the resulting path; it does not by itself bypass pathfinding.
#[inline]
fn movement_flags_force_direct_dispatch(flags: crate::sequence::MoveFlags) -> bool {
    flags.contains(crate::sequence::MoveFlags::MAP)
        || flags.contains(crate::sequence::MoveFlags::STRAIGHT)
}

#[inline]
fn movement_path_dispatch_is_direct(
    flags: crate::sequence::MoveFlags,
    goal_crosses_layer: bool,
    post_door_route_handoff: bool,
    current_layer_reachable: bool,
) -> bool {
    movement_flags_force_direct_dispatch(flags)
        || (!(goal_crosses_layer && !post_door_route_handoff) && current_layer_reachable)
}

/// Detect the re-entrant door handoff where a postponed Move is translated
/// before the route continuation registered by the completed PassDoor.
///
/// The topology only tells us that the movement element's retained goal layer
/// may be stale across the door transit. It must never substitute for
/// An unreachable current-layer handoff still enters the pathfinder.
fn has_deferred_post_door_route_continuation(
    manager: &crate::sequence::SequenceManager,
    owner: EntityId,
    source: MapPoint,
    entity_layer: u16,
) -> bool {
    use crate::element::Command;
    use crate::sequence::{SequenceElementData, SequenceState};

    manager
        .deferred_elements_to_go()
        .into_iter()
        .any(|(route_id, move_idx)| {
            let Some(sequence) = manager.get_sequence(route_id) else {
                return false;
            };
            let Some(route_move) = sequence.get(move_idx) else {
                return false;
            };
            if move_idx < 2
                || route_move.owner != Some(owner)
                || route_move.command != Command::Move
                || route_move.state != SequenceState::Todo
            {
                return false;
            }

            let route_assert = &sequence.elements[move_idx - 1];
            let pass_door = &sequence.elements[move_idx - 2];
            if route_assert.owner != Some(owner)
                || route_assert.command != Command::AssertPosition
                || route_assert.state != SequenceState::Terminated
                || pass_door.owner != Some(owner)
                || pass_door.command != Command::PassDoor
                || pass_door.state != SequenceState::Terminated
            {
                return false;
            }

            let assert_at_source = matches!(
                &route_assert.data,
                SequenceElementData::Movement { destination, .. } if *destination == source
            );
            let pass_exits_here = matches!(
                &pass_door.data,
                SequenceElementData::Movement {
                    destination,
                    layer,
                    gate_id: Some(_),
                    ..
                } if *destination == source && *layer == entity_layer
            );
            assert_at_source && pass_exits_here
        })
}

/// This gate applies to every path request; command type and actor posture do
/// not provide bypasses.
#[inline]
fn path_request_needs_source_extraction(direct_dispatch: bool, source_authorized: bool) -> bool {
    !direct_dispatch && !source_authorized
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ActorPostSeekInteraction {
    Hit,
    Tie,
    Untie,
}

/// Identify an actor-owned interaction whose init-time 40-unit validity
/// guard immediately follows a completed entity Seek.
///
/// The copied terminal movement can lose its own target, but the Original
/// retains one seek-target reference on the actor. Scope the early handoff to
/// a post-seek interaction with that exact antagonist; an unrelated tail must
/// remain on the ordinary sequence-manager path.
fn actor_post_seek_interaction(
    actor: &crate::element::ActorData,
) -> Option<ActorPostSeekInteraction> {
    let element = actor
        .post_seek_sequence
        .as_ref()
        .and_then(|sequence| sequence.elements.first())?;
    let antagonist = match &element.data {
        crate::sequence::SequenceElementData::Interaction { antagonist } => *antagonist,
        _ => None,
    }?;
    if actor.seek_target != Some(antagonist) {
        return None;
    }
    match element.command {
        crate::element::Command::HitCmd => Some(ActorPostSeekInteraction::Hit),
        crate::element::Command::TieCmd => Some(ActorPostSeekInteraction::Tie),
        crate::element::Command::Untie => Some(ActorPostSeekInteraction::Untie),
        _ => None,
    }
}

/// Hit and Tie initialization compare the raw map-space squared distance with
/// 1600.
/// Keep the strict comparison: a victim at exactly 40 units is valid.
fn interaction_exceeds_init_range(owner: MapPoint, victim: MapPoint) -> bool {
    let dx = victim.x - owner.x;
    let dy = victim.y - owner.y;
    dx * dx + dy * dy > 1600.0
}

/// Input passed to a ladder/wall lift's action translation.
///
/// The original game's movement-animation selection passes an authored upright walk/run
/// action through verbatim: the lift itself maps `RunningUpright` to its fast
/// climb row, independently of the fast-movement flag. Rust can also reach this point
/// with a carried movement variant, where the element's speed flag remains the
/// useful normalization signal.
#[inline]
fn climb_lift_translation_input(action: OrderType, is_fast: bool) -> OrderType {
    match action {
        OrderType::WalkingUpright | OrderType::RunningUpright => action,
        OrderType::WalkingWithSword
        | OrderType::RunningWithSword
        | OrderType::WalkingWithShield
        | OrderType::WalkingCrouched
        | OrderType::WalkingWithCorpse => {
            if is_fast {
                OrderType::RunningUpright
            } else {
                OrderType::WalkingUpright
            }
        }
        other => other,
    }
}

/// Apply the lift-sector portion of movement-animation selection.
///
/// The current actor sector is authoritative. In particular, an actor leaving
/// a lift translates its movement action before the door callback changes the
/// sector, while an actor approaching the lift from outside does not.
pub(super) fn grid_sector_for_position_handle(
    level: &crate::fast_find_grid::LevelGrid,
    sector: crate::position_interface::SectorHandle,
) -> Option<&crate::fast_find_grid::GridSector> {
    match sector.arena_index() {
        Some(index) => Some(level.sectors.get(usize::from(index)).unwrap_or_else(|| {
            panic!(
                "sector {} carries missing exact arena index {}",
                sector.get(),
                index.get()
            )
        })),
        None => {
            let number = crate::sector::SectorNumber::new(i16::from(sector));
            level
                .sector_number_map
                .get(&number)
                .and_then(|&index| level.sectors.get(index))
        }
    }
}

pub(super) fn lift_endpoint_points_for_sector(
    sector: &crate::fast_find_grid::GridSector,
) -> (MapPoint, MapPoint) {
    let low = sector.low_exit_point.unwrap_or_else(|| {
        panic!(
            "movement animation selection: lift sector {} missing low exit point",
            sector.sector_number
        )
    });
    let high = sector.high_exit_point.unwrap_or_else(|| {
        panic!(
            "movement animation selection: lift sector {} missing high exit point",
            sector.sector_number
        )
    });
    (low, high)
}

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
        elem.posture()
    } else {
        posture_after
    };
    let Some(sector_handle) = elem.sector() else {
        return action;
    };
    // The original game samples the actor's live sector reference. Public sector numbers
    // are not unique, so retain the arena identity carried by the saved position;
    // number lookup is only the compatibility path for identity-less saves.
    let Some(sector) = grid_sector_for_position_handle(&fast_grid.level, sector_handle) else {
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
                    "movement animation selection: climb posture does not match lift sector"
                );
                return action;
            }
            let (low, high) = lift_endpoint_points_for_sector(sector);
            let position = elem.position_map();
            let ladder_dx = low.x - high.x;
            let ladder_dy = low.y - high.y;
            let movement_dx = destination.x - position.x;
            let movement_dy = destination.y - position.y;
            let going_down = ladder_dx * movement_dx + ladder_dy * movement_dy >= 0.0;
            lift_type.translate_climb_action(action, going_down)
        }
        // The default posture arm still applies the lift's upright action
        // translation. This matters for
        // resumed PassDoor elements whose serialized transition result is a
        // non-movement posture such as Lying: while the live actor is already
        // upright in the lift, that dormant result remains stamped on the
        // element and the stairs action must still be selected.
        Posture::CarryingCorpse
        | Posture::Crouched
        | Posture::CarryingOnShoulders
        | Posture::HelpingToClimb
        | Posture::SimulatingBeggar => action,
        _ => lift_type.translate_upright_action(action),
    }
}

/// Mobile geometry sampled at one actor's live creation-order slot.
///
/// Unlike the other immutable movement preparation, this must not escape the
/// owner boundary: an actor before a mobile sees its previous position and an
/// actor after the mobile sees the geometry translated by that master's
/// update.
struct LiveMobileGeometry {
    mobile_lines_by_layer: std::collections::BTreeMap<u16, Vec<crate::fast_find_grid::GridLine>>,
    mobile_points_by_layer: std::collections::BTreeMap<u16, Vec<crate::repulsive::RepulsivePoint>>,
    mobile_polygons_by_layer:
        std::collections::BTreeMap<u16, Vec<Vec<crate::coordinates::MapPoint>>>,
}

#[derive(Clone, Copy, Debug)]
struct RiderChargeExecution {
    /// Identity of the same order object after Execute returned. Rider charge
    /// may legitimately assign that object a fresh ID on its last animation
    /// frame. `None` means a synchronous callback replaced the entry object
    /// while Execute was still running.
    completion_order_id: Option<std::num::NonZeroU32>,
}

/// State seen when the post-Execute line-crossing boundary opens for `owner`.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PostExecuteCrossingObservation {
    pub owner: EntityId,
    /// Identity and action of the owner's current order at that point.
    pub current_order: Option<(std::num::NonZeroU32, OrderType)>,
}

/// A fixed front-order replacement applied when `owner` reaches the
/// post-Execute line-crossing boundary.
///
/// Stands in for a synchronous line-crossing callback that replaces the entry
/// order after Execute returned. That state cannot be staged before the tick
/// (Execute itself rewrites the same order object), and no fixture drives a
/// real script line crossing yet.
// TODO: replace with a script-line-crossing fixture whose callback issues the
// replacement, then delete this injection.
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(super) struct PostExecuteOrderReplacement {
    pub owner: EntityId,
    pub seq_id: crate::sequence::SequenceId,
    pub elem_idx: usize,
    pub expected_order_type: OrderType,
    pub order_type: OrderType,
    pub order_id: std::num::NonZeroU32,
}

#[cfg(test)]
thread_local! {
    static MOBILE_CROSSING_INCREMENTS: super::test_support::Probe<MapVec> =
        const { super::test_support::Probe::new() };
    static POST_EXECUTE_CROSSINGS: super::test_support::Probe<PostExecuteCrossingObservation> =
        const { super::test_support::Probe::new() };
    static POST_EXECUTE_ORDER_REPLACEMENT: std::cell::RefCell<Option<PostExecuteOrderReplacement>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(super) fn capture_mobile_crossing_increments<T>(f: impl FnOnce() -> T) -> (T, Vec<MapVec>) {
    MOBILE_CROSSING_INCREMENTS.with(|increments| increments.capture(f))
}

#[cfg(test)]
fn observe_mobile_crossing_increment(increment: MapVec) {
    MOBILE_CROSSING_INCREMENTS.with(|increments| increments.record(increment));
}

#[cfg(test)]
pub(super) fn capture_post_execute_crossings<T>(
    f: impl FnOnce() -> T,
) -> (T, Vec<PostExecuteCrossingObservation>) {
    POST_EXECUTE_CROSSINGS.with(|crossings| crossings.capture(f))
}

#[cfg(test)]
pub(super) fn install_post_execute_order_replacement(replacement: PostExecuteOrderReplacement) {
    POST_EXECUTE_ORDER_REPLACEMENT.with(|slot| {
        assert!(
            slot.borrow_mut().replace(replacement).is_none(),
            "post-Execute order replacement must not already be installed"
        );
    });
}

#[cfg(test)]
fn observe_post_execute_crossing(engine: &mut EngineInner, entity_id: EntityId) {
    POST_EXECUTE_CROSSINGS.with(|crossings| {
        crossings.record_with(|| PostExecuteCrossingObservation {
            owner: entity_id,
            current_order: engine
                .orders
                .sequence_manager
                .current_order_for_actor(&engine.world.entities, entity_id)
                .map(|(_, _, order)| (order.order_id, order.order_type)),
        });
    });
    let replacement = POST_EXECUTE_ORDER_REPLACEMENT.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot
            .as_ref()
            .is_some_and(|replacement| replacement.owner == entity_id)
        {
            slot.take()
        } else {
            None
        }
    });
    if let Some(replacement) = replacement {
        let order = engine
            .orders
            .sequence_manager
            .get_element_mut(replacement.seq_id, replacement.elem_idx)
            .and_then(|element| element.orders.front_mut())
            .expect("post-Execute replacement retains the selected element");
        assert_eq!(order.order_type, replacement.expected_order_type);
        order.order_type = replacement.order_type;
        order.order_id = replacement.order_id;
    }
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub(super) struct MovementOwnerSelection {
    pub seq_id: crate::sequence::SequenceId,
    pub elem_idx: usize,
    pub order_id: std::num::NonZeroU32,
}

pub(super) fn order_uses_distance_motion(order: OrderType) -> bool {
    matches!(
        order,
        OrderType::WalkingUpright
            | OrderType::WalkingCrouched
            | OrderType::WalkingAlerted
            | OrderType::RunningUpright
            | OrderType::WalkingWithSword
            | OrderType::RunningWithSword
            | OrderType::WalkingWithShield
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

#[inline]
fn refresh_pc_walking_shield_after_execute(
    entity: &mut crate::element::Entity,
    profiles: &crate::profiles::ProfileManager,
    order_action: OrderType,
) {
    if entity.is_pc() && order_action == OrderType::WalkingWithShield {
        crate::bow_shot::refresh_retained_shield_obstacle(entity, profiles);
    }
}

/// Movement actions which turn immediately before beginning motion.
///
/// This distinction remains observable under `FreezeAll`: the sprite returns
/// an in-progress result before animation or displacement, but the actor-side
/// turn has already happened.
fn order_turns_before_motion(order: OrderType) -> bool {
    order_uses_distance_motion(order)
        || matches!(
            order,
            OrderType::TransitionWalkingUprightWaitingUpright
                | OrderType::TransitionRunningUprightWaitingUpright
                | OrderType::TransitionWaitingUprightWalkingUpright
                | OrderType::TransitionWaitingUprightRunningUpright
                | OrderType::TransitionWalkingUprightRunningUpright
                | OrderType::TransitionRunningUprightWalkingUpright
                | OrderType::TransitionWaitingCrouchedWalkingCrouched
                | OrderType::TransitionWalkingCrouchedWaitingCrouched
                | OrderType::TransitionWalkingCrouchedWalkingUpright
                | OrderType::TransitionWalkingUprightWalkingCrouched
                | OrderType::TransitionWalkingCrouchedRunningUpright
                | OrderType::TransitionRunningUprightWalkingCrouched
        )
}

/// Scale the sprite-frame distance by the movement element's speed factor
/// before applying the turn slowdown and its
/// minimum useful step. Direct transition orders process motion without
/// the element speed factor, while seek transitions pass it explicitly.
pub(super) fn scaled_motion_distance(
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

/// Lift-wall and ladder orders are direct dispatch actions, including their
/// start/landing transitions. A split door
/// route can retain a stale mirror of the preceding climb action, but that
/// mirror must not make the selected transition fall back to an action-state
/// walk animation.
#[inline]
fn literal_lift_sprite_action(action: OrderType) -> Option<OrderType> {
    climb_lift_type(action).map(|_| action)
}

#[inline]
fn door_type_uses_lift_climb_direction(door_type: crate::gate::DoorType) -> bool {
    matches!(
        door_type,
        crate::gate::DoorType::LiftHigh
            | crate::gate::DoorType::LiftHighCrenel
            | crate::gate::DoorType::LiftLow
    )
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

/// Fast ladder/wall Execute arms return immediately when their first
/// motion step terminates. Running on stairs also executes two motion
/// calls per tick, but its loop deliberately has no such early return.
fn fast_climb_stops_after_first_termination(action: OrderType) -> bool {
    matches!(
        action,
        OrderType::ClimbingWallUpFast
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
        // The fast non-animation climbing tokens are dispatch /
        // pathfinder speed tokens. Actor handling processes them by
        // playing the normal climb animation row with the running motion method.
        OrderType::RunningStairs => OrderType::WalkingStairs,
        OrderType::ClimbingWallUpFast => OrderType::ClimbingWallUp,
        OrderType::ClimbingWallDownFast => OrderType::ClimbingWallDown,
        OrderType::ClimbingLadderUpFast => OrderType::ClimbingLadderUp,
        OrderType::ClimbingLadderDownFast => OrderType::ClimbingLadderDown,
        other => other,
    }
}

/// Whether an actor climb order applies the lift's fixed facing.
///
/// This occurs only during initialization. A climb order reached recursively
/// after a door step
/// can therefore start without replacing the facing inherited from that
/// transition.
fn initialising_climb_uses_lift_direction(
    action: OrderType,
    lift_type: crate::sector::LiftType,
    initialising: bool,
) -> bool {
    initialising
        && matches!(
            (action, lift_type),
            (
                OrderType::ClimbingWallUp
                    | OrderType::ClimbingWallDown
                    | OrderType::ClimbingWallUpFast
                    | OrderType::ClimbingWallDownFast,
                crate::sector::LiftType::Wall
            ) | (
                OrderType::ClimbingLadderUp
                    | OrderType::ClimbingLadderDown
                    | OrderType::ClimbingLadderUpFast
                    | OrderType::ClimbingLadderDownFast,
                crate::sector::LiftType::Ladder
            )
        )
}

/// Whether a terminal translated door transition still has an authoritative
/// PassDoor owner when the runtime `ActiveDoorPass` mirror is absent.
///
/// Restored Original saves can carry the complete translated order chain in
/// the serialized PassDoor sequence without reconstructing that Rust-only
/// mirror. The caller treats that serialized chain as ownership while the
/// actor's saved position-state door supplies geometry when needed. The
/// PC crenel climb-up and ladder-down exits are also recoverable without
/// either representation because their original-game execution arms only publish
/// actor state before advancing to `PASSING_DOOR`.
pub(super) fn pass_door_transition_completion_has_owner(
    command: crate::element::Command,
    has_materialized_or_restored_door_pass: bool,
    action: OrderType,
    is_pc: bool,
) -> bool {
    has_materialized_or_restored_door_pass
        || (command == crate::element::Command::PassDoor
            && matches!(
                action,
                OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel if is_pc
            ))
        || (command == crate::element::Command::PassDoor
            && matches!(
                action,
                OrderType::TransitionClimbingLadderDownWaitingUpright
                    | OrderType::TransitionClimbingLadderDownWaitingUprightAlerted
            ))
}

pub(super) fn door_click_polygon_at(doors: &[crate::gate::Door], click: MapPoint) -> Option<u32> {
    doors
        .iter()
        .enumerate()
        .find(|(_, door)| door.is_door() && door.click_polygon_contains(click.x, click.y))
        .map(|(idx, _)| idx as u32)
}

pub(super) fn movement_execute_state_effect(
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
        (OT::TransitionWalkingCrouchedWaitingCrouched, MS::Done | MS::Terminated) => {
            Some((P::Crouched, AS::Waiting))
        }
        (
            OT::TransitionWaitingCrouchedWalkingCrouched
            | OT::TransitionWalkingUprightWalkingCrouched
            | OT::TransitionRunningUprightWalkingCrouched,
            MS::Done | MS::Terminated,
        ) => Some((P::Crouched, AS::Moving)),
        (OT::TransitionWalkingCrouchedWalkingUpright, MS::Done | MS::Terminated) => {
            Some((P::Upright, AS::Moving))
        }
        (OT::TransitionWalkingCrouchedRunningUpright, MS::Done | MS::Terminated) => {
            Some((P::Upright, AS::MovingFast))
        }
        (
            OT::TransitionWaitingUprightRunningUpright | OT::TransitionWalkingUprightRunningUpright,
            MS::Done | MS::Terminated,
        ) => Some((P::Upright, AS::MovingFast)),
        (OT::TransitionRunningUprightWalkingUpright, MS::Done | MS::Terminated) => {
            Some((P::Upright, AS::Moving))
        }
        (
            OT::WalkingUpright | OT::WalkingAlerted | OT::WalkingStairs | OT::RunningStairs,
            MS::Start,
        ) => Some((P::Upright, AS::Moving)),
        // The crouched walk starts the actor moving without standing it
        // up; only the PC executes this animation.
        (OT::WalkingCrouched, MS::Start) => Some((P::Crouched, AS::Moving)),
        // Unlike the neighboring walk/stairs arms, Original stamps this
        // state unconditionally after seeking or motion processing. A fresh
        // short run can therefore return Terminated without ever exposing
        // Start and must still leave the actor MovingFast.
        (OT::RunningUpright, _) => Some((P::Upright, AS::MovingFast)),
        (OT::WalkingWithSword, MS::Start) => Some((P::Upright, AS::MovingSword)),
        (OT::RunningWithSword, MS::Start) => Some((P::Upright, AS::MovingFastSword)),
        // The PC WalkingWithShield Execute arm stamps MovingShield after
        // every seeking or motion result, then replaces it with
        // HoldingShield when that result is TERMINATED.
        (OT::WalkingWithShield, MS::Terminated) => Some((P::Upright, AS::HoldingShield)),
        (OT::WalkingWithShield, _) => Some((P::Upright, AS::MovingShield)),
        (OT::WalkingWithCorpse, MS::Start) => Some((P::CarryingCorpse, AS::Moving)),
        (OT::WalkingWithCorpse, MS::Terminated) => Some((P::CarryingCorpse, AS::Waiting)),
        (OT::ClimbingWallUp | OT::ClimbingWallDown, MS::Start) => Some((P::OnWall, AS::Moving)),
        (
            OT::TransitionWaitingUprightClimbingLadderUp
            | OT::TransitionWaitingUprightClimbingLadderUpAlerted,
            MS::Done | MS::Terminated,
        ) => Some((P::OnLadder, AS::Moving)),
        _ => None,
    }
}

/// The shipped game clears an anti-vibration deviation latch when an in-place
/// movement startup transition starts. Both PCs and NPCs distinguish the two
/// upright handoffs: an actual waiting sprite entering a
/// waiting-to-walking/running startup retires the preceding movement's latch,
/// while a walking-to-waiting exit preserves it for the following `Turn`.
/// The action-state translator can also generate the same startup token while
/// a non-waiting action (for example `RaisingShield`) is still displayed; that
/// case does not retire the latch in the Original.
#[inline]
fn should_clear_deviated_for_aligned_transition_start(
    _is_pc: bool,
    execute_order_initialising: bool,
    is_transition_anim: bool,
    order_action: OrderType,
    previous_sprite_action: OrderType,
    deviated: bool,
    position: MapPoint,
    goal: MapPoint,
) -> bool {
    execute_order_initialising
        && is_transition_anim
        && matches!(
            order_action,
            OrderType::TransitionWaitingUprightWalkingUpright
                | OrderType::TransitionWaitingUprightRunningUpright
        )
        && matches!(
            previous_sprite_action,
            OrderType::WaitingUpright
                | OrderType::WaitingUprightBored
                | OrderType::WaitingUprightBoredRandom
                | OrderType::WaitingAlerted
        )
        && deviated
        && position == goal
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
    speed <= 0.0 && !tolerance_arrival && (distance > f32::EPSILON || !distance.is_finite())
}

#[inline]
fn motion_recomputes_exact_position(
    is_transition: bool,
    has_map_target: bool,
    speed: f32,
    distance: f32,
) -> bool {
    is_transition && has_map_target && speed > 0.0 && distance <= f32::EPSILON
}

/// Apply the forecast update after nonzero motion displacement.
/// Transition-distance orders use a separate commit path from ordinary walking,
/// though both
/// through the same forecast update before its arrival check.
fn refresh_motion_forecast(
    sprite: &mut crate::sprite::Sprite,
    speed: f32,
    split_motion_speeds: Option<(f32, f32)>,
) {
    if sprite.position_iface.is_blocked() {
        return;
    }

    // Fast movement processes motion twice. Each nonzero step updates
    // the forecast, so the second distance wins when it moved; otherwise the
    // first call's forecast remains live.
    let forecast_distance = match split_motion_speeds {
        Some((_, second)) if second != 0.0 => second,
        Some((first, _)) => first,
        None => speed,
    };
    if forecast_distance == 0.0 {
        return;
    }

    let wait = sprite.wait_time(sprite.current_row, sprite.current_frame);
    sprite
        .position_iface
        .update_forecasted_movement(forecast_distance, wait + 1);
}

/// Original only performs the exact zero-tolerance goal snap from the
/// post-movement arrival branches. An order which starts at its goal is
/// consumed without rewriting the actor's coordinates.
#[inline]
fn should_snap_arrival(
    arrived_after_committed_step: bool,
    tolerance_arrival: bool,
    order_tolerance: f32,
    deviated: bool,
) -> bool {
    arrived_after_committed_step && !tolerance_arrival && order_tolerance == 0.0 && !deviated
}

fn both_sword_ranges_contain_distance(
    distance: f32,
    my_maximal: u16,
    my_uber: u16,
    opponent_maximal: u16,
    opponent_uber: u16,
) -> bool {
    let between =
        |maximal: u16, uber: u16| f32::from(maximal) < distance && distance <= f32::from(uber);
    between(my_maximal, my_uber) && between(opponent_maximal, opponent_uber)
}

/// Does the step this execution is about to commit reach the goal?
///
/// Motion processing moves first and only then asks the position interface
/// whether the goal is reached, so a call that would otherwise return `START`
/// can return `TERMINATED` instead. Rust stages the physical step until after
/// the sprite call, so the answer has to be projected on a throwaway copy of
/// the position interface, anti-collision and all. Comparing the straight-line
/// distance against the step length is not a substitute: the predicate is a
/// tolerance-compared dot product against the movement increment, and a step
/// deviated around another actor both leaves that line and rebuilds the
/// increment it is measured against.
fn projected_step_reaches_goal(
    position_iface: &crate::position_interface::PositionInterface,
    mover: Option<&super::anti_collision::CollisionMover>,
    collision: super::anti_collision::CollisionWorld<'_>,
    static_repulsive_points: &[crate::ai::RepulsivePoint],
    mobile: &LiveMobileGeometry,
    grid: &crate::fast_find_grid::FastFindGrid,
    goal: MapPoint,
    target: Option<crate::position_interface::TargetInfo>,
    speed: f32,
) -> bool {
    if speed == 0.0 {
        return false;
    }
    let mut projected = position_iface.clone();
    let increment = projected.get_increment_map();
    let anti_on = projected.is_anti_collision_on();
    let (dx_step, dy_step, recovered_from_deviation, rebuild_after_deviation) =
        if anti_on && let Some(mover) = mover.filter(|mover| mover.active) {
            let move_box = *projected.get_move_box();
            let half_diagonal = projected.get_half_diagonal();
            let was_deviated = projected.is_deviated();
            let mut state = super::anti_collision::AntiCollisionState {
                pi: &mut projected,
                move_box,
                half_diagonal,
                goal_map: goal,
            };
            let (dx_step, dy_step) = super::anti_collision::apply_anti_collision_step(
                mover,
                collision,
                static_repulsive_points,
                mobile
                    .mobile_points_by_layer
                    .get(&mover.layer)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
                mobile
                    .mobile_lines_by_layer
                    .get(&mover.layer)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
                mobile
                    .mobile_polygons_by_layer
                    .get(&mover.layer)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
                Some(grid),
                Some(&mut state),
                increment.x,
                increment.y,
                speed,
                anti_on,
            );
            (
                dx_step,
                dy_step,
                was_deviated && !state.pi.is_deviated(),
                state.pi.is_deviated() && state.pi.blocked_count == 0,
            )
        } else {
            (increment.x * speed, increment.y * speed, false, false)
        };
    let mut projected_position = projected.map_position();
    projected_position.x += dx_step;
    projected_position.y += dy_step;
    projected.set_map_position(projected_position);
    // A committed deviation invalidates the cached increment and rebuilds it
    // from the new position toward the same goal, and the arrival predicate
    // that follows reads the rebuilt vector. Skipping the rebuild leaves the
    // dot product measuring against the pre-deviation heading, which is how a
    // sidestepped walker looked as if it had already arrived.
    if rebuild_after_deviation && (dx_step != 0.0 || dy_step != 0.0) {
        projected.reset_increment_computed();
        projected.compute_increment_all(false);
    } else if recovered_from_deviation {
        projected.reset_increment_computed();
        projected.compute_increment_all(true);
    }
    projected.is_goal_reached(grid, target)
}

/// Motion state observed by the original game after seek movement.
///
/// Entity-target seeking consumes non-terminal sprite results and returns
/// `IN_PROGRESS`; point seeks return the raw result. This matters because the
/// caller's Execute switch must not observe either a raw `START` or `DONE`
/// while the seek wrapper remains active. Running upright is deliberately
/// excluded: its original-game execution sets fast movement unconditionally after
/// seeking, irrespective of the returned motion state.
///
/// The ordinary arrival branch outranks the sprite's own result: once the
/// committed step satisfies the goal predicate, motion processing returns
/// `TERMINATED` whatever it was about to report, so a walk that merely
/// continued (`IN_PROGRESS`) or finished its action frame (`DONE`) still
/// reaches the Execute arm as a termination.
fn movement_execute_visible_motion(
    order: OrderType,
    motion: MotionState,
    reaches_goal_this_step: bool,
    entity_target_seek: bool,
) -> MotionState {
    if reaches_goal_this_step {
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

#[derive(Clone, Debug, Default)]
pub(super) struct MovementOwnerMotion {
    pub initial: Option<MotionState>,
    pub post_completion_override: Option<MotionState>,
}

fn committed_arrival_post_completion_override(
    raw_sprite_motion: MotionState,
    visible_execute_motion: MotionState,
    reaches_goal_this_step: bool,
) -> Option<MotionState> {
    (reaches_goal_this_step && raw_sprite_motion != visible_execute_motion)
        .then_some(visible_execute_motion)
}

/// The original game's wait timer uses an unsigned 32-bit counter. A stationary
/// entity seek deliberately wraps zero to `UINT_MAX`; the signed refresh gate
/// then continues to regard the wrapped values as elapsed.
#[inline]
fn age_seek_refresh_wait(wait: u32) -> u32 {
    wait.wrapping_sub(1)
}

/// Number of times the original game invokes seek movement for
/// one execution of this movement order when seeking is enabled.
///
/// The flag alone is not enough: authored wall and ladder orders retain it
/// while their execution arms process motion directly. Conversely,
/// Running on stairs performs the seek step twice.
#[inline]
pub(super) fn perform_seek_calls_per_execute(order: OrderType) -> u32 {
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

/// Prepare the raw pathfinder points for movement-order post-processing.
///
/// Path processing starts its order loop at index one whenever the first point
/// is not used. Do not re-check that
/// the first point equals the request source here: legacy floating-point
/// equality is not the gate, and a source poisoned with NaNs must still be
/// skipped rather than becoming a live movement order.
///
/// Returns the raw count because final-order tolerance and antagonist
/// metadata depend on the pre-skip path exactly as they do in Original.
fn prepare_path_waypoints_for_postprocess(
    waypoints: &mut Vec<MapPoint>,
    use_first_point: bool,
) -> usize {
    let raw_waypoint_count = waypoints.len();
    if !use_first_point && waypoints.len() > 1 {
        waypoints.remove(0);
    }
    raw_waypoint_count
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

/// Shape of the goal passed to [`EngineInner::launch_gate_movement_sequence`].
///
/// Unifies the three goal flavours (point, door, line) into a single
/// builder; the function switches on this enum to pick the right
/// trailing-step shape.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub(crate) enum GoalShape {
    /// Point-goal. The actor walks to this map point after the last gate,
    /// retaining the caller's arrival tolerance (notably for AI approaches).
    Point { point: MapPoint, tolerance: f32 },
    /// Entity-target seek goal.  The trailing MOVE keeps the target
    /// element, SEEK flag, and tolerance so arrival uses the same
    /// live target-distance predicate as a plain `Command::Seek`.
    Seek {
        point: MapPoint,
        target: EntityId,
        tolerance: f32,
    },
    /// Direct entity-target route built by target interaction.
    /// Unlike `Seek`, the target pointer is retained on each ordinary MOVE
    /// but seeking is disabled and the actor's seek-refresh state is not
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

#[inline]
pub(crate) fn building_exit_wait_frames(sim: &crate::sim_rng::SimulationContext) -> u32 {
    crate::sim_rng::u32(sim, crate::sim_rng::RngSite::RuntimeBuildingExitWait, 0..16)
        + crate::sim_rng::u32(sim, crate::sim_rng::RngSite::RuntimeBuildingExitWait, 0..16)
}

fn route_sector_by_exact_handle(
    engine: &EngineInner,
    sector: crate::position_interface::SectorHandle,
) -> Option<&crate::fast_find_grid::GridSector> {
    grid_sector_for_position_handle(&engine.world.fast_grid.level, sector)
}

fn ai_move_goal_door(
    engine: &EngineInner,
    goal_sector: crate::position_interface::SectorHandle,
    goal_sector_index: Option<crate::fast_find_grid::SectorIndex>,
) -> Option<crate::gate::DoorIndex> {
    let exact_goal_sector =
        goal_sector_index.map_or(goal_sector, |index| goal_sector.with_arena_index(index));
    route_sector_by_exact_handle(engine, exact_goal_sector)
        .filter(|sector| sector.sector_type.is_door())
        .and_then(|sector| sector.door_index)
        .and_then(crate::gate::DoorIndex::new)
}

/// Timeout queue entry for a Move/Seek element whose pathfind failed.
/// When the pathfinder returns no path, the request is stamped with the
/// current universal frame counter and pushed onto this list.  After
/// 100 frames the element transitions to `Impossible` (and, for PCs,
/// the "unable to do something" speech line fires).
///
/// This is **not** a retry queue: the path is not re-dispatched during the
/// 100-frame window. The element sits waiting (no orders, so the actor's idle
/// animation drives) until it is cancelled (halt / postpone) or the timeout
/// elapses.
#[derive(
    Debug,
    Clone,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub(crate) struct FailedPathRequest {
    pub(crate) owner: EntityId,
    pub(crate) seq_id: crate::sequence::SequenceId,
    pub(crate) elem_idx: usize,
    /// Universal frame counter at failure time.  Ages out at
    /// `first_fail_frame + 100`.
    pub(crate) first_fail_frame: u32,
    /// Exact path-request payload retained by the original game's timeout list.
    pub(crate) request: PendingPathRequest,
}

impl FailedPathRequest {
    pub(crate) fn from_pending(request: PendingPathRequest, first_fail_frame: u32) -> Self {
        Self {
            owner: request.owner,
            seq_id: request.seq_id,
            elem_idx: request.elem_idx,
            first_fail_frame,
            request,
        }
    }
}

/// Snapshot of one legacy path request waiting for A*.
///
/// Direct / straight moves never enter this queue. Requests that do need A*
/// snapshot their dispatch inputs here, then [`PathScheduleContext`] resolves at
/// most one request at the designated path-processing point per frame.
#[derive(
    Debug,
    Clone,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
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
    /// Despite the field name, this is the actor's
    /// sector number and is converted to a graph-area index during A*.
    pub(crate) sector: u16,
    /// Exact serialized sector value. Request creation does not initialize
    /// this member and pathfinding never reads it, but v48
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

impl PendingPathRequest {
    pub(in crate::engine) fn references_entity(&self, id: EntityId) -> bool {
        self.owner == id || self.antagonist == Some(id)
    }

    #[cfg(test)]
    pub(crate) fn test_request(
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> Self {
        Self {
            restored_from_v48: false,
            owner,
            seq_id,
            elem_idx,
            source: MapPoint::new(10.0, 10.0),
            dest: MapPoint::new(20.0, 20.0),
            layer: 0,
            sector: 0,
            legacy_sector: 0,
            half_diagonal_idx: 0,
            use_first_point: false,
            move_action: OrderType::WalkingUpright,
            speed: crate::pathfinder::PathFinderSpeed::Medium,
            reverse: false,
            tolerance: 0.0,
            antagonist: None,
            is_pass_door: false,
            elem_flags: crate::sequence::MoveFlags::empty(),
            sword_movement_context: false,
            is_fast: false,
        }
    }
}

fn parity_path_request_state(
    fast_grid: &crate::fast_find_grid::FastFindGrid,
    request: &PendingPathRequest,
) -> crate::pathfinder::ParityPathRequest {
    let half_diagonal = fast_grid
        .try_move_box_half_diagonal(usize::from(request.half_diagonal_idx))
        .unwrap_or_else(|| {
            panic!(
                "path request for {:?} references missing half-diagonal index {}",
                request.owner, request.half_diagonal_idx
            )
        });
    crate::pathfinder::ParityPathRequest {
        actor: request.owner,
        antagonist: request.antagonist,
        layer: request.layer,
        area: request.sector,
        source: request.source,
        goal: request.dest,
        half_diagonal_index: request.half_diagonal_idx,
        half_diagonal,
        animation: request.move_action as u32,
        reverse: request.reverse,
        speed: request.speed as u8,
        tolerance: request.tolerance,
        use_first_point: request.use_first_point,
    }
}

#[derive(
    Debug,
    Clone,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
struct ProcessedPathRequest {
    request: PendingPathRequest,
    waypoints: Option<Vec<MapPoint>>,
}

#[derive(Debug)]
pub(crate) struct ParityPendingPathRequest {
    pub(crate) request: crate::pathfinder::ParityPathRequest,
    pub(crate) sequence_id: crate::sequence::SequenceId,
    pub(crate) element_index: usize,
    pub(crate) in_flight: bool,
    pub(crate) waypoints: Option<Vec<MapPoint>>,
}

/// Legacy path-request ordering plus the pathfinder's in-flight result.
///
/// Path request insertion leaves queues of length zero or one alone.
/// From length two onward it stably sorts by speed, except that the in-flight
/// entry cannot be displaced. The original WAITING branch starts work but
/// returns no result; a later READY call delivers it and starts the next
/// request. `in_flight` preserves that one-call latency.
#[derive(
    Debug,
    Clone,
    Default,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub(crate) struct PendingPathRequestQueue {
    waiting: Vec<PendingPathRequest>,
    in_flight: Option<ProcessedPathRequest>,
    /// Cancellation marker for the logical list head. Cancelling it
    /// head does not remove it; its eventual result remains observable but is
    /// delivered with `valid=false` and consumes the call's one result slot.
    #[serde(default)]
    ignore_next_path: bool,
}

impl PendingPathRequestQueue {
    /// Restore the exact post-save FIFO. The Original writer excludes an
    /// ignored/in-flight head and writes the remaining list in order, so every
    /// deserialized request is waiting and no completion result is present.
    pub(crate) fn restore_v48_waiting(waiting: Vec<PendingPathRequest>) -> Self {
        Self {
            waiting,
            in_flight: None,
            ignore_next_path: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn v48_waiting(&self) -> &[PendingPathRequest] {
        &self.waiting
    }

    pub(crate) fn has_in_flight(&self) -> bool {
        self.in_flight.is_some()
    }

    pub(crate) fn parity_state(
        &self,
        fast_grid: &crate::fast_find_grid::FastFindGrid,
    ) -> (bool, Vec<ParityPendingPathRequest>) {
        let mut requests =
            Vec::with_capacity(self.waiting.len() + usize::from(self.in_flight.is_some()));
        if let Some(processed) = &self.in_flight {
            requests.push(ParityPendingPathRequest {
                request: parity_path_request_state(fast_grid, &processed.request),
                sequence_id: processed.request.seq_id,
                element_index: processed.request.elem_idx,
                in_flight: true,
                // Parity projection only (consumed by the original-state
                // snapshot in `tick.rs` / `movement.rs::parity_state`
                // callers, not by the scheduler). The original keeps a
                // diagnostic in-flight slot whose waypoint list is always
                // an array: a failed completed search stores an EMPTY list
                // there, never "no list". `processed.waypoints == None` is
                // this port's failed-search marker, so the documented legacy
                // shape of that state is `Some(vec![])`, and the runtime
                // `None` is untouched so the scheduler still dispatches its
                // real failure outcome. Projecting `None` here would change
                // the snapshot shape (`null` vs `[]`) against original traces.
                waypoints: Some(processed.waypoints.clone().unwrap_or_default()),
            });
        }
        requests.extend(self.waiting.iter().map(|request| ParityPendingPathRequest {
            request: parity_path_request_state(fast_grid, request),
            sequence_id: request.seq_id,
            element_index: request.elem_idx,
            in_flight: false,
            waypoints: None,
        }));
        (self.ignore_next_path, requests)
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

    fn take_completed(&mut self) -> Option<(ProcessedPathRequest, bool)> {
        let processed = self.in_flight.take()?;
        let valid = !std::mem::take(&mut self.ignore_next_path);
        Some((processed, valid))
    }

    fn pop_to_start(&mut self) -> Option<PendingPathRequest> {
        (!self.waiting.is_empty()).then(|| self.waiting.remove(0))
    }

    fn set_in_flight(&mut self, request: PendingPathRequest, waypoints: Option<Vec<MapPoint>>) {
        debug_assert!(self.in_flight.is_none());
        self.in_flight = Some(ProcessedPathRequest { request, waypoints });
    }

    pub(super) fn remove_entity(&mut self, owner: EntityId) {
        let in_flight_is_owner = self
            .in_flight
            .as_ref()
            .is_some_and(|processed| processed.request.references_entity(owner));
        let waiting_head_is_owner = self.in_flight.is_none()
            && self
                .waiting
                .first()
                .is_some_and(|request| request.references_entity(owner));

        // Entity teardown follows the same path cancellation timing as an
        // interrupted movement element: the logical head stays in the queue,
        // is delivered invalid, and consumes this barrier's result slot.
        // Only later requests involving the removed actor disappear immediately.
        if in_flight_is_owner || waiting_head_is_owner {
            self.ignore_next_path = true;
        }
        let first_waiting = usize::from(waiting_head_is_owner);
        self.waiting = self
            .waiting
            .drain(..)
            .enumerate()
            .filter_map(|(index, request)| {
                (index < first_waiting || !request.references_entity(owner)).then_some(request)
            })
            .collect();
    }

    /// Cancelling the list head
    /// marks its eventual result stale instead of removing it, while later
    /// requests for the same actor are deleted immediately. The retained head
    /// still occupies one path-request processing result slot.
    pub(crate) fn cancel_for_owner(&mut self, owner: EntityId) {
        let head_owner = self
            .in_flight
            .as_ref()
            .map(|processed| processed.request.owner)
            .or_else(|| self.waiting.first().map(|request| request.owner));
        if head_owner == Some(owner) {
            self.ignore_next_path = true;
        }

        // The Original scans from logical list index 1 and deletes only the
        // first later request for this actor. With an in-flight head every
        // waiting entry starts at logical index 1; otherwise waiting[0] is the
        // retained head.
        let first_waiting = usize::from(self.in_flight.is_none());
        if let Some(relative) = self
            .waiting
            .get(first_waiting..)
            .and_then(|waiting| waiting.iter().position(|request| request.owner == owner))
        {
            self.waiting.remove(first_waiting + relative);
        }
    }

    fn first_for_owner_mut(&mut self, owner: EntityId) -> Option<&mut PendingPathRequest> {
        if self
            .in_flight
            .as_ref()
            .is_some_and(|processed| processed.request.owner == owner)
        {
            return self
                .in_flight
                .as_mut()
                .map(|processed| &mut processed.request);
        }
        self.waiting
            .iter_mut()
            .find(|request| request.owner == owner)
    }

    /// Make the first request for this actor fast.
    pub(super) fn make_fast(&mut self, owner: EntityId, pathfinder_index: u16) {
        let Some(request) = self.first_for_owner_mut(owner) else {
            return;
        };
        request.move_action = match request.move_action {
            OrderType::RunningWithSword
            | OrderType::RunningUpright
            | OrderType::ClimbingLadderUpFast
            | OrderType::ClimbingLadderDownFast
            | OrderType::ClimbingWallUpFast
            | OrderType::ClimbingWallDownFast => request.move_action,
            OrderType::WalkingUpright
            | OrderType::WalkingCrouched
            | OrderType::WalkingWithShield => {
                request.half_diagonal_idx = pathfinder_index;
                OrderType::RunningUpright
            }
            OrderType::WalkingWithSword => {
                request.half_diagonal_idx = pathfinder_index;
                OrderType::RunningWithSword
            }
            OrderType::ClimbingLadderUp => {
                request.half_diagonal_idx = pathfinder_index;
                OrderType::ClimbingLadderUpFast
            }
            OrderType::ClimbingLadderDown => {
                request.half_diagonal_idx = pathfinder_index;
                OrderType::ClimbingLadderDownFast
            }
            OrderType::ClimbingWallUp => {
                request.half_diagonal_idx = pathfinder_index;
                OrderType::ClimbingWallUpFast
            }
            OrderType::ClimbingWallDown => {
                request.half_diagonal_idx = pathfinder_index;
                OrderType::ClimbingWallDownFast
            }
            action => {
                panic!("make_fast received unsupported pending action {action:?} for {owner:?}")
            }
        };
    }

    /// Make the first request for this actor slow.
    pub(super) fn make_slow(&mut self, owner: EntityId, pathfinder_index: u16) {
        let Some(request) = self.first_for_owner_mut(owner) else {
            return;
        };
        request.move_action = match request.move_action {
            OrderType::WalkingUpright
            | OrderType::WalkingCrouched
            | OrderType::ClimbingLadderUp
            | OrderType::ClimbingLadderDown
            | OrderType::ClimbingWallUp
            | OrderType::ClimbingWallDown => request.move_action,
            OrderType::RunningUpright => OrderType::WalkingUpright,
            OrderType::RunningWithSword => OrderType::WalkingWithSword,
            OrderType::ClimbingLadderUpFast => {
                request.half_diagonal_idx = pathfinder_index;
                OrderType::ClimbingLadderUp
            }
            OrderType::ClimbingLadderDownFast => {
                request.half_diagonal_idx = pathfinder_index;
                OrderType::ClimbingLadderDown
            }
            OrderType::ClimbingWallUpFast => {
                request.half_diagonal_idx = pathfinder_index;
                OrderType::ClimbingWallUp
            }
            OrderType::ClimbingWallDownFast => {
                request.half_diagonal_idx = pathfinder_index;
                OrderType::ClimbingWallDown
            }
            action => {
                panic!("make_slow received unsupported pending action {action:?} for {owner:?}")
            }
        };
    }

    /// Make the first request for this actor upright.
    pub(super) fn make_upright(&mut self, owner: EntityId, pathfinder_index: u16) {
        let Some(request) = self.first_for_owner_mut(owner) else {
            return;
        };
        request.move_action = match request.move_action {
            OrderType::WalkingUpright
            | OrderType::RunningUpright
            | OrderType::ClimbingLadderUp
            | OrderType::ClimbingLadderDown
            | OrderType::ClimbingWallUp
            | OrderType::ClimbingWallDown
            | OrderType::ClimbingLadderUpFast
            | OrderType::ClimbingLadderDownFast
            | OrderType::ClimbingWallUpFast
            | OrderType::ClimbingWallDownFast => request.move_action,
            OrderType::WalkingCrouched => {
                request.half_diagonal_idx = pathfinder_index;
                OrderType::WalkingUpright
            }
            action => {
                panic!("make_upright received unsupported pending action {action:?} for {owner:?}")
            }
        };
    }

    /// Make the first request for this actor crouched.
    pub(super) fn make_crouched(&mut self, owner: EntityId, pathfinder_index: u16) {
        let Some(request) = self.first_for_owner_mut(owner) else {
            return;
        };
        request.move_action = match request.move_action {
            OrderType::WalkingUpright | OrderType::RunningUpright => {
                request.half_diagonal_idx = pathfinder_index;
                OrderType::WalkingCrouched
            }
            action => {
                panic!("make_crouched received unsupported pending action {action:?} for {owner:?}")
            }
        };
    }

    pub(super) fn clear(&mut self) {
        self.waiting.clear();
        self.in_flight = None;
        self.ignore_next_path = false;
    }
}

pub(super) use path_scheduling::{CompletedPathWork, PathScheduleContext};

#[inline]
fn retained_cancelled_path_result(retained_cancelled_head: bool) -> Option<Vec<MapPoint>> {
    retained_cancelled_head.then(Vec::new)
}

/// Outcome of [`EngineInner::try_dispatch_move_path`], the unified
/// pathfind-and-populate pipeline invoked from the hourglass Move
/// dispatch.
#[derive(Debug)]
pub(crate) enum MovePathOutcome {
    /// Path found and movement orders populated. The instruction boundary
    /// accepts the translated element; execution changes the actor's state.
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
    /// The actor's current state forbids the move outright (contest
    /// archer). The refusal bark has already been played; the caller marks
    /// the element `Impossible`.
    Refused,
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
    adapt_source_to_current_door_with_identity(doors, door_handle, door_direction)
        .map(|(point, sector, layer)| (point, u16::from(sector), layer))
}

/// Identity-preserving form of [`adapt_source_to_current_door`]. Original
/// copies the complete endpoint position, including its sector reference.
pub(crate) fn adapt_source_to_current_door_with_identity(
    doors: &[crate::gate::Door],
    door_handle: crate::position_interface::DoorHandle,
    door_direction: bool,
) -> Option<(MapPoint, crate::position_interface::SectorHandle, u16)> {
    let door = doors.get(usize::from(door_handle))?;
    // door_direction true → use the "in" side of the door as the
    // source; false → use the "out" side.
    if door_direction {
        let handle = crate::position_interface::SectorHandle::new(u16::from(door.sector_in))?;
        Some((
            door.point_in,
            door.sector_in_index
                .map_or(handle, |index| handle.with_arena_index(index)),
            door.layer_in,
        ))
    } else {
        let handle = crate::position_interface::SectorHandle::new(u16::from(door.sector_out))?;
        Some((
            door.point_out,
            door.sector_out_index
                .map_or(handle, |index| handle.with_arena_index(index)),
            door.layer_out,
        ))
    }
}

/// Active door state used as the route-construction origin.
///
/// Rust keeps an executing translated pass in `ActorData` rather than always
/// mirroring it into the position state. Prefer that live pass until its
/// first `PassingDoor` callback, but only while the pass still owns the
/// installed actor order. A postponed pass can remain in this Rust-only slot
/// while an unrelated command is installed; the original game's door query then
/// reflects only the position state and must not be reconstructed from the
/// dormant pass. Door traversal clears the active door at the
/// callback even though the translated movement element can keep executing
/// its far-side walk, so later commands must use the live position/sector
/// instead of adapting through the completed gate.
///
/// The direction reported here is the pass's live traversal direction, not the
/// movement element's retained direction. The original game reads the live
/// door direction written during movement translation, where it comes from the
/// current-sector versus entrance-sector test performed at launch—the same test
/// door-pass dispatch reproduces into `ActiveDoorPass::direct`.
/// `ActiveDoorPass::position_direct` mirrors the *element's* serialized
/// direction, which only AI positioning consumes.
pub(crate) fn current_door_for_route_source(
    entity: &crate::element::Entity,
) -> Option<(crate::position_interface::DoorHandle, bool)> {
    entity
        .actor_data()
        .and_then(|actor| {
            actor.active_door_pass.as_ref().filter(|pass| {
                pass.triggers_fired == 0
                    && actor
                        .installed_order
                        .is_some_and(|order| order.order_type == pass.current_action)
            })
        })
        .map(|pass| (pass.door_index, pass.direct))
        .or_else(|| {
            let position = entity.position_iface();
            position
                .get_door()
                .map(|door| (door, position.get_door_direction()))
        })
}

/// Compare the object identities returned for two authored positions.
///
/// Movement setup compares sector identities directly, so equal script-facing sector
/// numbers do not imply that the positions occupy the same motion sector.
#[cfg(test)]
pub(super) fn sector_hits_have_distinct_identity(
    source: crate::fast_find_grid::SectorHit,
    goal: crate::fast_find_grid::SectorHit,
    expected_sector: crate::position_interface::SectorHandle,
) -> bool {
    match (source, goal) {
        (
            crate::fast_find_grid::SectorHit::Found {
                sector_idx: source_idx,
                sector_number: source_number,
            },
            crate::fast_find_grid::SectorHit::Found {
                sector_idx: goal_idx,
                sector_number: goal_number,
            },
        ) => {
            let expected = crate::sector::SectorNumber::new(u16::from(expected_sector) as i16);
            source_number == expected && goal_number == expected && source_idx != goal_idx
        }
        _ => false,
    }
}

/// Radius for circular dispatch. Original evaluates the integer macro
/// `RHENGINE_GROUP_LIMIT_MAX / 3`, so 70 becomes exactly 23 before conversion
/// to its floating-point vector.
pub(in crate::engine) const CIRCULAR_DISPATCH_RADIUS: f32 = 23.0;

/// Exclusive actor-to-actor distance bound for mercenary formation.
pub(in crate::engine) const GROUP_LIMIT_MAX: f32 = 70.0;

/// Original's exclusive lower actor-to-actor distance bound.
pub(in crate::engine) const GROUP_LIMIT_MIN: f32 = 10.0;

/// Use the ordered junction walk which chooses between mercenary and circular
/// group placement. This is not a
/// centroid-radius test; the ordered actor-to-actor junctions are authoritative.
pub(in crate::engine) fn uses_mercenary_group_formation(pc_positions: &[MapPoint]) -> bool {
    if pc_positions.len() <= 1 {
        return true;
    }

    let mut used_junctions = std::collections::BTreeSet::new();
    for (current_index, current) in pc_positions.iter().enumerate() {
        let mut minimum_distance = GROUP_LIMIT_MAX;
        let mut used_index = None;
        for (compare_index, compare) in pc_positions.iter().enumerate() {
            if compare_index == current_index {
                continue;
            }
            let junction = if compare_index < current_index {
                (compare_index, current_index)
            } else {
                (current_index, compare_index)
            };
            if compare_index != pc_positions.len() - 1 && used_junctions.contains(&junction) {
                continue;
            }
            let dx = current.x - compare.x;
            let dy = current.y - compare.y;
            let distance = (dx * dx + dy * dy).sqrt();
            if distance < minimum_distance {
                minimum_distance = distance;
                used_index = Some(compare_index);
                break;
            }
        }
        if let Some(compare_index) = used_index {
            used_junctions.insert(if compare_index < current_index {
                (compare_index, current_index)
            } else {
                (current_index, compare_index)
            });
        }
        if minimum_distance >= GROUP_LIMIT_MAX || minimum_distance <= GROUP_LIMIT_MIN {
            return false;
        }
    }
    true
}

/// Rebuild Original's per-actor formation box at a candidate destination.
///
/// Ordinary sectors translate the live map-space movement box by the displacement
/// from the actor to the candidate. Lift sectors instead
/// obtain the upright movement box and translate that zero-centred box to the
/// candidate. The original game's movement-box query returns
/// the primary move box for every posture, but keeping the two source forms
/// distinct preserves the actual call boundary and saved live-box state.
fn group_move_candidate_box(
    live_move_box_map: MapBBox,
    upright_move_box: crate::coordinates::MoveBox,
    actor_position: MapPoint,
    candidate: MapPoint,
    is_lift: bool,
) -> MapBBox {
    if is_lift {
        upright_move_box.translated(candidate)
    } else {
        live_move_box_map.translated(candidate - actor_position)
    }
}

/// Build a compact-group formation box using the expected sequence of
/// floating-point operations:
///
/// * ordinary: live map-space movement bounds minus formation center plus destination
/// * lift: upright movement bounds translated by actor position minus lift center plus destination
///
/// These translations must not be algebraically collapsed. The intermediate
/// rounding is observable in the path goal recorded by the Original engine.
fn group_move_mercenary_box(
    live_move_box_map: MapBBox,
    upright_move_box: crate::coordinates::MoveBox,
    actor_position: MapPoint,
    center: MapPoint,
    click: MapPoint,
    is_lift: bool,
) -> MapBBox {
    let centered = if is_lift {
        upright_move_box
            .translated(actor_position)
            .translated(MapVec::new(-center.x, -center.y))
    } else {
        live_move_box_map.translated(MapVec::new(-center.x, -center.y))
    };
    centered.translated(MapVec::new(click.x, click.y))
}

fn group_move_sector_kinds(sector_type: crate::sector::SectorType) -> (bool, bool, bool) {
    (
        sector_type.is_lift(),
        sector_type.is_door(),
        sector_type.is_jump(),
    )
}

#[inline]
fn group_move_route_goal(
    recorded_goal: Option<(crate::sector::SectorNumber, u16)>,
    selected_sector: Option<crate::sector::SectorNumber>,
    selected_layer: u16,
) -> (Option<crate::sector::SectorNumber>, u16) {
    recorded_goal
        .map(|(sector, layer)| (Some(sector), layer))
        .unwrap_or((selected_sector, selected_layer))
}

/// Recover the exact original-game route-goal identity without resolving a public
/// number through the lossy number map. A recorded goal normally names the
/// selected motion sector directly; patch/jump overlays retain an explicit
/// `underlying_sector` edge to the authoritative route goal.
fn group_move_route_goal_index(
    recorded_goal: Option<(crate::sector::SectorNumber, u16)>,
    selected_sector: Option<crate::sector::SectorNumber>,
    selected_sector_index: Option<crate::fast_find_grid::SectorIndex>,
    selected_layer: u16,
    selected_grid_sector: Option<&crate::fast_find_grid::GridSector>,
    level: &crate::fast_find_grid::LevelGrid,
) -> Option<crate::fast_find_grid::SectorIndex> {
    let Some((recorded_sector, recorded_layer)) = recorded_goal else {
        return selected_sector_index;
    };
    if selected_sector == Some(recorded_sector) && selected_layer == recorded_layer {
        return selected_sector_index;
    }
    selected_grid_sector
        .and_then(|sector| sector.underlying_sector)
        .filter(|&index| {
            level.sectors.get(usize::from(index)).is_some_and(|sector| {
                sector.sector_number == recorded_sector && sector.layer == recorded_layer
            })
        })
}

/// Prefer the exact sparse FastFindGrid slot retained by replay translation.
/// A public sector number is not unique in retained topology, so an explicit
/// slot must agree with the recorded public sector number rather than falling
/// back to a coincident spatial hit. The original game passes the sector reference and
/// goal level independently; a sector's topology layer is not
/// an identity component here.
fn resolve_group_move_route_goal_index(
    recorded_goal: Option<(crate::sector::SectorNumber, u16)>,
    exact_goal_index: Option<crate::fast_find_grid::SectorIndex>,
    selected_sector: Option<crate::sector::SectorNumber>,
    selected_sector_index: Option<crate::fast_find_grid::SectorIndex>,
    selected_layer: u16,
    selected_grid_sector: Option<&crate::fast_find_grid::GridSector>,
    level: &crate::fast_find_grid::LevelGrid,
) -> Option<crate::fast_find_grid::SectorIndex> {
    if let Some(index) = exact_goal_index {
        let (recorded_sector, _recorded_layer) = recorded_goal.unwrap_or_else(|| {
            panic!("replay group-move exact goal index requires a recorded goal identity")
        });
        let sector = level.sectors.get(usize::from(index)).unwrap_or_else(|| {
            panic!("replay group-move exact goal index {index:?} is absent from retained topology")
        });
        assert_eq!(
            sector.sector_number, recorded_sector,
            "replay group-move exact goal index disagrees with its recorded public sector number"
        );
        return Some(index);
    }

    group_move_route_goal_index(
        recorded_goal,
        selected_sector,
        selected_sector_index,
        selected_layer,
        selected_grid_sector,
        level,
    )
}

#[inline]
fn group_move_door_selection(
    spatial_clicked_door_index: Option<u32>,
    spatial_is_door_click: bool,
    recorded_door_route: Option<bool>,
) -> (Option<u32>, bool, bool) {
    let (route_door_index, route_is_door) = match recorded_door_route {
        Some(false) => (None, false),
        Some(true) => (
            Some(spatial_clicked_door_index.unwrap_or_else(|| {
                panic!("recorded group move requires a door route but no Rust door was hit")
            })),
            true,
        ),
        None => (spatial_clicked_door_index, spatial_is_door_click),
    };

    // Schema-16's reconstructed route kind is authoritative for the selected
    // door identity too. Rust's spatial query can land on a coincident door
    // polygon even when Original selected the ordinary area underneath; using
    // that reconstructed hit would incorrectly skip position authorization.
    // Live commands have no override and continue to use the spatial result.
    let bypass_formation_authorization = recorded_door_route
        .map(|_| route_is_door)
        .unwrap_or(spatial_is_door_click);
    (
        route_door_index,
        route_is_door,
        bypass_formation_authorization,
    )
}

fn retained_jump_goal_uses_underlying_sector(
    level: &crate::fast_find_grid::LevelGrid,
    goal: crate::sector::SectorNumber,
    layer: u16,
    underlying: Option<crate::fast_find_grid::SectorIndex>,
) -> bool {
    let Some(underlying) = underlying else {
        return false;
    };
    let mut matches = level.sectors.iter().filter(|sector| {
        sector.layer == layer
            && sector.sector_number == goal
            && sector.sector_type.is_jump()
            && sector.underlying_sector == Some(underlying)
    });
    matches.next().is_some() && matches.next().is_none()
}

/// Recognize an old replay's collapsed jump-sector goal.
///
/// Before route-construction outcomes and exact arena identities were added to
/// parity traces, group-movement recording retained only the public number of
/// the original game's selected sector. A jump overlay consumes a sparse construction
/// slot, but movement replaces it with the underlying motion sector when
/// no jump line can execute. The spatial query may
/// already have fallen through to that underlying sector, leaving only the
/// legacy unmapped goal number to reveal the rewrite.
///
/// The spatial arena may stand in for that missing identity only when it is
/// unambiguous: the retained slot is a known non-position ordinary sector, a
/// unique retained jump overlay links to the spatial hit, the hit is otherwise a
/// valid non-door/non-jump/non-lift sector, and its exact arena is already every
/// actor's exact source arena. Explicit modern route outcomes and identities
/// remain authoritative.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
struct LegacyUnmappedJumpGoal {
    recorded_goal: Option<(crate::sector::SectorNumber, u16)>,
    exact_goal_index: Option<crate::fast_find_grid::SectorIndex>,
    has_recorded_route_outcome: bool,
    recorded_door_route: Option<bool>,
    is_door_click: bool,
    is_jump_click: bool,
    is_lift_click: bool,
    is_valid: bool,
    selected_sector_index: Option<crate::fast_find_grid::SectorIndex>,
    selected_layer: u16,
    retained_jump_falls_back_to_spatial: bool,
    all_source_arenas_match_spatial: bool,
}

fn legacy_unmapped_jump_goal_matches_spatial_source(
    topology: Option<&crate::engine::LegacyGridTopologyAssets>,
    request: LegacyUnmappedJumpGoal,
) -> bool {
    let LegacyUnmappedJumpGoal {
        recorded_goal,
        exact_goal_index,
        has_recorded_route_outcome,
        recorded_door_route,
        is_door_click,
        is_jump_click,
        is_lift_click,
        is_valid,
        selected_sector_index,
        selected_layer,
        retained_jump_falls_back_to_spatial,
        all_source_arenas_match_spatial,
    } = request;
    if exact_goal_index.is_some()
        || has_recorded_route_outcome
        || recorded_door_route == Some(true)
        || is_door_click
        || is_jump_click
        || is_lift_click
        || !is_valid
        || selected_sector_index.is_none()
        || !retained_jump_falls_back_to_spatial
        || !all_source_arenas_match_spatial
    {
        return false;
    }
    let Some((goal, goal_layer)) = recorded_goal else {
        return false;
    };
    if goal_layer != selected_layer {
        return false;
    }
    let Ok(slot) = usize::try_from(goal.get()) else {
        return false;
    };
    let Some(topology) = topology else {
        return false;
    };
    matches!(
        topology.sectors.get(slot),
        Some(crate::engine::LegacyGridSectorAsset::NullOrOrdinary)
    ) && topology
        .position_sector_numbers
        .get(slot)
        .is_some_and(Option::is_none)
        && topology
            .position_sector_indices
            .get(slot)
            .is_some_and(Option::is_none)
}

/// Whether exact non-door route-goal provenance suppresses a reconstructed
/// selected-door hit.
///
/// The original game keeps goal and selected sectors independent in
/// group movement: the former drives movement-sequence construction, while the
/// latter supplies the door flag for formation authorization. An explicit recorded
/// door route proves the selected door must survive even when an unmapped
/// Original goal was translated to a non-door terminal search arena.
#[inline]
fn group_move_masks_spatial_door_for_recorded_goal(
    exact_recorded_goal_is_non_door: bool,
    recorded_door_route: Option<bool>,
) -> bool {
    exact_recorded_goal_is_non_door && recorded_door_route != Some(true)
}

#[inline]
fn group_move_uses_simple_route(
    has_recorded_route_outcome: bool,
    is_door_click: bool,
    is_valid: bool,
    goal_sector: Option<crate::sector::SectorNumber>,
    goal_sector_index: Option<crate::fast_find_grid::SectorIndex>,
    goal_layer: u16,
    source_sector: u16,
    source_sector_index: Option<crate::fast_find_grid::SectorIndex>,
    source_layer: u16,
) -> bool {
    // A schema-16 route-construction event proves that Original entered
    // Movement-sequence construction's cross-sector branch. Reconstructed source/goal
    // handles can nevertheless compare equal when overlapping Original
    // sectors collapse onto one Rust identity. The recorded outcome wins over
    // that apparent same-sector result, for both success and failure.
    if has_recorded_route_outcome {
        return false;
    }
    // Movement passes the patch-aware goal sector into the queued sequence.
    // A coincident selected-sector door only controls formation authorization;
    // it cannot turn an equal source/goal sector into a door traversal.
    let same_topology = match (goal_sector_index, source_sector_index) {
        (Some(goal), Some(source)) => goal == source,
        _ => goal_sector
            .is_some_and(|goal| u16::from(goal) == source_sector && goal_layer == source_layer),
    };
    same_topology || (!is_door_click && (!is_valid || goal_sector.is_none()))
}

/// Return an authoritative schema-16 route outcome when one was recorded.
/// `Some(None)` is deliberately distinct from `None`: the former means
/// Original already ran gate A* and observed failure, while the latter permits
/// the live engine to resolve a route itself.
fn recorded_group_move_route_result<T>(
    actor: EntityId,
    successful: Option<T>,
    failed_count: usize,
) -> Option<Option<T>> {
    assert!(
        failed_count <= 1,
        "recorded group move contains duplicate failed routes for {actor:?}"
    );
    assert!(
        failed_count == 0 || successful.is_none(),
        "recorded group move marks {actor:?} route as both successful and failed"
    );
    if failed_count != 0 {
        Some(None)
    } else {
        successful.map(Some)
    }
}

/// Group movement receives one resolved upright action from the click
/// dispatcher. It does not infer sword movement from an actor's opponent list;
/// Movement-animation selection performs any live action-state adaptation when
/// the movement is instructed.
#[inline]
fn player_group_move_action(run: bool) -> OrderType {
    if run {
        OrderType::RunningUpright
    } else {
        OrderType::WalkingUpright
    }
}

/// Recover the complete live sector identity sampled before group movement.
///
/// An adopted compatibility position can retain only the public sector
/// number. In a loaded exact grid, resolve that omitted identity from the
/// actor's current point/public number/layer using the same invariant-checked
/// boundary as every other Rust `Position(element)` snapshot. A wholly empty
/// fixture remains number-only; ambiguous or absent loaded topology is an
/// invariant failure rather than a lossy public-number guess.
#[inline]
fn group_move_source_sector(
    engine: &EngineInner,
    actor: EntityId,
    element: &crate::element::ElementData,
) -> crate::position_interface::SectorHandle {
    super::ai::ai_view_position_sector(engine, element).unwrap_or_else(|| {
        panic!("selected group-move actor {actor:?} has no source sector identity")
    })
}

#[inline]
fn group_move_route_source(
    engine: &EngineInner,
    actor: EntityId,
    entity: &crate::element::Entity,
    doors: &[crate::gate::Door],
) -> (MapPoint, crate::position_interface::SectorHandle, u16) {
    current_door_for_route_source(entity)
        .and_then(|(door_handle, door_direction)| {
            adapt_source_to_current_door_with_identity(doors, door_handle, door_direction)
        })
        .unwrap_or_else(|| {
            let element = entity.element_data();
            (
                element.position_map(),
                group_move_source_sector(engine, actor, element),
                element.layer(),
            )
        })
}

fn find_group_move_gate_path(
    doors: &[crate::gate::Door],
    owner: EntityId,
    source: MapPoint,
    source_sector: crate::position_interface::SectorHandle,
    goal: MapPoint,
    goal_sector: crate::sector::SectorNumber,
    goal_sector_index: Option<crate::fast_find_grid::SectorIndex>,
    goal_layer: u16,
    auth: Option<&crate::gate::ActorAuthInfo>,
    building_is_authorized: &impl Fn(crate::sector::SectorNumber) -> bool,
    sector_lift_type: &impl Fn(crate::sector::SectorNumber) -> Option<crate::sector::LiftType>,
) -> Option<Vec<crate::gate::GatePathStep>> {
    let source_sector_index = source_sector.arena_index();
    let exact_graph = doors
        .iter()
        .any(|door| door.sector_out_index.is_some() || door.sector_in_index.is_some());
    if exact_graph {
        assert!(
            source_sector_index.is_some(),
            "TODO(parity): cross-sector group move for {owner:?} lacks exact source arena identity"
        );
        if goal_sector_index.is_none() {
            // Original-game group moves can retain a mission-patch sector as
            // goal sector. Those constructors participate in gate A* by
            // pointer identity, but intentionally have no Rust position
            // polygon (and therefore no arena index). If no indexed door
            // endpoint exposes the recorded public number, keep the goal as
            // a number-only key: the exact graph cannot fabricate a match and
            // the search returns the same authoritative failure as Original.
            // A represented endpoint, on the other hand, must never lose its
            // exact identity and fall back to a potentially duplicated public
            // number.
            let represented_exact_goal = doors.iter().any(|door| {
                (door.sector_out == goal_sector && door.sector_out_index.is_some())
                    || (door.sector_in == goal_sector && door.sector_in_index.is_some())
            });
            assert!(
                !represented_exact_goal,
                "TODO(parity): authoritative group-move goal sector {} on layer {} lacks exact arena provenance",
                u16::from(goal_sector),
                goal_layer
            );
        }
    }
    crate::gate::find_path_gates_with_sector_indices(
        doors,
        (source.x, source.y),
        u16::from(source_sector),
        source_sector_index,
        (goal.x, goal.y),
        u16::from(goal_sector),
        goal_sector_index,
        auth,
        false,
        building_is_authorized,
        sector_lift_type,
    )
}

/// Movement Execute arms which return without calling into `Sprite` still
/// produce an authoritative `mmotionState` in the Original actor. Rust uses
/// the sprite's transient motion latch to carry specialized Execute results to
/// the actor coordinator, so these non-sprite arms must publish their return
/// explicitly.
#[inline]
fn non_sprite_movement_motion(action: OrderType) -> Option<MotionState> {
    match action {
        OrderType::Freezing => Some(MotionState::InProgress),
        OrderType::PassingDoor => Some(MotionState::Terminated),
        _ => None,
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
#[cfg(test)]
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

    let mut candidates = circular_dispatch_candidate_points(n, click_point);
    // Original inserts every authorized candidate at the head of its list.
    candidates.reverse();
    assign_circular_dispatch_candidates(pc_positions, &candidates, &vec![true; n]).0
}

/// Generate Original's actor-indexed circular candidates before authorization.
#[cfg(test)]
pub(in crate::engine) fn circular_dispatch_candidate_points(
    n: usize,
    click_point: MapPoint,
) -> Vec<MapPoint> {
    circular_dispatch_offsets(n)
        .into_iter()
        .map(|offset| click_point + offset)
        .collect()
}

pub(in crate::engine) fn circular_dispatch_offsets(n: usize) -> Vec<MapVec> {
    (0..n)
        .map(|i| {
            // Preserve `i * (TWO_PI / n)`: the parenthesized f32 division is
            // observable. Rotation then uses double sin/cos and casts each
            // component back to f32.
            let angle = i as f32 * (std::f32::consts::TAU / n as f32);
            let sine = f64::from(angle).sin();
            let cosine = f64::from(angle).cos();
            MapVec::new(
                (sine * f64::from(CIRCULAR_DISPATCH_RADIUS)) as f32,
                -(cosine * f64::from(CIRCULAR_DISPATCH_RADIUS)) as f32,
            )
        })
        .collect()
}

/// Assign an already-authorized, prepend-ordered candidate list using
/// Original's shared nearest-slot/worst-claimant loop.
pub(in crate::engine) fn assign_circular_dispatch_candidates(
    pc_positions: &[MapPoint],
    candidates: &[MapPoint],
    eligible: &[bool],
) -> (Vec<MapPoint>, Vec<usize>) {
    assert_eq!(pc_positions.len(), eligible.len());
    let n = pc_positions.len();

    let mut result = vec![MapPoint::new(0.0, 0.0); n];
    let mut assigned: Vec<bool> = eligible.iter().map(|eligible| !eligible).collect();
    let mut candidate_taken = vec![false; candidates.len()];
    let mut dispatch_order = Vec::with_capacity(candidates.len());

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
                let d = f64::from(dx * dx + dy * dy).sqrt() as f32;
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
                    dispatch_order.push(ci);
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
                    dispatch_order.push(worst);
                    any_assigned = true;
                }
            }
        }

        if !any_assigned || assigned.iter().all(|&a| a) {
            break;
        }
    }

    (result, dispatch_order)
}

/// Build the portion of a line-jump click sequence that follows arrival at
/// the selected source line.
///
/// The approach to that line follows the ordinary move-to-line route and may
/// therefore
/// contain an `AssertPosition` and a complete gate path.  Keeping only the
/// post-arrival elements here prevents callers from accidentally replacing
/// that route with one direct, potentially blocked movement segment.
pub(crate) fn build_line_jump_click_tail(
    owner: EntityId,
    action: OrderType,
    source_line_idx: crate::jump_line::JumpLineIndex,
    destination_line_idx: crate::jump_line::JumpLineIndex,
    click_point: MapPoint,
    click_layer: u16,
    speed_factor: f32,
) -> Vec<crate::sequence::SequenceElement> {
    use crate::element::Command;
    use crate::sequence::{Field, FieldValue, MoveFlags, SequenceElement, SequenceElementData};

    let mut jump = SequenceElement::new_generic(1, Command::JumpCmd, Some(owner));
    jump.set_property(Field::JumplineSource, FieldValue::LineId(source_line_idx));
    jump.set_property(
        Field::JumplineDestination,
        FieldValue::LineId(destination_line_idx),
    );

    let mut final_move = SequenceElement::new_movement(2, Command::Move, Some(owner), action);
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
    vec![jump, final_move]
}

/// Actor that owns the routed approach to a selected jump line.
///
/// Movement substitutes the carrier only for the approach-to-line phase; the
/// explicit jump and post-jump movement
/// remain owned by the selected PC.
pub(in crate::engine) fn line_jump_approach_owner(
    engine: &EngineInner,
    selected_pc: EntityId,
) -> EntityId {
    let selected = engine.expect_entity(selected_pc, "line-jump selected PC");
    if selected.element_data().posture() != crate::element::Posture::OnShoulders {
        return selected_pc;
    }
    let carrier = selected
        .human_data()
        .unwrap_or_else(|| panic!("OnShoulders line-jump owner {selected_pc:?} is not human"))
        .carrier
        .unwrap_or_else(|| {
            panic!("OnShoulders line-jump owner {selected_pc:?} has no retained carrier")
        });
    let carrier_entity = engine.expect_entity(carrier, "OnShoulders line-jump carrier");
    assert!(
        carrier_entity.is_pc(),
        "OnShoulders line-jump carrier {carrier:?} for {selected_pc:?} is not a PC"
    );
    carrier
}

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
    /// Used by the final-order completion check to distinguish an
    /// arrival at the sampled target from an exhausted stale path.
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

/// Immutable operands sampled from the selected movement order before motion.
/// Keeping this named value separate from committed-step outcomes prevents a
/// later order mutation from silently changing the current Execute inputs.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
struct SelectedMovementOrder {
    goal: MapPoint,
    action_state: crate::element::ActionState,
    order_id: Option<std::num::NonZeroU32>,
    door_pass_anim: Option<OrderType>,
    is_final_waypoint: bool,
    order_action: OrderType,
    move_seq_id: crate::sequence::SequenceId,
    move_elem_idx: usize,
    active_move_flags: crate::sequence::MoveFlags,
    order_tolerance: f32,
    order_compute_direction: bool,
    order_reverse: bool,
    order_antagonist: Option<EntityId>,
    transition_distance_continuation: bool,
    next_destination_same_action: Option<MapPoint>,
    legacy_serialized_order_chain: bool,
}

/// Observations retained across the ordinary step's arrival boundary.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
struct MovementArrivalBoundary {
    tolerance_arrival: bool,
    point_seek_post_arrival: bool,
    arrived_after_committed_step: bool,
    is_sword_motion: bool,
    live_seek_target: Option<(
        MapPoint,
        Option<crate::position_interface::SectorHandle>,
        Option<MapPoint>,
    )>,
}

/// Literal operands of the ordinary position commit, captured after motion.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
struct MovementStepOperands {
    actor_id: crate::entity_id::ActorId,
    provenance_frame: u32,
    speed: f32,
    split_motion_speeds: Option<(f32, f32)>,
    first_step_committed: bool,
    cached_increment: MapVec,
    anti_on: bool,
}

/// Argument plumbing shared by the two movement-Execute anti-collision
/// dispatches (the transition fast-climb arm and the ordinary walk arm).
/// A free function rather than a method: at both call sites the mover is
/// held as a live `&mut` borrow out of the entity table, so `self` cannot
/// be borrowed as a whole.
fn apply_prepared_anti_collision_step(
    frame: u32,
    mover: &super::anti_collision::CollisionMover,
    collision: super::anti_collision::CollisionWorld<'_>,
    static_repulsive_points: &[crate::ai::RepulsivePoint],
    prepared: &LiveMobileGeometry,
    fast_grid: &crate::fast_find_grid::FastFindGrid,
    state: &mut super::anti_collision::AntiCollisionState<'_>,
    nx: f32,
    ny: f32,
    speed: f32,
    anti_on: bool,
) -> (f32, f32) {
    super::anti_collision::with_goal_owner_anti_frame(frame, || {
        let trace = super::anti_collision::goal_owner_anti_debug_frame(mover.id).is_some();
        let before = trace.then(|| {
            (
                state.pi.map_position(),
                state.pi.map_goal(),
                state.pi.is_deviated(),
                state.pi.blocked_count,
                state.pi.radius,
            )
        });
        let result = super::anti_collision::apply_anti_collision_step(
            mover,
            collision,
            static_repulsive_points,
            prepared
                .mobile_points_by_layer
                .get(&mover.layer)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            prepared
                .mobile_lines_by_layer
                .get(&mover.layer)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            prepared
                .mobile_polygons_by_layer
                .get(&mover.layer)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            Some(fast_grid),
            Some(&mut *state),
            nx,
            ny,
            speed,
            anti_on,
        );
        if trace {
            eprintln!(
                "[GOAL_OWNER frame={frame} owner={:?} stage=anti_result requested_bits={:08x},{:08x},{:08x} result_bits={:08x},{:08x} before={before:?} after=({:?},{:?},{},{},{})]",
                mover.id,
                nx.to_bits(),
                ny.to_bits(),
                speed.to_bits(),
                result.0.to_bits(),
                result.1.to_bits(),
                state.pi.map_position(),
                state.pi.map_goal(),
                state.pi.is_deviated(),
                state.pi.blocked_count,
                state.pi.radius,
            );
        }
        result
    })
}

impl EngineInner {
    /// Opt-in sequence/path ownership trace for parity frontiers where the
    /// queued path operands already agree but the selected movement command
    /// does not. Keep this on stderr and outside serialized state so enabling
    /// it cannot affect simulation or cache compatibility.
    pub(in crate::engine) fn trace_path_owner_lifecycle(
        &self,
        stage: &'static str,
        owner: EntityId,
        focus: Option<(crate::sequence::SequenceId, usize)>,
    ) {
        let Some(filter) = super::diagnostics::config().path_owner else {
            return;
        };
        let frame = self.control.frame_counter;
        if filter.frame.is_some_and(|expected| expected != frame)
            || self.get_entity(owner).is_none()
        {
            return;
        }
        let creation_order = self.world.original_creation_order(owner);
        if filter
            .creation_order
            .is_some_and(|expected| expected != creation_order)
        {
            return;
        }

        let manager = &self.orders.sequence_manager;
        let selected = self.world.entities.current_element_for_actor(owner);
        let current_order = manager
            .current_order_for_actor(&self.world.entities, owner)
            .map(|(sequence_id, element_index, order)| {
                (
                    sequence_id,
                    element_index,
                    order.order_type,
                    order.order_id,
                    order.done,
                    order.target_x.to_bits(),
                    order.target_y.to_bits(),
                    order.tolerance.to_bits(),
                    order.move_flags,
                    order.antagonist,
                )
            });
        let graph = manager
            .sequences_iter()
            .flat_map(|sequence| {
                sequence
                    .elements
                    .iter()
                    .enumerate()
                    .filter(move |(_, element)| element.owner == Some(owner))
                    .map(move |(element_index, element)| {
                        (
                            sequence.id,
                            element_index,
                            element.command,
                            element.state,
                            element.priority,
                            element.cross_postponed,
                            manager.is_registered_to_go(sequence.id, element_index),
                            element.current_order().map(|order| {
                                (
                                    order.order_type,
                                    order.order_id,
                                    order.done,
                                    order.target_x.to_bits(),
                                    order.target_y.to_bits(),
                                    order.tolerance.to_bits(),
                                    order.move_flags,
                                    order.antagonist,
                                )
                            }),
                        )
                    })
            })
            .collect::<Vec<_>>();
        let actor = self.get_entity(owner).and_then(|entity| {
            entity.actor_data().map(|actor| {
                (
                    actor.action_state,
                    actor
                        .installed_order
                        .as_ref()
                        .map(|order| (order.order_type, order.order_id)),
                )
            })
        });
        let position = self.get_entity(owner).map(|entity| {
            (
                entity.position_iface().map_position(),
                entity.position_iface().old_map_position(),
                entity.position_iface().map_goal(),
                entity
                    .position_iface()
                    .is_increment_map_computed()
                    .then(|| entity.position_iface().get_increment_map()),
                entity.position_iface().is_moving(),
                entity.position_iface().is_deviated(),
            )
        });
        let pending_paths = self
            .orders
            .pending_path_requests
            .parity_state(&self.world.fast_grid);
        eprintln!(
            "[PATH_OWNER frame={frame} co={creation_order} owner={} stage={stage} focus={focus:?} selected={selected:?} current_order={current_order:?} actor={actor:?} position={position:?} pending={pending_paths:?} graph={graph:?}]",
            owner.index(),
        );
    }

    /// Consume one queued element-position update at the beginning of
    /// this actor's update, before order selection.
    ///
    /// Original gives the map-space queue priority when both flags are set;
    /// the world-space queue remains armed for the next actor frame. The
    /// resulting teleport still traverses line-crossing detection.
    pub(super) fn apply_delayed_actor_position(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
    ) {
        let (old_pos, new_pos, layer, posture, is_carried, is_human) = {
            let Some(entity) = self.world.entities.get_mut(entity_id) else {
                panic!("delayed-position owner {entity_id:?} disappeared before actor update");
            };
            let posture = entity.element_data().posture();
            let is_carried = entity
                .human_data()
                .is_some_and(|human| human.carrier.is_some());
            let is_human = entity.is_human();
            let Some((old_pos, new_pos, layer)) =
                entity.element_data_mut().apply_next_delayed_position()
            else {
                return;
            };
            (old_pos, new_pos, layer, posture, is_carried, is_human)
        };

        if !actor_line_crossing_eligible(
            posture,
            is_carried,
            self.world.fast_grid.level.map_bbox.contains_point(new_pos),
        ) {
            return;
        }

        // Original queries one unified LINE_CROSS list here.  Its multi-line
        // arm runs the shared roll/increment update tail even when
        // every crossed line is non-elevation.  Keep the candidate count
        // intact across the elevation and callback dispatches; splitting the
        // queries first loses that observable `count > 1` branch.
        let crossing_indices = self
            .world
            .fast_grid
            .get_actor_crossing_line_indices(layer, old_pos, new_pos);
        let crossing_count = crossing_indices.len();
        let elevation_indices = crossing_indices
            .iter()
            .copied()
            .filter(|&line_index| {
                self.world.fast_grid.level.lines[usize::from(line_index)].is_elevation
            })
            .collect::<Vec<_>>();
        let callback_indices = crossing_indices
            .into_iter()
            .filter(|&line_index| {
                crossing_count == 1
                    || !self.world.fast_grid.level.lines[usize::from(line_index)].is_elevation
            })
            .collect::<Vec<_>>();
        let crossed_elevation = self.check_for_elevation_line_crossing_indices(
            assets,
            entity_id,
            old_pos,
            new_pos,
            layer,
            elevation_indices,
        );
        if crossed_elevation || crossing_count > 1 {
            if is_human {
                self.update_roll_after_crossing(assets, entity_id);
            }
            let compute_direction = self
                .orders
                .sequence_manager
                .current_order_for_actor(&self.world.entities, entity_id)
                .map(|(_, _, order)| order.compute_direction);
            if let Some(compute_direction) = compute_direction
                && let Some(entity) = self.world.entities.get_mut(entity_id)
            {
                entity
                    .position_iface_mut()
                    .compute_increment_all(compute_direction);
            }
        }
        self.check_for_non_elevation_line_crossing_indices(
            sim,
            assets,
            entity_id,
            old_pos,
            new_pos,
            callback_indices,
        );
    }

    /// Check authorization for gate pathfinding.
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

    /// Execute the original game's rider-charging action inside its
    /// rider's creation-ordered movement slot. Returns true only when the live
    /// selected movement order was exactly `RiderCharging` and consumed the
    /// slot; stale state is cleared for every other live-order shape.
    fn tick_rider_charge_owner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &crate::engine::LevelAssets,
        rider_id: EntityId,
        frozen_all: bool,
        anti_context: Option<&LiveMobileGeometry>,
    ) -> Option<RiderChargeExecution> {
        use crate::element::{ActionState, Posture};
        use crate::weapons::SwordStrike;

        let provenance_frame = self.control.frame_counter;
        let selected = self.world.entities.current_element_for_actor(rider_id);
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
                actor.last_executed_rider_charge_order_id = None;
            }
            return None;
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

        // Rider-charge execution samples these before turning and motion on
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

        // The actor's new-order identity is distinct from the sprite's
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
            let obstacles = self.world.sight_obstacles(assets);
            let mut pending_victims = Vec::new();
            for &victim_id in &self.world.actor_registry_ids {
                let victim = self.expect_entity(victim_id, "rider charge candidate");
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
            let rider = self.world.entities[rider_id]
                .as_mut()
                .expect("rider remained present during charge initialization");
            // Rider-charge execution reuses the human actor's serialized
            // sword-strike victim storage. A charge clears and refills
            // that list, removes each landed victim from it, but deliberately
            // leaves unhit candidates behind when the charge order ends. A
            // later lateral/circle strike can therefore inherit them. Keep
            // the active charge view and the human-owned serialized view in
            // lockstep instead of treating the charge candidates as private
            // transient state.
            rider
                .human_data_mut()
                .expect("RiderCharging soldier must have human data")
                .sword_sweep
                .victims = pending_victims;
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
            target_element: order.antagonist,
        };
        let (motion_state, actual_frame) = {
            let (entity, neighbours) = self
                .world
                .entities
                .split_owner(rider_id)
                .expect("rider remained present before charge motion");
            let collision = super::anti_collision::CollisionWorld {
                neighbours,
                profiles: &assets.profile_manager,
            };
            let mover = super::anti_collision::CollisionMover::new(rider_id, entity);
            let elem = entity.element_data_mut();
            // Rider-charge execution turns before processing motion. The first
            // Execute therefore turns toward the previously installed goal;
            // Motion processing initializes this order and computes its new goal
            // only afterward.
            elem.sprite.position_iface.turn();
            let (mut state, distance) = if frozen_all {
                // FrozenAll short-circuits motion before it
                // changes row/frame/order state. Rider-charge execution continues
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
            // Motion processing initializes the new direction goal after the
            // caller's Turn, then applies the standard turning slowdown to
            // this frame's distance using that now-live direction/goal pair.
            let distance = scaled_motion_distance(
                distance,
                speed_factor,
                true,
                elem.sprite.position_iface.get_direction()
                    != elem.sprite.position_iface.get_direction_goal(),
            );
            if distance != 0.0 {
                let pre_position = elem.position_map();
                let increment = elem.sprite.position_iface.get_increment_map();
                let anti_on = elem.sprite.position_iface.is_anti_collision_on();
                let (dx_step, dy_step, recovered_from_deviation, rebuild_after_deviation) =
                    if let Some(prepared) = anti_context
                        && anti_on
                        && mover.active
                    {
                        let move_box = *elem.sprite.position_iface.get_move_box();
                        let half_diagonal = elem.sprite.position_iface.get_half_diagonal();
                        let was_deviated = elem.sprite.position_iface.is_deviated();
                        let mut anti_state = super::anti_collision::AntiCollisionState {
                            pi: &mut elem.sprite.position_iface,
                            move_box,
                            half_diagonal,
                            goal_map: goal,
                        };
                        let (dx_step, dy_step) = apply_prepared_anti_collision_step(
                            provenance_frame,
                            &mover,
                            collision,
                            &self.ai.global.repulsive_points,
                            prepared,
                            &self.world.fast_grid,
                            &mut anti_state,
                            increment.x,
                            increment.y,
                            distance,
                            anti_on,
                        );
                        (
                            dx_step,
                            dy_step,
                            was_deviated && !anti_state.pi.is_deviated(),
                            anti_state.pi.is_deviated() && anti_state.pi.blocked_count == 0,
                        )
                    } else {
                        (increment.x * distance, increment.y * distance, false, false)
                    };

                if elem.sprite.position_iface.is_blocked() {
                    // Motion processing returns an aborted result before committing the
                    // requested step or refreshing its forecast.
                    state = MotionState::Aborted;
                } else {
                    if rebuild_after_deviation && (dx_step != 0.0 || dy_step != 0.0) {
                        let raw = vector_to_sector_0_to_15(dx_step, dy_step);
                        elem.set_direction_goal(if order.reverse { raw ^ 8 } else { raw });
                    }
                    elem.set_position_map(MapPoint::new(
                        pre_position.x + dx_step,
                        pre_position.y + dy_step,
                    ));
                    if rebuild_after_deviation && (dx_step != 0.0 || dy_step != 0.0) {
                        elem.sprite.position_iface.reset_increment_computed();
                        elem.sprite.position_iface.compute_increment_all(false);
                    } else if recovered_from_deviation {
                        elem.sprite.position_iface.reset_increment_computed();
                        elem.sprite.position_iface.compute_increment_all(true);
                    }
                    if elem
                        .sprite
                        .position_iface
                        .is_goal_reached(&self.world.fast_grid, None)
                    {
                        if !elem.sprite.position_iface.is_deviated()
                            && elem.sprite.position_iface.get_tolerance() == 0.0
                        {
                            elem.set_position_map(goal);
                        }
                        state = MotionState::Terminated;
                    }
                    let wait = elem
                        .sprite
                        .wait_time(elem.sprite.current_row, elem.sprite.current_frame);
                    elem.sprite
                        .position_iface
                        .update_forecasted_movement(distance, wait + 1);
                    elem.update_grid_cell();
                }
            }
            elem.sprite.last_motion_state = Some(state);
            (state, elem.sprite.current_frame)
        };
        let last_frame = actual_frame == transition_frames - 1;
        if matches!(motion_state, MotionState::Start) {
            let entity = self.world.entities[rider_id]
                .as_mut()
                .expect("rider remained present after charge motion");
            assert_eq!(
                entity.element_data().posture(),
                Posture::Upright,
                "rider charge must start upright"
            );
            let actor = entity
                .actor_data_mut()
                .expect("RiderCharging soldier must have actor data");
            actor.action_state = ActionState::MovingFast;
            entity
                .element_data_mut()
                .publish_order_posture(Posture::Upright);
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

        let mut victim_index = 0;
        loop {
            let rider = self.expect_entity(rider_id, "rider charge owner");
            let Some(victim_id) = rider
                .human_data()
                .expect("rider must be human")
                .sword_sweep
                .victims
                .get(victim_index)
                .copied()
            else {
                break;
            };
            let Some(victim) = self.world.entities.get(victim_id) else {
                // Original-game references cannot become holes independently; Rust
                // entity removal can. Retain the pending ID so this is visible
                // state rather than silently fabricating a resolved hit.
                victim_index += 1;
                continue;
            };
            if victim.element_data().layer() != sampled_layer
                || !rider_charge_point_in_quad(victim.element_data().position_map(), hit_quad)
            {
                victim_index += 1;
                continue;
            }
            self.queue_sword_damage(
                sim,
                assets,
                victim_id,
                rider_id,
                SwordStrike::Charge,
                weapon_profile_id,
            );
            self.expect_entity_mut(rider_id, "rider charge owner")
                .human_data_mut()
                .expect("rider must be human")
                .sword_sweep
                .victims
                .remove(victim_index);
        }

        let completion_order_id = if last_frame {
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
            let rewritten_id = if still_same {
                let fresh_id = self.orders.allocate_order_id();
                let current = self
                    .orders
                    .sequence_manager
                    .get_element_mut(seq_id, elem_idx)
                    .and_then(|element| element.orders.front_mut())
                    .expect("validated rider charge order disappeared before rewrite");
                current.order_type = OrderType::RunningUpright;
                current.order_id = fresh_id;
                // Rider execution mutates the order action and assigns a new ID on
                // the last charge frame; update the explicit pointer mirror
                // with that same in-place object mutation.
                self.world.entities[rider_id]
                    .as_mut()
                    .expect("rider disappeared before charge order publication")
                    .actor_data_mut()
                    .expect("RiderCharging soldier must have actor data")
                    .installed_order = Some(crate::element::InstalledActorOrder {
                    order_id: fresh_id,
                    order_type: OrderType::RunningUpright,
                });
                Some(fresh_id)
            } else {
                None
            };
            rewritten_id
        } else {
            self.orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .and_then(|element| element.current_order())
                .filter(|current| {
                    current.order_type == OrderType::RiderCharging
                        && current.order_id == order.order_id
                })
                .map(|current| current.order_id)
        };

        // The actor tick clears the new-order flag even when FrozenAll prevents
        // motion initialization. Stamp
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
        Some(RiderChargeExecution {
            completion_order_id,
        })
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
            .expect_entity(principal_id, "sword-movement principal opponent")
            .human_data()
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

    pub(super) fn sword_movement_termination_warrants_provoke(
        &self,
        assets: &crate::engine::LevelAssets,
        entity_id: EntityId,
    ) -> bool {
        let principal_id = self
            .get_entity(entity_id)
            .and_then(|e| e.human_data())
            .and_then(|h| h.opponents.first().copied());
        let Some(principal_id) = principal_id else {
            return false;
        };

        let is_mutual = self
            .expect_entity(principal_id, "sword-movement principal opponent")
            .human_data()
            .and_then(|h| h.opponents.first().copied())
            .map(|opp| opp == entity_id)
            .unwrap_or(false);
        if !is_mutual {
            return false;
        }

        let me = self.expect_entity(entity_id, "sword-movement provoke owner");
        let opponent = self.expect_entity(principal_id, "sword-movement principal opponent");
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
            return false;
        };
        let Some(opponent_weapon) =
            crate::engine::melee::get_hth_weapon_id_full(opponent, &assets.profile_manager)
                .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
        else {
            return false;
        };

        let my_maximal = me_weapon.distance[crate::weapons::WeaponDistance::Maximal as usize];
        let my_uber = me_weapon.distance[crate::weapons::WeaponDistance::Uber as usize];
        let opponent_maximal =
            opponent_weapon.distance[crate::weapons::WeaponDistance::Maximal as usize];
        let opponent_uber = opponent_weapon.distance[crate::weapons::WeaponDistance::Uber as usize];
        tracing::trace!(
            ?entity_id,
            ?principal_id,
            distance,
            my_maximal,
            my_uber,
            opponent_maximal,
            opponent_uber,
            "checking sword-movement termination Provoke"
        );
        both_sword_ranges_contain_distance(
            distance,
            my_maximal,
            my_uber,
            opponent_maximal,
            opponent_uber,
        )
    }

    pub(super) fn launch_sword_movement_termination_provoke(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
    ) {
        self.launch_element(
            sim,
            assets,
            crate::sequence::SequenceElement::new(
                1,
                crate::element::Command::Provoke,
                Some(entity_id),
            ),
        );
    }

    /// A stale sword move can reach its owner slot after
    /// the actor's final opponent has gone away; unless the movement was
    /// explicitly forced, Original aborts that element and submits one
    /// quit-swordfight command before facing the opponent or processing motion.
    pub(super) fn abort_orphaned_sword_movement(
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

        // Human action execution calls the selected movement element's
        // Stop(Injury) before registering QuitSwordfight. That exact-root
        // stop follows only the element's linked successor/postponed graph;
        // in particular, a facing turn queued by a reach-point event is
        // interrupted and sends its condolence card on this stack. Do not use
        // actor stopping here: its pending-list scan would also stop unrelated
        // work, and retiring the movement before this boundary would release
        // the Turn to be inherited by QuitSwordfight.
        let selected_priority = {
            let resolver = Self::priority_resolver(&self.world.entities);
            self.orders.sequence_manager.resolve_element_stop_priority(
                selected.seq_id,
                selected.elem_idx,
                &resolver,
            )
        };
        if selected_priority >= crate::sequence::SequencePriority::Injury {
            let owner_pos = self
                .get_entity(owner)
                .expect("orphan sword movement owner disappeared before Stop")
                .element_data()
                .position_map();
            self.stop_movement_from_root(
                sim,
                assets,
                &mut Vec::new(),
                owner,
                (selected.seq_id, selected.elem_idx),
                owner_pos,
                crate::sequence::SequencePriority::Injury,
                &|engine, element| Self::priority_resolver(&engine.world.entities)(element),
            );
            // Movement stopping delivers the selected movement's notification
            // before base sequence-element stopping walks the
            // linked successor/postponed graph.  That callback may re-enter
            // AI and mutate the graph, so it is a real owner boundary rather
            // than a batchable cleanup detail.
            {
                self.stop_owner_current_from_root(
                    sim,
                    assets,
                    &mut Vec::new(),
                    Some((selected.seq_id, selected.elem_idx)),
                    crate::sequence::SequencePriority::Injury,
                    &|engine, element| Self::priority_resolver(&engine.world.entities)(element),
                );
            }
        }
        // Human action execution only registers this command here. Its ABORTED return
        // reaches the actor update first; the later sequence-manager update
        // calls the ordinary actor-instruction path, which translates the
        // lowering order and overwrites mmotionState with IN_PROGRESS. Direct
        // prebuilt-order instruction at this Execute boundary left the later
        // ABORTED latch authoritative for the whole frame.
        self.launch_element(
            sim,
            assets,
            crate::sequence::SequenceElement::new(
                1,
                crate::element::Command::QuitSwordfight,
                Some(owner),
            ),
        );

        // Original announces EVENT_QUIT_SWORDFIGHT from this guard in the
        // same call, before the Execute arm returns ABORTED — so the soldier's
        // brain has already left its swordfight substate when any later phase
        // of this frame runs. Only soldiers have a receiver here.
        tracing::trace!(
            owner = owner.index(),
            frame = self.control.frame_counter,
            "orphaned sword movement aborted; sending EVENT_QUIT_SWORDFIGHT"
        );
        if matches!(self.world.entities.get(owner), Some(Entity::Soldier(_))) {
            self.execute_ai_callback(
                sim,
                assets,
                owner,
                &crate::ai::Stimulus::new(crate::ai::StimulusType::EventQuitSwordfight),
            );
        }

        // The actor update captures the entry movement before execution. Only
        // after human action execution has returned ABORTED does it mark that captured
        // element Impossible. Keep this after the direct soldier callback so
        // neither QuitSwordfight nor the callback can inherit the stopped
        // Turn's cross-element link.
        self.element_impossible(
            sim,
            assets,
            &mut Vec::new(),
            selected.seq_id,
            selected.elem_idx,
        );
        true
    }

    /// Resolve the point used to face an opponent or danger for this owner,
    /// plus whether that point is compared against
    /// the actor's ground position rather than its map position.
    ///
    /// `None` reproduces opponent-facing's non-soldier, non-swordfighting early
    /// return, which yields `WALKING_SWORD` without touching the facing.
    pub(super) fn combat_face_target_for_owner(
        &self,
        owner: EntityId,
        executes_shield_movement: bool,
    ) -> (Option<MapPoint>, bool) {
        let mut combat_face_target = None;
        let mut combat_face_target_is_ground = false;
        for (_actor_id, entity) in self
            .world
            .entities
            .actors()
            .filter(|(id, _)| EntityId::from(*id) == owner)
        {
            let actor = entity
                .actor_data()
                .expect("entities.actors() yielded non-actor entity");
            // Shield bearers face the stored danger point. Freshly launched
            // RaiseShield keeps the transient 2D copy on ActorData; loaded
            // saves retain the original game's authoritative shield-danger point on
            // PcData. The latter must not be mistaken for an absent point and
            // fall through to the protected-PC branch.
            // Sword fighters face their principal opponent.
            let saved_shield_face_point = entity.pc_data().and_then(|pc| {
                let pt = pc.shield_danger_point;
                (pt.x != 0.0 || pt.y != 0.0 || pt.z != 0.0)
                    .then(|| crate::coordinates::MapPoint::new(pt.x, pt.y))
            });
            if executes_shield_movement
                && let Some(pt) = actor.shield_face_point.or(saved_shield_face_point)
            {
                combat_face_target = Some(pt);
                // Danger-facing subtracts the ground position from the raw
                // X/Y of the shield danger point.
                combat_face_target_is_ground = true;
                continue;
            }
            // Shield bearer with no danger point stored: face *away*
            // from the protected ally.  Encode this as a target equal
            // to `2 * self_pos - ally_pos` so the downstream
            // `vector_to_sector_0_to_15(target - self)` math aims the
            // shield-bearer away from the ally.
            if executes_shield_movement
                && let Some(protected_id) = entity.pc_data().and_then(|pc| pc.shield_protected)
                && let Some(ally) = self.world.entities.get(protected_id)
            {
                let self_pos = entity.element_data().position();
                let ally_pos = ally.element_data().position();
                combat_face_target = Some(crate::coordinates::MapPoint {
                    x: 2.0 * self_pos.x - ally_pos.x,
                    y: 2.0 * self_pos.y - ally_pos.y,
                });
                combat_face_target_is_ground = true;
                continue;
            }
            if executes_shield_movement {
                // Danger-facing does not fall through to general
                // opponent-facing logic when its own point is unresolved.
                continue;
            }
            // Opponent-facing dispatch for sword movement:
            //   swordfighting → principal opponent's ground position
            //   else if soldier → primary target's ground position
            //   else            → return WALKING_SWORD without facing change
            //
            // Build this even before `action_state` flips to MovingSword;
            // forced sword movement can still be represented only by the
            // movement element's FORCE_SWORD_MOVEMENT flag at this point.
            //
            // The non-soldier, non-swordfighting branch returns
            // `WALKING_SWORD` immediately, without constructing a facing
            // vector. Keep that distinct as `None`: using the actor's own
            // position as a sentinel is not equivalent because Position and
            // Ground position can differ while cached projection state is
            // refreshed, turning a nominally-zero vector into a small real
            // angle and selecting a strafe row.
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
                // Primary target—the soldier's AI-picked priority target,
                // which can differ from opponents[0]. The stored handle is a
                // raw element slot and the occupant is any human, not just a
                // PC: soldiers routinely keep an enemy soldier as their
                // primary target once a swordfight has ended, and facing it
                // is what keeps the fighter turned toward the melee.
                entity
                    .ai_controller()
                    .and_then(|c| c.primary_target)
                    .map(crate::ai::AiEntityHandle::get)
                    .and_then(|slot| self.world.entities.id_at_legacy_slot(slot))
            } else {
                None
            };

            if let Some(opp_id) = opp_id_opt
                && let Some(opp) = self.world.entities.get(opp_id)
            {
                let position = opp.element_data().position();
                combat_face_target =
                    Some(crate::coordinates::MapPoint::new(position.x, position.y));
                combat_face_target_is_ground = true;
            }
        }
        (combat_face_target, combat_face_target_is_ground)
    }

    /// Run the sword-/shield-walking Execute arm's facing prologue for a frame
    /// whose seeking is about to take its moved-target refresh
    /// branch.
    ///
    /// Both opponent-facing and danger-facing happen *before* seeking.
    /// The moved-target branch returns an in-progress result without ever
    /// reaching motion processing, so the opponent-facing direction update
    /// and its following turn still land on the
    /// seek-refresh frame. Rust evaluates seek refresh ahead of the movement
    /// Execute arm, so without this the goal keeps its previous value for one
    /// frame.
    pub(super) fn apply_pre_perform_seek_facing_prologue(&mut self, owner: EntityId) {
        let Some(entity) = self.world.entities.get(owner) else {
            return;
        };
        let Some(actor) = entity.actor_data() else {
            return;
        };
        if actor.execution_frozen {
            return;
        }
        let action_state = actor.action_state;
        let door_pass_anim: Option<OrderType> =
            actor.active_door_pass.as_ref().map(|dp| dp.current_action);
        let Some(order_action) = self
            .orders
            .sequence_manager
            .current_order_for_actor(&self.world.entities, owner)
            .map(|(_, _, order)| order.order_type)
        else {
            return;
        };
        let executes_shield_movement =
            executes_shield_movement_action(door_pass_anim, order_action);
        let is_sword_motion = is_sword_motion_context(action_state, door_pass_anim, order_action);
        let (combat_target, target_is_ground) =
            self.combat_face_target_for_owner(owner, executes_shield_movement);
        // Human movement always faces the opponent; PC shield movement always
        // faces danger. Both return without writing when no facing
        // point exists, which is exactly `combat_target == None`.
        if !(executes_shield_movement || is_sword_motion) {
            return;
        }
        let Some(opp_pos) = combat_target else {
            // Danger-facing still turns when neither a danger point
            // nor a protected ally supplied a new direction.
            if executes_shield_movement && let Some(entity) = self.world.entities.get_mut(owner) {
                let _ = entity.element_data_mut().sprite.position_iface.turn();
            }
            return;
        };
        let entity = self
            .world
            .entities
            .get_mut(owner)
            .expect("facing-prologue owner disappeared between borrows");
        let elem = entity.element_data_mut();
        let face_origin = if target_is_ground {
            let position = elem.position();
            crate::coordinates::MapPoint::new(position.x, position.y)
        } else {
            elem.position_map()
        };
        let fdx = opp_pos.x - face_origin.x;
        let fdy = opp_pos.y - face_origin.y;
        elem.set_direction_goal(crate::position_interface::vector_to_sector_0_to_15_iso(
            fdx, fdy,
        ));
        let _ = elem.sprite.position_iface.turn();
    }

    pub(super) fn tick_entity_movement_owner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &crate::engine::LevelAssets,
        owner: EntityId,
        selected: Option<MovementOwnerSelection>,
    ) -> MovementOwnerMotion {
        let Some(selected) = selected else {
            return MovementOwnerMotion::default();
        };
        if !self.prepare_movement_owner_execution(sim, assets, owner, selected) {
            return MovementOwnerMotion::default();
        }

        // Sample mutable mobile geometry only now, at this actor's
        // entity slot. Preparing it once before the live owner walk freezes
        // every actor onto the same side of intervening mobile masters.
        let prepared = self.live_mobile_geometry();

        let final_tolerance = self.movement_final_tolerance(owner, selected);
        self.turn_movement_owner_drunken(owner, selected);

        if let Some(entity) = self.world.entities.get(owner) {
            super::animation::direction_provenance_snapshot(
                entity.position_iface(),
                owner,
                self.control.frame_counter,
                "movement_after_prepass",
            );
        }
        if self.tick_movement_rider_charge(sim, assets, owner, selected, &prepared) {
            return MovementOwnerMotion::default();
        }

        // The coordinator already chose the one live owner. Resolve its typed
        // actor ID directly instead of scanning and allocating an actor list.
        let actor_id = match owner {
            EntityId::Pc(id) => Some(crate::entity_id::ActorId::Pc(id)),
            EntityId::Soldier(id) => Some(crate::entity_id::ActorId::Soldier(id)),
            EntityId::Civilian(id) => Some(crate::entity_id::ActorId::Civilian(id)),
            _ => None,
        };
        if let Some(actor_id) = actor_id.filter(|_| self.world.entities.get(owner).is_some()) {
            return self.tick_one_movement_actor(
                sim,
                assets,
                owner,
                selected,
                actor_id,
                final_tolerance,
                &prepared,
            );
        }
        self.finish_actor_movement(sim, assets, owner, selected, None);
        MovementOwnerMotion::default()
    }

    /// Entry selection is latched once; rejected/frozen execution never falls
    /// through to a second movement order in the same owner slot.
    fn prepare_movement_owner_execution(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &crate::engine::LevelAssets,
        owner: EntityId,
        selected: MovementOwnerSelection,
    ) -> bool {
        let selected_is_live = self
            .orders
            .sequence_manager
            .get_element(selected.seq_id, selected.elem_idx)
            .filter(|element| element.owner == Some(owner) && element.data.is_movement())
            .and_then(|element| element.current_order())
            .is_some_and(|order| order.order_id == selected.order_id);
        if !selected_is_live {
            return false;
        }
        if let Some(entity) = self.world.entities.get(owner) {
            super::animation::direction_provenance_snapshot(
                entity.position_iface(),
                owner,
                self.control.frame_counter,
                "movement_entry",
            );
        }
        if self
            .world
            .entities
            .get(owner)
            .and_then(|entity| entity.actor_data())
            .is_some_and(|actor| actor.execution_frozen)
        {
            return false;
        }
        let selected_command = self
            .world
            .entities
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
            return false;
        }
        if self.abort_orphaned_sword_movement(sim, assets, owner, selected) {
            // Sequence-element launch registers the replacement for the later
            // sequence-manager phase. It must not execute at this actor
            // boundary: the original game exposes swordfight exit as the current
            // command for one frame before its lowering order starts.
            return false;
        }
        // Freeze-all is read only during sprite updates and in NPC AI gates;
        // actor execution itself is never gated on it. The non-animation
        // door-passing arm reaches no sprite
        // method at all: it completes door traversal / restores anti-collision,
        // forwards the stature message, and returns a terminated result whatever
        // the global freeze is. A `FreezeAll(true)` landing on the frame that
        // owns the door action point must therefore not defer it — doing so
        // held the pass open one extra frame and delayed every successor
        // (AI unlock, the route's own WaitTimer, the next path request).
        let selected_is_door_pass_action_point = self
            .orders
            .sequence_manager
            .get_element(selected.seq_id, selected.elem_idx)
            .and_then(|element| element.current_order())
            .is_some_and(|order| {
                order.order_id == selected.order_id && order.order_type == OrderType::PassingDoor
            });
        if self.actors_frozen() && !selected_is_door_pass_action_point {
            // Globally frozen sprite motion returns IN_PROGRESS
            // before touching any row/frame state, and that is what the
            // movement Execute arm hands back to the actor. Latch it here:
            // otherwise the arm runs without a sprite call and the actor
            // keeps re-reporting whatever edge the last unfrozen frame left
            // behind, typically a stale START.
            if let Some(entity) = self.world.entities.get_mut(owner) {
                entity.element_data_mut().sprite.last_motion_state =
                    Some(crate::sprite::MotionState::InProgress);
            }
            let frozen_order =
                self.execute_globally_frozen_pre_motion_owner(sim, assets, owner, selected);
            // RunningUpright is exceptional among the ordinary movement
            // execution paths: it sets the fast-moving state unconditionally
            // after performing motion, not only on its first tick. Therefore the
            // IN_PROGRESS returned by globally frozen sprite motion still
            // changes a waiting actor to moving-fast.
            if frozen_order == OrderType::RunningUpright {
                let entity = self
                    .world
                    .entities
                    .get_mut(owner)
                    .unwrap_or_else(|| panic!("globally frozen runner {owner:?} disappeared"));
                entity
                    .element_data_mut()
                    .publish_order_posture(crate::element::Posture::Upright);
                entity
                    .actor_data_mut()
                    .expect("globally frozen runner is not an actor")
                    .action_state = crate::element::ActionState::MovingFast;
            }
            if frozen_order == OrderType::WalkingWithShield {
                let entity = self.world.entities.get_mut(owner).unwrap_or_else(|| {
                    panic!("globally frozen shield walker {owner:?} disappeared")
                });
                let (posture, action_state) = movement_execute_state_effect(
                    frozen_order,
                    crate::sprite::MotionState::InProgress,
                )
                .expect("WalkingWithShield must own an unconditional Execute state effect");
                entity.set_posture(posture);
                entity
                    .actor_data_mut()
                    .expect("globally frozen shield walker is not an actor")
                    .action_state = action_state;
                refresh_pc_walking_shield_after_execute(
                    entity,
                    &assets.profile_manager,
                    frozen_order,
                );
            }
            // FrozenAll suppresses sprite motion but not the action-execution
            // work before it: climb Turn() above and both rider-specific
            // Soldier arms remain live. RiderCharging performs its polygon
            // work or RunningUpright samples that frozen frame and may Think.
            let charge_execution = self.tick_rider_charge_owner(sim, assets, owner, true, None);
            if charge_execution.is_none() && self.selected_galopp_decision_frame(owner, selected) {
                self.dispatch_galopp_loop_event(sim, assets, owner);
            }
            return false;
        }

        true
    }

    /// Seeking retains its entry destination across refresh and handoff callbacks.
    fn movement_final_tolerance(
        &self,
        owner: EntityId,
        selected: MovementOwnerSelection,
    ) -> FinalTol {
        let entity = self
            .world
            .entities
            .get(owner)
            .expect("movement owner disappeared");
        let actor = entity
            .actor_data()
            .expect("movement owner must be an actor");
        let element = self
            .orders
            .sequence_manager
            .get_element(selected.seq_id, selected.elem_idx)
            .expect("selected movement element disappeared");
        let crate::sequence::SequenceElementData::Movement {
            flags,
            element: target,
            destination,
            ..
        } = &element.data
        else {
            panic!("selected movement element must have movement data");
        };
        if !flags.contains(crate::sequence::MoveFlags::SEEK) {
            return FinalTol::default();
        }
        let target_entity = target.and_then(|id| self.world.entities.get(id));
        let shield = flags.contains(crate::sequence::MoveFlags::SEEK_SHIELD);
        if target_entity.is_none() && !shield {
            return FinalTol::default();
        }
        FinalTol {
            tol: actor.seek_distance,
            directional: flags.contains(crate::sequence::MoveFlags::DIRECTIONAL_TOLERANCE),
            target_is_actor: target_entity.is_some_and(|entity| entity.actor_data().is_some()),
            target_id: target_entity.and(*target),
            use_point: flags.contains(crate::sequence::MoveFlags::USE_POINT),
            shield_destination: shield.then_some(*destination),
            last_seek_target_position: actor.last_seek_target_position,
            has_post_seek: actor.post_seek_sequence.is_some()
                || self
                    .orders
                    .sequence_manager
                    .get_sequence(selected.seq_id)
                    .is_some_and(|sequence| selected.elem_idx + 1 < sequence.elements.len()),
        }
    }

    fn movement_goal_target_info(
        &self,
        antagonist: Option<EntityId>,
    ) -> Option<crate::position_interface::TargetInfo> {
        antagonist
            .and_then(|id| self.world.entities.get(id))
            .map(|target| crate::position_interface::TargetInfo {
                radius: target.position_iface().get_radius(),
            })
    }

    fn turn_movement_owner_drunken(&mut self, owner: EntityId, selected: MovementOwnerSelection) {
        let element = self
            .orders
            .sequence_manager
            .get_element(selected.seq_id, selected.elem_idx)
            .expect("selected movement element disappeared");
        let selected_uses_seek = matches!(&element.data,
            crate::sequence::SequenceElementData::Movement { flags, .. }
                if flags.contains(crate::sequence::MoveFlags::SEEK));
        let Some(order) = element
            .current_order()
            .filter(|order| order.order_id == selected.order_id)
        else {
            return;
        };
        // Seeking owns the turn itself; ordinary drunken walking turns before motion.
        if !should_apply_drunken_turn(selected_uses_seek, order.order_type) {
            return;
        }
        let Some(soldier) = self
            .world
            .entities
            .get_mut(owner)
            .and_then(Entity::as_soldier_mut)
        else {
            return;
        };
        if soldier
            .npc
            .ai_brain
            .base()
            .is_some_and(|brain| brain.blood_alcohol > 0)
        {
            turn_drunken(&mut soldier.element.sprite.position_iface);
        }
    }

    fn movement_owner_lift(
        &self,
        owner: EntityId,
        selected: MovementOwnerSelection,
    ) -> (Option<LiftAnimContext>, Option<i16>, bool) {
        // Resolve sector translation at the animation call site.
        let mut lift_translation = None;
        let mut door_pass_climb_direction = None;
        let mut decorative_building_trap_at_destination = false;
        let entity = self
            .world
            .entities
            .get(owner)
            .expect("movement owner disappeared");
        let actor_id = owner;
        let posture = entity.element_data().posture();
        let door_pass = entity
            .actor_data()
            .and_then(|actor| actor.active_door_pass.as_ref());
        let door_pass_action = door_pass.map(|dp| dp.current_action);
        let Some(sector) = entity.element_data().sector() else {
            return (None, None, false);
        };
        let Some(gs) = grid_sector_for_position_handle(&self.world.fast_grid.level, sector) else {
            return (None, None, false);
        };
        if let Some(action) = door_pass_action
            && let Some(expected) = climb_lift_type(action)
        {
            door_pass_climb_direction = entity
                    .actor_data()
                    .and_then(|actor| actor.active_door_pass.as_ref())
                    .and_then(|dp| {
                        let door = self
                            .script_domains
                            .interactables
                            .doors
                            .get(usize::from(dp.door_index))
                            .unwrap_or_else(|| {
                                panic!(
                                    "door-pass climb owner {actor_id:?} references missing door {}",
                                    dp.door_index
                                )
                            });
                        door_type_uses_lift_climb_direction(door.door_type)
                            .then(|| {
                                crate::position_interface::SectorHandle::new(u16::from(
                                    door.sector_in,
                                ))
                                .map(|handle| {
                                    door.sector_in_index
                                        .map_or(handle, |index| handle.with_arena_index(index))
                                })
                            })
                            .flatten()
                    })
                    .map(|sector_in| {
                        grid_sector_for_position_handle(&self.world.fast_grid.level, sector_in)
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
            if action == OrderType::ClimbingLadderDown
                && door_pass.is_some_and(|pass| {
                    pass.current_reverse
                        && self
                            .script_domains
                            .interactables
                            .doors
                            .get(usize::from(pass.door_index))
                            .is_some_and(|door| {
                                door.door_type == crate::gate::DoorType::BuildingTrap
                            })
                })
                && self
                    .orders
                    .sequence_manager
                    .get_element(selected.seq_id, selected.elem_idx)
                    .and_then(|element| element.current_order())
                    .filter(|order| order.order_id == selected.order_id)
                    .is_some_and(|order| {
                        entity.element_data().position_map()
                            == MapPoint::new(order.target_x, order.target_y)
                    })
            {
                // TODO(parity): the decorative building-trap row
                // invalidly treats its building sector as a lift sector.
                // The three shipped witnesses read direction zero. Keep
                // that release-build compatibility value confined to the
                // exact-target reverse row which immediately terminates.
                door_pass_climb_direction = Some(0);
                decorative_building_trap_at_destination = true;
            }
        }
        let Some(lt) = gs.lift_type else {
            return (
                None,
                door_pass_climb_direction,
                decorative_building_trap_at_destination,
            );
        };
        match posture {
            crate::element::Posture::Upright => {
                lift_translation = Some(LiftAnimContext::Upright(lt));
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
                let (pt_low, pt_high) = lift_endpoint_points_for_sector(gs);
                let ladder_dx = pt_low.x - pt_high.x;
                let ladder_dy = pt_low.y - pt_high.y;
                lift_translation = Some(LiftAnimContext::OnClimb {
                    lift_type: lt,
                    lift_direction: gs.lift_direction,
                    ladder_dx,
                    ladder_dy,
                });
            }
            _ => {}
        }
        if lift_translation.is_none()
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
            let (pt_low, pt_high) = lift_endpoint_points_for_sector(gs);
            lift_translation = Some(LiftAnimContext::OnClimb {
                lift_type: lt,
                lift_direction: gs.lift_direction,
                ladder_dx: pt_low.x - pt_high.x,
                ladder_dy: pt_low.y - pt_high.y,
            });
        }

        (
            lift_translation,
            door_pass_climb_direction,
            decorative_building_trap_at_destination,
        )
    }

    /// Actor update checks crossings after Execute returns, then interprets
    /// its motion result. Only abortion retains the entry-selected element.
    fn finish_actor_movement(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        selected: MovementOwnerSelection,
        motion: Option<MotionState>,
    ) {
        let compute_direction = self
            .orders
            .sequence_manager
            .get_element(selected.seq_id, selected.elem_idx)
            // A synchronous seek handoff clears the actor's order reference.
            // The retired element can retain its order deque until collection,
            // but crossing must not recompute a trajectory from that old order.
            .filter(|element| element.state == crate::sequence::SequenceState::InProgress)
            .and_then(|element| element.current_order())
            .map(|order| order.compute_direction);
        self.dispatch_actor_post_execute_line_crossing(sim, assets, owner, compute_direction);
        match motion {
            Some(MotionState::Aborted) => {
                self.element_impossible(
                    sim,
                    assets,
                    &mut Vec::new(),
                    selected.seq_id,
                    selected.elem_idx,
                );
            }
            Some(MotionState::Terminated) => {
                self.advance_live_order_after_terminal_handoff(sim, assets, owner)
            }
            _ => {}
        }
    }

    fn refresh_movement_transition_seek(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        let Some((flags, target, action)) = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .and_then(|element| match &element.data {
                crate::sequence::SequenceElementData::Movement {
                    flags,
                    element,
                    action,
                    ..
                } => element.map(|target| (*flags, target, *action)),
                _ => None,
            })
        else {
            return;
        };
        let new_target_pos = self
            .world
            .entities
            .get(target)
            .expect("transition seek target disappeared")
            .element_data()
            .position_map();
        let actor = self
            .world
            .entities
            .get_mut(owner)
            .expect("transition seek owner disappeared")
            .actor_data_mut()
            .expect("transition seek owner must be an actor");
        actor.action_state = actor.action_state.set_moving(false, false);
        self.apply_seek_refresh(
            sim,
            assets,
            &mut Vec::new(),
            crate::engine::refresh_seek::EntitySeekRequest {
                owner,
                sequence_id: seq_id,
                element_index: elem_idx,
                target,
                action,
                flags,
            },
            new_target_pos,
        );
    }

    fn abort_pinched_pc_sword_movement(&mut self, owner: EntityId, motion: &mut MotionState) {
        // These calls are inside the Human/PC sword movement Execute arms,
        // after motion processing and before actor completion or order advancement.
        let pinch_abort = self.world.entities.get(owner).and_then(|entity| {
            entity.actor_data()?;
            // The player-character update gates the override on the
            // live selected sequence element: it must exist and must not
            // carry non-interruptible priority
            // priority flag. A door pass is
            // exactly that priority
            // non-interruptible, so a sword walk that
            // belongs to a PassDoor element never aborts — the update's
            // ABORTED arm asserts the same invariant
            // as required by the abort invariant. Without this gate the
            // aborted pop cancelled the door pass's own order advance
            // and the actor replayed the walk instead of reaching its
            // PASSING_DOOR action point.
            let selected_priority = self
                .world
                .entities
                .current_element_for_actor(owner)
                .and_then(|(seq_id, elem_idx)| {
                    self.orders.sequence_manager.get_element(seq_id, elem_idx)
                })
                .map(|element| element.priority)?;
            if selected_priority == crate::sequence::SequencePriority::NonInterruptable {
                return None;
            }
            if !entity.position_iface().is_moving_map()
                || !crate::engine::melee::enemies_are_blocking_my_movement(
                    &self.world.entities,
                    owner,
                )
            {
                return None;
            }
            Some(())
        });
        if pinch_abort.is_some() {
            // The player-character update overrides the nested human
            // motion result with an aborted state here. The
            // the base actor update therefore marks the entry-latched
            // element Impossible and does not run its TERMINATED
            // order-advancement arm, even when motion processing had already
            // reached the short step-back destination.
            *motion = MotionState::Aborted;
        }
    }

    /// Rider charge owns Execute completely; retain its callback/identity ordering.
    fn tick_movement_rider_charge(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
        selected: MovementOwnerSelection,
        prepared: &LiveMobileGeometry,
    ) -> bool {
        let rider_entry_compute_direction = self
            .orders
            .sequence_manager
            .get_element(selected.seq_id, selected.elem_idx)
            .and_then(|element| element.current_order())
            .filter(|order| order.order_id == selected.order_id)
            .map(|order| order.compute_direction);
        if let Some(charge_execution) =
            self.tick_rider_charge_owner(sim, assets, entity_id, false, Some(prepared))
        {
            let charge_motion = self
                .world
                .entities
                .get(entity_id)
                .and_then(|entity| entity.element_data().sprite.last_motion_state);
            self.dispatch_actor_post_execute_line_crossing(
                sim,
                assets,
                entity_id,
                rider_entry_compute_direction,
            );
            if charge_motion == Some(MotionState::Terminated) {
                // The actor update advances only after line-crossing
                // callbacks. Rider-charge execution may legitimately allocate a new ID
                // on its same order; compare against that post-execution
                // identity, while still refusing to consume a callback
                // replacement installed after Execute returned.
                let entry_still_current = self
                    .orders
                    .sequence_manager
                    .get_element(selected.seq_id, selected.elem_idx)
                    .and_then(|element| element.current_order())
                    .is_some_and(|order| {
                        Some(order.order_id) == charge_execution.completion_order_id
                    });
                if entry_still_current {
                    self.do_next_order(sim, assets, selected.seq_id, selected.elem_idx);
                }
                if let Some(entity) = self.world.entities.get_mut(entity_id) {
                    entity.element_data_mut().sprite.last_motion_state = charge_motion;
                }
            } else if charge_motion == Some(MotionState::Aborted) {
                self.element_impossible(
                    sim,
                    assets,
                    &mut Vec::new(),
                    selected.seq_id,
                    selected.elem_idx,
                );
                if let Some(entity) = self.world.entities.get_mut(entity_id) {
                    entity.position_iface_mut().reset_box_blocked();
                }
            }
            return true;
        }
        false
    }

    /// Movement Execute body for the single movement owner. Every early
    /// `return` is a per-actor "done" exit.
    fn tick_one_movement_actor(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &crate::engine::LevelAssets,
        owner: EntityId,
        selected: MovementOwnerSelection,
        actor_id: crate::entity_id::ActorId,
        final_tolerance: FinalTol,
        prepared: &LiveMobileGeometry,
    ) -> MovementOwnerMotion {
        let entity_id = actor_id.into();
        let seek_operands = self.movement_seek_operands(entity_id, selected, final_tolerance);
        let entity = self
            .world
            .entities
            .get_mut(entity_id)
            .expect("movement actor ID collected from entity table must remain present");
        let Some(selection) = MovementStepCtx::select_movement_order(
            entity,
            &self.orders.sequence_manager,
            selected,
            actor_id,
            entity_id,
            self.control.frame_counter,
        ) else {
            self.finish_actor_movement(sim, assets, owner, selected, None);
            return MovementOwnerMotion::default();
        };
        let mut ctx = MovementStepCtx {
            engine: self,
            sim,
            assets,
            owner,
            actor_id,
            entity_id,
            final_tolerance,
            prepared,
            seek_operands,
            traits: selection.traits,
            order: selection.order,
            order_compute_direction: selection.order.order_compute_direction,
            terminal_pc_external_direction_goal: selection.terminal_pc_external_direction_goal,
        };
        if let Some(motion) = ctx.execute_non_sprite_movement_action() {
            ctx.engine
                .finish_actor_movement(sim, assets, owner, selected, Some(motion));
            return MovementOwnerMotion {
                initial: (motion == MotionState::InProgress).then_some(motion),
                post_completion_override: None,
            };
        }
        let seek = ctx.age_movement_seek_refresh();
        let facing = ctx.apply_combat_movement_facing();
        let anim = ctx.select_movement_animation(seek, facing);
        let plan = ctx.plan_movement_motion(seek, facing, anim);
        let first_motion = ctx.perform_first_movement_motion(&plan);
        let mut step = ctx.perform_fast_climb_second_motion(&plan, first_motion);
        let effects = ctx.resolve_movement_step_effects(&plan, &step);
        ctx.trace_door_pass_movement_state(&plan, &effects);
        let motion = if plan.is_transition_without_tolerance_arrival() {
            ctx.tick_movement_transition(&plan, &mut step, &effects)
        } else if !stationary_motion_waits(
            effects.speed,
            plan.seek.tolerance_arrival,
            plan.facing.dist,
        ) && let Some(mut arrival) = ctx.prepare_movement_arrival(&plan, &effects)
        {
            ctx.run_movement_arrival_loop(&plan, &step, &effects, &mut arrival)
                .unwrap_or(effects.state_effect_motion)
        } else {
            effects.state_effect_motion
        };
        let motion = ctx.finish_movement_execute(&plan, &step, motion);
        // Keep the entry orientation across crossing and order-advance callbacks.
        let terminal_direction = (plan.is_transition_without_tolerance_arrival()
            && step.motion_state == MotionState::Terminated)
            .then_some(ctx.terminal_pc_external_direction_goal)
            .flatten();
        ctx.engine
            .finish_actor_movement(sim, assets, owner, selected, Some(motion));
        if let Some((external_direction, movement_direction)) = terminal_direction {
            let entity = ctx
                .engine
                .world
                .entities
                .get_mut(owner)
                .expect("terminal movement owner disappeared");
            if i16::from(entity.position_iface().get_direction_goal()) == movement_direction {
                entity
                    .element_data_mut()
                    .set_direction_goal(external_direction);
            }
        }
        MovementOwnerMotion {
            initial: (motion == MotionState::InProgress).then_some(motion),
            post_completion_override: committed_arrival_post_completion_override(
                step.motion_state,
                effects.state_effect_motion,
                effects.state_effect_motion == MotionState::Terminated,
            ),
        }
    }

    /// Commit collision-adjusted geometry and its forecast.
    /// Returns whether movement aborted; arrival and START remain caller-owned.
    // Disjoint world/AI borrows remain explicit rather than introducing another
    // runtime owner or copying collision state into a temporary context.
    fn commit_ordinary_movement_step(
        entity: &mut crate::element::Entity,
        selected_order: SelectedMovementOrder,
        operands: MovementStepOperands,
        collision: super::anti_collision::CollisionWorld<'_>,
        repulsive_points: &[crate::ai::RepulsivePoint],
        prepared: &LiveMobileGeometry,
        fast_grid: &crate::fast_find_grid::FastFindGrid,
        titbits: &mut crate::titbit::TitbitManager,
    ) -> bool {
        let SelectedMovementOrder {
            goal,
            order_reverse,
            ..
        } = selected_order;
        let MovementStepOperands {
            actor_id,
            provenance_frame,
            speed,
            split_motion_speeds,
            first_step_committed,
            cached_increment,
            anti_on,
        } = operands;
        let entity_id = actor_id.into();
        let mover = super::anti_collision::CollisionMover::new(entity_id, entity);
        let nx = cached_increment.x;
        let ny = cached_increment.y;
        // Preserve the two storage roundings of Original's
        // two-step fast-climb dispatch. See the
        // transition branch above for why the summed distance is
        // insufficient even when both calls use one increment.
        let split_motion_target = split_motion_speeds
            .filter(|_| !anti_on && !first_step_committed)
            .map(|(first_speed, second_speed)| {
                let mut target = entity.element_data().position_map();
                target.x += nx * first_speed;
                target.y += ny * first_speed;
                target.x += nx * second_speed;
                target.y += ny * second_speed;
                target
            });
        // Pull transient anti-collision context from position_iface
        // (move box, half-diagonal) + the current path goal.  The
        // persistent state (deviated / blocked_count / box_blocked /
        // radius) lives on the actor's PI directly now.
        let (dx_step, dy_step, recovered_from_deviation, rebuild_after_deviation) =
            if anti_on && mover.active {
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
                let (dx_step, dy_step) = apply_prepared_anti_collision_step(
                    provenance_frame,
                    &mover,
                    collision,
                    repulsive_points,
                    prepared,
                    fast_grid,
                    &mut state,
                    nx,
                    ny,
                    speed,
                    anti_on,
                );
                (
                    dx_step,
                    dy_step,
                    was_deviated && !state.pi.is_deviated(),
                    // A successfully committed deviation expands the
                    // blocked box, resets the counter, and Original
                    // rebuilds the cached increment. Its
                    // blocked-count break-through path instead uses
                    // MoveMap and deliberately retains the old cache.
                    state.pi.is_deviated() && state.pi.blocked_count == 0,
                )
            } else {
                (nx * speed, ny * speed, false, false)
            };
        {
            let elem = entity.element_data_mut();
            if rebuild_after_deviation && (dx_step != 0.0 || dy_step != 0.0) {
                // A committed deviation faces along the step it
                // just took, then invalidates and reconstructs the
                // cached increment from the new position to the
                // original goal (the rebuild deliberately retains
                // this direction rather than recomputing it).  The
                // break-through barge sets its own facing inside
                // the anti-collision step, so it is excluded here.
                let raw = vector_to_sector_0_to_15(dx_step, dy_step);
                elem.set_direction_goal(if order_reverse { raw ^ 8 } else { raw });
            }
            let pm = split_motion_target.unwrap_or_else(|| {
                let mut pm = elem.position_map();
                pm.x += dx_step;
                pm.y += dy_step;
                pm
            });
            elem.set_position_map(pm);
            if rebuild_after_deviation && (dx_step != 0.0 || dy_step != 0.0) {
                elem.sprite.position_iface.reset_increment_computed();
                elem.sprite.position_iface.compute_increment_all(false);
            } else if recovered_from_deviation {
                // Original's no-new-deviation recovery branch commits
                // the (possibly zero-length) step, clears
                // deviation status, and rebuilds the increment with
                // direction computation enabled.
                elem.sprite.position_iface.reset_increment_computed();
                elem.sprite.position_iface.compute_increment_all(true);
            }
        }

        // Refresh the movement forecast used to lead moving
        // targets (arrow / stone / apple aiming).  This sits at
        // the same point as the position commit: after the
        // anti-collision step, using the effective distance and
        // the wait time of the frame the sprite has just
        // reached.  A blocked step aborts before reaching it.
        //
        // The fast climb arms commit two motion calls in one
        // tick; only the later one's distance survives in the
        // forecast, so prefer the second speed when it moved.
        refresh_motion_forecast(entity.sprite_mut(), speed, split_motion_speeds);

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
                    titbits.add_titbit(
                        crate::coordinates::WorldPoint3D {
                            x: pos.x,
                            y: pos.y,
                            z: pos.z,
                        },
                        layer,
                        crate::titbit::TitbitKind::Water,
                        crate::titbit::ElementHandle::INVALID,
                        0,
                        crate::titbit::ElementHandle::INVALID,
                        false,
                        crate::titbit::INVALID_ID,
                        true,
                        None,
                        None,
                    );
                } else {
                    elem.sprite.splitch_count = elem.sprite.splitch_count.saturating_add(1);
                }
            }
        }

        // When the blocked counter trips, the motion aborts
        // and the backing sequence element is marked
        // Impossible.
        let movement_aborted = entity.position_iface().is_blocked();
        if movement_aborted {
            let actor = entity.actor_data_mut().expect("actor-only branch");
            let restore_anti_collision = {
                let restore_anti_collision = actor.active_door_pass.is_some();
                if restore_anti_collision {
                    tracing::warn!(
                        entity = ?entity_id,
                        "DoorPass: movement blocked; clearing active pass with aborted movement"
                    );
                    actor.active_door_pass = None;
                }
                // The movement Execute switches have no ABORTED
                // state branch. The actor update marks the captured
                // element Impossible, but the actor keeps whatever
                // live state Execute established before returning.
                // In particular a walking actor remains Moving;
                // RunningUpright's unconditional Execute effect is
                // applied below and still publishes MovingFast.
                restore_anti_collision
            };
            if restore_anti_collision {
                entity.position_iface_mut().set_anti_collision_on(true);
            }
            entity.position_iface_mut().reset_box_blocked();
        }

        movement_aborted
    }

    /// Commit the first fast-motion call before the second turn samples its
    /// position and increment. Stairs may snap here; wall/ladder steps may not.
    // These are disjoint engine borrows, not a second movement-state owner.
    fn commit_first_fast_movement_step(
        sprite: &mut crate::sprite::Sprite,
        selected_order: SelectedMovementOrder,
        goal_target_info: Option<crate::position_interface::TargetInfo>,
        provenance_frame: u32,
        first_speed: f32,
        mover: super::anti_collision::CollisionMover,
        collision: super::anti_collision::CollisionWorld<'_>,
        repulsive_points: &[crate::ai::RepulsivePoint],
        prepared: &LiveMobileGeometry,
        fast_grid: &crate::fast_find_grid::FastFindGrid,
    ) -> (MapPoint, MapVec, f32, MapPoint) {
        let SelectedMovementOrder {
            goal,
            order_reverse,
            order_action,
            order_tolerance,
            ..
        } = selected_order;
        let first_pre = sprite.position_iface.map_position();
        let first_increment = sprite.position_iface.get_increment_map();
        let anti_on = sprite.position_iface.is_anti_collision_on();
        let (first_dx, first_dy, recovered, rebuild) = if anti_on && mover.active {
            let move_box = *sprite.position_iface.get_move_box();
            let half_diagonal = sprite.position_iface.get_half_diagonal();
            let was_deviated = sprite.position_iface.is_deviated();
            let mut state = super::anti_collision::AntiCollisionState {
                pi: &mut sprite.position_iface,
                move_box,
                half_diagonal,
                goal_map: goal,
            };
            let (dx, dy) = apply_prepared_anti_collision_step(
                provenance_frame,
                &mover,
                collision,
                repulsive_points,
                prepared,
                fast_grid,
                &mut state,
                first_increment.x,
                first_increment.y,
                first_speed,
                true,
            );
            (
                dx,
                dy,
                was_deviated && !state.pi.is_deviated(),
                state.pi.is_deviated() && state.pi.blocked_count == 0,
            )
        } else {
            (
                first_increment.x * first_speed,
                first_increment.y * first_speed,
                false,
                false,
            )
        };
        let first_raw_post = MapPoint::new(first_pre.x + first_dx, first_pre.y + first_dy);
        sprite.position_iface.set_map_position(first_raw_post);
        if rebuild && (first_dx != 0.0 || first_dy != 0.0) {
            let raw = vector_to_sector_0_to_15(first_dx, first_dy);
            sprite
                .position_iface
                .set_direction(crate::position_interface::Direction::from_raw(i32::from(
                    if order_reverse { raw ^ 8 } else { raw },
                )));
            sprite.position_iface.reset_increment_computed();
            sprite.position_iface.compute_increment_all(false);
        } else if recovered {
            sprite.position_iface.reset_increment_computed();
            sprite.position_iface.compute_increment_all(true);
        }
        // Running on stairs is the one double-motion
        // Execute arm which deliberately continues after its first
        // motion processing returns a terminated result. That first step still
        // owns the complete ordinary arrival branch: goal-arrival testing,
        // followed by the zero-tolerance goal snap.  The second
        // turning and motion therefore observe the snapped position,
        // rather than both raw displacements being committed before a
        // single aggregate arrival check.
        let first_post = if order_action == OrderType::RunningStairs
            && sprite
                .position_iface
                .is_goal_reached(fast_grid, goal_target_info)
            && order_tolerance == 0.0
            && !sprite.position_iface.is_deviated()
        {
            sprite.position_iface.set_map_position(goal);
            goal
        } else {
            first_raw_post
        };

        // Fast wall/ladder Execute arms contain two literal
        // motion steps. The original game refreshes the forecast at
        // the end of each nonzero call, immediately after its
        // position commit. Keep that first write here: when the
        // second sprite frame has zero distance the stationary tail
        // returns before the aggregate commit below, and the first
        // call's forecast must remain observable.
        refresh_motion_forecast(sprite, first_speed, None);
        (first_pre, first_increment, first_speed, first_post)
    }

    /// Settle a reached ordinary waypoint and return its motion result.
    fn settle_movement_waypoint(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        ft: FinalTol,
        selected_order: SelectedMovementOrder,
        entity_id: EntityId,
        boundary: MovementArrivalBoundary,
    ) -> MotionState {
        let SelectedMovementOrder {
            goal,
            order_tolerance,
            move_seq_id,
            move_elem_idx,
            is_final_waypoint,
            order_action,
            active_move_flags,
            ..
        } = selected_order;
        let MovementArrivalBoundary {
            tolerance_arrival,
            point_seek_post_arrival,
            arrived_after_committed_step,
            is_sword_motion,
            live_seek_target,
        } = boundary;
        let entity = self
            .world
            .entities
            .get_mut(entity_id)
            .expect("movement owner disappeared at waypoint");
        let orders = &mut self.orders;
        // Original-game movement and seek processing returns terminated
        // after committing the step which reaches the goal. Rust
        // stages geometry after the sprite call, so its raw
        // motion state can still be DONE here. Queue the Human
        // Execute termination callback at the authoritative
        // arrival boundary; it owns the range-based Provoke
        // launched after sword movement.
        // Reached waypoint — snap to it and advance. Original's
        // ordinary motion snap happens only while moving;
        // its TillLastFrame equivalent requires nonzero distance
        // and increment. If an order starts at its exact goal,
        // consume it without needlessly recomputing map -> 3D.
        if should_snap_arrival(
            arrived_after_committed_step,
            tolerance_arrival,
            order_tolerance,
            entity.position_iface().is_deviated(),
        ) {
            entity
                .element_data_mut()
                .set_position_map(crate::coordinates::MapPoint {
                    x: goal.x,
                    y: goal.y,
                });
        }
        let eid = entity_id;

        // A final concrete waypoint is only the position at
        // which the target was observed when this Seek was
        // built.  When the walking order terminates, Original
        // Seeking validates that stale waypoint against the
        // live target before it may hand off to the post-seek
        // action:
        //
        //   same sector
        //   && (target has not moved || live target is in range)
        //
        // If that check fails, seek refresh replaces the movement
        // immediately and the exhausted old order must not reach
        // generic `do_next_order` (which would launch the Hit /
        // interaction tail unconditionally).
        let movement_is_last_sequence_element = orders
            .sequence_manager
            .get_sequence(move_seq_id)
            .map(|sequence| move_elem_idx + 1 >= sequence.elements.len())
            .unwrap_or(false);
        let final_entity_seek_arrival =
            if is_final_waypoint && movement_is_last_sequence_element && ft.target_id.is_some() {
                live_seek_target.map(|(target_position, target_sector, _)| {
                    let same_sector =
                        target_sector.is_some() && target_sector == entity.element_data().sector();
                    let target_unchanged = target_position == ft.last_seek_target_position;
                    same_sector && (target_unchanged || tolerance_arrival)
                })
            } else {
                None
            };
        if final_entity_seek_arrival == Some(false) {
            tracing::trace!(
                ?eid,
                "tick_move: final seek waypoint is stale; refreshing against live target",
            );
            refresh_pc_walking_shield_after_execute(entity, &assets.profile_manager, order_action);
            self.refresh_movement_transition_seek(sim, assets, eid, move_seq_id, move_elem_idx);
            return MotionState::InProgress;
        }

        // The sibling case, where a stop transition is still
        // queued behind the movement order that just terminated.
        // A transition covers its own animation distance, so it
        // may only take over when the live target sits within
        // that travel plus the seek distance. A target that has
        // drifted beyond it refreshes the seek instead, and the
        // stale transition never plays.
        if !is_final_waypoint
            && let Some((target_position, _, target_point)) = live_seek_target
            && target_position != ft.last_seek_target_position
            && let Some(next_action) = orders
                .sequence_manager
                .get_element(move_seq_id, move_elem_idx)
                .and_then(|element| element.orders.get(1))
                .map(|order| order.order_type)
            && matches!(
                next_action,
                OrderType::TransitionRunningUprightWaitingUpright
                    | OrderType::TransitionWalkingUprightWaitingUpright
                    | OrderType::TransitionWalkingCrouchedWaitingCrouched
            )
        {
            let aim = target_point.unwrap_or(target_position);
            let here = entity.element_data().position_map();
            let dx = aim.x - here.x;
            let dy = if ft.directional {
                const INVERSE_ASPECT_RATIO: f32 = 1.743_446_8;
                (aim.y - here.y) * INVERSE_ASPECT_RATIO
            } else {
                aim.y - here.y
            };
            let reach =
                (f32::from(entity.sprite().distance_for_animation(next_action)) + ft.tol) * 1.05;
            if dx * dx + dy * dy > reach * reach {
                // Motion processing already committed this frame's
                // step before seeking decided to refresh.
                // the actor update still runs
                // line-crossing checks after execution returns, so
                // preserve the segment even though the refreshed
                // seek replaces the current movement before the
                // crossing callback.

                tracing::trace!(
                    ?eid,
                    ?next_action,
                    reach,
                    "tick_move: seek target out of stop-transition reach; refreshing",
                );
                refresh_pc_walking_shield_after_execute(
                    entity,
                    &assets.profile_manager,
                    order_action,
                );
                self.refresh_movement_transition_seek(sim, assets, eid, move_seq_id, move_elem_idx);
                return MotionState::InProgress;
            }
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

        if start_post_seek {
            // Post-seek sequence launch makes seeking return
            // TERMINATED, so human action execution observes the sword
            // movement completion before the actor update advances
            // the selected element.
            // The original game terminates the seek before starting the post-seek sequence
            // and launches the interaction without rewriting the
            // actor state. The interaction's generated transition
            // owns any later Moving→Waiting change.
            actor.active_door_pass = None;
            if is_sword_motion && let Some(human) = entity.human_data_mut() {
                human.last_motion_was_step_back_in_combat =
                    active_move_flags.contains(crate::sequence::MoveFlags::STEP_BACK_IN_COMBAT);
            }
            refresh_pc_walking_shield_after_execute(entity, &assets.profile_manager, order_action);
            return if self.start_post_seek_sequence(
                sim,
                assets,
                &mut Vec::new(),
                eid,
                Some((move_seq_id, move_elem_idx)),
            ) {
                MotionState::Terminated
            } else {
                MotionState::InProgress
            };
        }

        // With no post-seek tail, the successful final
        // entity-target arrival remains inside seeking. It
        // arms an immediate refresh check and returns InProgress
        // instead of consuming the final order.
        if final_entity_seek_arrival == Some(true) {
            actor.seek_refresh_wait = 0;
            refresh_pc_walking_shield_after_execute(entity, &assets.profile_manager, order_action);
            return MotionState::InProgress;
        }

        if is_final_waypoint {
            // All waypoints for current walk step consumed.
            // Check if we have more door-pass steps.
            let advance = if actor.active_door_pass.is_some() {
                Self::advance_door_pass(actor, eid, goal, &mut orders.next_order_id)
            } else {
                DoorPassAdvance::Done { completed: None }
            };

            match advance {
                DoorPassAdvance::Continue {
                    order_id,
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
                    let mut order =
                        crate::order::Order::new(action, destination.x, destination.y, order_id);
                    order.reverse = reverse;
                    order.compute_direction = compute_direction;
                    order.tolerance = tolerance;
                    insert_door_pass_successor(
                        orders
                            .sequence_manager
                            .get_element_mut(move_seq_id, move_elem_idx)
                            .expect("door-pass successor element disappeared"),
                        order,
                    );
                }
                DoorPassAdvance::Paused { transition_order } => {
                    // Transition animation queued — push the
                    // order onto the actor's current sequence
                    // element after the loop closes.
                    insert_door_pass_successor(
                        orders
                            .sequence_manager
                            .get_element_mut(move_seq_id, move_elem_idx)
                            .expect("door-pass successor element disappeared"),
                        transition_order,
                    );
                }
                DoorPassAdvance::ActionPoint { order } => {
                    insert_door_pass_successor(
                        orders
                            .sequence_manager
                            .get_element_mut(move_seq_id, move_elem_idx)
                            .expect("door-pass successor element disappeared"),
                        order,
                    );
                }
                DoorPassAdvance::Done { completed } => {
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
                    // Keep the movement action state until an
                    // optional end transition actually finishes.
                    // The actor's walking execution branch leaves
                    // MOVING unchanged on a terminated motion; the
                    // transition-to-waiting arm performs the state
                    // change itself. The two PC carry-walk Execute
                    // overrides are exceptions: both explicitly
                    // restore WAITING on a terminated motion even
                    // when the Move has NO_TRANSITIONS.
                    if matches!(
                        order_action,
                        OrderType::WalkingWithCorpse | OrderType::WalkingCarryingOnShoulders
                    ) {
                        actor.action_state = crate::element::ActionState::Waiting;
                    }
                    actor.active_door_pass = None;
                    if is_sword_motion && let Some(human) = entity.human_data_mut() {
                        human.last_motion_was_step_back_in_combat = active_move_flags
                            .contains(crate::sequence::MoveFlags::STEP_BACK_IN_COMBAT);
                    }
                    if let Some((door_index, direct)) = completed {
                        self.commit_completed_door_pass_position(assets, eid, door_index, direct);
                        self.apply_completed_door_pass_lift_entry_state(eid, door_index, direct);
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
        MotionState::Terminated
    }

    /// Read the selected movement order and apply only its execution-entry
    /// ownership repairs. None retains the old per-actor early-exit behavior.
    fn prepare_selected_movement_order(
        entity: &mut crate::element::Entity,
        manager: &crate::sequence::SequenceManager,
        selected: MovementOwnerSelection,
        entity_id: EntityId,
        is_swordfighting: bool,
    ) -> Option<SelectedMovementOrder> {
        let actor = match entity.actor_data_mut() {
            Some(a) => a,
            None => return None,
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
        let move_elem = manager
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
                return None;
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
                restore_anti_collision
            };
            if restore_anti_collision {
                entity.position_iface_mut().set_anti_collision_on(true);
            }
            return None;
        };
        if !has_moving_state
            && actor
                .selected_sequence_element
                .map(|selected| (selected.sequence_id, selected.element_index))
                != Some((seq_id, elem_idx))
        {
            // A parallel movement element can remain in progress
            // while a higher-priority non-movement element owns the
            // actor. Only bootstrap a non-moving actor when this Move
            // is its selected current element.
            return None;
        }
        let Some(order) = manager
            .get_element(seq_id, elem_idx)
            .and_then(|e| e.current_order())
        else {
            return None;
        };
        let goal = MapPoint::new(order.target_x, order.target_y);
        let order_id = Some(order.order_id);
        let order_action = order.order_type;
        let order_tolerance = order.tolerance;
        let order_compute_direction = order.compute_direction;
        let order_reverse = order.reverse;
        let order_antagonist = order.antagonist;
        let transition_distance_continuation = order.transition_distance_continuation;
        let next_destination_same_action = manager
            .get_element(seq_id, elem_idx)
            .and_then(|e| e.next_order())
            .filter(|next| next.order_type == order_action)
            .map(|next| MapPoint::new(next.target_x, next.target_y));
        let active_move_flags = manager
            .get_element(seq_id, elem_idx)
            .and_then(|e| match &e.data {
                crate::sequence::SequenceElementData::Movement { flags, .. } => Some(*flags),
                _ => None,
            })
            .unwrap_or(crate::sequence::MoveFlags::empty());
        let legacy_serialized_order_chain = manager
            .get_element(seq_id, elem_idx)
            .is_some_and(|element| element.legacy_v48.is_some());

        // A materialized walk/run successor can sit behind a
        // speed-change transition in the sequence-manager queue.
        // When it becomes current, Original's single order list makes
        // that concrete action authoritative; retire the split
        // door-pass transition mirror at the same owner boundary.
        if let Some(pass) = actor.active_door_pass.as_mut() {
            synchronize_selected_door_pass_walk_action(&mut pass.current_action, order_action);
        }

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
        let is_final_waypoint = manager
            .get_element(seq_id, elem_idx)
            .map(|e| e.orders.len() <= 1)
            .unwrap_or(true);
        // Use the animation from the active door-pass Walk step.
        let door_pass_anim: Option<OrderType> =
            actor.active_door_pass.as_ref().map(|dp| dp.current_action);
        Some(SelectedMovementOrder {
            goal,
            action_state: actor.action_state,
            order_id,
            door_pass_anim,
            is_final_waypoint,
            order_action,
            move_seq_id: seq_id,
            move_elem_idx: elem_idx,
            active_move_flags,
            order_tolerance,
            order_compute_direction,
            order_reverse,
            order_antagonist,
            transition_distance_continuation,
            next_destination_same_action,
            legacy_serialized_order_chain,
        })
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
            // Actor update publishes its current order before Execute.
            self.publish_selected_order_as_installed(owner);
            let selected = self
                .orders
                .sequence_manager
                .current_order_for_actor(&self.world.entities, owner)
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
            // The returned `MovementOwnerMotion` (explicit execute motion +
            // terminal order pops) is only meaningful inside a live actor
            // slot, where the coordinator folds it into the actor's explicit
            // execute motion and batches the pops (`tick.rs`). This
            // test-only wrapper has no enclosing slot, so it is dropped.
            let _motion = self.tick_entity_movement_owner(sim, assets, owner, selected);
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
        assets: &LevelAssets,
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
        // opponent-facing to a concrete forward/backward/strafe sword row.
        let (posture_after, action_after) = self
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
        // Human movement-animation selection first handles a stamped sword
        // state itself. Otherwise it delegates to Actor's base method, which
        // switches on the actor's *live* action state. That second arm is
        // observable when transition generation has stamped Waiting after
        // appending StopMenace/LowerSword orders: translation still authors a
        // sword movement from the live Menacing state, and Execute aborts it
        // later if the fight has ended. Base ignores the authored walk/run in
        // that arm and chooses solely from the fast-movement flag.
        //
        // FORCE does not participate in translation; human action execution reads it
        // later solely to keep an already-translated sword move alive after
        // the opponent list becomes empty. A resumed Move|FORCE whose live
        // state is already Waiting therefore remains an ordinary walk.
        let stamped_sword_movement_context =
            posture_after == crate::element::Posture::Upright && action_after.is_sword();
        if stamped_sword_movement_context {
            move_action = sword_movement_dispatch_action(move_action);
        }
        // PC shield-action arm: a shield-wielding PC with an Upright
        // stamped posture rewrites the movement element's stored `action`:
        //   WALKING_UPRIGHT / WALKING_WITH_CORPSE → WALKING_WITH_SHIELD
        //   WALKING_WITH_SHIELD                     → already set, no-op
        //   RUNNING_UPRIGHT                         → no
        //                                             running-with-shield
        //                                             anim, leave the
        //                                             upright variant
        //   default                                 → warn (would
        //                                             assert in dev).
        // This derived override is gated on PC and on the stamped state. It
        // authors the logical shield token unconditionally; danger-facing
        // resolves the concrete sprite row later. Actors which delegate to
        // the base implementation get the separate live-state rewrite below.
        let owner_entity =
            self.world.entities.get(owner).unwrap_or_else(|| {
                panic!("movement owner {owner:?} disappeared during translation")
            });
        let owner_is_pc = owner_entity.is_pc();
        let live_action_state = owner_entity
            .actor_data()
            .expect("movement owner must retain actor data during translation")
            .action_state;
        let owner_sector = owner_entity.element_data().sector();
        let owner_is_on_lift = self.sector_is_lift(owner_sector);
        let live_sword_movement_context = !stamped_sword_movement_context
            && posture_after == crate::element::Posture::Upright
            && !owner_is_on_lift
            && (live_action_state.is_sword()
                || live_action_state == crate::element::ActionState::Menacing);
        if live_sword_movement_context {
            move_action = if is_fast {
                OrderType::RunningWithSword
            } else {
                OrderType::WalkingWithSword
            };
        }
        let sword_movement_context = stamped_sword_movement_context || live_sword_movement_context;
        let pc_stamped_shield_context = owner_is_pc
            && posture_after == crate::element::Posture::Upright
            && action_after.is_shield();
        if pc_stamped_shield_context {
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
                        "movement animation selection: shield action_state with \
                         unrecognised movement action",
                    );
                    None
                }
            };
            if let Some(want) = want {
                move_action = want;
            }
        }
        // The PC and Human overrides delegate to Actor's base implementation
        // unless their stamped shield/sword arms above consume the request.
        // Base movement-animation selection switches on the actor's *live*
        // action state.  This is how a soldier already moving with a shield
        // rewrites a newly instructed upright path even though the sequence
        // retained an older non-shield action-state stamp.
        if !sword_movement_context
            && !pc_stamped_shield_context
            && posture_after == crate::element::Posture::Upright
            && !owner_is_on_lift
            && live_action_state.is_shield()
        {
            move_action = OrderType::WalkingWithShield;
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
                // A PC can resume a movement whose authored action still
                // carries a sword token after QuitSwordfight lowered the
                // weapon. The original game's base movement-animation selection treats
                // that combination as an ordinary upright walk/run; NPCs
                // retain the token.
                let inner_arm = matches!(
                    action_after,
                    crate::element::ActionState::Waiting
                        | crate::element::ActionState::Bored
                        | crate::element::ActionState::Moving
                        | crate::element::ActionState::MovingFast
                        | crate::element::ActionState::Sleeping
                        | crate::element::ActionState::Listening
                ) || action_after.is_bow();
                if !owner_is_on_lift && inner_arm {
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
                        OrderType::WalkingWithSword if owner_is_pc => OrderType::WalkingUpright,
                        OrderType::RunningWithSword if owner_is_pc => OrderType::RunningUpright,
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
                        // any non-listed type.
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
        // Human movement-animation selection handles upright
        // sword states in the derived override and deliberately does not call
        // the actor's base behavior. The logical sword token is
        // therefore authoritative even in a lift sector (the Human Execute
        // override chooses the concrete combat row later).  This matters when
        // a postponed combat approach resumes on stairs: RUNNING_WITH_SWORD
        // must not collapse to the lift's ordinary WalkingStairs row.  Base
        // lift translation still applies to non-sword and authored climb
        // movement.
        if !sword_movement_context && !pc_stamped_shield_context {
            // The sword / shield / corpse movement tokens are only ever
            // assigned to an element whose post-transition posture is
            // Upright, so a movement that reaches a wall or ladder carries
            // the plain walk or run action and the lift sector answers a run
            // with the fast climb. Rust can still arrive here holding a
            // carried-over variant token; normalise it to the speed the
            // element is actually moving at before the lift translates it.
            let lift_input = if matches!(
                posture_after,
                crate::element::Posture::OnWall | crate::element::Posture::OnLadder
            ) {
                climb_lift_translation_input(move_action, is_fast)
            } else {
                move_action
            };
            move_action =
                self.determine_lift_movement_animation(owner, posture_after, lift_input, dest);
        }
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
                elem.posture_after_transition = entity.element_data().posture();
            }
        }

        // Read entity position / layer / sector / pathfinder index +
        // current move box + half diagonal (half diagonal drives the
        // thick-reachability pre-check below).
        let (mut source, entity_layer, entity_sector, pf_idx, move_box_map, half_diagonal) = {
            let entity = match self.world.entities.get(owner) {
                Some(e) => e,
                _ => return MovePathOutcome::ActorGone,
            };
            let elem = entity.element_data();
            let pi = entity.position_iface();
            let pf_idx = u16::from(pi.get_pathfinder_index().unwrap_or_else(|| {
                panic!("movement owner {owner:?} has no configured pathfinder index")
            }));
            (
                elem.position_map(),
                elem.layer(),
                elem.sector().map(u16::from).unwrap_or(0),
                pf_idx,
                *pi.get_move_box_map(),
                pi.get_half_diagonal(),
            )
        };

        // A PC disguised as an anonymous archer is pinned to its shooting
        // spot for the duration of the contest: the move is refused outright
        // and the hero complains instead of walking away.
        if owner_is_pc
            && self.world.entities.get(owner).is_some_and(|e| {
                e.element_data().posture() == crate::element::Posture::AnonymousArcher
            })
        {
            tracing::debug!(
                actor = ?owner,
                "try_dispatch_move_path: anonymous archer may not move",
            );
            self.hero_speaking(
                assets,
                owner,
                crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
            );
            return MovePathOutcome::Refused;
        }

        // Before queuing a path request, if the move is flagged
        // MAP / STRAIGHT, or the source→dest segment is
        // thick-reachable, skip the pathfinder entirely and emit a
        // single direct order.  The pathfinder is never invoked when
        // a straight line suffices.
        //
        // Without this pre-check, short clicks that are directly
        // walkable still hit A*, which can route the actor through
        // source-adjacent graph nodes (extra waypoints near
        // the last node) and produce the "keeps moving old
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
        let movement_goal_crosses_layer = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .is_some_and(|e| match &e.data {
                crate::sequence::SequenceElementData::Movement { layer, sector, .. } => {
                    sector.is_some() && *layer != entity_layer
                }
                _ => false,
            });
        let post_door_route_handoff = movement_goal_crosses_layer
            && has_deferred_post_door_route_continuation(
                &self.orders.sequence_manager,
                owner,
                source,
                entity_layer,
            );
        let current_layer_reachable =
            self.world
                .fast_grid
                .is_reachable_thick(source, dest, entity_layer, half_diagonal);
        // A normal explicit cross-layer goal must be routed. At the exact
        // terminal-door seam the stored goal layer can lag behind the actor,
        // so defer to the same current-layer thick-reachability result that
        // Original performs instead of forcing either outcome.
        let straight_ok = movement_path_dispatch_is_direct(
            move_flags,
            movement_goal_crosses_layer,
            post_door_route_handoff,
            current_layer_reachable,
        );

        // Before submitting a path request, check whether the actor's
        // move box is in an authorized position. Direct MAP / STRAIGHT /
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
        let source_authorized = straight_ok
            || self
                .world
                .fast_grid
                .is_position_authorized(&move_box_map, entity_layer);
        if path_request_needs_source_extraction(straight_ok, source_authorized) {
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
                //
                // Path request insertion stops the requesting actor with the
                // default priority, so the
                // stop priority is the declared default
                // normal priority, not waiting priority. The distinction decides whether the
                // incoming Normal-priority Move element is stopped at all:
                // sequence element only stops when its priority is at least the
                // stop priority, and normal priority (8)
                // is stronger than waiting priority (9)
                // in the original game's sequence cascade. With `Wait` the Move
                // survived, kept the actor's selection, and left the sprite's
                // map goal installed for one extra frame.
                tracing::warn!(
                    actor = ?owner,
                    src_x = source.x,
                    src_y = source.y,
                    layer = entity_layer,
                    "try_dispatch_move_path: actor cannot be extracted from obstacle (Stop + Wait)",
                );
                self.stop_owner(
                    sim,
                    assets,
                    &mut Vec::new(),
                    owner,
                    crate::sequence::SequencePriority::Normal,
                    &|engine, element| Self::priority_resolver(&engine.world.entities)(element),
                );
                let mut wait_elem = crate::sequence::SequenceElement::new(
                    1,
                    crate::element::Command::Wait,
                    Some(owner),
                );
                wait_elem.priority = crate::sequence::SequencePriority::Wait;
                let mut seq = crate::sequence::Sequence::new();
                seq.append_element(wait_elem);
                self.launch_sequence(sim, assets, seq);
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
            // The original game leaves the sector value uninitialized and never reads it.
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

        // Movement dispatch completes direct / straight moves
        // immediately, but converts only A*-requiring moves to MOVE_WAITING
        // and queues a path request.
        if !straight_ok {
            if let Some(elem) = self
                .orders
                .sequence_manager
                .get_element_mut(seq_id, elem_idx)
            {
                elem.command = crate::element::Command::MoveWaiting;
                elem.push_order(crate::order::Order::new(
                    OrderType::Freezing,
                    source.x,
                    source.y,
                    crate::order::alloc_order_id(&mut self.orders.next_order_id),
                ));
            }
            let parity_request = crate::pathfinder::parity_path_capture_is_active()
                .then(|| parity_path_request_state(&self.world.fast_grid, &request));
            self.trace_path_owner_lifecycle("before_path_enqueue", owner, Some((seq_id, elem_idx)));
            self.orders.pending_path_requests.enqueue(request);
            self.trace_path_owner_lifecycle("after_path_enqueue", owner, Some((seq_id, elem_idx)));
            if let Some(request) = parity_request {
                crate::pathfinder::record_parity_path_event(
                    crate::pathfinder::ParityPathEvent::Queued(request),
                );
            }
            return MovePathOutcome::Pending;
        }

        self.finish_move_path(sim, request, vec![source, dest]);
        MovePathOutcome::Success
    }

    pub(super) fn finish_move_path(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        request: PendingPathRequest,
        mut waypoints: Vec<MapPoint>,
    ) {
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

        let selected_pre_path_tail = self
            .orders
            .sequence_manager
            .current_order_for_actor(&self.world.entities, owner)
            .and_then(|(selected_seq, selected_idx, current)| {
                let element = self.orders.sequence_manager.get_element(seq_id, elem_idx)?;
                let tail_index = element.orders.len().checked_sub(1)?;
                let tail = element.orders.get(tail_index)?;
                let installed_matches = self
                    .world
                    .entities
                    .get(owner)
                    .and_then(Entity::actor_data)
                    .and_then(|actor| actor.installed_order)
                    .is_some_and(|installed| installed.order_id == current.order_id);
                (selected_seq == seq_id
                    && selected_idx == elem_idx
                    && current.order_id == tail.order_id
                    && installed_matches)
                    .then_some(tail_index)
            });

        // Path-request processing materializes movement orders beginning at
        // the first path index, so an unrequested raw source never becomes an
        // order seen by either actor or soldier path postprocessing.
        let raw_waypoint_count =
            prepare_path_waypoints_for_postprocess(&mut waypoints, use_first_point);

        // Drunken-soldier path deviation applies later, after actor
        // Path postprocessing has inserted its transition orders. The original game
        // soldier override skips those transitions and inserts midpoint
        // copies only before upright walking/running orders.
        let is_movement_anim = matches!(
            move_action,
            OrderType::WalkingUpright | OrderType::RunningUpright
        );

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
        // Path-request processing only stamps tolerance and antagonist when the
        // raw original-game path contains more than one point. Decide this before
        // removing a leading source waypoint: raw [source, goal] emits one
        // order with metadata, while a direct raw [goal] order keeps the
        // order defaults.
        let (final_order_tolerance, final_order_antagonist) =
            original_final_path_metadata(raw_waypoint_count, tolerance, antagonist);

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
        let mut rewritten_installed_order = None;
        {
            if let Some((elem, next_order_id)) =
                self.orders.element_with_order_ids_mut(seq_id, elem_idx)
            {
                // Fresh Rust movement elements retain their generated
                // transition prefix through `num_transition_orders`. A
                // restored original-game waiting movement instead owns the exact
                // serialized pre-path queue, whose last waiting order must be
                // reused in place when the saved request completes.
                // Path-request processing marks a resolved movement element as
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
                rewritten_installed_order = selected_pre_path_tail
                    .and_then(|tail_index| elem.orders.get(tail_index))
                    .map(|order| crate::element::InstalledActorOrder {
                        order_id: order.order_id,
                        order_type: order.order_type,
                    });
            }
        }

        if let Some(installed_order) = rewritten_installed_order {
            // Path-request processing reuses the selected movement's final
            // pre-path order, assigns a new ID, and changes its action in place.
            // Keep the explicit order mirror on that rewritten object; a
            // later path postprocessing may insert other orders ahead of it but
            // does not repoint the order until the next actor update.
            self.world
                .entities
                .get_mut(owner)
                .and_then(Entity::actor_data_mut)
                .expect("resolved path owner lost actor data")
                .installed_order = Some(installed_order);
        }

        // Splice startup / end transitions into the order queue
        // based on the actor's posture + action state.
        self.post_process_path(seq_id, elem_idx);

        if is_movement_anim && !is_pass_door {
            let (blood_alcohol, half_diagonal, move_box) = self
                .world
                .entities
                .get(owner)
                .and_then(|entity| {
                    let blood_alcohol = entity
                        .npc_data()
                        .and_then(|npc| npc.ai_brain.base())?
                        .blood_alcohol;
                    let position = entity.position_iface();
                    Some((
                        blood_alcohol,
                        position.get_half_diagonal(),
                        *position.get_move_box(),
                    ))
                })
                .unwrap_or_default();
            if blood_alcohol > 0 {
                let grid = &self.world.fast_grid;
                if let Some((element, next_order_id)) =
                    self.orders.element_with_order_ids_mut(seq_id, elem_idx)
                {
                    crate::engine::tick::apply_drunken_order_deviation(
                        sim,
                        element,
                        source,
                        blood_alcohol,
                        move_action == OrderType::RunningUpright,
                        entity_layer,
                        &move_box,
                        half_diagonal,
                        &grid,
                        next_order_id,
                    );
                }
            }
        }
    }
}

/// Append the PC arrival bark after all movement at the next command level.
///
/// The original game's movement passes the step count through
/// movement construction; every appended movement consumes the current value
/// and increments it before the destination bark is constructed. Keeping the
/// bark parallel with a
/// pathfinding Move makes its immediate termination complete the whole level,
/// killing the new `MoveWaiting` and cancelling its queued request.
fn append_arrival_speech(sequence: &mut crate::sequence::Sequence, owner: EntityId) {
    let level = sequence
        .last()
        .unwrap_or_else(|| panic!("arrival speech requires a preceding movement element"))
        .command_level
        .saturating_add(1);
    sequence.append_element(crate::sequence::SequenceElement::new(
        level,
        crate::element::Command::SpeakHeroReachDestination,
        Some(owner),
    ));
}

impl EngineInner {
    pub(in crate::engine) fn advance_live_order_after_terminal_handoff(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        if let Some((seq_id, elem_idx)) = self.world.entities.current_element_for_actor(owner) {
            let exhausts_pending_move = self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .is_some_and(|element| {
                    element.command == crate::element::Command::MoveWaiting
                        && element.orders.len() == 1
                });
            if exhausts_pending_move {
                // The live order advancement below exhausts the re-entrantly
                // selected movement sequence element. Its original-game
                // Termination teardown cancels the path request, so
                // the retained logical queue head must still complete one
                // frame later with valid=false and an empty raw path
                // because ignored path work exits before producing a result.
                self.world.pathfinder.cancel_requests_for(owner);
                self.orders.pending_path_requests.cancel_for_owner(owner);
                self.orders
                    .failed_path_requests
                    .retain(|request| request.owner != owner);
            }
            if debug_post_seek_handoff_enabled() {
                let command_and_orders = self
                    .orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .map(|element| (element.command, element.orders.len()));
                eprintln!(
                    "[POST_SEEK frame={} owner={owner:?} stage=live_advance target={:?} command_and_orders={command_and_orders:?} exhausts_pending_move={exhausts_pending_move}]",
                    self.control.frame_counter,
                    (seq_id, elem_idx),
                );
            }
            self.do_next_order(sim, assets, seq_id, elem_idx);
        } else if debug_post_seek_handoff_enabled() {
            eprintln!(
                "[POST_SEEK frame={} owner={owner:?} stage=live_advance_no_current]",
                self.control.frame_counter,
            );
        }
    }
}

/// Drunken turning happens before motion. On a fresh order this therefore
/// observes the
/// retained direction goal from the previous motion; motion processing installs
/// the new order's goal afterwards through increment computation.
fn turn_drunken(pi: &mut crate::position_interface::PositionInterface) {
    let current = u16::from(pi.get_direction());
    let goal = u16::from(pi.get_direction_goal());
    if crate::engine::soldier_helpers::turn_drunken_is_very_slow(current, goal) {
        pi.turn_very_slow();
    } else {
        pi.turn_slow(2);
    }
}

fn should_apply_drunken_turn(
    selected_uses_seek: bool,
    order_action: crate::order::OrderType,
) -> bool {
    !selected_uses_seek && order_action == crate::order::OrderType::WalkingUpright
}

fn should_apply_plain_movement_turn(
    is_drunken_soldier: bool,
    flags: crate::sequence::MoveFlags,
    order_action: crate::order::OrderType,
) -> bool {
    !is_drunken_soldier
        || flags.contains(crate::sequence::MoveFlags::SEEK)
        || order_action != crate::order::OrderType::WalkingUpright
}

#[cfg(test)]
mod tests;
