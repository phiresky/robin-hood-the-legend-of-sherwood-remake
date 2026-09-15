use super::*;
use crate::element::{Command, Posture};

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

    assert_eq!(literal_lift_sprite_action(OT::PassingDoor), None);
    assert_eq!(literal_lift_sprite_action(OT::WalkingUpright), None);
}

#[test]
fn restored_pass_door_completion_accepts_serialized_and_geometry_free_ownership() {
    assert!(pass_door_transition_completion_has_owner(
        Command::PassDoor,
        false,
        OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel,
        true,
    ));
    assert!(pass_door_transition_completion_has_owner(
        Command::PassDoor,
        true,
        OrderType::TransitionWaitingCrouchedClimbingLadderDown,
        true,
    ));

    for action in [
        OrderType::TransitionClimbingLadderDownWaitingUpright,
        OrderType::TransitionClimbingLadderDownWaitingUprightAlerted,
    ] {
        assert!(pass_door_transition_completion_has_owner(
            Command::PassDoor,
            false,
            action,
            false,
        ));
    }

    assert!(
        !pass_door_transition_completion_has_owner(
            Command::PassDoor,
            false,
            OrderType::TransitionClimbingWallDownWaitingUpright,
            true,
        ),
        "door-dependent transition completion still requires either the materialized pass or its restored serialized chain"
    );
    assert!(
        !pass_door_transition_completion_has_owner(
            Command::Move,
            false,
            OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel,
            true,
        ),
        "an unrelated movement sequence must not acquire PassDoor completion semantics"
    );
}
