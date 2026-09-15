use super::*;
use crate::coordinates::{MapPoint, WorldPoint3D};
use crate::element::{Command, EyeStatus, Posture};
use crate::sight_obstacle::{ObstaclePoint, SightObstacle};

fn fixture(radius: u16) -> (EngineInner, LevelAssets, EntityId, EntityId) {
    let (mut engine, assets, viewer, target) =
        crate::engine::ai::battle_decision_observation_tests::fixture(false);
    for (id, x) in [(viewer, 0.0), (target, 200.0)] {
        let entity = engine.get_entity_mut(id).unwrap();
        entity
            .element_data_mut()
            .set_position(WorldPoint3D::new(x, 0.0, 0.0));
        entity.element_data_mut().set_direction_instantly(4);
        entity.element_data_mut().active = true;
    }
    let npc = engine
        .get_entity_mut(viewer)
        .unwrap()
        .ai_actor_data_mut()
        .unwrap();
    npc.view_radius = radius;
    npc.view_direction = [1.0, 0.0];
    npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
    npc.eye_status = EyeStatus::LookForward;
    (engine, assets, viewer, target)
}

fn place(engine: &mut EngineInner, id: EntityId, point: WorldPoint3D) {
    engine
        .get_entity_mut(id)
        .unwrap()
        .element_data_mut()
        .set_position(point);
}

fn door_position(engine: &mut EngineInner, owner: EntityId, point: MapPoint) {
    let sector = engine
        .get_entity(owner)
        .unwrap()
        .element_data()
        .sector()
        .unwrap();
    let gate = crate::gate::DoorIndex::new(engine.script_domains.interactables.doors.len() as u32)
        .unwrap();
    engine
        .script_domains
        .interactables
        .doors
        .push(crate::gate::Door {
            point_in: point,
            point_out: point,
            sector_in: crate::sector::SectorNumber::new(1),
            sector_out: crate::sector::SectorNumber::new(1),
            sector_in_index: sector.arena_index(),
            sector_out_index: sector.arena_index(),
            ..Default::default()
        });
    let mut element = crate::sequence::SequenceElement::new_movement(
        1,
        Command::PassDoor,
        Some(owner),
        crate::order::OrderType::WalkingUpright,
    );
    let crate::sequence::SequenceElementData::Movement {
        gate_id, direction, ..
    } = &mut element.data
    else {
        unreachable!()
    };
    *gate_id = Some(gate);
    *direction = 1;
    let sequence = engine.orders.sequence_manager.insert_element(element);
    engine
        .orders
        .sequence_manager
        .start_sequence_level(sequence);
    engine.select_sequence_element(owner, Some((sequence, 0)));
    engine.element_in_progress(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        &mut Vec::new(),
        sequence,
        0,
    );
}

fn wall(x0: f32, y0: f32, x1: f32, y1: f32, top: f32) -> SightObstacle {
    let mut wall = SightObstacle::new_default(0);
    wall.obstacle_points = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
        .into_iter()
        .map(|(x, y)| ObstaclePoint {
            x,
            y,
            z_top: top,
            z_bottom: 0.0,
        })
        .collect();
    wall.top_plane_points = [[x0, y0, top], [x1, y0, top], [x0, y1, top]];
    wall.bottom_plane_points = [[x0, y0, 0.0], [x1, y0, 0.0], [x0, y1, 0.0]];
    wall.rebuild_geometry();
    wall
}

fn install_obstacle(engine: &mut EngineInner, assets: &mut LevelAssets, obstacle: SightObstacle) {
    assets.environment.static_sight_obstacles = std::sync::Arc::new(vec![obstacle]);
    engine.world.static_sight_obstacle_active = vec![true];
}

#[test]
fn detection_180_uses_raw_actor_xy_instead_of_selected_door_position() {
    let (mut engine, mut assets, viewer, target) = fixture(400);
    door_position(&mut engine, target, MapPoint::new(200.0, 30.0));
    install_obstacle(
        &mut engine,
        &mut assets,
        wall(95.0, 10.0, 105.0, 20.0, 80.0),
    );
    assert!(engine.live_ai_detects_180(&assets, viewer, target));
}

#[test]
fn detection_180_los_preserves_stored_world_point_bits() {
    let (mut engine, assets, viewer, target) = fixture(400);
    let raw = WorldPoint3D::new(1555.9615, f32::from_bits(1143810793), 46.78665);
    assert_ne!(((raw.y - raw.z) + raw.z).to_bits(), raw.y.to_bits());
    place(&mut engine, viewer, WorldPoint3D::new(1373.0, 595.0, 0.0));
    place(&mut engine, target, raw);
    crate::sight_obstacle::begin_parity_visibility_capture();
    assert!(engine.live_ai_detects_180(&assets, viewer, target));
    let queries = crate::sight_obstacle::take_parity_visibility_capture();
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].destination[0].to_bits(), raw.x.to_bits());
    assert_eq!(queries[0].destination[1].to_bits(), raw.y.to_bits());
    assert_eq!(
        queries[0].destination[2].to_bits(),
        (raw.z + crate::stealth::detection_z_for_posture(Posture::Upright, false)).to_bits()
    );
}

#[test]
fn detection_180_uses_live_radius_without_generic_standard_view_box() {
    let (mut engine, assets, viewer, target) = fixture(600);
    place(&mut engine, target, WorldPoint3D::new(500.0, 0.0, 0.0));
    assert!(engine.live_ai_detects_180(&assets, viewer, target));
}

#[test]
fn detection_180_close_sideways_shortcut_precedes_opaque_los() {
    let (mut engine, mut assets, viewer, target) = fixture(400);
    place(&mut engine, target, WorldPoint3D::new(0.0, 20.0, 0.0));
    install_obstacle(
        &mut engine,
        &mut assets,
        wall(-10.0, 10.0, 10.0, 15.0, 80.0),
    );
    crate::sight_obstacle::begin_parity_visibility_capture();
    assert!(engine.live_ai_detects_180(&assets, viewer, target));
    assert!(crate::sight_obstacle::take_parity_visibility_capture().is_empty());
}

#[test]
fn detection_180_projects_ground_radius_before_los() {
    let (mut engine, assets, viewer, target) = fixture(400);
    place(&mut engine, target, WorldPoint3D::new(399.0, 0.0, 0.0));
    crate::sight_obstacle::begin_parity_visibility_capture();
    assert!(!engine.live_ai_detects_180(&assets, viewer, target));
    assert!(crate::sight_obstacle::take_parity_visibility_capture().is_empty());
}

#[test]
fn live_visibility_reuses_surface_radius_until_the_next_frame_but_never_caches_zero() {
    let (mut engine, assets, viewer, target) = fixture(100);
    place(&mut engine, target, WorldPoint3D::new(80.0, 0.0, 0.0));
    assert!(engine.npc_is_detecting_human(&assets, viewer, target, 100));

    // The current radius now admits this actor, but the ground projection
    // calculated earlier in this frame still limits visibility.
    engine
        .get_entity_mut(viewer)
        .unwrap()
        .ai_actor_data_mut()
        .unwrap()
        .view_radius = 400;
    place(&mut engine, target, WorldPoint3D::new(200.0, 0.0, 0.0));
    crate::sight_obstacle::begin_parity_visibility_capture();
    assert!(!engine.npc_is_detecting_human(&assets, viewer, target, 100));
    assert!(crate::sight_obstacle::take_parity_visibility_capture().is_empty());
    engine.control.frame_counter = 101;
    crate::sight_obstacle::begin_parity_visibility_capture();
    assert!(engine.npc_is_detecting_human(&assets, viewer, target, 101));
    assert_eq!(
        crate::sight_obstacle::take_parity_visibility_capture().len(),
        1
    );

    // At an eye height equal to the view radius, the ground sphere projects
    // to zero. A changed radius must therefore be recomputed even this frame.
    let (mut engine, assets, viewer, target) = fixture(45);
    place(&mut engine, target, WorldPoint3D::new(30.0, 0.0, 0.0));
    assert!(!engine.npc_is_detecting_human(&assets, viewer, target, 100));
    engine
        .get_entity_mut(viewer)
        .unwrap()
        .ai_actor_data_mut()
        .unwrap()
        .view_radius = 400;
    assert!(engine.npc_is_detecting_human(&assets, viewer, target, 100));
}

#[test]
fn detection_180_rereads_active_unconscious_target() {
    let (mut engine, assets, viewer, target) = fixture(400);
    engine
        .get_entity_mut(target)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .unconscious = true;
    assert!(engine.live_ai_detects_180(&assets, viewer, target));
    engine
        .get_entity_mut(target)
        .unwrap()
        .element_data_mut()
        .active = false;
    assert!(!engine.live_ai_detects_180(&assets, viewer, target));
}

fn building_sector(engine: &mut EngineInner) -> crate::position_interface::SectorHandle {
    let mut sector = crate::engine::test_support::square_sector(
        7,
        0,
        MapPoint::ZERO,
        MapPoint::new(1000.0, 1000.0),
    );
    sector.sector_type |= crate::sector::SectorType::BUILDING;
    let index = engine.world.fast_grid_mut().add_sector(sector, 0);
    crate::position_interface::SectorHandle::new(7)
        .unwrap()
        .with_arena_index(crate::fast_find_grid::SectorIndex::new(index).unwrap())
}

#[test]
fn standalone_180_allows_target_inside_building_while_normal_and_360_reject_it() {
    let (mut engine, assets, viewer, target) = fixture(400);
    let building = building_sector(&mut engine);
    engine
        .get_entity_mut(target)
        .unwrap()
        .element_data_mut()
        .set_sector(Some(building));
    assert!(engine.live_ai_detects_180(&assets, viewer, target));
    assert!(!engine.npc_is_detecting_human(&assets, viewer, target, 100));
    assert!(!engine.patrol_member_visible(&assets, viewer, target));
}

#[test]
fn detecting_360_rereads_raw_active_actor_geometry_during_door_transit() {
    let (mut engine, assets, viewer, target) = fixture(400);
    place(&mut engine, target, WorldPoint3D::new(20.0, 0.0, 0.0));
    door_position(&mut engine, viewer, MapPoint::new(-900.0, 0.0));
    door_position(&mut engine, target, MapPoint::new(900.0, 0.0));
    for (posture, unconscious) in [(Posture::Upright, true), (Posture::Tied, false)] {
        engine
            .get_entity_mut(target)
            .unwrap()
            .element_data_mut()
            .set_posture(posture);
        engine
            .get_entity_mut(target)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .unconscious = unconscious;
        assert!(engine.patrol_member_visible(&assets, viewer, target));
    }
    engine
        .get_entity_mut(target)
        .unwrap()
        .element_data_mut()
        .active = false;
    assert!(!engine.patrol_member_visible(&assets, viewer, target));
}

#[test]
fn normal_detection_uses_raw_pass_door_geometry_and_current_active_flag() {
    let (mut engine, assets, viewer, target) = fixture(400);
    door_position(&mut engine, viewer, MapPoint::new(-900.0, 0.0));
    door_position(&mut engine, target, MapPoint::new(900.0, 0.0));
    engine
        .get_entity_mut(viewer)
        .unwrap()
        .element_data_mut()
        .hidden_in_building = true;
    engine
        .get_entity_mut(target)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .unconscious = true;
    assert!(engine.npc_is_detecting_human(&assets, viewer, target, 100));
    engine
        .get_entity_mut(target)
        .unwrap()
        .element_data_mut()
        .active = false;
    assert!(!engine.npc_is_detecting_human(&assets, viewer, target, 100));
}

#[test]
fn normal_detection_same_building_uses_current_body_and_door_gates() {
    for gate in 0..5 {
        let (mut engine, assets, viewer, target) = fixture(400);
        let building = building_sector(&mut engine);
        for id in [viewer, target] {
            engine
                .get_entity_mut(id)
                .unwrap()
                .element_data_mut()
                .set_sector(Some(building));
        }
        engine
            .get_entity_mut(target)
            .unwrap()
            .element_data_mut()
            .active = false;
        match gate {
            1 => {
                engine
                    .get_entity_mut(target)
                    .unwrap()
                    .pc_data_mut()
                    .unwrap()
                    .life_points = 0
            }
            2 => {
                engine
                    .get_entity_mut(target)
                    .unwrap()
                    .human_data_mut()
                    .unwrap()
                    .unconscious = true
            }
            3 => {
                door_position(&mut engine, target, MapPoint::new(200.0, 0.0));
            }
            4 => {
                // Physical choreography alone does not select a PassDoor command.
                engine
                    .get_entity_mut(target)
                    .unwrap()
                    .actor_data_mut()
                    .unwrap()
                    .active_door_pass = Some(crate::element::ActiveDoorPass {
                    door_index: crate::gate::DoorIndex::new(0).unwrap(),
                    direct: true,
                    position_direct: true,
                    triggers_fired: 0,
                })
            }
            _ => {}
        }
        assert_eq!(
            engine.npc_is_detecting_human(&assets, viewer, target, 100),
            gate == 0 || gate == 4,
            "gate {gate}"
        );
    }
}

#[test]
fn normal_detection_projects_radius_on_current_target_obstacle_top_plane() {
    let (mut engine, mut assets, viewer, target) = fixture(400);
    place(&mut engine, target, WorldPoint3D::new(380.0, 0.0, 0.0));
    assert!(engine.npc_is_detecting_human(&assets, viewer, target, 100));
    let mut platform = wall(350.0, -20.0, 410.0, 20.0, 200.0);
    platform.obstacle_type = crate::sight_obstacle::SIGHTOBSTACLE_PROJECTION_AREA;
    install_obstacle(&mut engine, &mut assets, platform);
    engine
        .get_entity_mut(target)
        .unwrap()
        .element_data_mut()
        .set_obstacle_index(
            crate::position_interface::ObstacleHandle::new(0),
            Some(crate::position_interface::PlaneZCoeffs {
                az: 0.0,
                bz: 0.0,
                dz: 200.0,
            }),
        );
    place(&mut engine, target, WorldPoint3D::new(380.0, 0.0, 200.0));
    assert!(!engine.npc_is_detecting_human(&assets, viewer, target, 100));
}

#[test]
fn look_there_broadcast_uses_raw_owner_range_during_door_transit() {
    let (mut engine, mut assets, owner, _) = fixture(400);
    let friend = engine.add_test_entity(crate::engine::test_support::actors::make_test_ai_soldier(
        Camp::Lacklandists,
    ));
    let sector = engine
        .get_entity(owner)
        .unwrap()
        .element_data()
        .sector()
        .map(|sector| sector.with_arena_index(crate::fast_find_grid::SectorIndex::new(0).unwrap()));
    engine
        .get_entity_mut(friend)
        .unwrap()
        .element_data_mut()
        .set_sector(sector);
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
    place(&mut engine, owner, WorldPoint3D::new(722.0, 1695.0, 160.0));
    place(&mut engine, friend, WorldPoint3D::new(713.0, 1663.0, 250.0));
    door_position(&mut engine, owner, MapPoint::new(1709.0, 2228.0));
    let ai = engine
        .get_entity_mut(friend)
        .unwrap()
        .enemy_ai_mut()
        .unwrap();
    ai.base.current_state = crate::ai::AiState::Default;
    ai.base.current_substate = crate::ai::Substate::DefaultOnPost;
    let hint = crate::ai::Position {
        x: 1154.0,
        y: 1860.0,
        sector,
        level: 0,
    };
    engine.execute_ai_look_there(&crate::sim_rng::test_context(), &assets, owner, hint, 100);
    let ai = engine.get_entity(friend).unwrap().enemy_ai().unwrap();
    assert_eq!(ai.base.seek_position, hint);
    assert_eq!(
        ai.base.current_substate,
        crate::ai::Substate::WonderingWatching
    );
}
