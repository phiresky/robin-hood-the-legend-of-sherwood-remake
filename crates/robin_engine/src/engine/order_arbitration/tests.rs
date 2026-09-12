use super::*;
use crate::element::{Command, ListenPhase, Posture};
use crate::engine::movement::{FailedPathRequest, PendingPathRequest, PendingPathRequestQueue};
use crate::engine::test_support::actors::make_test_soldier;
use crate::entity_id::{EntityId, EntityIdKind};
use crate::order::{Order, OrderType};
use crate::sequence::{SequenceElement, SequenceState};

#[test]
fn waiter_preparation_uses_only_world_and_orders_and_cancels_only_its_owner() {
    let mut world = WorldState::new();
    let mut orders = OrderRuntime::new();
    let owner = EntityId::new(0, EntityIdKind::Soldier);
    let other = EntityId::new(1, EntityIdKind::Soldier);
    world
        .entities
        .push(Some(make_test_soldier(Posture::Upright)));
    world
        .entities
        .push(Some(make_test_soldier(Posture::Upright)));

    let mut element = SequenceElement::new_movement(
        1,
        Command::MoveWaiting,
        Some(owner),
        OrderType::WalkingUpright,
    );
    element.retained_movement_goal = Some(crate::coordinates::MapPoint::new(12.0, 34.0));
    element.orders.push_back(Order::new(
        OrderType::Freezing,
        0.0,
        0.0,
        orders.allocate_order_id(),
    ));
    let waiter = orders.sequence_manager.launch_element(element);
    let other_sequence =
        orders
            .sequence_manager
            .launch_element(SequenceElement::new(1, Command::Wait, Some(other)));
    orders.pending_path_requests = PendingPathRequestQueue::restore_v48_waiting(vec![
        PendingPathRequest::test_request(owner, waiter, 0),
        PendingPathRequest::test_request(other, other_sequence, 0),
        PendingPathRequest::test_request(owner, waiter, 0),
    ]);
    orders.failed_path_requests = vec![
        FailedPathRequest::from_pending(PendingPathRequest::test_request(owner, waiter, 0), 0),
        FailedPathRequest::from_pending(
            PendingPathRequest::test_request(other, other_sequence, 0),
            0,
        ),
    ];

    prepare_cross_postponed_waiter(&mut world, &mut orders, waiter, 0);

    let element = orders.sequence_manager.get_element(waiter, 0).unwrap();
    assert_eq!(element.command, Command::Move);
    assert_eq!(element.state, SequenceState::Postponed);
    assert!(element.orders.is_empty());
    assert_eq!(element.retained_movement_goal, None);
    assert!(!orders.sequence_manager.is_registered_to_go(waiter, 0));
    let pending = orders.pending_path_requests.v48_waiting();
    // Cancellation retains the logical head as stale so it still consumes
    // this barrier's processing slot; only the later owner request is removed.
    assert_eq!(pending.len(), 2);
    assert_eq!(pending[0].owner, owner);
    assert_eq!(pending[1].owner, other);
    assert_eq!(
        serde_json::to_value(&orders.pending_path_requests).unwrap()["ignore_next_path"],
        true,
    );
    assert_eq!(orders.failed_path_requests.len(), 1);
    assert_eq!(orders.failed_path_requests[0].owner, other);
}

#[test]
fn mechanics_stop_clears_only_the_selected_elements_ability_mirror() {
    for mirror_matches_selected in [false, true] {
        let mut world = WorldState::new();
        let mut orders = OrderRuntime::new();
        let owner = EntityId::new(0, EntityIdKind::Soldier);
        world
            .entities
            .push(Some(make_test_soldier(Posture::Upright)));
        let selected = orders.sequence_manager.launch_element(SequenceElement::new(
            1,
            Command::Wait,
            Some(owner),
        ));
        orders.sequence_manager.element_in_progress(selected, 0);
        let actor = world
            .entities
            .get_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap();
        actor.active_ability.sequence_id = Some(selected);
        actor.active_ability.element_index = usize::from(!mirror_matches_selected);
        actor.active_ability.kind = Some(crate::movement::AbilityKind::Listen);
        actor.listen_phase = ListenPhase::CountingDown;
        actor.listen_wait_time = 12;

        stop_owner_active_mechanics(&mut world, &mut orders, owner);

        let actor = world.entities.get(owner).unwrap().actor_data().unwrap();
        if mirror_matches_selected {
            assert_eq!(actor.active_ability.kind, None);
            assert_eq!(actor.listen_phase, ListenPhase::Inactive);
            assert_eq!(actor.listen_wait_time, 0);
        } else {
            assert_eq!(
                actor.active_ability.kind,
                Some(crate::movement::AbilityKind::Listen)
            );
            assert_eq!(actor.listen_phase, ListenPhase::CountingDown);
            assert_eq!(actor.listen_wait_time, 12);
        }
    }
}
