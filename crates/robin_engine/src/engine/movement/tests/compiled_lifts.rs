use super::*;

pub(super) fn physical_stair_fixture() -> serde_json::Value {
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
    let compiled: serde_json::Value = serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../level-editor/shared/test-fixtures/physical-stair-area.json"
    )))
    .unwrap();
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
    geometry["motion_data"]["layers"][2][0] = compiled["area"].clone();
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
    lift["physical_navigation"] = compiled["navigation"].clone();
    document
}

fn joined_physical_stair_fixture() -> serde_json::Value {
    let mut document = physical_stair_fixture();
    let geometry = &mut document["asset_geometry"];
    geometry["motion_data"]["layers"][2][0]["polygon"]["points"] = serde_json::json!([
        [390, 300],
        [400, 260],
        [410, 200],
        [410, 300],
        [400, 360],
        [390, 400]
    ]);
    geometry["motion_data"]["layers"][2][0]["obstacles"] = serde_json::json!([
        {"state_id":1,"polygon":{"points":[[398,268],[402,248],[402,348],[398,368]]}}
    ]);
    let lift = &mut geometry["lifts"][0];
    lift["physical_navigation"]["plane"] = serde_json::json!([4.0, 0.0, -1560.0]);
    lift["physical_navigation"]["floor_patches"] = serde_json::json!([
        {"plane":[4.0,0.0,-1560.0],"boundary":[[390,300],[400,300],[400,400],[390,400]]},
        {"plane":[6.0,0.0,-2360.0],"boundary":[[400,300],[410,300],[410,400],[400,400]]}
    ]);
    lift["physical_navigation"]["obstacles"] = serde_json::json!([
        {"motion_obstacle":0,"polygon":[[398,300],[402,300],[402,400],[398,400]]}
    ]);
    lift["physical_navigation"]["doors"][0]["inside"][2] = serde_json::json!(8);
    lift["physical_navigation"]["doors"][1]["inside"][2] = serde_json::json!(88);
    lift["doors"][0]["point_in"] = serde_json::json!([392, 342]);
    lift["doors"][1]["point_in"] = serde_json::json!([408, 262]);
    let receiver = geometry["sight_obstacles"][2].clone();
    geometry["sight_obstacles"].as_array_mut().unwrap().pop();
    for (x0, x1, z0, z1) in [(390, 400, 0, 40), (400, 410, 40, 100)] {
        let mut part = receiver.clone();
        part["points"] = serde_json::json!([
            {"x":x0,"y":300,"z_bottom":z0,"z_top":z0},
            {"x":x1,"y":300,"z_bottom":z1,"z_top":z1},
            {"x":x1,"y":400,"z_bottom":z1,"z_top":z1},
            {"x":x0,"y":400,"z_bottom":z0,"z_top":z0}
        ]);
        geometry["sight_obstacles"]
            .as_array_mut()
            .unwrap()
            .push(part);
    }
    document
}

#[test]
fn joined_physical_stair_gates_follow_both_floors_and_live_barriers() {
    let document = joined_physical_stair_fixture();
    let (mut engine, assets) = compiled_walkway(&serde_json::to_vec(&document).unwrap());
    let sim = crate::sim_rng::test_context();
    for (step, open) in [false, true, false, true].into_iter().enumerate() {
        if step > 0 {
            engine.apply_patch(
                TickCtx::new(&sim, &assets),
                crate::patch::PatchIndex::new(0).unwrap(),
            );
        }
        for (from, to) in [(0, 1), (1, 0)] {
            let result = super::exported_stairs::walk_exported_stairs(
                engine.clone(),
                assets.clone(),
                from,
                to,
            );
            if open {
                assert_eq!(result, Ok(true), "joined flight {from}->{to}");
            } else {
                assert!(
                    result
                        .as_ref()
                        .is_err_and(|error| error.starts_with("lift route stalled")),
                    "closed joined flight: {result:?}"
                );
            }
        }
    }
}

#[test]
fn joined_physical_stair_admission_rejects_wrong_coverage_and_seam_heights() {
    let document = joined_physical_stair_fixture();
    let lift: crate::level_data::RawLift =
        serde_json::from_value(document["asset_geometry"]["lifts"][0].clone()).unwrap();
    let area: crate::level_data::RawMotionArea =
        serde_json::from_value(document["asset_geometry"]["motion_data"]["layers"][2][0].clone())
            .unwrap();
    let physical = lift.physical_navigation.as_ref().unwrap();
    physical.validate(&lift, &area).unwrap();
    let mut changed = physical.clone();
    changed.floor_patches[1].plane[2] += 1.0;
    assert!(
        changed
            .validate(&lift, &area)
            .unwrap_err()
            .contains("shared boundary")
    );
    let mut changed = physical.clone();
    for patch in &mut changed.floor_patches {
        for point in &mut patch.boundary {
            if point[1] == 400.0 {
                point[1] = 390.0;
            }
        }
    }
    assert!(
        changed
            .validate(&lift, &area)
            .unwrap_err()
            .contains("walking boundary")
    );
    let mut changed = physical.clone();
    changed.floor_patches[0].boundary = vec![
        [390., 300.],
        [395., 300.],
        [395., 301.],
        [395.01, 301.],
        [395.01, 300.],
        [400., 300.],
        [400., 400.],
        [390., 400.],
    ];
    assert!(
        changed
            .validate(&lift, &area)
            .unwrap_err()
            .contains("walking boundary")
    );
    let restored: crate::level_data::RawLift = bitcode::decode(&bitcode::encode(&lift)).unwrap();
    restored
        .physical_navigation
        .as_ref()
        .unwrap()
        .validate(&restored, &area)
        .unwrap();
}

fn physical_walker(
    engine: &mut EngineInner,
    assets: &mut LevelAssets,
    sector: u16,
    source: [f32; 2],
    destination: [f32; 2],
) -> crate::element::EntityId {
    let stair = &assets.navigation.physical_stairs[&sector];
    let source_world = stair.world_position(source).unwrap();
    let coefficients = stair.plane_at(source).unwrap();
    let destination = stair.world_position(destination).unwrap().map(|v| v as f32);
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
    let [a, b, c] = coefficients.map(|v| v as f32);
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
fn physical_stair_gate_routes_cross_between_landings_in_both_directions() {
    for (reverse, controlled) in [false, true]
        .into_iter()
        .flat_map(|reverse| [false, true].map(|controlled| (reverse, controlled)))
    {
        let mut document = edge_on_physical_stair_fixture();
        if controlled {
            let geometry = &mut document["asset_geometry"];
            geometry["motion_data"]["layers"][2][0]["obstacles"] = serde_json::json!([
                {"state_id":1,"polygon":{"points":[[380,200],[420,200],[420,200],[380,200]]}}
            ]);
            geometry["lifts"][0]["physical_navigation"]["obstacles"] = serde_json::json!([
                {"motion_obstacle":0,"polygon":[[380,349],[420,349],[420,351],[380,351]]}
            ]);
            geometry["movement_transitions"] = serde_json::json!([{
                "id":"edge-on-stair-barrier","waypoint":[400,180],"sector":0,"layer":0,
                "active":true,"definitive":false,"apply_polygon":{"points":[]},"no_apply_polygon":{"points":[]},
                "motion_changes":[{"sector":2,"layer":2,"changing_obstacle":0}]
            }]);
        }
        let (mut engine, mut assets) = compiled_walkway(&serde_json::to_vec(&document).unwrap());
        let (source, source_number, goal, goal_number, expected) = if reverse {
            (
                MapPoint::new(400., 220.),
                1,
                MapPoint::new(400., 180.),
                0,
                [400., 280., 100.],
            )
        } else {
            (
                MapPoint::new(400., 180.),
                0,
                MapPoint::new(400., 220.),
                1,
                [400., 420., 200.],
            )
        };
        let handle = |engine: &EngineInner, number: u16| {
            let index = engine.world.fast_grid.level.sector_number_map
                [&crate::sector::SectorNumber::new(number as i16)];
            crate::position_interface::SectorHandle::new(number)
                .unwrap()
                .with_arena_index(crate::fast_find_grid::SectorIndex::new(index as u32).unwrap())
        };
        let source_sector = handle(&engine, source_number);
        let goal_sector = handle(&engine, goal_number);
        let owner = walking_pc(
            &mut engine,
            &mut assets,
            source,
            source_number,
            source_sector,
        );
        let receiver =
            engine.get_projection_area_index(&assets, source_sector, source_number, source);
        engine.set_obstacle_and_material(&assets, owner, receiver);
        let authorization = engine.ent(owner).actor_auth_info();
        let path = crate::gate::find_path_gates_with_sector_indices(
            &engine.script_domains.interactables.doors,
            (source.x, source.y),
            source_number,
            source_sector.arena_index(),
            (goal.x, goal.y),
            goal_number,
            goal_sector.arena_index(),
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
        .expect("physical stair must retain a two-door gate route");
        assert_eq!(path.len(), 2);
        let sim = crate::sim_rng::test_context();
        let patch = crate::patch::PatchIndex::new(0).unwrap();
        if controlled {
            engine.apply_patch(TickCtx::new(&sim, &assets), patch);
        }
        engine
            .launch_gate_movement_sequence(
                TickCtx::new(&sim, &assets),
                &mut vec![],
                GateRouteRequest {
                    entity_id: owner,
                    source_sector: Some(source_sector),
                    gate_path: path,
                    goal: GoalShape::Point {
                        point: goal,
                        tolerance: 0.,
                    },
                    goal_layer: goal_number,
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
            .unwrap();
        let mut middle_ticks = 0;
        let mut closed_at = None;
        let mut held_position = None;
        let mut reopened = false;
        for tick in 0..600 {
            engine.control.frame_counter += 1;
            engine.t_hourglass_phase_sequences(&assets);
            engine.hourglass_phase_paths(TickCtx::new(&sim, &assets));
            engine.t_tick_actor_owner_envelopes(&assets);
            let position = engine.ent(owner).position_iface().get_position();
            if controlled
                && closed_at.is_none()
                && if reverse {
                    position.z < 175. && position.z > 170.
                } else {
                    position.z > 125. && position.z < 130.
                }
            {
                engine.apply_patch(TickCtx::new(&sim, &assets), patch);
                closed_at = Some(tick);
                held_position = Some(position);
            } else if let Some(closed) = closed_at
                && !reopened
            {
                assert_eq!(
                    Some(position),
                    held_position,
                    "a closed barrier must stop the in-flight route"
                );
                if tick >= closed + 8 {
                    engine.apply_patch(TickCtx::new(&sim, &assets), patch);
                    reopened = true;
                }
            }
            if position.z > 130. && position.z < 170. {
                middle_ticks += 1;
            }
            if [position.x, position.y, position.z] == expected {
                break;
            }
        }
        let position = engine.ent(owner).position_iface().get_position();
        assert_eq!(
            [position.x, position.y, position.z],
            expected,
            "reverse={reverse}, order={:?}",
            engine.actor_installed_order(owner)
        );
        assert!(
            middle_ticks > 10,
            "the route must walk the physical span rather than skip coincident screen endpoints"
        );
        assert!(
            !controlled || reopened,
            "controlled route must exercise closure and reopening"
        );
    }
}

fn physical_stair_landing_control_fixture() -> serde_json::Value {
    let mut document = physical_stair_fixture();
    let geometry = &mut document["asset_geometry"];
    geometry["motion_data"]["layers"][0][0]["obstacles"] = serde_json::json!([
        {"state_id":1,"polygon":{"points":[[387,340],[390,340],[390,360],[387,360]]}}
    ]);
    geometry["movement_transitions"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "id":"landing-barrier", "waypoint":[350,320], "sector":0, "layer":0,
            "active":true, "definitive":false,
            "apply_polygon":{"points":[]}, "no_apply_polygon":{"points":[]},
            "motion_changes":[{"sector":0,"layer":0,"changing_obstacle":0}]
        }));
    document
}

#[test]
fn physical_stair_landing_collision_follows_its_own_live_control() {
    let document = physical_stair_landing_control_fixture();
    let (mut engine, assets) = compiled_walkway(&serde_json::to_vec(&document).unwrap());
    let stair = &assets.navigation.physical_stairs[&3];
    let route = |engine: &EngineInner| {
        stair
            .route(
                &engine.world.pathfinder,
                [395., 350.],
                [390., 350.],
                crate::coordinates::MoveBoxHalfDiagonal::new(6., 3.),
            )
            .unwrap()
    };
    let sim = crate::sim_rng::test_context();
    engine.apply_patch(
        TickCtx::new(&sim, &assets),
        crate::patch::PatchIndex::new(0).unwrap(),
    );
    assert!(
        route(&engine).is_none(),
        "closed landing barrier must block the stair footprint"
    );
    engine.apply_patch(
        TickCtx::new(&sim, &assets),
        crate::patch::PatchIndex::new(1).unwrap(),
    );
    assert!(
        route(&engine).is_some(),
        "opening the landing barrier must expose its real floor"
    );
    let restored = engine.world.pathfinder.clone();
    engine.apply_patch(
        TickCtx::new(&sim, &assets),
        crate::patch::PatchIndex::new(1).unwrap(),
    );
    assert!(route(&engine).is_none());
    engine.world.pathfinder = restored;
    assert!(
        route(&engine).is_some(),
        "restored state must drive landing clearance too"
    );
}

#[test]
fn landing_barrier_crushes_only_overlapping_physical_stair_footprints() {
    let document = physical_stair_landing_control_fixture();
    let (mut engine, mut assets) = compiled_walkway(&serde_json::to_vec(&document).unwrap());
    let sim = crate::sim_rng::test_context();
    let stair_patch = crate::patch::PatchIndex::new(0).unwrap();
    let landing_patch = crate::patch::PatchIndex::new(1).unwrap();
    engine.apply_patch(TickCtx::new(&sim, &assets), stair_patch);
    engine.apply_patch(TickCtx::new(&sim, &assets), landing_patch);
    let overlapping = physical_walker(&mut engine, &mut assets, 3, [392., 350.], [408., 350.]);
    let clear = physical_walker(&mut engine, &mut assets, 3, [397., 350.], [408., 350.]);
    let selected_order = |engine: &EngineInner| {
        let (sequence, element) = engine.current_sequence_element_for_actor(clear).unwrap();
        let order = engine
            .orders
            .sequence_manager
            .get_element(sequence, element)
            .unwrap()
            .current_order()
            .unwrap();
        (
            order.storage_slot,
            order.physical_stair,
            order.destination_3d,
        )
    };
    let selected = selected_order(&engine);
    assert_eq!(selected.1, Some(3));
    assert!(!engine.ent(overlapping).element_data().unreachable);
    assert!(!engine.ent(clear).element_data().unreachable);
    engine.apply_patch(TickCtx::new(&sim, &assets), landing_patch);
    assert!(
        engine.ent(overlapping).element_data().unreachable,
        "the actor's center is on the stair, but its footprint overlaps the closed landing barrier"
    );
    assert!(!engine.ent(clear).element_data().unreachable);
    assert_eq!(
        selected_order(&engine),
        selected,
        "a landing control must not replace the unaffected physical order"
    );
    engine.apply_patch(TickCtx::new(&sim, &assets), landing_patch);
    assert!(
        !engine.ent(clear).element_data().unreachable,
        "reopening does not create a crushing obstacle"
    );
}

#[test]
fn stair_barrier_crushes_only_overlapping_bound_landing_footprints() {
    let document = physical_stair_fixture();
    let (mut engine, mut assets) = compiled_walkway(&serde_json::to_vec(&document).unwrap());
    let sim = crate::sim_rng::test_context();
    let patch = crate::patch::PatchIndex::new(0).unwrap();
    engine.apply_patch(TickCtx::new(&sim, &assets), patch);
    let index =
        engine.world.fast_grid.level.sector_number_map[&crate::sector::SectorNumber::new(0)];
    let sector = crate::position_interface::SectorHandle::new(0)
        .unwrap()
        .with_arena_index(crate::fast_find_grid::SectorIndex::new(index as u32).unwrap());
    let overlapping = walking_pc(
        &mut engine,
        &mut assets,
        MapPoint::new(388., 350.),
        0,
        sector,
    );
    let clear = walking_pc(
        &mut engine,
        &mut assets,
        MapPoint::new(380., 350.),
        0,
        sector,
    );
    engine.apply_patch(TickCtx::new(&sim, &assets), patch);
    assert!(
        engine.ent(overlapping).element_data().unreachable,
        "the actor's center is on the landing, but its footprint overlaps the closed stair barrier"
    );
    assert!(!engine.ent(clear).element_data().unreachable);
}

#[test]
fn physical_stair_actor_crosses_both_doors_using_bound_landing_support() {
    for endpoint in 0..2 {
        for direct in [true, false] {
            let mut document = edge_on_physical_stair_fixture();
            let receivers = document["asset_geometry"]["sight_obstacles"]
                .as_array()
                .unwrap()
                .clone();
            let mut split = Vec::new();
            for (receiver, ranges, height) in [
                (&receivers[0], [(270, 295), (295, 300)], 100),
                (&receivers[1], [(400, 405), (405, 430)], 200),
            ] {
                for (low, high) in ranges {
                    let mut part = receiver.clone();
                    part["points"] = serde_json::json!([
                        {"x":380,"y":low,"z_bottom":height,"z_top":height},
                        {"x":420,"y":low,"z_bottom":height,"z_top":height},
                        {"x":420,"y":high,"z_bottom":height,"z_top":height},
                        {"x":380,"y":high,"z_bottom":height,"z_top":height}
                    ]);
                    split.push(part);
                }
            }
            document["asset_geometry"]["sight_obstacles"] = serde_json::json!(split);
            let (mut engine, mut assets) =
                compiled_walkway(&serde_json::to_vec(&document).unwrap());
            let door = engine.script_domains.interactables.doors[endpoint].clone();
            let definition = assets.navigation.physical_stairs[&2].definition.clone();
            let physical = &definition.doors[endpoint];
            let (point, layer, number, index) = if direct {
                (
                    door.point_out,
                    door.layer_out,
                    door.sector_out,
                    door.sector_out_index,
                )
            } else {
                (
                    door.point_in,
                    door.layer_in,
                    door.sector_in,
                    door.sector_in_index,
                )
            };
            let sector = crate::position_interface::SectorHandle::from_number(number)
                .with_arena_index(index.unwrap());
            let owner = walking_pc(&mut engine, &mut assets, point, layer, sector);
            if direct {
                let receiver = engine.get_projection_area_index(&assets, sector, layer, point);
                engine.set_obstacle_and_material(&assets, owner, receiver);
            } else {
                let [az, bz, dz] = definition.plane.map(|value| value as f32);
                engine
                    .ent_mut(owner)
                    .position_iface_mut()
                    .set_obstacle_at_ground_position(
                        None,
                        Some(crate::position_interface::PlaneZCoeffs { az, bz, dz }),
                        crate::coordinates::GroundPoint::new(
                            physical.inside[0],
                            physical.inside[1],
                        ),
                    )
                    .unwrap();
            }
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
            *gate_id = Some(crate::gate::DoorIndex::new(endpoint as u32).unwrap());
            let sequence = engine.t_launch_in_progress(&assets, element);
            engine.select_sequence_element(owner, Some((sequence, 0)));
            let reference = crate::sequence::SequenceElementRef::new(sequence, 0);
            engine.stamp_element_transition_state(owner, reference);
            engine.instruct_pass_door(
                TickCtx::new(&crate::sim_rng::test_context(), &assets),
                &mut vec![],
                owner,
                reference,
            );
            let expected = if direct {
                physical.inside
            } else {
                physical.outside
            };
            for _ in 0..100 {
                engine.t_tick_actor_owner_envelopes(&assets);
                let pi = engine.ent(owner).position_iface();
                if pi.get_sector().unwrap().get() != 2 {
                    assert_actor_receiver(
                        &engine,
                        &assets,
                        owner,
                        pi.get_sector().unwrap(),
                        pi.get_layer().get(),
                        pi.get_position().to_map(),
                    );
                }
                let point = engine.ent(owner).position_iface().get_position();
                if [point.x, point.y, point.z] == expected {
                    break;
                }
            }
            let point = engine.ent(owner).position_iface().get_position();
            assert_eq!(
                [point.x, point.y, point.z],
                expected,
                "door {endpoint}, direct={direct}"
            );
            assert_eq!(
                engine.ent(owner).element_data().sector().unwrap().get(),
                if direct { 2 } else { endpoint as u16 }
            );
        }
    }
}

#[test]
fn physical_stair_door_orders_keep_world_goals_on_the_inside_walk() {
    for endpoint in 0..2 {
        for direct in [true, false] {
            let (mut engine, mut assets) =
                compiled_walkway(&serde_json::to_vec(&edge_on_physical_stair_fixture()).unwrap());
            let door = engine.script_domains.interactables.doors[endpoint].clone();
            let physical = assets.navigation.physical_stairs[&2].definition.doors[endpoint].clone();
            let owner = if direct {
                let sector = crate::position_interface::SectorHandle::from_number(door.sector_out)
                    .with_arena_index(door.sector_out_index.unwrap());
                walking_pc(
                    &mut engine,
                    &mut assets,
                    door.point_out,
                    door.layer_out,
                    sector,
                )
            } else {
                physical_walker(
                    &mut engine,
                    &mut assets,
                    2,
                    [physical.inside[0], physical.inside[1]],
                    [physical.inside[0], physical.inside[1]],
                )
            };
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
            *gate_id = Some(crate::gate::DoorIndex::new(endpoint as u32).unwrap());
            let sequence = engine.orders.sequence_manager.insert_element(element);
            let reference = crate::sequence::SequenceElementRef::new(sequence, 0);
            engine.stamp_element_transition_state(owner, reference);
            engine.instruct_pass_door(
                TickCtx::new(&crate::sim_rng::test_context(), &assets),
                &mut vec![],
                owner,
                reference,
            );
            let orders = &engine.seq().get_element(sequence, 0).unwrap().orders;
            assert_eq!(orders.len(), 4);
            let inside = if direct { 2 } else { 0 };
            assert_eq!(orders[inside].physical_stair, Some(2));
            assert_eq!(
                orders[inside].destination_3d,
                if direct {
                    physical.inside
                } else {
                    physical.middle
                }
            );
            assert_eq!(orders[2 - inside].physical_stair, None);
        }
    }
}

#[test]
fn physical_stair_door_handoffs_preserve_distinct_world_endpoints() {
    for endpoint in 0..2 {
        let (mut engine, mut assets) =
            compiled_walkway(&serde_json::to_vec(&edge_on_physical_stair_fixture()).unwrap());
        let door = engine.script_domains.interactables.doors[endpoint].clone();
        let physical = assets.navigation.physical_stairs[&2].definition.doors[endpoint].clone();
        let sector = crate::position_interface::SectorHandle::from_number(door.sector_out)
            .with_arena_index(door.sector_out_index.unwrap());
        let owner = walking_pc(
            &mut engine,
            &mut assets,
            door.point_mid,
            door.layer_out,
            sector,
        );
        let index = crate::gate::DoorIndex::new(endpoint as u32).unwrap();
        let rng = crate::sim_rng::test_context();
        engine.execute_pass_door(TickCtx::new(&rng, &assets), owner, index, true);
        let expected = crate::coordinates::WorldPoint3D::new(
            physical.middle[0],
            physical.middle[1],
            physical.middle[2],
        );
        assert_eq!(engine.ent(owner).position_iface().get_position(), expected);
        assert_eq!(engine.ent(owner).element_data().sector().unwrap().get(), 2);
        assert_eq!(
            engine.ent(owner).element_data().position_map(),
            MapPoint::new(400., 200.)
        );
        engine.execute_pass_door(TickCtx::new(&rng, &assets), owner, index, false);
        assert_eq!(engine.ent(owner).position_iface().get_position(), expected);
        assert_eq!(
            engine.ent(owner).element_data().sector().unwrap().get(),
            endpoint as u16
        );
        // The restored landing receiver must also govern the subsequent step.
        engine
            .ent_mut(owner)
            .element_data_mut()
            .set_position_map(door.point_out);
        let position = engine.ent(owner).position_iface().get_position();
        assert_eq!([position.x, position.y, position.z], physical.outside);
    }
}

#[test]
fn ordinary_door_handoff_installs_and_releases_the_physical_stair_floor() {
    let (mut engine, mut assets) =
        compiled_walkway(&serde_json::to_vec(&physical_stair_fixture()).unwrap());
    let door = engine.script_domains.interactables.doors[0].clone();
    engine.script_domains.interactables.doors[0].owning_lift_sector = None;
    engine.script_domains.interactables.doors[0].door_type = crate::gate::DoorType::Default;
    let source = crate::position_interface::SectorHandle::from_number(door.sector_out)
        .with_arena_index(door.sector_out_index.unwrap());
    let owner = walking_pc(
        &mut engine,
        &mut assets,
        door.point_in,
        door.layer_out,
        source,
    );
    let rng = crate::sim_rng::test_context();
    engine.execute_pass_door(
        TickCtx::new(&rng, &assets),
        owner,
        crate::gate::DoorIndex::new(0).unwrap(),
        true,
    );
    let physical = &assets.navigation.physical_stairs[&3].definition;
    let world = engine.ent(owner).position_iface().get_position();
    let plane =
        robin_level_data::stair_navigation::StairNavigationPlane::new(physical.plane).unwrap();
    assert!(
        plane.contains_runtime_position([world.x, world.y, world.z]),
        "{world:?}"
    );
    assert_eq!(world.to_map(), door.point_in);
    engine
        .ent_mut(owner)
        .element_data_mut()
        .set_position_map(door.point_out);
    engine.execute_pass_door(
        TickCtx::new(&rng, &assets),
        owner,
        crate::gate::DoorIndex::new(0).unwrap(),
        false,
    );
    let world = engine.ent(owner).position_iface().get_position();
    assert_eq!(world.z, 0.0);
    assert_eq!(world.to_map(), door.point_out);
}

#[test]
fn ordinary_edge_on_passage_handoff_uses_its_world_midpoint() {
    for index in [0, 1] {
        let (mut engine, mut assets) =
            compiled_walkway(&serde_json::to_vec(&edge_on_physical_stair_fixture()).unwrap());
        let endpoints = assets.navigation.physical_stairs[&2].definition.doors[index].clone();
        let door = engine.script_domains.interactables.doors[index].clone();
        let runtime = &mut engine.script_domains.interactables.doors[index];
        runtime.owning_lift_sector = None;
        runtime.door_type = crate::gate::DoorType::Default;
        runtime.world_endpoints = Some(endpoints.clone());
        let source = crate::position_interface::SectorHandle::from_number(door.sector_out)
            .with_arena_index(door.sector_out_index.unwrap());
        let owner = walking_pc(
            &mut engine,
            &mut assets,
            door.point_mid,
            door.layer_out,
            source,
        );
        let rng = crate::sim_rng::test_context();
        engine.execute_pass_door(
            TickCtx::new(&rng, &assets),
            owner,
            crate::gate::DoorIndex::new(index as u32).unwrap(),
            true,
        );
        let position = engine.ent(owner).position_iface().get_position();
        assert_eq!([position.x, position.y, position.z], endpoints.middle);
        assert_eq!(
            position.to_map(),
            engine.ent(owner).element_data().position_map()
        );
    }
}

#[test]
fn ordinary_stair_exit_probe_follows_the_receiving_edge_direction() {
    let mut descriptor = physical_stair_fixture();
    let obstacles = descriptor["asset_geometry"]["sight_obstacles"]
        .as_array_mut()
        .unwrap();
    let receiver = obstacles.len();
    obstacles.push(serde_json::json!({
        "points": ([[800.,850.], [1000.,950.], [1000.,800.], [800.,700.]].map(|[x,y]| serde_json::json!({"x":x,"y":y,"z_bottom":0.,"z_top":0.}))),
        "projection_area": [0,0], "opaque":false, "solid":false, "mouse":true,
        "show_shadow_polygon":false,"default_material":0,"material_indices":[]
    }));
    let (mut engine, mut assets) = compiled_walkway(&serde_json::to_vec(&descriptor).unwrap());
    let door = engine.script_domains.interactables.doors[0].clone();
    let runtime = &mut engine.script_domains.interactables.doors[0];
    runtime.owning_lift_sector = None;
    runtime.door_type = crate::gate::DoorType::Default;
    runtime.point_out = MapPoint::new(920., 900.125);
    let source = crate::position_interface::SectorHandle::from_number(door.sector_in)
        .with_arena_index(door.sector_in_index.unwrap());
    let target = crate::position_interface::SectorHandle::from_number(door.sector_out)
        .with_arena_index(door.sector_out_index.unwrap());
    let point = MapPoint::new(900., 900.0_f32.next_up());
    assert_eq!(
        engine.get_projection_area_index(&assets, target, door.layer_out, point),
        None
    );
    assert_eq!(
        engine.get_projection_area_index(
            &assets,
            target,
            door.layer_out,
            MapPoint::new(point.x.next_up(), point.y.next_up())
        ),
        None
    );
    let owner = walking_pc(&mut engine, &mut assets, point, door.layer_in, source);
    let sim = crate::sim_rng::test_context();
    engine.execute_pass_door(
        TickCtx::new(&sim, &assets),
        owner,
        crate::gate::DoorIndex::new(0).unwrap(),
        false,
    );
    assert_eq!(
        engine.ent(owner).position_iface().get_obstacle(),
        Some(crate::sight_obstacle::SightObstacleIndex::new(receiver as u32).unwrap())
    );
}

#[test]
fn physical_stair_point_and_ordinary_gate_dispatch_use_world_orders_and_reach_the_goal() {
    for (reverse, gate_approach) in [(false, false), (true, false), (false, true), (true, true)] {
        let (mut engine, mut assets) =
            compiled_walkway(&serde_json::to_vec(&physical_stair_fixture()).unwrap());
        let sim = crate::sim_rng::test_context();
        engine.apply_patch(
            TickCtx::new(&sim, &assets),
            crate::patch::PatchIndex::new(0).unwrap(),
        );
        let (source, mut goal) = if reverse {
            ([404., 380.], [396., 320.])
        } else {
            ([396., 320.], [404., 380.])
        };
        if gate_approach {
            goal[0] += 0.25;
        }
        let owner = physical_walker(&mut engine, &mut assets, 3, source, goal);
        let goal_z = 5. * goal[0] - 1950.;
        let destination = MapPoint::new(goal[0], goal[1] - goal_z);
        if gate_approach {
            let door = &mut engine.script_domains.interactables.doors[0];
            door.owning_lift_sector = None;
            door.door_type = crate::gate::DoorType::Default;
            door.point_in = destination;
            door.world_endpoints = Some(robin_level_data::physical_stair::PhysicalStairDoor {
                inside: [goal[0], goal[1], goal_z],
                middle: [goal[0], goal[1], goal_z],
                outside: [goal[0], goal[1], goal_z],
            });
        }
        let (sequence, index) = engine.current_sequence_element_for_actor(owner).unwrap();
        engine.install_actor_order(owner, None);
        let element = engine
            .orders
            .sequence_manager
            .get_element_mut(sequence, index)
            .unwrap();
        element.orders.clear();
        element.command = Command::Move;
        if let crate::sequence::SequenceElementData::Movement {
            destination: stored,
            layer,
            sector,
            gate_id,
            ..
        } = &mut element.data
        {
            *stored = if gate_approach {
                MapPoint::new(destination.x.round(), destination.y.round())
            } else {
                destination
            };
            *layer = if gate_approach { 0 } else { 2 };
            *sector = if gate_approach {
                None
            } else {
                crate::position_interface::SectorHandle::new(3)
            };
            *gate_id = gate_approach.then(|| crate::gate::DoorIndex::new(0).unwrap());
        }
        assert!(engine.extract_move_instruction_owner(&assets, owner));
        assert!(matches!(
            engine.try_dispatch_move_path(
                TickCtx::new(&sim, &assets),
                owner,
                SequenceElementRef::new(sequence, index),
                destination,
                OrderType::WalkingStairs,
            ),
            MovePathOutcome::Success
        ));
        assert!(engine.orders.pending_path_requests.waiting.is_empty());
        let element = engine
            .orders
            .sequence_manager
            .get_element(sequence, index)
            .unwrap();
        assert!(
            element
                .orders
                .iter()
                .any(|order| order.physical_stair == Some(3)
                    && order.destination_3d == [goal[0], goal[1], goal_z])
        );
        for _ in 0..300 {
            engine.t_tick_actor_owner_envelopes(&assets);
            let position = engine.ent(owner).position_iface().get_position();
            if [position.x, position.y, position.z] == [goal[0], goal[1], goal_z] {
                break;
            }
        }
        let position = engine.ent(owner).position_iface().get_position();
        assert_eq!(
            [position.x, position.y, position.z],
            [goal[0], goal[1], goal_z]
        );
    }
}

#[test]
fn physical_stair_point_dispatch_rejects_ambiguous_and_unsupported_goals() {
    for (document, sector, destination) in [
        (
            edge_on_physical_stair_fixture(),
            2,
            MapPoint::new(400., 200.),
        ),
        (physical_stair_fixture(), 3, MapPoint::new(600., 330.)),
        // This goal is on the floor but inside the initially closed barrier.
        (physical_stair_fixture(), 3, MapPoint::new(400., 300.)),
    ] {
        let (mut engine, mut assets) = compiled_walkway(&serde_json::to_vec(&document).unwrap());
        let owner = physical_walker(&mut engine, &mut assets, sector, [400., 320.], [400., 380.]);
        let before = engine.ent(owner).position_iface().get_position();
        let (sequence, index) = engine.current_sequence_element_for_actor(owner).unwrap();
        let element = engine
            .orders
            .sequence_manager
            .get_element_mut(sequence, index)
            .unwrap();
        element.orders.clear();
        element.command = Command::Move;
        if let crate::sequence::SequenceElementData::Movement {
            destination: stored,
            layer,
            sector: stored_sector,
            ..
        } = &mut element.data
        {
            *stored = destination;
            *layer = 2;
            *stored_sector = crate::position_interface::SectorHandle::new(sector);
        }
        assert!(matches!(
            engine.try_dispatch_move_path(
                TickCtx::new(&crate::sim_rng::test_context(), &assets),
                owner,
                SequenceElementRef::new(sequence, index),
                destination,
                OrderType::WalkingStairs,
            ),
            MovePathOutcome::Refused
        ));
        assert!(engine.orders.pending_path_requests.waiting.is_empty());
        assert_eq!(engine.ent(owner).position_iface().get_position(), before);
    }
}

#[test]
fn physical_stair_source_authorization_preserves_world_position() {
    for (source, supported) in [
        ([400., 320.], true),
        ([400., 380.], true),
        ([400., 450.], false),
    ] {
        let (mut engine, mut assets) =
            compiled_walkway(&serde_json::to_vec(&edge_on_physical_stair_fixture()).unwrap());
        let owner = physical_walker(&mut engine, &mut assets, 2, source, [400., 380.]);
        let before = engine.ent(owner).position_iface().get_position();
        assert_eq!(
            engine.extract_move_instruction_owner(&assets, owner),
            supported
        );
        assert_eq!(engine.ent(owner).position_iface().get_position(), before);
    }
}

#[test]
fn physical_stair_source_authorization_reads_live_barriers() {
    let (mut engine, mut assets) =
        compiled_walkway(&serde_json::to_vec(&physical_stair_fixture()).unwrap());
    let owner = physical_walker(&mut engine, &mut assets, 3, [400., 350.], [400., 380.]);
    let before = engine.ent(owner).position_iface().get_position();
    assert!(!engine.extract_move_instruction_owner(&assets, owner));
    let sim = crate::sim_rng::test_context();
    engine.apply_patch(
        TickCtx::new(&sim, &assets),
        crate::patch::PatchIndex::new(0).unwrap(),
    );
    assert!(engine.extract_move_instruction_owner(&assets, owner));
    assert_eq!(engine.ent(owner).position_iface().get_position(), before);
}

#[test]
fn physical_stair_transitions_wait_at_the_goal_and_preserve_unfinished_world_distance() {
    for transition_y in [320.1, 350.] {
        let (mut engine, mut assets) =
            compiled_walkway(&serde_json::to_vec(&edge_on_physical_stair_fixture()).unwrap());
        let owner = physical_walker(&mut engine, &mut assets, 2, [400., 320.], [400., 380.]);
        let action = OrderType::TransitionWaitingUprightWalkingUpright;
        let entity = engine.ent_mut(owner);
        entity.element_data_mut().set_direction_instantly(8);
        let sprite = entity.sprite_mut();
        let mut script = sprite.scripts[0].clone();
        script.action_id = action as u16;
        Arc::make_mut(&mut sprite.conversion)[action as usize] = sprite.scripts.len() as u16;
        Arc::make_mut(&mut sprite.scripts).extend(vec![script; 16]);
        let destination = [400., transition_y, transition_y - 200.];
        let mut transition =
            crate::order::Order::new(action, 400., 200., engine.orders.allocate_order_id());
        transition.physical_stair = Some(2);
        transition.destination_3d = destination;
        let transition_id = transition.order_id;
        let (sequence, element) = engine.current_sequence_element_for_actor(owner).unwrap();
        engine
            .orders
            .sequence_manager
            .get_element_mut(sequence, element)
            .unwrap()
            .insert_order(0, transition);
        engine.install_actor_order(owner, None);
        let mut held_at_transition_goal = false;
        let mut saw_continuation = false;
        let mut visited_transition_goal = false;
        for _ in 0..300 {
            engine.t_tick_actor_owner_envelopes(&assets);
            let position = engine.ent(owner).position_iface().get_position();
            let at_transition_goal = [position.x, position.y, position.z] == destination;
            visited_transition_goal |= at_transition_goal;
            if let Some(element) = engine
                .orders
                .sequence_manager
                .get_element(sequence, element)
            {
                held_at_transition_goal |= at_transition_goal
                    && element
                        .current_order()
                        .is_some_and(|order| order.order_id == transition_id);
                for order in &element.orders {
                    if order.transition_distance_continuation {
                        saw_continuation = true;
                        assert_eq!(order.physical_stair, Some(2));
                        assert_eq!(order.destination_3d, destination);
                    }
                }
            }
            if position.y == 380. {
                break;
            }
        }
        assert_eq!(engine.ent(owner).position_iface().get_position().y, 380.);
        assert!(
            visited_transition_goal,
            "the physical transition target must not be skipped"
        );
        if transition_y < 321. {
            assert!(held_at_transition_goal);
        } else {
            assert!(
                saw_continuation,
                "exhausted animation must retain remaining world distance"
            );
        }
    }
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
fn physical_stair_water_splashes_follow_animation_distance_and_world_position() {
    for (distance, wet) in [(2, true), (3, true), (3, false)] {
        let (mut engine, mut assets) =
            compiled_walkway(&serde_json::to_vec(&edge_on_physical_stair_fixture()).unwrap());
        let owner = physical_walker(&mut engine, &mut assets, 2, [400., 320.], [400., 380.]);
        let entity = engine.ent_mut(owner);
        // Face along world +Y so turn slowdown does not change the threshold under test.
        entity.element_data_mut().set_direction_instantly(8);
        if wet {
            entity
                .element_data_mut()
                .set_material(crate::element::GameMaterial::Water);
        }
        for script in Arc::make_mut(&mut entity.sprite_mut().scripts) {
            script.distances.fill(distance);
            script.sum_distance = distance * 3;
            script.average_speed = f32::from(distance);
        }
        let mut moves = 0;
        for _ in 0..30 {
            let before = engine.ent(owner).position_iface().get_position();
            engine.t_tick_actor_owner_envelopes(&assets);
            let position = engine.ent(owner).position_iface().get_position();
            if position == before {
                continue;
            }
            moves += 1;
            assert!((engine.ent(owner).element_data().position_map().y - 200.).abs() < 0.001);
            let particles = engine.feedback.titbit_manager.titbits();
            let expected = if wet && distance > 2 { moves / 3 } else { 0 };
            assert_eq!(particles.len(), expected);
            assert_eq!(
                engine.ent(owner).sprite().splitch_count as usize,
                if wet && distance > 2 { moves % 3 } else { 0 },
                "distance={distance}, wet={wet}, moves={moves}, position={position:?}, material={:?}",
                engine.ent(owner).element_data().material()
            );
            if expected > 0 && moves % 3 == 0 {
                let particle = particles.last().unwrap();
                assert_eq!(particle.kind, crate::titbit::TitbitKind::Water);
                assert_eq!(particle.position, position);
                assert_eq!(particle.layer, 2);
            }
            if moves == 6 {
                break;
            }
        }
        assert_eq!(moves, 6);
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
fn physical_stair_waits_for_a_neighbour_on_its_bound_landing() {
    for neighbour_height in [100., 200.] {
        let (mut engine, mut assets) =
            compiled_walkway(&serde_json::to_vec(&edge_on_physical_stair_fixture()).unwrap());
        let owner = physical_walker(&mut engine, &mut assets, 2, [400., 320.], [400., 300.]);
        let index =
            engine.world.fast_grid.level.sector_number_map[&crate::sector::SectorNumber::new(0)];
        let sector = crate::position_interface::SectorHandle::new(0)
            .unwrap()
            .with_arena_index(crate::fast_find_grid::SectorIndex::new(index as u32).unwrap());
        let neighbour = walking_pc(
            &mut engine,
            &mut assets,
            MapPoint::new(400., 299. - neighbour_height),
            0,
            sector,
        );
        engine
            .ent_mut(neighbour)
            .position_iface_mut()
            .set_obstacle_at_ground_position(
                None,
                Some(crate::position_interface::PlaneZCoeffs {
                    az: 0.,
                    bz: 0.,
                    dz: neighbour_height,
                }),
                crate::coordinates::GroundPoint::new(400., 299.),
            )
            .unwrap();
        let start = engine.ent(owner).position_iface().get_position();
        for _ in 0..8 {
            engine.t_tick_actor_owner_envelopes(&assets);
        }
        let position = engine.ent(owner).position_iface().get_position();
        if neighbour_height == 100. {
            assert_eq!(
                position, start,
                "landing footprint must block the stair endpoint"
            );
        } else {
            assert_ne!(
                position, start,
                "an actor on a different height must not obstruct this landing"
            );
        }
        engine.ent_mut(neighbour).element_data_mut().active = false;
        for _ in 0..100 {
            engine.t_tick_actor_owner_envelopes(&assets);
            if engine.ent(owner).position_iface().get_position().y == 300. {
                break;
            }
        }
        assert_eq!(engine.ent(owner).position_iface().get_position().y, 300.);
    }
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
        "physical neighbour was ignored or blocked a usable route: position={:?}, order={:?}, blocked={}",
        engine.ent(owner).position_iface().get_position(),
        engine.actor_installed_order(owner),
        engine.ent(owner).position_iface().blocked_count
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
        // This helper rotates projected geometry, not an asset in world space.
        // Keep legacy traversal coverage; physical placement uses editor exports.
        lift.physical_navigation = None;
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
