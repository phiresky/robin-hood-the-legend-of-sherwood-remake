use super::*;
#[test]
fn absent_and_stale_selections_do_not_run_movement_or_completion() {
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(crate::element::Entity::Pc(
        crate::engine::test_support::actors::unbound_pc(crate::element::Posture::Upright),
    ));
    let order_id = engine.orders.allocate_order_id();
    let mut movement = crate::sequence::SequenceElement::new_movement(
        1,
        crate::element::Command::Move,
        Some(owner),
        OrderType::WalkingUpright,
    );
    movement.orders.push_back(crate::order::Order::new(
        OrderType::WalkingUpright,
        100.0,
        100.0,
        order_id,
    ));
    let seq_id = engine.orders.sequence_manager.launch_element(movement);
    let stale = MovementOwnerSelection {
        seq_id,
        elem_idx: 0,
        order_id: std::num::NonZeroU32::new(order_id.get().checked_add(1).unwrap()).unwrap(),
    };
    let before = engine
        .get_entity(owner)
        .unwrap()
        .element_data()
        .position_map();
    for selected in [None, Some(stale)] {
        let result = engine.tick_entity_movement_owner(
            &crate::sim_rng::test_context(),
            &LevelAssets::new(),
            owner,
            selected,
        );
        assert!(result.initial.is_none());
        assert!(result.post_completion_override.is_none());
        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .element_data()
                .position_map(),
            before
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(seq_id, 0)
                .unwrap()
                .current_order()
                .unwrap()
                .order_id,
            order_id
        );
    }
}
