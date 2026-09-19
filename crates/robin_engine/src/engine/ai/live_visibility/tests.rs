use super::*;
use crate::coordinates::{MapPoint, WorldPoint3D};
use crate::element::{EyeStatus, Posture};
use crate::engine::TickCtx;
use crate::sight_obstacle::{ObstaclePoint, SightObstacle};

fn fixture(radius: u16) -> (EngineInner, LevelAssets, EntityId, EntityId) {
    let (mut engine, assets, viewer, target) =
        crate::engine::ai::battle_decision_observation_tests::fixture(false);
    for (id, x) in [(viewer, 0.0), (target, 200.0)] {
        let entity = engine.ent_mut(id);
        entity
            .element_data_mut()
            .set_position(WorldPoint3D::new(x, 0.0, 0.0));
        entity.element_data_mut().set_direction_instantly(4);
        entity.element_data_mut().active = true;
    }
    let npc = engine.ent_mut(viewer).ai_actor_data_mut().unwrap();
    npc.view_radius = radius;
    npc.view_direction = [1.0, 0.0];
    npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
    npc.eye_status = EyeStatus::LookForward;
    (engine, assets, viewer, target)
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
    crate::engine::test_support::extra_engine_combat::enter_test_door(
        &mut engine,
        target,
        MapPoint::new(200.0, 30.0),
    );
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
    engine.place(viewer, WorldPoint3D::new(1373.0, 595.0, 0.0));
    engine.place(target, raw);
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
    engine.place(target, WorldPoint3D::new(500.0, 0.0, 0.0));
    assert!(engine.live_ai_detects_180(&assets, viewer, target));
}

#[test]
fn detection_180_close_sideways_shortcut_precedes_opaque_los() {
    let (mut engine, mut assets, viewer, target) = fixture(400);
    engine.place(target, WorldPoint3D::new(0.0, 20.0, 0.0));
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
    engine.place(target, WorldPoint3D::new(399.0, 0.0, 0.0));
    crate::sight_obstacle::begin_parity_visibility_capture();
    assert!(!engine.live_ai_detects_180(&assets, viewer, target));
    assert!(crate::sight_obstacle::take_parity_visibility_capture().is_empty());
}

#[test]
fn live_visibility_reuses_surface_radius_until_the_next_frame_but_never_caches_zero() {
    let (mut engine, assets, viewer, target) = fixture(100);
    engine.place(target, WorldPoint3D::new(80.0, 0.0, 0.0));
    assert!(engine.npc_is_detecting_human(&assets, viewer, target, 100));

    // The current radius now admits this actor, but the ground projection
    // calculated earlier in this frame still limits visibility.
    engine
        .ent_mut(viewer)
        .ai_actor_data_mut()
        .unwrap()
        .view_radius = 400;
    engine.place(target, WorldPoint3D::new(200.0, 0.0, 0.0));
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
    engine.place(target, WorldPoint3D::new(30.0, 0.0, 0.0));
    assert!(!engine.npc_is_detecting_human(&assets, viewer, target, 100));
    engine
        .ent_mut(viewer)
        .ai_actor_data_mut()
        .unwrap()
        .view_radius = 400;
    assert!(engine.npc_is_detecting_human(&assets, viewer, target, 100));
}

#[test]
fn detection_180_rereads_active_unconscious_target() {
    let (mut engine, assets, viewer, target) = fixture(400);
    engine.human_mut(target).unconscious = true;
    assert!(engine.live_ai_detects_180(&assets, viewer, target));
    engine.set_active(target, false);
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
    engine.elem_mut(target).set_sector(Some(building));
    assert!(engine.live_ai_detects_180(&assets, viewer, target));
    assert!(!engine.npc_is_detecting_human(&assets, viewer, target, 100));
    assert!(!engine.patrol_member_visible(&assets, viewer, target));
}

#[test]
fn detecting_360_rereads_raw_active_actor_geometry_during_door_transit() {
    let (mut engine, assets, viewer, target) = fixture(400);
    engine.place(target, WorldPoint3D::new(20.0, 0.0, 0.0));
    crate::engine::test_support::extra_engine_combat::enter_test_door(
        &mut engine,
        viewer,
        MapPoint::new(-900.0, 0.0),
    );
    crate::engine::test_support::extra_engine_combat::enter_test_door(
        &mut engine,
        target,
        MapPoint::new(900.0, 0.0),
    );
    for (posture, unconscious) in [(Posture::Upright, true), (Posture::Tied, false)] {
        engine.elem_mut(target).set_posture(posture);
        engine.human_mut(target).unconscious = unconscious;
        assert!(engine.patrol_member_visible(&assets, viewer, target));
    }
    engine.set_active(target, false);
    assert!(!engine.patrol_member_visible(&assets, viewer, target));
}

#[test]
fn normal_detection_uses_raw_pass_door_geometry_and_current_active_flag() {
    let (mut engine, assets, viewer, target) = fixture(400);
    crate::engine::test_support::extra_engine_combat::enter_test_door(
        &mut engine,
        viewer,
        MapPoint::new(-900.0, 0.0),
    );
    crate::engine::test_support::extra_engine_combat::enter_test_door(
        &mut engine,
        target,
        MapPoint::new(900.0, 0.0),
    );
    engine.elem_mut(viewer).hidden_in_building = true;
    engine.human_mut(target).unconscious = true;
    assert!(engine.npc_is_detecting_human(&assets, viewer, target, 100));
    engine.set_active(target, false);
    assert!(!engine.npc_is_detecting_human(&assets, viewer, target, 100));
}

#[test]
fn normal_detection_same_building_uses_current_body_and_door_gates() {
    for gate in 0..5 {
        let (mut engine, assets, viewer, target) = fixture(400);
        let building = building_sector(&mut engine);
        for id in [viewer, target] {
            engine.elem_mut(id).set_sector(Some(building));
        }
        engine.set_active(target, false);
        match gate {
            1 => engine.pc_mut(target).life_points = 0,
            2 => engine.human_mut(target).unconscious = true,
            3 => {
                crate::engine::test_support::extra_engine_combat::enter_test_door(
                    &mut engine,
                    target,
                    MapPoint::new(200.0, 0.0),
                );
            }
            4 => {
                // Physical choreography alone does not select a PassDoor command.
                engine
                    .ent_mut(target)
                    .position_iface_mut()
                    .set_door(crate::position_interface::DoorHandle::new(0).unwrap(), true);
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
    engine.place(target, WorldPoint3D::new(380.0, 0.0, 0.0));
    assert!(engine.npc_is_detecting_human(&assets, viewer, target, 100));
    let mut platform = wall(350.0, -20.0, 410.0, 20.0, 200.0);
    platform.obstacle_type = crate::sight_obstacle::SIGHTOBSTACLE_PROJECTION_AREA;
    install_obstacle(&mut engine, &mut assets, platform);
    engine.elem_mut(target).set_obstacle_index(
        crate::position_interface::ObstacleHandle::new(0),
        Some(crate::position_interface::PlaneZCoeffs {
            az: 0.0,
            bz: 0.0,
            dz: 200.0,
        }),
    );
    engine.place(target, WorldPoint3D::new(380.0, 0.0, 200.0));
    assert!(!engine.npc_is_detecting_human(&assets, viewer, target, 100));
}

#[test]
fn look_there_broadcast_uses_raw_owner_range_during_door_transit() {
    let (mut engine, mut assets, owner, _) = fixture(400);
    let friend = engine.add_test_entity(crate::engine::test_support::actors::make_test_ai_soldier(
        Camp::Lacklandists,
    ));
    let sector = engine
        .sector_of(owner)
        .map(|sector| sector.with_arena_index(crate::fast_find_grid::SectorIndex::new(0).unwrap()));
    engine.elem_mut(friend).set_sector(sector);
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.place(owner, WorldPoint3D::new(722.0, 1695.0, 160.0));
    engine.place(friend, WorldPoint3D::new(713.0, 1663.0, 250.0));
    crate::engine::test_support::extra_engine_combat::enter_test_door(
        &mut engine,
        owner,
        MapPoint::new(1709.0, 2228.0),
    );
    let ai = engine.enemy_mut(friend);
    ai.base.current_state = crate::ai::AiState::Default;
    ai.base.current_substate = crate::ai::Substate::DefaultOnPost;
    let hint = crate::ai::Position {
        x: 1154.0,
        y: 1860.0,
        sector,
        level: 0,
    };
    engine.execute_ai_look_there(
        TickCtx::new(&crate::sim_rng::test_context(), &assets),
        owner,
        hint,
        100,
    );
    let ai = engine.enemy(friend);
    assert_eq!(ai.base.seek_position, hint);
    assert_eq!(
        ai.base.current_substate,
        crate::ai::Substate::WonderingWatching
    );
}
