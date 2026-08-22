//! Movement ticking, pathfinding dispatch, and order processing.

use super::*;
use crate::coordinates::{MapBBox, MapPoint, MapVec};
use crate::element::{ActiveDoorPass, EntityId};
use crate::entities::EntitySlots;
use crate::movement::ActiveMovement;
use crate::order::OrderType;
use crate::position_interface::vector_to_sector_0_to_15;
use crate::sprite::{FrameProgression, MotionMethod, MotionOrderContext, MotionState};

#[inline]
fn debug_post_seek_handoff_enabled() -> bool {
    std::env::var_os("PARITY_DEBUG_POST_SEEK_HANDOFF").is_some()
}

#[derive(Debug, Clone, Copy)]
enum MovementPopGoalOwnerKind {
    Pc,
    Soldier,
    Civilian,
}

#[derive(Debug, Clone, Copy)]
struct MovementPopGoalOwnerDebugConfig {
    frame: u32,
    kind: MovementPopGoalOwnerKind,
    index: u32,
}

fn movement_pop_goal_owner_debug_config() -> Option<&'static MovementPopGoalOwnerDebugConfig> {
    static CONFIG: std::sync::OnceLock<Option<MovementPopGoalOwnerDebugConfig>> =
        std::sync::OnceLock::new();
    CONFIG
        .get_or_init(|| {
            std::env::var_os("PARITY_DEBUG_GOAL_OWNER_HANDOFF")?;
            let frame = std::env::var("PARITY_DEBUG_GOAL_OWNER_FRAME").unwrap_or_else(|_| {
                panic!(
                    "PARITY_DEBUG_GOAL_OWNER_HANDOFF requires PARITY_DEBUG_GOAL_OWNER_FRAME=FRAME"
                )
            });
            let frame = frame.parse::<u32>().unwrap_or_else(|error| {
                panic!("invalid PARITY_DEBUG_GOAL_OWNER_FRAME={frame:?}: {error}")
            });
            let owner = std::env::var("PARITY_DEBUG_GOAL_OWNER").unwrap_or_else(|_| {
                panic!(
                    "PARITY_DEBUG_GOAL_OWNER_HANDOFF requires PARITY_DEBUG_GOAL_OWNER=pc|soldier|civilian:INDEX"
                )
            });
            let (kind, index) = owner.split_once(':').unwrap_or_else(|| {
                panic!("PARITY_DEBUG_GOAL_OWNER must look like pc|soldier|civilian:INDEX")
            });
            let kind = match kind {
                "pc" => MovementPopGoalOwnerKind::Pc,
                "soldier" => MovementPopGoalOwnerKind::Soldier,
                "civilian" => MovementPopGoalOwnerKind::Civilian,
                unsupported => {
                    panic!("PARITY_DEBUG_GOAL_OWNER has unsupported kind {unsupported:?}")
                }
            };
            let index = index.parse::<u32>().unwrap_or_else(|error| {
                panic!("invalid PARITY_DEBUG_GOAL_OWNER={owner:?}: {error}")
            });
            Some(MovementPopGoalOwnerDebugConfig { frame, kind, index })
        })
        .as_ref()
}

fn movement_pop_goal_owner_debug_matches(frame: u32, owner: EntityId) -> bool {
    let Some(config) = movement_pop_goal_owner_debug_config() else {
        return false;
    };
    config.frame == frame
        && config.index == owner.index()
        && matches!(
            (config.kind, owner),
            (MovementPopGoalOwnerKind::Pc, EntityId::Pc(_))
                | (MovementPopGoalOwnerKind::Soldier, EntityId::Soldier(_))
                | (MovementPopGoalOwnerKind::Civilian, EntityId::Civilian(_))
        )
}

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

/// Original `RHElementActor::InstructOwner(RHCOMMAND_MOVE)` direct-dispatch
/// predicate. `RHMOVE_LINE` changes post-processing of the resulting path; it
/// does not by itself bypass `RHPathFinder::AddPathRequest`.
#[inline]
fn movement_flags_force_direct_dispatch(flags: crate::sequence::MoveFlags) -> bool {
    flags.contains(crate::sequence::MoveFlags::MAP)
        || flags.contains(crate::sequence::MoveFlags::STRAIGHT)
}

/// `RHPathFinder::AddPathRequest` runs this gate for every request it receives;
/// command type and actor posture do not provide bypasses.
#[inline]
fn path_request_needs_source_extraction(direct_dispatch: bool, source_authorized: bool) -> bool {
    !direct_dispatch && !source_authorized
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ActorPostSeekInteraction {
    Hit,
    Tie,
}

/// Identify an actor-owned interaction whose init-time 40-unit validity
/// guard immediately follows a completed entity Seek.
///
/// The copied terminal movement can lose its own target, but the Original
/// retains one `mpSeekTarget` pointer on the actor. Scope the early handoff to
/// a post-seek interaction with that exact antagonist; an unrelated tail must
/// remain on the ordinary sequence-manager path.
fn actor_post_seek_interaction(
    actor: &crate::element::ActorData,
) -> Option<ActorPostSeekInteraction> {
    let element = actor
        .post_seek_sequence
        .as_deref()
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
        _ => None,
    }
}

/// Original Hit and Tie initialization compare the raw map-space
/// `SBGeoVector2D::SquareNorm()` with 1600
/// (`RHelementactorhuman.cpp:6653-6678`, `RHelementactorpc.cpp:7157-7181`).
/// Keep the strict comparison: a victim at exactly 40 units is valid.
fn interaction_exceeds_init_range(owner: MapPoint, victim: MapPoint) -> bool {
    let dx = victim.x - owner.x;
    let dy = victim.y - owner.y;
    dx * dx + dy * dy > 1600.0
}

#[cfg(test)]
mod post_seek_hit_handoff_tests {
    use super::*;

    #[test]
    fn hit_init_range_uses_raw_map_square_norm_and_includes_exact_boundary() {
        let owner = MapPoint::new(1099.375_2, 1823.835_4);
        let nescafe_target = MapPoint::new(1055.0, 1790.0);
        assert!(interaction_exceeds_init_range(owner, nescafe_target));

        let owner = MapPoint::new(1483.855_8, 2720.03);
        let cyrdach_target = MapPoint::new(1470.0, 2759.0);
        assert!(interaction_exceeds_init_range(owner, cyrdach_target));

        assert!(!interaction_exceeds_init_range(
            MapPoint::ZERO,
            MapPoint::new(40.0, 0.0)
        ));
        assert!(interaction_exceeds_init_range(
            MapPoint::ZERO,
            MapPoint::new(f32::from_bits(40.0f32.to_bits() + 1), 0.0)
        ));
    }
}

/// Input passed to a ladder/wall lift's action translation.
///
/// Original `DetermineMovementAnimation` passes an authored upright walk/run
/// action through verbatim: the lift itself maps `RunningUpright` to its fast
/// climb row, independently of `RHMOVE_FAST`. Rust can also reach this point
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

#[cfg(test)]
mod group_move_authorization_tests {
    use super::*;
    use crate::coordinates::MoveBox;
    use crate::sector::SectorType;

    #[test]
    fn ordinary_formation_uses_live_actor_box_not_generic_upright_box() {
        let bbox = group_move_candidate_box(
            MapBBox::from_coords(90.0, 90.0, 110.0, 110.0),
            MoveBox::from_coords(-2.0, -2.0, 2.0, 2.0),
            MapPoint::new(100.0, 100.0),
            MapPoint::new(200.0, 220.0),
            false,
        );
        assert_eq!((bbox.x_min(), bbox.y_min()), (190.0, 210.0));
        assert_eq!((bbox.x_max(), bbox.y_max()), (210.0, 230.0));
    }

    #[test]
    fn ordinary_formation_preserves_live_box_offset_from_actor_position() {
        let bbox = group_move_candidate_box(
            MapBBox::from_coords(94.0, 97.0, 109.0, 112.0),
            MoveBox::from_coords(-20.0, -20.0, 20.0, 20.0),
            MapPoint::new(100.0, 100.0),
            MapPoint::new(300.0, 400.0),
            false,
        );
        assert_eq!((bbox.x_min(), bbox.y_min()), (294.0, 397.0));
        assert_eq!((bbox.x_max(), bbox.y_max()), (309.0, 412.0));
    }

    #[test]
    fn mercenary_box_preserves_original_float_operation_order() {
        // Savegame_032/replay-006 exposed this exact boundary: collapsing
        // `(box - center) + click` into `box + (click - center)` rounds the
        // final X coordinate down by one ULP.
        let actor_x = f32::from_bits(1_151_945_109);
        let click_x = f32::from_bits(1_124_501_081);
        let actor = MapPoint::new(actor_x, 688.9211);
        let click = MapPoint::new(click_x, 489.68);
        let live_box =
            MapBBox::from_coords(actor.x - 6.0, actor.y - 4.0, actor.x + 6.0, actor.y + 4.0);

        let source_order = group_move_mercenary_box(
            live_box,
            MoveBox::from_coords(-6.0, -4.0, 6.0, 4.0),
            actor,
            actor,
            click,
            false,
        );
        let collapsed = live_box.translated(click - actor);

        assert_eq!(source_order.center().x.to_bits(), 1_124_501_081);
        assert_eq!(collapsed.center().x.to_bits(), 1_124_501_080);
    }

    #[test]
    fn lift_formation_uses_upright_zero_centered_box() {
        let bbox = group_move_candidate_box(
            MapBBox::from_coords(90.0, 90.0, 110.0, 110.0),
            MoveBox::from_coords(-3.0, -4.0, 5.0, 6.0),
            MapPoint::new(100.0, 100.0),
            MapPoint::new(300.0, 400.0),
            true,
        );
        assert_eq!((bbox.x_min(), bbox.y_min()), (297.0, 396.0));
        assert_eq!((bbox.x_max(), bbox.y_max()), (305.0, 406.0));
    }

    #[test]
    fn replay_goal_sector_kind_retains_lift_door_and_jump_flags() {
        assert_eq!(
            group_move_sector_kinds(SectorType::LIFT),
            (true, false, false)
        );
        assert_eq!(
            group_move_sector_kinds(SectorType::DOOR),
            (false, true, false)
        );
        assert_eq!(
            group_move_sector_kinds(SectorType::JUMP),
            (false, false, true)
        );
    }

    #[test]
    fn recorded_route_goal_remains_independent_of_coincident_selected_overlay() {
        // Savegame_linux3/Profile003/Savegame008/replay018 frame 16221:
        // the selected overlay resolves at the click independently, while
        // RecordGroupMove's patch-aware pSectorGoal remains sector 288/L4.
        // Losing the recorded identity turns the command into a same-sector
        // move; preserving it reaches gate A*, whose failure leaves the old
        // Wait sequence installed just as AppendMoveToSequence does.
        let selected_overlay = Some(crate::sector::SectorNumber::new(33));
        let recorded = Some((crate::sector::SectorNumber::new(288), 4));

        assert_eq!(
            group_move_route_goal(recorded, selected_overlay, 0),
            (Some(crate::sector::SectorNumber::new(288)), 4)
        );
        assert_eq!(
            group_move_sector_kinds(SectorType::MOTION),
            (false, false, false),
            "selected-sector semantics are still derived from the overlay"
        );
    }

    #[test]
    fn player_group_move_uses_resolved_upright_click_action() {
        assert_eq!(player_group_move_action(false), OrderType::WalkingUpright);
        assert_eq!(player_group_move_action(true), OrderType::RunningUpright);
    }

    #[test]
    fn non_sprite_movement_actions_return_authoritative_motion_states() {
        assert_eq!(
            non_sprite_movement_motion(OrderType::Freezing),
            Some(MotionState::InProgress)
        );
        assert_eq!(
            non_sprite_movement_motion(OrderType::PassingDoor),
            Some(MotionState::Terminated)
        );
        assert_eq!(non_sprite_movement_motion(OrderType::WalkingUpright), None);
    }

    #[test]
    fn authored_running_action_stays_fast_on_climb_without_fast_flag() {
        assert_eq!(
            climb_lift_translation_input(OrderType::RunningUpright, false),
            OrderType::RunningUpright
        );
        assert_eq!(
            crate::sector::LiftType::Ladder.translate_climb_action(
                climb_lift_translation_input(OrderType::RunningUpright, false),
                false,
            ),
            OrderType::ClimbingLadderUpFast
        );
    }

    #[test]
    fn concrete_door_walk_replaces_a_retired_transition_mirror() {
        let mut mirrored = OrderType::TransitionWalkingUprightRunningUpright;
        synchronize_selected_door_pass_walk_action(&mut mirrored, OrderType::RunningUpright);
        assert_eq!(mirrored, OrderType::RunningUpright);

        synchronize_selected_door_pass_walk_action(&mut mirrored, OrderType::PassingDoor);
        assert_eq!(
            mirrored,
            OrderType::RunningUpright,
            "a non-animation action point must leave the last sprite action intact"
        );
    }

    #[test]
    fn exhausted_transition_discards_zero_destination_door_tail() {
        let mut pass = ActiveDoorPass {
            door_index: crate::gate::DoorIndex(67),
            direct: false,
            position_direct: false,
            steps: [crate::element::DoorPassStep::PassingDoor].into(),
            triggers_fired: 1,
            current_action: OrderType::TransitionWalkingUprightRunningUpright,
            current_reverse: false,
            saved_action_state: None,
        };

        discard_lazy_door_pass_following_orders(Some(&mut pass));

        assert!(
            pass.steps.is_empty(),
            "Original deletes the trailing zero-destination PassingDoor order"
        );
        assert_eq!(
            completed_door_pass_to_commit(true, Some((crate::gate::DoorIndex(67), false))),
            None,
            "a deleted final PassingDoor cannot snap the actor to the authored door endpoint"
        );
        assert_eq!(
            completed_door_pass_to_commit(false, Some((crate::gate::DoorIndex(67), false))),
            Some((crate::gate::DoorIndex(67), false)),
            "an ordinarily completed door pass still performs its final position commit"
        );
    }

    #[test]
    fn explicit_door_speed_transition_ignores_stale_walk_mirror() {
        assert_eq!(
            door_pass_sprite_animation_override(
                OrderType::TransitionWaitingUprightRunningUpright,
                Some(OrderType::WalkingUpright),
            ),
            None,
            "Original executes the concrete transition inserted by MakeFast"
        );
        assert_eq!(
            door_pass_sprite_animation_override(
                OrderType::RunningUpright,
                Some(OrderType::WalkingUpright),
            ),
            Some(OrderType::WalkingUpright),
            "concrete distance motion still accepts the active door-route animation"
        );
        assert_eq!(
            door_pass_sprite_animation_override(
                OrderType::TransitionWaitingUprightClimbingWallUp,
                Some(OrderType::TransitionWaitingUprightClimbingWallUp),
            ),
            Some(OrderType::TransitionWaitingUprightClimbingWallUp),
            "an agreeing door-authored transition mirror remains valid"
        );
    }

    #[test]
    fn recursively_reached_climb_keeps_transition_facing() {
        assert!(!initialising_climb_uses_lift_direction(
            OrderType::ClimbingLadderUp,
            crate::sector::LiftType::Ladder,
            false,
        ));
        assert!(initialising_climb_uses_lift_direction(
            OrderType::ClimbingLadderUp,
            crate::sector::LiftType::Ladder,
            true,
        ));
    }
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
        // Original's default posture arm still applies the lift's upright
        // action translation (RHelementactor.cpp:4735-4745). This matters for
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

#[derive(Clone, Copy, Debug)]
struct RiderChargeExecution {
    /// Identity of the same order object after Execute returned. Rider charge
    /// may legitimately assign that object a fresh ID on its last animation
    /// frame. `None` means a synchronous callback replaced the entry object
    /// while Execute was still running.
    completion_order_id: Option<std::num::NonZeroU32>,
}

#[cfg(test)]
thread_local! {
    static LAST_MOBILE_CROSSING_INCREMENT: std::cell::Cell<Option<MapVec>> = const { std::cell::Cell::new(None) };
    static POST_EXECUTE_CROSSING_OBSERVER: std::cell::RefCell<Option<Box<dyn FnMut(&mut EngineInner, EntityId)>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(super) fn take_last_mobile_crossing_increment() -> Option<MapVec> {
    LAST_MOBILE_CROSSING_INCREMENT.with(|increment| increment.take())
}

#[cfg(test)]
pub(super) fn set_post_execute_crossing_observer(
    observer: Option<Box<dyn FnMut(&mut EngineInner, EntityId)>>,
) {
    POST_EXECUTE_CROSSING_OBSERVER.with(|slot| *slot.borrow_mut() = observer);
}

#[cfg(test)]
fn observe_post_execute_crossing(engine: &mut EngineInner, entity_id: EntityId) {
    POST_EXECUTE_CROSSING_OBSERVER.with(|slot| {
        if let Some(observer) = slot.borrow_mut().as_mut() {
            observer(engine, entity_id);
        }
    });
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

/// Keep the split door-pass walk mirror aligned with the concrete order that
/// has reached the actor slot.
///
/// Original stores the translated door route and posture-transition copies in
/// one order list. Once a transition retires, the following walk/run order is
/// immediately authoritative. Rust keeps the untranslated tail in
/// `ActiveDoorPass`; without this rebind a transition written into
/// `current_action` by MakeFast/MakeSlow can continue supplying the sprite row
/// while the concrete successor is already executing.
fn synchronize_selected_door_pass_walk_action(
    current_action: &mut OrderType,
    selected_action: OrderType,
) {
    if order_uses_distance_motion(selected_action) {
        *current_action = selected_action;
    }
}

/// Mirror `RHSprite::PerformMotion(TILL_LAST_FRAME)` deleting every following
/// order when a transition loops short of its goal and cannot find a later
/// distinct animation with a nonzero destination.
///
/// Original's translated door route lives in that one order list
/// (`RHsprite.cpp:1844-1881`). Rust stores its untranslated suffix separately,
/// so the same deletion must also empty `ActiveDoorPass::steps` or a discarded
/// zero-destination `PASSING_DOOR` action point will be materialized again.
fn discard_lazy_door_pass_following_orders(pass: Option<&mut ActiveDoorPass>) {
    if let Some(pass) = pass {
        pass.steps.clear();
    }
}

/// Insert a materialized lazy door-pass step at the same side of a copied
/// transition-distance continuation as Original's single translated order
/// list.
///
/// `PerformMotion(TILL_LAST_FRAME)` inserts the copied continuation immediately
/// before the first later, distinct animation with a nonzero destination. Rust
/// stores the orders preceding that animation (PassingDoor and zero-target
/// posture transitions) in `ActiveDoorPass`, so those steps must be inserted
/// before the concrete continuation. The matching authored walk belongs after
/// it.
fn insert_door_pass_successor(
    element: &mut crate::sequence::SequenceElement,
    order: crate::order::Order,
) {
    let continuation = element
        .orders
        .iter()
        .position(|queued| queued.transition_distance_continuation);
    let Some(continuation) = continuation else {
        element.push_order(order);
        return;
    };
    let copied = &element.orders[continuation];
    let is_matching_authored_walk =
        (order.target_x != 0.0 || order.target_y != 0.0) && order.order_type == copied.order_type;
    let insertion = continuation + usize::from(is_matching_authored_walk);
    element.insert_order(insertion, order);
}

fn completed_door_pass_to_commit(
    discarded_following_orders: bool,
    completed: Option<(crate::gate::DoorIndex, bool)>,
) -> Option<(crate::gate::DoorIndex, bool)> {
    (!discarded_following_orders).then_some(completed).flatten()
}

/// Ignore a stale split door-route mirror when a distinct concrete transition
/// has reached the actor slot.
///
/// Original keeps the whole translated route in one order list, so an
/// explicit transition inserted by MakeFast/MakeSlow is authoritative. Rust's
/// mirror remains useful for concrete distance motion and for door-authored
/// transitions where it already agrees with the selected order.
fn door_pass_sprite_animation_override(
    selected_action: OrderType,
    current_action: Option<OrderType>,
) -> Option<OrderType> {
    current_action.filter(|current| {
        order_uses_distance_motion(selected_action) || *current == selected_action
    })
}

/// Movement Execute arms which call `Turn()` immediately before entering
/// `RHSprite::PerformMotion`.
///
/// This distinction remains observable under `FreezeAll`: the sprite returns
/// `RHMOTION_IN_PROGRESS` before animation or displacement, but the actor-side
/// turn has already happened (`RHelementactor.cpp:1142-1341`).
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

/// Match `RHSprite::PerformMotion`: scale the sprite-frame distance by the
/// movement element's speed factor before applying the turn slowdown and its
/// minimum useful step. Direct transition orders call `PerformMotion` without
/// the element speed factor, while seek transitions route through
/// `RHElementActor::PerformSeek`, which passes the element factor explicitly.
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

/// Lift-wall and ladder orders are literal `RHElementActor::Execute` dispatch
/// actions, including their start/landing transitions. A split Rust door
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
/// `PerformMotion` call terminates. `RunningStairs` also executes two motion
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

/// Whether an actor climb order applies the lift's fixed facing.
///
/// Original `RHElementActor::Execute` does this only inside
/// `IsInitialisation()`. A climb order reached recursively after a door step
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

/// Posture owned eagerly by a lift animation while executing a door-pass
/// step. Wall-exit transitions are different from the climb rows: Original
/// `RHElementActor::Execute` only inherits `OnWall` when the transition is
/// initialized, then its raw `DONE` edge is allowed to publish the landing
/// posture while the animation wrapper remains installed.
fn door_pass_eager_posture(
    action: OrderType,
    has_door_pass_animation: bool,
    execute_order_initialising: bool,
    decorative_building_trap_at_destination: bool,
) -> Option<crate::element::Posture> {
    use crate::element::Posture;

    if !has_door_pass_animation || decorative_building_trap_at_destination {
        return None;
    }
    match action {
        OrderType::ClimbingWallUp
        | OrderType::ClimbingWallDown
        | OrderType::ClimbingWallUpFast
        | OrderType::ClimbingWallDownFast => Some(Posture::OnWall),
        OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel
        | OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel
            if execute_order_initialising =>
        {
            Some(Posture::Flying)
        }
        OrderType::TransitionClimbingWallUpWaitingCrouched
        | OrderType::TransitionClimbingWallDownWaitingUpright
            if execute_order_initialising =>
        {
            Some(Posture::OnWall)
        }
        OrderType::ClimbingLadderUp
        | OrderType::ClimbingLadderDown
        | OrderType::ClimbingLadderUpFast
        | OrderType::ClimbingLadderDownFast => Some(Posture::OnLadder),
        _ => None,
    }
}

#[cfg(test)]
mod door_pass_posture_tests {
    use super::*;
    use crate::element::{Command, Posture};
    use crate::order::Order;
    use crate::sequence::SequenceElement;

    #[test]
    fn recursively_reached_wall_exit_preserves_its_done_posture() {
        let transition = OrderType::TransitionClimbingWallUpWaitingCrouched;

        assert_eq!(
            door_pass_eager_posture(transition, true, true, false),
            Some(Posture::OnWall),
            "an initializing wall-exit transition inherits the climb posture"
        );
        assert_eq!(
            door_pass_eager_posture(transition, true, false, false),
            None,
            "a recursively reached Execute must not stomp the crouched posture published by DONE"
        );
        assert_eq!(
            door_pass_eager_posture(OrderType::ClimbingWallUp, true, false, false),
            Some(Posture::OnWall),
            "ordinary wall-climb rows continue owning their posture on every Execute"
        );
    }

    #[test]
    fn crenel_and_sibling_lift_transitions_keep_their_selected_sprite_action() {
        use OrderType as OT;

        // Interactive session 002, PC 126, frame 707: the selected crenel
        // transition is action 255 while the split door mirror still names
        // the preceding climb. Original dispatches action 255 literally.
        assert_eq!(
            literal_lift_sprite_action(OT::TransitionClimbingWallUpWaitingCrouchedCrenel),
            Some(OT::TransitionClimbingWallUpWaitingCrouchedCrenel)
        );

        for sibling in [
            OT::TransitionWaitingUprightClimbingWallUp,
            OT::TransitionClimbingWallUpWaitingCrouched,
            OT::TransitionWaitingCrouchedClimbingWallDown,
            OT::TransitionWaitingCrouchedClimbingWallDownCrenel,
            OT::TransitionClimbingWallDownWaitingUpright,
            OT::TransitionWaitingUprightClimbingLadderUp,
            OT::TransitionWaitingUprightClimbingLadderUpAlerted,
            OT::TransitionClimbingLadderUpWaitingCrouched,
            OT::TransitionClimbingLadderUpWaitingUprightAlerted,
            OT::TransitionWaitingCrouchedClimbingLadderDown,
            OT::TransitionWaitingUprightClimbingLadderDownAlerted,
            OT::TransitionClimbingLadderDownWaitingUpright,
            OT::TransitionClimbingLadderDownWaitingUprightAlerted,
        ] {
            assert_eq!(literal_lift_sprite_action(sibling), Some(sibling));
        }

        assert_eq!(
            door_pass_sprite_animation_override(
                OT::TransitionClimbingWallUpWaitingCrouchedCrenel,
                Some(OT::ClimbingWallUp),
            ),
            None,
            "a stale split-route mirror must not replace the selected transition"
        );
        assert_eq!(literal_lift_sprite_action(OT::PassingDoor), None);
        assert_eq!(literal_lift_sprite_action(OT::WalkingUpright), None);
    }

    #[test]
    fn lazy_door_steps_keep_original_position_around_copied_continuation() {
        let owner = EntityId::Pc(crate::entity_id::PcId(1));
        let mut element = SequenceElement::new_movement(
            1,
            Command::PassDoor,
            Some(owner),
            OrderType::WalkingUpright,
        );
        element.orders.clear();
        element.orders.push_back(Order::new(
            OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel,
            20.0,
            30.0,
            std::num::NonZeroU32::new(1).unwrap(),
        ));
        let mut copied_walk = Order::new(
            OrderType::WalkingUpright,
            20.0,
            30.0,
            std::num::NonZeroU32::new(2).unwrap(),
        );
        copied_walk.transition_distance_continuation = true;
        element.orders.push_back(copied_walk);

        insert_door_pass_successor(
            &mut element,
            Order::new(
                OrderType::PassingDoor,
                0.0,
                0.0,
                std::num::NonZeroU32::new(3).unwrap(),
            ),
        );
        insert_door_pass_successor(
            &mut element,
            Order::new(
                OrderType::TransitionCrouchingUp,
                0.0,
                0.0,
                std::num::NonZeroU32::new(4).unwrap(),
            ),
        );
        insert_door_pass_successor(
            &mut element,
            Order::new(
                OrderType::WalkingUpright,
                20.0,
                30.0,
                std::num::NonZeroU32::new(5).unwrap(),
            ),
        );

        assert_eq!(
            element
                .orders
                .iter()
                .map(|order| order.order_type)
                .collect::<Vec<_>>(),
            vec![
                OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel,
                OrderType::PassingDoor,
                OrderType::TransitionCrouchingUp,
                OrderType::WalkingUpright,
                OrderType::WalkingUpright,
            ],
            "zero-target door steps precede the copied walk, while the authored matching walk follows it"
        );
        assert!(element.orders[3].transition_distance_continuation);
        assert!(!element.orders[4].transition_distance_continuation);
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

fn is_sword_motion_context(
    action_state: crate::element::ActionState,
    door_pass_action: Option<OrderType>,
    order_action: OrderType,
) -> bool {
    let selected_action =
        door_pass_sprite_animation_override(order_action, door_pass_action).unwrap_or(order_action);
    let stale_sword_state = matches!(
        action_state,
        crate::element::ActionState::MovingSword | crate::element::ActionState::MovingFastSword
    );

    // Human's FaceOpponent override owns only its literal sword-movement
    // Execute arms. DetermineMovementAnimation performs the state-dependent
    // WALKING_UPRIGHT -> WALKING_WITH_SWORD rewrite while translating an
    // ordinary movement element (`RHelementactorhuman.cpp:2365-2401`). A
    // PassDoor step is already a concrete order selected by the door
    // translator; Execute dispatches that literal action. Thus neither an
    // ordinary WalkingUpright door step nor a wall/ladder transition may be
    // reclassified from the preceding step's still-latched MOVING_SWORD
    // state. Only non-door ordinary movement retains that translation-time
    // fallback in Rust, where the successor START effect has not yet
    // published its new state.
    let stale_state_converts_selected_action = door_pass_action.is_none()
        && matches!(
            selected_action,
            OrderType::WalkingUpright | OrderType::RunningUpright | OrderType::WalkingWithCorpse
        );
    is_sword_movement_nonanimation(selected_action)
        || stale_sword_state && stale_state_converts_selected_action
}

/// Whether the selected logical action enters Human's sword-movement Execute
/// arm. The broader sword-motion context also retains the outgoing live state
/// for facing and sprite selection, but that stale state does not own START
/// side effects once an ordinary successor is selected.
fn executes_sword_movement_action(
    door_pass_action: Option<OrderType>,
    order_action: OrderType,
) -> bool {
    let action =
        door_pass_sprite_animation_override(order_action, door_pass_action).unwrap_or(order_action);
    is_sword_movement_nonanimation(action)
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

/// Signed angle from the movement displacement to the actor's facing vector,
/// as `RHElementActorHuman::FaceOpponent` measures it, normalised to
/// `[0, 2π)`.
///
/// This reproduces the determinant/dot form rather than differencing two
/// `atan2` results, because the two disagree once the determinant vanishes.
/// A degenerate displacement — an order whose destination is already the
/// actor's position — makes both terms zero and yields a half turn, so the
/// actor walks backwards rather than inheriting whatever direction it happens
/// to be facing.
fn combat_movement_angle(displacement: (f32, f32), facing: (f32, f32)) -> f32 {
    let (dx, dy) = displacement;
    let (fx, fy) = facing;
    let dot = dx * fx + dy * fy;
    let det = dx * fy - dy * fx;
    let mut angle = if det == 0.0 {
        if dot > 0.0 { 0.0 } else { std::f32::consts::PI }
    } else {
        // The ratio is formed in single precision before the arc tangent,
        // so an overflow to infinity here still resolves to a quarter turn.
        let raw = f64::from(det / dot).atan() as f32;
        if dot >= 0.0 {
            raw
        } else if det > 0.0 {
            raw + std::f32::consts::PI
        } else {
            raw - std::f32::consts::PI
        }
    };
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
            // FaceDangerPoint's four comparisons all reject NaN, then its
            // release-build fallback returns WALKING_SHIELD.
            if angle.is_nan() {
                return OrderType::WalkingShield;
            }
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
            // FaceOpponent likewise falls through its disabled assertion to
            // WALKING_SWORD when unchecked normalization produced a NaN goal.
            if angle.is_nan() {
                return OrderType::WalkingSword;
            }
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
        // state unconditionally after PerformSeek/PerformMotion. A fresh
        // short run can therefore return Terminated without ever exposing
        // Start and must still leave the actor MovingFast.
        (OT::RunningUpright, _) => Some((P::Upright, AS::MovingFast)),
        (OT::WalkingWithSword, MS::Start) => Some((P::Upright, AS::MovingSword)),
        (OT::RunningWithSword, MS::Start) => Some((P::Upright, AS::MovingFastSword)),
        // The PC WalkingWithShield Execute arm stamps MovingShield after
        // every PerformSeek/PerformMotion result, then replaces it with
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

fn take_transition_distance_first_execute(transition_distance_continuation: &mut bool) -> bool {
    std::mem::take(transition_distance_continuation)
}

fn take_deferred_movement_state_start(deferred_movement_state_start: &mut bool) -> bool {
    std::mem::take(deferred_movement_state_start)
}

fn should_defer_pc_movement_state_start(is_pc: bool, entity_target_seek: bool) -> bool {
    is_pc && !entity_target_seek
}

/// The shipped game clears an anti-vibration deviation latch when an in-place
/// movement startup transition starts. Both PCs and NPCs distinguish the two
/// upright handoffs: a waiting-to-walking/running startup retires the
/// preceding movement's latch, while a walking-to-waiting exit preserves it
/// for the following `Turn`.
#[inline]
fn should_clear_deviated_for_aligned_transition_start(
    _is_pc: bool,
    execute_order_initialising: bool,
    is_transition_anim: bool,
    order_action: OrderType,
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

/// Mirror the forecast update at the end of Original's nonzero
/// `RHSprite::PerformMotion` displacement. Transition-distance orders use a
/// separate commit path from ordinary walking in Rust, but Original runs both
/// through the same forecast update before its arrival check.
fn refresh_motion_forecast(
    sprite: &mut crate::sprite::Sprite,
    speed: f32,
    split_motion_speeds: Option<(f32, f32)>,
) {
    if sprite.position_iface.is_blocked() {
        return;
    }

    // Fast movement executes PerformMotion twice. Each nonzero call updates
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

/// Whether `PerformSeek` exposes a wrapped `PerformMotion` termination to
/// `RHElementActorHuman::Execute`. A successful terminal entity seek without
/// a post-seek sequence deliberately converts it back to IN_PROGRESS so it
/// can wait/refresh in place.
fn perform_seek_exposes_motion_termination(
    starts_post_seek: bool,
    final_entity_seek_arrival: Option<bool>,
) -> bool {
    starts_post_seek || final_entity_seek_arrival != Some(true)
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

/// Does the step this Execute is about to commit satisfy `IsGoalReached`?
///
/// `PerformMotion` moves first and only then asks the position interface
/// whether the goal is reached, so a call that would otherwise return `START`
/// can return `TERMINATED` instead. Rust stages the physical step until after
/// the sprite call, so the answer has to be projected on a throwaway copy of
/// the position interface, anti-collision and all. Comparing the straight-line
/// distance against the step length is not a substitute: the predicate is a
/// tolerance-compared dot product against the movement increment, and a step
/// deviated around another actor both leaves that line and rebuilds the
/// increment it is measured against.
#[allow(clippy::too_many_arguments)]
fn projected_step_reaches_goal(
    position_iface: &crate::position_interface::PositionInterface,
    mover_snapshot: Option<&super::anti_collision::ActorSnapshot>,
    neighbours: &[Option<super::anti_collision::ActorSnapshot>],
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
        if anti_on && let Some(mover) = mover_snapshot.filter(|snapshot| snapshot.active) {
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
                neighbours,
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

/// Publish an interleaved motion call's authoritative commit to the serial
/// anti-collision snapshot. Goal snapping can make this differ from the raw
/// requested displacement; offset repulsive geometry must follow the stored
/// position, not the discarded overshoot.
fn sync_snapshot_after_committed_step(
    snapshot: &mut super::anti_collision::ActorSnapshot,
    pre_position: MapPoint,
    post_position: MapPoint,
) {
    super::anti_collision::sync_snapshot_after_move(
        snapshot,
        post_position,
        post_position - pre_position,
    );
}

/// Motion state observed by the Original Execute arm after `PerformSeek`.
///
/// Entity-target `PerformSeek` consumes non-terminal sprite results and returns
/// `IN_PROGRESS`; point seeks return the raw result. This matters because the
/// caller's Execute switch must not observe either a raw `START` or `DONE`
/// while the seek wrapper remains active. Running upright is deliberately
/// excluded: its Original Execute arm sets `MOVING_FAST` unconditionally after
/// `PerformSeek`, irrespective of the returned motion state.
///
/// The ordinary arrival branch outranks the sprite's own result: once the
/// committed step satisfies the goal predicate, `PerformMotion` returns
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

#[derive(Clone, Copy, Debug, Default)]
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

fn cancel_aborted_order_pop(
    order_pops: &mut Vec<(crate::sequence::SequenceId, usize)>,
    seq_id: crate::sequence::SequenceId,
    elem_idx: usize,
) {
    order_pops.retain(|&(queued_seq, queued_idx)| queued_seq != seq_id || queued_idx != elem_idx);
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

/// Drop the pathfinder's source point before materializing movement orders.
///
/// `RHEngine::ProcessPathRequests` starts its order loop at index one whenever
/// `bUseFirstPoint` is false (`RHengine.cpp:8410-8423`).  Do not re-check that
/// the first point equals the request source here: legacy floating-point
/// equality is not the gate, and a source poisoned with NaNs must still be
/// skipped rather than becoming a live movement order.
fn discard_unrequested_path_source(waypoints: &mut Vec<MapPoint>, use_first_point: bool) {
    if !use_first_point && waypoints.len() > 1 {
        waypoints.remove(0);
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

/// Whether an outgoing movement is still in one of the generated locomotion
/// transitions that owns the previously published waypoint.
///
/// A replacement instructed while the actor is in a concrete walk/run order
/// does not inherit that waypoint: Original clears it at the replacement
/// arbitration boundary and leaves it zero until the new movement executes.
/// The transition case is different because the transition itself continues
/// to own the live movement forecast across the hand-off.
fn movement_transition_retains_goal(order: OrderType) -> bool {
    matches!(
        order,
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
    )
}

#[cfg(test)]
mod movement_goal_replacement_tests {
    use super::*;

    #[test]
    fn only_live_locomotion_transitions_retain_the_outgoing_goal() {
        assert!(movement_transition_retains_goal(
            OrderType::TransitionWaitingUprightRunningUpright
        ));
        assert!(movement_transition_retains_goal(
            OrderType::TransitionRunningUprightWaitingUpright
        ));
        assert!(!movement_transition_retains_goal(OrderType::RunningUpright));
        assert!(!movement_transition_retains_goal(OrderType::WalkingUpright));
    }
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

#[inline]
pub(crate) fn building_exit_wait_frames(sim: &crate::sim_rng::SimulationContext) -> u32 {
    crate::sim_rng::u32(sim, crate::sim_rng::RngSite::RuntimeBuildingExitWait, 0..16)
        + crate::sim_rng::u32(sim, crate::sim_rng::RngSite::RuntimeBuildingExitWait, 0..16)
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
    /// Exact `RHpathRequest` payload retained by the Original timeout list.
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

#[cfg(test)]
impl PendingPathRequest {
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
    Debug, Clone, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash,
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
    /// Original `RHPathFinder::mbIgnoreNextPath`. Cancelling the logical list
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

    pub(super) fn retain_not_owned_by(&mut self, owner: EntityId) {
        let removed_ignored_head = self.ignore_next_path
            && self
                .in_flight
                .as_ref()
                .map(|processed| processed.request.owner)
                .or_else(|| self.waiting.first().map(|request| request.owner))
                == Some(owner);
        self.waiting.retain(|request| request.owner != owner);
        if self
            .in_flight
            .as_ref()
            .is_some_and(|processed| processed.request.owner == owner)
        {
            self.in_flight = None;
        }
        if removed_ignored_head {
            self.ignore_next_path = false;
        }
    }

    /// Mirror `RHPathFinder::CancelPathRequest`: cancelling the list head
    /// marks its eventual result stale instead of removing it, while later
    /// requests for the same actor are deleted immediately. The retained head
    /// still occupies one `ProcessPathRequests` result slot.
    pub(super) fn cancel_for_owner(&mut self, owner: EntityId) {
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

    /// Mirror `RHPathFinder::MakeFast` on the first request for this actor.
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
            action => panic!(
                "RHPathFinder::MakeFast received unsupported pending action {action:?} for {owner:?}"
            ),
        };
    }

    /// Mirror `RHPathFinder::MakeSlow` on the first request for this actor.
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
            action => panic!(
                "RHPathFinder::MakeSlow received unsupported pending action {action:?} for {owner:?}"
            ),
        };
    }

    /// Mirror `RHPathFinder::MakeUpright` on the first request for this actor.
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
            action => panic!(
                "RHPathFinder::MakeUpright received unsupported pending action {action:?} for {owner:?}"
            ),
        };
    }

    /// Mirror `RHPathFinder::MakeCrouched` on the first request for this actor.
    pub(super) fn make_crouched(&mut self, owner: EntityId, pathfinder_index: u16) {
        let Some(request) = self.first_for_owner_mut(owner) else {
            return;
        };
        request.move_action = match request.move_action {
            OrderType::WalkingUpright | OrderType::RunningUpright => {
                request.half_diagonal_idx = pathfinder_index;
                OrderType::WalkingCrouched
            }
            action => panic!(
                "RHPathFinder::MakeCrouched received unsupported pending action {action:?} for {owner:?}"
            ),
        };
    }

    pub(super) fn clear(&mut self) {
        self.waiting.clear();
        self.in_flight = None;
        self.ignore_next_path = false;
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
        let (processed, valid) = self.orders.pending_path_requests.take_completed()?;
        let request = processed.request;
        if crate::pathfinder::parity_path_capture_is_active() {
            crate::pathfinder::record_parity_path_event(
                crate::pathfinder::ParityPathEvent::Completed {
                    request: parity_path_request_state(&self.world.fast_grid, &request),
                    valid,
                    // Original records the raw path even when cancellation
                    // makes the delivery invalid. A failed A* request has an
                    // empty raw path but remains a valid delivery.
                    waypoints: processed.waypoints.clone().unwrap_or_default(),
                },
            );
        }
        if !valid {
            return None;
        }
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
        // `RHPathFinder::ProcessPathRequests` never inspects the requesting
        // sequence element (`original-code/RHpathfinder.cpp:806-820,891-901`).
        // Every entry that is still in `mListPathRequests` is started and,
        // one call later, delivered — including entries whose element has
        // since been interrupted. Only `CancelPathRequest` removes an entry,
        // and it removes at most the first *later* request for the actor
        // while the logical head merely gets `mbIgnoreNextPath`
        // (`original-code/RHpathfinder.cpp:538-598`). Skipping a request here
        // because its element died would hand the freed result slot to the
        // next queued request a frame early.
        let retained_cancelled_head = self.orders.pending_path_requests.ignore_next_path;
        let Some(request) = self.orders.pending_path_requests.pop_to_start() else {
            return;
        };
        // Original FindPathNodes observes mbIgnoreNextPath and exits before
        // expanding its first node. The retained head therefore delivers an
        // invalid completion with an empty raw path; it must not calculate a
        // route merely because Rust runs pathfinding synchronously.
        let waypoints = retained_cancelled_path_result(retained_cancelled_head).or_else(|| {
            self.world.pathfinder.find_path(
                assets.pathfinder_graph.as_ref(),
                &self.world.fast_grid,
                request.layer,
                request.sector,
                request.half_diagonal_idx,
                request.source,
                request.dest,
                request.use_first_point,
            )
        });
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

#[inline]
fn retained_cancelled_path_result(retained_cancelled_head: bool) -> Option<Vec<MapPoint>> {
    retained_cancelled_head.then(Vec::new)
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
/// mirroring it into `PositionInterface`. Prefer that live pass until its
/// first `PassingDoor` callback. Original `RHElementActor::PassDoor` clears
/// `GetDoor()` at that callback even though the translated movement element
/// can keep executing its far-side walk, so later commands must use the live
/// position/sector instead of adapting through the completed gate.
///
/// The direction reported here is the pass's live traversal direction, not the
/// movement element's retained `mswDirection`. `RHSequence::AppendMoveToSequence`
/// reads `pTarget->GetDoorDirection()` (`original-code/RHsequence.cpp:369`),
/// which is `RHPositionInterface::mbDoorDirection`
/// (`original-code/RHpositioninterface.h:297-298`). That field is written by
/// `SetDoor( pDoor, <live direction> )` inside `RHElementActor::Translate`
/// (`original-code/RHelementactor.cpp:4035`, `:4053`, `:4080`, `:4110`,
/// `:4165`, `:4199`, `:4242`, `:4295`), where the direction comes from the
/// `GetSector() == pSectorIn` test performed at launch — the same test
/// `PassDoor::dispatch` reproduces into `ActiveDoorPass::direct`.
/// `ActiveDoorPass::position_direct` mirrors the *element's* serialized
/// `mswDirection` (`original-code/RHSequenceElementMovement.cpp:394-407`),
/// which only `RHArtificialIntelligence::Position` consumes.
pub(crate) fn current_door_for_route_source(
    entity: &crate::element::Entity,
) -> (crate::position_interface::DoorHandle, bool) {
    entity
        .actor_data()
        .and_then(|actor| actor.active_door_pass.as_ref())
        .filter(|pass| pass.triggers_fired == 0)
        .map(|pass| {
            (
                crate::position_interface::DoorHandle(pass.door_index.0),
                pass.direct,
            )
        })
        .unwrap_or_else(|| {
            let position = entity.position_iface();
            (position.get_door(), position.get_door_direction())
        })
}

/// Compare the object identities returned for two authored positions.
///
/// Original's GoTo branch compares `RHSector*` values directly
/// (`RHartificialintelligence.cpp:2566`), so equal script-facing sector
/// numbers do not imply that the positions occupy the same motion sector.
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

#[cfg(test)]
mod route_source_tests {
    use super::{current_door_for_route_source, sector_hits_have_distinct_identity};
    use crate::element::{
        ActiveDoorPass, ActorData, ActorPc, ElementData, Entity, HumanData, PcData,
    };
    use crate::gate::DoorIndex;
    use crate::order::OrderType;
    use crate::position_interface::DoorHandle;

    fn pc_with_door_pass(triggers_fired: u8) -> Entity {
        pc_with_door_pass_directions(triggers_fired, true, true)
    }

    fn pc_with_door_pass_directions(
        triggers_fired: u8,
        direct: bool,
        position_direct: bool,
    ) -> Entity {
        Entity::Pc(ActorPc {
            element: ElementData::default(),
            actor: ActorData {
                active_door_pass: Some(ActiveDoorPass {
                    door_index: DoorIndex(53),
                    direct,
                    position_direct,
                    steps: Default::default(),
                    triggers_fired,
                    current_action: OrderType::WalkingUpright,
                    current_reverse: false,
                    saved_action_state: None,
                }),
                ..ActorData::default()
            },
            human: HumanData::default(),
            pc: PcData::default(),
        })
    }

    #[test]
    fn route_source_uses_active_door_before_pass_callback() {
        let pc = pc_with_door_pass(0);

        assert_eq!(current_door_for_route_source(&pc), (DoorHandle(53), true));
    }

    #[test]
    fn route_source_drops_active_door_after_pass_callback() {
        let pc = pc_with_door_pass(1);

        assert_eq!(
            current_door_for_route_source(&pc),
            (DoorHandle::NULL, false)
        );
    }

    #[test]
    fn route_source_reports_live_traversal_direction_not_element_direction() {
        // `RHSequence::AppendMoveToSequence` reads `GetDoorDirection()`
        // (`original-code/RHsequence.cpp:369`), the position interface field
        // `SetDoor` writes from the live `GetSector() == pSectorIn` test at
        // launch. A v48-restored movement element can carry a different
        // serialized `mswDirection`; that value belongs to
        // `RHArtificialIntelligence::Position`, not to route sourcing.
        let pc = pc_with_door_pass_directions(0, true, false);

        assert_eq!(current_door_for_route_source(&pc), (DoorHandle(53), true));
    }

    #[test]
    fn route_source_uses_position_door_during_pass_callback_queue_window() {
        let mut pc = pc_with_door_pass(1);
        pc.position_iface_mut().set_door(DoorHandle(17), false);

        assert_eq!(current_door_for_route_source(&pc), (DoorHandle(17), false));
    }

    #[test]
    fn goto_compares_sector_object_identity_even_when_numbers_match() {
        use crate::fast_find_grid::{SectorHit, SectorIndex};
        use crate::sector::SectorNumber;

        let number = SectorNumber::new(18);
        let hit = |index| SectorHit::Found {
            sector_idx: SectorIndex::new(index).unwrap(),
            sector_number: number,
        };

        let expected = crate::position_interface::SectorHandle::new(18).unwrap();
        assert!(sector_hits_have_distinct_identity(
            hit(12),
            hit(37),
            expected
        ));
        assert!(!sector_hits_have_distinct_identity(
            hit(12),
            hit(12),
            expected
        ));
        assert!(!sector_hits_have_distinct_identity(
            hit(12),
            SectorHit::None,
            expected
        ));
        assert!(!sector_hits_have_distinct_identity(
            hit(12),
            SectorHit::Found {
                sector_idx: SectorIndex::new(37).unwrap(),
                sector_number: SectorNumber::new(19),
            },
            expected
        ));
    }
}

/// Radius for circular dispatch (one third of [`GROUP_LIMIT_MAX`]).
pub(in crate::engine) const CIRCULAR_DISPATCH_RADIUS: f32 = 60.0;

/// Maximum centroid-to-member distance for mercenary formation to apply.
/// When any member is farther than this from the centroid, fall back to
/// circular dispatch.
pub(in crate::engine) const GROUP_LIMIT_MAX: f32 = 180.0;

/// Rebuild Original's per-actor formation box at a candidate destination.
///
/// Ordinary sectors translate the live `GetMoveBoxMap()` by the displacement
/// from the actor to the candidate. Lift sectors instead ask for
/// `GetMoveBox(RHPOSTURE_UPRIGHT)` and translate that zero-centred box to the
/// candidate. Original's current `GetMoveBox(posture)` implementation returns
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

/// Build a compact-group formation box in the same sequence of floating-point
/// operations as `RHEngine::PerformGroupMove`:
///
/// * ordinary: `GetMoveBoxMap() - vectorCenter + pointDestination`
/// * lift: `GetMoveBox(Upright) + actorPosition - vectorCenter + pointDestination`
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

/// `PerformGroupMove` receives one resolved upright action from the click
/// dispatcher. It does not infer sword movement from an actor's opponent list;
/// `DetermineMovementAnimation` performs any live action-state adaptation when
/// the movement is instructed.
#[inline]
fn player_group_move_action(run: bool) -> OrderType {
    if run {
        OrderType::RunningUpright
    } else {
        OrderType::WalkingUpright
    }
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
    /// Whether arrival calls `StartPostSeekSequence` rather than merely
    /// continuing to a later element in the same Rust sequence.
    launches_post_seek: bool,
}

/// Owner-scoped pre-pass snapshots for one `tick_entity_movement_owner`
/// call, captured before the mutable per-actor movement pass so
/// `tick_one_movement_actor` can borrow the entity table mutably.
struct MovementPrepass {
    combat_face_target: Option<MapPoint>,
    combat_face_target_is_ground: bool,
    speed_factor: f32,
    goal_target_info: Option<crate::position_interface::TargetInfo>,
    final_tolerance: FinalTol,
    point_seek_post_sector: Option<crate::position_interface::SectorHandle>,
    lift_translation: Option<LiftAnimContext>,
    door_pass_climb_direction: Option<i16>,
    decorative_building_trap_at_destination: bool,
}

/// Deferred side effects collected by `tick_one_movement_actor` while the
/// per-actor entity borrow is live, then drained in parity-critical order
/// by `tick_entity_movement_owner` after the movement pass.
#[derive(Default)]
struct MovementDeferred {
    post_completion_motion_override: Option<crate::sprite::MotionState>,
    sword_movement_starts: Vec<EntityId>,
    sword_movement_terminations: Vec<EntityId>,
    // Collect movement results that need sequence manager notification.
    // We can't call sequence_manager while iterating entities mutably.
    // Door-pass triggers to execute after the movement loop (need &mut self).
    door_triggers: Vec<(EntityId, crate::gate::DoorIndex, bool, u8)>,
    // Door-pass Transition orders to push onto the actor's current
    // sequence element after the loop closes (needs sequence_manager).
    transition_pushes: Vec<(crate::sequence::SequenceId, usize, crate::order::Order)>,
    // Pending `DoorPassStep::Select` hulk requests — processed after the
    // loop since they mutate both the carrier and its carried target.
    select_triggers: Vec<(EntityId, f32)>,
    completed_door_passes: Vec<(EntityId, crate::gate::DoorIndex, bool)>,
    // Rider entities whose running animation hit the charge
    // decision frames while carrying RIDER_CHARGE.
    galopp_event: bool,
    // Movement elements whose sprite motion returned the blocked-
    // abort signal and must be marked Impossible after the entity
    // borrow ends.
    blocked_impossible: Vec<(crate::sequence::SequenceId, usize)>,
    door_pass_transition_start_effects: Vec<EntityId>,
    door_pass_transition_done_effects: Vec<EntityId>,
    door_pass_transition_completion_effects: Vec<(EntityId, OrderType)>,
    /// Final door-pass goals retired by the terminal Execute arm. Original
    /// clears these through DoNextOrder/condolation only after
    /// Actor::CheckForLineCrossing has had a chance to rebuild the cached
    /// increment against the still-live destination.
    terminal_door_pass_goal_clears: Vec<EntityId>,
    post_seek_arrivals: Vec<(EntityId, crate::sequence::SequenceId, usize)>,
    /// Terminal state effect from a movement transition whose `PerformSeek`
    /// pre-motion tolerance arm launched a post-seek sequence. Original
    /// launches that sequence synchronously from inside `PerformSeek`; only
    /// after its callbacks return does the surrounding transition Execute arm
    /// apply its TERMINATED posture/action-state switch.
    post_seek_terminal_state_effects: Vec<(
        EntityId,
        crate::element::Posture,
        crate::element::ActionState,
    )>,
    /// Same ordering contract as `post_seek_terminal_state_effects`, for the
    /// Rust representation where the post-seek continuation is a following
    /// element of the same sequence and is exposed by the terminal order pop.
    sequence_seek_terminal_state_effects: Vec<(
        EntityId,
        crate::element::Posture,
        crate::element::ActionState,
    )>,
    // Elevation-line crossings detected during this tick. Dispatched
    // after the entity loop so `check_for_line_crossing` can borrow
    // `self` for the fast-grid query and obstacle swap.
    // Each entry is `(entity_id, old_pos, layer)` in projected map
    // coordinates; the segment endpoint is resolved from the actor's
    // live position when the checks are dispatched, because
    // CheckForLineCrossing runs after the whole Execute arm and some
    // arms reposition the actor in their completion branch. Geometry
    // queries convert at the call boundary.
    line_cross_checks: Vec<(EntityId, MapPoint, u16)>,
    // Original collects all non-elevation LINE_CROSS kinds together,
    // sorts once by travel distance, then checks patch/script/sound flags
    // on each line in that order.
    non_elevation_cross_checks: Vec<(EntityId, MapPoint, u16)>,
    // Final entity-seek orders whose live target no longer matches the
    // sampled endpoint. Original refreshes these only when the current
    // order itself terminates; merely exposing a stop-transition as the
    // next order is not a refresh boundary.
    transition_seek_refreshes: Vec<(EntityId, crate::sequence::SequenceId, usize)>,
    // Waypoint arrivals (both intermediate and final) — each
    // triggers one `do_next_order` call on the actor's Move
    // element to pop the walking order that represented that
    // waypoint.  Each waypoint is its own order on the actor's
    // movement order list, and the engine pops them as the actor
    // crosses them.  Collected here and processed after the entity
    // loop so the `do_next_order` call can borrow `self` mutably.
    order_pops: Vec<(crate::sequence::SequenceId, usize)>,
    // A resolved mouse orientation can run immediately before the last
    // tick of a generated stop transition.  The transition still turns
    // toward that externally supplied direction, but its outgoing Move
    // order owns the cached movement increment.  Keep enough context to
    // restore the external goal after the Move's terminal condolence has
    // retired that cache.
    terminal_pc_direction_goal_restores: Vec<(EntityId, i16, i16)>,

    // Water-splash titbit emissions queued from the walk branch.
    // Drained after the entity loop so `titbit_manager.add_titbit`
    // can borrow `&mut self` without colliding with the active
    // entity borrow.
    water_splash_emits: Vec<(EntityId, crate::coordinates::WorldPoint3D, u16)>,
    movement_state_effects: Vec<(
        EntityId,
        crate::element::Posture,
        crate::element::ActionState,
    )>,
    // PC movement actions actually dispatched this frame.  The original
    // RHElementActorPC performs action-specific side effects from inside
    // the matching Execute arm, so posture alone is not a substitute for
    // this per-frame execution record.
    executed_pc_movement_actions: Vec<(EntityId, OrderType)>,
    executed_sword_movement: bool,
}

/// Preserve Actor::Hourglass' post-Execute line-crossing segment when a
/// movement step reaches a seek arrival whose synchronous handoff returns
/// before the ordinary movement tail. The segment endpoint is deliberately
/// resolved later from the actor's live position, exactly like the common
/// tail; only the outer pre-Execute position is retained here.
fn queue_committed_arrival_crossing(
    deferred: &mut MovementDeferred,
    entity_id: EntityId,
    old_pos: MapPoint,
    layer: u16,
    arrived_after_committed_step: bool,
    eligible_for_crossing: bool,
) -> bool {
    if !arrived_after_committed_step || !eligible_for_crossing {
        return false;
    }
    deferred.line_cross_checks.push((entity_id, old_pos, layer));
    deferred
        .non_elevation_cross_checks
        .push((entity_id, old_pos, layer));
    true
}

fn clear_terminal_door_pass_goal(entity: &mut Entity) {
    entity
        .position_iface_mut()
        .set_map_goal(crate::coordinates::MapPoint::ZERO);
}

/// Argument plumbing shared by the two movement-Execute anti-collision
/// dispatches (the transition fast-climb arm and the ordinary walk arm).
/// A free function rather than a method: at both call sites the mover is
/// held as a live `&mut` borrow out of the entity table, so `self` cannot
/// be borrowed as a whole.
#[allow(clippy::too_many_arguments)]
fn apply_prepared_anti_collision_step(
    frame: u32,
    mover_snap: &super::anti_collision::ActorSnapshot,
    anti_snapshots: &EntitySlots<Option<super::anti_collision::ActorSnapshot>>,
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
        let trace = super::anti_collision::goal_owner_anti_debug_frame(mover_snap.id).is_some();
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
            mover_snap,
            anti_snapshots.as_slice(),
            static_repulsive_points,
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
                mover_snap.id,
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
        if std::env::var_os("PARITY_DEBUG_PATH_OWNER_LIFECYCLE").is_none() {
            return;
        }
        let parse_filter = |name: &str| {
            std::env::var(name).ok().map(|value| {
                value.parse::<u32>().unwrap_or_else(|error| {
                    panic!("invalid {name}={value:?} for path-owner lifecycle diagnostic: {error}")
                })
            })
        };
        let frame = self.control.frame_counter;
        if parse_filter("PARITY_DEBUG_PATH_OWNER_FRAME").is_some_and(|expected| expected != frame)
            || self.get_entity(owner).is_none()
        {
            return;
        }
        let creation_order = self.world.original_creation_order(owner);
        if parse_filter("PARITY_DEBUG_PATH_OWNER_CREATION_ORDER")
            .is_some_and(|expected| expected != creation_order)
        {
            return;
        }

        let manager = &self.orders.sequence_manager;
        let selected = manager.current_element_for_actor(owner);
        let current_order =
            manager
                .current_order_for_actor(owner)
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

    pub(crate) fn parity_failed_path_requests(
        &self,
    ) -> Vec<crate::pathfinder::ParityFailedPathRequest> {
        self.orders
            .failed_path_requests
            .iter()
            .map(|failed| {
                let request = &failed.request;
                assert_eq!(
                    failed.owner, request.owner,
                    "failed-path timeout owner disagrees with retained request"
                );
                crate::pathfinder::ParityFailedPathRequest {
                    request: parity_path_request_state(&self.world.fast_grid, request),
                    sector: request.legacy_sector,
                    time: failed.first_fail_frame,
                }
            })
            .collect()
    }

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
        let _ = is_pc;
        self.check_for_non_elevation_line_crossing(sim, assets, entity_id, old_pos, new_pos, layer);
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
        anti_context: Option<(
            &LiveMobileGeometry,
            &mut EntitySlots<Option<super::anti_collision::ActorSnapshot>>,
        )>,
    ) -> Option<RiderChargeExecution> {
        use crate::element::{ActionState, ActiveRiderCharge, Posture};
        use crate::weapons::SwordStrike;

        let provenance_frame = self.control.frame_counter;
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
            let rider = self.world.entities[rider_id]
                .as_mut()
                .expect("rider remained present during charge initialization");
            // ExecuteRiderCharge reuses RHElementActorHuman's serialized
            // mlistSwordStrikeVictims storage. A charge clears and refills
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
                .victims = pending_victims.clone();
            let actor = rider
                .actor_data_mut()
                .expect("RiderCharging soldier must have actor data");
            actor.sweep_state = None;
            actor.active_rider_charge = Some(ActiveRiderCharge { pending_victims });
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
            let entity = self.world.entities[rider_id]
                .as_mut()
                .expect("rider remained present before charge motion");
            let elem = entity.element_data_mut();
            // ExecuteRiderCharge calls Turn before PerformMotion. The first
            // Execute therefore turns toward the previously installed goal;
            // PerformMotion initializes this order and computes its new goal
            // only afterward.
            elem.sprite.position_iface.turn();
            let (mut state, distance) = if frozen_all {
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
            // PerformMotion initializes the new direction goal after the
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
                    if let Some((prepared, anti_snapshots)) = anti_context.as_ref()
                        && anti_on
                        && let Some(mover_snapshot) = anti_snapshots
                            .get(rider_id)
                            .and_then(|slot| slot.as_ref())
                            .filter(|snapshot| snapshot.active)
                            .cloned()
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
                            &mover_snapshot,
                            anti_snapshots,
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
                    // PerformMotion returns ABORTED before committing the
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

                if let Some((_, anti_snapshots)) = anti_context
                    && let Some(snapshot) = anti_snapshots
                        .get_mut(rider_id)
                        .and_then(|slot| slot.as_mut())
                {
                    sync_snapshot_after_committed_step(snapshot, pre_position, elem.position_map());
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
            let rider = self.world.entities[rider_id]
                .as_mut()
                .expect("rider remained present while resolving charge hit");
            rider
                .human_data_mut()
                .expect("RiderCharging soldier must have human data")
                .sword_sweep
                .victims
                .retain(|pending| *pending != victim_id);
            rider
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
                // Rider Execute mutates mpOrder->action and calls NewID on
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
            self.world.entities[rider_id]
                .as_mut()
                .expect("rider remained present at charge completion")
                .actor_data_mut()
                .expect("RiderCharging soldier must have actor data")
                .active_rider_charge = None;
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
        Some(RiderChargeExecution {
            completion_order_id,
        })
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

    pub(super) fn launch_sword_movement_termination_provoke(&mut self, entity_id: EntityId) {
        self.launch_element(crate::sequence::SequenceElement::new(
            1,
            crate::element::Command::Provoke,
            Some(entity_id),
        ));
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

    /// Source-exact `PerformGroupMove` formation-slot authorization for one
    /// selected actor. Unlike the generic click snap, this must use the
    /// actor's live move box rather than pathfinder half-diagonal table entry
    /// zero; different PCs and adopted saves can carry different boxes.
    fn authorize_group_move_destination(
        &self,
        actor: EntityId,
        candidate: MapPoint,
        reference: MapPoint,
        layer: u16,
        is_lift: bool,
    ) -> Option<MapPoint> {
        let entity = self.get_entity(actor)?;
        let position = entity.position_iface();
        let mut bbox = group_move_candidate_box(
            *position.get_move_box_map(),
            *position.get_move_box(),
            entity.element_data().position_map(),
            candidate,
            is_lift,
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
        self.perform_group_move_with_destinations(
            sim,
            assets,
            pc_ids,
            click_point,
            run,
            show_marker,
            goal_override,
            None,
        );
    }

    /// Run the normal group-movement resolution while retaining explicit
    /// role-aware destinations. Allied formations use this rather than
    /// invoking [`Self::perform_group_move`] once per soldier, so the group
    /// shares the same click-sector resolution and slot-authorization pass as
    /// an ordinary multi-hero click.
    pub(super) fn perform_group_move_to_slots(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        actor_ids: &[EntityId],
        click_point: MapPoint,
        destinations: &[MapPoint],
        run: bool,
        show_marker: bool,
    ) {
        assert_eq!(
            actor_ids.len(),
            destinations.len(),
            "explicit group-move destination count must match actor count"
        );
        self.perform_group_move_with_destinations(
            sim,
            assets,
            actor_ids,
            click_point,
            run,
            show_marker,
            None,
            Some(destinations),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn perform_group_move_with_destinations(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        pc_ids: &[EntityId],
        click_point: MapPoint,
        run: bool,
        show_marker: bool,
        goal_override: Option<(crate::sector::SectorNumber, u16)>,
        explicit_destinations: Option<&[MapPoint]>,
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
        // Top-down layer search reconstructs `mpSelectedSector`, whose sector
        // kind drives the door/lift/jump semantics below.  It is deliberately
        // independent of `goal_override`: Original `PerformGroupMove` can use
        // a patch's sector as `pSectorGoal` while `mpSelectedSector` remains
        // the coincident mouse-selection overlay (RHengine.cpp:5322-5337).
        // RecordGroupMove stores that patch-aware route goal, not necessarily
        // `mpSelectedSector`, so replay must preserve both identities.
        let hit = self
            .world
            .fast_grid
            .get_sector_screen(click_point, reference);
        let selected_grid_sector = hit
            .sector_idx
            .and_then(|i| self.world.fast_grid.level.sectors.get(usize::from(i)));
        let (is_lift_click, is_door_click_sector, is_jump_click) = selected_grid_sector
            .map(|sector| group_move_sector_kinds(sector.sector_type))
            .unwrap_or((false, false, false));
        let jump_underlying_sector = selected_grid_sector
            .filter(|sector| sector.sector_type.is_jump())
            .and_then(|sector| sector.underlying_sector)
            .and_then(|index| self.world.fast_grid.level.sectors.get(usize::from(index)))
            .map(|sector| (sector.sector_number, sector.layer));
        let clicked_sector_door_index = selected_grid_sector.and_then(|sector| sector.door_index);
        let clicked_polygon_door_index = self.scripts.mission.as_ref().and_then(|_| {
            door_click_polygon_at(&self.script_domains.interactables.doors, click_point)
        });
        let clicked_door_index = clicked_sector_door_index.or(clicked_polygon_door_index);
        let is_door_click = is_door_click_sector || clicked_door_index.is_some();
        let (route_goal_sector, route_goal_layer) =
            group_move_route_goal(goal_override, hit.sector, hit.layer);

        let (
            goal_sector,
            effective_click,
            effective_layer,
            is_valid,
            is_lift_click,
            is_door_click,
            is_jump_click,
            clicked_jump_sector_idx,
            jump_underlying_sector,
            clicked_door_index,
        ) = if goal_override.is_some() {
            (
                route_goal_sector,
                click_point,
                route_goal_layer,
                true,
                is_lift_click,
                is_door_click,
                is_jump_click,
                if is_jump_click { hit.sector_idx } else { None },
                jump_underlying_sector,
                clicked_door_index,
            )
        } else {
            let is_valid = hit.is_valid_for_move(&self.world.fast_grid);

            // ── Door/Drawbridge click shortcut ──
            //
            // When the click hits a door sector, bypass the walkability
            // snap on formation slots.  Per-PC routing must also skip
            // `snap_click_to_walkable` so the destination stays in the
            // door sector and the gate-A* path routes through the
            // door's entry point (the door sector itself is not a
            // motion area).
            // Door index of the clicked door sector, if any.  Used to
            // route the per-PC gate search via `find_path_to_door` and
            // emit a `GoalShape::Door` terminal element.
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
                is_lift_click,
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
        let pc_positions: Vec<MapPoint> = positions
            .iter()
            .map(|(pc_id, _, _, _)| {
                self.get_entity(*pc_id)
                    .unwrap_or_else(|| panic!("selected group-move actor {pc_id:?} is missing"))
                    .element_data()
                    .position_map()
            })
            .collect();
        let (mercenary_center, dests) = if let Some(destinations) = explicit_destinations {
            (None, destinations.to_vec())
        } else {
            let n = pc_positions.len() as f32;
            let mut cx = pc_positions.iter().map(|p| p.x).sum::<f32>();
            let mut cy = pc_positions.iter().map(|p| p.y).sum::<f32>();
            // Original multiplies the accumulated vector by the reciprocal;
            // preserve that operation rather than compiling this as two
            // divisions with potentially different rounding.
            let reciprocal = 1.0f32 / n;
            cx *= reciprocal;
            cy *= reciprocal;
            let max_sq_dist = pc_positions
                .iter()
                .map(|p| {
                    let dx = p.x - cx;
                    let dy = p.y - cy;
                    dx * dx + dy * dy
                })
                .fold(0.0f32, f32::max);
            if max_sq_dist <= GROUP_LIMIT_MAX * GROUP_LIMIT_MAX {
                (
                    Some(MapPoint::new(cx, cy)),
                    mercenary_formation_destinations(&pc_positions, effective_click),
                )
            } else {
                (
                    None,
                    circular_dispatch_destinations(&pc_positions, effective_click),
                )
            }
        };

        // ── Per-PC routing ──
        // For each PC, decide between:
        //   1. Same-sector: simple MOVE
        //   2. Cross-sector (door/lift): gate-A* sequence
        for ((pc_id, _, pc_src_layer, src_sector), formation_dest) in
            positions.iter().zip(dests.iter())
        {
            let owner_is_pc = self
                .get_entity(*pc_id)
                .unwrap_or_else(|| panic!("selected group-move actor {pc_id:?} is missing"))
                .is_pc();
            // Compact-group placement is authorized exactly once, before
            // PerformMove, using the box produced by Original's ordered
            // `box - center + click` translations. Reconstructing a point
            // first and then translating the box is algebraically equivalent
            // but changes f32 rounding at path-goal boundaries.
            let mercenary_dest;
            let dest = if let Some(center) = mercenary_center {
                let Some(entity) = self.get_entity(*pc_id) else {
                    panic!("selected group-move actor {pc_id:?} is missing");
                };
                let position = entity.position_iface();
                let mut bbox = group_move_mercenary_box(
                    *position.get_move_box_map(),
                    *position.get_move_box(),
                    entity.element_data().position_map(),
                    center,
                    effective_click,
                    is_lift_click,
                );
                if !is_door_click
                    && !self.world.fast_grid.find_authorized_position_toward(
                        &mut bbox,
                        effective_click,
                        effective_layer,
                    )
                {
                    self.hero_speaking(
                        assets,
                        *pc_id,
                        crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
                    );
                    continue;
                }
                mercenary_dest = bbox.center();
                &mercenary_dest
            } else {
                formation_dest
            };
            let mut pc_goal_sector = goal_sector;
            let mut pc_effective_layer = effective_layer;
            if is_jump_click {
                // PerformGroupMove authorizes each formation slot before
                // PerformMove tests whether the selected jump is usable.
                // Keep the raw click through the jump-sector hit test, then
                // apply that same move-box authorization here; the coarse
                // nearest-walkable fallback is not equivalent near a jump
                // landing boundary.
                let resolved_jump_dest = if mercenary_center.is_some() {
                    Some(*dest)
                } else {
                    self.authorize_group_move_destination(
                        *pc_id,
                        *dest,
                        effective_click,
                        pc_effective_layer,
                        is_lift_click,
                    )
                };
                let Some(resolved_jump_dest) = resolved_jump_dest else {
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
                        true,
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

                    let mut seq = build_line_jump_click_sequence(
                        *pc_id,
                        player_group_move_action(run),
                        source_line_idx,
                        &source_line,
                        destination_line_idx,
                        resolved_jump_dest,
                        pc_effective_layer,
                        1.0,
                    );
                    if owner_is_pc {
                        let speak = crate::sequence::SequenceElement::new(
                            4,
                            crate::element::Command::SpeakHeroReachDestination,
                            Some(*pc_id),
                        );
                        seq.append_element(speak);
                    }
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
                let snap_res = if is_door_click || mercenary_center.is_some() {
                    Some(*dest)
                } else {
                    self.authorize_group_move_destination(
                        *pc_id,
                        *dest,
                        effective_click,
                        pc_effective_layer,
                        is_lift_click,
                    )
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
                let mut move_elem = crate::sequence::SequenceElement::new_movement(
                    1,
                    crate::element::Command::Move,
                    Some(*pc_id),
                    player_group_move_action(run),
                );
                if let crate::sequence::SequenceElementData::Movement {
                    destination, layer, ..
                } = &mut move_elem.data
                {
                    *destination = snapped;
                    *layer = pc_effective_layer;
                }

                // Append a `SpeakHeroReachDestination` element after
                // the move and cap the sequence with any
                // posture-cleanup sub-elements the PC needs (re-equip
                // bow, re-crouch, re-enter HelpingClimb / Beggar,
                // demote trailing ShootBow to ShootBowOnce).  The PC's
                // `Instruct` override terminates the Speak element on
                // dispatch and queues the HERO_DONE_COMMAND bark
                // (handled by `arbitrate_instruct`).
                let mut seq = crate::sequence::Sequence::new();
                seq.append_element(move_elem);
                if owner_is_pc {
                    append_arrival_speech(&mut seq, *pc_id);
                }
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
            let resolved_dest = if is_door_click || mercenary_center.is_some() {
                *dest
            } else {
                let Some(resolved) = self.authorize_group_move_destination(
                    *pc_id,
                    *dest,
                    effective_click,
                    pc_effective_layer,
                    is_lift_click,
                ) else {
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
                    // This is the same failed route-construction outcome as
                    // FindPathIntoDoor / FindPathGates returning false in
                    // RHSequence::AppendMoveToSequence.  Original reports
                    // every such failure through the authoritative unable
                    // bark before abandoning the new sequence.
                    tracing::warn!(
                        actor = ?pc_id,
                        "skipping gate path without resolved goal sector"
                    );
                    self.hero_speaking(
                        assets,
                        *pc_id,
                        crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
                    );
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
                        &|sector| self.building_sector_is_authorized(sector),
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
                        player_group_move_action(run),
                        door_goal.is_none(),
                        1.0,
                        crate::sequence::MoveFlags::empty(),
                        Vec::new(),
                        Vec::new(),
                        owner_is_pc,
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

        // Does the entity have the lockpick contextual action?
        // Needed to choose the lockpick sub-element branch.
        let has_lockpick = self
            .expect_entity(entity_id, "gate-route lockpick check")
            .actor_auth_info()
            .has_lockpick;

        // Resolve sector → is_building via the fast grid. Returns false for
        // unknown sectors (ordinary motion areas are not buildings).
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
            tracing::trace!(
                entity = ?entity_id,
                gate_idx,
                door = ?shot.door_index,
                direct = shot.direct,
                prev_sector,
                new_sector = shot.new_sector,
                old_is_building,
                is_jump = shot.is_jump,
                "gate-traversal sequence emits a gate"
            );

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
                let r = building_exit_wait_frames(sim);
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
                // Original uses `mbDirect ? pointIn : pointOut`, which is
                // the path-local exit for either traversal direction, so the
                // sprite faces the lock while picking it.
                let camera_pt = shot.exit;
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
                // The Original gate carries path-local `RHGate::mbDirect`
                // while constructing PassDoor. AI::Position then reads the
                // selected movement element's direction to commit the actor
                // to the side it is entering. Materialize that traversal
                // direction instead of leaving Rust's element at its default.
                direction: i16::from(shot.direct),
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
                        let r = building_exit_wait_frames(sim);
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
        // arrived" line once the destination is reached. Original
        // `PerformMove` passes the incremented `uwCount` left by
        // `AppendMoveToSequence`, so speech is the next command level,
        // after the final movement has completed. The PC's `Instruct`
        // override terminates it on dispatch and queues
        // `HeroDoneCommand` via `arbitrate_instruct`.
        if append_arrival_speech && !seq.is_empty() {
            let speak_level = seq
                .last()
                .map(|element| element.command_level.saturating_add(1))
                .unwrap_or(level);
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
        let debug_decision_path = crate::ai_enemy::decision_path_debug_enabled()
            && crate::ai_enemy::decision_path_debug_matches_raw(
                self.control.frame_counter,
                entity_id.index(),
            );
        if debug_decision_path {
            eprintln!(
                "AIDECISION frame={} owner={} stage=preflight_enter order={:?} target=({:08x},{:08x}) move_flags={} tolerance_bits={:08x} no_halt={} reverse={} find_accessible={} ask_obstacle={} compute_direction={}",
                self.control.frame_counter,
                entity_id.index(),
                intent.order_type,
                intent.target_x.to_bits(),
                intent.target_y.to_bits(),
                intent.move_flags,
                intent.tolerance.to_bits(),
                intent.no_halt,
                intent.reverse,
                intent.find_accessible,
                intent.ask_obstacle,
                intent.compute_direction,
            );
        }
        // Upper-bound check.  `AiController::go_to` already rejects
        // `target_x <= 0 || target_y <= 0` before pushing the intent;
        // the engine drain owns the `>= GetLevelSize()` half because
        // `level_size` lives on the shared cutscene camera, not on
        // `AiContext`. Direct RHMOVE_MAP elements are the exception: the
        // merry-man exit path intentionally targets a reinforcement door's
        // PointOut beyond the playable map and Actor::Instruct admits MAP
        // movement without the ordinary reachable-position gate.
        let move_flags =
            crate::sequence::MoveFlags::from_bits_truncate(u32::from(intent.move_flags));
        if !move_flags.contains(crate::sequence::MoveFlags::MAP) {
            let level_w = self.feedback.cutscene_camera.level_size.x;
            let level_h = self.feedback.cutscene_camera.level_size.y;
            if level_w > 0.0 && intent.target_x >= level_w
                || level_h > 0.0 && intent.target_y >= level_h
            {
                self.set_ai_couldnt_reachpoint(entity_id);
                if debug_decision_path {
                    eprintln!(
                        "AIDECISION frame={} owner={} stage=preflight_result result=reject_upper_bound level=({:08x},{:08x}) target=({:08x},{:08x})",
                        self.control.frame_counter,
                        entity_id.index(),
                        level_w.to_bits(),
                        level_h.to_bits(),
                        intent.target_x.to_bits(),
                        intent.target_y.to_bits(),
                    );
                }
                return false;
            }
        }

        if !intent.find_accessible && !intent.ask_obstacle {
            if debug_decision_path {
                eprintln!(
                    "AIDECISION frame={} owner={} stage=preflight_result result=accepted_no_checks",
                    self.control.frame_counter,
                    entity_id.index(),
                );
            }
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
                if debug_decision_path {
                    eprintln!(
                        "AIDECISION frame={} owner={} stage=preflight_result result=reject_find_accessible target=({:08x},{:08x}) layer={} move_box={:?}",
                        self.control.frame_counter,
                        entity_id.index(),
                        intent.target_x.to_bits(),
                        intent.target_y.to_bits(),
                        layer,
                        move_box,
                    );
                }
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
                if debug_decision_path {
                    eprintln!(
                        "AIDECISION frame={} owner={} stage=preflight_result result=reject_straight from=({:08x},{:08x}) target=({:08x},{:08x}) layer={} move_box={:?}",
                        self.control.frame_counter,
                        entity_id.index(),
                        position.x.to_bits(),
                        position.y.to_bits(),
                        intent.target_x.to_bits(),
                        intent.target_y.to_bits(),
                        layer,
                        move_box,
                    );
                }
                return false;
            }
        }

        if debug_decision_path {
            eprintln!(
                "AIDECISION frame={} owner={} stage=preflight_result result=accepted target=({:08x},{:08x})",
                self.control.frame_counter,
                entity_id.index(),
                intent.target_x.to_bits(),
                intent.target_y.to_bits(),
            );
        }
        true
    }

    /// Set `AiController::couldnt_reachpoint = true` on the entity, used
    /// by the GoTo pre-flight gates to surface a same-frame failure to
    /// the AI's stuck-retry / fallback logic.
    #[track_caller]
    fn set_ai_couldnt_reachpoint(&mut self, entity_id: EntityId) {
        let debug_decision_path = crate::ai_enemy::decision_path_debug_enabled()
            && crate::ai_enemy::decision_path_debug_matches_raw(
                self.control.frame_counter,
                entity_id.index(),
            );
        if debug_decision_path {
            eprintln!(
                "AIDECISION frame={} owner={} stage=set_couldnt_reachpoint caller={}",
                self.control.frame_counter,
                entity_id.index(),
                std::panic::Location::caller(),
            );
        }
        let Some(entity) = self.world.entities.get_mut(entity_id) else {
            return;
        };
        if let Some(ai) = entity.ai_controller_mut() {
            ai.couldnt_reachpoint = true;
        }
    }

    /// Mark the engine-owned result of an AI order as available to the
    /// deferred EndThink surface.  Original builds/authorizes the order
    /// inline; Rust must not interpret an earlier nested drain with no result
    /// as a successful authorization.
    fn resolve_ai_engine_completion_verdict(&mut self, entity_id: EntityId) {
        let entity = self.world.entities.get_mut(entity_id).unwrap_or_else(|| {
            panic!("AI order owner {entity_id:?} disappeared before its engine verdict")
        });
        let ai = entity.ai_controller_mut().unwrap_or_else(|| {
            panic!("AI order owner {entity_id:?} lost its controller before its engine verdict")
        });
        ai.resolve_engine_completion_verdict();
    }

    pub(super) fn launch_ai_move(
        &mut self,
        entity_id: EntityId,
        intent: &crate::order::AiOrderIntent,
    ) {
        // One AI think can legitimately emit two distinct `GoTo` intents for
        // the same actor (`RHArtificialMalignity::ReconsiderSwordfightObservation`
        // runs its defensive step-back `GoTo` and then deliberately falls
        // through — `original-code/RHartificialmalignity.cpp:15502-15519` has
        // no `return` — into the attack block's `GoNear`). Each
        // `RHArtificialIntelligence::GoTo` builds its own `RHSequence` and
        // hands it to `RHSequenceManager::LaunchSequence`
        // (`original-code/RHartificialintelligence.cpp:2453,2594`), so both
        // land on `mlistSequenceElementsToGo` and both are instructed, in
        // launch order, by the sequence-manager hourglass
        // (`original-code/RHsequencemanager.cpp:938-952`). Nothing in `GoTo`
        // discards the earlier one: its pre-launch halt is the dead
        // `uwFlags & GOTO_NOHALT == 0` gate, which C++ precedence evaluates as
        // `uwFlags & 0` (`original-code/RHartificialintelligence.cpp:2423`).
        //
        // So keep every intent, in FIFO order. An explicit
        // `Halt`/`StopAll` still invalidates the queued ones, because
        // `halt_actor` drops this actor's pending intents at exactly that
        // boundary — that is Original's
        // `StopNotYetLaunchedSequenceElements`.
        let mut intent = intent.clone();
        if intent.defer_instruction && intent.not_before_frame.is_none() {
            intent.not_before_frame = Some(self.control.frame_counter.saturating_add(1));
        }
        if intent.source_position.is_none() {
            let (raw_source, raw_sector, raw_layer, door_handle, door_direction) = {
                let entity = self.get_entity(entity_id).unwrap_or_else(|| {
                    panic!("AI GoTo source actor {entity_id:?} disappeared before enqueue")
                });
                let element = entity.element_data();
                let (door_handle, door_direction) = current_door_for_route_source(entity);
                (
                    element.position_map(),
                    element.sector(),
                    element.layer(),
                    door_handle,
                    door_direction,
                )
            };
            // Original chooses the simple same-topology Move from the raw
            // actor topology. Only the cross-topology branch calls
            // AppendMoveToSequence, which adapts an actor already committed
            // to a selected door onto its far side. Snapshot that exact
            // call-time decision rather than the actor's potentially
            // different door state at the later drain.
            let goal_layer = intent.target_layer.unwrap_or(raw_layer);
            let goal_sector = intent.target_sector.or(raw_sector);
            let target_point = crate::coordinates::MapPoint::new(intent.target_x, intent.target_y);
            // GoTo compares the two `RHSector*` values directly
            // (`RHartificialintelligence.cpp:2538`). SectorHandle carries
            // only the public number, but multiple motion polygons may share
            // that number. Recover their arena identities from the two
            // authored points while both call-time positions are available.
            // This matters even when FindPathGates later returns an empty
            // gate list: AppendMoveToSequence still emits its leading
            // AssertPosition.
            let source_target_sector_identity_differs = raw_sector
                .filter(|source_sector| Some(*source_sector) == goal_sector)
                .is_some_and(|shared_sector| {
                    sector_hits_have_distinct_identity(
                        self.world
                            .fast_grid
                            .get_sector(raw_source, target_point, raw_layer),
                        self.world
                            .fast_grid
                            .get_sector(target_point, raw_source, goal_layer),
                        shared_sector,
                    )
                });
            intent.source_target_sector_identity_differs |= source_target_sector_identity_differs;
            let crosses_raw_topology = goal_layer != raw_layer
                || goal_sector != raw_sector
                || source_target_sector_identity_differs;
            let adapted_source = crosses_raw_topology
                .then(|| {
                    self.scripts.mission.as_ref().and_then(|_| {
                        adapt_source_to_current_door(
                            &self.script_domains.interactables.doors,
                            door_handle,
                            door_direction,
                        )
                    })
                })
                .flatten();
            let (source, sector, layer) = adapted_source
                .map(|(point, sector, layer)| {
                    (
                        point,
                        crate::position_interface::SectorHandle::new(sector),
                        layer,
                    )
                })
                .unwrap_or((raw_source, raw_sector, raw_layer));
            intent.source_position = Some(source);
            intent.source_sector = sector;
            intent.source_layer = Some(layer);
        }
        self.orders.pending_move_requests.push((entity_id, intent));
    }

    /// Drain the pending-move-request queue and launch a Move
    /// sequence element for each.  Runs once per tick from the
    /// hourglass pipeline.  Determinism: requests drain in FIFO order
    /// of enqueue (a `Vec` with `retain`+`push` on launch preserves
    /// this).
    pub(super) fn drain_pending_move_requests(&mut self, sim: &crate::sim_rng::SimulationContext) {
        let requests = std::mem::take(&mut self.orders.pending_move_requests);
        let mut deferred = Vec::new();
        for (entity_id, intent) in requests {
            if intent
                .not_before_frame
                .is_some_and(|frame| frame > self.control.frame_counter)
            {
                deferred.push((entity_id, intent));
                continue;
            }
            let launched = self.do_launch_ai_move(sim, entity_id, &intent);
            if launched.is_some() && intent.halt_after_launch_for_path_waiter {
                self.halt_actor(entity_id);
            }
            self.resolve_ai_engine_completion_verdict(entity_id);
        }
        // Work authored while draining may already have appended newer
        // intents. The retained older FIFO prefix stays ahead of those.
        if !deferred.is_empty() {
            deferred.append(&mut self.orders.pending_move_requests);
            self.orders.pending_move_requests = deferred;
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
            // A continuation that resumes an Original call stack after its
            // authored manager boundary may construct a movement now but must
            // leave its instruction to the next ordinary drain. The global
            // `drain_pending_move_requests` intentionally ignores this marker
            // when that boundary arrives.
            if entity_id == owner
                && !request
                    .1
                    .not_before_frame
                    .is_some_and(|frame| frame > self.control.frame_counter)
            {
                owner_requests.push(request);
            } else {
                remaining.push(request);
            }
        }
        self.orders.pending_move_requests = remaining;
        let mut launched = Vec::new();
        for (_, intent) in owner_requests {
            if let Some(sequence_id) = self.do_launch_ai_move(sim, owner, &intent) {
                if intent.halt_after_launch_for_path_waiter {
                    // RHArtificialIntelligence::GoTo checks
                    // `mpMe->IsComputingPath()` after LaunchSequence. In this
                    // recursive path-waiter case it remains true, so Halt
                    // synchronously interrupts the newly registered Move
                    // before SequenceManager::Hourglass can instruct it.
                    self.halt_actor(owner);
                } else {
                    launched.push(sequence_id);
                }
            }
            self.resolve_ai_engine_completion_verdict(owner);
        }
        launched
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
        let (source, source_layer, source_sector) = if let Some(source) = intent.source_position {
            (
                source,
                intent.source_layer.unwrap_or_else(|| {
                    panic!("AI GoTo for {entity_id:?} captured a source position without a layer")
                }),
                intent.source_sector,
            )
        } else {
            // Backward-compatible fallback for old serialized intents that
            // predate the enqueue-time topology snapshot.
            self.scripts
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
                .unwrap_or((raw_source, raw_source_layer, raw_source_sector))
        };
        let goal_layer = intent.target_layer.unwrap_or(source_layer);
        let goal_sector = intent.target_sector.or(source_sector);
        let move_flags =
            crate::sequence::MoveFlags::from_bits_truncate(u32::from(intent.move_flags));

        let action = intent.order_type;
        // A layer transition requires gate routing even when the numeric
        // sector handle happens to remain the same.
        let crosses_topology = goal_layer != source_layer
            || goal_sector != source_sector
            || intent.source_target_sector_identity_differs;
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
                        &|sector| self.building_sector_is_authorized(sector),
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
            // Original FindPathGates can only authorize a cross-sector
            // AppendMoveToSequence with at least one gate. If our compact
            // sector handles compare equal while retained pointer provenance
            // says they differ, its empty same-number result is failure, not
            // a direct Move. In particular, do not replace the actor's
            // existing sequence in this case.
            if gate_path.is_empty() && intent.source_target_sector_identity_differs {
                tracing::warn!(
                    ?entity_id,
                    source_sector = u16::from(source_sector),
                    goal_sector = u16::from(goal_sector),
                    identity_differs = intent.source_target_sector_identity_differs,
                    "cross-sector AI GoTo resolved to an empty gate route"
                );
                self.set_ai_couldnt_reachpoint(entity_id);
                return None;
            }
            let mut prefix = Vec::new();
            if intent.quit_swordfight_before_move {
                prefix.push(crate::sequence::SequenceElement::new(
                    1,
                    crate::element::Command::QuitSwordfight,
                    Some(entity_id),
                ));
            }
            if intent.enter_swordfight_before_move {
                prefix.push(Self::goto_enter_swordfight_element(
                    prefix.len() as u16 + 1,
                    entity_id,
                ));
            }
            if intent.stop_menace_before_move {
                prefix.push(crate::sequence::SequenceElement::new(
                    prefix.len() as u16 + 1,
                    crate::element::Command::StopMenace,
                    Some(entity_id),
                ));
            }
            if intent.lower_shield_before_move {
                prefix.push(crate::sequence::SequenceElement::new(
                    prefix.len() as u16 + 1,
                    crate::element::Command::LowerShield,
                    Some(entity_id),
                ));
            }
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
            tracing::debug!(
                target: "parity_rng_owner",
                frame = self.control.frame_counter,
                owner = ?entity_id,
                caller = "do_launch_ai_move",
                source_x = source.x,
                source_y = source.y,
                source_layer,
                source_sector = u16::from(source_sector),
                goal_x = dest.x,
                goal_y = dest.y,
                goal_layer,
                goal_sector = u16::from(goal_sector),
                action = ?action,
                move_flags = move_flags.bits(),
                tolerance = intent.tolerance,
                speed_factor = intent.speed_factor,
                quit_swordfight = intent.quit_swordfight_before_move,
                stop_menace = intent.stop_menace_before_move,
                door_goal = ?door_goal,
                gate_path = ?gate_path,
                "about to build cross-sector AI GoTo sequence"
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

        let move_level = 1
            + u16::from(intent.quit_swordfight_before_move)
            + u16::from(intent.enter_swordfight_before_move)
            + u16::from(intent.stop_menace_before_move)
            + u16::from(intent.lower_shield_before_move);
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
        if intent.enter_swordfight_before_move {
            let level = sequence
                .last()
                .map_or(1, |element| element.command_level.saturating_add(1));
            sequence.append_element(Self::goto_enter_swordfight_element(level, entity_id));
        }
        if intent.stop_menace_before_move {
            sequence.append_element(crate::sequence::SequenceElement::new(
                sequence
                    .last()
                    .map_or(1, |element| element.command_level.saturating_add(1)),
                crate::element::Command::StopMenace,
                Some(entity_id),
            ));
        }
        if intent.lower_shield_before_move {
            sequence.append_element(crate::sequence::SequenceElement::new(
                sequence
                    .last()
                    .map_or(1, |element| element.command_level.saturating_add(1)),
                crate::element::Command::LowerShield,
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

    /// Test the gate portion of `AppendMoveToSequence` before registering an
    /// AI move that will immediately be cancelled by GoTo's legacy
    /// `IsComputingPath` tail.
    ///
    /// The Original constructs a cross-topology sequence synchronously. If
    /// gate construction fails, `GoTo` publishes `mbCouldntReachpoint` before
    /// it notices and halts the outgoing `MOVE_WAITING` element
    /// (`RHartificialintelligence.cpp:2538-2580,2614-2620`). Rust normally
    /// defers construction through `pending_move_requests`; that tail halt
    /// would otherwise erase the raw intent before the failure can be seen.
    fn ai_move_gate_route_is_authorized(
        &self,
        entity_id: EntityId,
        intent: &crate::order::AiOrderIntent,
    ) -> bool {
        let Some(entity) = self.get_entity(entity_id) else {
            return false;
        };
        let ed = entity.element_data();
        let (door_handle, door_direction) = current_door_for_route_source(entity);
        let raw_source = ed.position_map();
        let raw_layer = ed.layer();
        let raw_sector = ed.sector();
        let (source, source_layer, source_sector) = if let Some(source) = intent.source_position {
            (
                source,
                intent.source_layer.unwrap_or_else(|| {
                    panic!("AI GoTo for {entity_id:?} captured a source position without a layer")
                }),
                intent.source_sector,
            )
        } else {
            self.scripts
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
                .unwrap_or((raw_source, raw_layer, raw_sector))
        };
        let goal_layer = intent.target_layer.unwrap_or(source_layer);
        let goal_sector = intent.target_sector.or(source_sector);
        if goal_layer == source_layer && goal_sector == source_sector {
            return true;
        }
        let (Some(source_sector), Some(goal_sector), Some(_)) =
            (source_sector, goal_sector, self.scripts.mission.as_ref())
        else {
            return false;
        };
        let auth = entity.actor_auth_info();
        let level = &self.world.fast_grid.level;
        let move_flags =
            crate::sequence::MoveFlags::from_bits_truncate(u32::from(intent.move_flags));
        let door_goal = self
            .grid_sector_by_number(crate::sector::SectorNumber::new(
                u16::from(goal_sector) as i16
            ))
            .filter(|sector| sector.sector_type.is_door())
            .and_then(|sector| sector.door_index)
            .map(crate::gate::DoorIndex);
        let goal = (intent.target_x, intent.target_y);
        if let Some(door_index) = door_goal {
            crate::gate::find_path_into_door(
                &self.script_domains.interactables.doors,
                (source.x, source.y),
                u16::from(source_sector),
                door_index,
                Some(&auth),
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
            .is_some()
        } else {
            crate::gate::find_path_gates(
                &self.script_domains.interactables.doors,
                (source.x, source.y),
                u16::from(source_sector),
                goal,
                u16::from(goal_sector),
                Some(&auth),
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
            .is_some()
        }
    }

    /// The raise-sword element `RHArtificialIntelligence::GoTo` inserts into
    /// the movement's own sequence for `GOTO_SWORD` when the actor is not
    /// already in a sword action state
    /// (`original-code/RHartificialintelligence.cpp:2486-2495`): a generic
    /// `ENTER_SWORDFIGHT` with a null opponent, no jump-line destination and
    /// `SWORDFIGHT_PREPARED` cleared.
    fn goto_enter_swordfight_element(
        command_level: u16,
        entity_id: EntityId,
    ) -> crate::sequence::SequenceElement {
        let mut element = crate::sequence::SequenceElement::new_generic(
            command_level,
            crate::element::Command::EnterSwordfight,
            Some(entity_id),
        );
        element.set_property(
            crate::sequence::Field::Opponent,
            crate::sequence::FieldValue::Integer(0),
        );
        element.set_property(
            crate::sequence::Field::JumplineDestination,
            crate::sequence::FieldValue::Integer(0),
        );
        element.set_property(
            crate::sequence::Field::SwordfightPrepared,
            crate::sequence::FieldValue::Bool(false),
        );
        element
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
        let (action, door_index, current_sector, execute_order_initialising, position) = self
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
                    entity.element_data().position_map(),
                ))
            })
            .unwrap_or_else(|| panic!("globally frozen movement owner {owner:?} is not an actor"));
        let Some(expected_lift_type) = climb_lift_type(action) else {
            return;
        };

        let selected_order = self
            .orders
            .sequence_manager
            .get_element(selected.seq_id, selected.elem_idx)
            .and_then(|element| element.current_order())
            .filter(|order| order.order_id == selected.order_id)
            .expect("globally frozen climb owner lost its selected order");
        let lift_direction = if let Some(door_index) = door_index {
            let door = self
                .script_domains
                .interactables
                .doors
                .get(usize::from(door_index))
                .unwrap_or_else(|| {
                    panic!(
                        "globally frozen climb owner {owner:?} references missing door {door_index}"
                    )
                });
            if door.door_type == crate::gate::DoorType::BuildingTrap
                && action == OrderType::ClimbingLadderDown
                && selected_order.reverse
                && position == MapPoint::new(selected_order.target_x, selected_order.target_y)
            {
                // TODO(parity): Original casts the BuildingTrap's inside
                // RHSectorBuilding to RHSectorLift in the decorative ladder
                // Execute arm. Three shipped traces consistently expose zero
                // from that invalid release-build read. Preserve that narrow
                // compatibility result without applying it to real ladders or
                // to a decorative row which still has distance to travel.
                None
            } else if !door_type_uses_lift_climb_direction(door.door_type) {
                // Building-trap passes deliberately contain a decorative
                // ClimbingLadderDown order even though their inside sector is
                // a building. It skips only the lift-facing setup; the climb
                // Execute arm still calls Turn below while sprites are frozen.
                None
            } else {
                Some(door.sector_in)
            }
        } else {
            let sector = current_sector.unwrap_or_else(|| {
                panic!("globally frozen climb owner {owner:?} has no lift sector")
            });
            Some(crate::sector::SectorNumber::new(i16::from(sector)))
        }
        .map(|sector_number| {
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
            if action == OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel {
                (lift.lift_direction + 8) & 15
            } else {
                lift.lift_direction
            }
        });
        let lift_direction = if execute_order_initialising
            && door_index.is_some_and(|door_index| {
                self.script_domains
                    .interactables
                    .doors
                    .get(usize::from(door_index))
                    .is_some_and(|door| {
                        door.door_type == crate::gate::DoorType::BuildingTrap
                            && action == OrderType::ClimbingLadderDown
                            && selected_order.reverse
                            && position
                                == MapPoint::new(selected_order.target_x, selected_order.target_y)
                    })
            }) {
            Some(0)
        } else {
            lift_direction
        };
        let turns = if is_fast_climb_action(action) { 2 } else { 1 };
        let entity = self
            .world
            .entities
            .get_mut(owner)
            .expect("globally frozen climb owner disappeared after canonical lookup");
        if execute_order_initialising && let Some(direction) = lift_direction {
            entity.element_data_mut().set_direction_goal(direction);
        }
        for _ in 0..turns {
            entity.element_data_mut().sprite.position_iface.turn();
        }
    }

    fn execute_globally_frozen_pre_motion_owner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        selected: MovementOwnerSelection,
    ) -> OrderType {
        let (order_action, flags, target, destination) = self
            .orders
            .sequence_manager
            .get_element(selected.seq_id, selected.elem_idx)
            .and_then(|element| {
                let (flags, target, destination) = match &element.data {
                    crate::sequence::SequenceElementData::Movement {
                        flags,
                        element,
                        destination,
                        ..
                    } => (*flags, *element, *destination),
                    _ => return None,
                };
                element
                    .current_order()
                    .filter(|order| order.order_id == selected.order_id)
                    .map(|order| (order.order_type, flags, target, destination))
            })
            .expect("globally frozen movement owner lost its selected order");
        if climb_lift_type(order_action).is_some() {
            self.turn_globally_frozen_climb_owner(owner, selected);
            return order_action;
        }

        if flags.contains(crate::sequence::MoveFlags::SEEK) {
            let (owner_position, owner_sector, seek_target, seek_distance, has_post_seek) = self
                .world
                .entities
                .get(owner)
                .and_then(|entity| {
                    let actor = entity.actor_data()?;
                    Some((
                        entity.element_data().position_map(),
                        entity.element_data().sector(),
                        actor.seek_target,
                        actor.seek_distance,
                        actor.post_seek_sequence.is_some(),
                    ))
                })
                .unwrap_or_else(|| panic!("globally frozen seek owner {owner:?} is not an actor"));

            // `mbSeekToPoint` takes the unconditional Turn/PerformMotion arm.
            // An entity seek whose target was cleared instead returns
            // TERMINATED before touching either the wait counter or facing.
            let Some(seek_target) = seek_target else {
                if target.is_none() {
                    self.world
                        .entities
                        .get_mut(owner)
                        .expect("globally frozen point-seek owner disappeared")
                        .position_iface_mut()
                        .turn();
                }
                return order_action;
            };
            assert_eq!(
                target,
                Some(seek_target),
                "globally frozen seek owner {owner:?} has inconsistent actor/element targets"
            );

            let target_entity = self.world.entities.get(seek_target).unwrap_or_else(|| {
                panic!(
                    "globally frozen seek owner {owner:?} references missing target {seek_target:?}"
                )
            });
            let target_position = target_entity.element_data().position_map();
            let target_sector = target_entity.element_data().sector();
            let use_point = flags.contains(crate::sequence::MoveFlags::USE_POINT);
            let point = if use_point {
                target_entity
                    .cxx_current_point_map()
                    .filter(|point| *point != target_position)
                    .unwrap_or(target_position)
            } else {
                target_position
            };
            let delta = if flags.contains(crate::sequence::MoveFlags::SEEK_SHIELD) {
                assert!(
                    self.world
                        .entities
                        .get(owner)
                        .is_some_and(crate::element::Entity::is_pc),
                    "SEEK_SHIELD owner {owner:?} is not a PC"
                );
                destination - owner_position
            } else {
                point - owner_position
            };
            let dy = if flags.contains(crate::sequence::MoveFlags::DIRECTIONAL_TOLERANCE) {
                delta.y * 1.743_446_8
            } else {
                delta.y
            };
            let in_tolerance = owner_sector == target_sector
                && delta.x * delta.x + dy * dy < seek_distance * seek_distance * 1.1025;

            if in_tolerance {
                if has_post_seek {
                    if debug_post_seek_handoff_enabled() {
                        eprintln!(
                            "[POST_SEEK frame={} owner={owner:?} stage=frozen_in_tolerance selected={:?} actors_frozen={}]",
                            self.control.frame_counter,
                            (selected.seq_id, selected.elem_idx, selected.order_id),
                            self.actors_frozen(),
                        );
                    }
                    let launched = self.start_post_seek_sequence(
                        sim,
                        assets,
                        owner,
                        Some((selected.seq_id, selected.elem_idx)),
                    );
                    if debug_post_seek_handoff_enabled() {
                        eprintln!(
                            "[POST_SEEK frame={} owner={owner:?} stage=frozen_launch_done launched={launched} current={:?}]",
                            self.control.frame_counter,
                            self.orders
                                .sequence_manager
                                .current_element_for_actor(owner),
                        );
                    }
                    return order_action;
                }
                // PerformAction(FROZEN) returns before sprite motion, then
                // PerformSeek ages the shared unsigned wait scalar.
                let actor = self
                    .world
                    .entities
                    .get_mut(owner)
                    .and_then(|entity| entity.actor_data_mut())
                    .expect("globally frozen seek owner lost actor data");
                actor.seek_refresh_wait = age_seek_refresh_wait(actor.seek_refresh_wait);
                actor.wait_time = actor.seek_refresh_wait;
                return order_action;
            }

            // The moved-target refresh test runs in
            // `tick_refresh_seek_for_owner` immediately before this owner
            // Execute. If it did not replace the seek, PerformSeek ages the
            // counter and turns before frozen PerformMotion returns.
            let entity = self
                .world
                .entities
                .get_mut(owner)
                .expect("globally frozen seek owner disappeared before Turn");
            let actor = entity
                .actor_data_mut()
                .expect("globally frozen seek owner lost actor data before Turn");
            actor.seek_refresh_wait = age_seek_refresh_wait(actor.seek_refresh_wait);
            actor.wait_time = actor.seek_refresh_wait;
            entity.position_iface_mut().turn();
            return order_action;
        }

        if !order_turns_before_motion(order_action) {
            return order_action;
        }
        self.world
            .entities
            .get_mut(owner)
            .unwrap_or_else(|| panic!("globally frozen movement owner {owner:?} disappeared"))
            .position_iface_mut()
            .turn();
        order_action
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

        // Human::Execute calls the selected movement element's virtual
        // Stop(Injury) before registering QuitSwordfight. That exact-root
        // stop follows only the element's linked successor/postponed graph;
        // in particular, a FaceTo Turn queued by EventReachPoint is
        // interrupted and sends its condolence card on this stack. Do not use
        // Actor::Stop here: its pending-list scan would also stop unrelated
        // work, and retiring the movement before this boundary would release
        // the Turn to be inherited by QuitSwordfight.
        let selected_priority = {
            let resolver = Self::priority_resolver(&self.world.entities);
            let element = self
                .orders
                .sequence_manager
                .get_element_mut(selected.seq_id, selected.elem_idx)
                .expect("selected orphan sword movement disappeared before Stop");
            if element.priority == crate::sequence::SequencePriority::NotYetSet {
                let mut resolved = resolver(element);
                if resolved == crate::sequence::SequencePriority::None {
                    resolved = crate::sequence::SequencePriority::Normal;
                }
                element.priority = resolved;
            }
            element.priority
        };
        if selected_priority >= crate::sequence::SequencePriority::Injury {
            let owner_pos = self
                .get_entity(owner)
                .expect("orphan sword movement owner disappeared before Stop")
                .element_data()
                .position_map();
            {
                let resolver = Self::priority_resolver(&self.world.entities);
                let pathfinder = &mut self.world.pathfinder;
                self.orders.sequence_manager.stop_movement_from_root(
                    owner,
                    (selected.seq_id, selected.elem_idx),
                    owner_pos,
                    crate::sequence::SequencePriority::Injury,
                    &resolver,
                    &mut self.orders.next_order_id,
                    &mut |id| pathfinder.cancel_requests_for(id),
                );
            }
            // Movement::Stop delivers the selected movement's card from
            // StopMovement before its base SequenceElement::Stop walks the
            // linked successor/postponed graph.  That callback may re-enter
            // AI and mutate the graph, so it is a real owner boundary rather
            // than a batchable cleanup detail.
            self.dispatch_condolations_for_owner_boundary(sim, owner, assets);
            {
                let resolver = Self::priority_resolver(&self.world.entities);
                self.orders.sequence_manager.stop_owner_current_from_root(
                    owner,
                    Some((selected.seq_id, selected.elem_idx)),
                    crate::sequence::SequencePriority::Injury,
                    &resolver,
                );
            }
        }
        self.dispatch_condolations_for_owner_boundary(sim, owner, assets);
        // Human::Execute only registers this command here. Its ABORTED return
        // reaches Actor::Hourglass first; SequenceManager::Hourglass later
        // calls the ordinary Actor::Instruct path, which translates the
        // lowering order and overwrites mmotionState with IN_PROGRESS. Direct
        // prebuilt-order instruction at this Execute boundary left the later
        // ABORTED latch authoritative for the whole frame.
        self.launch_element(crate::sequence::SequenceElement::new(
            1,
            crate::element::Command::QuitSwordfight,
            Some(owner),
        ));

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
            self.dispatch_synchronous_ai_think_preserving_detection_fifo(
                sim,
                owner,
                assets,
                crate::ai::Stimulus::new(crate::ai::StimulusType::EventQuitSwordfight),
            );
        }

        // Actor::Hourglass captures the entry movement before Execute. Only
        // after Human::Execute has returned ABORTED does it mark that captured
        // element Impossible. Keep this after the direct soldier callback so
        // neither QuitSwordfight nor the callback can inherit the stopped
        // Turn's cross-element link.
        self.orders
            .sequence_manager
            .element_impossible(selected.seq_id, selected.elem_idx);
        self.dispatch_condolations_for_owner_boundary(sim, owner, assets);
        true
    }

    /// Resolve the point `RHElementActorHuman::FaceOpponent`
    /// (RHelementactorhuman.cpp:7480) or `RHElementActorPC::FaceDangerPoint`
    /// would face for this owner, plus whether that point is compared against
    /// the actor's ground position rather than its map position.
    ///
    /// `None` reproduces FaceOpponent's non-soldier, non-swordfighting early
    /// return, which yields `WALKING_SWORD` without touching the facing.
    pub(super) fn combat_face_target_for_owner(&self, owner: EntityId) -> (Option<MapPoint>, bool) {
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
            let is_shield_moving = matches!(
                actor.action_state,
                crate::element::ActionState::MovingShield
            );
            // Shield bearers face the stored danger point.
            // Sword fighters face their principal opponent.
            if is_shield_moving && let Some(pt) = actor.shield_face_point {
                combat_face_target = Some(pt);
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
                combat_face_target = Some(crate::coordinates::MapPoint {
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
            // The non-soldier, non-swordfighting branch returns
            // `WALKING_SWORD` immediately, without constructing a facing
            // vector. Keep that distinct as `None`: using the actor's own
            // position as a sentinel is not equivalent because Position and
            // PositionGround can differ while cached projection state is
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
                // GetPrimaryTarget — soldier's AI-picked priority target,
                // which can differ from opponents[0]. The stored handle is a
                // raw element slot and the occupant is any human, not just a
                // PC: soldiers routinely keep an enemy soldier as their
                // primary target once a swordfight has ended, and facing it
                // is what keeps the fighter turned toward the melee.
                entity
                    .ai_controller()
                    .map(|c| c.primary_target)
                    .filter(|slot| *slot != 0)
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
    /// whose `PerformSeek` is about to take its moved-target `RefreshSeek`
    /// branch.
    ///
    /// `RHElementActorHuman::Execute` calls `FaceOpponent` at
    /// RHelementactorhuman.cpp:3662 and `RHElementActorPC::Execute` calls
    /// `FaceDangerPoint` at RHelementactorpc.cpp:5514, both *before* entering
    /// `RHElementActor::PerformSeek` (RHelementactorhuman.cpp:3667,
    /// RHelementactorpc.cpp:5541).  PerformSeek's moved-target branch
    /// (RHelementactor.cpp:7913) returns `RHMOTION_IN_PROGRESS` without ever
    /// reaching `PerformMotion`, so `FaceOpponent`'s
    /// `SetDirection( vDirection.GetSector0to15( ASPECT_RATIO ) )` and its
    /// following `Turn()` (RHelementactorhuman.cpp:7511-7512) still land on the
    /// RefreshSeek frame.  Rust evaluates RefreshSeek ahead of the movement
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
        let Some(seq_id) = actor.active_movement.sequence_id else {
            return;
        };
        let elem_idx = actor.active_movement.element_index;
        let Some(order_action) = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .and_then(|element| element.orders.front())
            .map(|order| order.order_type)
        else {
            return;
        };
        let is_shield_motion = matches!(action_state, crate::element::ActionState::MovingShield);
        let is_sword_motion = is_sword_motion_context(action_state, door_pass_anim, order_action);
        let (combat_target, target_is_ground) = self.combat_face_target_for_owner(owner);
        // Human's arm always calls FaceOpponent; the PC shield arm always
        // calls FaceDangerPoint. Both return without writing when no facing
        // point exists, which is exactly `combat_target == None`.
        if !((is_shield_motion && combat_target.is_some()) || is_sword_motion) {
            return;
        }
        let Some(opp_pos) = combat_target else {
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
        let selected_is_live = self
            .orders
            .sequence_manager
            .get_element(selected.seq_id, selected.elem_idx)
            .filter(|element| element.owner == Some(owner) && element.data.is_movement())
            .and_then(|element| element.current_order())
            .is_some_and(|order| order.order_id == selected.order_id);
        if !selected_is_live {
            return MovementOwnerMotion::default();
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
            return MovementOwnerMotion::default();
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
            return MovementOwnerMotion::default();
        }
        if self.abort_orphaned_sword_movement(sim, assets, owner, selected) {
            // LaunchSequenceElement registers the replacement for the later
            // sequence-manager phase. It must not execute at this actor
            // boundary: Original exposes QuitSwordfight as the current
            // command for one frame before its lowering order starts.
            return MovementOwnerMotion::default();
        }
        // `IsFrozenAll()` is read ONLY inside `RHSprite`
        // (`original-code/RHsprite.cpp:739`, `:985`, `:1042`, `:1084`, `:1124`,
        // `:1430`) and in the NPC AI gates; `RHElementActor::Execute` itself is
        // never gated on it. Its `RHNONANIMATION_PASSING_DOOR` arm
        // (`original-code/RHelementactor.cpp:2786-2807`) reaches no Sprite
        // method at all: it calls `PassDoor()` / restores anti-collision,
        // forwards `MSG_STATURE`, and returns `RHMOTION_TERMINATED` whatever
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
            // A globally frozen Sprite::PerformMotion returns IN_PROGRESS
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
            // Execute arms: it calls SetStates(MOVING_FAST) unconditionally
            // after PerformMotion, not only for RHMOTION_START. Therefore the
            // IN_PROGRESS returned by globally frozen sprite motion still
            // changes a waiting actor to moving-fast.
            if frozen_order == OrderType::RunningUpright {
                let entity = self
                    .world
                    .entities
                    .get_mut(owner)
                    .unwrap_or_else(|| panic!("globally frozen runner {owner:?} disappeared"));
                entity.element_data_mut().posture = crate::element::Posture::Upright;
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
            // FrozenAll suppresses Sprite::PerformMotion but not the Execute
            // work before it: climb Turn() above and both rider-specific
            // Soldier arms remain live. RiderCharging performs its polygon
            // work or RunningUpright samples that frozen frame and may Think.
            let charge_execution = self.tick_rider_charge_owner(sim, assets, owner, true, None);
            if charge_execution.is_none() && self.selected_galopp_decision_frame(owner, selected) {
                self.dispatch_galopp_loop_event(sim, assets, owner);
            }
            return MovementOwnerMotion::default();
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
        let (combat_face_target, combat_face_target_is_ground) =
            self.combat_face_target_for_owner(owner);

        // Pre-pass: look up the current sequence-element speed factor
        // for every entity with an active movement
        // (`distance *= speed_factor` during the per-frame motion
        // update). Pre-computed here so the main loop can borrow
        // `self.world.entities` mutably while consulting
        // `self.orders.sequence_manager` for the factor.
        let mut speed_factor = 1.0;
        // `RHSprite::PerformMotion` passes its cached `mpTargetElement` into
        // `RHPositionInterface::IsGoalReached`.  The target radius matters
        // when anti-collision left the actor deviated and blocked: Original
        // accepts center separation below `self.radius + target.radius + 10`
        // even when the ordinary zero tolerance has not crossed the waypoint.
        // Snapshot it before the mutable movement pass for the same reason as
        // the speed factor and live-seek metadata below.
        let mut goal_target_info = None;
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
        let mut final_tolerance = FinalTol::default();
        let mut point_seek_post_sector = None;
        for (_actor_id, entity) in self
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
                speed_factor = elem.speed_factor();
                goal_target_info = elem
                    .current_order()
                    .and_then(|order| order.antagonist)
                    .and_then(|target| self.world.entities.get(target))
                    .map(|target| crate::position_interface::TargetInfo {
                        radius: target.position_iface().get_radius(),
                    });
                if let crate::sequence::SequenceElementData::Movement {
                    flags,
                    tolerance: _,
                    element: target_elem,
                    destination,
                    sector: _,
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
                        let (resolved_target_id, target_is_actor) = match target_elem
                            .and_then(|id| self.get_entity(id))
                        {
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
                                if actor.post_seek_sequence.is_some()
                                    && actor.continuation.seek_to_point
                                {
                                    point_seek_post_sector = match actor.continuation.seek_sector {
                                        Some(crate::actor_state::ActorSeekSector::Position(
                                            sector,
                                        )) => Some(sector),
                                        // Runtime point Seek stores a
                                        // Position sector. A legacy Door
                                        // pointer cannot equal the
                                        // actor's ordinary sector here,
                                        // matching Original's pointer
                                        // comparison in PerformSeek.
                                        Some(crate::actor_state::ActorSeekSector::Door(_))
                                        | None => None,
                                    };
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
                            final_tolerance = FinalTol {
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
                                launches_post_seek: actor.post_seek_sequence.is_some(),
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
        let mut lift_translation = None;
        let mut door_pass_climb_direction = None;
        let mut decorative_building_trap_at_destination = false;
        for (actor_id, entity) in self
            .world
            .entities
            .actors()
            .filter(|(id, _)| EntityId::from(*id) == owner)
        {
            let posture = entity.element_data().posture;
            let door_pass = entity
                .actor_data()
                .and_then(|actor| actor.active_door_pass.as_ref());
            let door_pass_action = door_pass.map(|dp| dp.current_action);
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
                            .then_some(door.sector_in)
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
                    // TODO(parity): Original's decorative BuildingTrap row
                    // invalidly casts its RHSectorBuilding to RHSectorLift.
                    // The three shipped witnesses read direction zero. Keep
                    // that release-build compatibility value confined to the
                    // exact-target reverse row which immediately terminates.
                    door_pass_climb_direction = Some(0);
                    decorative_building_trap_at_destination = true;
                }
            }
            let Some(lt) = gs.lift_type else { continue };
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
                    let (pt_low, pt_high) = self.lift_endpoint_points(gs.sector_number);
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
                let (pt_low, pt_high) = self.lift_endpoint_points(gs.sector_number);
                lift_translation = Some(LiftAnimContext::OnClimb {
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
        let mut anti_snapshots =
            super::anti_collision::snapshot_all(&self.world.entities, &assets.profile_manager);

        let prepass = MovementPrepass {
            combat_face_target,
            combat_face_target_is_ground,
            speed_factor,
            goal_target_info,
            final_tolerance,
            point_seek_post_sector,
            lift_translation,
            door_pass_climb_direction,
            decorative_building_trap_at_destination,
        };
        if let Some(entity) = self.world.entities.get(owner) {
            super::animation::direction_provenance_snapshot(
                entity.position_iface(),
                owner,
                self.control.frame_counter,
                "movement_after_prepass",
            );
        }
        let mut deferred = MovementDeferred::default();

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
        for actor_id in movement_actor_ids {
            self.tick_one_movement_actor(
                sim,
                assets,
                owner,
                selected,
                actor_id,
                &prepass,
                &prepared,
                &mut anti_snapshots,
                &mut deferred,
            );
        }
        let MovementDeferred {
            post_completion_motion_override,
            sword_movement_starts,
            sword_movement_terminations,
            door_triggers,
            transition_pushes,
            select_triggers,
            completed_door_passes,
            galopp_event,
            blocked_impossible,
            door_pass_transition_start_effects,
            door_pass_transition_done_effects,
            door_pass_transition_completion_effects,
            terminal_door_pass_goal_clears,
            post_seek_arrivals,
            post_seek_terminal_state_effects,
            sequence_seek_terminal_state_effects,
            line_cross_checks,
            non_elevation_cross_checks,
            transition_seek_refreshes,
            mut order_pops,
            terminal_pc_direction_goal_restores,
            water_splash_emits,
            movement_state_effects,
            executed_pc_movement_actions,
            executed_sword_movement,
        } = deferred;
        if debug_post_seek_handoff_enabled() {
            let actor_seek = self.world.entities.get(owner).and_then(|entity| {
                let actor = entity.actor_data()?;
                Some((
                    actor.seek_target,
                    actor.post_seek_sequence.is_some(),
                    actor.active_door_pass.is_some(),
                ))
            });
            eprintln!(
                "[POST_SEEK frame={} owner={owner:?} stage=movement_deferred actors_frozen={} arrivals={post_seek_arrivals:?} order_pops={order_pops:?} actor_seek={actor_seek:?}]",
                self.control.frame_counter,
                self.actors_frozen(),
            );
        }

        // The PC WalkingWithCorpse override moves the carried actor inside
        // this Execute arm, immediately after the carrier's PerformMotion.
        // Later creation slots (including NPC RefreshDetection) therefore see
        // the body's new position in the same frame.
        for &(carrier_id, action) in &executed_pc_movement_actions {
            if action == OrderType::WalkingWithCorpse {
                crate::abilities::sync_walking_corpse_for_carrier(
                    &mut self.world.entities,
                    &assets.profile_manager,
                    carrier_id,
                );
            }
        }

        for entity_id in sword_movement_starts {
            self.apply_sword_movement_start_initiative_transfer(entity_id);
        }
        let state_effect_frame = self.control.frame_counter;
        for (entity_id, posture, action_state) in movement_state_effects {
            if let Some(entity) = self.get_entity_mut(entity_id) {
                tracing::trace!(
                    target: "robin_engine::engine::movement::state_effect",
                    ?entity_id,
                    frame = state_effect_frame,
                    ?posture,
                    ?action_state,
                    "movement Execute state side effect"
                );
                entity.set_posture(posture);
                if let Some(actor) = entity.actor_data_mut() {
                    actor.action_state = action_state;
                }
            }
        }
        // Human's sword-movement Execute arm removes newly far opponents
        // immediately after PerformMotion/PerformSeek and before inspecting
        // the terminal motion state (`RHelementactorhuman.cpp:3778-3844`).
        // Besides removing the old principal, this may synchronously promote
        // a reciprocal in-range opponent to principal, so the Provoke gate
        // must observe the updated list.
        if executed_sword_movement {
            self.quit_swordfight_with_far_opponents(sim, assets, owner);
        }

        // Original evaluates the terminal sword-movement Provoke gate inside
        // Human::Execute, after the far-opponent removal above but before base
        // Actor::Hourglass runs line-crossing callbacks. Those callbacks may
        // project the actor onto a different elevation and move the live 3D
        // position across a weapon-range boundary. Snapshot the complete
        // mutual-opponent/range decision at that exact boundary.
        let sword_movement_provokes = sword_movement_terminations
            .into_iter()
            .filter(|&entity_id| {
                self.sword_movement_termination_warrants_provoke(assets, entity_id)
            })
            .collect::<Vec<_>>();
        // LaunchSequenceElement only registers this ordinary-priority work;
        // its later Go/Instruct still runs after terminal order advancement.
        // Register now so a synchronous StartPostSeekSequence registers its
        // SpeakHeroReachDestination behind the Provoke, as in Original.
        for entity_id in sword_movement_provokes {
            self.launch_sword_movement_termination_provoke(entity_id);
        }
        for entity_id in door_pass_transition_start_effects {
            self.apply_door_pass_transition_start_side_effects(assets, entity_id);
        }
        for entity_id in door_pass_transition_done_effects {
            self.apply_door_pass_transition_done_side_effects(assets, entity_id);
        }
        for (entity_id, action) in door_pass_transition_completion_effects {
            self.apply_door_pass_transition_completion_side_effects(assets, entity_id, action);
        }

        // PerformSeek calls SetMovingActionState and RefreshSeek synchronously
        // from the actor's Execute arm. Actor::Hourglass observes the
        // replacement movement when it subsequently checks line crossings.
        // Keep this before crossing resolution/dispatch; delaying it until
        // afterwards lets LINE_SOUND/LINE_SCRIPT callbacks inspect the stale
        // seek and can enter the seeking AI state an extra time.
        // `PerformSeek` answers both of these branches with an explicit
        // `return RHMOTION_IN_PROGRESS` immediately after `RefreshSeek`
        // (`original-code/RHelementactor.cpp:7963-7970` for the stale final
        // waypoint, `:8002-8007` for the out-of-reach stop transition), so the
        // Execute result Actor::Hourglass latches is IN_PROGRESS regardless of
        // the state `RefreshSeek` left on the replaced element.
        let mut refreshed_seek_in_progress = false;
        for (owner, seq_id, elem_idx) in transition_seek_refreshes {
            // Re-read the seek element's flags / target / tolerance / action
            // because another staged Execute effect may have changed adjacent
            // elements. When it no longer looks like an entity-target seek,
            // skip silently.
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
                if let Some(actor) = self
                    .get_entity_mut(owner)
                    .and_then(|entity| entity.actor_data_mut())
                {
                    let before = actor.action_state;
                    actor.action_state = actor.action_state.set_moving(false, false);
                    tracing::trace!(
                        target: "parity_post_process_path",
                        ?owner,
                        ?before,
                        after = ?actor.action_state,
                        "transition seek refresh arming moving state",
                    );
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
                refreshed_seek_in_progress = true;
            }
        }

        // Resolve every queued crossing segment against the actor's live
        // position now that all Execute-arm completion branches have run.
        // CheckForLineCrossing samples GetPositionMap() at this point and
        // gathers the crossed lines once; the elevation swap and the
        // patch/script/sound tail then both work off that single segment,
        // so the endpoint must not be re-sampled between the two passes.
        // The in-bounds guard is the Original's GetBoxMap().IsInside early
        // return, likewise evaluated on the live position.
        let resolve_cross_checks = |engine: &Self, queued: Vec<(EntityId, MapPoint, u16)>| {
            queued
                .into_iter()
                .filter_map(|(entity_id, old_pos, layer)| {
                    let new_pos = engine.get_entity(entity_id)?.element_data().position_map();
                    engine
                        .world
                        .fast_grid
                        .level
                        .map_bbox
                        .contains_point(new_pos)
                        .then_some((entity_id, old_pos, new_pos, layer))
                })
                .collect::<Vec<_>>()
        };
        let line_cross_checks = resolve_cross_checks(self, line_cross_checks);
        let non_elevation_cross_checks = resolve_cross_checks(self, non_elevation_cross_checks);
        // Dispatch elevation-line crossings detected during the loop.
        // Runs as a post-pass after the per-actor movement update.
        // When a human actor crosses an elevation line, we also fire
        // `UpdateRoll` so any in-progress Rolling combat_anim can
        // re-aim its flight at the new obstacle's slope.
        for (entity_id, old_pos, new_pos, layer) in line_cross_checks {
            let crossed = self.check_for_line_crossing(assets, entity_id, old_pos, new_pos, layer);
            if crossed {
                let is_human = self
                    .expect_entity(entity_id, "line-crossing mover")
                    .is_human();
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

        for (entity_id, old_pos, new_pos, layer) in non_elevation_cross_checks {
            self.check_for_non_elevation_line_crossing(
                sim, assets, entity_id, old_pos, new_pos, layer,
            );
        }

        // RHElementActor::Hourglass dispatches CheckForLineCrossing before
        // its TERMINATED arm calls DoNextOrder. The latter can synchronously
        // retire the completed door pass and clear its movement goal. Keep
        // that ordering: an elevation crossing must recompute the increment
        // from the live destination, not from the cleared (0, 0) sentinel.
        for entity_id in terminal_door_pass_goal_clears {
            let entity = self.world.entities.get_mut(entity_id).unwrap_or_else(|| {
                panic!(
                    "terminal door-pass goal owner {entity_id:?} disappeared after line crossing"
                )
            });
            tracing::trace!(
                target: "parity_owner_handoff",
                owner = ?entity_id,
                goal = ?entity.position_iface().map_goal(),
                "door-pass stop transition clearing movement goal after line crossing"
            );
            clear_terminal_door_pass_goal(entity);
        }

        // These calls are inside the Human/PC sword movement Execute arms,
        // after PerformMotion and before base Actor completion/DoNextOrder.
        if executed_sword_movement {
            if matches!(owner, EntityId::Pc(_)) {
                let pinch_abort = self.world.entities.get(owner).and_then(|entity| {
                    entity.actor_data()?;
                    // `RHElementActorPC::Execute` gates the override on the
                    // live `mpSequenceElement`: it must exist AND must not
                    // carry `RHPRIORITY_NON_INTERRUPTABLE`
                    // (`RHelementactorpc.cpp:3667-3673`). A door pass is
                    // exactly that priority
                    // (`RHElementActor::DeterminePriority`,
                    // `RHelementactor.cpp:5506-5507`), so a sword walk that
                    // belongs to a PassDoor element never aborts — Hourglass'
                    // ABORTED arm asserts the same invariant
                    // (`RHelementactor.cpp:1206`). Without this gate the
                    // aborted pop cancelled the door pass's own order advance
                    // and the actor replayed the walk instead of reaching its
                    // PASSING_DOOR action point.
                    let selected_priority = self
                        .orders
                        .sequence_manager
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
                    Some((selected.seq_id, selected.elem_idx))
                });
                if let Some((seq_id, elem_idx)) = pinch_abort {
                    // RHElementActorPC::Execute overrides the nested Human
                    // PerformMotion result with RHMOTION_ABORTED here.  The
                    // base Actor::Hourglass therefore marks the entry-latched
                    // element Impossible and does not run its TERMINATED
                    // DoNextOrder arm, even when PerformMotion had already
                    // reached the short step-back destination.
                    cancel_aborted_order_pop(&mut order_pops, seq_id, elem_idx);
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
            self.commit_completed_door_pass_position(assets, entity_id, door_index, direct);
            self.apply_completed_door_pass_lift_entry_state(entity_id, door_index, direct);
        }

        // Push queued door-pass Transition orders onto each actor's
        // current sequence element.  The current order list — the
        // transition order blocks subsequent orders until its sprite
        // animation completes.
        for (seq_id, elem_idx, order) in transition_pushes {
            let element = self
                .orders
                .sequence_manager
                .get_element_mut(seq_id, elem_idx)
                .unwrap_or_else(|| {
                    panic!("door-pass successor has stale element handle ({seq_id:?}, {elem_idx})")
                });
            insert_door_pass_successor(element, order);
        }

        // Fire pending Select hulk flashes.
        for (entity_id, speed) in select_triggers {
            self.apply_select_hulk(entity_id, speed);
        }

        let mut post_seek_reentrant_order_advances = Vec::new();
        for (entity_id, seq_id, elem_idx) in post_seek_arrivals {
            let launched =
                self.start_post_seek_sequence(sim, assets, entity_id, Some((seq_id, elem_idx)));
            if debug_post_seek_handoff_enabled() {
                eprintln!(
                    "[POST_SEEK frame={} owner={entity_id:?} stage=ordinary_launch launched={launched} current={:?}]",
                    self.control.frame_counter,
                    self.orders
                        .sequence_manager
                        .current_element_for_actor(entity_id),
                );
            }
            if launched {
                post_seek_reentrant_order_advances.push(entity_id);
            }
        }
        // `StartPostSeekSequence` launches the actor-owned interaction from
        // inside `PerformSeek`. Close that synchronous launch before returning
        // to the surrounding movement-transition Execute switch: FaceTo and
        // similar callbacks must still observe the outgoing MovingFast state.
        self.drain_script_synchronous_actions(sim, assets, &mut Vec::new())
            .unwrap_or_else(|error| {
                panic!(
                    "movement owner {owner:?} failed to drain synchronous post-seek work: {error:?}"
                )
            });
        for (entity_id, posture, action_state) in post_seek_terminal_state_effects {
            let entity = self.get_entity_mut(entity_id).unwrap_or_else(|| {
                panic!(
                    "post-seek transition owner {entity_id:?} disappeared before its terminal state effect"
                )
            });
            entity.set_posture(posture);
            entity
                .actor_data_mut()
                .unwrap_or_else(|| {
                    panic!("post-seek transition owner {entity_id:?} is not an actor")
                })
                .action_state = action_state;
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

        // Actor::Hourglass remembers the entry-selected element only for its
        // ABORTED arm.  A successful PerformSeek interaction is different:
        // StartPostSeekSequence terminates the seek and synchronously selects
        // the interaction's first owned element, then the surrounding
        // Execute returns TERMINATED and Hourglass calls DoNextOrder through
        // the actor's *live* mpSequenceElement.  Consequently a newly queued
        // MoveWaiting can have its Freezing order consumed here while its
        // path request remains in RHPathFinder's queue.  The ordinary
        // captured order pop below intentionally rejects replacement
        // selections, so reproduce this re-entrant live-pointer seam
        // explicitly for post-seek launches only.
        for entity_id in post_seek_reentrant_order_advances {
            self.advance_live_order_after_reentrant_seek(entity_id);
        }

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
            self.pop_selected_movement_order(seq_id, elem_idx);
        }
        for (entity_id, external_direction, movement_direction) in
            terminal_pc_direction_goal_restores
        {
            let entity = self.world.entities.get_mut(entity_id).unwrap_or_else(|| {
                panic!(
                    "terminal PC direction-goal owner {entity_id:?} disappeared during movement completion"
                )
            });
            // A synchronously instructed successor may deliberately have
            // installed a third direction.  Only replace the outgoing Move's
            // trajectory direction; never overwrite such successor work.
            if i16::from(entity.position_iface().get_direction_goal()) == movement_direction {
                entity
                    .element_data_mut()
                    .set_direction_goal(external_direction);
            }
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

        for (entity_id, posture, action_state) in sequence_seek_terminal_state_effects {
            let entity = self.get_entity_mut(entity_id).unwrap_or_else(|| {
                panic!(
                    "sequence-seek transition owner {entity_id:?} disappeared before its terminal state effect"
                )
            });
            entity.set_posture(posture);
            entity
                .actor_data_mut()
                .unwrap_or_else(|| {
                    panic!("sequence-seek transition owner {entity_id:?} is not an actor")
                })
                .action_state = action_state;
        }

        MovementOwnerMotion {
            initial: refreshed_seek_in_progress.then_some(crate::sprite::MotionState::InProgress),
            post_completion_override: post_completion_motion_override,
        }
    }

    /// Movement Execute body for the single movement owner. The caller's
    /// actor-id collection filters the entity table down to `actor_id ==
    /// owner`, so this runs at most once per `tick_entity_movement_owner`
    /// call; every early `return` is a per-actor "done" exit.
    #[allow(clippy::too_many_arguments)]
    fn tick_one_movement_actor(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &crate::engine::LevelAssets,
        owner: EntityId,
        selected: MovementOwnerSelection,
        actor_id: crate::entity_id::ActorId,
        prepass: &MovementPrepass,
        prepared: &LiveMobileGeometry,
        anti_snapshots: &mut EntitySlots<Option<super::anti_collision::ActorSnapshot>>,
        deferred: &mut MovementDeferred,
    ) {
        let mut speed_factor = prepass.speed_factor;
        let entity_id = actor_id.into();
        let rider_entry_compute_direction = self
            .orders
            .sequence_manager
            .get_element(selected.seq_id, selected.elem_idx)
            .and_then(|element| element.current_order())
            .filter(|order| order.order_id == selected.order_id)
            .map(|order| order.compute_direction);
        if let Some(charge_execution) = self.tick_rider_charge_owner(
            sim,
            assets,
            entity_id,
            false,
            Some((prepared, anti_snapshots)),
        ) {
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
                // Actor::Hourglass advances only after line-crossing
                // callbacks. ExecuteRiderCharge may legitimately call NewID
                // on its same `pOrder`; compare against that post-Execute
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
                    self.do_next_order(selected.seq_id, selected.elem_idx);
                }
                if let Some(actor) = self
                    .world
                    .entities
                    .get_mut(entity_id)
                    .and_then(Entity::actor_data_mut)
                {
                    actor.active_rider_charge = None;
                }
                if let Some(entity) = self.world.entities.get_mut(entity_id) {
                    entity.element_data_mut().sprite.last_motion_state = charge_motion;
                }
            } else if charge_motion == Some(MotionState::Aborted) {
                deferred
                    .blocked_impossible
                    .push((selected.seq_id, selected.elem_idx));
                if let Some(entity) = self.world.entities.get_mut(entity_id) {
                    let actor = entity
                        .actor_data_mut()
                        .expect("RiderCharging owner must remain an actor");
                    actor.clear_path();
                    actor.active_movement.clear();
                    actor.active_rider_charge = None;
                    entity.position_iface_mut().reset_box_blocked();
                }
            }
            return;
        }
        let ft = prepass.final_tolerance;
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
        let live_seek_target_ground = ft
            .target_id
            .and_then(|target_id| self.world.entities.get(target_id))
            .map(|target| target.ground_position());
        // PerformSeek owns mpSeekTarget on the actor independently of the
        // copied movement element (`RHelementactor.cpp`). A terminating
        // transition can therefore have no FinalTol target while the
        // actor still owns the entity interaction. Keep this snapshot
        // separate: genuine point seeks have no actor-owned target.
        let actor_seek_flags = self
            .orders
            .sequence_manager
            .get_element(selected.seq_id, selected.elem_idx)
            .and_then(|element| match &element.data {
                crate::sequence::SequenceElementData::Movement { flags, .. } => Some(*flags),
                _ => None,
            })
            .unwrap_or_else(|| {
                panic!(
                    "selected PerformSeek owner {entity_id:?} lost movement flags for sequence {:?} element {}",
                    selected.seq_id, selected.elem_idx
                )
            });
        let live_actor_seek_target = self.world.entities.get(entity_id).and_then(|entity| {
            let actor = entity.actor_data()?;
            let target = self.world.entities.get(actor.seek_target?)?;
            let target_position = target.element_data().position_map();
            let sampled_target = actor_seek_flags
                .contains(crate::sequence::MoveFlags::USE_POINT)
                .then(|| target.cxx_current_point_map())
                .flatten()
                .filter(|point| *point != target_position)
                .unwrap_or(target_position);
            let delta = sampled_target - entity.element_data().position_map();
            let stretched_y =
                if actor_seek_flags.contains(crate::sequence::MoveFlags::DIRECTIONAL_TOLERANCE) {
                    delta.y * 1.743_446_8
                } else {
                    delta.y
                };
            let target_unchanged_or_in_tolerance = target_position
                == actor.last_seek_target_position
                || delta.x * delta.x + stretched_y * stretched_y
                    < actor.seek_distance * actor.seek_distance * 1.1025;
            Some((
                target_position,
                target.ground_position(),
                target.element_data().sector(),
                target_unchanged_or_in_tolerance,
            ))
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
                    .expect("SEEK FinalTol must have shield_destination or a live target position");
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
        let provenance_frame = self.control.frame_counter;
        let diagnostic_creation_order =
            crate::sprite::sprite_row_diagnostic_creation_order(provenance_frame, || {
                self.world.original_creation_order(entity_id)
            });
        let sprite_row_diagnostic = diagnostic_creation_order.is_some();
        let entity = self
            .world
            .entities
            .get_mut(entity_id)
            .expect("movement actor ID collected from entity table must remain present");
        super::animation::direction_provenance_snapshot(
            entity.position_iface(),
            entity_id,
            provenance_frame,
            "movement_execute_entry",
        );
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
            order_antagonist,
            transition_distance_continuation,
            deferred_movement_state_start,
            next_destination_same_action,
            legacy_serialized_order_chain,
        ) = {
            let actor = match entity.actor_data_mut() {
                Some(a) => a,
                None => return,
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
                    return;
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
                return;
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
                return;
            }
            let Some(order) = self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .and_then(|e| e.current_order())
            else {
                return;
            };
            let goal = MapPoint::new(order.target_x, order.target_y);
            let order_id = Some(order.order_id);
            let order_action = order.order_type;
            let order_tolerance = order.tolerance;
            let order_compute_direction = order.compute_direction;
            let order_reverse = order.reverse;
            let order_antagonist = order.antagonist;
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
                    crate::sequence::SequenceElementData::Movement { flags, .. } => Some(*flags),
                    _ => None,
                })
                .unwrap_or(crate::sequence::MoveFlags::empty());
            let legacy_serialized_order_chain = self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .is_some_and(|element| element.legacy_v48.is_some());

            // A materialized walk/run successor can sit behind a
            // MakeFast/MakeSlow transition in the sequence-manager queue.
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
                order_antagonist,
                transition_distance_continuation,
                deferred_movement_state_start,
                next_destination_same_action,
                legacy_serialized_order_chain,
            )
        };
        let terminal_pc_external_direction_goal = if is_pc
            && is_final_waypoint
            && matches!(
                order_action,
                OrderType::TransitionWalkingUprightWaitingUpright
                    | OrderType::TransitionRunningUprightWaitingUpright
                    | OrderType::TransitionWalkingCrouchedWaitingCrouched
            )
            && order_compute_direction
            // A new movement order owns the goal unconditionally:
            // Original PerformMotion initializes it with
            // ComputeIncrementAll before any terminal cleanup can
            // observe an external orientation.  Only an already-running
            // order can have been reoriented between Execute calls.
            && order_id.is_some_and(|order_id| {
                entity.element_data().sprite.last_processed_order_id == order_id.get()
            }) {
            let pi = entity.position_iface();
            if !pi.is_increment_all_computed() {
                None
            } else {
                let increment = pi.get_increment();
                let mut movement_direction = vector_to_sector_0_to_15(increment.x, increment.y);
                if order_reverse {
                    movement_direction ^= 8;
                }
                let live_direction_goal = i16::from(pi.get_direction_goal());
                (live_direction_goal != movement_direction)
                    .then_some((live_direction_goal, movement_direction))
            }
        } else {
            None
        };

        if order_action == OrderType::Freezing {
            // `MOVE_WAITING` carries a temporary FREEZING order while
            // the pathfinder owns the request.  The original
            // RHElementActor::Execute arm returns IN_PROGRESS without
            // touching the sprite; this token has no destination-backed
            // motion state to initialize or validate.
            entity.element_data_mut().sprite.last_motion_state =
                non_sprite_movement_motion(order_action);
            return;
        }

        if order_action == OrderType::PassingDoor {
            // Actor::Execute returns TERMINATED directly after the door
            // callback; no Sprite method runs for this action point.
            entity.element_data_mut().sprite.last_motion_state =
                non_sprite_movement_motion(order_action);
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
                    deferred.door_triggers.push((
                        eid,
                        crate::gate::DoorIndex::from(door.0),
                        direct,
                        0,
                    ));
                    entity.position_iface_mut().clear_door();
                }
                self.orders.messenger.send(crate::messenger::Message::new(
                    crate::messenger::MessageType::Simple(crate::messenger::SimpleMessage::Stature),
                ));
                deferred.order_pops.push((move_seq_id, move_elem_idx));
                return;
            }
            let actor = entity
                .actor_data_mut()
                .expect("door-pass action point owner is not an actor");
            let dp = actor.active_door_pass.as_mut().unwrap_or_else(|| {
                panic!("door-pass action point {order_action:?} for {eid:?} has no active pass")
            });
            let trigger_num = dp.triggers_fired;
            dp.triggers_fired += 1;
            deferred
                .door_triggers
                .push((eid, dp.door_index, dp.direct, trigger_num));

            let advance = Self::advance_door_pass(
                actor,
                eid,
                goal,
                &mut deferred.door_triggers,
                &mut deferred.select_triggers,
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
                    let mut order =
                        crate::order::Order::new(action, destination.x, destination.y, order_id);
                    order.reverse = reverse;
                    order.compute_direction = compute_direction;
                    order.tolerance = tolerance;
                    deferred
                        .transition_pushes
                        .push((move_seq_id, move_elem_idx, order));
                }
                DoorPassAdvance::Paused { transition_order } => {
                    deferred
                        .transition_pushes
                        .push((move_seq_id, move_elem_idx, transition_order));
                }
                DoorPassAdvance::ActionPoint { order } => {
                    deferred
                        .transition_pushes
                        .push((move_seq_id, move_elem_idx, order));
                }
                DoorPassAdvance::Done { completed } => {
                    if let Some((door_index, direct)) = completed {
                        deferred
                            .completed_door_passes
                            .push((eid, door_index, direct));
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
            deferred.order_pops.push((move_seq_id, move_elem_idx));
            return;
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
        let combat_target = prepass.combat_face_target;
        let is_sword_motion = is_sword_motion_context(action_state, door_pass_anim, order_action);
        let executes_sword_movement = executes_sword_movement_action(door_pass_anim, order_action);
        let is_shield_motion = matches!(action_state, crate::element::ActionState::MovingShield);
        let is_combat = (is_shield_motion && combat_target.is_some()) || is_sword_motion;
        if is_combat {
            // Face opponent instead of movement direction.  Use
            // `set_direction_goal` + per-frame `turn()` rather
            // than instantly snapping facing, so the facing
            // rotates one step per frame toward the opponent.
            if let Some(opp_pos) = combat_target {
                let face_origin = if prepass.combat_face_target_is_ground {
                    let position = elem.position();
                    crate::coordinates::MapPoint::new(position.x, position.y)
                } else {
                    elem.position_map()
                };
                let fdx = opp_pos.x - face_origin.x;
                let fdy = opp_pos.y - face_origin.y;
                tracing::trace!(
                    entity = ?entity_id,
                    frame = self.control.frame_counter,
                    origin_x = face_origin.x,
                    origin_y = face_origin.y,
                    target_x = opp_pos.x,
                    target_y = opp_pos.y,
                    sector = crate::position_interface::vector_to_sector_0_to_15_iso(fdx, fdy),
                    "combat facing target"
                );
                super::animation::direction_provenance_snapshot(
                    &elem.sprite.position_iface,
                    entity_id,
                    provenance_frame,
                    "writer:combat_face_goal:before",
                );
                elem.set_direction_goal(crate::position_interface::vector_to_sector_0_to_15_iso(
                    fdx, fdy,
                ));
                super::animation::direction_provenance_snapshot(
                    &elem.sprite.position_iface,
                    entity_id,
                    provenance_frame,
                    "writer:combat_face_goal:after",
                );
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
            door_pass_sprite_animation_override(order_action, door_pass_anim)
                .filter(|anim| !is_sword_movement_nonanimation(*anim))
        {
            // PassDoor supplies the current translated movement step, but
            // Soldier::Execute still dispatches that logical action
            // through its attentive-animation override. In particular,
            // an attentive WalkingUpright door step plays
            // WalkingAlerted and therefore uses its distinct frame
            // distances.
            super::animation::soldier_movement_animation(dp_anim, soldier_attentive, action_state)
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
                // The facing vector is the one FaceOpponent measures
                // against, so keep it as a vector: reducing it to an angle
                // first would lose the degenerate cases the Original
                // resolves through its determinant test.
                let facing = if let Some(opp_pos) = combat_target {
                    let face_origin = if prepass.combat_face_target_is_ground {
                        let position = elem.position();
                        crate::coordinates::MapPoint::new(position.x, position.y)
                    } else {
                        elem.position_map()
                    };
                    let fdx = opp_pos.x - face_origin.x;
                    let fdy = opp_pos.y - face_origin.y;
                    // Preserve FaceOpponent's literal vector, including
                    // the zero vector produced by co-located fighters.
                    // SBGeoVector2D::Angle resolves dot == det == 0 to PI,
                    // selecting the backwards-sword animation. Replacing
                    // it with the current heading selects a strafe row.
                    (fdx, fdy)
                } else {
                    let heading = (elem.direction() as f32) * std::f32::consts::PI / 8.0;
                    (heading.cos(), heading.sin())
                };
                let angle = combat_movement_angle((dx, dy), facing);
                // MovingSword and MovingFastSword both use the
                // directional walking/strafing sword animations — the
                // `fast` flag is ignored when selecting the animation.
                // Running in combat is implemented by playing the walking
                // animation under `MotionMethod::Fast`.
                let sword_anim = combat_directional_animation(action_state, angle);
                tracing::trace!(
                    target: "parity_face_opponent",
                    ?entity_id,
                    goal_x = goal.x,
                    goal_y = goal.y,
                    here_x = elem.position_map().x,
                    here_y = elem.position_map().y,
                    ?combat_target,
                    ground_origin = prepass.combat_face_target_is_ground,
                    facing_x = facing.0,
                    facing_y = facing.1,
                    angle,
                    ?sword_anim,
                    "FaceOpponent combat row selection",
                );
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
            let base =
                literal_lift_sprite_action(order_action).unwrap_or_else(|| match order_action {
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
                    | OrderType::ClimbingLadderDownFast
                    | OrderType::WalkingWithCorpse
                    | OrderType::WalkingCarryingOnShoulders => order_action,
                    _ => match action_state {
                        crate::element::ActionState::MovingFast => OrderType::RunningUpright,
                        _ => OrderType::WalkingUpright,
                    },
                });
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
            // vector. Snapshotted in `lift_translation` so we don't have
            // to re-borrow the grid or door table mid-loop.
            let base =
                if !order_uses_distance_motion(order_action) || is_authored_climb_action(base) {
                    // DetermineMovementAnimation rewrites the movement
                    // element once when it is instructed. Every path order
                    // retains that authored climb direction, even if a later
                    // waypoint briefly bends the other way.
                    base
                } else {
                    match prepass.lift_translation {
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
                };
            // RHElementActorSoldier::Execute receives the action after
            // DetermineMovementAnimation has translated it for the lift,
            // then substitutes the attentive sprite animation. The order
            // matters for stairs: translating WalkingStairsAlerted again
            // would collapse it back to ordinary WalkingStairs.
            super::animation::soldier_movement_animation(base, soldier_attentive, action_state)
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
            let ft = prepass.final_tolerance;
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
                speed_factor = if dist_sq < 25.0 {
                    1.0
                } else if dist_sq < 100.0 {
                    1.5
                } else {
                    2.0
                };
            }
        }
        let speed_factor = speed_factor;
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
        let fast_climb_motion = is_fast_climb_action(order_action) || is_fast_climb_action(anim);
        let fast_climb_stops_after_first_termination =
            fast_climb_stops_after_first_termination(order_action)
                || fast_climb_stops_after_first_termination(anim);
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
        }) = prepass.lift_translation
            && initialising_climb_uses_lift_direction(anim, lift_type, execute_order_initialising)
        {
            super::animation::direction_provenance_snapshot(
                &elem.sprite.position_iface,
                entity_id,
                provenance_frame,
                "writer:initial_climb_lift_goal:before",
            );
            elem.set_direction_goal(lift_direction);
            super::animation::direction_provenance_snapshot(
                &elem.sprite.position_iface,
                entity_id,
                provenance_frame,
                "writer:initial_climb_lift_goal:after",
            );
        }
        if let Some(posture) = door_pass_eager_posture(
            anim,
            door_pass_anim.is_some(),
            execute_order_initialising,
            prepass.decorative_building_trap_at_destination,
        ) {
            elem.posture = posture;
        }
        if execute_order_initialising && let Some(climb_dir) = prepass.door_pass_climb_direction {
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
            super::animation::direction_provenance_snapshot(
                &elem.sprite.position_iface,
                entity_id,
                provenance_frame,
                "writer:initial_door_climb_goal:before",
            );
            elem.set_direction_goal(dir);
            super::animation::direction_provenance_snapshot(
                &elem.sprite.position_iface,
                entity_id,
                provenance_frame,
                "writer:initial_door_climb_goal:after",
            );
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
            target_element: order_antagonist,
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
        // Human::FaceOpponent / FaceDangerPoint calls Turn before
        // PerformSeek. When the seek continues, PerformSeek calls Turn a
        // second time immediately before PerformMotion; when tolerance
        // has already been reached, it returns after only this first
        // turn. A non-soldier without a live opponent returns from
        // FaceOpponent before setting a direction or turning.
        if is_combat
            && combat_target.is_some()
            && active_move_flags.contains(crate::sequence::MoveFlags::SEEK)
        {
            super::animation::direction_provenance_snapshot(
                &sprite.position_iface,
                entity_id,
                provenance_frame,
                "turn:combat_face:before",
            );
            let _ = sprite.position_iface.turn();
            super::animation::direction_provenance_snapshot(
                &sprite.position_iface,
                entity_id,
                provenance_frame,
                "turn:combat_face:after",
            );
        }
        // Human's sword-movement Execute arm
        // (`RHelementactorhuman.cpp:3631`) is the one movement arm that has no
        // `Turn()` of its own: its non-SEEK branch goes straight to
        // `mpSprite->PerformMotion` (`RHelementactorhuman.cpp:3660`), and its
        // only pre-motion rotation is the one inside `FaceOpponent`
        // (`RHelementactorhuman.cpp:7513`). `FaceOpponent` returns at
        // `RHelementactorhuman.cpp:7505` — before `SetDirection` and before
        // `Turn` — when a non-soldier is no longer swordfighting, which is
        // exactly the `combat_target.is_none()` case resolved above. A
        // non-interruptible order such as PASS_DOOR keeps that arm selected
        // after `QuitSwordfightWithFarOpponents` has emptied the opponent
        // list, so the Original then rotates the actor not at all while its
        // direction goal stays where the door route last left it. Every other
        // arm reaching the block below does turn: `Actor::Execute`'s ordinary
        // movement arms call `Turn()` explicitly in their non-SEEK branch
        // (`RHelementactor.cpp:2687`), `PerformSeek` calls it in both of its
        // own branches (`RHelementactor.cpp:7805`, `:7925`), and
        // `FaceDangerPoint` always turns (`RHelementactorpc.cpp:8914`).
        let sword_arm_without_face_turn = executes_sword_movement
            && combat_target.is_none()
            && !active_move_flags.contains(crate::sequence::MoveFlags::SEEK);
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
            if !sword_arm_without_face_turn {
                super::animation::direction_provenance_snapshot(
                    &sprite.position_iface,
                    entity_id,
                    provenance_frame,
                    "turn:perform_seek:before",
                );
                let _ = sprite.position_iface.turn();
                super::animation::direction_provenance_snapshot(
                    &sprite.position_iface,
                    entity_id,
                    provenance_frame,
                    "turn:perform_seek:after",
                );
            }
            let diagnostic_pre = sprite_row_diagnostic.then(|| sprite.sprite_row_diagnostic_pre());
            let played_direction = u16::from(sprite.position_iface.get_direction().as_u8());
            let result = sprite.perform_motion(
                sim,
                motion_order,
                sprite_motion_order_for_nonanimation(anim),
                played_direction,
                FrameProgression::Default,
                false,
                motion_method,
                dest_already_at_pos,
            );
            if let Some(pre) = diagnostic_pre {
                sprite.emit_sprite_row_diagnostic(
                    "perform_motion",
                    provenance_frame,
                    diagnostic_creation_order.expect("enabled diagnostic has owner"),
                    entity_id.index(),
                    order_action,
                    sprite_motion_order_for_nonanimation(anim),
                    played_direction,
                    FrameProgression::Default,
                    pre,
                    result.0,
                );
            }
            super::animation::direction_provenance_snapshot(
                &sprite.position_iface,
                entity_id,
                provenance_frame,
                "perform_motion:return",
            );
            // A generated walking- or running-start transition can begin
            // exactly where an anti-collision deviation ended (destination
            // == current map position). The shipped Linux game drops the
            // deviation latch on that zero-distance START tick, so every
            // following in-place `Turn()` rotates immediately in *both*
            // directions — a counter-clockwise first-call rotation from a +2
            // count (Savegame_010 replay-014 frame 1030) rules out the
            // previous count-priming model from task #545. Savegame_032
            // replay-010 additionally proves the running-start case: the
            // visible turn history establishes a -2 count immediately before
            // the aligned start, then its next clockwise shield turn rotates
            // on the first call. The available C++ source does not expose the
            // latch update responsible for this save-observable detail; see
            // `clear_deviated_for_aligned_transition_start` for the trace
            // evidence bounding it to exactly this startup initialization.
            // The matching walking-to-waiting exit deliberately preserves
            // the latch (Savegame_023 replay-027, Soldier 136, frame 25195).
            if should_clear_deviated_for_aligned_transition_start(
                is_pc,
                execute_order_initialising,
                is_transition_anim,
                order_action,
                sprite.position_iface.is_deviated(),
                sprite.position_iface.map_position(),
                goal,
            ) {
                sprite
                    .position_iface
                    .clear_deviated_for_aligned_transition_start();
            }
            result
        };
        if tolerance_arrival {
            // This PerformSeek branch returns before calling any Sprite
            // method. Preserve the wrapper's authoritative Execute result
            // for Actor::Hourglass just as the non-sprite movement arms
            // above do. Leaving the prior sprite DONE latched causes the
            // successful StartPostSeekSequence termination to be hidden as
            // IN_PROGRESS by the generic entity-seek projection.
            sprite.last_motion_state = Some(motion_state);
        }
        let first_frame_dist_raw = frame_dist_raw;
        let first_direction_differs_from_goal =
            sprite.position_iface.get_direction() != sprite.position_iface.get_direction_goal();
        let fast_motion_outer_pre = sprite.position_iface.map_position();
        let mut first_fast_commit = None;
        let mut second_fast_operands = None;
        // Fast ladder/wall Execute contains two literal PerformMotion
        // calls, but returns immediately when the first one reaches the
        // order goal. RunningStairs has the same two-call loop without
        // that early return, so its terminal tick still advances the
        // sprite in the second call.
        // Project that first call through the same anti-collision query
        // used by the committed movement below.  Deferring all position
        // work until after both sprite calls otherwise advances the
        // animation counter once too often on a terminal first call; the
        // next climb order can then move one simulation frame early.
        let first_fast_call_terminates = if !tolerance_arrival
            && fast_climb_stops_after_first_termination
            && motion_state != MotionState::Terminated
        {
            let first_speed = scaled_motion_distance(
                first_frame_dist_raw,
                speed_factor,
                apply_speed_factor,
                first_direction_differs_from_goal,
            );
            projected_step_reaches_goal(
                &sprite.position_iface,
                anti_snapshots.get(actor_id).and_then(|slot| slot.as_ref()),
                anti_snapshots.as_slice(),
                &self.ai.global.repulsive_points,
                &prepared,
                &self.world.fast_grid,
                goal,
                prepass.goal_target_info,
                first_speed,
            )
        } else {
            false
        };
        let mut second_frame_dist_raw = None;
        if !tolerance_arrival
            && fast_climb_motion
            && motion_state != MotionState::Terminated
            && !first_fast_call_terminates
        {
            let first_speed = scaled_motion_distance(
                first_frame_dist_raw,
                speed_factor,
                apply_speed_factor,
                first_direction_differs_from_goal,
            );
            if first_speed != 0.0 {
                let first_pre = sprite.position_iface.map_position();
                let first_increment = sprite.position_iface.get_increment_map();
                let anti_on = sprite.position_iface.is_anti_collision_on();
                let (first_dx, first_dy, recovered, rebuild) = if anti_on
                    && let Some(mover_snapshot) = anti_snapshots
                        .get(actor_id)
                        .and_then(|slot| slot.as_ref())
                        .filter(|snapshot| snapshot.active)
                        .cloned()
                {
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
                        &mover_snapshot,
                        anti_snapshots,
                        &self.ai.global.repulsive_points,
                        prepared,
                        &self.world.fast_grid,
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
                    sprite.position_iface.set_direction(
                        crate::position_interface::Direction::from_raw(i32::from(
                            if order_reverse { raw ^ 8 } else { raw },
                        )),
                    );
                    sprite.position_iface.reset_increment_computed();
                    sprite.position_iface.compute_increment_all(false);
                } else if recovered {
                    sprite.position_iface.reset_increment_computed();
                    sprite.position_iface.compute_increment_all(true);
                }
                // RHNONANIMATION_RUNNING_STAIRS is the one double-motion
                // Execute arm which deliberately continues after its first
                // PerformMotion returns TERMINATED.  That first call still
                // owns the complete ordinary arrival branch: IsGoalReached,
                // followed by the zero-tolerance goal snap.  The second
                // Turn/PerformMotion therefore observes the snapped position,
                // rather than both raw displacements being committed before a
                // single aggregate arrival check.
                let first_post = if order_action == OrderType::RunningStairs
                    && sprite
                        .position_iface
                        .is_goal_reached(&self.world.fast_grid, prepass.goal_target_info)
                    && order_tolerance == 0.0
                    && !sprite.position_iface.is_deviated()
                {
                    sprite.position_iface.set_map_position(goal);
                    goal
                } else {
                    first_raw_post
                };
                if let Some(snapshot) = anti_snapshots
                    .get_mut(actor_id)
                    .and_then(|slot| slot.as_mut())
                {
                    sync_snapshot_after_committed_step(snapshot, first_pre, first_post);
                }
                first_fast_commit = Some((first_pre, first_increment, first_speed, first_post));
            }
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
            second_fast_operands = Some((
                sprite.position_iface.map_position(),
                sprite.position_iface.get_increment_map(),
            ));
        }
        // PerformMotion refreshes RHPositionInterface::mpTargetElement
        // when a new order is initialized. Anti-collision follows that
        // call in the same actor slot in Original, so the mover snapshot
        // must observe the newly installed order's antagonist now rather
        // than the target retained from the preceding order at the
        // top-of-tick snapshot boundary.
        if let Some(snapshot) = anti_snapshots
            .get_mut(actor_id)
            .and_then(|slot| slot.as_mut())
        {
            snapshot.target_element = sprite.position_iface.target_element();
        }
        deferred.executed_sword_movement = is_sword_motion;
        if is_pc {
            deferred
                .executed_pc_movement_actions
                .push((entity_id, order_action));
        }
        if door_pass_anim.is_some()
            && matches!(motion_state, MotionState::Start)
            && matches!(
                anim,
                OrderType::TransitionClimbingLadderUpWaitingCrouched
                    | OrderType::TransitionClimbingLadderUpWaitingUprightAlerted
            )
        {
            deferred.door_pass_transition_start_effects.push(entity_id);
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
            deferred.door_pass_transition_done_effects.push(entity_id);
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
                deferred.galopp_event = true;
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
        let (speed, split_motion_speeds) = if let Some(second_distance) = second_frame_dist_raw {
            // The fast stairs/ladder/wall arms contain two literal
            // PerformMotion calls. Each call applies its own turning
            // slowdown using the direction reached by the immediately
            // preceding Turn(), so a first call that is still rotating
            // must not inherit the second call's newly aligned state.
            let first_speed = scaled_motion_distance(
                first_frame_dist_raw,
                speed_factor,
                apply_speed_factor,
                first_direction_differs_from_goal,
            );
            let second_speed = scaled_motion_distance(
                second_distance,
                speed_factor,
                apply_speed_factor,
                direction_differs_from_goal,
            );
            (
                if first_fast_commit.is_some() {
                    second_speed
                } else {
                    first_speed + second_speed
                },
                Some((first_speed, second_speed)),
            )
        } else {
            (
                scaled_motion_distance(
                    frame_dist_raw,
                    speed_factor,
                    apply_speed_factor,
                    direction_differs_from_goal,
                ),
                None,
            )
        };
        let mut discarded_lazy_door_followers = false;
        // PerformMotion applies the distance before returning its motion
        // state. A fresh walking order that reaches its goal on that same
        // invocation returns TERMINATED, not START, so the walking
        // Execute arm does not enter the Moving action state. Our
        // position update is staged below; fold that imminent arrival
        // into the state-effect result now.
        let entity_target_seek =
            active_move_flags.contains(crate::sequence::MoveFlags::SEEK) && ft.target_id.is_some();
        // The ordinary (non-TillLastFrame) arrival branch runs only when
        // the sprite actually advanced the actor, and it asks the position
        // interface rather than comparing straight-line distances. A
        // walker that sidesteps a neighbour covers more ground than
        // remains to its goal and still ends the frame short of it.
        let reaches_goal_this_step = !is_transition_anim
            && projected_step_reaches_goal(
                &sprite.position_iface,
                anti_snapshots.get(actor_id).and_then(|slot| slot.as_ref()),
                anti_snapshots.as_slice(),
                &self.ai.global.repulsive_points,
                &prepared,
                &self.world.fast_grid,
                goal,
                prepass.goal_target_info,
                speed,
            );
        let state_effect_motion = movement_execute_visible_motion(
            order_action,
            motion_state,
            reaches_goal_this_step,
            entity_target_seek,
        );
        deferred.post_completion_motion_override = committed_arrival_post_completion_override(
            motion_state,
            state_effect_motion,
            reaches_goal_this_step,
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
        if matches!(state_effect_motion, MotionState::Start) && executes_sword_movement {
            deferred.sword_movement_starts.push(entity_id);
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
            reaches_goal_this_step,
            order_tolerance,
            deviated = sprite.position_iface.is_deviated(),
            anti_collision = sprite.position_iface.is_anti_collision_on(),
            goal_x = goal.x,
            goal_y = goal.y,
            increment_x = sprite.position_iface.get_increment_map().x,
            increment_y = sprite.position_iface.get_increment_map().y,
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
            deferred
                .movement_state_effects
                .push((entity_id, posture, action_state));
        }
        if is_transition_anim
            && tolerance_arrival
            && let Some((posture, action_state)) =
                movement_execute_state_effect(order_action, state_effect_motion)
        {
            if ft.launches_post_seek {
                // StartPostSeekSequence runs synchronously inside
                // PerformSeek. Its interaction callbacks must therefore see
                // the pre-transition action state. The surrounding transition
                // Execute switch applies TERMINATED only after that recursive
                // work returns.
                deferred
                    .post_seek_terminal_state_effects
                    .push((entity_id, posture, action_state));
            } else if ft.has_post_seek {
                deferred.sequence_seek_terminal_state_effects.push((
                    entity_id,
                    posture,
                    action_state,
                ));
            } else {
                deferred
                    .movement_state_effects
                    .push((entity_id, posture, action_state));
            }
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
            let transition_recomputes_exact_position = motion_recomputes_exact_position(
                is_transition_anim,
                transition_has_map_target,
                speed,
                dist,
            );
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
                // The fast stairs/ladder/wall tokens invoke PerformMotion
                // twice. With anti-collision disabled, Original stores the
                // first position update before applying the second one.
                // Combining both distances and rounding only the final
                // sum moves large map coordinates by an ULP and can
                // amplify into a visible elevation error on steep planes.
                let split_motion_target =
                    split_motion_speeds
                        .filter(|_| !anti_on)
                        .map(|(first_speed, second_speed)| {
                            let mut target = entity.element_data().position_map();
                            target.x += nx * first_speed;
                            target.y += ny * first_speed;
                            target.x += nx * second_speed;
                            target.y += ny * second_speed;
                            target
                        });
                let goal_map = crate::coordinates::MapPoint::new(goal.x, goal.y);
                let (move_box, half_diagonal) = {
                    let pi = entity.position_iface();
                    (*pi.get_move_box(), pi.get_half_diagonal())
                };
                let (dx_step, dy_step, deviated, recovered_from_deviation) =
                    if let Some(mover_snap) = anti_snapshots
                        .get(actor_id)
                        .and_then(|slot| slot.as_ref())
                        .filter(|snapshot| snapshot.active)
                    {
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
                            mover_snap,
                            anti_snapshots,
                            &self.ai.global.repulsive_points,
                            prepared,
                            &self.world.fast_grid,
                            &mut state,
                            nx,
                            ny,
                            speed,
                            anti_on,
                        );
                        (
                            dx_step,
                            dy_step,
                            // Only a committed deviation (blocked counter
                            // reset) faces along its step and rebuilds the
                            // increment here; a break-through barge keeps
                            // the facing and cached increment the
                            // anti-collision step left behind.
                            state.pi.is_deviated() && state.pi.blocked_count == 0,
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
                let position = split_motion_target.unwrap_or_else(|| {
                    let mut position = elem.position_map();
                    position.x += dx_step;
                    position.y += dy_step;
                    position
                });
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
            } else if transition_recomputes_exact_position {
                // PerformMotion gates its position update on animation
                // distance, not on the length of the normalized map
                // increment. With an exact-position transition target a
                // nonzero sprite-frame distance therefore still reaches
                // UpdatePositionAntiCollision with a zero increment. That
                // call is observable even though it cannot displace the
                // actor: its empty-candidate recovery drops a preceding
                // deviation latch before ComputePositionAll. Skipping the
                // call left a stopping soldier in TurnAntiVibration on the
                // following frame (Linux Savegame_036 replay-015, Soldier
                // 144), delaying the visible counter-clockwise turn.
                let recovered_from_deviation = if entity.position_iface().is_anti_collision_on()
                    && let Some(mover_snap) = anti_snapshots
                        .get(actor_id)
                        .and_then(|slot| slot.as_ref())
                        .filter(|snapshot| snapshot.active)
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
                    let step = apply_prepared_anti_collision_step(
                        provenance_frame,
                        mover_snap,
                        anti_snapshots,
                        &self.ai.global.repulsive_points,
                        prepared,
                        &self.world.fast_grid,
                        &mut state,
                        0.0,
                        0.0,
                        speed,
                        true,
                    );
                    debug_assert_eq!(step, (0.0, 0.0));
                    was_deviated && !state.pi.is_deviated()
                } else {
                    false
                };
                let position = entity.element_data().position_map();
                let elem = entity.element_data_mut();
                elem.set_position_map(position);
                if recovered_from_deviation {
                    elem.sprite.position_iface.reset_increment_computed();
                    elem.sprite.position_iface.compute_increment_all(true);
                }
                elem.update_grid_cell();
                // The same nonzero-animation-distance block ends with
                // UpdateForecastedMovement even though the cached
                // increment is zero at the goal. This clears a preceding
                // running forecast before projectile leading samples it.
                refresh_motion_forecast(entity.sprite_mut(), speed, split_motion_speeds);
            }
            if transition_has_distance {
                // Original's shared PerformMotion path refreshes target
                // leading after every committed transition displacement,
                // before IsGoalReached can clear the live increment. A
                // missing refresh here made arrows aim at the target's
                // current point during start/stop transitions.
                refresh_motion_forecast(entity.sprite_mut(), speed, split_motion_speeds);
            }
            // TILL_LAST_FRAME still performs the ordinary arrival check
            // after every nonzero transition step. Reaching the target
            // zeros both increments and snaps an undeviated zero-tolerance
            // actor, but the transition keeps playing until its animation
            // loops unless the next order uses the same animation.
            let transition_goal_reached = entity
                .position_iface()
                .is_goal_reached(&self.world.fast_grid, prepass.goal_target_info);
            let transition_increment_nonzero = {
                let increment = entity.position_iface().get_increment_map();
                increment.x != 0.0 || increment.y != 0.0
            };
            if transition_goal_reached && speed != 0.0 && transition_increment_nonzero {
                let should_snap = !entity.position_iface().is_deviated() && order_tolerance == 0.0;
                entity.position_iface_mut().zero_all_increments();
                tracing::trace!(
                    ?entity_id,
                    ?anim,
                    ?goal,
                    should_snap,
                    from = ?entity.element_data().position_map(),
                    "transition goal reached"
                );
                if should_snap {
                    entity.element_data_mut().set_position_map(goal);
                }
                if next_destination_same_action.is_some() {
                    motion_state = MotionState::Terminated;
                }
            }
            // Actor::Hourglass runs CheckForLineCrossing after Execute
            // returns, so the segment endpoint is resolved from the live
            // position at dispatch time. A TillLastFrame step may
            // overshoot and snap back to its goal; the discarded
            // overshoot must not trigger a boundary.
            if let Some((old_pos, layer, eligible)) = transition_crossing_start
                && eligible
            {
                deferred.line_cross_checks.push((entity_id, old_pos, layer));
                deferred
                    .non_elevation_cross_checks
                    .push((entity_id, old_pos, layer));
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
                deferred
                    .movement_state_effects
                    .push((entity_id, posture, next_action_state));
            }
            let door_transition_state_effect_due = matches!(motion_state, MotionState::Terminated)
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
                        | OrderType::TransitionWaitingUprightClimbingLadderDownAlerted
                        | OrderType::TransitionClimbingLadderDownWaitingUpright
                        | OrderType::TransitionClimbingLadderDownWaitingUprightAlerted
                        | OrderType::TransitionClimbingWallUpWaitingCrouched
                        | OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel
                        | OrderType::TransitionWaitingCrouchedClimbingWallDown
                        | OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel
                        | OrderType::TransitionClimbingWallDownWaitingUpright
                )
            {
                deferred
                    .door_pass_transition_completion_effects
                    .push((entity_id, order_action));
            }
            if matches!(motion_state, MotionState::Terminated) {
                if let Some((external_direction, movement_direction)) =
                    terminal_pc_external_direction_goal
                {
                    deferred.terminal_pc_direction_goal_restores.push((
                        entity_id,
                        external_direction,
                        movement_direction,
                    ));
                }
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
                                } if *destination != MapPoint::ZERO && *action != order_action => {
                                    Some(*action)
                                }
                                _ => None,
                            })
                        });
                    let next_order_id = &mut self.orders.next_order_id;
                    let mut continuation_door_action = None;
                    let mut discard_lazy_door_followers = false;
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
                            lazy_next_animation.map(|animation| (element.orders.len(), animation))
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
                            if should_defer_pc_movement_state_start(is_pc, entity_target_seek)
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
                            discard_lazy_door_followers = true;
                        }
                    }
                    if discard_lazy_door_followers {
                        discard_lazy_door_pass_following_orders(
                            entity
                                .actor_data_mut()
                                .and_then(|actor| actor.active_door_pass.as_mut()),
                        );
                        discarded_lazy_door_followers = true;
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
                // PerformSeek wraps the transition animation too. When
                // the last stop transition terminates, Original checks
                // the live target before retiring the movement: an
                // unchanged same-sector target completes the seek and
                // starts its actor-owned post-seek interaction.
                //
                // The ordinary walking-arrival path below performs this
                // same check, but transition animations return through
                // this earlier branch and must close the handoff here.
                // PerformMotion(TILL_LAST_FRAME) may have deleted every
                // same-animation follower above after looping short of the
                // current destination. Original PerformSeek asks
                // GetNextOrder() only after that synchronous cleanup, so
                // the just-truncated current order is now the final
                // waypoint even when it was not final at Execute entry.
                let is_final_waypoint_after_transition_cleanup = self
                    .orders
                    .sequence_manager
                    .get_element(move_seq_id, move_elem_idx)
                    .is_none_or(|element| element.orders.len() <= 1);
                let movement_is_last_sequence_element = self
                    .orders
                    .sequence_manager
                    .get_sequence(move_seq_id)
                    .map(|sequence| move_elem_idx + 1 >= sequence.elements.len())
                    .unwrap_or(false);
                let final_entity_seek_arrival = if is_final_waypoint_after_transition_cleanup
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
                    // PerformSeek reports this frame as still in progress
                    // once it decides to refresh, so the Execute arm never
                    // reaches the switch that would retire the actor into
                    // its waiting state. Drop the effect the terminating
                    // transition queued a moment ago; leaving it applied
                    // strands the actor at a standstill, and the refresh
                    // then reads that as a walk rather than the run it was
                    // already doing.
                    deferred
                        .movement_state_effects
                        .retain(|(id, _, _)| *id != eid);
                    deferred
                        .transition_seek_refreshes
                        .push((eid, move_seq_id, move_elem_idx));
                    return;
                }
                // PerformMotion(TILL_LAST_FRAME) can mutate the order list
                // before returning TERMINATED: when a startup transition
                // loops short of its destination, it inserts a copied order
                // using the next distinct animation. PerformSeek then reads
                // that live successor, just like it does after an ordinary
                // walking waypoint, and rejects an out-of-reach stop
                // transition when the seek target moved in the meantime
                // (RHelementactor.cpp:7974-8007, RHsprite.cpp:1849-1925).
                if !is_final_waypoint_after_transition_cleanup
                    && let Some((target_position, _, target_point)) = live_seek_target
                    && target_position != ft.last_seek_target_position
                    && let Some(next_action) = self
                        .orders
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
                    let reach = (f32::from(entity.sprite().distance_for_animation(next_action))
                        + ft.tol)
                        * 1.05;
                    if dx * dx + dy * dy > reach * reach {
                        deferred
                            .movement_state_effects
                            .retain(|(id, _, _)| *id != eid);
                        deferred
                            .transition_seek_refreshes
                            .push((eid, move_seq_id, move_elem_idx));
                        tracing::trace!(
                            ?eid,
                            ?next_action,
                            reach,
                            "tick_move: looped transition exposed stale stop; refreshing seek",
                        );
                        return;
                    }
                }
                // A Hit can be attached to a Seek whose authored stop
                // transition uses up the last few map units before the
                // interaction.  Original terminates that transition at
                // this boundary, then the HITTING init guard rejects an
                // antagonist still farther than 40 map units away.  The
                // rejected post-seek never becomes the actor's visible
                // command at the frame boundary (Nescafe save controls:
                // 55.8 and 41.4 units respectively).  Rust previously
                // instructed the Hit during this same movement drain,
                // exposing one spurious HitCmd frame before its ordinary
                // next-Execute validity guard rejected it.
                let terminal_interaction = entity
                    .is_pc()
                    .then(|| {
                        actor_post_seek_interaction(entity.actor_data().expect("actor-only branch"))
                    })
                    .flatten();
                let terminal_interaction_out_of_range = final_entity_seek_arrival == Some(true)
                    // HITTING's rejected initialization is collapsed at this
                    // boundary by the controls above. TYING is different:
                    // Original publishes the newly instructed Tying order as
                    // IN_PROGRESS first, and its Execute-time position check
                    // cannot run until the actor's following Hourglass.
                    && terminal_interaction == Some(ActorPostSeekInteraction::Hit)
                    && live_seek_target
                        .map(|(target_position, _, _)| {
                            let here = entity.element_data().position_map();
                            interaction_exceeds_init_range(here, target_position)
                        })
                        .unwrap_or(false);
                if terminal_interaction_out_of_range {
                    // HITTING initialization turns toward its antagonist
                    // before the validity check which aborts it
                    // (`RHelementactorhuman.cpp:4462-4472`).
                    let target_ground = live_seek_target_ground
                        .expect("terminal entity seek retained its target ground position");
                    let here_ground = entity.ground_position();
                    let facing = vector_to_sector_0_to_15(
                        target_ground.x - here_ground.x,
                        target_ground.y - here_ground.y,
                    );
                    entity.element_data_mut().set_direction_goal(facing);

                    // PC-only: these replay controls use the PC Hit arm,
                    // whose invalid interaction has no NPC Think/AI
                    // continuation. Keep NPC post-seek lifecycle on the
                    // ordinary sequence-manager path.
                    let actor = entity.actor_data_mut().expect("actor-only branch");
                    // StartPostSeekSequence clears the seek ownership and
                    // folds its overloaded wait scalar before HITTING is
                    // instructed; the later ABORTED result does not
                    // restore any of it. Mirror that pre-abort teardown.
                    actor.wait_time = actor.seek_refresh_wait;
                    actor.seek_target = None;
                    actor.post_seek_sequence = None;
                    actor.clear_path();
                    actor.active_movement.clear();
                    actor.active_door_pass = None;
                    deferred.order_pops.push((move_seq_id, move_elem_idx));
                    return;
                }
                if final_entity_seek_arrival == Some(true) {
                    let actor = entity.actor_data_mut().expect("actor-only branch");
                    if actor.post_seek_sequence.is_some() && actor.active_door_pass.is_none() {
                        deferred
                            .post_seek_arrivals
                            .push((eid, move_seq_id, move_elem_idx));
                        actor.clear_path();
                        actor.active_movement.clear();
                        actor.active_door_pass = None;
                    } else {
                        // No action consumes the arrival yet. Match
                        // PerformSeek's frozen refresh arm rather than
                        // exhausting the final transition order.
                        actor.seek_refresh_wait = 0;
                    }
                    return;
                }
                // Point-target Seek reaches this early transition arm
                // after its authored stop transition terminates. Original
                // only starts the post-seek when mpSeekSector still equals
                // the actor's sector; a player Stop can terminate the
                // transition short of that sector.
                // Retiring it through the ordinary order-pop path first
                // creates a fallback Wait and leaves the post-seek action
                // stranded on ActorData for one frame (or forever).
                let final_point_post_seek_arrival = is_final_waypoint_after_transition_cleanup
                    && ft.target_id.is_none()
                    && prepass
                        .point_seek_post_sector
                        .map(|seek_sector| entity.element_data().sector() == Some(seek_sector))
                        .unwrap_or(false)
                    && entity
                        .actor_data()
                        .is_some_and(|actor| actor.post_seek_sequence.is_some())
                    && entity
                        .actor_data()
                        .is_some_and(|actor| actor.active_door_pass.is_none());
                // A copied terminal stop transition can lose the movement
                // element's target while Actor::PerformSeek still owns the
                // entity in mpSeekTarget. That remains entity-seek mode, not
                // point-seek mode: on the last order Original starts the
                // post-seek when the target is still in our sector and either
                // has not moved since RefreshSeek or is inside tolerance.
                let final_actor_entity_post_seek_arrival = is_final_waypoint
                    && movement_is_last_sequence_element
                    && ft.target_id.is_none()
                    && actor_seek_flags.contains(crate::sequence::MoveFlags::SEEK)
                    && live_actor_seek_target.is_some_and(
                        |(_, _, target_sector, target_unchanged_or_in_tolerance)| {
                            target_sector.is_some()
                                && target_sector == entity.element_data().sector()
                                && target_unchanged_or_in_tolerance
                        },
                    )
                    && entity
                        .actor_data()
                        .is_some_and(|actor| actor.post_seek_sequence.is_some())
                    && entity
                        .actor_data()
                        .is_some_and(|actor| actor.active_door_pass.is_none());
                let final_actor_owned_post_seek_arrival =
                    final_point_post_seek_arrival || final_actor_entity_post_seek_arrival;
                let actor_owned_interaction = entity
                    .is_pc()
                    .then(|| {
                        actor_post_seek_interaction(entity.actor_data().expect("actor-only branch"))
                    })
                    .flatten();
                let actor_owned_interaction_out_of_range = final_actor_owned_post_seek_arrival
                    && actor_owned_interaction == Some(ActorPostSeekInteraction::Hit)
                    && live_actor_seek_target
                        .map(|(target_position, _, _, _)| {
                            interaction_exceeds_init_range(
                                entity.element_data().position_map(),
                                target_position,
                            )
                        })
                        .unwrap_or(false);
                if actor_owned_interaction_out_of_range {
                    // A copied terminal transition can lose its movement
                    // element target while PerformSeek's mpSeekTarget
                    // remains actor-owned. HITTING still turns before its
                    // validity abort.
                    let (_, target_ground, _, _) = live_actor_seek_target
                        .expect("out-of-range actor-owned Hit retained a live target");
                    let here_ground = entity.ground_position();
                    let facing = vector_to_sector_0_to_15(
                        target_ground.x - here_ground.x,
                        target_ground.y - here_ground.y,
                    );
                    entity.element_data_mut().set_direction_goal(facing);

                    let actor = entity.actor_data_mut().expect("actor-only branch");
                    actor.wait_time = actor.seek_refresh_wait;
                    actor.seek_target = None;
                    actor.post_seek_sequence = None;
                    actor.clear_path();
                    actor.active_movement.clear();
                    actor.active_door_pass = None;
                    deferred.order_pops.push((move_seq_id, move_elem_idx));
                    return;
                }
                let actor = entity.actor_data_mut().expect("actor-only branch");
                if final_actor_owned_post_seek_arrival {
                    deferred
                        .post_seek_arrivals
                        .push((eid, move_seq_id, move_elem_idx));
                    actor.clear_path();
                    actor.active_movement.clear();
                    actor.active_door_pass = None;
                    return;
                }
                // Pop via the element we actually dispatched (`move_seq_id` /
                // `move_elem_idx`), not `actor.active_movement.sequence_id`
                // — the latter can be stale/None when the Move element was
                // launched by the AI without setting active_movement
                // (soldier chase paths).
                deferred.order_pops.push((move_seq_id, move_elem_idx));
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
                            &mut deferred.door_triggers,
                            &mut deferred.select_triggers,
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
                            deferred
                                .transition_pushes
                                .push((move_seq_id, move_elem_idx, order));
                        }
                        DoorPassAdvance::Paused { transition_order } => {
                            deferred.transition_pushes.push((
                                move_seq_id,
                                move_elem_idx,
                                transition_order,
                            ));
                        }
                        DoorPassAdvance::ActionPoint { order } => {
                            deferred
                                .transition_pushes
                                .push((move_seq_id, move_elem_idx, order));
                        }
                        DoorPassAdvance::Done { completed } => {
                            if let Some((door_index, direct)) = completed_door_pass_to_commit(
                                discarded_lazy_door_followers,
                                completed,
                            ) {
                                deferred
                                    .completed_door_passes
                                    .push((eid, door_index, direct));
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
                        deferred.terminal_door_pass_goal_clears.push(eid);
                    }
                }
            }
            return;
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
                deferred
                    .movement_state_effects
                    .push((entity_id, posture, next_action_state));
            }
            return;
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
        // Actor::Hourglass snapshots mpPosition before Execute and uses that
        // outer position for CheckForLineCrossing after Execute returns.
        // RunningStairs performs two literal motion commits inside Execute;
        // by this point `old_pos` is already the post-first-commit position.
        // Retain the outer snapshot so a bond crossed only by the first
        // substep is not lost.
        let crossing_old_pos = if first_fast_commit.is_some() {
            fast_motion_outer_pre
        } else {
            old_pos
        };
        let entity_layer = elem.layer();
        let entity_posture = elem.posture;
        let eligible_for_crossing = actor_line_crossing_eligible(
            entity_posture,
            human_is_carried,
            self.world
                .fast_grid
                .level
                .map_bbox
                .contains_point(crossing_old_pos),
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
        let ft = prepass.final_tolerance;
        let mut point_seek_post_arrival = is_final_waypoint
            && dist <= speed
            && prepass
                .point_seek_post_sector
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
            refresh_pc_walking_shield_after_execute(entity, &assets.profile_manager, order_action);
            return;
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
        let mut arrived_after_committed_step = false;
        let mut arrival_crossing_queued = false;
        'arrival: loop {
            if post_step_arrival {
                // Original PerformMotion/PerformSeek returns TERMINATED
                // after committing the step which reaches the goal. Rust
                // stages geometry after the sprite call, so its raw
                // motion state can still be DONE here. Queue the Human
                // Execute termination callback at the authoritative
                // arrival boundary; it owns the range-based Provoke
                // launched after sword movement.
                // Reached waypoint — snap to it and advance. Original's
                // ordinary PerformMotion snap lives inside `if (bMoving)`;
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
                arrival_crossing_queued |= queue_committed_arrival_crossing(
                    deferred,
                    eid,
                    crossing_old_pos,
                    entity_layer,
                    arrived_after_committed_step,
                    eligible_for_crossing,
                );

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
                    deferred
                        .transition_seek_refreshes
                        .push((eid, move_seq_id, move_elem_idx));
                    tracing::trace!(
                        ?eid,
                        "tick_move: final seek waypoint is stale; refreshing against live target",
                    );
                    refresh_pc_walking_shield_after_execute(
                        entity,
                        &assets.profile_manager,
                        order_action,
                    );
                    return;
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
                    && let Some(next_action) = self
                        .orders
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
                    let reach = (f32::from(entity.sprite().distance_for_animation(next_action))
                        + ft.tol)
                        * 1.05;
                    if dx * dx + dy * dy > reach * reach {
                        // PerformMotion already committed this frame's
                        // step before PerformSeek decided to refresh.
                        // Actor::Hourglass still runs
                        // CheckForLineCrossing after Execute returns, so
                        // preserve the segment even though the refreshed
                        // seek replaces the current movement before the
                        // crossing callback.
                        if eligible_for_crossing {
                            deferred
                                .line_cross_checks
                                .push((eid, crossing_old_pos, entity_layer));
                            deferred.non_elevation_cross_checks.push((
                                eid,
                                crossing_old_pos,
                                entity_layer,
                            ));
                        }
                        deferred
                            .transition_seek_refreshes
                            .push((eid, move_seq_id, move_elem_idx));
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
                        return;
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

                if is_sword_motion
                    && perform_seek_exposes_motion_termination(
                        start_post_seek,
                        final_entity_seek_arrival,
                    )
                {
                    deferred.sword_movement_terminations.push(entity_id);
                }

                // Waypoint reached — queue a `do_next_order` pop on
                // the actor's Move element.
                if start_post_seek {
                    deferred
                        .post_seek_arrivals
                        .push((eid, move_seq_id, move_elem_idx));
                } else {
                    deferred.order_pops.push((move_seq_id, move_elem_idx));
                }

                if start_post_seek {
                    // StartPostSeekSequence makes PerformSeek return
                    // TERMINATED, so Human::Execute observes the sword
                    // movement completion before Actor::Hourglass advances
                    // the selected element.
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
                    refresh_pc_walking_shield_after_execute(
                        entity,
                        &assets.profile_manager,
                        order_action,
                    );
                    return;
                }

                // With no post-seek tail, the successful final
                // entity-target arrival remains inside PerformSeek.  It
                // arms an immediate refresh check and returns InProgress
                // instead of consuming the final order.
                if final_entity_seek_arrival == Some(true) {
                    actor.seek_refresh_wait = 0;
                    refresh_pc_walking_shield_after_execute(
                        entity,
                        &assets.profile_manager,
                        order_action,
                    );
                    return;
                }

                if is_final_waypoint {
                    // All waypoints for current walk step consumed.
                    // Check if we have more door-pass steps.
                    let advance = if actor.active_door_pass.is_some() {
                        Self::advance_door_pass(
                            actor,
                            eid,
                            goal,
                            &mut deferred.door_triggers,
                            &mut deferred.select_triggers,
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
                            deferred
                                .transition_pushes
                                .push((move_seq_id, move_elem_idx, order));
                        }
                        DoorPassAdvance::Paused { transition_order } => {
                            // Transition animation queued — push the
                            // order onto the actor's current sequence
                            // element after the loop closes.
                            deferred.transition_pushes.push((
                                move_seq_id,
                                move_elem_idx,
                                transition_order,
                            ));
                        }
                        DoorPassAdvance::ActionPoint { order } => {
                            deferred
                                .transition_pushes
                                .push((move_seq_id, move_elem_idx, order));
                        }
                        DoorPassAdvance::Done { completed } => {
                            if let Some((door_index, direct)) = completed {
                                deferred
                                    .completed_door_passes
                                    .push((eid, door_index, direct));
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
                            // change itself. The two PC carry-walk Execute
                            // overrides are exceptions: both explicitly
                            // restore WAITING on RHMOTION_TERMINATED even
                            // when the Move has NO_TRANSITIONS.
                            if matches!(
                                order_action,
                                OrderType::WalkingWithCorpse
                                    | OrderType::WalkingCarryingOnShoulders
                            ) {
                                actor.action_state = crate::element::ActionState::Waiting;
                            }
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
                let movement_diag_pre_position = if first_fast_commit.is_some() {
                    fast_motion_outer_pre
                } else {
                    entity.element_data().position_map()
                };
                let movement_diag_old_position = entity.position_iface().old_map_position();
                let movement_diag_deviated_before = entity.position_iface().is_deviated();
                let movement_diag_blocked_count_before = entity.position_iface().blocked_count;
                // Preserve the two storage roundings of Original's
                // double-PerformMotion fast-climb dispatch. See the
                // transition branch above for why the summed distance is
                // insufficient even when both calls use one increment.
                let split_motion_target = split_motion_speeds
                    .filter(|_| !anti_on && first_fast_commit.is_none())
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
                    if anti_on
                        && let Some(mover_snap) = anti_snapshots
                            .get(actor_id)
                            .and_then(|slot| slot.as_ref())
                            .filter(|snapshot| snapshot.active)
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
                        let (dx_step, dy_step) = apply_prepared_anti_collision_step(
                            provenance_frame,
                            mover_snap,
                            anti_snapshots,
                            &self.ai.global.repulsive_points,
                            prepared,
                            &self.world.fast_grid,
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
                let new_pos_x;
                let new_pos_y;
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
                        // `IsDeviated`, and rebuilds the increment with
                        // direction computation enabled.
                        elem.sprite.position_iface.reset_increment_computed();
                        elem.sprite.position_iface.compute_increment_all(true);
                    }
                    new_pos_x = pm.x;
                    new_pos_y = pm.y;
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
                            deferred.water_splash_emits.push((
                                entity_id,
                                crate::coordinates::WorldPoint3D {
                                    x: pos.x,
                                    y: pos.y,
                                    z: pos.z,
                                },
                                layer,
                            ));
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
                    if let Some(seq_id) = actor.active_movement.sequence_id {
                        deferred
                            .blocked_impossible
                            .push((seq_id, actor.active_movement.element_index));
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
                        // The movement Execute switches have no ABORTED
                        // state arm. Actor::Hourglass marks the captured
                        // element Impossible, but the actor keeps whatever
                        // live state Execute established before returning.
                        // In particular a walking actor remains Moving;
                        // RunningUpright's unconditional Execute effect is
                        // applied below and still publishes MovingFast.
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
                    .is_goal_reached(&self.world.fast_grid, prepass.goal_target_info);
                let movement_diag_raw_post = entity.element_data().position_map();
                // PerformMotion snaps an undeviated zero-tolerance arrival
                // after IsGoalReached. Include that authoritative visible
                // result in the diagnostic while retaining the raw
                // anti-collision commit separately.
                let movement_diag_post = if movement_goal_reached
                    && order_tolerance == 0.0
                    && !entity.position_iface().is_deviated()
                {
                    goal
                } else {
                    movement_diag_raw_post
                };
                let movement_diag_split_calls =
                    if crate::movement_diagnostics::parity_movement_capture_active() {
                        split_motion_speeds.map_or_else(Vec::new, |(_, second_speed)| {
                            let mut calls = Vec::with_capacity(2);
                            if let Some((first_pre, first_increment, first_speed, first_post)) =
                                first_fast_commit
                            {
                                calls.push(crate::movement_diagnostics::ParityMovementCall {
                                    frame_distance_raw: first_frame_dist_raw.into(),
                                    effective_distance: first_speed.into(),
                                    pre_position: first_pre.into(),
                                    requested_delta: MapVec::new(
                                        first_increment.x * first_speed,
                                        first_increment.y * first_speed,
                                    )
                                    .into(),
                                    post_position: first_post.into(),
                                });
                            }
                            let (second_pre, second_increment) = second_fast_operands
                                .expect("split motion requires captured second-call operands");
                            calls.push(crate::movement_diagnostics::ParityMovementCall {
                                frame_distance_raw: second_frame_dist_raw
                                    .expect("split speeds require a second motion distance")
                                    .into(),
                                effective_distance: second_speed.into(),
                                pre_position: second_pre.into(),
                                requested_delta: MapVec::new(
                                    second_increment.x * second_speed,
                                    second_increment.y * second_speed,
                                )
                                .into(),
                                post_position: movement_diag_raw_post.into(),
                            });
                            calls
                        })
                    } else {
                        Vec::new()
                    };
                crate::movement_diagnostics::record_parity_movement_step(
                    crate::movement_diagnostics::ParityMovementStep {
                        entity: entity_id,
                        order_action: format!("{order_action:?}"),
                        animation: format!("{anim:?}"),
                        motion_method: format!("{motion_method:?}"),
                        pre_position: movement_diag_pre_position.into(),
                        old_position: movement_diag_old_position.into(),
                        goal: goal.into(),
                        cached_increment: cached_increment.into(),
                        frame_distance_raw: frame_dist_raw.into(),
                        speed_factor: speed_factor.into(),
                        speed_factor_applied: apply_speed_factor,
                        direction_differs_from_goal,
                        effective_distance: speed.into(),
                        anti_collision_on: anti_on,
                        deviated_before: movement_diag_deviated_before,
                        blocked_count_before: movement_diag_blocked_count_before,
                        requested_delta: crate::coordinates::MapVec::new(nx * speed, ny * speed)
                            .into(),
                        raw_committed_delta: (movement_diag_raw_post - movement_diag_pre_position)
                            .into(),
                        committed_delta: (movement_diag_post - movement_diag_pre_position).into(),
                        post_position: movement_diag_post.into(),
                        deviated_after: entity.position_iface().is_deviated(),
                        blocked_count_after: entity.position_iface().blocked_count,
                        goal_reached_after_commit: movement_goal_reached,
                        split_calls: movement_diag_split_calls,
                    },
                );
                point_seek_post_arrival = is_final_waypoint
                    && movement_goal_reached
                    && prepass
                        .point_seek_post_sector
                        .map(|seek_sector| entity.element_data().sector() == Some(seek_sector))
                        .unwrap_or(false);
                post_step_arrival = movement_goal_reached || tolerance_arrival;
                if post_step_arrival {
                    arrived_after_committed_step = true;
                    continue 'arrival;
                }
                break 'arrival;
            }
        }

        // RHElementActorPC::Execute updates the retained shield after
        // every WALKING_WITH_SHIELD PerformSeek/PerformMotion call,
        // including a tolerance-arrival frame with no displacement.
        refresh_pc_walking_shield_after_execute(entity, &assets.profile_manager, order_action);

        // Queue an elevation-line-cross check for this tick. The
        // actual fast-grid query + obstacle swap runs after the
        // loop, since `check_for_line_crossing` needs `&mut self`.
        //
        // Also queue a patch-line-cross check for PC actors —
        // LINE_PATCH handling is gated to PCs only.
        let new_pos = entity.element_data().position_map();
        let new_position_in_bounds = self.world.fast_grid.level.map_bbox.contains_point(new_pos);
        tracing::trace!(
            target: "robin_engine::elevation_crossing",
            ?entity_id,
            eligible_for_crossing,
            new_position_in_bounds,
            posture = ?entity_posture,
            human_is_carried,
            layer = entity_layer,
            old_x = crossing_old_pos.x,
            old_y = crossing_old_pos.y,
            new_x = new_pos.x,
            new_y = new_pos.y,
            "considered queuing elevation crossing"
        );
        if eligible_for_crossing && !arrival_crossing_queued {
            deferred
                .line_cross_checks
                .push((entity_id, crossing_old_pos, entity_layer));
            deferred
                .non_elevation_cross_checks
                .push((entity_id, crossing_old_pos, entity_layer));
        }
        // Order pops are drained after all actors so the current order is
        // still physically at the front here. Treat an already-queued
        // pop as a completed Execute when deciding whether a deferred
        // START survives this actor slot.
        let current_order_will_advance = deferred
            .order_pops
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
            deferred
                .movement_state_effects
                .push((entity_id, posture, next_action_state));
        }
        // The authored successor owns the deferred movement START only
        // if it remains current after this Execute. A very short
        // successor can complete and hand off to its stop transition in
        // the same call; Original retains Waiting in that case. The
        // Execute switch still only reacts to the motion state it is
        // handed, so a successor whose START the seek wrapper swallowed
        // owns no state effect to postpone.
        if deferred_movement_state_start_due
            && matches!(state_effect_motion, MotionState::Start)
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
            if executes_sword_movement {
                deferred.sword_movement_starts.push(entity_id);
            }
            deferred
                .movement_state_effects
                .push((entity_id, posture, next_action_state));
        }
        // A generated transition-distance copy reports START when first
        // booked, but its movement state is authoritative only if that
        // copied order remains current after the Execute. A short copy
        // may satisfy its arrival predicate and hand off in the same
        // call; Original retains the transition's Waiting state for that
        // frame. This survival rule applies to PCs too; their separate
        // deferred-successor marker covers the later authored order.
        if transition_distance_first_execute_due
            && matches!(state_effect_motion, MotionState::Start)
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
            if executes_sword_movement {
                deferred.sword_movement_starts.push(entity_id);
            }
            deferred
                .movement_state_effects
                .push((entity_id, posture, next_action_state));
        }
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
            let _ = self.tick_entity_movement_owner(sim, assets, owner, selected);
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
    pub(super) fn process_pending_ai_orders(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        let npc_ids: Vec<_> = self.world.entities.npc_ids().collect();
        for npc_id in npc_ids {
            self.launch_pending_orders_for_npc(sim, assets, npc_id);
        }
    }

    /// Per-NPC half of [`Self::process_pending_ai_orders`] — drains one
    /// NPC's `pending_orders` queue and launches the corresponding
    /// movement / turn / generic sequences.  Called both from the
    /// top-of-tick global pass and from the per-NPC synchronous drain
    /// in [`EngineInner::dispatch_think_with_drain`] so `Face` / `GoTo`
    /// etc. take effect inside the same call stack as the handler that
    /// issued them — `Face` / `GoTo` launch the sequence inline.
    pub(super) fn launch_pending_orders_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
    ) {
        self.launch_pending_orders_for_npc_mode(sim, assets, entity_id, false);
    }

    pub(super) fn launch_pending_orders_for_npc_mode(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
        defer_turn_instruction: bool,
    ) {
        self.launch_pending_orders_for_npc_mode_after_halt(
            sim,
            assets,
            entity_id,
            defer_turn_instruction,
            false,
        );
    }

    pub(super) fn launch_pending_orders_for_npc_mode_after_halt(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
        _defer_turn_instruction: bool,
        halt_already_applied: bool,
    ) {
        let debug_decision_path = crate::ai_enemy::decision_path_debug_enabled()
            && crate::ai_enemy::decision_path_debug_matches_raw(
                self.control.frame_counter,
                entity_id.index(),
            );
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
        if debug_decision_path {
            let (couldnt_reachpoint, already_on_point, owner_work) = self
                .world
                .entities
                .get(entity_id)
                .and_then(Entity::ai_controller)
                .map(|ai| {
                    (
                        ai.couldnt_reachpoint,
                        ai.already_on_point,
                        format!("{:?}", ai.outbox.reentrant.owner_work),
                    )
                })
                .unwrap_or_else(|| panic!("diagnostic owner {} lost AI", entity_id.index()));
            eprintln!(
                "AIDECISION frame={} owner={} stage=drain_intents count={} take_halt={} halt_already_applied={} couldnt={} already={} owner_work={}",
                self.control.frame_counter,
                entity_id.index(),
                intents.len(),
                take_halt,
                halt_already_applied,
                couldnt_reachpoint,
                already_on_point,
                owner_work,
            );
        }
        // Once an authorized GoTo has deferred the live path-waiter's tail
        // Halt, later GoTo calls in the same authored batch observe the
        // post-Halt state. The request drain preserves FIFO, so the marked
        // first replacement is constructed and cancelled before those later
        // moves are built.
        let mut path_waiter_tail_deferred = false;
        for intent in intents {
            let is_movement = matches!(
                intent.order_type,
                OrderType::WalkingUpright
                    | OrderType::RunningUpright
                    | OrderType::WalkingCrouched
                    | OrderType::WalkingAlerted
                    | OrderType::RiderCharging
            );
            if is_movement && !self.allied_allows_combat_movement(entity_id) {
                tracing::trace!(
                    ?entity_id,
                    order = ?intent.order_type,
                    "allied stance suppressed AI-authored combat movement"
                );
                self.resolve_ai_engine_completion_verdict(entity_id);
                continue;
            }
            match intent.order_type {
                OrderType::WalkingUpright
                | OrderType::RunningUpright
                | OrderType::WalkingCrouched
                | OrderType::WalkingAlerted
                | OrderType::RiderCharging => {
                    let was_computing_path = !path_waiter_tail_deferred
                        && self
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
                    if was_computing_path {
                        let ai = self
                            .world
                            .entities
                            .get_mut(entity_id)
                            .and_then(Entity::ai_controller_mut)
                            .unwrap_or_else(|| {
                                panic!(
                                    "movement owner {} lost AI while preserving path-waiter provenance",
                                    entity_id.index()
                                )
                            });
                        if ai.outbox.reentrant.reconsider_approach_completion_pending {
                            ai.outbox.reentrant.reconsider_approach_replaced_path_waiter = true;
                        }
                    }
                    // `find_accessible` / `ask_obstacle` pre-flight
                    // gates.  Run them before the halt so a failure
                    // leaves the outgoing sequence in place rather
                    // than tearing it down only to abandon the new
                    // move.
                    let mut intent = intent;
                    if !self.preflight_ai_goto(entity_id, &mut intent) {
                        self.resolve_ai_engine_completion_verdict(entity_id);
                        continue;
                    }
                    // A generated locomotion transition continues to own its
                    // previously published waypoint while a replacement Move
                    // becomes mpSequenceElement. Preserve that cached goal
                    // until the replacement installs a concrete waypoint.
                    // A concrete MoveOk walk/run does not have this lifetime:
                    // Original clears its goal at replacement arbitration and
                    // leaves zero visible until the new movement executes.
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
                                .is_some_and(|element| {
                                    element.data.is_movement()
                                        && element.current_order().is_some_and(|order| {
                                            movement_transition_retains_goal(order.order_type)
                                        })
                                })
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
                    let route_rejected_before_launch = was_computing_path
                        && !self.ai_move_gate_route_is_authorized(entity_id, &intent);
                    if route_rejected_before_launch {
                        self.set_ai_couldnt_reachpoint(entity_id);
                        self.resolve_ai_engine_completion_verdict(entity_id);
                    } else {
                        // `launch_ai_move` only stages the intent; the actual
                        // AppendMoveToSequence construction happens in the
                        // pending-request drain below. Preserve the effective
                        // GoTo tail until that drain so an existing
                        // MOVE_WAITING does not erase the replacement before
                        // its gate sequence (and building-exit random waits)
                        // has been constructed. Original constructs and
                        // launches first, then observes IsComputingPath and
                        // Halts (`RHartificialintelligence.cpp:2538-2620`).
                        intent.halt_after_launch_for_path_waiter = was_computing_path;
                        self.launch_ai_move(entity_id, &intent);
                        path_waiter_tail_deferred |= was_computing_path;
                    }
                    if debug_decision_path {
                        let ai = self
                            .world
                            .entities
                            .get(entity_id)
                            .and_then(Entity::ai_controller)
                            .unwrap_or_else(|| {
                                panic!("diagnostic owner {} lost AI", entity_id.index())
                            });
                        eprintln!(
                            "AIDECISION frame={} owner={} stage=launch_move_done order={:?} target=({:08x},{:08x}) move_flags={} tolerance_bits={:08x} no_halt={} reverse={} couldnt={} already={} owner_work={:?}",
                            self.control.frame_counter,
                            entity_id.index(),
                            intent.order_type,
                            intent.target_x.to_bits(),
                            intent.target_y.to_bits(),
                            intent.move_flags,
                            intent.tolerance.to_bits(),
                            intent.no_halt,
                            intent.reverse,
                            ai.couldnt_reachpoint,
                            ai.already_on_point,
                            ai.outbox.reentrant.owner_work,
                        );
                    }

                    // GoTo has a separate, effective tail check after
                    // launching its sequence: an actor whose old movement is
                    // still waiting on the pathfinder is halted. The halt
                    // also cancels the just-registered replacement, matching
                    // StopNotYetLaunchedSequenceElements.
                    if was_computing_path {
                        self.check_shape1_contract(entity_id);
                        if route_rejected_before_launch {
                            // No replacement sequence exists on this branch,
                            // so the tail can halt the outgoing waiter now.
                            // Authorized routes carry the halt marker through
                            // `do_launch_ai_move` and are halted immediately
                            // after construction by the request drain.
                            self.halt_actor(entity_id);
                        }
                    }
                }
                OrderType::Turning => {
                    let turn_command = if intent.fast_turn {
                        crate::element::Command::TurnFast
                    } else {
                        crate::element::Command::Turn
                    };
                    // FaceTo(point/vector) resolves its sector before it
                    // enters FaceTo(UWORD), which then Halts and registers
                    // TURN. Preserve that authored sector across both Halt
                    // and the deferred manager boundary instead of
                    // recomputing it from a potentially newer actor position.
                    let direction = intent.explicit_direction.or_else(|| {
                        self.world.entities.get(entity_id).map(|entity| {
                            let position = entity.element_data().position_map();
                            crate::position_interface::vector_to_sector_0_to_15_iso(
                                intent.target_x - position.x,
                                intent.target_y - position.y,
                            )
                        })
                    });
                    // A SetState callback can synchronously register an
                    // attentive-mode transition, then a re-entrant
                    // EventReachPoint can register FaceTo before the manager
                    // hourglass gets to either element. Original arbitration
                    // postpones the Turn without translating it, so its
                    // direction goal remains untouched until the attentive
                    // transition finishes.
                    let selected_is_movement = self
                        .orders
                        .sequence_manager
                        .current_element_for_actor(entity_id)
                        .and_then(|(seq, idx)| self.orders.sequence_manager.get_element(seq, idx))
                        .is_some_and(|element| element.data.is_movement());
                    if selected_is_movement || !intent.no_halt {
                        self.halt_actor(entity_id);
                    }
                    // FaceTo's Halt has now synchronously performed any
                    // selected element's Actor condolence.  Whatever map
                    // goal remains after that callback is the value the
                    // ensuing Turn observes: RHSprite::PerformAction does
                    // not overwrite PositionGoalMap for an animation
                    // order.  A halt that only rewrites a live movement into
                    // its stop transition leaves the element selected and the
                    // cached destination intact; a halt that interrupts the
                    // element outright clears it.  Sampling after the halt
                    // reproduces both without having to predict which
                    // happened.  Carry the result on the deferred element so
                    // Rust's staged Turn initialization does not mistake the
                    // animation order's zero destination for a movement goal.
                    // This also covers FaceTo launched from an
                    // EventReachPoint callback after an empty SEEK was
                    // rewritten to MOVE and terminated without becoming the
                    // actor's selected element.
                    let retained_goal = self
                        .world
                        .entities
                        .get(entity_id)
                        .map(|entity| entity.position_iface().map_goal());
                    self.launch_turn_sequence_deferred_no_transitions(
                        entity_id,
                        turn_command,
                        direction,
                        intent.target_x,
                        intent.target_y,
                        retained_goal,
                    );
                }
                _ => {
                    // Other order types go on their own single-order
                    // sequence for the animation driver to pick up.
                    let order = intent.stamp(self.orders.allocate_order_id());
                    self.launch_single_order_sequence_stamped(
                        sim,
                        assets,
                        entity_id,
                        crate::element::Command::Generic,
                        order,
                    );
                }
            }
            if !is_movement {
                self.resolve_ai_engine_completion_verdict(entity_id);
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

    /// Apply the source-position extraction performed at the start of
    /// `RHElementActor::InstructOwner(RHCOMMAND_MOVE)`.
    ///
    /// `RHCOMMAND_SEEK` falls through that same arm in the Original, so this
    /// must run before RefreshSeek resolves an entity target or constructs a
    /// cross-sector route.  Keeping it at the later path-dispatch boundary
    /// skips the correction whenever Seek is consumed while building its
    /// replacement sequence.
    pub(super) fn extract_move_instruction_owner(&mut self, owner: EntityId) -> bool {
        let (entity_layer, pf_idx, move_box_map) = {
            let Some(entity) = self.world.entities.get(owner) else {
                return false;
            };
            let pi = entity.position_iface();
            let pf_idx = {
                let index = pi.get_pathfinder_index();
                if index == u16::MAX { 0 } else { index }
            };
            (
                entity.element_data().layer(),
                pf_idx,
                *pi.get_move_box_map(),
            )
        };

        if self
            .world
            .fast_grid
            .is_position_authorized(&move_box_map, entity_layer)
        {
            return true;
        }

        let capture_extraction = crate::movement_diagnostics::parity_movement_capture_active();
        let original_box = move_box_map;
        let mut box_element = Self::expand_move_box_for_command_extraction(move_box_map);
        let expanded_box = box_element;
        let expanded_motion_lines = capture_extraction
            .then(|| {
                self.world
                    .fast_grid
                    .get_active_motion_line_indices(entity_layer, &expanded_box)
                    .into_iter()
                    .map(|index| usize::from(index) as u32)
                    .collect()
            })
            .unwrap_or_default();
        let authorized = self
            .world
            .fast_grid
            .find_authorized_position(&mut box_element, entity_layer);
        let authorized_box = box_element;
        let authorized_motion_lines = capture_extraction
            .then(|| {
                self.world
                    .fast_grid
                    .get_active_motion_line_indices(entity_layer, &authorized_box)
                    .into_iter()
                    .map(|index| usize::from(index) as u32)
                    .collect()
            })
            .unwrap_or_default();
        let center = authorized.then(|| authorized_box.center());
        if let Some(center) = center {
            let source = MapPoint::new(center.x, center.y);
            let entity = self.get_entity_mut(owner).unwrap_or_else(|| {
                panic!("RHCOMMAND_MOVE extraction owner {owner:?} disappeared after lookup")
            });
            entity.position_iface_mut().set_map_position(source);
            let elem = entity.element_data_mut();
            elem.set_position_map(source);
            elem.update_grid_cell();
        }
        let corrected_position = authorized.then(|| {
            self.get_entity(owner)
                .unwrap_or_else(|| {
                    panic!("RHCOMMAND_MOVE extraction owner {owner:?} disappeared after correction")
                })
                .element_data()
                .position_map()
        });
        let sector = self
            .get_entity(owner)
            .and_then(|entity| entity.element_data().sector())
            .map(u16::from);
        if capture_extraction {
            crate::movement_diagnostics::record_parity_move_box_extraction(
                crate::movement_diagnostics::ParityMoveBoxExtraction {
                    entity: owner,
                    layer: entity_layer,
                    sector,
                    pathfinder_area: pf_idx,
                    original_box: original_box.into(),
                    expanded_box: expanded_box.into(),
                    expanded_motion_lines,
                    authorized,
                    authorized_box: authorized_box.into(),
                    authorized_motion_lines,
                    authorized_center: center.map(Into::into),
                    corrected_position: corrected_position.map(Into::into),
                },
            );
        }

        true
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

        let indices = self
            .world
            .fast_grid
            .get_crossing_elevation_line_indices(layer, old_pos, new_pos);
        self.check_for_elevation_line_crossing_indices(
            assets, entity_id, old_pos, new_pos, layer, indices,
        )
    }

    /// Dispatch an already-filtered elevation subset from Actor's unified
    /// `LINE_CROSS` list. This keeps Original's candidate count and callback
    /// set on the same boundary.
    fn check_for_elevation_line_crossing_indices(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        old_pos: MapPoint,
        new_pos: MapPoint,
        layer: u16,
        mut indices: Vec<crate::fast_find_grid::LineIndex>,
    ) -> bool {
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

    /// Dispatch the Original actor `CheckForLineCrossing` non-elevation tail.
    ///
    /// Patch, script, and sound lines share one candidate list and one stable
    /// distance sort. For each line the Original checks those flags in that
    /// order, so callbacks from different boundary kinds remain interleaved
    /// by the actor's travel order rather than grouped by kind.
    pub(super) fn check_for_non_elevation_line_crossing(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
        old_pos: MapPoint,
        new_pos: MapPoint,
        layer: u16,
    ) {
        if old_pos == new_pos {
            return;
        }
        let indices = self
            .world
            .fast_grid
            .get_actor_non_elevation_crossing_line_indices(layer, old_pos, new_pos);
        self.check_for_non_elevation_line_crossing_indices(
            sim, assets, entity_id, old_pos, new_pos, indices,
        );
    }

    fn check_for_non_elevation_line_crossing_indices(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
        old_pos: MapPoint,
        new_pos: MapPoint,
        indices: Vec<crate::fast_find_grid::LineIndex>,
    ) {
        if indices.is_empty() {
            return;
        }

        let movement = crate::geo2d::segment(old_pos.to_geo(), new_pos.to_geo());
        let mut crossed: Vec<(f32, crate::fast_find_grid::LineIndex)> = indices
            .into_iter()
            .filter_map(|line_index| {
                let line = &self.world.fast_grid.level.lines[usize::from(line_index)];
                let old_dx = old_pos.x - line.a.x;
                let old_dy = old_pos.y - line.a.y;
                let line_dx = line.b.x - line.a.x;
                let line_dy = line.b.y - line.a.y;
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

        let is_pc = self
            .get_entity(entity_id)
            .unwrap_or_else(|| panic!("line-crossing actor {entity_id} is missing"))
            .is_pc();
        for (_, line_index) in crossed {
            let (is_patch, is_script, is_sound) = {
                let line = &self.world.fast_grid.level.lines[usize::from(line_index)];
                (line.is_patch, line.is_script, line.is_sound)
            };
            if is_patch && is_pc {
                self.dispatch_patch_line_crossing(sim, assets, entity_id, new_pos, line_index);
            }
            if is_script {
                self.dispatch_script_line_crossing(sim, assets, entity_id, new_pos, line_index);
            }
            if is_sound {
                self.dispatch_sound_line_crossing(assets, entity_id, new_pos, line_index);
            }
        }
    }

    /// Close Original `RHElementActor::Hourglass`'s post-`Execute`
    /// `CheckForLineCrossing` boundary for an actor whose selected Execute arm
    /// moved it without going through the movement owner.
    ///
    /// Original collects every `LINE_CROSS` candidate once. With exactly one
    /// line it recomputes the actor increment only when that line is an
    /// elevation bond; with multiple lines it unconditionally runs the shared
    /// recompute block, even when every candidate is non-elevation. Keep that
    /// observable branch shape: corpse placement can cross coincident sound
    /// and script/patch boundaries while initializing a generic dying order.
    pub(super) fn dispatch_actor_post_execute_line_crossing(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
        entry_compute_direction: Option<bool>,
    ) {
        #[cfg(test)]
        observe_post_execute_crossing(self, entity_id);
        let (old_pos, new_pos, layer, posture, is_carried, is_human) = {
            let entity =
                self.world.entities.get(entity_id).unwrap_or_else(|| {
                    panic!("post-Execute crossing owner {entity_id:?} is missing")
                });
            (
                entity.position_iface().old_map_position(),
                entity.element_data().position_map(),
                entity.element_data().layer(),
                entity.element_data().posture,
                entity
                    .human_data()
                    .is_some_and(|human| human.carrier.is_some()),
                entity.is_human(),
            )
        };
        if old_pos == new_pos
            || !actor_line_crossing_eligible(
                posture,
                is_carried,
                self.world.fast_grid.level.map_bbox.contains_point(new_pos),
            )
        {
            return;
        }

        // Original obtains one SBListUnique<RHLine*> for LINE_CROSS, then
        // filters it once. Keep this exact list stable across callbacks: an
        // Enter/Leave may change line activity but cannot retroactively change
        // this Hourglass's branch or dispatch set.
        let crossing_indices = self
            .world
            .fast_grid
            .get_actor_crossing_line_indices(layer, old_pos, new_pos);
        let crossing_count = crossing_indices.len();
        if crossing_count == 0 {
            return;
        }
        let elevation_indices = crossing_indices
            .iter()
            .copied()
            .filter(|&line_index| {
                self.world.fast_grid.level.lines[usize::from(line_index)].is_elevation
            })
            .collect::<Vec<_>>();
        // In Original's single-line arm every flag on that one RHLine is
        // dispatched. In the multi-line arm elevation lines are grouped at
        // the front and excluded from the later patch/script/sound loop.
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
            // `mpOrder` is the entry-latched pointer in Original. Execute may
            // already have exhausted the Rust order deque, but Actor does not
            // call DoNextOrder until after this crossing boundary.
            if let Some(compute_direction) = entry_compute_direction
                && let Some(entity) = self.world.entities.get_mut(entity_id)
            {
                // Preserve PositionInterface's cached-computation contract:
                // Original calls ComputeIncrementAll here without forcibly
                // clearing its flags. Elevation crossing/reprojection may
                // have invalidated them; an otherwise cached vector remains
                // authoritative.
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
    fn dispatch_patch_line_crossing(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
        new_pos: MapPoint,
        line_index: crate::fast_find_grid::LineIndex,
    ) {
        let occupant = crate::patch::OccupantId(entity_id.index());

        // `Patch::enter` / `leave` recurse onto the actor's carried
        // entity when the actor is a PC and is currently carrying
        // someone.  Resolve that here once (same entity for every
        // crossed patch this tick) so each per-patch Enter/Leave can
        // mirror the recursion. The combined dispatcher gates this arm to
        // PCs, matching CheckForLineCrossing.
        let carried_occupant = self
            .get_entity(entity_id)
            .and_then(|e| match e {
                crate::element::Entity::Pc(pc) => pc.pc.carried,
                _ => None,
            })
            .map(|cid| crate::patch::OccupantId(cid.index()));

        let patch_index = self.world.fast_grid.level.lines[usize::from(line_index)]
            .patch_index
            .unwrap_or_else(|| panic!("LINE_PATCH {line_index:?} has no owning patch"));
        // Snapshot the apply-sector polygon test result + active state before
        // mutating the patch, preserving is_active → is_inside → Enter/Leave
        // → conditional Apply.
        let patch_usize = patch_index.get() as usize;
        let Some(patch) = self.script_domains.interactables.patches.get(patch_usize) else {
            return;
        };
        if !patch.is_active() {
            return;
        }
        let Some(apply_sector_idx) = patch.apply_sector_index else {
            tracing::warn!(
                patch = %patch_index,
                "LINE_PATCH crossing on patch with no apply sector — skipping",
            );
            return;
        };
        let Some(apply_sector) = self
            .world
            .fast_grid
            .level
            .sectors
            .get(apply_sector_idx as usize)
        else {
            return;
        };
        let inside_apply = apply_sector.contains_point(new_pos);

        let effects = {
            let Some(patch) = self
                .script_domains
                .interactables
                .patches
                .get_mut(patch_usize)
            else {
                return;
            };
            if inside_apply {
                patch.enter(occupant);
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
    fn dispatch_sound_line_crossing(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        new_pos: MapPoint,
        line_index: crate::fast_find_grid::LineIndex,
    ) {
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

        let line = &self.world.fast_grid.level.lines[usize::from(line_index)];
        let raw_index = line
            .sound_material_sector_index
            .unwrap_or_else(|| panic!("LINE_SOUND {line_index:?} has no owning material sector"));
        let sector = assets
            .all_material_sectors
            .get(usize::from(raw_index))
            .and_then(Option::as_ref)
            .unwrap_or_else(|| {
                panic!("LINE_SOUND {line_index:?} references missing material sector {raw_index}")
            });
        let new_material = if sector.contains(new_pos) {
            sector.material
        } else {
            obstacle_material
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
                    ?line_index,
                    "dispatch_sound_line_crossing: refreshed material"
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
        // Human::DetermineMovementAnimation chooses the sword token only
        // from the action state stamped by Actor::Instruct. The FORCE flag
        // does not participate in translation; Human::Execute reads it later
        // solely to keep an already-translated sword move alive after the
        // opponent list becomes empty. This distinction matters when an
        // upright Move|FORCE is postponed behind QuitSwordfight: on resume
        // the freshly stamped Waiting state must produce an ordinary walk.
        let sword_movement_context =
            posture_after == crate::element::Posture::Upright && action_after.is_sword();
        if sword_movement_context {
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
        // authors the logical shield token unconditionally; FaceDangerPoint
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
                        "DetermineMovementAnimation: shield action_state with \
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
        // Base DetermineMovementAnimation switches on the actor's *live*
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
                // weapon. Original's base DetermineMovementAnimation treats
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
        // RHElementActorHuman::DetermineMovementAnimation handles upright
        // sword states in the derived override and deliberately does not call
        // RHElementActor's base implementation.  The logical sword token is
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
                elem.posture_after_transition = entity.element_data().posture;
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
            )
        };

        // A PC disguised as an anonymous archer is pinned to its shooting
        // spot for the duration of the contest: the move is refused outright
        // and the hero complains instead of walking away.
        if owner_is_pc
            && self.world.entities.get(owner).is_some_and(|e| {
                e.element_data().posture == crate::element::Posture::AnonymousArcher
            })
        {
            tracing::debug!(
                actor = ?owner,
                "try_dispatch_move_path: anonymous archer may not move",
            );
            self.hero_speaking(
                _assets,
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
        let straight_ok = if movement_flags_force_direct_dispatch(move_flags) {
            true
        } else {
            let reachable =
                self.world
                    .fast_grid
                    .is_reachable_thick(source, dest, entity_layer, half_diagonal);
            self.world.fast_grid.trace_reachable_thick_decision(
                self.control.frame_counter,
                source,
                dest,
                entity_layer,
                half_diagonal,
                reachable,
            );
            reachable
        };

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
                // `RHPathFinder::AddPathRequest` (`RHpathfinder.cpp:464-465`)
                // calls `pRequest->pActor->Stop()` with no argument, so the
                // stop priority is the declared default
                // `RHPRIORITY_NORMAL` (`RHelementactor.h:273`) — NOT
                // `RHPRIORITY_WAIT`.  The distinction decides whether the
                // incoming Normal-priority Move element is stopped at all:
                // `RHSequenceElement::Stop` only acts when
                // `mPriority >= priorityOfStop`
                // (`RHsequenceelement.cpp:528`), and `RHPRIORITY_NORMAL`(8)
                // is stronger than `RHPRIORITY_WAIT`(9)
                // (`RHsequenceelement.h:38-51`).  With `Wait` the Move
                // survived, kept the actor's selection, and left the sprite's
                // PositionGoalMap installed for one extra frame.
                tracing::warn!(
                    actor = ?owner,
                    src_x = source.x,
                    src_y = source.y,
                    layer = entity_layer,
                    "try_dispatch_move_path: actor cannot be extracted from obstacle (Stop + Wait)",
                );
                self.stop_owner(owner, crate::sequence::SequencePriority::Normal);
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

        let selected_pre_path_tail = self
            .orders
            .sequence_manager
            .current_order_for_actor(owner)
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
        discard_unrequested_path_source(&mut waypoints, use_first_point);
        let mut rewritten_installed_order = None;
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
                rewritten_installed_order = selected_pre_path_tail
                    .and_then(|tail_index| elem.orders.get(tail_index))
                    .map(|order| crate::element::InstalledActorOrder {
                        order_id: order.order_id,
                        order_type: order.order_type,
                    });
            }
        }

        if let Some(installed_order) = rewritten_installed_order {
            // ProcessPathRequests reuses the selected movement's final
            // pre-path RHOrder, calls NewID, and changes its action in place.
            // Keep the explicit mpOrder mirror on that rewritten object; a
            // later PostProcessPath may insert other orders ahead of it but
            // does not repoint mpOrder until the next Actor::Hourglass.
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

        // Install the derived Rust movement latch, but do not change the
        // actor's action state here. Original Translate/PostProcessPath only
        // builds the order queue; the later actor Execute slot changes state
        // when PerformMotion returns START (for every movement family, not
        // only sword movement). This distinction is observable when the
        // sequence manager instructs a Move after the actor loop: that frame
        // must retain the pre-movement action state.
        if let Some(entity) = self.world.entities.get_mut(owner)
            && let Some(actor) = entity.actor_data_mut()
        {
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

/// Append the PC arrival bark after all movement at the next command level.
///
/// Original `PerformMove` passes `uwCount` by reference through
/// `AppendMoveToSequence`; every appended movement consumes the current value
/// and increments it before `SPEAK_HERO_REACH_DESTINATION` is constructed
/// (`original-code/RHengine.cpp:10046-10052`,
/// `original-code/RHsequence.cpp:657-661`). Keeping the bark parallel with a
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

#[cfg(test)]
mod arrival_speech_topology_tests {
    use super::*;
    use crate::element::Command;
    use crate::order::OrderType;
    use crate::sequence::{Sequence, SequenceElement};

    #[test]
    fn same_sector_arrival_speech_follows_move_instead_of_running_in_parallel() {
        let owner = EntityId::Pc(crate::entity_id::PcId(7));
        let mut sequence = Sequence::new();
        sequence.append_element(SequenceElement::new_movement(
            1,
            Command::Move,
            Some(owner),
            OrderType::WalkingUpright,
        ));

        append_arrival_speech(&mut sequence, owner);

        assert_eq!(sequence.elements.len(), 2);
        assert_eq!(sequence.elements[0].command_level, 1);
        assert_eq!(
            sequence.elements[1].command,
            Command::SpeakHeroReachDestination
        );
        assert_eq!(
            sequence.elements[1].command_level, 2,
            "arrival speech must wait for the pathfinding Move to finish"
        );
    }
}

impl EngineInner {
    fn trace_selected_movement_order_pop(
        &self,
        stage: &'static str,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        result: &'static str,
    ) {
        let frame = self.control.frame_counter;
        if !movement_pop_goal_owner_debug_matches(frame, owner) {
            return;
        }

        let manager = &self.orders.sequence_manager;
        let captured = manager.get_element(seq_id, elem_idx).map(|element| {
            (
                element.command,
                element.state,
                element.data.is_movement(),
                element.orders.len(),
                element.current_order().map(|order| {
                    (
                        order.order_type,
                        order.order_id,
                        order.done,
                        order.target_x.to_bits(),
                        order.target_y.to_bits(),
                    )
                }),
            )
        });
        let live_selected = manager.current_element_for_actor(owner);
        let live_order =
            manager
                .current_order_for_actor(owner)
                .map(|(live_seq, live_idx, order)| {
                    (
                        live_seq,
                        live_idx,
                        order.order_type,
                        order.order_id,
                        order.done,
                        order.target_x.to_bits(),
                        order.target_y.to_bits(),
                    )
                });
        let translating = manager.goal_owner_debug_translating();
        let entity = self.world.entities.get(owner).unwrap_or_else(|| {
            panic!("movement-pop diagnostic owner {owner:?} disappeared at frame {frame}")
        });
        let actor = entity.actor_data().unwrap_or_else(|| {
            panic!("movement-pop diagnostic owner {owner:?} is not an actor at frame {frame}")
        });
        let position = entity.position_iface();
        eprintln!(
            "[GOAL_OWNER frame={frame} owner={owner:?} stage=movement_pop_{stage} result={result} captured_seq={seq_id:?} captured_elem={elem_idx} captured={captured:?} live_selected={live_selected:?} live_order={live_order:?} translating={translating:?} active_movement={:?} installed_order={:?} action_state={:?} goal={:?} position={:?} moving={} moving_map={}]",
            actor.active_movement,
            actor.installed_order,
            actor.action_state,
            position.map_goal(),
            position.map_position(),
            position.is_moving(),
            position.is_moving_map(),
        );
    }

    fn pop_selected_movement_order(
        &mut self,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        let selection = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .and_then(|element| {
                let owner = element.owner?;
                let selected = (element.state == crate::sequence::SequenceState::InProgress
                    && element.data.is_movement()
                    && self
                        .orders
                        .sequence_manager
                        .current_element_for_actor(owner)
                        == Some((seq_id, elem_idx)))
                .then_some((owner, element.orders.len() == 1));
                Some((owner, selected))
            });
        let diagnostic_owner = selection.map(|(owner, _)| owner);
        if let Some(owner) = diagnostic_owner {
            self.trace_selected_movement_order_pop("entry", owner, seq_id, elem_idx, "pending");
        }
        let selected = selection.and_then(|(_, selected)| selected);
        let Some((owner, final_order_will_exhaust)) = selected else {
            if let Some(owner) = diagnostic_owner {
                let manager = &self.orders.sequence_manager;
                let result = match manager.get_element(seq_id, elem_idx) {
                    None => "rejected_missing_element",
                    Some(element) if element.owner.is_none() => "rejected_missing_owner",
                    Some(element)
                        if element.state != crate::sequence::SequenceState::InProgress =>
                    {
                        "rejected_not_in_progress"
                    }
                    Some(element) if !element.data.is_movement() => "rejected_not_movement",
                    Some(_) => "rejected_live_selection_mismatch",
                };
                self.trace_selected_movement_order_pop("return", owner, seq_id, elem_idx, result);
            }
            // The pop was collected before a synchronous callback selected a
            // replacement. It no longer owns either the actor order or its
            // sprite goal, so applying it now would mutate the replacement.
            return;
        };

        if final_order_will_exhaust {
            // `RHElementActor::DoNextOrder` exhausts the selected Move, and
            // its synchronous `SendCondolationCard` clears PositionGoalMap
            // before a postponed replacement is instructed. Rust's
            // `do_next_order` drives that same synchronous promotion.
            // Invalidate the replacement's queue-time snapshot first, or a
            // promoted MoveWaiting can restore the outgoing goal during the
            // callback and hide the selected-element clear until its first
            // Execute.
            self.orders
                .sequence_manager
                .clear_retained_movement_goals_for_actor(owner);
        }
        self.do_next_order(seq_id, elem_idx);
        self.trace_selected_movement_order_pop("return", owner, seq_id, elem_idx, "accepted");
    }

    pub(in crate::engine) fn advance_live_order_after_reentrant_seek(&mut self, owner: EntityId) {
        if let Some((seq_id, elem_idx)) = self
            .orders
            .sequence_manager
            .current_element_for_actor(owner)
        {
            let exhausts_pending_move = self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .is_some_and(|element| {
                    element.command == crate::element::Command::MoveWaiting
                        && element.orders.len() == 1
                });
            if exhausts_pending_move {
                // The live DoNextOrder below exhausts the re-entrantly
                // selected RHSequenceElementMovement. Its Original
                // SetState(TERMINATED) teardown calls CancelPathRequest, so
                // the retained logical queue head must still complete one
                // frame later with valid=false and an empty raw path
                // (RHpathfinder.cpp:538-598, 712-738; FindPathNodes exits on
                // mbIgnoreNextPath at 3130-3150).
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
            self.do_next_order(seq_id, elem_idx);
        } else if debug_post_seek_handoff_enabled() {
            eprintln!(
                "[POST_SEEK frame={} owner={owner:?} stage=live_advance_no_current]",
                self.control.frame_counter,
            );
        }
    }

    pub(in crate::engine) fn live_pending_seek_freezing_order(&self, owner: EntityId) -> bool {
        let Some(actor) = self.world.entities.get(owner).and_then(Entity::actor_data) else {
            return false;
        };
        if actor.seek_target.is_none() || actor.post_seek_sequence.is_none() {
            return false;
        }
        self.orders
            .sequence_manager
            .current_element_for_actor(owner)
            .and_then(|(seq_id, elem_idx)| {
                self.orders.sequence_manager.get_element(seq_id, elem_idx)
            })
            .is_some_and(|element| {
                element.command == crate::element::Command::MoveWaiting
                    && element.orders.len() == 1
                    && element
                        .current_order()
                        .is_some_and(|order| order.order_type == OrderType::Freezing)
            })
    }

    pub(in crate::engine) fn live_seek_has_completed_parallel_element(
        &self,
        owner: EntityId,
    ) -> bool {
        self.orders
            .sequence_manager
            .current_element_for_actor(owner)
            .and_then(|(sequence_id, element_index)| {
                let sequence = self.orders.sequence_manager.get_sequence(sequence_id)?;
                let command_level = sequence.elements.get(element_index)?.command_level;
                Some(
                    sequence
                        .elements
                        .iter()
                        .enumerate()
                        .any(|(index, element)| {
                            index != element_index
                                && element.command_level == command_level
                                && element.state == crate::sequence::SequenceState::Terminated
                        }),
                )
            })
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod orphaned_sword_movement_tests {
    use super::*;
    use crate::element::{
        ActionState, ActorData, ActorPc, ActorSoldier, Command, ElementData, ElementKind, Entity,
        HumanData, NpcData, PcData, Posture, SoldierData,
    };
    use crate::order::Order;
    use crate::sequence::{
        MoveFlags, SequenceElement, SequenceElementData, SequencePriority, SequenceState,
    };
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    fn make_test_pc(posture: Posture) -> Entity {
        Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                posture,
                ..Default::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        })
    }

    fn assets_with_test_pc_profile() -> LevelAssets {
        let mut profiles = crate::profiles::ProfileManager::new();
        profiles
            .characters
            .push(crate::profiles::CharacterProfile::default());
        profiles.soldiers.push(crate::profiles::SoldierProfile {
            hth_weapon_id: 1,
            ..crate::profiles::SoldierProfile::default()
        });
        profiles
            .hth_weapons
            .push(crate::profiles::HtHWeaponProfile::default());
        LevelAssets {
            profile_manager: std::sync::Arc::new(profiles),
            ..LevelAssets::new()
        }
    }

    fn shield_movement_sprite() -> crate::sprite::Sprite {
        let directional_script = |action: OrderType| SpriteScript {
            action_id: action as u16,
            action_done: 0,
            average_speed: 8.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 8,
            frame_ids: vec![1],
            delays: vec![0],
            distances: vec![8],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
        };
        let mut scripts = Vec::with_capacity(64);
        for action in [
            OrderType::WalkingShield,
            OrderType::WalkingBackwardsShield,
            OrderType::StrafingRightShield,
            OrderType::StrafingLeftShield,
        ] {
            scripts.extend(vec![directional_script(action); 16]);
        }
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[OrderType::WalkingShield as usize] = 0;
        conversion[OrderType::WalkingBackwardsShield as usize] = 16;
        conversion[OrderType::StrafingRightShield as usize] = 32;
        conversion[OrderType::StrafingLeftShield as usize] = 48;
        crate::sprite::Sprite::new(
            std::sync::Arc::new(scripts),
            std::sync::Arc::new(conversion),
        )
    }

    fn install_pc_walking_shield(
        engine: &mut EngineInner,
        start: MapPoint,
        destination: MapPoint,
        action_state: ActionState,
    ) -> EntityId {
        let mut pc = make_test_pc(Posture::Upright);
        pc.element_data_mut().sprite = shield_movement_sprite();
        pc.element_data_mut().sprite.position_iface.set_move_box(
            crate::coordinates::MoveBox::from_coords(-4.0, -4.0, 4.0, 4.0),
        );
        pc.element_data_mut()
            .sprite
            .position_iface
            .set_anti_collision_on(false);
        pc.element_data_mut().set_position_map(start);
        pc.element_data_mut().set_direction_instantly(4);
        pc.actor_data_mut().unwrap().action_state = action_state;
        let owner = engine.add_entity(pc);

        let mut movement = SequenceElement::new_movement(
            1,
            Command::Move,
            Some(owner),
            OrderType::WalkingWithShield,
        );
        movement.orders.push_back(Order::test_new(
            OrderType::WalkingWithShield,
            destination.x,
            destination.y,
        ));
        let sequence = engine.orders.sequence_manager.launch_element(movement);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);
        owner
    }

    fn dispatch_shield_movement(
        owner_is_pc: bool,
        live_action_state: ActionState,
        stamped_action_state: ActionState,
        install_shield_row: bool,
        on_lift: bool,
    ) -> OrderType {
        let mut engine = EngineInner::new();
        let start = MapPoint::new(100.0, 100.0);
        let destination = MapPoint::new(140.0, 100.0);
        let mut element = ElementData {
            kind: if owner_is_pc {
                ElementKind::ActorPc
            } else {
                ElementKind::ActorSoldier
            },
            active: true,
            posture: Posture::Upright,
            sprite: if install_shield_row {
                shield_movement_sprite()
            } else {
                crate::sprite::Sprite::default()
            },
            ..ElementData::default()
        };
        element
            .sprite
            .position_iface
            .set_move_box(crate::coordinates::MoveBox::from_coords(
                -4.0, -4.0, 4.0, 4.0,
            ));
        element.sprite.position_iface.set_anti_collision_on(false);
        element.set_position_map(start);
        if on_lift {
            use crate::fast_find_grid::GridSector;
            use crate::sector::{LiftType, SectorNumber, SectorType};

            let sector_number = SectorNumber::new(1);
            element.set_sector(Some(
                crate::position_interface::SectorHandle::new(1).unwrap(),
            ));
            let level = std::sync::Arc::make_mut(&mut engine.world.fast_grid_mut().level);
            level.sector_number_map.insert(sector_number, 0);
            level.sectors.push(GridSector {
                points: Vec::new(),
                bounding_box: crate::coordinates::MapBBox::new(),
                sector_type: SectorType::LIFT,
                layer: 0,
                sector_number,
                door_index: None,
                lift_type: Some(LiftType::Stairs),
                lift_direction: 0,
                force_crouched: false,
                building_index: None,
                low_exit_point: None,
                high_exit_point: None,
                lowest_door_index: None,
                jump_line_indices: Vec::new(),
                gate_indices: Vec::new(),
                underlying_sector: None,
            });
        }
        let actor = ActorData {
            action_state: live_action_state,
            ..ActorData::default()
        };
        let owner = if owner_is_pc {
            engine.add_entity(Entity::Pc(ActorPc {
                element,
                actor,
                human: HumanData::default(),
                pc: PcData::default(),
            }))
        } else {
            engine.add_entity(Entity::Soldier(ActorSoldier {
                element,
                actor,
                human: HumanData::default(),
                npc: NpcData::default(),
                soldier: SoldierData::default(),
            }))
        };

        let mut movement =
            SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::WalkingUpright);
        movement.posture_after_transition = Posture::Upright;
        movement.action_state_after_transition = stamped_action_state;
        let SequenceElementData::Movement {
            destination: goal, ..
        } = &mut movement.data
        else {
            unreachable!("new_movement must create movement data")
        };
        *goal = destination;
        let sequence = engine.orders.sequence_manager.launch_element(movement);
        assert!(matches!(
            engine.try_dispatch_move_path(
                &crate::sim_rng::test_context(),
                &LevelAssets::new(),
                owner,
                sequence,
                0,
                destination,
                OrderType::WalkingUpright,
            ),
            MovePathOutcome::Success
        ));
        let movement = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .expect("dispatched shield movement must remain registered");
        let SequenceElementData::Movement { action, .. } = &movement.data else {
            panic!("dispatched shield movement changed data kind")
        };
        *action
    }

    #[test]
    fn soldier_live_shield_state_rewrites_new_upright_path_request() {
        assert_eq!(
            dispatch_shield_movement(
                false,
                ActionState::MovingShield,
                ActionState::Waiting,
                false,
                false,
            ),
            OrderType::WalkingWithShield,
            "Actor::DetermineMovementAnimation reads Soldier 61's live shield state, not the older sequence stamp"
        );
    }

    #[test]
    fn pc_stamped_shield_override_still_rewrites_upright_path_request() {
        assert_eq!(
            dispatch_shield_movement(
                true,
                ActionState::Waiting,
                ActionState::HoldingShield,
                false,
                true,
            ),
            OrderType::WalkingWithShield,
            "the PC override owns its stamped shield arm before sprite-row checks and base lift translation"
        );
    }

    #[test]
    fn terminal_pc_walking_with_shield_stamps_holding_shield() {
        let mut engine = EngineInner::new();
        let assets = assets_with_test_pc_profile();
        let owner = install_pc_walking_shield(
            &mut engine,
            MapPoint::new(100.0, 100.0),
            MapPoint::new(100.0, 100.0),
            ActionState::Waiting,
        );

        engine.tick_entity_movement(&crate::sim_rng::test_context(), &assets);

        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .action_state,
            ActionState::HoldingShield
        );
    }

    #[test]
    fn frozen_all_pc_walking_with_shield_still_refreshes_retained_box() {
        let mut engine = EngineInner::new();
        let assets = assets_with_test_pc_profile();
        let mut pc = make_test_pc(Posture::Upright);
        pc.element_data_mut().sprite = shield_movement_sprite();
        pc.element_data_mut()
            .set_position_map(MapPoint::new(100.0, 100.0));
        pc.element_data_mut().set_direction_instantly(4);
        pc.actor_data_mut().unwrap().action_state = ActionState::Waiting;
        let stale = crate::bow_shot::compute_shield_obstacle(
            MapPoint::new(-50.0, 100.0),
            0.0,
            4,
            &crate::bow_shot::shield_params_for_pc(false),
        );
        pc.actor_data_mut().unwrap().shield_obstacle = Some(stale);
        let owner = engine.add_entity(pc);

        let mut movement = SequenceElement::new_movement(
            1,
            Command::Move,
            Some(owner),
            OrderType::WalkingWithShield,
        );
        movement
            .orders
            .push_back(Order::test_new(OrderType::WalkingWithShield, 140.0, 100.0));
        let sequence = engine.orders.sequence_manager.launch_element(movement);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);
        engine.set_actors_frozen(true);

        engine.tick_entity_movement(&crate::sim_rng::test_context(), &assets);

        let entity = engine.get_entity(owner).unwrap();
        assert_eq!(
            entity.element_data().position_map(),
            MapPoint::new(100.0, 100.0)
        );
        assert_eq!(
            entity.actor_data().unwrap().action_state,
            ActionState::MovingShield,
            "FrozenAll suppresses sprite motion, but not the PC WalkingWithShield Execute state stamp"
        );
        let actual = entity
            .actor_data()
            .unwrap()
            .shield_obstacle
            .as_ref()
            .unwrap();
        let expected = crate::bow_shot::compute_shield_obstacle(
            MapPoint::new(100.0, 100.0),
            0.0,
            4,
            &crate::bow_shot::shield_params_for_pc(false),
        );
        assert_eq!(actual.box_3d_min, expected.box_3d_min);
        assert_eq!(actual.box_3d_max, expected.box_3d_max);
    }

    fn install_blocked_upright_movement(
        action: OrderType,
        initial_action_state: ActionState,
    ) -> (EngineInner, EntityId, crate::sequence::SequenceId) {
        let mut engine = EngineInner::new();
        let start = MapPoint::new(100.0, 100.0);
        let destination = MapPoint::new(140.0, 100.0);
        let script = SpriteScript {
            action_id: action as u16,
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
        conversion[action as usize] = 0;
        let mut element = ElementData {
            kind: ElementKind::ActorPc,
            active: true,
            posture: Posture::Upright,
            sprite: crate::sprite::Sprite::new(
                std::sync::Arc::new(vec![script; 16]),
                std::sync::Arc::new(conversion),
            ),
            ..ElementData::default()
        };
        element.sprite.position_iface.set_anti_collision_on(false);
        element.set_position_map(start);
        let owner = engine.add_entity(Entity::Pc(ActorPc {
            element,
            actor: ActorData {
                action_state: initial_action_state,
                ..ActorData::default()
            },
            human: HumanData::default(),
            pc: PcData::default(),
        }));

        let order_id = engine.orders.allocate_order_id();
        let mut movement = SequenceElement::new_movement(1, Command::Move, Some(owner), action);
        movement.priority = SequencePriority::Normal;
        movement
            .orders
            .push_back(Order::new(action, destination.x, destination.y, order_id));
        let sequence = engine.orders.sequence_manager.launch_element(movement);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);
        let owner_entity = engine.get_entity_mut(owner).unwrap();
        owner_entity.actor_data_mut().unwrap().active_movement = ActiveMovement::new(sequence, 0);
        owner_entity
            .element_data_mut()
            .sprite
            .last_processed_order_id = order_id.get();
        let position_iface = owner_entity.position_iface_mut();
        position_iface.set_map_goal(destination);
        position_iface.compute_increment_all(true);
        position_iface.blocked_count = 51;

        (engine, owner, sequence)
    }

    fn assert_blocked_upright_movement_state(
        action: OrderType,
        initial_action_state: ActionState,
        expected_action_state: ActionState,
    ) {
        let (mut engine, owner, sequence) =
            install_blocked_upright_movement(action, initial_action_state);

        engine.tick_entity_movement(
            &crate::sim_rng::test_context(),
            &assets_with_test_pc_profile(),
        );

        let entity = engine.get_entity(owner).unwrap();
        assert_eq!(
            entity.actor_data().unwrap().action_state,
            expected_action_state
        );
        assert_eq!(
            entity.actor_data().unwrap().active_movement,
            ActiveMovement::none(),
            "the aborted movement tracker must still detach"
        );
        assert_eq!(entity.position_iface().blocked_count, 0);
        assert!(entity.position_iface().box_blocked.0.is_none());
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .unwrap()
                .state,
            SequenceState::Impossible,
            "Actor::Hourglass must still reject the blocked element"
        );
    }

    #[test]
    fn blocked_walking_upright_retains_moving_state_while_tearing_down_movement() {
        assert_blocked_upright_movement_state(
            OrderType::WalkingUpright,
            ActionState::Moving,
            ActionState::Moving,
        );
    }

    #[test]
    fn blocked_running_upright_still_applies_its_unconditional_moving_fast_state() {
        assert_blocked_upright_movement_state(
            OrderType::RunningUpright,
            ActionState::Waiting,
            ActionState::MovingFast,
        );
    }

    fn install_sword_movement_for_kind(
        force: bool,
        soldier: bool,
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

        let walking_script = SpriteScript {
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
        let directional_script = |action: OrderType| SpriteScript {
            action_id: action as u16,
            action_done: 0,
            average_speed: 17.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 17,
            frame_ids: vec![1],
            delays: vec![0],
            distances: vec![17],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
        };
        let mut scripts = vec![walking_script; 16];
        scripts.extend(vec![
            directional_script(OrderType::WalkingBackwardsSword);
            16
        ]);
        scripts.extend(vec![directional_script(OrderType::StrafingRightSword); 16]);
        scripts.extend(vec![directional_script(OrderType::StrafingLeftSword); 16]);
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[OrderType::WalkingSword as usize] = 0;
        conversion[OrderType::WalkingBackwardsSword as usize] = 16;
        conversion[OrderType::StrafingRightSword as usize] = 32;
        conversion[OrderType::StrafingLeftSword as usize] = 48;
        element.sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(scripts),
            std::sync::Arc::new(conversion),
        );
        element.sprite.position_iface.set_anti_collision_on(false);
        element.set_position_map(start);

        let actor = ActorData {
            action_state: ActionState::MovingSword,
            ..ActorData::default()
        };
        let owner = if soldier {
            element.kind = ElementKind::ActorSoldier;
            let mut npc = NpcData::default();
            let mut enemy_ai = crate::ai_enemy::EnemyAi::new(0);
            enemy_ai.hth_weapon_id = 1;
            npc.ai_brain = crate::element::AiBrain::Enemy(Box::new(enemy_ai));
            engine.add_entity(Entity::Soldier(ActorSoldier {
                element,
                actor,
                human: HumanData::default(),
                npc,
                soldier: SoldierData::default(),
            }))
        } else {
            engine.add_entity(Entity::Pc(ActorPc {
                element,
                actor,
                human: HumanData::default(),
                pc: PcData {
                    life_points: 50,
                    ..PcData::default()
                },
            }))
        };

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

    fn install_sword_movement(
        force: bool,
    ) -> (
        EngineInner,
        EntityId,
        crate::sequence::SequenceId,
        std::num::NonZeroU32,
        MapPoint,
    ) {
        install_sword_movement_for_kind(force, false)
    }

    #[test]
    fn lowered_actor_still_aborts_unrewritten_resumed_sword_move() {
        let (mut engine, owner, movement_sequence, order_id, _start) =
            install_sword_movement(false);
        let movement = engine
            .orders
            .sequence_manager
            .get_element_mut(movement_sequence, 0)
            .unwrap();
        movement.command = Command::MoveOk;
        engine
            .get_entity_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .action_state = ActionState::Waiting;

        let aborted = engine.abort_orphaned_sword_movement(
            &crate::sim_rng::test_context(),
            &assets_with_test_pc_profile(),
            owner,
            MovementOwnerSelection {
                seq_id: movement_sequence,
                elem_idx: 0,
                order_id,
            },
        );

        assert!(
            aborted,
            "Human::Execute has no non-sword action-state exception for an unrewritten sword order"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(movement_sequence, 0)
                .unwrap()
                .state,
            SequenceState::Impossible,
            "the captured resumed MoveOk must be rejected after Execute returns ABORTED"
        );
    }

    #[test]
    fn evaluate_opponents_rewrite_survives_postpone_as_untranslated_upright_move() {
        let (mut engine, owner, movement_sequence, _order_id, _start) =
            install_sword_movement(false);
        let sim = crate::sim_rng::test_context();
        let assets = assets_with_test_pc_profile();

        engine.evaluate_opponents(&sim, &assets, owner);

        let quit_sequence = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .find_map(|sequence| {
                sequence.elements.first().and_then(|element| {
                    (element.owner == Some(owner) && element.command == Command::QuitSwordfight)
                        .then_some(sequence.id)
                })
            })
            .expect("EvaluateOpponents must register QuitSwordfight");
        engine.engine_postpone(quit_sequence, 0, movement_sequence, 0);

        let movement = engine
            .orders
            .sequence_manager
            .get_element(movement_sequence, 0)
            .expect("rewritten movement must remain registered behind QuitSwordfight");
        let SequenceElementData::Movement { action, .. } = movement.data else {
            panic!("rewritten movement changed data kind")
        };
        assert_eq!(action, OrderType::WalkingUpright);
        assert_eq!(movement.command, Command::Move);
        assert_eq!(movement.state, SequenceState::Postponed);
        assert!(
            movement.orders.is_empty(),
            "Original postponement deletes the old sword order so resume retranslates the rewritten upright action"
        );
    }

    #[test]
    fn blocked_terminal_sword_execute_marks_impossible_without_entering_waiting_sword() {
        let (mut engine, owner, movement_sequence, order_id, _start) = install_sword_movement(true);
        let mut opponent_element = ElementData {
            kind: ElementKind::ActorPc,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        opponent_element.set_position_map(MapPoint::new(100.0, 50.0));
        let opponent = engine.add_entity(Entity::Pc(ActorPc {
            element: opponent_element,
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        }));
        engine
            .get_entity_mut(owner)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .opponents
            .push(opponent);

        let owner_entity = engine.get_entity_mut(owner).unwrap();
        // The replay's strafe order was already initialized before its
        // blocked counter crossed the threshold. A fresh fixture order calls
        // Sprite::initialize_motion_order and resets that counter, so install
        // the matching live sprite-order identity and cached trajectory first.
        owner_entity
            .element_data_mut()
            .sprite
            .last_processed_order_id = order_id.get();
        let position_iface = owner_entity.position_iface_mut();
        position_iface.set_map_goal(MapPoint::new(140.0, 100.0));
        position_iface.compute_increment_all(true);
        position_iface.blocked_count = 51;

        engine.tick_entity_movement(
            &crate::sim_rng::test_context(),
            &assets_with_test_pc_profile(),
        );

        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .action_state,
            ActionState::MovingSword,
            "the Human Execute ABORTED arm does not normalize a live sword-motion state"
        );
        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .element_data()
                .sprite
                .last_action,
            OrderType::StrafingLeftSword,
            "the regression drives the same terminal strafe family as Soldier 57"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(movement_sequence, 0)
                .unwrap()
                .state,
            SequenceState::Impossible,
            "Actor::Hourglass must still reject the blocked movement element"
        );
    }

    #[test]
    fn nonforced_sword_movement_without_opponents_aborts_before_motion_and_quits_once() {
        let (mut engine, owner, movement_sequence, order_id, start) = install_sword_movement(false);
        let sim = crate::sim_rng::test_context();
        let assets = assets_with_test_pc_profile();

        engine.tick_entity_movement(&sim, &assets);

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
        let quit_elements = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .filter(|element| {
                element.owner == Some(owner) && element.command == Command::QuitSwordfight
            })
            .collect::<Vec<_>>();
        assert_eq!(
            quit_elements.len(),
            1,
            "one rejected Execute invocation must launch exactly one QuitSwordfight"
        );
        assert_eq!(
            quit_elements[0].state,
            SequenceState::Todo,
            "Human::Execute registers QuitSwordfight; the later manager Hourglass owns Actor::Instruct"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .current_element_for_actor(owner),
            None,
            "the rejected movement is already gone but deferred QuitSwordfight is not selected until manager dispatch"
        );

        engine
            .get_entity_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .continuation
            .motion_state = crate::sprite::MotionState::Aborted;
        let mut display = crate::engine::HostDisplayState::default();
        engine.hourglass_phase_sequences(&sim, &mut display, &assets);

        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .continuation
                .motion_state,
            crate::sprite::MotionState::InProgress,
            "the later accepted Actor::Instruct must overwrite Execute's ABORTED result"
        );
        assert_eq!(
            engine.actor_order_type(owner),
            Some(OrderType::TransitionLoweringSword)
        );
    }

    #[test]
    fn npc_orphaned_sword_movement_stops_linked_turn_before_quit() {
        let (mut engine, owner, movement_sequence, _order_id, _start) =
            install_sword_movement_for_kind(false, true);
        let sim = crate::sim_rng::test_context();
        let assets = assets_with_test_pc_profile();
        engine
            .get_entity_mut(owner)
            .unwrap()
            .position_iface_mut()
            .set_direction_instantly(crate::position_interface::Direction::from_raw(10));

        let mut turn = SequenceElement::new_generic(1, Command::Turn, Some(owner));
        turn.priority = SequencePriority::Normal;
        turn.set_property(
            crate::sequence::Field::Direction,
            crate::sequence::FieldValue::Integer(9),
        );
        let turn_sequence = engine.orders.sequence_manager.launch_element(turn);
        engine
            .orders
            .sequence_manager
            .postpone_element(turn_sequence, 0);
        engine
            .orders
            .sequence_manager
            .get_element_mut(movement_sequence, 0)
            .unwrap()
            .cross_postponed = Some((turn_sequence, 0));

        let unrelated_sequence = engine
            .orders
            .sequence_manager
            .launch_element(SequenceElement::new(1, Command::LookLeft, Some(owner)));

        let ((), cards) = crate::engine::soldier_helpers::capture_condolation_cards(|| {
            engine.tick_entity_movement(&sim, &assets);
        });

        assert_eq!(
            cards
                .iter()
                .filter(|(card_owner, _)| *card_owner == owner)
                .map(|(_, command)| *command)
                .take(2)
                .collect::<Vec<_>>(),
            vec![Command::Move, Command::Turn],
            "Movement::StopMovement must deliver the selected movement card before base Stop reaches the linked Turn"
        );

        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(turn_sequence, 0)
                .unwrap()
                .state,
            SequenceState::Interrupted,
            "Stop(Injury) must close the linked Turn before QuitSwordfight is registered"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(movement_sequence, 0)
                .unwrap()
                .cross_postponed,
            None,
            "the interrupted Turn must not remain inheritable by QuitSwordfight"
        );
        assert!(
            engine
                .orders
                .sequence_manager
                .is_registered_to_go(unrelated_sequence, 0),
            "the exact-root stop must preserve unrelated pending owner work"
        );
        let quit = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .find(|element| {
                element.owner == Some(owner) && element.command == Command::QuitSwordfight
            })
            .expect("the orphan guard must register QuitSwordfight");
        assert_eq!(
            quit.cross_postponed, None,
            "QuitSwordfight must not inherit the interrupted Turn"
        );
        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .position_iface()
                .get_direction_goal()
                .as_u8(),
            10,
            "an unexecuted direction-9 Turn must not overwrite the live direction goal"
        );
        let events = engine
            .get_entity(owner)
            .unwrap()
            .ai_controller()
            .unwrap()
            .ai_log
            .iter()
            .filter(|entry| entry.line_type == crate::ai::LogLineType::Event)
            .map(|entry| entry.info)
            .collect::<Vec<_>>();
        let turn_card = events
            .iter()
            .position(|event| *event == crate::ai::StimulusType::EventDone as u16)
            .expect("interrupting the linked Turn must synchronously send its condolence card");
        let quit_event = events
            .iter()
            .position(|event| *event == crate::ai::StimulusType::EventQuitSwordfight as u16)
            .expect("the orphan guard must notify the soldier brain about quitting");
        assert!(
            turn_card < quit_event,
            "the Turn condolence callback must close before EVENT_QUIT_SWORDFIGHT"
        );
    }

    #[test]
    fn pc_pinch_abort_cancels_terminal_pop_before_impossible() {
        let (mut engine, _owner, movement_sequence, _order_id, _start) =
            install_sword_movement(false);
        let unrelated = crate::sequence::SequenceId(movement_sequence.0 + 1);
        let mut order_pops = vec![(movement_sequence, 0), (unrelated, 0)];

        cancel_aborted_order_pop(&mut order_pops, movement_sequence, 0);
        assert_eq!(order_pops, vec![(unrelated, 0)]);

        engine
            .orders
            .sequence_manager
            .element_impossible(movement_sequence, 0);
        for (seq_id, elem_idx) in order_pops {
            if engine
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .is_some()
            {
                engine.do_next_order(seq_id, elem_idx);
            }
        }
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(movement_sequence, 0)
                .unwrap()
                .state,
            SequenceState::Impossible,
            "the PC Execute ABORTED result must not be overwritten by the nested motion's queued TERMINATED pop"
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
            owner_entity.element_data().sprite.last_action,
            OrderType::WalkingSword,
            "a non-soldier without opponents takes FaceOpponent's explicit WalkingSword fallback, not a directional row computed from a self-position sentinel"
        );
        assert_eq!(
            owner_entity.element_data().position_map(),
            MapPoint::new(start.x + 6.0, start.y),
            "the WalkingSword frame distance must be used instead of an accidental backward/strafe distance"
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

    #[test]
    fn sword_movement_with_colocated_opponent_preserves_zero_facing_vector() {
        let (mut engine, owner, movement_sequence, _order_id, _start) =
            install_sword_movement(true);

        engine
            .orders
            .sequence_manager
            .get_element_mut(movement_sequence, 0)
            .and_then(|element| element.orders.front_mut())
            .expect("test movement keeps its selected order")
            .compute_direction = false;

        {
            let owner_element = engine.get_entity_mut(owner).unwrap().element_data_mut();
            owner_element.set_direction_instantly(7);
            owner_element.set_direction_goal(9);
        }

        let mut opponent_element = ElementData {
            kind: ElementKind::ActorPc,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        opponent_element.set_position(
            engine
                .get_entity(owner)
                .expect("test owner exists")
                .element_data()
                .position(),
        );
        let opponent = engine.add_entity(Entity::Pc(ActorPc {
            element: opponent_element,
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData {
                life_points: 50,
                ..PcData::default()
            },
        }));
        engine
            .get_entity_mut(owner)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .opponents
            .push(opponent);

        engine.tick_entity_movement(
            &crate::sim_rng::test_context(),
            &assets_with_test_pc_profile(),
        );

        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .element_data()
                .sprite
                .last_action,
            OrderType::WalkingBackwardsSword,
            "FaceOpponent passes a co-located opponent's literal zero vector to Angle, which resolves to PI"
        );
        let owner_element = engine.get_entity(owner).unwrap().element_data();
        assert_eq!(
            owner_element
                .sprite
                .position_iface
                .get_direction_goal()
                .as_u8(),
            0,
            "FaceOpponent passes its literal zero vector through GetSector0to15 and SetDirection"
        );
        assert_eq!(
            owner_element.direction(),
            6,
            "Turn must rotate one step from direction 7 toward the zero-vector goal 0"
        );
    }

    #[test]
    fn combat_seek_applies_face_opponent_and_perform_seek_turns() {
        let (mut engine, owner, movement_sequence, _order_id, start) = install_sword_movement(true);
        let (face_x, face_y) = crate::element::direction_vector_16(11);
        let opponent_position = MapPoint::new(
            start.x + face_x * 30.0,
            start.y + face_y * crate::position_interface::ASPECT_RATIO * 30.0,
        );
        assert_eq!(
            crate::position_interface::vector_to_sector_0_to_15_iso(
                opponent_position.x - start.x,
                opponent_position.y - start.y,
            ),
            11,
        );

        let mut opponent_element = ElementData {
            kind: ElementKind::ActorPc,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        opponent_element.set_position_map(opponent_position);
        let opponent = engine.add_entity(Entity::Pc(ActorPc {
            element: opponent_element,
            actor: ActorData::default(),
            human: HumanData {
                opponents: vec![owner],
                ..HumanData::default()
            },
            pc: PcData {
                life_points: 50,
                ..PcData::default()
            },
        }));

        {
            let entity = engine.get_entity_mut(owner).unwrap();
            entity.human_data_mut().unwrap().opponents.push(opponent);
            let actor = entity.actor_data_mut().unwrap();
            actor.seek_target = Some(opponent);
            actor.last_seek_target_position = opponent_position;
            actor.seek_distance = 0.0;
            let position = entity.position_iface_mut();
            let mut state = position.v48_serialized_state();
            state.direction = crate::position_interface::Direction::from_raw(10);
            state.direction_goal = crate::position_interface::Direction::from_raw(11);
            state.anti_collision_on = false;
            state.deviated = true;
            state.direction_count = 0;
            position.restore_v48_serialized_state(state);
        }
        let movement = engine
            .orders
            .sequence_manager
            .get_element_mut(movement_sequence, 0)
            .unwrap();
        let SequenceElementData::Movement { flags, element, .. } = &mut movement.data else {
            unreachable!("fixture movement changed data kind")
        };
        flags.insert(MoveFlags::SEEK);
        *element = Some(opponent);

        let sim = crate::sim_rng::test_context();
        let assets = assets_with_test_pc_profile();
        engine.tick_entity_movement(&sim, &assets);

        let first = engine.get_entity(owner).unwrap().position_iface();
        assert_eq!(first.get_direction().as_u8(), 10);
        assert_eq!(first.v48_serialized_state().direction_count, 2);

        engine.tick_entity_movement(&sim, &assets);

        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .position_iface()
                .get_direction()
                .as_u8(),
            11,
            "FaceOpponent's first Turn must rotate once the prior frame's two-call anti-vibration count is stable"
        );
    }

    #[test]
    fn postponed_forced_move_resuming_after_sword_lowered_walks_upright() {
        let mut engine = EngineInner::new();
        let start = MapPoint::new(100.0, 100.0);
        let destination = MapPoint::new(140.0, 100.0);
        let mut element = ElementData {
            kind: ElementKind::ActorPc,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        };
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
                // QuitSwordfight's lowering animation has completed before
                // the postponed Move is instructed again.
                action_state: ActionState::Waiting,
                ..ActorData::default()
            },
            human: HumanData::default(),
            pc: PcData {
                life_points: 50,
                ..PcData::default()
            },
        }));

        let mut movement = SequenceElement::new_movement(
            1,
            Command::Move,
            Some(owner),
            OrderType::WalkingWithSword,
        );
        movement.priority = SequencePriority::Normal;
        movement.posture_after_transition = Posture::Upright;
        movement.action_state_after_transition = ActionState::Waiting;
        let SequenceElementData::Movement {
            destination: stored_destination,
            flags,
            ..
        } = &mut movement.data
        else {
            unreachable!("new_movement must create movement data")
        };
        *stored_destination = destination;
        *flags |= MoveFlags::FORCE_SWORD_MOVEMENT;
        let sequence = engine.orders.sequence_manager.launch_element(movement);

        assert!(matches!(
            engine.try_dispatch_move_path(
                &crate::sim_rng::test_context(),
                &LevelAssets::new(),
                owner,
                sequence,
                0,
                destination,
                OrderType::WalkingWithSword,
            ),
            MovePathOutcome::Success
        ));

        let movement = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .expect("dispatched postponed movement must remain registered");
        let SequenceElementData::Movement { action, .. } = &movement.data else {
            panic!("dispatched movement changed data kind")
        };
        assert_eq!(*action, OrderType::WalkingUpright);
        assert_eq!(
            movement.orders.front().map(|order| order.order_type),
            Some(OrderType::TransitionWaitingUprightWalkingUpright)
        );
        assert_eq!(
            movement.action_state_after_transition,
            ActionState::Waiting,
            "FORCE is an Execute guard, not a translated action-state override"
        );
        assert!(matches!(
            &movement.data,
            SequenceElementData::Movement { flags, .. }
                if flags.contains(MoveFlags::FORCE_SWORD_MOVEMENT)
        ));
        assert!(
            !is_sword_motion_context(
                ActionState::Waiting,
                None,
                OrderType::TransitionWaitingUprightWalkingUpright,
            ),
            "FORCE on the owning element must not reroute an ordinary transition through FaceOpponent"
        );
    }

    #[test]
    fn ordinary_successor_does_not_inherit_sword_movement_start_side_effects() {
        assert!(
            !is_sword_motion_context(
                ActionState::MovingSword,
                Some(OrderType::WalkingUpright),
                OrderType::WalkingUpright,
            ),
            "a concrete ordinary door successor must not re-enter Human's sword-facing Execute arm"
        );
        assert!(
            !executes_sword_movement_action(
                Some(OrderType::WalkingUpright),
                OrderType::WalkingUpright,
            ),
            "an ordinary door successor must execute the ordinary walking START arm"
        );
        assert!(executes_sword_movement_action(
            Some(OrderType::WalkingWithSword),
            OrderType::WalkingWithSword,
        ));
        assert!(executes_sword_movement_action(
            None,
            OrderType::RunningWithSword,
        ));
    }
}

#[cfg(test)]
mod movement_transition_state_tests {
    use super::*;
    use crate::element::{
        ActionState, ActiveDoorPass, ActorData, ActorPc, ActorSoldier, AiBrain, Camp, Command,
        ElementData, ElementKind, Entity, HumanData, NpcData, PcData, Posture, SoldierData,
    };
    use crate::order::{AiOrderIntent, Order};
    use crate::sequence::{
        MoveFlags, Sequence, SequenceElement, SequenceElementData, SequencePriority, SequenceState,
    };
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    #[test]
    fn map_exit_move_bypasses_ordinary_level_bounds_preflight() {
        fn make_owner(engine: &mut EngineInner) -> EntityId {
            let mut element = ElementData {
                kind: ElementKind::ActorSoldier,
                active: true,
                posture: Posture::Upright,
                ..ElementData::default()
            };
            element.set_position_map(MapPoint::new(90.0, 90.0));
            let mut npc = NpcData::default();
            npc.ai_brain = AiBrain::Enemy(Box::default());
            engine.add_entity(Entity::Soldier(ActorSoldier {
                element,
                actor: ActorData::default(),
                human: HumanData::default(),
                npc,
                soldier: SoldierData::default(),
            }))
        }

        let sim = crate::sim_rng::test_context();
        let assets = LevelAssets::new();
        let destination = crate::ai::Position {
            x: 100.0,
            y: 130.0,
            sector: crate::position_interface::SectorHandle::new(0),
            level: 0,
        };

        let mut map_exit = EngineInner::new();
        map_exit.feedback.cutscene_camera.level_size =
            crate::coordinates::MapSize::new(100.0, 100.0);
        let map_owner = make_owner(&mut map_exit);
        map_exit
            .get_entity_mut(map_owner)
            .unwrap()
            .ai_controller_mut()
            .unwrap()
            .run_to_map_exit(destination);
        map_exit.launch_pending_orders_for_npc_mode(&sim, &assets, map_owner, false);
        let launched = map_exit.drain_pending_move_requests_for_owner(&sim, map_owner);
        assert_eq!(launched.len(), 1, "RHMOVE_MAP must launch through PointOut");
        let element = map_exit
            .orders
            .sequence_manager
            .get_element(launched[0], 0)
            .expect("map-exit movement must remain registered for manager Hourglass");
        let SequenceElementData::Movement {
            destination: actual,
            flags,
            ..
        } = &element.data
        else {
            panic!("map-exit sequence must contain movement")
        };
        assert_eq!(*actual, MapPoint::new(destination.x, destination.y));
        assert!(flags.contains(MoveFlags::MAP));
        assert!(
            !map_exit
                .get_entity(map_owner)
                .unwrap()
                .ai_controller()
                .unwrap()
                .couldnt_reachpoint
        );

        for ordinary_destination in [MapPoint::new(100.0, 90.0), MapPoint::new(90.0, 130.0)] {
            let mut ordinary = EngineInner::new();
            ordinary.feedback.cutscene_camera.level_size =
                crate::coordinates::MapSize::new(100.0, 100.0);
            let ordinary_owner = make_owner(&mut ordinary);
            ordinary
                .get_entity_mut(ordinary_owner)
                .unwrap()
                .ai_controller_mut()
                .unwrap()
                .outbox
                .actor
                .orders
                .push(AiOrderIntent::new(
                    OrderType::RunningUpright,
                    ordinary_destination.x,
                    ordinary_destination.y,
                ));
            ordinary.launch_pending_orders_for_npc_mode(&sim, &assets, ordinary_owner, false);
            assert!(
                ordinary
                    .drain_pending_move_requests_for_owner(&sim, ordinary_owner)
                    .is_empty(),
                "ordinary GoTo at or outside the level must retain the existing rejection"
            );
            assert!(
                ordinary
                    .get_entity(ordinary_owner)
                    .unwrap()
                    .ai_controller()
                    .unwrap()
                    .couldnt_reachpoint
            );
        }
    }

    fn run_stale_sword_crenel_transition() -> (u8, u8) {
        use crate::fast_find_grid::GridSector;
        use crate::gate::{Door, DoorIndex, DoorType};
        use crate::sector::{LiftType, SectorNumber, SectorType};

        let mut engine = EngineInner::new();
        let transition = OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel;
        let script = SpriteScript {
            action_id: transition as u16,
            action_done: 7,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1; 8],
            delays: vec![0; 8],
            distances: vec![0; 8],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 8],
            sound_ids: vec![0; 8],
        };
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[transition as usize] = 0;
        let start = MapPoint::new(100.0, 100.0);
        let goal = MapPoint::new(108.0, 106.0);
        let lift_sector = SectorNumber::new(7);

        let mut opponent = Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                active: true,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        });
        opponent
            .element_data_mut()
            .set_position_map(MapPoint::new(120.0, 90.0));
        let opponent = engine.add_entity(opponent);

        let mut owner_entity = Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                active: true,
                posture: Posture::Upright,
                ..ElementData::default()
            },
            actor: ActorData {
                action_state: ActionState::MovingSword,
                execute_order_initialising: true,
                active_door_pass: Some(ActiveDoorPass {
                    door_index: DoorIndex(0),
                    direct: true,
                    position_direct: true,
                    steps: Default::default(),
                    triggers_fired: 0,
                    current_action: transition,
                    current_reverse: false,
                    saved_action_state: None,
                }),
                ..ActorData::default()
            },
            human: HumanData {
                opponents: vec![opponent],
                ..HumanData::default()
            },
            pc: PcData::default(),
        });
        owner_entity.element_data_mut().sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script; 16]),
            std::sync::Arc::new(conversion),
        );
        owner_entity.element_data_mut().set_position_map(start);
        owner_entity.element_data_mut().set_direction_instantly(2);
        owner_entity
            .element_data_mut()
            .set_sector(crate::position_interface::SectorHandle::new(7));
        owner_entity
            .position_iface_mut()
            .set_anti_collision_on(false);
        let owner = engine.add_entity(owner_entity);

        {
            let level = std::sync::Arc::make_mut(&mut engine.world.fast_grid_mut().level);
            level.sector_number_map.insert(lift_sector, 0);
            level.sectors.push(GridSector {
                points: Vec::new(),
                bounding_box: crate::coordinates::MapBBox::new(),
                sector_type: SectorType::LIFT,
                layer: 0,
                sector_number: lift_sector,
                door_index: Some(0),
                lift_type: Some(LiftType::Wall),
                lift_direction: 15,
                force_crouched: false,
                building_index: None,
                low_exit_point: Some(start),
                high_exit_point: Some(goal),
                lowest_door_index: Some(0),
                jump_line_indices: Vec::new(),
                gate_indices: Vec::new(),
                underlying_sector: None,
            });
        }
        engine.script_domains.interactables.doors.push(Door {
            door_type: DoorType::LiftHighCrenel,
            sector_in: lift_sector,
            point_in: goal,
            ..Door::default()
        });

        let order_id = engine.orders.allocate_order_id();
        let mut movement = SequenceElement::new_movement(
            1,
            Command::PassDoor,
            Some(owner),
            OrderType::WalkingWithSword,
        );
        movement
            .orders
            .push_back(Order::new(transition, goal.x, goal.y, order_id));
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

        let _ = engine.tick_entity_movement_owner(
            &crate::sim_rng::test_context(),
            &LevelAssets::new(),
            owner,
            Some(MovementOwnerSelection {
                seq_id: sequence,
                elem_idx: 0,
                order_id,
            }),
        );
        let pi = engine.get_entity(owner).unwrap().position_iface();
        (pi.get_direction().as_u8(), pi.get_direction_goal().as_u8())
    }

    #[test]
    fn crenel_transition_does_not_inherit_stale_sword_facing_context() {
        assert!(matches!(
            ActionState::MovingSword,
            ActionState::MovingSword | ActionState::MovingFastSword
        ));
        assert!(!order_uses_distance_motion(
            OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel
        ));
        assert!(
            climb_lift_type(OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel).is_some()
        );
        assert!(
            !is_sword_motion_context(
                ActionState::MovingSword,
                Some(OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel),
                OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel,
            ),
            "the base Actor crenel transition must retain its lift-facing goal instead of entering Human::FaceOpponent"
        );
        let (direction, goal) = run_stale_sword_crenel_transition();
        assert_eq!(
            direction, 3,
            "the trace-shaped Execute must turn toward the climb goal, from direction 2 to 3"
        );
        assert_ne!(
            goal, 1,
            "the opponent-facing writer selected by the old classification must not replace the climb trajectory"
        );
    }

    #[test]
    fn door_distance_successor_dispatches_its_literal_non_sword_action() {
        assert!(order_uses_distance_motion(OrderType::WalkingUpright));
        assert!(
            !is_sword_motion_context(
                ActionState::MovingSword,
                Some(OrderType::WalkingUpright),
                OrderType::WalkingUpright,
            ),
            "PassDoor's concrete WalkingUpright order enters base Actor::Execute"
        );
        assert!(
            is_sword_motion_context(ActionState::MovingSword, None, OrderType::WalkingUpright,),
            "ordinary movement still receives the translation-time sword rewrite"
        );
        assert!(is_sword_motion_context(
            ActionState::Waiting,
            None,
            OrderType::WalkingWithSword,
        ));
        assert!(!is_sword_motion_context(
            ActionState::MovingSword,
            None,
            OrderType::ClimbingWallDown,
        ));
        assert!(!is_sword_motion_context(
            ActionState::MovingSword,
            None,
            OrderType::WalkingCrouched,
        ));
    }

    #[test]
    fn stale_sword_state_does_not_face_opponent_before_plain_door_walk() {
        use crate::gate::{Door, DoorIndex};

        let mut engine = EngineInner::new();
        let start = MapPoint::new(100.0, 100.0);
        let destination = MapPoint::new(108.0, 106.0);
        let action = OrderType::WalkingUpright;
        let script = SpriteScript {
            action_id: action as u16,
            action_done: 1,
            average_speed: 1.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 16,
            frame_ids: vec![1, 2],
            delays: vec![0; 2],
            distances: vec![1; 2],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 2],
            sound_ids: vec![0; 2],
        };
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[action as usize] = 0;

        // The opponent lies in sector 0. The translated door step, however,
        // inherited sector 15 from the preceding door route and must execute
        // Actor's literal WALKING_UPRIGHT arm before PerformMotion replaces
        // the trajectory cache for the new destination.
        let mut opponent = Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                active: true,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        });
        opponent
            .element_data_mut()
            .set_position_map(MapPoint::new(100.0, 80.0));
        let opponent = engine.add_entity(opponent);

        let mut owner_entity = Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                active: true,
                posture: Posture::Upright,
                ..ElementData::default()
            },
            actor: ActorData {
                action_state: ActionState::MovingSword,
                execute_order_initialising: true,
                active_door_pass: Some(ActiveDoorPass {
                    door_index: DoorIndex(0),
                    direct: true,
                    position_direct: true,
                    steps: Default::default(),
                    triggers_fired: 0,
                    current_action: action,
                    current_reverse: false,
                    saved_action_state: None,
                }),
                ..ActorData::default()
            },
            human: HumanData {
                opponents: vec![opponent],
                ..HumanData::default()
            },
            pc: PcData::default(),
        });
        owner_entity.element_data_mut().sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script; 16]),
            std::sync::Arc::new(conversion),
        );
        owner_entity.element_data_mut().set_position_map(start);
        owner_entity.element_data_mut().set_direction_instantly(0);
        owner_entity.element_data_mut().set_direction_goal(15);
        owner_entity
            .position_iface_mut()
            .set_anti_collision_on(false);
        let owner = engine.add_entity(owner_entity);
        engine
            .script_domains
            .interactables
            .doors
            .push(Door::default());

        let order_id = engine.orders.allocate_order_id();
        let mut movement = SequenceElement::new_movement(1, Command::PassDoor, Some(owner), action);
        let mut order = Order::new(action, destination.x, destination.y, order_id);
        order.compute_direction = false;
        movement.orders.push_back(order);
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

        let _ = engine.tick_entity_movement_owner(
            &crate::sim_rng::test_context(),
            &LevelAssets::new(),
            owner,
            Some(MovementOwnerSelection {
                seq_id: sequence,
                elem_idx: 0,
                order_id,
            }),
        );
        let sprite = &engine.get_entity(owner).unwrap().element_data().sprite;
        assert_eq!(sprite.position_iface.get_direction().as_u8(), 15);
        assert_eq!(sprite.current_row, 15);
    }

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
        let first_tick_forecast = engine
            .get_entity(owner)
            .unwrap()
            .position_iface()
            .get_forecasted_movement();
        assert_ne!(
            first_tick_forecast,
            crate::coordinates::WorldVec3D::ZERO,
            "a moving startup transition must restore the forecast that its action change reset"
        );
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

    #[test]
    fn exhausted_stop_transition_clears_goal_before_queued_point_goto_promotion() {
        let mut engine = EngineInner::new();
        let old_goal = MapPoint::new(263.0, 794.0);
        let new_goal = MapPoint::new(363.0, 794.0);
        let stop_transition = OrderType::TransitionRunningUprightWaitingUpright;
        let start_transition = OrderType::TransitionWaitingUprightRunningUpright;
        let script = |action: OrderType| SpriteScript {
            action_id: action as u16,
            action_done: 1,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1, 2],
            delays: vec![0; 2],
            distances: vec![0; 2],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 2],
            sound_ids: vec![0; 2],
        };
        let moving_script = SpriteScript {
            action_id: OrderType::RunningUpright as u16,
            action_done: 1,
            average_speed: 1.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 2,
            frame_ids: vec![1, 2],
            delays: vec![0; 2],
            distances: vec![1; 2],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 2],
            sound_ids: vec![0; 2],
        };
        let mut scripts = Vec::with_capacity(48);
        scripts.extend(vec![script(stop_transition); 16]);
        scripts.extend(vec![script(start_transition); 16]);
        scripts.extend(vec![moving_script; 16]);
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[stop_transition as usize] = 0;
        conversion[start_transition as usize] = 16;
        conversion[OrderType::RunningUpright as usize] = 32;

        let mut element = ElementData {
            kind: ElementKind::ActorSoldier,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        element.sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(scripts),
            std::sync::Arc::new(conversion),
        );
        element
            .sprite
            .position_iface
            .set_move_box(crate::coordinates::MoveBox::from_coords(
                -4.0, -4.0, 4.0, 4.0,
            ));
        element.sprite.position_iface.set_anti_collision_on(false);
        element.set_position_map(MapPoint::new(300.0, 794.0));
        let mut npc = NpcData::default();
        npc.ai_brain = AiBrain::Enemy(Box::default());
        let owner = engine.add_entity(Entity::Soldier(ActorSoldier {
            element,
            actor: ActorData {
                action_state: ActionState::MovingFast,
                ..ActorData::default()
            },
            human: HumanData::default(),
            npc,
            soldier: SoldierData::default(),
        }));

        let mut outgoing = SequenceElement::new_movement(
            1,
            Command::MoveOk,
            Some(owner),
            OrderType::RunningUpright,
        );
        outgoing.priority = SequencePriority::Normal;
        outgoing.orders.push_back(Order::new(
            OrderType::RunningUpright,
            old_goal.x,
            old_goal.y,
            engine.orders.allocate_order_id(),
        ));
        let outgoing_sequence = engine.orders.sequence_manager.launch_element(outgoing);
        engine
            .orders
            .sequence_manager
            .element_in_progress(outgoing_sequence, 0);
        {
            let entity = engine.get_entity_mut(owner).unwrap();
            entity.actor_data_mut().unwrap().active_movement =
                ActiveMovement::new(outgoing_sequence, 0);
            entity.position_iface_mut().set_map_goal(old_goal);
            entity.ai_controller_mut().unwrap().outbox.actor.halt = true;
        }

        let sim = crate::sim_rng::test_context();
        let assets = LevelAssets::new();
        engine.launch_pending_orders_for_npc_mode(&sim, &assets, owner, false);

        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .position_iface()
                .map_goal(),
            old_goal,
            "StopAll retains the selected movement goal while its stop transition is live"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(outgoing_sequence, 0)
                .unwrap()
                .current_order()
                .unwrap()
                .order_type,
            stop_transition
        );
        // Exercise the source-relevant postponed-successor boundary explicitly:
        // once StopAll has built its live StopMovement transition, keep that
        // transition authoritative while the ordinary point GoTo is instructed.
        // The terminal DoNextOrder below must therefore release the GoTo
        // synchronously, rather than letting the fixture interrupt the stop
        // transition before there is an exhaustion boundary to test.
        engine
            .orders
            .sequence_manager
            .get_element_mut(outgoing_sequence, 0)
            .unwrap()
            .priority = SequencePriority::Script;

        {
            let ai = engine
                .get_entity_mut(owner)
                .unwrap()
                .ai_controller_mut()
                .unwrap();
            let mut goto = AiOrderIntent::new(OrderType::RunningUpright, new_goal.x, new_goal.y);
            goto.move_flags = MoveFlags::STRAIGHT.bits() as u16;
            ai.outbox.actor.orders.push(goto);
        }
        engine.launch_pending_orders_for_npc_mode(&sim, &assets, owner, false);
        let mut display = crate::engine::HostDisplayState::default();
        engine.hourglass_phase_sequences(&sim, &mut display, &assets);

        // `engine_postpone` intentionally drops the ordinary handoff cache;
        // model the later queue-time replacement snapshot that exposed the
        // replay bug only after the real GoTo has passed through Instruct and
        // become the outgoing transition's postponed successor.
        let outgoing_element_id = engine
            .orders
            .sequence_manager
            .get_element(outgoing_sequence, 0)
            .unwrap()
            .id;
        let replacement_handle = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .find(|element| {
                element.owner == Some(owner)
                    && element.id != outgoing_element_id
                    && element.data.is_movement()
            })
            .map(|element| element.id)
            .expect("queued point GoTo must register a replacement movement");
        let (replacement_sequence, replacement_index) = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| {
                sequence
                    .elements
                    .iter()
                    .enumerate()
                    .map(move |(index, element)| (sequence.id, index, element.id))
            })
            .find_map(|(sequence, index, id)| {
                (id == replacement_handle).then_some((sequence, index))
            })
            .expect("replacement movement handle must remain registered");
        engine
            .orders
            .sequence_manager
            .get_element_mut(replacement_sequence, replacement_index)
            .unwrap()
            .retained_movement_goal = Some(old_goal);

        let replacement = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .find(|element| {
                element.owner == Some(owner)
                    && element.id
                        != engine
                            .orders
                            .sequence_manager
                            .get_element(outgoing_sequence, 0)
                            .unwrap()
                            .id
                    && element.data.is_movement()
            })
            .expect("queued point GoTo must register a replacement movement");
        assert_eq!(
            replacement.retained_movement_goal,
            Some(old_goal),
            "the regression must carry the stale snapshot that could resurrect the exhausted goal"
        );
        assert_eq!(
            replacement.state,
            crate::sequence::SequenceState::Postponed,
            "the queued point GoTo must wait behind the live StopAll transition"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .current_element_for_actor(owner),
            Some((outgoing_sequence, 0)),
            "the stop transition must still own the actor before its terminal tick"
        );
        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .position_iface()
                .map_goal(),
            old_goal,
            "queuing the point GoTo does not end the live stop transition"
        );

        engine.tick_entity_movement(&sim, &assets);
        engine.tick_entity_movement(&sim, &assets);

        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(outgoing_sequence, 0)
                .unwrap()
                .state,
            crate::sequence::SequenceState::Terminated
        );
        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .position_iface()
                .map_goal(),
            MapPoint::ZERO,
            "the selected outgoing Move condolence must remain observable on the replacement-promotion frame"
        );

        engine.hourglass_phase_sequences(&sim, &mut display, &assets);
        // Consume the replacement's authored start transition. Stop at the
        // exact boundary where its point-movement order is selected but has
        // not yet executed; the following tick is its first Execute.
        for _ in 0..4 {
            let point_move_selected = engine
                .orders
                .sequence_manager
                .get_element(replacement_sequence, replacement_index)
                .and_then(|element| element.current_order())
                .is_some_and(|order| {
                    order.order_type == OrderType::RunningUpright
                        && (order.target_x - new_goal.x).abs() <= 0.02
                        && (order.target_y - new_goal.y).abs() <= 0.02
                });
            if point_move_selected {
                break;
            }
            engine.tick_entity_movement(&sim, &assets);
        }
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(replacement_sequence, replacement_index)
                .unwrap()
                .current_order()
                .unwrap()
                .order_type,
            OrderType::RunningUpright,
            "fixture must reach the selected point-movement order before its first Execute"
        );
        let selected_point_order = engine
            .orders
            .sequence_manager
            .get_element(replacement_sequence, replacement_index)
            .unwrap()
            .current_order()
            .unwrap();
        assert!(
            (selected_point_order.target_x - new_goal.x).abs() <= 0.02
                && (selected_point_order.target_y - new_goal.y).abs() <= 0.02,
            "fixture must select the authored destination endpoint, not the source-side transition-distance continuation"
        );
        engine.tick_entity_movement(&sim, &assets);
        let installed_goal = engine
            .get_entity(owner)
            .unwrap()
            .position_iface()
            .map_goal();
        assert!(
            (installed_goal.x - new_goal.x).abs() <= 0.02
                && (installed_goal.y - new_goal.y).abs() <= 0.02,
            "the promoted point GoTo installs its destination on first Execute"
        );
    }

    #[test]
    fn stale_nonselected_final_pop_does_not_clear_live_replacement_goal() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                active: true,
                posture: Posture::Upright,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        }));
        let stale_goal = MapPoint::new(263.0, 794.0);
        let replacement_goal = MapPoint::new(363.0, 794.0);

        let mut stale = SequenceElement::new_movement(
            1,
            Command::MoveOk,
            Some(owner),
            OrderType::RunningUpright,
        );
        stale.orders.push_back(Order::test_new(
            OrderType::RunningUpright,
            stale_goal.x,
            stale_goal.y,
        ));
        let stale_sequence = engine.orders.sequence_manager.launch_element(stale);
        engine
            .orders
            .sequence_manager
            .element_in_progress(stale_sequence, 0);

        let mut replacement = SequenceElement::new_movement(
            1,
            Command::MoveWaiting,
            Some(owner),
            OrderType::RunningUpright,
        );
        replacement.retained_movement_goal = Some(replacement_goal);
        replacement.orders.push_back(Order::test_new(
            OrderType::Freezing,
            replacement_goal.x,
            replacement_goal.y,
        ));
        let replacement_sequence = engine.orders.sequence_manager.launch_element(replacement);
        engine
            .orders
            .sequence_manager
            .set_translating_element(Some((
                owner,
                crate::sequence::SequenceElementRef::new(replacement_sequence, 0),
            )));
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .current_element_for_actor(owner),
            Some((replacement_sequence, 0)),
            "control requires the replacement to be authoritative before the stale pop drains"
        );
        engine
            .get_entity_mut(owner)
            .unwrap()
            .position_iface_mut()
            .set_map_goal(replacement_goal);
        let stale_orders_before_pop = engine
            .orders
            .sequence_manager
            .get_element(stale_sequence, 0)
            .unwrap()
            .orders
            .len();
        assert_eq!(
            stale_orders_before_pop, 1,
            "control requires a live final stale order for the queued pop"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(stale_sequence, 0)
                .unwrap()
                .state,
            crate::sequence::SequenceState::InProgress,
            "control requires a stale InProgress owner, not prior terminal teardown"
        );

        engine.pop_selected_movement_order(stale_sequence, 0);
        engine.orders.sequence_manager.set_translating_element(None);

        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(stale_sequence, 0)
                .unwrap()
                .orders
                .len(),
            stale_orders_before_pop,
            "the stale queued pop must not perform any further order teardown after replacement selection"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(replacement_sequence, 0)
                .unwrap()
                .retained_movement_goal,
            Some(replacement_goal),
            "a stale pop must not erase the authoritative replacement's retained goal"
        );
        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .position_iface()
                .map_goal(),
            replacement_goal,
            "a stale pop must leave the authoritative replacement goal untouched"
        );
    }

    #[test]
    fn terminal_reentrant_seek_advances_live_move_waiting_order() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                active: true,
                posture: Posture::Crouched,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        }));
        {
            let actor = engine
                .get_entity_mut(owner)
                .unwrap()
                .actor_data_mut()
                .unwrap();
            actor.seek_target = Some(owner);
            actor.post_seek_sequence = Some(Box::new(Sequence::new()));
        }
        let mut outgoing = SequenceElement::new_movement(
            1,
            Command::MoveOk,
            Some(owner),
            OrderType::WalkingCrouched,
        );
        outgoing.orders.push_back(Order::test_new(
            OrderType::TransitionWalkingCrouchedWaitingCrouched,
            867.70776,
            2471.1958,
        ));
        let outgoing_sequence = engine.orders.sequence_manager.launch_element(outgoing);
        engine
            .orders
            .sequence_manager
            .element_in_progress(outgoing_sequence, 0);
        let mut waiting = SequenceElement::new_movement(
            1,
            Command::MoveWaiting,
            Some(owner),
            OrderType::WalkingCrouched,
        );
        waiting
            .orders
            .push_back(Order::test_new(OrderType::Freezing, 867.70776, 2471.1958));
        let sequence = engine.orders.sequence_manager.launch_element(waiting);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);
        let mut completed_parallel =
            SequenceElement::new(2, Command::SpeakHeroReachDestination, Some(owner));
        completed_parallel.state = SequenceState::Terminated;
        engine
            .orders
            .sequence_manager
            .get_sequence_mut(sequence)
            .unwrap()
            .elements
            .push(completed_parallel);
        engine
            .orders
            .sequence_manager
            .set_translating_element(Some((
                owner,
                crate::sequence::SequenceElementRef::new(sequence, 0),
            )));
        engine
            .get_entity_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .installed_order = Some(crate::element::InstalledActorOrder {
            order_id: engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .unwrap()
                .current_order()
                .unwrap()
                .order_id,
            order_type: OrderType::Freezing,
        });
        engine
            .orders
            .pending_path_requests
            .enqueue(PendingPathRequest::test_request(owner, sequence, 0));

        assert!(engine.live_pending_seek_freezing_order(owner));
        assert!(
            !engine.live_seek_has_completed_parallel_element(owner),
            "a completed later-level element must not consume an ordinary postponed Move"
        );
        engine
            .orders
            .sequence_manager
            .get_element_mut(sequence, 1)
            .unwrap()
            .command_level = 1;
        assert!(engine.live_seek_has_completed_parallel_element(owner));
        engine.advance_live_order_after_reentrant_seek(owner);
        engine.orders.sequence_manager.set_translating_element(None);

        let element = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .expect("terminated post-seek replacement remains inspectable");
        assert!(element.orders.is_empty());
        assert_eq!(element.state, SequenceState::Terminated);
        assert!(
            engine
                .get_entity(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .installed_order
                .is_none(),
            "the actor's live Freezing order is the one Hourglass advances"
        );
        assert!(
            engine.orders.pending_path_requests.ignore_next_path,
            "terminating the live MoveWaiting must retain and invalidate its pathfinder head"
        );
        assert_eq!(
            engine.orders.pending_path_requests.waiting.len(),
            1,
            "Original CancelPathRequest retains the logical head until its completion slot"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(outgoing_sequence, 0)
                .unwrap()
                .orders
                .len(),
            1,
            "the captured outgoing order remains stale; Hourglass advances the live replacement"
        );
    }

    #[test]
    fn terminal_pc_stop_transition_keeps_mouse_orientation_goal() {
        let mut engine = EngineInner::new();
        let transition = OrderType::TransitionWalkingUprightWaitingUpright;
        let script = SpriteScript {
            action_id: transition as u16,
            action_done: 1,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1, 2],
            delays: vec![0; 2],
            distances: vec![0; 2],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 2],
            sound_ids: vec![0; 2],
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
        element.set_position_map(MapPoint::new(100.0, 100.0));
        element.set_direction_instantly(6);
        let owner = engine.add_entity(Entity::Pc(ActorPc {
            element,
            actor: ActorData {
                action_state: ActionState::Waiting,
                ..ActorData::default()
            },
            human: HumanData::default(),
            pc: PcData::default(),
        }));

        let mut movement = SequenceElement::new_movement(
            1,
            Command::MoveOk,
            Some(owner),
            OrderType::WalkingUpright,
        );
        movement.priority = SequencePriority::Normal;
        movement.orders.clear();
        let order_id = engine.orders.allocate_order_id();
        movement
            .orders
            .push_back(Order::new(transition, 500.0, 428.0, order_id));
        let sequence = engine.orders.sequence_manager.launch_element(movement);
        let registered = engine.orders.sequence_manager.hourglass();
        assert_eq!(registered.len(), 1);
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

        let sim = crate::sim_rng::test_context();
        let assets = LevelAssets::new();
        engine.tick_entity_movement(&sim, &assets);
        assert_eq!(
            i16::from(
                engine
                    .get_entity(owner)
                    .unwrap()
                    .position_iface()
                    .get_direction_goal()
            ),
            6,
            "the stop transition must initially own the outgoing movement direction"
        );

        // PerformOrientation runs before the next engine frame and turns once;
        // the transition's Execute performs the second turn before terminating.
        let entity = engine.get_entity_mut(owner).unwrap();
        entity.element_data_mut().set_direction_goal(4);
        entity.position_iface_mut().turn();
        engine.tick_entity_movement(&sim, &assets);

        let entity = engine.get_entity(owner).unwrap();
        assert_eq!(i16::from(entity.position_iface().get_direction()), 4);
        assert_eq!(
            i16::from(entity.position_iface().get_direction_goal()),
            4,
            "terminal Move cleanup must not resurrect the outgoing movement facing"
        );
        assert_eq!(entity.position_iface().map_goal(), MapPoint::ZERO);
    }

    #[test]
    fn new_terminal_pc_stop_transition_replaces_stale_direction_goal() {
        let mut engine = EngineInner::new();
        let transition = OrderType::TransitionWalkingUprightWaitingUpright;
        let destination = MapPoint::new(101.0, 104.0);
        assert_eq!(
            vector_to_sector_0_to_15(destination.x - 100.0, destination.y - 100.0),
            7,
            "fixture movement vector must reproduce QuickSave r011's direction goal"
        );
        let script = SpriteScript {
            action_id: transition as u16,
            action_done: 0,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1],
            delays: vec![0],
            distances: vec![0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
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
        element.set_position_map(MapPoint::new(100.0, 100.0));
        element.sprite.position_iface.set_map_goal(destination);
        element.sprite.position_iface.compute_increment_all(true);
        element.set_direction_goal(0);
        let owner = engine.add_entity(Entity::Pc(ActorPc {
            element,
            actor: ActorData {
                action_state: ActionState::Waiting,
                ..ActorData::default()
            },
            human: HumanData::default(),
            pc: PcData::default(),
        }));

        let mut movement = SequenceElement::new_movement(
            1,
            Command::MoveOk,
            Some(owner),
            OrderType::WalkingUpright,
        );
        movement.priority = SequencePriority::Normal;
        movement.orders.clear();
        let order_id = engine.orders.allocate_order_id();
        movement.orders.push_back(Order::new(
            transition,
            destination.x,
            destination.y,
            order_id,
        ));
        let sequence = engine.orders.sequence_manager.launch_element(movement);
        let registered = engine.orders.sequence_manager.hourglass();
        assert_eq!(registered.len(), 1);
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

        let entity = engine.get_entity(owner).unwrap();
        assert_eq!(
            entity.element_data().sprite.last_processed_order_id,
            order_id.get(),
            "regression must execute the newly initialized transition"
        );
        assert_eq!(
            i16::from(entity.position_iface().get_direction_goal()),
            7,
            "new-order ComputeIncrementAll must replace rather than restore the stale goal"
        );
    }

    #[test]
    fn terminal_door_pass_goal_clear_follows_crossing_recompute() {
        let mut entity = Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                posture: Posture::Upright,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        });
        let destination = MapPoint::new(1447.3046, 620.1602);
        let terminal_position = MapPoint::new(1446.2887, 620.40155);
        {
            let pi = entity.position_iface_mut();
            pi.set_map_position(terminal_position);
            pi.set_map_goal(destination);
            // CrossElevationLine invalidates the cached trajectory before
            // Actor::CheckForLineCrossing rebuilds it against mpOrder's live
            // destination.
            pi.compute_increment_all(true);
        }
        let crossing_increment = entity.position_iface().get_increment_map();
        assert!(
            crossing_increment.x > 0.9 && crossing_increment.y < -0.2,
            "fixture must retain Save042's eastward terminal trajectory"
        );

        clear_terminal_door_pass_goal(&mut entity);

        assert_eq!(entity.position_iface().map_goal(), MapPoint::ZERO);
        assert_eq!(
            entity.position_iface().raw_increment_map(),
            crossing_increment,
            "terminal condolation clears the goal only after crossing has rebuilt the cached increment"
        );
        assert!(
            !entity.position_iface().is_increment_map_computed(),
            "clearing the completed goal must invalidate, but not overwrite, Original's cached increment"
        );
        let origin_length = (terminal_position.x * terminal_position.x
            + terminal_position.y * terminal_position.y)
            .sqrt();
        let origin_increment = MapVec::new(
            -terminal_position.x / origin_length,
            -terminal_position.y / origin_length,
        );
        assert_ne!(
            crossing_increment, origin_increment,
            "crossing must not rebuild the trajectory from the cleared (0, 0) sentinel"
        );
    }

    fn install_terminal_interaction_seek(
        command: Command,
        distance: f32,
    ) -> (EngineInner, EntityId, EntityId) {
        let mut engine = EngineInner::new();
        let transition = OrderType::TransitionWalkingUprightWaitingUpright;
        let script = SpriteScript {
            action_id: transition as u16,
            action_done: 1,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1, 2],
            delays: vec![0; 2],
            distances: vec![0; 2],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 2],
            sound_ids: vec![0; 2],
        };
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[transition as usize] = 0;
        conversion[OrderType::TransitionWaitingUprightWalkingUpright as usize] = 16;

        let mut owner_element = ElementData {
            kind: ElementKind::ActorPc,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        owner_element.sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script; 32]),
            std::sync::Arc::new(conversion),
        );
        owner_element
            .sprite
            .position_iface
            .set_move_box(crate::coordinates::MoveBox::from_coords(
                -4.0, -4.0, 4.0, 4.0,
            ));
        owner_element
            .sprite
            .position_iface
            .set_anti_collision_on(false);
        owner_element.set_sector(Some(
            crate::position_interface::SectorHandle::new(1).unwrap(),
        ));
        owner_element.set_position_map(MapPoint::new(100.0, 100.0));
        owner_element.set_direction_goal(7);
        let owner = engine.add_entity(Entity::Pc(ActorPc {
            element: owner_element,
            actor: ActorData {
                action_state: ActionState::Moving,
                seek_distance: 34.0,
                ..ActorData::default()
            },
            human: HumanData::default(),
            pc: PcData::default(),
        }));

        let mut target_element = ElementData {
            kind: ElementKind::ActorSoldier,
            active: true,
            posture: if command == Command::TieCmd {
                Posture::Lying
            } else {
                Posture::Upright
            },
            ..ElementData::default()
        };
        target_element.sprite.position_iface.set_move_box(
            crate::coordinates::MoveBox::from_coords(-4.0, -4.0, 4.0, 4.0),
        );
        target_element
            .sprite
            .position_iface
            .set_anti_collision_on(false);
        target_element.set_sector(Some(
            crate::position_interface::SectorHandle::new(1).unwrap(),
        ));
        target_element.set_position_map(MapPoint::new(100.0 + distance, 100.0));
        let target = engine.add_entity(Entity::Soldier(ActorSoldier {
            element: target_element,
            actor: ActorData::default(),
            human: HumanData {
                unconscious: command == Command::TieCmd,
                ..HumanData::default()
            },
            npc: NpcData::default(),
            soldier: SoldierData {
                cached_camp: Camp::Lacklandists,
                ..SoldierData::default()
            },
        }));

        let mut interaction = Sequence::new();
        interaction.append_element(SequenceElement::new_interaction(
            1,
            command,
            Some(owner),
            Some(target),
        ));
        let actor = engine
            .get_entity_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap();
        actor.seek_target = Some(target);
        actor.last_seek_target_position = MapPoint::new(100.0 + distance, 100.0);
        actor.post_seek_sequence = Some(Box::new(interaction));

        let mut movement = SequenceElement::new_movement(
            1,
            Command::MoveOk,
            Some(owner),
            OrderType::WalkingUpright,
        );
        movement.priority = SequencePriority::Normal;
        if let SequenceElementData::Movement {
            flags,
            element,
            sector,
            tolerance,
            destination,
            ..
        } = &mut movement.data
        {
            *flags = MoveFlags::SEEK;
            // Reproduce the copied terminal-transition shape from the
            // schema-14 controls: FinalTol no longer carries the movement
            // element target, while PerformSeek still owns seek_target on
            // ActorData.
            *element = None;
            *sector = crate::position_interface::SectorHandle::new(1);
            *tolerance = 34.0;
            *destination = MapPoint::new(100.0 + distance, 100.0);
        }
        movement.orders.clear();
        let order_id = engine.orders.allocate_order_id();
        movement
            .orders
            .push_back(Order::new(transition, 100.0, 100.0, order_id));
        let sequence = engine.orders.sequence_manager.launch_element(movement);
        let registered = engine.orders.sequence_manager.hourglass();
        assert_eq!(
            registered.len(),
            1,
            "fixture must consume its launch registration"
        );
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
        (engine, owner, target)
    }

    fn finish_terminal_seek_tick(engine: &mut EngineInner, owner: EntityId) {
        let sim = crate::sim_rng::test_context();
        let assets = LevelAssets::new();
        for _ in 0..64 {
            engine.tick_entity_movement(&sim, &assets);
            engine.hourglass_phase_sequences(
                &sim,
                &mut crate::engine::HostDisplayState::default(),
                &assets,
            );
            if engine
                .get_entity(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .post_seek_sequence
                .is_none()
            {
                return;
            }
        }
        panic!(
            "terminal seek transition did not finish: order={:?}, post_seek_present={}",
            engine.actor_order_type(owner),
            engine
                .get_entity(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .post_seek_sequence
                .is_some()
        );
    }

    #[test]
    fn terminal_pc_hit_seek_outside_init_range_aborts_without_visible_hit() {
        let (mut engine, owner, target) = install_terminal_interaction_seek(Command::HitCmd, 55.8);
        finish_terminal_seek_tick(&mut engine, owner);
        assert_ne!(engine.actor_order_type(owner), Some(OrderType::Hitting));
        let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
        assert!(actor.post_seek_sequence.is_none());
        assert!(actor.seek_target.is_none());
        assert!(actor.active_movement.sequence_id.is_none());
        let expected = vector_to_sector_0_to_15(
            engine.get_entity(target).unwrap().ground_position().x
                - engine.get_entity(owner).unwrap().ground_position().x,
            engine.get_entity(target).unwrap().ground_position().y
                - engine.get_entity(owner).unwrap().ground_position().y,
        );
        assert_eq!(
            i16::from(
                engine
                    .get_entity(owner)
                    .unwrap()
                    .position_iface()
                    .get_direction_goal(),
            ),
            expected
        );
    }

    #[test]
    fn terminal_pc_hit_seek_at_init_range_launches_hit_normally() {
        let (mut engine, owner, _target) = install_terminal_interaction_seek(Command::HitCmd, 40.0);
        finish_terminal_seek_tick(&mut engine, owner);
        assert_eq!(engine.actor_order_type(owner), Some(OrderType::Hitting));
    }

    #[test]
    fn terminal_non_seek_move_does_not_launch_stale_actor_post_seek() {
        let (mut engine, owner, _target) = install_terminal_interaction_seek(Command::HitCmd, 16.0);
        let (seq_id, elem_idx) = {
            let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
            (
                actor.active_movement.sequence_id.unwrap(),
                actor.active_movement.element_index,
            )
        };
        let SequenceElementData::Movement { flags, .. } = &mut engine
            .orders
            .sequence_manager
            .get_element_mut(seq_id, elem_idx)
            .expect("terminal movement fixture lost its selected element")
            .data
        else {
            panic!("terminal movement fixture lost movement data")
        };
        *flags = MoveFlags::empty();

        let sim = crate::sim_rng::test_context();
        let assets = LevelAssets::new();
        for _ in 0..64 {
            engine.tick_entity_movement(&sim, &assets);
            engine.hourglass_phase_sequences(
                &sim,
                &mut crate::engine::HostDisplayState::default(),
                &assets,
            );
            if engine
                .get_entity(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .active_movement
                .sequence_id
                .is_none()
            {
                break;
            }
        }

        let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
        assert!(actor.active_movement.sequence_id.is_none());
        assert!(
            actor.post_seek_sequence.is_some(),
            "ordinary Move must not consume stale actor-owned post-seek state"
        );
        assert_ne!(engine.actor_order_type(owner), Some(OrderType::Hitting));
    }

    #[test]
    fn looped_seek_stop_transition_rechecks_final_order_after_deleting_followers() {
        let (mut engine, owner, target) = install_terminal_interaction_seek(Command::HitCmd, 40.0);
        let (seq_id, elem_idx) = {
            let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
            (
                actor.active_movement.sequence_id.unwrap(),
                actor.active_movement.element_index,
            )
        };
        let transition = OrderType::TransitionWalkingUprightWaitingUpright;
        let order_ids = [
            engine.orders.allocate_order_id(),
            engine.orders.allocate_order_id(),
            engine.orders.allocate_order_id(),
        ];
        let element = engine
            .orders
            .sequence_manager
            .get_element_mut(seq_id, elem_idx)
            .unwrap();
        let SequenceElementData::Movement {
            element: seek_target,
            ..
        } = &mut element.data
        else {
            panic!("seek regression lost movement data")
        };
        *seek_target = Some(target);
        element.orders.clear();
        for (destination_x, order_id) in [110.0, 120.0, 140.0].into_iter().zip(order_ids) {
            element
                .orders
                .push_back(Order::new(transition, destination_x, 100.0, order_id));
        }
        assert_eq!(element.orders.len(), 3);
        assert_ne!(
            engine
                .get_entity(owner)
                .unwrap()
                .element_data()
                .position_map(),
            MapPoint::new(110.0, 100.0),
            "the first transition must loop before reaching its destination"
        );

        finish_terminal_seek_tick(&mut engine, owner);

        assert_eq!(
            engine.actor_order_type(owner),
            Some(OrderType::Hitting),
            "PerformSeek must observe that TillLastFrame deleted all followers and launch the attached post-seek"
        );
    }

    #[test]
    fn looped_seek_start_transition_refreshes_before_copied_stop_transition() {
        let (mut engine, owner, target) = install_terminal_interaction_seek(Command::HealCmd, 70.0);
        let (seq_id, elem_idx) = {
            let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
            (
                actor.active_movement.sequence_id.unwrap(),
                actor.active_movement.element_index,
            )
        };
        let stale_target = MapPoint::new(140.0, 100.0);
        {
            let actor = engine
                .get_entity_mut(owner)
                .unwrap()
                .actor_data_mut()
                .unwrap();
            actor.last_seek_target_position = stale_target;
            actor.seek_refresh_wait = 22;
            actor.wait_time = 22;
        }
        let order_ids = [
            engine.orders.allocate_order_id(),
            engine.orders.allocate_order_id(),
        ];
        let element = engine
            .orders
            .sequence_manager
            .get_element_mut(seq_id, elem_idx)
            .unwrap();
        let SequenceElementData::Movement {
            element: seek_target,
            destination,
            tolerance,
            ..
        } = &mut element.data
        else {
            panic!("seek regression lost movement data")
        };
        *seek_target = Some(target);
        *destination = stale_target;
        *tolerance = 17.0;
        element.orders.clear();
        element.orders.push_back(Order::new(
            OrderType::TransitionWaitingUprightWalkingUpright,
            120.0,
            100.0,
            order_ids[0],
        ));
        element.orders.push_back(Order::new(
            OrderType::TransitionWalkingUprightWaitingUpright,
            stale_target.x,
            stale_target.y,
            order_ids[1],
        ));

        let sim = crate::sim_rng::test_context();
        let assets = LevelAssets::new();
        for _ in 0..8 {
            engine.tick_entity_movement(&sim, &assets);
            if engine
                .get_entity(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .seek_refresh_wait
                == 25
            {
                break;
            }
        }

        let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
        assert_eq!(actor.seek_refresh_wait, 25);
        assert_eq!(actor.wait_time, 25);
        assert_eq!(
            actor.last_seek_target_position,
            engine
                .get_entity(target)
                .unwrap()
                .element_data()
                .position_map()
        );
        assert_eq!(actor.action_state, ActionState::Moving);
        assert_ne!(
            engine.actor_order_type(owner),
            Some(OrderType::TransitionWalkingUprightWaitingUpright),
            "PerformSeek must refresh before executing the copied stale stop transition"
        );
    }

    #[test]
    fn terminal_pc_hit_in_live_range_precedes_expired_stale_seek_refresh() {
        let (mut engine, owner, target) = install_terminal_interaction_seek(Command::HitCmd, 20.0);
        let (seq_id, elem_idx) = {
            let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
            (
                actor.active_movement.sequence_id.unwrap(),
                actor.active_movement.element_index,
            )
        };
        let target_position = engine
            .get_entity(target)
            .unwrap()
            .element_data()
            .position_map();
        let stale_position = MapPoint::new(target_position.x + 100.0, target_position.y);
        let SequenceElementData::Movement {
            element,
            destination,
            ..
        } = &mut engine
            .orders
            .sequence_manager
            .get_element_mut(seq_id, elem_idx)
            .unwrap()
            .data
        else {
            panic!("terminal seek fixture lost movement data")
        };
        *element = Some(target);
        *destination = stale_position;
        {
            let actor = engine
                .get_entity_mut(owner)
                .unwrap()
                .actor_data_mut()
                .unwrap();
            actor.seek_refresh_wait = 0;
            actor.wait_time = 0;
            actor.last_seek_target_position = stale_position;
        }

        let sim = crate::sim_rng::test_context();
        let assets = LevelAssets::new();
        assert!(
            !engine.tick_refresh_seek_for_owner(&sim, &assets, owner),
            "PerformSeek must observe the live in-range target before testing stale-route refresh"
        );
        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .seek_refresh_wait,
            0,
            "the suppressed refresh must not rearm the 25-frame timer"
        );

        finish_terminal_seek_tick(&mut engine, owner);
        assert_eq!(engine.actor_order_type(owner), Some(OrderType::Hitting));
    }

    #[test]
    fn terminal_pc_tie_seek_outside_init_range_publishes_tie_before_execute_validation() {
        let (mut engine, owner, _target) = install_terminal_interaction_seek(Command::TieCmd, 55.8);
        finish_terminal_seek_tick(&mut engine, owner);

        assert_eq!(engine.actor_order_type(owner), Some(OrderType::Tying));
        let entity = engine.get_entity(owner).unwrap();
        let actor = entity.actor_data().unwrap();
        assert!(actor.post_seek_sequence.is_none());
        assert!(actor.seek_target.is_none());
        assert!(actor.active_movement.sequence_id.is_none());
        assert_eq!(
            i16::from(entity.position_iface().get_direction_goal()),
            7,
            "Tie translation itself must not run the next Hourglass's initialization turn"
        );
    }

    #[test]
    fn terminal_pc_tie_seek_at_init_range_launches_tie_normally() {
        let (mut engine, owner, _target) = install_terminal_interaction_seek(Command::TieCmd, 40.0);
        finish_terminal_seek_tick(&mut engine, owner);
        assert_eq!(engine.actor_order_type(owner), Some(OrderType::Tying));
    }

    #[test]
    fn stopped_short_point_seek_does_not_launch_post_seek() {
        let (mut engine, owner, _target) = install_terminal_interaction_seek(Command::HitCmd, 40.0);
        let seek_sector = crate::position_interface::SectorHandle::new(2).unwrap();
        let (sequence_id, element_index) = {
            let actor = engine
                .get_entity_mut(owner)
                .unwrap()
                .actor_data_mut()
                .unwrap();
            actor.seek_target = None;
            actor.continuation.seek_to_point = true;
            actor.continuation.seek_layer = 0;
            actor.continuation.seek_sector =
                Some(crate::actor_state::ActorSeekSector::Position(seek_sector));
            let mut post_seek = Sequence::new();
            post_seek.append_element(SequenceElement::new(1, Command::DropAle, Some(owner)));
            actor.post_seek_sequence = Some(Box::new(post_seek));
            (
                actor.active_movement.sequence_id.unwrap(),
                actor.active_movement.element_index,
            )
        };
        let movement = engine
            .orders
            .sequence_manager
            .get_element_mut(sequence_id, element_index)
            .unwrap();
        let SequenceElementData::Movement { sector, .. } = &mut movement.data else {
            panic!("point-seek fixture must retain its movement element")
        };
        *sector = Some(seek_sector);

        let sim = crate::sim_rng::test_context();
        let assets = LevelAssets::new();
        for _ in 0..4 {
            engine.tick_entity_movement(&sim, &assets);
            engine.hourglass_phase_sequences(
                &sim,
                &mut crate::engine::HostDisplayState::default(),
                &assets,
            );
            if engine
                .get_entity(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .active_movement
                .sequence_id
                .is_none()
            {
                break;
            }
        }

        let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
        assert!(actor.active_movement.sequence_id.is_none());
        assert!(
            actor.post_seek_sequence.is_some(),
            "a stop transition ending outside the point seek's sector must not launch its stale post-seek"
        );
        assert_ne!(engine.actor_order_type(owner), Some(OrderType::DroppingAle));
    }

    #[test]
    fn translated_point_seek_terminal_in_matching_sector_launches_drop_ale() {
        let (mut engine, owner, _target) = install_terminal_interaction_seek(Command::HitCmd, 40.0);
        let (old_sequence, old_index) = {
            let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
            (
                actor.active_movement.sequence_id.unwrap(),
                actor.active_movement.element_index,
            )
        };
        engine.orders.sequence_manager.element_interrupted(
            old_sequence,
            old_index,
            crate::sequence::CascadeFlags::FOLLOWING,
        );
        {
            let actor = engine
                .get_entity_mut(owner)
                .unwrap()
                .actor_data_mut()
                .unwrap();
            actor.active_movement.clear();
            actor.post_seek_sequence = None;
        }
        engine.dispatch_condolations(&crate::sim_rng::test_context(), &LevelAssets::new());

        let seek_sector = crate::position_interface::SectorHandle::new(1).unwrap();
        let seek_layer = 3;
        let destination = MapPoint::new(100.0, 100.0);
        let mut post_seek = Sequence::new();
        post_seek.append_element(SequenceElement::new(1, Command::DropAle, Some(owner)));
        let mut seek =
            SequenceElement::new_movement(1, Command::Seek, Some(owner), OrderType::WalkingUpright);
        seek.priority = SequencePriority::Normal;
        let SequenceElementData::Movement {
            destination: stored_destination,
            sector,
            layer,
            post_seek_sequence,
            ..
        } = &mut seek.data
        else {
            unreachable!("Seek must have movement data")
        };
        *stored_destination = destination;
        *sector = Some(seek_sector);
        *layer = seek_layer;
        *post_seek_sequence = Some(Box::new(post_seek));

        let transient = engine.orders.sequence_manager.launch_element(seek);
        engine.hourglass_phase_sequences(
            &crate::sim_rng::test_context(),
            &mut crate::engine::HostDisplayState::default(),
            &LevelAssets::new(),
        );

        let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
        assert!(actor.continuation.seek_to_point);
        assert_eq!(actor.continuation.seek_layer, seek_layer);
        assert_eq!(
            actor.continuation.seek_sector,
            Some(crate::actor_state::ActorSeekSector::Position(seek_sector))
        );
        assert!(actor.post_seek_sequence.is_some());

        let (movement_sequence, movement_index) = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .filter(|sequence| sequence.id != transient)
            .find_map(|sequence| {
                sequence
                    .elements
                    .iter()
                    .enumerate()
                    .find_map(|(index, element)| {
                        (element.owner == Some(owner)
                            && element.data.is_movement()
                            && element.state == SequenceState::InProgress)
                            .then_some((sequence.id, index))
                    })
            })
            .expect("Translate(SEEK) must launch a concrete MOVE|SEEK replacement");
        let transition = OrderType::TransitionWalkingUprightWaitingUpright;
        let order_id = engine.orders.allocate_order_id();
        let follower_order_id = engine.orders.allocate_order_id();
        let movement = engine
            .orders
            .sequence_manager
            .get_element_mut(movement_sequence, movement_index)
            .unwrap();
        movement.command = Command::MoveOk;
        let SequenceElementData::Movement { sector, .. } = &mut movement.data else {
            panic!("translated point Seek replacement lost movement data")
        };
        *sector = crate::position_interface::SectorHandle::new(2);
        movement.orders.clear();
        movement.orders.push_back(Order::new(
            transition,
            destination.x,
            destination.y,
            order_id,
        ));
        movement.orders.push_back(Order::new(
            transition,
            destination.x,
            destination.y,
            follower_order_id,
        ));
        // Point-target PerformSeek keys the terminal handoff on the absence
        // of another movement order, not on this movement element being the
        // final element of its sequence. Keep a later sibling present to
        // cover that distinction.
        engine
            .orders
            .sequence_manager
            .get_sequence_mut(movement_sequence)
            .unwrap()
            .append_element(SequenceElement::new(2, Command::Wait, Some(owner)));
        {
            let entity = engine.get_entity_mut(owner).unwrap();
            entity.actor_data_mut().unwrap().active_movement =
                ActiveMovement::new(movement_sequence, movement_index);
            entity.position_iface_mut().set_map_goal(destination);
        }

        finish_terminal_seek_tick(&mut engine, owner);
        let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
        assert!(actor.post_seek_sequence.is_none());
        assert!(actor.active_movement.sequence_id.is_none());
        assert_eq!(engine.actor_order_type(owner), Some(OrderType::DroppingAle));
    }
}

#[cfg(test)]
mod arrival_snap_tests {
    use super::{
        MovementDeferred, both_sword_ranges_contain_distance,
        perform_seek_exposes_motion_termination, queue_committed_arrival_crossing,
        should_snap_arrival,
    };
    use crate::coordinates::MapPoint;
    use crate::element::{EntityId, PcId};

    #[test]
    fn committed_seek_arrival_retains_both_actor_crossing_passes() {
        let mut deferred = MovementDeferred::default();
        let owner = EntityId::Pc(PcId(252));
        let old_pos = MapPoint::new(1284.826_3, 2277.009_3);

        assert!(queue_committed_arrival_crossing(
            &mut deferred,
            owner,
            old_pos,
            0,
            true,
            true,
        ));
        assert_eq!(deferred.line_cross_checks, vec![(owner, old_pos, 0)]);
        assert_eq!(
            deferred.non_elevation_cross_checks,
            vec![(owner, old_pos, 0)]
        );
    }

    #[test]
    fn stationary_or_ineligible_seek_arrival_does_not_queue_crossing() {
        let mut deferred = MovementDeferred::default();
        let owner = EntityId::Pc(PcId(252));
        let old_pos = MapPoint::new(1284.826_3, 2277.009_3);

        assert!(!queue_committed_arrival_crossing(
            &mut deferred,
            owner,
            old_pos,
            0,
            false,
            true,
        ));
        assert!(!queue_committed_arrival_crossing(
            &mut deferred,
            owner,
            old_pos,
            0,
            true,
            false,
        ));
        assert!(deferred.line_cross_checks.is_empty());
        assert!(deferred.non_elevation_cross_checks.is_empty());
    }

    #[test]
    fn exact_goal_without_a_committed_step_does_not_snap() {
        assert!(!should_snap_arrival(false, false, 0.0, false));
        assert!(should_snap_arrival(true, false, 0.0, false));
        assert!(!should_snap_arrival(true, true, 0.0, false));
        assert!(!should_snap_arrival(true, false, 1.0, false));
        assert!(!should_snap_arrival(true, false, 0.0, true));
    }

    #[test]
    fn entity_seek_wait_hides_wrapped_motion_termination() {
        assert!(!perform_seek_exposes_motion_termination(false, Some(true)));
        assert!(perform_seek_exposes_motion_termination(true, Some(true)));
        assert!(perform_seek_exposes_motion_termination(false, None));
    }

    #[test]
    fn sword_provoke_range_is_snapshotted_before_line_crossing_projection() {
        // Linux2/Profile002/Savegame_015/replay-016: Original evaluates the
        // terminal gate inside Human::Execute at 89.7441025. Actor::Hourglass
        // then projects the owner onto a crossed elevation line, where the
        // same live-position calculation becomes 90.76145. The owner's
        // MAXIMAL boundary is 90, so re-evaluating after crossing invents a
        // Provoke that Original never registered.
        let execute_distance = f32::from_bits(0x42b3_7cfb);
        let after_crossing_distance = 90.76145_f32;
        assert!(!both_sword_ranges_contain_distance(
            execute_distance,
            90,
            150,
            70,
            150
        ));
        assert!(both_sword_ranges_contain_distance(
            after_crossing_distance,
            90,
            150,
            70,
            150
        ));
    }
}

#[cfg(test)]
mod aligned_transition_deviation_tests {
    use super::should_clear_deviated_for_aligned_transition_start;
    use crate::coordinates::MapPoint;

    #[test]
    fn pc_preserves_deviation_across_aligned_movement_exit_transition() {
        let aligned = MapPoint::new(407.967_35, 786.937_9);

        assert!(!should_clear_deviated_for_aligned_transition_start(
            true,
            true,
            true,
            crate::order::OrderType::TransitionWalkingUprightWaitingUpright,
            true,
            aligned,
            aligned,
        ));
    }

    #[test]
    fn pc_retires_stale_deviation_at_aligned_movement_start_transition() {
        let aligned = MapPoint::new(1297.909_3, 720.281_25);

        assert!(should_clear_deviated_for_aligned_transition_start(
            true,
            true,
            true,
            crate::order::OrderType::TransitionWaitingUprightWalkingUpright,
            true,
            aligned,
            aligned,
        ));
    }

    #[test]
    fn non_pc_clears_deviation_only_on_aligned_movement_start_transition() {
        let aligned = MapPoint::new(100.0, 200.0);

        assert!(!should_clear_deviated_for_aligned_transition_start(
            false,
            true,
            true,
            crate::order::OrderType::TransitionWalkingUprightWaitingUpright,
            true,
            aligned,
            aligned,
        ));
        assert!(should_clear_deviated_for_aligned_transition_start(
            false,
            true,
            true,
            crate::order::OrderType::TransitionWaitingUprightWalkingUpright,
            true,
            aligned,
            aligned,
        ));
        // Savegame_032 replay-010: Soldier 52 reaches an aligned generated
        // running startup with the observable anti-vibration history
        // `deviated=true, direction_count=-2`. The next clockwise shield
        // turn rotates on its first call in the Original, which is possible
        // only when this running startup drops the deviation latch.
        assert!(should_clear_deviated_for_aligned_transition_start(
            false,
            true,
            true,
            crate::order::OrderType::TransitionWaitingUprightRunningUpright,
            true,
            aligned,
            aligned,
        ));
        assert!(!should_clear_deviated_for_aligned_transition_start(
            false,
            false,
            true,
            crate::order::OrderType::TransitionWalkingUprightWaitingUpright,
            true,
            aligned,
            aligned,
        ));
        assert!(!should_clear_deviated_for_aligned_transition_start(
            false,
            true,
            false,
            crate::order::OrderType::TransitionWalkingUprightWaitingUpright,
            true,
            aligned,
            aligned,
        ));
    }
}

#[cfg(test)]
mod path_request_timing_tests {
    use super::*;
    use crate::entity_id::{PcId, SoldierId};

    #[test]
    fn line_goal_still_uses_pathfinder_when_thick_route_is_blocked() {
        use crate::sequence::MoveFlags;

        assert!(!movement_flags_force_direct_dispatch(MoveFlags::LINE));
        assert!(movement_flags_force_direct_dispatch(MoveFlags::MAP));
        assert!(movement_flags_force_direct_dispatch(MoveFlags::STRAIGHT));
    }

    #[test]
    fn every_non_direct_request_uses_the_source_extraction_gate() {
        assert!(path_request_needs_source_extraction(false, false));
        assert!(!path_request_needs_source_extraction(false, true));
        assert!(!path_request_needs_source_extraction(true, false));
    }

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
        let completed = queue.take_completed().map(|(processed, valid)| {
            assert!(valid, "ordinary fake-frame request became ignored");
            processed.request.owner
        });
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
    fn cancelled_head_is_delivered_invalid_and_occupies_its_result_slot() {
        let cancelled = EntityId::Pc(PcId(3));
        let successor = EntityId::Soldier(SoldierId(4));
        let mut queue = PendingPathRequestQueue::default();
        queue.enqueue(request(cancelled, crate::pathfinder::PathFinderSpeed::Fast));
        queue.enqueue(request(
            successor,
            crate::pathfinder::PathFinderSpeed::Medium,
        ));
        let first = queue.pop_to_start().expect("cancelled head starts");
        queue.set_in_flight(first, Some(vec![MapPoint::new(20.0, 20.0)]));

        queue.cancel_for_owner(cancelled);
        let (completed, valid) = queue
            .take_completed()
            .expect("cancelled head still completes");
        assert_eq!(completed.request.owner, cancelled);
        assert!(!valid);

        let next = queue.pop_to_start().expect("successor remains queued");
        assert_eq!(next.owner, successor);
    }

    #[test]
    fn cancelled_waiting_head_starts_even_after_its_element_dies() {
        let cancelled = EntityId::Soldier(SoldierId(3));
        let mut queue = PendingPathRequestQueue::default();
        queue.enqueue(request(cancelled, crate::pathfinder::PathFinderSpeed::Fast));

        queue.cancel_for_owner(cancelled);
        assert!(queue.ignore_next_path);
        let retained = queue.pop_to_start().expect("cancelled head remains queued");
        let cancelled_result = retained_cancelled_path_result(queue.ignore_next_path)
            .expect("cancelled path search exits with an empty raw path");
        assert!(cancelled_result.is_empty());
        queue.set_in_flight(retained, Some(cancelled_result));

        let (completed, valid) = queue
            .take_completed()
            .expect("cancelled head still consumes a completion slot");
        assert_eq!(completed.request.owner, cancelled);
        assert!(!valid);
    }

    #[test]
    fn parity_snapshot_keeps_ready_head_before_waiting_fifo() {
        let ready_owner = EntityId::Pc(PcId(3));
        let waiting_owner = EntityId::Soldier(SoldierId(4));
        let mut queue = PendingPathRequestQueue::default();
        queue.enqueue(request(
            ready_owner,
            crate::pathfinder::PathFinderSpeed::Fast,
        ));
        queue.enqueue(request(
            waiting_owner,
            crate::pathfinder::PathFinderSpeed::Medium,
        ));
        let ready = queue.pop_to_start().expect("head starts synchronously");
        queue.set_in_flight(ready, None);
        queue.cancel_for_owner(ready_owner);

        let mut grid = crate::fast_find_grid::FastFindGrid::new();
        grid.add_move_box_half_diagonal(crate::coordinates::MoveBoxHalfDiagonal::new(1.0, 1.0));
        let (ignored, snapshot) = queue.parity_state(&grid);

        assert!(ignored);
        assert_eq!(snapshot.len(), 2);
        assert!(snapshot[0].in_flight);
        assert_eq!(snapshot[0].request.actor, ready_owner);
        assert_eq!(snapshot[0].waypoints, Some(Vec::new()));
        assert!(!snapshot[1].in_flight);
        assert_eq!(snapshot[1].request.actor, waiting_owner);
        assert_eq!(snapshot[1].waypoints, None);
    }

    #[test]
    fn pending_make_rewrites_first_request_like_original_pathfinder() {
        let owner = EntityId::Pc(PcId(3));
        let mut queue = PendingPathRequestQueue::default();
        queue.enqueue(request(owner, crate::pathfinder::PathFinderSpeed::Fast));

        queue.make_fast(owner, 7);
        assert_eq!(queue.waiting[0].move_action, OrderType::RunningUpright);
        assert_eq!(queue.waiting[0].half_diagonal_idx, 7);

        queue.make_slow(owner, 7);
        assert_eq!(queue.waiting[0].move_action, OrderType::WalkingUpright);

        queue.make_crouched(owner, 9);
        assert_eq!(queue.waiting[0].move_action, OrderType::WalkingCrouched);
        assert_eq!(queue.waiting[0].half_diagonal_idx, 9);

        queue.make_upright(owner, 7);
        assert_eq!(queue.waiting[0].move_action, OrderType::WalkingUpright);
        assert_eq!(queue.waiting[0].half_diagonal_idx, 7);
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
            .push(FailedPathRequest::from_pending(
                PendingPathRequest::test_request(owner, sequence_id, 0),
                10,
            ));

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
    use crate::element::{
        ActionState, ActorData, ActorPc, ActorSoldier, Command, ElementData, ElementKind, Entity,
        HumanData, NpcData, PcData, Posture, SoldierData,
    };
    use crate::sequence::{
        Field, FieldValue, MoveFlags, Sequence, SequenceElement, SequenceElementData,
        SequencePriority, SequenceState,
    };

    fn extraction_test_pc(posture: Posture) -> Entity {
        Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                posture,
                ..Default::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        })
    }

    fn extraction_test_assets() -> LevelAssets {
        let mut profiles = crate::profiles::ProfileManager::new();
        profiles
            .characters
            .push(crate::profiles::CharacterProfile::default());
        LevelAssets {
            profile_manager: std::sync::Arc::new(profiles),
            ..LevelAssets::new()
        }
    }

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
            movement_execute_state_effect(
                OrderType::TransitionWalkingCrouchedWaitingCrouched,
                MotionState::Terminated
            ),
            Some((Posture::Crouched, ActionState::Waiting)),
            "a seek arrival in the crouched stop transition must publish Waiting before its post-seek interaction is instructed"
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
    fn terminal_first_running_upright_execute_still_stamps_moving_fast() {
        use crate::element::{ActionState, Posture};
        use crate::sprite::MotionState;

        assert_eq!(
            movement_execute_state_effect(OrderType::RunningUpright, MotionState::Terminated),
            Some((Posture::Upright, ActionState::MovingFast))
        );
    }

    #[test]
    fn in_progress_walking_with_shield_stamps_moving_shield() {
        use crate::element::{ActionState, Posture};
        use crate::sprite::MotionState;

        assert_eq!(
            movement_execute_state_effect(OrderType::WalkingWithShield, MotionState::InProgress),
            Some((Posture::Upright, ActionState::MovingShield))
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
    fn entity_target_seek_does_not_synthesize_deferred_pc_movement_start() {
        assert!(should_defer_pc_movement_state_start(true, false));
        assert!(
            !should_defer_pc_movement_state_start(true, true),
            "entity-target PerformSeek hides START from the Original Execute arm"
        );
        assert!(!should_defer_pc_movement_state_start(false, false));
    }

    fn run_fast_wall_anti_collision_fixture(
        first_distance: u16,
    ) -> (MapPoint, crate::movement_diagnostics::ParityMovementStep) {
        use crate::element::{
            ActorData, ActorPc, ElementData, ElementKind, HumanData, PcData, Posture,
        };
        use crate::fast_find_grid::GridSector;
        use crate::order::Order;
        use crate::sector::{LiftType, SectorNumber, SectorType};
        use crate::sequence::SequenceElement;
        use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

        let mut engine = EngineInner::new();
        let start = MapPoint::new(1760.418701171875, 1011.02197265625);
        let goal = MapPoint::new(1762.0, 996.0);
        let physical = OrderType::ClimbingWallDown;
        let script = SpriteScript {
            action_id: physical as u16,
            action_done: 7,
            average_speed: 4.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 32,
            frame_ids: vec![1; 8],
            delays: vec![0; 8],
            distances: std::iter::once(first_distance)
                .chain(std::iter::repeat_n(4, 7))
                .collect(),
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 8],
            sound_ids: vec![0; 8],
        };
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[physical as usize] = 0;
        let mut pc = Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                posture: Posture::OnWall,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        });
        pc.element_data_mut().active = true;
        pc.element_data_mut().sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script; 16]),
            std::sync::Arc::new(conversion),
        );
        pc.element_data_mut().set_position_map(start);
        pc.element_data_mut().set_direction_instantly(0);
        pc.element_data_mut().set_sector(Some(
            crate::position_interface::SectorHandle::new(1).unwrap(),
        ));
        pc.position_iface_mut()
            .set_move_box(crate::coordinates::MoveBox::from_coords(
                -4.0, -4.0, 4.0, 4.0,
            ));
        pc.position_iface_mut().set_anti_collision_on(true);
        let owner = engine.add_entity(pc);
        {
            let level = std::sync::Arc::make_mut(&mut engine.world.fast_grid_mut().level);
            level.sector_number_map.insert(SectorNumber::new(1), 0);
            level.sectors.push(GridSector {
                points: Vec::new(),
                bounding_box: crate::coordinates::MapBBox::new(),
                sector_type: SectorType::LIFT,
                layer: 0,
                sector_number: SectorNumber::new(1),
                door_index: None,
                lift_type: Some(LiftType::Wall),
                lift_direction: 0,
                force_crouched: false,
                building_index: None,
                low_exit_point: Some(goal),
                high_exit_point: Some(start),
                lowest_door_index: None,
                jump_line_indices: Vec::new(),
                gate_indices: Vec::new(),
                underlying_sector: None,
            });
        }
        let mut movement = SequenceElement::new_movement(
            1,
            Command::MoveOk,
            Some(owner),
            OrderType::ClimbingWallDownFast,
        );
        movement.orders.push_back(Order::test_new(
            OrderType::ClimbingWallDownFast,
            goal.x,
            goal.y,
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

        crate::movement_diagnostics::begin_parity_movement_capture();
        engine.tick_entity_movement(&crate::sim_rng::test_context(), &LevelAssets::new());
        let captures = crate::movement_diagnostics::take_parity_movement_capture();

        let position = engine
            .get_entity(owner)
            .unwrap()
            .element_data()
            .position_map();
        let capture = captures
            .iter()
            .find(|capture| capture.entity == owner)
            .expect("fast wall owner must emit a production movement capture")
            .clone();
        (position, capture)
    }

    #[test]
    fn fast_wall_anti_collision_commits_each_perform_motion_before_the_next() {
        let (position, capture) = run_fast_wall_anti_collision_fixture(4);
        assert_eq!(capture.split_calls.len(), 2);
        assert_eq!(capture.split_calls[0].pre_position.x.bits, 1155272038);
        assert_eq!(capture.split_calls[0].post_position.x.bits, 1155275468);
        assert_eq!(capture.split_calls[1].pre_position.x.bits, 1155275468);
        assert_eq!(capture.split_calls[1].post_position.x.bits, 1155278898);
        assert_eq!(position.x.to_bits(), 1155278898);
        assert_eq!(position.y.to_bits(), 1148896312);
        assert_ne!(
            position.x.to_bits(),
            1155278899,
            "one aggregated eight-unit commit reproduces the captured +1 ULP defect"
        );
    }

    #[test]
    fn zero_distance_first_fast_wall_call_still_executes_the_second() {
        let (position, capture) = run_fast_wall_anti_collision_fixture(0);
        assert_ne!(
            position.x.to_bits(),
            1155272038,
            "the nonzero second PerformMotion must still commit"
        );
        assert_eq!(
            capture.split_calls.len(),
            1,
            "Original emits no movement-step record for the zero-distance first call"
        );
        assert_eq!(capture.split_calls[0].frame_distance_raw.value, 4.0);
        assert_eq!(capture.split_calls[0].pre_position.x.bits, 1155272038);
    }

    #[test]
    fn running_stairs_second_call_observes_first_call_arrival_snap() {
        use crate::element::{
            ActorData, ActorPc, ElementData, ElementKind, HumanData, PcData, Posture,
        };
        use crate::order::Order;
        use crate::sequence::SequenceElement;
        use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

        let mut engine = EngineInner::new();
        let start = MapPoint::new(697.1148681640625, 1420.9931640625);
        let goal = MapPoint::new(696.0, 1423.0);
        let physical = OrderType::WalkingStairs;
        let script = SpriteScript {
            action_id: physical as u16,
            action_done: 7,
            average_speed: 5.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 40,
            frame_ids: vec![1; 8],
            delays: vec![0; 8],
            distances: vec![5; 8],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 8],
            sound_ids: vec![0; 8],
        };
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[physical as usize] = 0;
        let mut pc = Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                posture: Posture::Upright,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        });
        pc.element_data_mut().active = true;
        pc.element_data_mut().sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script; 16]),
            std::sync::Arc::new(conversion),
        );
        pc.element_data_mut().set_position_map(start);
        pc.element_data_mut().set_direction_instantly(11);
        pc.position_iface_mut().set_anti_collision_on(false);
        let owner = engine.add_entity(pc);

        let mut movement = SequenceElement::new_movement(
            1,
            Command::MoveOk,
            Some(owner),
            OrderType::RunningStairs,
        );
        movement
            .orders
            .push_back(Order::test_new(OrderType::RunningStairs, goal.x, goal.y));
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

        crate::movement_diagnostics::begin_parity_movement_capture();
        engine.tick_entity_movement(&crate::sim_rng::test_context(), &LevelAssets::new());
        let capture = crate::movement_diagnostics::take_parity_movement_capture()
            .into_iter()
            .find(|capture| capture.entity == owner)
            .expect("running-stairs owner must emit a production movement capture");

        assert_eq!(capture.split_calls.len(), 2);
        assert_eq!(
            capture.split_calls[0].pre_position.x.bits,
            start.x.to_bits()
        );
        assert_eq!(
            capture.split_calls[0].post_position.x.bits,
            goal.x.to_bits()
        );
        assert_eq!(
            capture.split_calls[0].post_position.y.bits,
            goal.y.to_bits()
        );
        assert_eq!(capture.split_calls[1].pre_position.x.bits, goal.x.to_bits());
        assert_eq!(capture.split_calls[1].pre_position.y.bits, goal.y.to_bits());
        assert_ne!(
            capture.split_calls[0].requested_delta.x.bits, 0,
            "the first call must genuinely overshoot before its arrival snap"
        );
        assert_ne!(
            capture.split_calls[1].requested_delta.x.bits, 0,
            "RunningStairs must still execute its second PerformMotion after termination"
        );

        let offset_point = start + crate::coordinates::MapVec::new(7.0, -3.0);
        let line_a = start + crate::coordinates::MapVec::new(-2.0, 4.0);
        let line_b = start + crate::coordinates::MapVec::new(6.0, 4.0);
        let mut snapshot = crate::engine::anti_collision::ActorSnapshot {
            id: owner,
            active: true,
            is_actor: true,
            is_human: true,
            is_ignored_for_anti_collision: false,
            position_map: start,
            layer: 0,
            sector: None,
            posture: Posture::Upright,
            element_kind: ElementKind::ActorPc,
            target_element: None,
            is_swordfighting: false,
            repulsive_point: None,
            extra_repulsive_points: vec![crate::repulsive::RepulsivePoint::new(
                offset_point,
                4.0,
                12.0,
            )],
            repulsive_lines: vec![crate::repulsive::RepulsiveLine::new(
                line_a, line_b, 0.0, 5.0,
            )],
        };
        sync_snapshot_after_committed_step(&mut snapshot, start, goal);
        let committed = goal - start;
        assert_eq!(
            snapshot.extra_repulsive_points[0].position,
            offset_point + committed,
            "offset repulsive geometry must follow the snapped commit, not the raw overshoot"
        );
        assert_eq!(snapshot.repulsive_lines[0].a, line_a + committed);
        assert_eq!(snapshot.repulsive_lines[0].b, line_b + committed);
    }

    fn run_running_stairs_outer_crossing_fixture(
        crosses_first_substep: bool,
    ) -> (
        EngineInner,
        EntityId,
        crate::movement_diagnostics::ParityMovementStep,
        crate::fast_find_grid::LineIndex,
    ) {
        use crate::element::{
            ActorData, ActorPc, ElementData, ElementKind, HumanData, PcData, Posture,
        };
        use crate::fast_find_grid::GridLine;
        use crate::order::Order;
        use crate::sequence::SequenceElement;
        use crate::sight_obstacle::SightObstacle;
        use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

        let mut engine = EngineInner::new();
        engine.world.fast_grid_mut().size_map(30, 30);
        engine.world.fast_grid_mut().allocate_layers(1);

        let start = MapPoint::new(697.1148681640625, 1420.9931640625);
        let goal = MapPoint::new(696.0, 1423.0);
        let physical = OrderType::WalkingStairs;
        let script = SpriteScript {
            action_id: physical as u16,
            action_done: 7,
            average_speed: 5.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 40,
            frame_ids: vec![1; 8],
            delays: vec![0; 8],
            distances: vec![5; 8],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 8],
            sound_ids: vec![0; 8],
        };
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[physical as usize] = 0;
        let mut pc = Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                active: true,
                posture: Posture::Upright,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        });
        pc.element_data_mut().sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script; 16]),
            std::sync::Arc::new(conversion),
        );
        pc.element_data_mut().set_position_map(start);
        pc.element_data_mut().set_direction_instantly(11);
        pc.position_iface_mut().set_anti_collision_on(false);
        let owner = engine.add_entity(pc);

        let mut movement = SequenceElement::new_movement(
            1,
            Command::MoveOk,
            Some(owner),
            OrderType::RunningStairs,
        );
        movement
            .orders
            .push_back(Order::test_new(OrderType::RunningStairs, goal.x, goal.y));
        movement.orders.push_back(Order::test_new(
            OrderType::RunningStairs,
            goal.x - 20.0,
            goal.y + 20.0,
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

        // A short perpendicular bond through the midpoint is crossed only
        // by the first start->goal substep. The control translates that
        // bond away from the complete outer movement segment.
        let midpoint = MapPoint::new((start.x + goal.x) * 0.5, (start.y + goal.y) * 0.5);
        let travel = goal - start;
        let offset_x = if crosses_first_substep { 0.0 } else { 30.0 };
        let line_center = MapPoint::new(midpoint.x + offset_x, midpoint.y);
        let line_a = MapPoint::new(
            line_center.x - travel.y * 2.0,
            line_center.y + travel.x * 2.0,
        );
        let line_b = MapPoint::new(
            line_center.x + travel.y * 2.0,
            line_center.y - travel.x * 2.0,
        );
        let line_index = engine
            .world
            .fast_grid_mut()
            .add_line(GridLine::new_elevation(line_a, line_b, None, Some(0)), 0);

        let mut ramp = SightObstacle::new_default(1);
        // z = 0.5*x - 340: a real sloped plane whose 3D movement vector
        // changes the facing selected by ComputeIncrementAll.
        ramp.top_plane_points = [
            [690.0, 1400.0, 5.0],
            [710.0, 1400.0, 15.0],
            [690.0, 1450.0, 5.0],
        ];
        let mut assets = LevelAssets::new();
        assets.static_sight_obstacles = std::sync::Arc::new(vec![ramp]);
        engine.world.static_sight_obstacle_active = vec![true];

        crate::movement_diagnostics::begin_parity_movement_capture();
        engine.tick_entity_movement(&crate::sim_rng::test_context(), &assets);
        let capture = crate::movement_diagnostics::take_parity_movement_capture()
            .into_iter()
            .find(|capture| capture.entity == owner)
            .expect("running-stairs owner must emit a production movement capture");
        (engine, owner, capture, line_index)
    }

    #[test]
    fn running_stairs_crossing_uses_outer_pre_position() {
        let (engine, owner, capture, line_index) = run_running_stairs_outer_crossing_fixture(true);
        assert_eq!(capture.split_calls.len(), 2);
        let outer_pre = MapPoint::new(
            capture.split_calls[0].pre_position.x.value,
            capture.split_calls[0].pre_position.y.value,
        );
        let first_post = MapPoint::new(
            capture.split_calls[0].post_position.x.value,
            capture.split_calls[0].post_position.y.value,
        );
        let second_post = MapPoint::new(
            capture.split_calls[1].post_position.x.value,
            capture.split_calls[1].post_position.y.value,
        );
        assert_eq!(
            engine
                .world
                .fast_grid
                .get_crossing_elevation_line_indices(0, outer_pre, first_post),
            vec![line_index],
            "the bond must be crossed by the first literal PerformMotion commit"
        );
        assert!(
            engine
                .world
                .fast_grid
                .get_crossing_elevation_line_indices(0, first_post, second_post)
                .is_empty(),
            "the post-first position used by the old Rust code must miss the bond"
        );

        let entity = engine.get_entity(owner).unwrap();
        assert_eq!(
            entity.element_data().obstacle_index().map(u16::from),
            Some(0)
        );
        let pi = entity.position_iface();
        assert!(
            pi.get_position().z > 0.0,
            "crossing must project onto the ramp"
        );
        assert_ne!(
            pi.get_increment().z,
            0.0,
            "the ramp must rebuild the 3D increment"
        );
        assert_eq!(
            pi.get_direction_goal(),
            crate::position_interface::vector_to_direction(
                pi.get_increment().x,
                pi.get_increment().y,
            ),
            "the post-crossing ComputeIncrementAll must refresh direction_goal"
        );
    }

    #[test]
    fn running_stairs_without_outer_crossing_keeps_ground_plane() {
        let (engine, owner, capture, line_index) = run_running_stairs_outer_crossing_fixture(false);
        let outer_pre = MapPoint::new(
            capture.split_calls[0].pre_position.x.value,
            capture.split_calls[0].pre_position.y.value,
        );
        let second_post = MapPoint::new(
            capture.split_calls[1].post_position.x.value,
            capture.split_calls[1].post_position.y.value,
        );
        assert!(
            engine
                .world
                .fast_grid
                .get_crossing_elevation_line_indices(0, outer_pre, second_post)
                .is_empty()
        );
        assert!(engine.world.fast_grid.level.lines[usize::from(line_index)].is_elevation);
        let entity = engine.get_entity(owner).unwrap();
        assert_eq!(entity.element_data().obstacle_index(), None);
        assert_eq!(entity.position_iface().get_position().z, 0.0);
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
        assert!(
            stationary_motion_waits(0.0, false, f32::NAN),
            "a zero-distance animation frame must not multiply an invalid movement increment by zero"
        );
    }

    #[test]
    fn only_nonzero_distance_transition_frames_recompute_an_exact_position() {
        assert!(motion_recomputes_exact_position(true, true, 2.0, 0.0));
        assert!(
            !motion_recomputes_exact_position(false, true, 2.0, 0.0),
            "an ordinary exact-goal move takes the arrival path and must preserve its coordinates"
        );
        assert!(
            !motion_recomputes_exact_position(true, true, 0.0, 0.0),
            "a zero-distance transition frame never enters Original's position-update block"
        );
    }

    #[test]
    fn exact_goal_transition_clears_stale_running_forecast() {
        use crate::element::{
            ActionState, ActorData, ActorPc, ElementData, ElementKind, HumanData, PcData, Posture,
        };
        use crate::order::Order;
        use crate::sequence::{SequenceElement, SequencePriority};
        use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

        let mut engine = EngineInner::new();
        let position = MapPoint::new(100.0, 100.0);
        let transition = OrderType::TransitionRunningUprightWaitingUpright;
        let script = SpriteScript {
            action_id: transition as u16,
            action_done: 2,
            average_speed: 2.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 6,
            frame_ids: vec![1, 2, 3],
            delays: vec![0; 3],
            distances: vec![2; 3],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
            sound_ids: vec![0; 3],
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
        element.sprite.last_action = transition;
        element
            .sprite
            .position_iface
            .set_move_box(crate::coordinates::MoveBox::from_coords(
                -4.0, -4.0, 4.0, 4.0,
            ));
        element.sprite.position_iface.set_anti_collision_on(true);
        element.sprite.position_iface.deviated = true;
        element.set_position_map(position);
        element
            .sprite
            .position_iface
            .set_map_goal(MapPoint::new(110.0, 100.0));
        element.sprite.position_iface.compute_increment_all(true);
        element
            .sprite
            .position_iface
            .update_forecasted_movement(5.0, 1);
        assert_ne!(
            element.sprite.position_iface.get_forecasted_movement(),
            crate::coordinates::WorldVec3D::ZERO
        );
        element.sprite.position_iface.zero_all_increments();

        let owner = engine.add_entity(Entity::Pc(ActorPc {
            element,
            actor: ActorData {
                action_state: ActionState::MovingFast,
                ..ActorData::default()
            },
            human: HumanData::default(),
            pc: PcData::default(),
        }));
        let order_id = engine.orders.allocate_order_id();
        let mut movement = SequenceElement::new_movement(
            1,
            Command::MoveOk,
            Some(owner),
            OrderType::RunningUpright,
        );
        movement.priority = SequencePriority::Normal;
        movement
            .orders
            .push_back(Order::new(transition, position.x, position.y, order_id));
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
                .position_iface()
                .get_forecasted_movement(),
            crate::coordinates::WorldVec3D::ZERO,
            "Original's nonzero animation-distance block refreshes the forecast with the zero goal increment"
        );
        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .element_data()
                .position_map(),
            position,
            "the forecast refresh must not move an actor already at the transition goal"
        );
        assert!(
            !engine
                .get_entity(owner)
                .unwrap()
                .position_iface()
                .is_deviated(),
            "Original still runs zero-increment anti-collision recovery for a nonzero transition animation distance"
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
    fn committed_step_arrival_outranks_every_sprite_result() {
        use crate::element::{ActionState, Posture};
        use crate::sprite::MotionState;

        for motion in [
            MotionState::Start,
            MotionState::InProgress,
            MotionState::Done,
        ] {
            let visible =
                movement_execute_visible_motion(OrderType::WalkingWithCorpse, motion, true, false);
            assert_eq!(
                visible,
                MotionState::Terminated,
                "a step that satisfies the goal predicate reaches Execute as a termination"
            );
            assert_eq!(
                committed_arrival_post_completion_override(motion, visible, true),
                Some(MotionState::Terminated),
                "staged arrival must preserve the Execute result for the post-completion latch"
            );
        }
        assert_eq!(
            movement_execute_state_effect(
                OrderType::WalkingWithCorpse,
                movement_execute_visible_motion(
                    OrderType::WalkingWithCorpse,
                    MotionState::InProgress,
                    true,
                    false,
                ),
            ),
            Some((Posture::CarryingCorpse, ActionState::Waiting)),
            "the corpse carrier settles back to Waiting on the waypoint it reaches"
        );
        assert_eq!(
            movement_execute_state_effect(
                OrderType::WalkingWithCorpse,
                movement_execute_visible_motion(
                    OrderType::WalkingWithCorpse,
                    MotionState::InProgress,
                    false,
                    false,
                ),
            ),
            None,
            "a walk that has not reached its waypoint owns no state effect"
        );
    }

    #[test]
    fn entity_seek_refresh_countdown_preserves_original_unsigned_wrap() {
        assert_eq!(age_seek_refresh_wait(25), 24);
        assert_eq!(age_seek_refresh_wait(0), u32::MAX);
        assert_eq!(age_seek_refresh_wait(u32::MAX), u32::MAX - 1);
    }

    #[test]
    fn seek_refresh_dispatch_follows_selected_execute_arm() {
        assert_eq!(perform_seek_calls_per_execute(OrderType::WalkingUpright), 1);
        assert_eq!(perform_seek_calls_per_execute(OrderType::RunningStairs), 2);
        assert_eq!(perform_seek_calls_per_execute(OrderType::ClimbingWallUp), 0);
        assert_eq!(
            perform_seek_calls_per_execute(OrderType::ClimbingWallDown),
            0
        );
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
    fn path_source_is_skipped_even_when_nan_breaks_source_equality() {
        let mut waypoints = vec![
            MapPoint::new(f32::from_bits(0xffc0_0000), f32::from_bits(0xffc0_0000)),
            MapPoint::new(342.5066, 1546.6641),
        ];

        discard_unrequested_path_source(&mut waypoints, false);

        assert_eq!(waypoints, vec![MapPoint::new(342.5066, 1546.6641)]);
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

    fn dispatch_instruction_extraction_fixture(
        command: Command,
        obstruct_owner: bool,
        self_seek: bool,
    ) -> (
        MapPoint,
        MapPoint,
        SequenceState,
        bool,
        bool,
        Option<crate::actor_state::ActorSeekSector>,
        u16,
    ) {
        use crate::fast_find_grid::GridLine;
        use crate::position_interface::SectorHandle;

        let mut engine = EngineInner::new();
        engine.world.fast_grid_mut().size_map(4, 4);
        engine.world.fast_grid_mut().allocate_layers(1);
        if obstruct_owner {
            engine.world.fast_grid_mut().add_line(
                GridLine::new(MapPoint::new(0.0, 128.0), MapPoint::new(256.0, 128.0), true),
                0,
            );
        }

        let start = MapPoint::new(130.0, 130.0);
        let mut owner_entity = extraction_test_pc(Posture::Upright);
        owner_entity.element_data_mut().set_position_map(start);
        owner_entity.element_data_mut().set_layer(0);
        owner_entity
            .element_data_mut()
            .set_sector(SectorHandle::new(1));
        owner_entity
            .position_iface_mut()
            .set_move_box(crate::coordinates::MoveBox::from_coords(
                -10.0, -5.0, 10.0, 5.0,
            ));
        owner_entity.position_iface_mut().set_map_position(start);
        let owner = engine.add_entity(owner_entity);
        {
            let actor = engine
                .get_entity_mut(owner)
                .unwrap()
                .actor_data_mut()
                .unwrap();
            actor.continuation.seek_to_point = true;
            actor.continuation.seek_layer = 7;
            actor.continuation.seek_sector = Some(crate::actor_state::ActorSeekSector::Position(
                SectorHandle::new(1).unwrap(),
            ));
        }

        let mut target_entity = Entity::Soldier(ActorSoldier {
            element: ElementData {
                kind: ElementKind::ActorSoldier,
                posture: Posture::Upright,
                ..Default::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData::default(),
            soldier: SoldierData::default(),
        });
        let target_position = MapPoint::new(200.0, 200.0);
        target_entity
            .element_data_mut()
            .set_position_map(target_position);
        target_entity.element_data_mut().set_layer(0);
        target_entity
            .element_data_mut()
            .set_sector(SectorHandle::new(2));
        target_entity
            .position_iface_mut()
            .set_map_position(target_position);
        let target = engine.add_entity(target_entity);

        let mut movement =
            SequenceElement::new_movement(1, command, Some(owner), OrderType::WalkingUpright);
        movement.priority = SequencePriority::Normal;
        movement.posture_after_transition = Posture::Upright;
        movement.action_state_after_transition = ActionState::Waiting;
        let SequenceElementData::Movement {
            destination,
            element,
            flags,
            tolerance,
            post_seek_sequence,
            ..
        } = &mut movement.data
        else {
            unreachable!("new_movement must create movement data")
        };
        *destination = target_position;
        *tolerance = 20.0;
        if command == Command::Seek {
            *element = Some(if self_seek { owner } else { target });
            *flags |= MoveFlags::SEEK;
            if self_seek {
                let mut post_seek = Sequence::new();
                post_seek.append_element(SequenceElement::new(1, Command::Wait, Some(owner)));
                *post_seek_sequence = Some(Box::new(post_seek));
            }
        } else {
            *flags |= MoveFlags::MAP;
        }
        let sequence = engine.orders.sequence_manager.launch_element(movement);

        let before = engine
            .get_entity(owner)
            .expect("fixture owner exists before dispatch")
            .element_data()
            .position_map();
        let _ = engine.dispatch_ordered_move_seek_instruct(
            &crate::sim_rng::test_context(),
            &extraction_test_assets(),
            owner,
            sequence,
            0,
        );
        let after = engine
            .get_entity(owner)
            .expect("fixture owner exists after dispatch")
            .element_data()
            .position_map();
        let state = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .expect("fixture movement remains inspectable")
            .state;
        let post_seek_launched = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .any(|candidate| {
                candidate.id != sequence
                    && candidate.elements.iter().any(|element| {
                        element.owner == Some(owner) && element.command == Command::Wait
                    })
            });
        let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
        (
            before,
            after,
            state,
            post_seek_launched,
            actor.continuation.seek_to_point,
            actor.continuation.seek_sector,
            actor.continuation.seek_layer,
        )
    }

    #[test]
    fn cross_sector_seek_extracts_owner_before_refresh_route_failure() {
        let (before, after, state, _, seek_to_point, seek_sector, seek_layer) =
            dispatch_instruction_extraction_fixture(Command::Seek, true, false);
        assert_ne!(
            after, before,
            "the old late-only extraction leaves a cross-sector Seek owner embedded"
        );
        assert_eq!(state, SequenceState::Impossible);
        assert!(!seek_to_point, "entity Seek must replace stale point mode");
        assert_eq!(
            seek_sector,
            Some(crate::actor_state::ActorSeekSector::Position(
                crate::position_interface::SectorHandle::new(1).unwrap()
            )),
            "entity Seek preserves dormant point-sector metadata"
        );
        assert_eq!(seek_layer, 7, "entity Seek preserves dormant point layer");
    }

    #[test]
    fn authorized_cross_sector_seek_does_not_move_owner() {
        let (before, after, state, _, seek_to_point, seek_sector, seek_layer) =
            dispatch_instruction_extraction_fixture(Command::Seek, false, false);
        assert_eq!(after, before);
        assert_eq!(state, SequenceState::Impossible);
        assert!(!seek_to_point);
        assert_eq!(
            seek_sector,
            Some(crate::actor_state::ActorSeekSector::Position(
                crate::position_interface::SectorHandle::new(1).unwrap()
            ))
        );
        assert_eq!(seek_layer, 7);
    }

    #[test]
    fn direct_move_keeps_instruction_boundary_extraction() {
        let (before, after, state, _, _, _, _) =
            dispatch_instruction_extraction_fixture(Command::Move, true, false);
        assert_ne!(after, before);
        assert_eq!(state, SequenceState::InProgress);
    }

    #[test]
    fn unauthorized_self_seek_skips_extraction_and_launches_post_seek() {
        let (before, after, state, post_seek_launched, _, _, _) =
            dispatch_instruction_extraction_fixture(Command::Seek, true, true);
        assert_eq!(after, before, "self-Seek returns before MOVE extraction");
        assert_eq!(state, SequenceState::Terminated);
        assert!(post_seek_launched, "self-Seek still launches its successor");
    }

    #[test]
    fn face_opponent_uses_original_displacement_to_facing_angle_sign() {
        use crate::element::ActionState;

        let right = combat_movement_angle((1.0, 0.0), (0.0, 1.0));
        assert_eq!(
            combat_directional_animation(ActionState::MovingSword, right),
            OrderType::StrafingRightSword,
            "Angle(eastward displacement, northward facing) is +90 degrees"
        );

        let left = combat_movement_angle((1.0, 0.0), (0.0, -1.0));
        assert_eq!(
            combat_directional_animation(ActionState::MovingSword, left),
            OrderType::StrafingLeftSword,
            "reversing the facing vector selects the opposite strafe"
        );

        // A destination the actor already stands on leaves both the dot and
        // the determinant at zero, which the Original resolves as a half turn
        // regardless of where the opponent is.
        for facing in [(0.0, 1.0), (1.0, 0.0), (-3.0, 7.5)] {
            assert_eq!(
                combat_directional_animation(
                    ActionState::MovingSword,
                    combat_movement_angle((0.0, 0.0), facing)
                ),
                OrderType::WalkingBackwardsSword,
                "a zero-length displacement walks backwards, not toward the facing sector"
            );
        }

        // Collinear vectors still resolve through the determinant test.
        assert_eq!(
            combat_directional_animation(
                ActionState::MovingSword,
                combat_movement_angle((2.0, 0.0), (5.0, 0.0))
            ),
            OrderType::WalkingSword,
            "moving straight at the opponent walks forward"
        );
        assert_eq!(
            combat_directional_animation(
                ActionState::MovingSword,
                combat_movement_angle((2.0, 0.0), (-5.0, 0.0))
            ),
            OrderType::WalkingBackwardsSword,
            "moving directly away from the opponent walks backwards"
        );
    }

    #[test]
    fn combat_nan_angle_uses_original_release_fallback_animation() {
        use crate::element::ActionState;

        let angle = combat_movement_angle((f32::NAN, f32::NAN), (1.0, 0.0));
        assert!(angle.is_nan());
        assert_eq!(
            combat_directional_animation(ActionState::MovingSword, angle),
            OrderType::WalkingSword
        );
        assert_eq!(
            combat_directional_animation(ActionState::MovingShield, angle),
            OrderType::WalkingShield
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
