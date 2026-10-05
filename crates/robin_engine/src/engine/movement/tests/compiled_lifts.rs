use super::*;

fn physical_stair_fixture() -> serde_json::Value {
    let mut document: serde_json::Value = serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/asset-lift.level.json"
    )))
    .unwrap();
    document["asset_geometry"]["motion_data"]["layers"][2][0]["obstacles"] = serde_json::json!([
        {"state_id": 1, "polygon": {"points": [[390,349],[410,249],[410,251],[390,351]]}}
    ]);
    document["asset_geometry"]["movement_transitions"] = serde_json::json!([{
        "id": "physical-stair-barrier", "waypoint": [350,320], "sector": 0, "layer": 0,
        "active": true, "definitive": false,
        "apply_polygon": {"points": []}, "no_apply_polygon": {"points": []},
        "motion_changes": [{"sector": 3, "layer": 2, "changing_obstacle": 0}]
    }]);
    document["asset_geometry"]["lifts"][0]["physical_navigation"] = serde_json::json!({
        "plane": [5.0, 0.0, -1950.0],
        "boundary": [[390,300],[410,300],[410,400],[390,400]],
        "obstacles": [{"motion_obstacle": 0, "polygon": [[390,349],[410,349],[410,351],[390,351]]}],
        "doors": [
            {"inside": [392,350,10], "middle": [390,350,0], "outside": [380,350,0]},
            {"inside": [408,350,90], "middle": [410,350,100], "outside": [420,350,100]}
        ]
    });
    document
}

fn edge_on_physical_stair_fixture() -> serde_json::Value {
    let mut document = physical_stair_fixture();
    let geometry = &mut document["asset_geometry"];
    geometry["movement_transitions"] = serde_json::json!([]);
    for (layer, points) in [
        serde_json::json!([[380, 170], [420, 170], [420, 200], [380, 200]]),
        serde_json::json!([[380, 200], [420, 200], [420, 230], [380, 230]]),
        serde_json::json!([[380, 200], [420, 200], [420, 200], [380, 200]]),
    ]
    .into_iter()
    .enumerate()
    {
        geometry["motion_data"]["layers"][layer][0]["polygon"]["points"] = points;
        geometry["motion_data"]["layers"][layer][0]["obstacles"] = serde_json::json!([]);
    }
    let template = geometry["sight_obstacles"][1].clone();
    let mut receivers = Vec::new();
    for (sector, z, y) in [(0, 100, 270), (1, 200, 400)] {
        let mut receiver = template.clone();
        receiver["projection_area"] = serde_json::json!([sector, sector]);
        receiver["points"] = serde_json::json!([
            {"x":380,"y":y,"z_bottom":z,"z_top":z},
            {"x":420,"y":y,"z_bottom":z,"z_top":z},
            {"x":420,"y":y+30,"z_bottom":z,"z_top":z},
            {"x":380,"y":y+30,"z_bottom":z,"z_top":z}
        ]);
        receivers.push(receiver);
    }
    geometry["sight_obstacles"] = serde_json::json!(receivers);
    let lift = &mut geometry["lifts"][0];
    lift["motion_area_index"] = serde_json::json!(2);
    for (index, outside_y) in [190, 210].into_iter().enumerate() {
        let door = &mut lift["doors"][index];
        door["sector_in"] = serde_json::json!(2);
        door["sector_out"] = serde_json::json!(index);
        door["point_mid"] = serde_json::json!([400, 200]);
        door["point_in"] = serde_json::json!([400, 200]);
        door["point_out"] = serde_json::json!([400, outside_y]);
    }
    lift["physical_navigation"] = serde_json::json!({
        "plane": [0.0,1.0,-200.0],
        "boundary": [[380,300],[420,300],[420,400],[380,400]],
        "obstacles": [],
        "doors": [
            {"inside":[400,310,110],"middle":[400,300,100],"outside":[400,290,100]},
            {"inside":[400,390,190],"middle":[400,400,200],"outside":[400,410,200]}
        ]
    });
    document
}

fn physical_walker(
    engine: &mut EngineInner,
    assets: &mut LevelAssets,
    sector: u16,
    source: [f32; 2],
    destination: [f32; 2],
) -> crate::element::EntityId {
    let definition = assets.navigation.physical_stairs[&sector]
        .definition
        .clone();
    let plane =
        robin_level_data::stair_navigation::StairNavigationPlane::new(definition.plane).unwrap();
    let source_world = plane.world_position(source.map(f64::from)).unwrap();
    let destination = plane
        .world_position(destination.map(f64::from))
        .unwrap()
        .map(|v| v as f32);
    let index = engine.world.fast_grid.level.sector_number_map
        [&crate::sector::SectorNumber::new(sector as i16)];
    let handle = crate::position_interface::SectorHandle::new(sector)
        .unwrap()
        .with_arena_index(crate::fast_find_grid::SectorIndex::new(index as u32).unwrap());
    let owner = walking_pc(
        engine,
        assets,
        MapPoint::new(
            source_world[0] as f32,
            (source_world[1] - source_world[2]) as f32,
        ),
        2,
        handle,
    );
    let [a, b, c] = definition.plane.map(|v| v as f32);
    engine
        .ent_mut(owner)
        .position_iface_mut()
        .set_obstacle_at_ground_position(
            None,
            Some(crate::position_interface::PlaneZCoeffs {
                az: a,
                bz: b,
                dz: c,
            }),
            crate::coordinates::GroundPoint::new(source[0], source[1]),
        )
        .unwrap();
    let goal =
        crate::coordinates::WorldPoint3D::new(destination[0], destination[1], destination[2]);
    let map = goal.to_map();
    let mut order = crate::order::Order::new(
        OrderType::WalkingStairs,
        map.x,
        map.y,
        engine.orders.allocate_order_id(),
    );
    order.physical_stair = Some(sector);
    order.destination_3d = destination;
    // Physical identity and destination must survive the native order encoding.
    let order = bitcode::decode(&bitcode::encode(&order)).unwrap();
    let mut movement =
        SequenceElement::new_movement(1, Command::MoveOk, Some(owner), OrderType::WalkingStairs);
    movement.orders.push_back(order);
    let sequence = engine.t_launch_in_progress(assets, movement);
    engine.select_sequence_element(owner, Some((sequence, 0)));
    owner
}

#[test]
fn physical_stair_actor_executes_edge_on_motion_in_both_directions() {
    for (source, goal) in [([400., 320.], [400., 380.]), ([400., 380.], [400., 320.])] {
        let (mut engine, mut assets) =
            compiled_walkway(&serde_json::to_vec(&edge_on_physical_stair_fixture()).unwrap());
        let owner = physical_walker(&mut engine, &mut assets, 2, source, goal);
        let mut reached = false;
        let mut moves = 0;
        for tick in 0..300 {
            let before = engine.ent(owner).position_iface().get_position();
            engine.t_tick_actor_owner_envelopes(&assets);
            let position = engine.ent(owner).position_iface().get_position();
            assert!(
                (position.z - (position.y - 200.)).abs() < 0.001,
                "{position:?}"
            );
            assert!((engine.ent(owner).element_data().position_map().y - 200.).abs() < 0.001);
            if position != before {
                moves += 1;
            }
            if tick == 10 {
                let saved = bitcode::encode(engine.ent(owner).position_iface());
                *engine.ent_mut(owner).position_iface_mut() = bitcode::decode(&saved).unwrap();
            }
            if [position.x, position.y] == goal {
                reached = true;
                break;
            }
        }
        assert!(
            reached && moves > 20,
            "physical actor failed: {:?}, moves={moves}",
            engine.ent(owner).position_iface().get_position()
        );
    }
}

#[test]
fn physical_stair_actor_stops_for_live_control_and_resumes_when_reopened() {
    let (mut engine, mut assets) =
        compiled_walkway(&serde_json::to_vec(&physical_stair_fixture()).unwrap());
    let sim = crate::sim_rng::test_context();
    let patch = crate::patch::PatchIndex::new(0).unwrap();
    engine.apply_patch(TickCtx::new(&sim, &assets), patch);
    let owner = physical_walker(&mut engine, &mut assets, 3, [400., 320.], [400., 380.]);
    for _ in 0..8 {
        engine.t_tick_actor_owner_envelopes(&assets);
    }
    let stopped = engine.ent(owner).position_iface().get_position();
    assert!(stopped.y > 320. && stopped.y < 340.);
    engine.apply_patch(TickCtx::new(&sim, &assets), patch);
    for _ in 0..8 {
        engine.t_tick_actor_owner_envelopes(&assets);
    }
    assert_eq!(engine.ent(owner).position_iface().get_position(), stopped);
    assert!(
        !engine.ent(owner).element_data().unreachable,
        "projected overlap must not crush a physically clear actor"
    );
    engine.apply_patch(TickCtx::new(&sim, &assets), patch);
    for _ in 0..120 {
        engine.t_tick_actor_owner_envelopes(&assets);
        if engine.ent(owner).position_iface().get_position().y == 380. {
            break;
        }
    }
    assert_eq!(
        engine.ent(owner).position_iface().get_position().y,
        380.,
        "state={:?}, order={:?}, blocked={}",
        engine.world.pathfinder.states,
        engine.actor_installed_order(owner),
        engine.ent(owner).position_iface().blocked_count
    );
}

#[test]
fn physical_stair_control_crushes_an_actor_inside_its_physical_obstacle() {
    let (mut engine, mut assets) =
        compiled_walkway(&serde_json::to_vec(&physical_stair_fixture()).unwrap());
    let sim = crate::sim_rng::test_context();
    let patch = crate::patch::PatchIndex::new(0).unwrap();
    engine.apply_patch(TickCtx::new(&sim, &assets), patch);
    let owner = physical_walker(&mut engine, &mut assets, 3, [400., 350.], [400., 380.]);
    assert!(!engine.ent(owner).element_data().unreachable);
    engine.apply_patch(TickCtx::new(&sim, &assets), patch);
    assert!(engine.ent(owner).element_data().unreachable);
}

#[test]
fn physical_stair_actor_avoids_neighbour_at_the_same_screen_position() {
    let (mut engine, mut assets) =
        compiled_walkway(&serde_json::to_vec(&edge_on_physical_stair_fixture()).unwrap());
    let owner = physical_walker(&mut engine, &mut assets, 2, [400., 320.], [400., 380.]);
    let blocker = physical_walker(&mut engine, &mut assets, 2, [400., 350.], [400., 350.]);
    let mut detoured = false;
    let mut reached = false;
    for _ in 0..300 {
        engine.t_tick_actor_owner_envelopes(&assets);
        let position = engine.ent(owner).position_iface().get_position();
        assert_eq!(engine.ent(blocker).position_iface().get_position().y, 350.);
        assert!(
            ((position.x - 400.).abs() - 5.)
                .max(0.)
                .hypot(((position.y - 350.).abs() - 2.).max(0.))
                >= 3.99,
            "actor overlapped its neighbour: {position:?}"
        );
        detoured |= (position.x - 400.).abs() > 8.;
        if position.y == 380. && position.x == 400. {
            reached = true;
            break;
        }
    }
    assert!(
        reached && detoured,
        "physical neighbour was ignored or blocked a usable route"
    );
}

#[test]
fn physical_stair_loading_routes_against_live_and_restored_obstacle_state() {
    let bytes = serde_json::to_vec(&physical_stair_fixture()).unwrap();
    let (mut engine, assets) = compiled_walkway(&bytes);
    let stairs = &assets.navigation.physical_stairs;
    assert_eq!(stairs.len(), 1);
    let stair = &stairs[&3];
    let route = |pathfinder: &crate::pathfinder::PathFinder| {
        stair
            .route(
                pathfinder,
                [400.0, 320.0],
                [400.0, 380.0],
                crate::coordinates::MoveBoxHalfDiagonal::new(6.0, 3.0),
            )
            .unwrap()
    };
    assert!(route(&engine.world.pathfinder).is_none());
    let saved = bitcode::encode(&engine.world.pathfinder);
    let sim = crate::sim_rng::test_context();
    let patch = crate::patch::PatchIndex::new(0).unwrap();
    engine.apply_patch(TickCtx::new(&sim, &assets), patch);
    let path = route(&engine.world.pathfinder).expect("opening the live barrier frees the stair");
    assert_eq!(path.first(), Some(&[400.0, 320.0]));
    assert_eq!(path.last(), Some(&[400.0, 380.0]));
    let restored: crate::pathfinder::PathFinder = bitcode::decode(&saved).unwrap();
    assert!(route(&restored).is_none());
    engine.reset_patch(TickCtx::new(&sim, &assets), patch);
    assert!(route(&engine.world.pathfinder).is_none());
    let serialized = serde_json::to_vec(&assets.navigation.physical_stairs).unwrap();
    let decoded: std::collections::BTreeMap<u16, crate::stair_navigation::BoundPhysicalStair> =
        serde_json::from_slice(&serialized).unwrap();
    assert!(
        decoded[&3]
            .route(
                &restored,
                [400.0, 320.0],
                [400.0, 380.0],
                crate::coordinates::MoveBoxHalfDiagonal::new(6.0, 3.0)
            )
            .unwrap()
            .is_none()
    );
}

#[test]
fn physical_stair_descriptor_rejects_missing_collision_and_mismatched_doors() {
    for (field, value) in [
        ("obstacles", serde_json::json!([])),
        (
            "obstacles",
            serde_json::json!([{"motion_obstacle": 1, "polygon": [[399,300],[401,300],[401,400],[399,400]]}]),
        ),
        ("doors", serde_json::json!([])),
        ("plane", serde_json::json!([5.0, 0.0, -1940.0])),
        (
            "boundary",
            serde_json::json!([[390, 300], [400, 300], [410, 300]]),
        ),
        (
            "boundary",
            serde_json::json!([[390, 300], [410, 400], [410, 300], [390, 400], [400, 420]]),
        ),
        (
            "boundary",
            serde_json::json!([[490, 300], [510, 300], [510, 400], [490, 400]]),
        ),
    ] {
        let mut document = physical_stair_fixture();
        document["asset_geometry"]["lifts"][0]["physical_navigation"][field] = value;
        let error = crate::level_data::LoadedLevel::hackable_from_json(
            &serde_json::to_vec(&document).unwrap(),
        )
        .expect_err(
            "invalid physical navigation must not silently drop collision or door ownership",
        );
        assert!(
            error.to_string().contains("physical stair"),
            "{field}: {error}"
        );
    }
    for endpoints in [serde_json::Value::Null, serde_json::json!([1, 0])] {
        let mut document = physical_stair_fixture();
        document["asset_geometry"]["lifts"][0]["endpoint_doors"] = endpoints;
        let error = crate::level_data::LoadedLevel::hackable_from_json(
            &serde_json::to_vec(&document).unwrap(),
        )
        .expect_err("physical endpoints must retain their height identities");
        assert!(error.to_string().contains("physical stair"), "{error}");
    }
}

fn placed_point(point: MapPoint, turn: u8) -> MapPoint {
    if turn == 0 {
        return point;
    }
    let (x, y) = (point.x - 400., point.y - 300.);
    let (x, y) = match turn {
        1 => (-y, x),
        2 => (-x, -y),
        3 => (y, -x),
        _ => unreachable!(),
    };
    MapPoint::new(x + 900., y + 800.)
}

pub(super) fn placed_stair_fixture(bytes: &[u8], turn: u8, lift_type: u8) -> Vec<u8> {
    transformed_lift_fixture(bytes, lift_type, i16::from(turn) * 4, |point| {
        placed_point(point, turn)
    })
}

pub(super) fn angled_lift_fixture(bytes: &[u8], degrees: f32, lift_type: u8) -> Vec<u8> {
    let (sin, cos) = degrees.to_radians().sin_cos();
    transformed_lift_fixture(bytes, lift_type, (degrees / 22.5).round() as i16, |point| {
        let (x, y) = (point.x - 400., point.y - 300.);
        MapPoint::new(900. + x * cos - y * sin, 800. + x * sin + y * cos)
    })
}

fn transformed_lift_fixture(
    bytes: &[u8],
    lift_type: u8,
    direction_delta: i16,
    place: impl Fn(MapPoint) -> MapPoint,
) -> Vec<u8> {
    let mut document: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    let mut geometry: crate::level_data::CompiledAssetGeometry =
        serde_json::from_value(document["asset_geometry"].clone()).unwrap();
    let transform = |point: &mut (i16, i16)| {
        let moved = place(MapPoint::new(f32::from(point.0), f32::from(point.1)));
        *point = (moved.x.round() as i16, moved.y.round() as i16);
    };
    for area in geometry.motion_data.layers.iter_mut().flatten() {
        for point in &mut area.polygon.points {
            transform(point);
        }
        for obstacle in &mut area.obstacles {
            for point in &mut obstacle.polygon.points {
                transform(point);
            }
        }
    }
    for obstacle in &mut geometry.sight_obstacles {
        for point in &mut obstacle.points {
            let moved = place(MapPoint::new(point.x, point.y - point.z_top));
            point.x = moved.x;
            point.y = moved.y + point.z_top;
        }
    }
    for lift in &mut geometry.lifts {
        lift.lift_type = lift_type;
        lift.endpoint_doors = Some([0, 1]);
        lift.direction = (lift.direction + direction_delta).rem_euclid(16);
        for door in &mut lift.doors {
            transform(&mut door.point_in);
            transform(&mut door.point_out);
            transform(&mut door.point_mid);
        }
    }
    document["asset_geometry"] = serde_json::to_value(geometry).unwrap();
    serde_json::to_vec(&document).unwrap()
}

#[test]
fn compiled_lift_endpoints_and_ai_follow_height_after_rotation() {
    use crate::ai::{ForecastInput, forecast_destination_for_ia};
    for degrees in [0., 68., 180., 248.] {
        let bytes = angled_lift_fixture(
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/asset-lift.level.json"
            )),
            degrees,
            1,
        );
        let (engine, _) = compiled_walkway(&bytes);
        let doors = &engine.script_domains.interactables.doors;
        let index = doors[0].sector_in_index.unwrap();
        let grid = &engine.world.fast_grid.level;
        let sector = &grid.sectors[usize::from(index)];
        assert_eq!(
            sector.lowest_door_index,
            Some(0),
            "fall endpoint at {degrees}"
        );
        assert_eq!(sector.highest_door_index, Some(1));
        for (up, endpoint) in [(false, 0), (true, 1)] {
            let forecast = forecast_destination_for_ia(
                &crate::sim_rng::test_context(),
                &ForecastInput {
                    position_map_x: doors[0].point_in.x,
                    position_map_y: doors[0].point_in.y,
                    sector: u16::from(sector.sector_number),
                    sector_handle: Some(
                        crate::position_interface::SectorHandle::from_number(sector.sector_number)
                            .with_arena_index(index),
                    ),
                    layer: doors[0].layer_in,
                    direction: 0,
                    forecasted_movement_z: if up { 1. } else { -1. },
                    door_pass: None,
                    passing_door_directly: false,
                },
                doors,
                &grid.sectors,
                &grid.sector_number_map,
            );
            assert_eq!(
                forecast.position.map_point(),
                doors[endpoint].point_out,
                "AI at {degrees}, up={up}"
            );
            assert_eq!(forecast.position.level, doors[endpoint].layer_out);
        }
    }
}

#[test]
fn compiled_climb_exit_preserves_climbing_posture_without_a_mission_script() {
    for (lift_type, posture) in [(2, Posture::OnLadder), (3, Posture::OnWall)] {
        let bytes = placed_stair_fixture(
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/asset-lift.level.json"
            )),
            0,
            lift_type,
        );
        let (mut engine, mut assets) = compiled_walkway(&bytes);
        assert!(engine.scripts.mission.is_none());
        let door = engine.script_domains.interactables.doors[0].clone();
        let sector = crate::position_interface::SectorHandle::from_number(door.sector_in)
            .with_arena_index(door.sector_in_index.unwrap());
        let owner = walking_pc(
            &mut engine,
            &mut assets,
            door.point_in,
            door.layer_in,
            sector,
        );
        engine
            .ent_mut(owner)
            .element_data_mut()
            .publish_order_posture(posture);
        let mut element = SequenceElement::new_movement(
            1,
            Command::PassDoor,
            Some(owner),
            OrderType::WalkingUpright,
        );
        let crate::sequence::SequenceElementData::Movement { gate_id, .. } = &mut element.data
        else {
            unreachable!()
        };
        *gate_id = Some(crate::gate::DoorIndex::new(0).unwrap());
        let sequence = engine.orders.sequence_manager.insert_element(element);
        let reference = crate::sequence::SequenceElementRef::new(sequence, 0);
        engine.stamp_element_transition_state(owner, reference);
        assert!(engine.generate_transition(
            TickCtx::new(&crate::sim_rng::test_context(), &assets),
            &mut vec![],
            owner,
            reference
        ));
        assert_eq!(
            engine
                .seq()
                .get_element(sequence, 0)
                .unwrap()
                .posture_after_transition,
            posture
        );
    }
}

#[test]
fn compiled_doors_and_patch_cursors_work_without_a_mission_script() {
    let (mut engine, _) = compiled_walkway(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/asset-door-transition.level.json"
    )));
    assert!(engine.scripts.mission.is_none());
    assert!(!engine.script_domains.interactables.doors.is_empty());
    assert!(!engine.script_domains.interactables.patches.is_empty());
    use crate::resource_ids::{RHMOUSE_DOOR_NO, RHMOUSE_DOOR_YES};
    for locked in [false, true, false] {
        engine.script_domains.interactables.doors[0].set_locked_pc(locked);
        engine.script_domains.interactables.patches[0].locked = locked;
        let expected = if locked {
            RHMOUSE_DOOR_NO
        } else {
            RHMOUSE_DOOR_YES
        };
        assert_eq!(engine.choose_door_cursor(Some(0), None), expected);
        assert_eq!(engine.choose_door_cursor(None, Some(0)), expected);
    }
    assert_eq!(engine.find_patch_for_door(0), Some(1));
}

#[test]
fn compiled_buildings_keep_ai_door_lists_without_a_mission_script() {
    let (mut engine, mut assets) = compiled_walkway(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/asset-interior.level.json"
    )));
    assert!(engine.scripts.mission.is_none());
    engine.init_ai(&crate::sim_rng::test_context(), &mut assets);
    assert!(!engine.ai.global.houses.is_empty());
    assert!(!engine.ai.global.door_rally_points.is_empty());
    assert!(
        engine
            .ai
            .global
            .houses
            .iter()
            .all(|house| !house.door_indices.is_empty())
    );
}

#[test]
fn queued_actor_routes_traverse_compiled_stairs_in_both_directions() {
    let bytes = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/asset-lift.level.json"
    ));
    for (turn, reverse) in (0..4).flat_map(|turn| [false, true].map(|reverse| (turn, reverse))) {
        let bytes = placed_stair_fixture(bytes, turn, 1);
        let (mut engine, mut assets) = compiled_walkway(&bytes);
        assert!(engine.scripts.mission.is_none());
        let low = (placed_point(MapPoint::new(370., 350.), turn), 0, 0);
        let high = (placed_point(MapPoint::new(430., 250.), turn), 1, 2);
        let ((source, layer, sector), (goal, goal_layer, goal_sector)) =
            if reverse { (high, low) } else { (low, high) };
        let handle = |engine: &EngineInner, sector: u16| {
            let index = engine.world.fast_grid.level.sector_number_map
                [&crate::sector::SectorNumber::new(sector as i16)];
            crate::position_interface::SectorHandle::new(sector)
                .unwrap()
                .with_arena_index(crate::fast_find_grid::SectorIndex::new(index as u32).unwrap())
        };
        let source_sector = handle(&engine, sector);
        let destination_sector = handle(&engine, goal_sector);
        let owner = walking_pc(&mut engine, &mut assets, source, layer, source_sector);
        let receiver = engine.get_projection_area_index(&assets, source_sector, layer, source);
        engine.set_obstacle_and_material(&assets, owner, receiver);
        let authorization = engine.ent(owner).actor_auth_info();
        let path = crate::gate::find_path_gates_with_sector_indices(
            &engine.script_domains.interactables.doors,
            (source.x, source.y),
            sector,
            source_sector.arena_index(),
            (goal.x, goal.y),
            goal_sector,
            destination_sector.arena_index(),
            Some(&authorization),
            false,
            &|_| true,
            &|number| {
                engine
                    .world
                    .fast_grid
                    .level
                    .sectors
                    .iter()
                    .find(|sector| sector.sector_number == number)
                    .and_then(|sector| sector.lift_type)
            },
        )
        .expect("stairs must have an authorized gate route");
        assert_eq!(path.len(), 2);
        let sim = crate::sim_rng::test_context();
        engine
            .launch_gate_movement_sequence(
                TickCtx::new(&sim, &assets),
                &mut vec![],
                crate::engine::movement::GateRouteRequest {
                    entity_id: owner,
                    source_sector: Some(source_sector),
                    gate_path: path,
                    goal: crate::engine::movement::GoalShape::Point {
                        point: goal,
                        tolerance: 0.,
                    },
                    goal_layer,
                    base_action: OrderType::WalkingUpright,
                    move_after_last_door: true,
                    speed_factor: 1.,
                    initial_flags: crate::sequence::MoveFlags::empty(),
                    prefix_elements: vec![],
                    tail_elements: vec![],
                    append_arrival_speech: false,
                    append_recovery: false,
                },
            )
            .expect("stairs route sequence");
        let mut crossed_lift = false;
        for _ in 0..300 {
            engine.control.frame_counter += 1;
            engine.t_hourglass_phase_sequences(&assets);
            engine.hourglass_phase_paths(TickCtx::new(&sim, &assets));
            engine.t_tick_actor_owner_envelopes(&assets);
            let element = engine.ent(owner).element_data();
            let position = element.position_map();
            let current_sector = element.sector().expect("walking actor lost its sector");
            crossed_lift |= current_sector.get() == 3 && element.layer() == 2;
            // PassingDoor changes membership at the midpoint. Receiving-plane
            // identity is directional at that exact contact; interior movement
            // must already use the destination plane.
            if !engine
                .script_domains
                .interactables
                .doors
                .iter()
                .any(|door| door.point_mid == position)
            {
                assert_actor_receiver(
                    &engine,
                    &assets,
                    owner,
                    current_sector,
                    element.layer(),
                    position,
                );
            }
            if (position - goal).length() < 0.01 && element.layer() == goal_layer {
                break;
            }
        }
        let element = engine.ent(owner).element_data();
        assert!(crossed_lift, "actor never entered the stair sector");
        assert!(
            (element.position_map() - goal).length() < 0.01,
            "actor stopped at {:?}, expected {goal:?}",
            element.position_map()
        );
        assert_eq!(element.layer(), goal_layer);
        assert_eq!(element.sector(), Some(destination_sector));
    }
}
