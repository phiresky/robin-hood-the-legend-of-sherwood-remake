use super::actor_line_crossing_eligible;
use crate::element::{Command, Posture};

#[test]
fn wall_and_ladder_climbers_still_check_elevation_lines() {
    assert!(actor_line_crossing_eligible(Posture::OnWall, false, true));
    assert!(actor_line_crossing_eligible(Posture::OnLadder, false, true));
    assert!(!actor_line_crossing_eligible(Posture::Flying, false, true));
    assert!(!actor_line_crossing_eligible(Posture::OnWall, true, true));
    assert!(!actor_line_crossing_eligible(Posture::OnWall, false, false));
}

#[test]
fn retired_seek_crossing_preserves_increment_and_direction() {
    use super::*;
    use crate::fast_find_grid::GridLine;
    use crate::sequence::{SequenceElement, SequenceState};

    for state in [SequenceState::InProgress, SequenceState::Terminated] {
        let mut engine = EngineInner::new();
        engine.world.fast_grid_mut().size_map(4, 4);
        engine.world.fast_grid_mut().allocate_layers(1);
        for index in 0..2 {
            engine.world.fast_grid_mut().add_line(
                GridLine::new_patch(
                    MapPoint::new(100.0, 131.0 + index as f32),
                    MapPoint::new(160.0, 131.0 + index as f32),
                    crate::patch::PatchIndex::new(index).unwrap(),
                ),
                0,
            );
        }
        let mut pc = crate::engine::test_support::actors::unbound_pc(Posture::Upright);
        let pi = &mut pc.element.sprite.position_iface;
        pi.set_map_position(MapPoint::new(130.0, 130.0));
        pi.new_move();
        pi.set_map_position(MapPoint::new(130.0, 134.0));
        pi.set_map_increment(MapVec::new(1.0, 0.0));
        pi.set_direction(crate::position_interface::Direction::from_raw(4));
        // A seek's synchronous removal clears its goal and invalidates the
        // increment cache, while retaining the last computed vector.
        pi.set_map_goal(MapPoint::ZERO);
        let owner = engine.add_test_entity(Entity::Pc(pc));
        let order_id = engine.orders.allocate_order_id();
        let mut movement =
            SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::WalkingUpright);
        movement.state = state;
        movement.orders.push_back(crate::order::Order::new(
            OrderType::WalkingUpright,
            150.0,
            140.0,
            order_id,
        ));
        let seq_id = engine.orders.sequence_manager.launch_element(movement);
        engine.finish_actor_movement(
            &crate::sim_rng::test_context(),
            &LevelAssets::new(),
            owner,
            MovementOwnerSelection {
                seq_id,
                elem_idx: 0,
                order_id: std::num::NonZeroU32::new(order_id.get()).unwrap(),
            },
            MovementCompletion::default(),
        );
        let pi = engine.get_entity(owner).unwrap().position_iface();
        if state == SequenceState::Terminated {
            assert_eq!(pi.raw_increment_map(), MapVec::new(1.0, 0.0));
            assert_eq!(pi.get_direction_goal().as_u8(), 4);
        } else {
            assert!(pi.raw_increment_map().x < 0.0);
            assert!(pi.raw_increment_map().y < 0.0);
            assert_ne!(pi.get_direction_goal().as_u8(), 4);
        }
    }
}
