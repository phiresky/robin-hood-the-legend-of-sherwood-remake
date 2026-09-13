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

#[test]
fn lazy_door_steps_keep_original_position_around_copied_continuation() {
    let owner = EntityId::Pc(crate::entity_id::PcId(1));
    let mut element =
        SequenceElement::new_movement(1, Command::PassDoor, Some(owner), OrderType::WalkingUpright);
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

#[test]
fn materialized_door_action_points_precede_copied_walk() {
    let mut pass = ActiveDoorPass {
        door_index: crate::gate::DoorIndex::new(43).expect("valid door index"),
        direct: true,
        position_direct: true,
        steps: [
            crate::element::DoorPassStep::Select { speed: 2.0 },
            crate::element::DoorPassStep::PassingDoor,
            crate::element::DoorPassStep::Walk {
                destination: MapPoint::new(20.0, 30.0),
                action: OrderType::RunningUpright,
                reverse: false,
                compute_direction: true,
                tolerance: 0.0,
            },
            crate::element::DoorPassStep::PassingDoor,
        ]
        .into(),
        preallocated_order_ids: [10_u32, 11, 12, 13].map(std::num::NonZeroU32::new).into(),
        triggers_fired: 0,
        current_action: OrderType::TransitionWalkingUprightRunningUpright,
        current_reverse: false,
        saved_action_state: None,
    };
    let mut restored = pass.clone();
    restored.preallocated_order_ids.truncate(1);
    let mut next_order_id = 99;

    let orders = materialize_door_action_point_prefix(&mut pass, &mut next_order_id);

    assert_eq!(
        orders
            .iter()
            .map(|order| order.order_type)
            .collect::<Vec<_>>(),
        vec![OrderType::Select, OrderType::PassingDoor]
    );
    assert!(
        orders
            .iter()
            .all(|order| order.completion == crate::order::OrderCompletion::AdvanceElement)
    );
    assert_eq!(
        orders
            .iter()
            .map(|order| order.order_id.get())
            .collect::<Vec<_>>(),
        [10, 11]
    );
    assert!(matches!(
        pass.steps.front(),
        Some(crate::element::DoorPassStep::Walk {
            destination,
            action: OrderType::RunningUpright,
            ..
        }) if *destination == MapPoint::new(20.0, 30.0)
    ));
    assert_eq!(pass.steps.len(), 2, "the suffix remains lazily translated");
    assert_eq!(
        pass.preallocated_order_ids
            .iter()
            .map(|id| id.expect("seeded reservation remains present").get())
            .collect::<Vec<_>>(),
        [12, 13],
        "the Walk reservation must stay aligned after the materialized prefix"
    );
    assert_eq!(
        next_order_id, 99,
        "reserved action points allocate no new IDs"
    );
    let restored_orders = materialize_door_action_point_prefix(&mut restored, &mut next_order_id);
    assert_eq!(
        restored_orders
            .iter()
            .map(|order| order.order_id.get())
            .collect::<Vec<_>>(),
        [10, 99]
    );
    assert_eq!(
        next_order_id, 100,
        "only the consumed unreserved action point allocates an ID"
    );
    assert_eq!(restored.steps.len(), 2);
    assert_eq!(restored.preallocated_order_ids, [None, None]);
    assert_eq!(
        restored.triggers_fired, 0,
        "materialization must not execute callbacks"
    );
}
