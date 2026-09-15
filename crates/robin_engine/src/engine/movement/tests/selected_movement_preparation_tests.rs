use super::*;
use crate::element::{ActionState, Command, Entity, Posture};
use crate::entity_id::{ActorId, PcId};
use crate::sequence::{SequenceElement, SequenceId, SequenceManager};

#[test]
fn stale_selected_order_repairs_moving_state_without_clearing_sword_stance() {
    let mut entity = Entity::Pc(crate::engine::test_support::actors::unbound_pc(
        Posture::Upright,
    ));
    let actor_id = ActorId::Pc(PcId(7));
    let selected = MovementOwnerSelection {
        seq_id: SequenceId(999),
        elem_idx: 0,
        order_id: std::num::NonZeroU32::new(1).unwrap(),
    };
    entity.actor_data_mut().unwrap().action_state = ActionState::MovingSword;
    entity.actor_data_mut().unwrap().active_movement =
        ActiveMovement::new(selected.seq_id, selected.elem_idx);

    assert!(
        EngineInner::prepare_selected_movement_order(
            &mut entity,
            &SequenceManager::new(),
            selected,
            actor_id.into(),
            false,
        )
        .is_none()
    );
    let actor = entity.actor_data().unwrap();
    assert_eq!(actor.action_state, ActionState::WaitingSword);
    assert_eq!(actor.active_movement.sequence_id, None);
}

#[test]
fn prepared_order_retains_selected_front_and_literal_successor() {
    let mut entity = Entity::Pc(crate::engine::test_support::actors::unbound_pc(
        Posture::Upright,
    ));
    entity.actor_data_mut().unwrap().action_state = ActionState::Moving;
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(entity);
    let EntityId::Pc(pc_id) = owner else {
        unreachable!()
    };
    let actor_id = ActorId::Pc(pc_id);
    let mut movement = SequenceElement::new_movement(
        1,
        Command::Move,
        Some(actor_id.into()),
        OrderType::WalkingUpright,
    );
    let mut order = crate::order::Order::test_new(OrderType::WalkingUpright, 12.5, -7.0);
    order.reverse = true;
    order.tolerance = 0.25;
    let order_id = order.order_id;
    movement.orders.push_back(order);
    movement.orders.push_back(crate::order::Order::test_new(
        OrderType::WalkingUpright,
        40.0,
        18.0,
    ));
    let seq_id = engine.launch_element(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        movement,
    );
    engine.element_in_progress(
        &crate::sim_rng::test_context(),
        &crate::engine::LevelAssets::new(),
        &mut Vec::new(),
        seq_id,
        0,
    );

    let prepared = EngineInner::prepare_selected_movement_order(
        engine.world.entities.get_mut(owner).unwrap(),
        &engine.orders.sequence_manager,
        MovementOwnerSelection {
            seq_id,
            elem_idx: 0,
            order_id,
        },
        actor_id.into(),
        false,
    )
    .expect("selected in-progress movement must prepare");

    assert_eq!(prepared.goal, MapPoint::new(12.5, -7.0));
    assert_eq!(prepared.order_id, Some(order_id));
    assert_eq!(prepared.order_tolerance.to_bits(), 0.25_f32.to_bits());
    assert!(prepared.order_reverse);
    assert!(!prepared.is_final_waypoint);
    assert_eq!(
        prepared.next_destination_same_action,
        Some(MapPoint::new(40.0, 18.0))
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap()
            .orders
            .len(),
        2
    );
}
