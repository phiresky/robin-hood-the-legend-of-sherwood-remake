use super::*;
use crate::element::{Camp, Detectable, DetectableType, Posture};
use crate::engine::test_support::{actors::make_test_ai_soldier, square_sector};
use crate::sight_obstacle::{ObstaclePoint, SightObstacle};

#[test]
fn phalanx_shield_reestablish_uses_raw_door_passing_target_position() {
    for protecting in [false, true] {
        let (mut engine, assets, owner, _, target) = fixture();
        engine.scripts.mission = Some(crate::engine::test_support::asm::empty_mission_script(
            "phalanx_gate.scs",
        ));
        let raw = crate::coordinates::WorldPoint3D::new(1137.7087, 1652.301 + 150.001, 150.001);
        engine
            .world
            .entities
            .get_mut(target)
            .unwrap()
            .element_data_mut()
            .set_position(raw);
        let sector = engine
            .world
            .entities
            .get(target)
            .unwrap()
            .element_data()
            .sector()
            .unwrap();
        engine
            .script_domains
            .interactables
            .doors
            .push(crate::gate::Door {
                point_in: MapPoint::new(1158.0, 1627.0),
                point_out: MapPoint::new(1158.0, 1627.0),
                sector_in: crate::sector::SectorNumber::new(1),
                sector_out: crate::sector::SectorNumber::new(1),
                sector_in_index: sector.arena_index(),
                sector_out_index: sector.arena_index(),
                ..Default::default()
            });
        let mut pass = crate::sequence::SequenceElement::new_movement(
            1,
            crate::element::Command::PassDoor,
            Some(target),
            crate::order::OrderType::WalkingUpright,
        );
        let crate::sequence::SequenceElementData::Movement {
            gate_id, direction, ..
        } = &mut pass.data
        else {
            unreachable!()
        };
        *gate_id = Some(crate::gate::DoorIndex::new(0).unwrap());
        *direction = 1;
        let sequence = engine.orders.sequence_manager.insert_element(pass);
        engine
            .orders
            .sequence_manager
            .start_sequence_level(sequence);
        engine.select_sequence_element(target, Some((sequence, 0)));
        engine.element_in_progress(
            &crate::sim_rng::test_context(),
            &LevelAssets::new(),
            &mut Vec::new(),
            sequence,
            0,
        );
        assert_eq!(
            engine.live_ai_position(target).map_point(),
            MapPoint::new(1158.0, 1627.0)
        );
        let ai = engine.enemy_ai_mut(owner, "phalanx shield fixture");
        ai.base.current_state = crate::ai::AiState::Attacking;
        ai.base.current_substate = if protecting {
            Substate::AttackingProtectingWithShield
        } else {
            Substate::AttackingPhalanx
        };
        ai.base.primary_target = Some(AiEntityHandle::new(target.index()));
        engine
            .world
            .entities
            .get_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .action_state = crate::element::ActionState::MovingShield;
        engine.enter_ai_think_frame(owner);
        if protecting {
            assert!(engine.execute_ai_shield_expected_event(
                &crate::sim_rng::test_context(),
                &assets,
                owner,
                crate::ai::StimulusType::EventTimer
            ));
        } else {
            engine.execute_ai_phalanx_timer(&crate::sim_rng::test_context(), &assets, owner);
        }
        let command = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|s| s.elements.iter())
            .find(|e| e.owner == Some(owner) && e.command == crate::element::Command::RaiseShield)
            .unwrap();
        assert!(
            matches!(command.get_property(crate::sequence::Field::ShieldDangerPoint),
        Some(crate::sequence::FieldValue::Point3D { x, y, z })
            if x.to_bits() == raw.x.to_bits() && y.to_bits() == raw.y.to_bits() && z.to_bits() == raw.z.to_bits())
        );
    }
}

fn fixture() -> (EngineInner, LevelAssets, EntityId, EntityId, EntityId) {
    let mut engine = EngineInner::new();
    engine.world.fast_grid_mut().size_map(40, 40);
    engine.world.fast_grid_mut().allocate_layers(1);
    let index = engine.world.fast_grid_mut().add_sector(
        square_sector(
            1,
            0,
            MapPoint::new(-500.0, -500.0),
            MapPoint::new(4000.0, 4000.0),
        ),
        0,
    );
    let sector = crate::position_interface::SectorHandle::new(1)
        .unwrap()
        .with_arena_index(crate::fast_find_grid::SectorIndex::new(index).unwrap());
    let left = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let right = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let target = engine.add_test_entity(make_test_ai_soldier(Camp::Royalists));
    for id in [left, right, target] {
        let entity = engine.world.entities.get_mut(id).unwrap();
        entity
            .element_data_mut()
            .set_sector_topology(Some(sector), sector.arena_index());
        entity
            .element_data_mut()
            .set_position_map(MapPoint::new(0.0, 0.0));
        entity.element_data_mut().set_direction_instantly(4);
        let npc = entity.npc_data_mut().unwrap();
        npc.life_points = 100;
        npc.view_radius = 1000;
        entity.ai_controller_mut().unwrap().owner_entity_id = Some(id);
    }
    let mut assets = LevelAssets::new();
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
    engine
        .enemy_ai_mut(left, "test left link")
        .right_combat_neighbour = Some(AiEntityHandle::new(right.index()));
    engine
        .enemy_ai_mut(right, "test right link")
        .left_combat_neighbour = Some(AiEntityHandle::new(left.index()));
    (engine, assets, left, right, target)
}

fn retain(engine: &mut EngineInner, member: EntityId, target: EntityId) {
    engine
        .enemy_ai_mut(member, "test retained target")
        .list_them
        .push(target.index());
}

fn detect(engine: &mut EngineInner, member: EntityId, target: EntityId) {
    engine
        .world
        .entities
        .get_mut(member)
        .unwrap()
        .ai_actor_data_mut()
        .unwrap()
        .detectable_lists[DetectableType::Enemy as usize]
        .push(Detectable {
            element: Some(target),
            detectable_type: DetectableType::Enemy,
            ..Default::default()
        });
}

fn targets(engine: &EngineInner, member: EntityId) -> &[u32] {
    &engine.enemy_ai(member, "test formation targets").list_them
}

#[test]
fn attack_gate_uses_literal_body_distance_including_elevation() {
    let (mut engine, assets, left, right, target) = fixture();
    engine
        .world
        .entities
        .get_mut(target)
        .unwrap()
        .element_data_mut()
        .set_position(crate::coordinates::WorldPoint3D::new(90.0, 80.0, 80.0));
    retain(&mut engine, right, target);
    let ai = engine.enemy_ai_mut(right, "test formation state");
    ai.base.current_state = crate::ai::AiState::Attacking;
    ai.base.current_substate = Substate::AttackingPhalanx;
    assert!(engine.live_ai_position(target).x < archer::PHALANX_ATTACK_DISTANCE as f32);
    engine.enter_ai_think_frame(right);
    assert!(!engine.reconsider_live_phalanx(&crate::sim_rng::test_context(), &assets, right));
    let ai = engine.enemy_ai(right, "test formation retained");
    assert_eq!(ai.base.current_substate, Substate::AttackingPhalanx);
    assert_eq!(
        ai.left_combat_neighbour,
        Some(AiEntityHandle::new(left.index()))
    );
}

#[test]
fn nearest_enemy_truncates_before_stable_tie_breaking() {
    let (mut engine, mut assets, left, _, first) = fixture();
    let second = engine.add_test_entity(make_test_ai_soldier(Camp::Royalists));
    let sector = engine
        .world
        .entities
        .get(first)
        .unwrap()
        .element_data()
        .sector();
    engine
        .world
        .entities
        .get_mut(second)
        .unwrap()
        .element_data_mut()
        .set_sector(sector);
    engine
        .world
        .entities
        .get_mut(second)
        .unwrap()
        .npc_data_mut()
        .unwrap()
        .life_points = 100;
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
    for (id, x) in [(first, 120.9), (second, 120.1)] {
        engine
            .world
            .entities
            .get_mut(id)
            .unwrap()
            .element_data_mut()
            .set_position_map(MapPoint::new(x, 0.0));
        retain(&mut engine, left, id);
    }
    engine.reinitialize_live_phalanx_enemies(&assets, left);
    assert_eq!(targets(&engine, left), &[first.index(), second.index()]);
    engine
        .world
        .entities
        .get_mut(second)
        .unwrap()
        .element_data_mut()
        .set_position_map(MapPoint::new(119.9, 0.0));
    engine.reinitialize_live_phalanx_enemies(&assets, left);
    assert_eq!(targets(&engine, left), &[second.index(), first.index()]);
}

#[test]
fn each_member_uses_its_own_view_radius() {
    let (mut engine, assets, left, right, target) = fixture();
    engine
        .world
        .entities
        .get_mut(left)
        .unwrap()
        .npc_data_mut()
        .unwrap()
        .view_radius = 300;
    engine
        .world
        .entities
        .get_mut(right)
        .unwrap()
        .npc_data_mut()
        .unwrap()
        .view_radius = 100;
    engine
        .world
        .entities
        .get_mut(target)
        .unwrap()
        .element_data_mut()
        .set_position_map(MapPoint::new(150.0, 0.0));
    retain(&mut engine, right, target);
    engine.reinitialize_live_phalanx_enemies(&assets, left);
    assert!(targets(&engine, left).is_empty());
    assert!(targets(&engine, right).is_empty());
}

#[test]
fn raw_target_coordinates_precede_detection_offset() {
    let (mut engine, assets, left, _, target) = fixture();
    engine
        .world
        .entities
        .get_mut(left)
        .unwrap()
        .element_data_mut()
        .set_position(crate::coordinates::WorldPoint3D::new(
            931.252, 1726.3948, 0.0,
        ));
    let entity = engine.world.entities.get_mut(target).unwrap();
    entity.set_posture(Posture::LeaningOut);
    entity.element_data_mut().set_direction_instantly(11);
    entity
        .element_data_mut()
        .set_position(crate::coordinates::WorldPoint3D::new(
            1029.8252, 1982.2124, 136.09698,
        ));
    retain(&mut engine, left, target);
    crate::sight_obstacle::begin_parity_visibility_capture();
    engine.reinitialize_live_phalanx_enemies(&assets, left);
    let queries = crate::sight_obstacle::take_parity_visibility_capture();
    assert_eq!(targets(&engine, left), &[target.index()]);
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].destination[1].to_bits(), 1_157_160_897);
}

#[test]
fn inactive_link_is_traversed_and_receives_the_completed_enemy_list() {
    let (mut engine, assets, left, right, target) = fixture();
    engine
        .world
        .entities
        .get_mut(target)
        .unwrap()
        .element_data_mut()
        .set_position_map(MapPoint::new(200.0, 0.0));
    retain(&mut engine, left, target);
    let entity = engine.world.entities.get_mut(right).unwrap();
    entity.element_data_mut().active = false;
    entity.human_data_mut().unwrap().unconscious = true;
    entity.npc_data_mut().unwrap().life_points = 0;
    engine.reinitialize_live_phalanx_enemies(&assets, left);
    assert_eq!(targets(&engine, left), &[target.index()]);
    assert_eq!(targets(&engine, right), &[target.index()]);
    assert_eq!(
        engine
            .world
            .entities
            .expect_enemy_ai(right, format_args!("test inactive target"))
            .base
            .primary_target,
        Some(AiEntityHandle::new(target.index()))
    );
}

#[test]
fn inactive_member_cannot_contribute_persistent_or_detectable_enemies() {
    let (mut engine, assets, left, right, target) = fixture();
    engine
        .world
        .entities
        .get_mut(target)
        .unwrap()
        .element_data_mut()
        .set_position_map(MapPoint::new(40.0, 0.0));
    retain(&mut engine, right, target);
    detect(&mut engine, right, target);
    engine
        .world
        .entities
        .get_mut(right)
        .unwrap()
        .element_data_mut()
        .active = false;
    crate::sight_obstacle::begin_parity_visibility_capture();
    engine.reinitialize_live_phalanx_enemies(&assets, left);
    let queries = crate::sight_obstacle::take_parity_visibility_capture();
    assert!(targets(&engine, left).is_empty());
    assert!(targets(&engine, right).is_empty());
    assert!(queries.is_empty());
}

#[test]
fn occlusion_rejects_persistent_and_detectable_enemies() {
    let (mut engine, mut assets, left, right, target) = fixture();
    engine
        .world
        .entities
        .get_mut(target)
        .unwrap()
        .element_data_mut()
        .set_position_map(MapPoint::new(200.0, 0.0));
    retain(&mut engine, right, target);
    detect(&mut engine, right, target);
    engine.reinitialize_live_phalanx_enemies(&assets, left);
    assert_eq!(targets(&engine, left), &[target.index()]);
    assets.environment.static_sight_obstacles = std::sync::Arc::new(vec![opaque_wall()]);
    engine.world.static_sight_obstacle_active = vec![true];
    engine.reinitialize_live_phalanx_enemies(&assets, left);
    assert!(targets(&engine, left).is_empty());
    assert!(targets(&engine, right).is_empty());
}

#[test]
fn rebuilding_live_lists_overwrites_prior_member_target_assignment() {
    let (mut engine, assets, left, right, target) = fixture();
    engine
        .world
        .entities
        .get_mut(target)
        .unwrap()
        .element_data_mut()
        .set_position_map(MapPoint::new(200.0, 0.0));
    retain(&mut engine, left, target);
    engine
        .enemy_ai_mut(right, "test old member target")
        .base
        .primary_target = Some(AiEntityHandle::new(left.index()));
    engine.reinitialize_live_phalanx_enemies(&assets, left);
    for member in [left, right] {
        let ai = engine.enemy_ai(member, "test assigned formation list");
        assert_eq!(ai.list_them, vec![target.index()]);
        assert_eq!(
            ai.base.primary_target,
            Some(AiEntityHandle::new(target.index()))
        );
    }
}

fn test_position(x: f32, y: f32) -> Position {
    Position {
        x,
        y,
        sector: None,
        level: 0,
    }
}

#[test]
fn phalanx_advance_uses_original_aspect_aware_normalization() {
    // Schema-14 Savegame_034/replay-009, frame 34182. Soldier 52 is
    // the center of the three-man phalanx and PC 167 is its target.
    // These are the literal normalized-vector results and
    // the aspect-adjusted normal calculation in the original game.
    let center = test_position(720.15155, 2198.4492);
    let target = test_position(967.95605, 2068.5835);
    let (forward, right) = phalanx_advance_vectors(target.map_point() - center.map_point());

    assert!((forward.x - 51.67761).abs() < 0.0001);
    assert!((forward.y - -27.082436).abs() < 0.0001);
    assert!((right.x - 16.863138).abs() < 0.0001);
    assert!((right.y - 10.586092).abs() < 0.0001);

    let new_center = (center.x + forward.x, center.y + forward.y);
    let left_slot = (new_center.0 - right.x, new_center.1 - right.y);
    let right_slot = (new_center.0 + right.x, new_center.1 + right.y);
    assert!((left_slot.0 - 754.966).abs() < 0.001);
    assert!((left_slot.1 - 2160.7808).abs() < 0.001);
    assert!((right_slot.0 - 788.6923).abs() < 0.001);
    assert!((right_slot.1 - 2181.953).abs() < 0.001);
}

fn opaque_wall() -> SightObstacle {
    let mut wall = SightObstacle::new_default(0);
    wall.obstacle_points = vec![
        ObstaclePoint {
            x: 95.0,
            y: -10.0,
            z_top: 80.0,
            z_bottom: 0.0,
        },
        ObstaclePoint {
            x: 105.0,
            y: -10.0,
            z_top: 80.0,
            z_bottom: 0.0,
        },
        ObstaclePoint {
            x: 105.0,
            y: 10.0,
            z_top: 80.0,
            z_bottom: 0.0,
        },
        ObstaclePoint {
            x: 95.0,
            y: 10.0,
            z_top: 80.0,
            z_bottom: 0.0,
        },
    ];
    wall.top_plane_points = [
        [95.0, -10.0, 80.0],
        [105.0, -10.0, 80.0],
        [95.0, 10.0, 80.0],
    ];
    wall.bottom_plane_points = [[95.0, -10.0, 0.0], [105.0, -10.0, 0.0], [95.0, 10.0, 0.0]];
    wall.rebuild_geometry();
    wall
}

#[test]
fn night_detection_orders_light_rays_before_target_los() {
    let (mut engine, assets, left, _, target) = fixture();
    engine
        .world
        .entities
        .get_mut(left)
        .unwrap()
        .element_data_mut()
        .set_position_map(MapPoint::new(500.0, 500.0));
    engine
        .world
        .entities
        .get_mut(left)
        .unwrap()
        .npc_data_mut()
        .unwrap()
        .view_radius = 500;
    engine
        .world
        .entities
        .get_mut(target)
        .unwrap()
        .element_data_mut()
        .set_position_map(MapPoint::new(600.0, 500.0));
    detect(&mut engine, left, target);
    engine.world.weather.ambiance = crate::engine::types::Ambiance::Night;
    let fast_grid = engine.world.fast_grid_mut();
    let barycentres = [(750.0, 500.0), (760.0, 510.0), (770.0, 490.0)];
    for (index, &(x, y)) in barycentres.iter().enumerate() {
        let points = vec![
            crate::coordinates::MapPoint::new(x - 4.0, y - 4.0),
            crate::coordinates::MapPoint::new(x + 4.0, y - 4.0),
            crate::coordinates::MapPoint::new(x + 4.0, y + 4.0),
            crate::coordinates::MapPoint::new(x - 4.0, y + 4.0),
        ];
        let mut bounding_box = crate::coordinates::MapBBox::new();
        for &point in &points {
            bounding_box.expand_point(point);
        }
        fast_grid.add_sector(
            crate::fast_find_grid::GridSector {
                points,
                bounding_box,
                sector_type: crate::sector::SectorType::SHADOW,
                layer: 0,
                sector_number: crate::sector::SectorNumber::new(index as i16 + 2),
                door_index: None,
                lift_type: None,
                lift_direction: 0,
                force_crouched: false,
                building_index: None,
                low_exit_point: None,
                high_exit_point: None,
                lowest_door_index: None,
                jump_line_indices: Vec::new(),
                gate_indices: Vec::new(),
                underlying_sector: None,
            },
            0,
        );
        std::sync::Arc::make_mut(&mut fast_grid.level)
            .shadow_data
            .insert(
                index as u32 + 1,
                crate::sector::ShadowData {
                    barycentre_2d: crate::coordinates::MapPoint::new(x, y),
                    barycentre_3d_x: x,
                    barycentre_3d_y: y,
                    barycentre_3d_z: 45.0,
                    radius: 4.0,
                },
            );
    }

    crate::sight_obstacle::begin_parity_visibility_capture();
    engine.reinitialize_live_phalanx_enemies(&assets, left);
    let queries = crate::sight_obstacle::take_parity_visibility_capture();
    assert_eq!(targets(&engine, left), &[target.index()]);
    assert_eq!(
        queries
            .iter()
            .map(|query| query.destination)
            .collect::<Vec<_>>(),
        vec![
            [750.0, 500.0, 45.0],
            [760.0, 510.0, 45.0],
            [770.0, 490.0, 45.0],
            [600.0, 500.0, 45.0],
        ]
    );
    assert!(queries.iter().all(|query| query.result));
    assert!(
        engine
            .ai
            .view_radius_cache
            .get(None, left, engine.control.frame_counter)
            .unwrap()
            > 0.0
    );
}
