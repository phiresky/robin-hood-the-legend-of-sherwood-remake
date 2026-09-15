use super::*;
use crate::element::Posture;
use crate::engine::test_support::actors::TestActor;
use crate::sequence::{SequenceElement, SequenceState};

fn transition_chain(suffix: &[(OrderType, MapPoint)]) -> (EngineInner, SelectedMovementOrder) {
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(TestActor::pc(Posture::Upright).build());
    engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .action_state = crate::element::ActionState::Moving;
    let mut element = SequenceElement::new_movement(
        1,
        crate::element::Command::PassDoor,
        Some(owner),
        OrderType::WalkingUpright,
    );
    element.orders.clear();
    let first_id = engine.orders.allocate_order_id();
    let mut transition = crate::order::Order::new(
        OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel,
        20.0,
        30.0,
        first_id,
    );
    transition.reverse = true;
    transition.compute_direction = false;
    transition.tolerance = 0.25;
    element.orders.push_back(transition);
    for &(action, target) in suffix {
        element.orders.push_back(crate::order::Order::new(
            action,
            target.x,
            target.y,
            engine.orders.allocate_order_id(),
        ));
    }
    element.state = SequenceState::InProgress;
    let seq_id = engine.orders.sequence_manager.insert_element(element);
    let selected = EngineInner::prepare_selected_movement_order(
        engine.world.entities.get_mut(owner).unwrap(),
        &engine.orders.sequence_manager,
        MovementOwnerSelection {
            seq_id,
            elem_idx: 0,
            order_id: first_id,
        },
        owner,
        false,
    )
    .expect("registered movement order is selected");
    (engine, selected)
}

#[test]
fn distance_continuation_preserves_door_action_points_and_authored_order_ids() {
    let (mut engine, selected) = transition_chain(&[
        (OrderType::Select, MapPoint::ZERO),
        (OrderType::PassingDoor, MapPoint::ZERO),
        (OrderType::TransitionCrouchingUp, MapPoint::ZERO),
        (OrderType::WalkingUpright, MapPoint::new(50.0, 60.0)),
        (OrderType::PassingDoor, MapPoint::ZERO),
    ]);
    let seq_id = selected.move_seq_id;
    let original_ids = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .unwrap()
        .orders
        .iter()
        .map(|order| order.order_id)
        .collect::<Vec<_>>();
    let new_id = engine.orders.next_order_id;

    engine.insert_transition_distance_continuation(selected);

    let element = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .unwrap();
    assert_eq!(
        element
            .orders
            .iter()
            .map(|order| order.order_type)
            .collect::<Vec<_>>(),
        [
            OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel,
            OrderType::Select,
            OrderType::PassingDoor,
            OrderType::TransitionCrouchingUp,
            OrderType::WalkingUpright,
            OrderType::WalkingUpright,
            OrderType::PassingDoor,
        ]
    );
    let continuation = &element.orders[4];
    assert!(continuation.transition_distance_continuation);
    assert_eq!(continuation.order_id.get(), new_id);
    assert_eq!((continuation.target_x, continuation.target_y), (20.0, 30.0));
    assert!(continuation.reverse);
    assert!(!continuation.compute_direction);
    assert_eq!(continuation.tolerance, 0.25);
    assert_eq!(
        element
            .orders
            .iter()
            .filter(|order| !order.transition_distance_continuation)
            .map(|order| order.order_id)
            .collect::<Vec<_>>(),
        original_ids
    );
    assert_eq!(
        (element.orders[5].target_x, element.orders[5].target_y),
        (50.0, 60.0)
    );
    assert_eq!(engine.orders.next_order_id, new_id + 1);
}

#[test]
fn exhausted_transition_removes_zero_destination_callbacks_without_allocating() {
    let (mut engine, selected) = transition_chain(&[
        (OrderType::Select, MapPoint::ZERO),
        (OrderType::PassingDoor, MapPoint::ZERO),
    ]);
    let seq_id = selected.move_seq_id;
    let first_id = selected.order_id.unwrap();
    let next_id = engine.orders.next_order_id;

    engine.insert_transition_distance_continuation(selected);

    let element = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .unwrap();
    assert_eq!(element.orders.len(), 1);
    assert_eq!(element.current_order().unwrap().order_id, first_id);
    assert_eq!(engine.orders.next_order_id, next_id);
}
