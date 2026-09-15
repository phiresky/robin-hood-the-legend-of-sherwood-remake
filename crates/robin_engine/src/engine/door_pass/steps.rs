//! Door traversal posture and completion rules.
use super::*;

/// Posture owned eagerly by a lift animation while executing a door-pass
/// step. Wall-exit transitions are different from the climb rows: they only
/// inherit `OnWall` when the transition is
/// initialized, then its raw `DONE` edge is allowed to publish the landing
/// posture while the animation wrapper remains installed.
pub(in crate::engine) fn door_pass_eager_posture(
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
