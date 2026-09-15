use super::*;

#[test]
fn climb_orders_keep_start_and_done_inside_entity_seek_routes() {
    use crate::element::{ActionState, Command, Entity, Posture};
    use crate::sequence::{
        MoveFlags, SequenceElement, SequenceElementData, SequencePriority, SequenceState,
    };
    use crate::sprite_script::SpriteScript;
    use std::sync::Arc;

    for (action, posture, lift_type) in [
        (
            OrderType::ClimbingLadderUp,
            Posture::OnLadder,
            crate::sector::LiftType::Ladder,
        ),
        (
            OrderType::ClimbingWallDown,
            Posture::OnWall,
            crate::sector::LiftType::Wall,
        ),
    ] {
        let mut engine = EngineInner::new();
        let sector = crate::engine::test_support::ensure_ordinary_sector(&mut engine, 1, 0);
        {
            let level = Arc::make_mut(&mut engine.world.fast_grid_mut().level);
            let lift = &mut level.sectors[0];
            lift.sector_type = crate::sector::SectorType::LIFT;
            lift.lift_type = Some(lift_type);
            lift.low_exit_point = Some(MapPoint::new(100.0, 200.0));
            lift.high_exit_point = Some(MapPoint::new(100.0, 0.0));
        }
        let script = SpriteScript {
            action_id: action as u16,
            action_done: 1,
            average_speed: 1.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 3,
            frame_ids: vec![1, 2, 3],
            delays: vec![0; 3],
            distances: vec![1; 3],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
            sound_ids: vec![0; 3],
        };
        let mut conversion = crate::engine::test_support::unmapped_conversion();
        conversion[action as usize] = 0;
        let mut pc = crate::engine::test_support::actors::unbound_pc(posture);
        pc.element.sprite =
            crate::sprite::Sprite::new(Arc::new(vec![script; 16]), Arc::new(conversion));
        pc.element.active = true;
        pc.element.set_position_map(MapPoint::new(100.0, 100.0));
        pc.element.set_sector(Some(sector));
        pc.element
            .sprite
            .position_iface
            .set_anti_collision_on(false);
        pc.actor.action_state = ActionState::Moving;
        let owner = engine.add_test_entity(Entity::Pc(pc));
        let mut target = crate::engine::test_support::actors::unbound_pc(Posture::Upright);
        target.element.active = false;
        target.element.set_position_map(MapPoint::new(100.0, 300.0));
        target.element.set_sector(Some(sector));
        let target = engine.add_test_entity(Entity::Pc(target));
        let actor = engine
            .get_entity_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap();
        actor.seek_target = Some(target);
        actor.last_seek_target_position = MapPoint::new(100.0, 300.0);
        actor.seek_distance = 5.0;
        actor.seek_refresh_wait = 20;

        let mut movement = SequenceElement::new_movement(1, Command::MoveOk, Some(owner), action);
        movement.state = SequenceState::InProgress;
        movement.priority = SequencePriority::Normal;
        if let SequenceElementData::Movement {
            flags,
            element,
            tolerance,
            destination,
            ..
        } = &mut movement.data
        {
            *flags = MoveFlags::SEEK;
            *element = Some(target);
            *tolerance = 5.0;
            *destination = MapPoint::new(100.0, 200.0);
        }
        let order_id = engine.orders.allocate_order_id();
        movement
            .orders
            .push_back(crate::order::Order::new(action, 100.0, 200.0, order_id));
        let sequence = engine.orders.sequence_manager.insert_element(movement);
        engine
            .orders
            .sequence_manager
            .start_sequence_level(sequence);
        engine.select_sequence_element(owner, Some((sequence, 0)));
        let sim = crate::sim_rng::test_context();
        let assets = engine.test_runtime_assets();

        for expected in [MotionState::Start, MotionState::Done] {
            engine.tick_actor_owner_envelopes(&sim, &assets);
            let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
            assert_eq!(actor.continuation.motion_state, expected, "{action:?}");
            assert_eq!(actor.installed_order.unwrap().order_id, order_id);
        }
        assert!(
            engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .unwrap()
                .current_order()
                .unwrap()
                .done
        );
    }
}

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
    let seq_id = engine.launch_element(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        movement,
    );
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
