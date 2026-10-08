use super::*;

#[test]
fn physical_walking_shortcut_respects_the_requested_destination_layer() {
    let cases: serde_json::Value = serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/asset-sloped-terrain-sockets.json"
    )))
    .unwrap();
    let mut descriptor = cases[0]["descriptor"].clone();
    let geometry = &mut descriptor["asset_geometry"];
    let mut area = geometry["motion_data"]["layers"][0][0].clone();
    area["polygon"]["points"] = serde_json::json!([[0, 0], [100, 0], [100, 100], [0, 100]]);
    area["precise_polygon"] = serde_json::json!([]);
    geometry["motion_data"]["layers"] = serde_json::json!([[area.clone()], [area], []]);
    let mut template = geometry["sight_obstacles"][0].clone();
    // These cases replace the plane's vertices, so derive anchors from those vertices.
    template.as_object_mut().unwrap().remove("projection_plane");
    geometry["sight_obstacles"] = serde_json::Value::Array(
        [0, 1]
            .into_iter()
            .map(|layer| {
                let mut receiver = template.clone();
                let z = layer * 30;
                receiver["projection_area"] = serde_json::json!([layer, layer]);
                receiver["points"] = serde_json::json!(
                    [(0, 0), (100, 0), (100, 100), (0, 100)].map(|(x, y)| serde_json::json!({
                        "x": x, "y": y+z, "z_bottom": z, "z_top": z
                    }))
                );
                receiver
            })
            .collect(),
    );
    let bytes = serde_json::to_vec(&descriptor).unwrap();
    for (target_layer, explicit_sector) in [(0, true), (1, true), (1, false)] {
        let (mut engine, mut assets) = compiled_walkway(&bytes);
        let handle = |index: usize| {
            crate::position_interface::SectorHandle::from_number(
                engine.world.fast_grid.level.sectors[index].sector_number,
            )
            .with_arena_index(crate::fast_find_grid::SectorIndex::new(index as u32).unwrap())
        };
        let source_sector = handle(0);
        let target_sector = handle(usize::from(target_layer));
        let source = MapPoint::new(30., 50.);
        let goal = MapPoint::new(70., 50.);
        let owner = walking_pc(&mut engine, &mut assets, source, 0, source_sector);
        let receiver = engine
            .get_projection_area_index(&assets, source_sector, 0, source)
            .unwrap();
        engine.set_obstacle_and_material(&assets, owner, Some(receiver));
        assert!(
            engine
                .current_physical_walking_floor(&assets, owner)
                .is_some()
        );
        assert!(
            engine
                .get_projection_area_index(&assets, target_sector, target_layer, goal)
                .is_some()
        );
        let mut movement =
            SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::WalkingUpright);
        let crate::sequence::SequenceElementData::Movement {
            destination,
            layer,
            sector,
            ..
        } = &mut movement.data
        else {
            unreachable!()
        };
        *destination = goal;
        *layer = target_layer;
        *sector = explicit_sector.then_some(target_sector);
        let sequence = engine.t_launch_in_progress(&assets, movement);
        let sim = crate::sim_rng::test_context();
        let result = engine.try_dispatch_move_path(
            TickCtx::new(&sim, &assets),
            owner,
            crate::sequence::SequenceElementRef::new(sequence, 0),
            goal,
            OrderType::WalkingUpright,
        );
        let physical = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .orders
            .iter()
            .any(|order| order.physical_walking.is_some());
        assert_eq!(
            physical,
            target_layer == 0 || !explicit_sector,
            "destination layer {target_layer}, explicit sector {explicit_sector}: {result:?}"
        );
        if target_layer == 1 && explicit_sector {
            assert!(
                matches!(result, MovePathOutcome::Pending),
                "cross-layer goal must retain normal route dispatch: {result:?}"
            );
        }
    }
}

#[test]
fn dispatched_walk_changes_receivers_between_overlapping_height_planes() {
    let descriptor = overlapping_receiving_floors();
    let (engine, assets) = compiled_walkway(&serde_json::to_vec(&descriptor).unwrap());
    let ground = MapPoint::new(50., 30.);
    let ramp = MapPoint::new(50., 45.);
    for (source, goal) in [(ground, ramp), (ramp, ground)] {
        assert_eq!(
            dispatch_building_approach(engine.clone(), assets.clone(), 0, 0, source, goal, None),
            Ok(())
        );
    }
}

#[test]
fn compiled_building_exit_binds_the_floor_at_its_midpoint() {
    let mut descriptor = overlapping_receiving_floors();
    let layers = descriptor["asset_geometry"]["motion_data"]["layers"]
        .as_array()
        .unwrap()
        .len();
    descriptor["asset_geometry"]["buildings"] = serde_json::json!([{"Building": {"doors": [{
        "door_type": 1, "active": true,
        "locked_pc": false, "unlockable": false,
        "locked_npc_villain": false, "locked_npc_civilian": false,
        "locked_pc_after_patch": false, "unlockable_after_patch": false,
        "locked_npc_villain_after_patch": false, "locked_npc_civilian_after_patch": false,
        "door_sector": {"points": [[40, 10], [60, 10], [60, 30], [40, 30]]},
        "point_out": [50, 45], "point_mid": [50, 30], "point_in": [50, 20],
        "sector_out": 0, "layer_out": 0, "sector_in": 4, "layer_in": layers - 1
    }]}}]);
    for physical in [false, true] {
        let (mut engine, mut assets) = compiled_walkway(&serde_json::to_vec(&descriptor).unwrap());
        if !physical {
            Arc::make_mut(&mut assets.navigation.physical_walking).clear();
        }
        let door = engine.script_domains.interactables.doors[0].clone();
        let sector = crate::position_interface::SectorHandle::from_number(door.sector_out)
            .with_arena_index(door.sector_out_index.unwrap());
        assert_ne!(
            engine.get_projection_area_index(&assets, sector, door.layer_out, door.point_mid),
            engine.get_projection_area_index(&assets, sector, door.layer_out, door.point_out),
            "the passage must cross between distinct receivers"
        );
        let owner = walking_pc(
            &mut engine,
            &mut assets,
            door.point_out,
            door.layer_out,
            sector,
        );
        let sim = crate::sim_rng::test_context();
        let index = crate::gate::DoorIndex::new(0).unwrap();
        engine.execute_pass_door(TickCtx::new(&sim, &assets), owner, index, true);
        engine
            .ent_mut(owner)
            .element_data_mut()
            .set_position_map(door.point_mid);
        engine.execute_pass_door(TickCtx::new(&sim, &assets), owner, index, false);
        let expected_point = if physical {
            door.point_mid
        } else {
            door.point_out
        };
        let receiver =
            engine.get_projection_area_index(&assets, sector, door.layer_out, expected_point);
        assert_eq!(
            engine.ent(owner).position_iface().get_obstacle(),
            receiver,
            "physical={physical}"
        );
        if physical {
            actor_receiver_result(
                &engine,
                &assets,
                owner,
                sector,
                door.layer_out,
                door.point_mid,
            )
            .unwrap();
        }
    }
}

fn overlapping_receiving_floors() -> serde_json::Value {
    let cases: serde_json::Value = serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/asset-sloped-terrain-sockets.json"
    )))
    .unwrap();
    let mut descriptor = cases[0]["descriptor"].clone();
    let geometry = &mut descriptor["asset_geometry"];
    geometry["motion_data"]["layers"][0][0]["polygon"]["points"] =
        serde_json::json!([[0, 0], [100, 0], [100, 100], [0, 100]]);
    geometry["motion_data"]["layers"][0][0]["precise_polygon"] = serde_json::json!([]);
    let mut template = geometry["sight_obstacles"][0].clone();
    template.as_object_mut().unwrap().remove("projection_plane");
    let receivers = [
        (vec![(0., 0.), (100., 0.), (100., 100.), (0., 100.)], false),
        (vec![(40., 40.), (60., 40.), (60., 60.), (40., 60.)], true),
    ]
    .into_iter()
    .map(|(points, raised)| {
        let mut receiver = template.clone();
        receiver["points"] = serde_json::Value::Array(
            points
                .into_iter()
                .map(|(x, y)| {
                    let z = if raised { (y - 40.) * 0.5 } else { 0. };
                    serde_json::json!({"x": x, "y": y, "z_bottom": z, "z_top": z})
                })
                .collect(),
        );
        receiver
    })
    .collect::<Vec<_>>();
    geometry["sight_obstacles"] = serde_json::json!(receivers);
    descriptor
}

#[test]
#[ignore = "requires exported building approaches via ROBIN_ASSET_MAP_DIAGNOSTICS"]
fn exported_building_approaches_support_dispatched_actor_routes() {
    audit_building_approaches(None, false);
}

#[test]
#[ignore = "requires exported approaches and ROBIN_CLIMB_RHS"]
fn exported_building_approaches_support_complete_sprite_routes() {
    let sprite = super::exported_stairs::complete_climb_sprite();
    audit_building_approaches(Some(&sprite), false);
}

#[test]
#[ignore = "requires exported entrances and ROBIN_CLIMB_RHS"]
fn exported_buildings_support_complete_sprite_round_trips() {
    let sprite = super::exported_stairs::complete_climb_sprite();
    audit_building_approaches(Some(&sprite), true);
}

fn audit_building_approaches(sprite: Option<&crate::sprite::Sprite>, round_trip: bool) {
    let directory = std::path::PathBuf::from(std::env::var("ROBIN_ASSET_MAP_DIAGNOSTICS").unwrap());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("diagnostics.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["complete"], true);
    let mut report = serde_json::json!({
        "scope": if round_trip { "building-entry-and-exit-not-rendering" } else { "dispatched-building-approaches-not-interior-entry-or-rendering" },
        "actor_half_diagonal": [6, 3],
        "complete_sprite": sprite.is_some(),
        "complete": false, "audit_finished": false, "results": []
    });
    let report_path = directory.join(if round_trip {
        "actor-building-round-trip-report.json"
    } else if sprite.is_some() {
        "actor-building-sprite-approach-report.json"
    } else {
        "actor-building-approach-report.json"
    });
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    let mut checked = 0;
    let mut failed = 0;
    for result in manifest["results"].as_array().unwrap() {
        let file = result["file"].as_str().unwrap();
        let bytes = std::fs::read(directory.join(file)).unwrap();
        let descriptor: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let dimensions = &descriptor["walkable_polygon"][2];
        let (engine, assets) = compiled_walkway_with_dimensions(
            &bytes,
            (
                dimensions[0].as_f64().unwrap() as f32 + 1.,
                dimensions[1].as_f64().unwrap() as f32 + 1.,
            ),
        );
        let probes = result["building_approaches"]
            .as_array()
            .expect("authored building approach probes");
        assert!(!probes.is_empty());
        assert_eq!(
            probes.len(),
            engine
                .script_domains
                .interactables
                .doors
                .iter()
                .filter(|door| door.door_type == crate::gate::DoorType::Building)
                .count()
        );
        for probe in probes {
            let matches = engine
                .script_domains
                .interactables
                .doors
                .iter()
                .enumerate()
                .filter(|(_, door)| {
                    door.door_type == crate::gate::DoorType::Building
                        && u64::from(door.layer_out) == probe["layer"].as_u64().unwrap()
                        && u64::from(u16::from(door.sector_out))
                            == probe["sector"].as_u64().unwrap()
                        && f64::from(door.point_out.x) == probe["point_out"][0].as_f64().unwrap()
                        && f64::from(door.point_out.y) == probe["point_out"][1].as_f64().unwrap()
                })
                .collect::<Vec<_>>();
            assert_eq!(matches.len(), 1, "ambiguous entrance probe: {probe}");
            let (index, door) = matches[0];
            let world = &probe["source_world"];
            let approach = MapPoint::new(
                world[0].as_f64().unwrap() as f32,
                (world[1].as_f64().unwrap() - world[2].as_f64().unwrap()) as f32,
            );
            let sector_index = door
                .sector_out_index
                .expect("bound building outside sector")
                .get() as usize;
            let routes = if round_trip {
                vec![(door.point_out, door.point_out)]
            } else {
                vec![(approach, door.point_out), (door.point_out, approach)]
            };
            for (source, goal) in routes {
                let outcome = if round_trip {
                    super::exported_stairs::walk_exported_building_round_trip(
                        engine.clone(),
                        assets.clone(),
                        index,
                        sprite.expect("complete building sprite"),
                    )
                    .and_then(|authorized| {
                        if authorized {
                            Ok(())
                        } else {
                            Err("building route was not authorized".into())
                        }
                    })
                } else {
                    dispatch_building_approach(
                        engine.clone(),
                        assets.clone(),
                        door.layer_out,
                        sector_index,
                        source,
                        goal,
                        sprite,
                    )
                };
                checked += 1;
                if outcome.is_err() {
                    failed += 1;
                }
                report["results"]
                    .as_array_mut()
                    .unwrap()
                    .push(serde_json::json!({
                        "file": file, "door": index, "probe": probe, "layer": door.layer_out,
                        "source": [source.x, source.y], "goal": [goal.x, goal.y],
                        "passed": outcome.is_ok(), "error": outcome.err()
                    }));
            }
        }
        std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    report["audit_finished"] = true.into();
    report["complete"] = (checked > 0 && failed == 0).into();
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    assert!(checked > 0);
    assert_eq!(
        failed,
        0,
        "{failed}/{checked} building routes failed; see {}",
        report_path.display()
    );
}

fn dispatch_building_approach(
    mut engine: EngineInner,
    mut assets: LevelAssets,
    layer: u16,
    sector_index: usize,
    source: MapPoint,
    goal: MapPoint,
    sprite: Option<&crate::sprite::Sprite>,
) -> Result<(), String> {
    let sector = &engine.world.fast_grid.level.sectors[sector_index];
    let handle = crate::position_interface::SectorHandle::new(u16::from(sector.sector_number))
        .unwrap()
        .with_arena_index(crate::fast_find_grid::SectorIndex::new(sector_index as u32).unwrap());
    let receiver = engine
        .get_projection_area_index(&assets, handle, layer, source)
        .ok_or_else(|| format!("approach source has no receiver: {source:?}"))?;
    engine
        .get_projection_area_index(&assets, handle, layer, goal)
        .ok_or_else(|| format!("approach goal has no receiver: {goal:?}"))?;
    let owner = walking_pc(&mut engine, &mut assets, source, layer, handle);
    if let Some(sprite) = sprite {
        let element = engine.ent_mut(owner).element_data_mut();
        let position = element.sprite.position_iface.clone();
        element.sprite = sprite.clone();
        element.sprite.position_iface = position;
    }
    engine.set_obstacle_and_material(&assets, owner, Some(receiver));
    let action = OrderType::WalkingUpright;
    let mut movement = SequenceElement::new_movement(1, Command::Move, Some(owner), action);
    let crate::sequence::SequenceElementData::Movement {
        destination,
        layer: target_layer,
        sector: target_sector,
        ..
    } = &mut movement.data
    else {
        unreachable!()
    };
    *destination = goal;
    *target_layer = layer;
    *target_sector = Some(handle);
    let sequence = engine.t_launch_in_progress(&assets, movement);
    let sim = crate::sim_rng::test_context();
    let outcome = engine.try_dispatch_move_path(
        TickCtx::new(&sim, &assets),
        owner,
        crate::sequence::SequenceElementRef::new(sequence, 0),
        goal,
        action,
    );
    if !matches!(outcome, MovePathOutcome::Pending | MovePathOutcome::Success) {
        return Err(format!("approach dispatch failed: {outcome:?}"));
    }
    for _ in 0..2 {
        engine.hourglass_phase_paths(TickCtx::new(&sim, &assets));
    }
    engine.select_sequence_element(owner, Some((sequence, 0)));
    let mut last_order = None;
    for _ in 0..500 {
        if let Some(order) = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .and_then(|element| element.orders.front())
        {
            last_order = Some(order.clone());
        }
        engine.t_tick_actor_owner_envelopes(&assets);
        let position = engine.ent(owner).element_data().position_map();
        actor_receiver_result(&engine, &assets, owner, handle, layer, position)?;
        if (position - goal).length() < 0.01 {
            return Ok(());
        }
        // A zero-distance stop animation reserves 0.01 map units when
        // inserted at the end of a walk, then plays without displacement.
        // Require the completed order chain and that specific animation;
        // moving or blocked actors do not get a wider arrival tolerance.
        let stopped = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .is_some_and(|element| element.orders.is_empty());
        let stop = OrderType::TransitionWalkingUprightWaitingUpright;
        if stopped
            && last_order
                .as_ref()
                .is_some_and(|order| order.order_type == stop)
            && engine.ent(owner).sprite().distance_for_animation(stop) == 0
        {
            let rounding = [position.x, position.y, goal.x, goal.y]
                .into_iter()
                .map(|value| (value.next_up() - value).abs())
                .fold(0.0_f32, f32::max);
            if (position - goal).length() <= 0.01 + 4.0 * rounding {
                return Ok(());
            }
        }
    }
    let position = engine.ent(owner).element_data().position();
    let orders = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .map(|element| &element.orders);
    Err(format!(
        "approach stalled at {position:?}; goal {goal:?}; orders {orders:?}; last_order {last_order:?}"
    ))
}

#[test]
#[ignore = "requires exported geometry and ray probes via ROBIN_ASSET_MAP_DIAGNOSTICS"]
fn exported_geometry_preserves_authored_sight_and_projectile_gaps() {
    use crate::sight_obstacle::{SIGHTOBSTACLE_OPAQUE, SIGHTOBSTACLE_SOLID, is_reachable_3d};
    let directory = std::path::PathBuf::from(std::env::var("ROBIN_ASSET_MAP_DIAGNOSTICS").unwrap());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("diagnostics.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["complete"], true);
    let report_path = directory.join("ray-probe-report.json");
    let mut report = serde_json::json!({"complete": false, "scope": "authored-initial-state-ray-probes", "results": []});
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    let mut checked = 0;
    let mut failures = 0;
    for result in manifest["results"].as_array().unwrap() {
        let Some(probes) = result["ray_probes"].as_array() else {
            continue;
        };
        let file = result["file"].as_str().unwrap();
        let bytes = std::fs::read(directory.join(file)).unwrap();
        let descriptor: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let dimensions = &descriptor["walkable_polygon"][2];
        let (engine, assets) = compiled_walkway_with_dimensions(
            &bytes,
            (
                dimensions[0].as_f64().unwrap() as f32 + 1.,
                dimensions[1].as_f64().unwrap() as f32 + 1.,
            ),
        );
        for probe in probes {
            let endpoints: [[f32; 3]; 2] =
                serde_json::from_value(probe["endpoints"].clone()).unwrap();
            for (kind, mask) in [
                ("sight", SIGHTOBSTACLE_OPAQUE),
                ("projectile", SIGHTOBSTACLE_SOLID),
            ] {
                let expected = probe["clear"].as_bool().unwrap();
                for reverse in [false, true] {
                    let (a, b) = if reverse {
                        (endpoints[1], endpoints[0])
                    } else {
                        (endpoints[0], endpoints[1])
                    };
                    let actual = is_reachable_3d(engine.sight_obstacles(&assets), a, b, mask);
                    checked += 1;
                    failures += usize::from(actual != expected);
                    report["results"]
                        .as_array_mut()
                        .unwrap()
                        .push(serde_json::json!({
                            "file": file, "probe": probe["name"], "kind": kind, "reverse": reverse,
                            "expected_clear": expected, "actual_clear": actual
                        }));
                }
            }
        }
    }
    report["complete"] = true.into();
    report["checked"] = checked.into();
    report["failures"] = failures.into();
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    assert!(checked > 0, "no sight/projectile probes tested");
    assert_eq!(failures, 0, "see {}", report_path.display());
    eprintln!("{checked} sight/projectile ray probes passed");
}

#[test]
fn loaded_movement_obstacles_reject_mouse_positions_inside_and_on_boundary() {
    let mut descriptor: serde_json::Value = serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/asset-navigation-copies.level.json"
    )))
    .unwrap();
    descriptor["asset_geometry"]["motion_data"]["layers"][1][0]["obstacles"] = serde_json::json!([{
        "state_id": 0,
        "polygon": { "points": [[480, 280], [520, 280], [520, 320], [480, 320]] }
    }]);
    let (engine, _) = compiled_walkway(&serde_json::to_vec(&descriptor).unwrap());
    let grid = &engine.world.fast_grid;
    for point in [
        MapPoint::new(500., 300.),
        MapPoint::new(480., 300.),
        MapPoint::new(480., 280.),
    ] {
        assert!(
            matches!(
                grid.get_sector(point, point, 1),
                crate::fast_find_grid::SectorHit::Blocked
            ),
            "movement obstacle must reject mouse position {point:?}"
        );
    }
    let outside = MapPoint::new(550., 300.);
    assert!(matches!(
        grid.get_sector(outside, outside, 1),
        crate::fast_find_grid::SectorHit::Found { .. }
    ));
}

#[test]
#[ignore = "requires assembly exports and endpoint routes via ROBIN_ASSET_MAP_DIAGNOSTICS"]
fn exported_endpoint_routes_support_actor_crossings() {
    let directory = std::path::PathBuf::from(std::env::var("ROBIN_ASSET_MAP_DIAGNOSTICS").unwrap());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("diagnostics.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["complete"], true);
    let mut checked = 0;
    for result in manifest["results"].as_array().unwrap() {
        let file = result["file"].as_str().unwrap();
        let bytes = std::fs::read(directory.join(file)).unwrap();
        let descriptor: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let layers = descriptor["asset_geometry"]["motion_data"]["layers"]
            .as_array()
            .unwrap();
        let (layer, sector) = if let (Some(layer), Some(sector)) =
            (result["layer"].as_u64(), result["sector"].as_u64())
        {
            (
                u16::try_from(layer).unwrap(),
                usize::try_from(sector).unwrap(),
            )
        } else {
            assert_eq!(
                layers
                    .iter()
                    .map(|layer| layer.as_array().unwrap().len())
                    .sum::<usize>(),
                1,
                "endpoint audit requires a single navigation region: {file}"
            );
            assert_eq!(layers[0].as_array().unwrap().len(), 1);
            (0, 0)
        };
        let dimensions = &descriptor["walkable_polygon"][2];
        let (engine, assets) = compiled_walkway_with_dimensions(
            &bytes,
            (
                dimensions[0].as_f64().unwrap() as f32 + 1.,
                dimensions[1].as_f64().unwrap() as f32 + 1.,
            ),
        );
        let routes = result["routes"]
            .as_array()
            .expect("endpoint routes are required");
        assert!(!routes.is_empty(), "no endpoint routes in {file}");
        if let Some(points) = result["blocked_points"].as_array() {
            let grid = &engine.world.fast_grid;
            for point in points {
                let x = point[0].as_f64().unwrap() as f32;
                let y = point[1].as_f64().unwrap() as f32;
                let position = MapPoint::new(x, y);
                let hit = grid.get_sector(position, position, layer);
                assert!(
                    matches!(hit, crate::fast_find_grid::SectorHit::Blocked),
                    "{file}: expected blocked position [{x}, {y}], got {hit:?}"
                );
            }
        }
        for route in routes {
            let point = |i: usize| {
                MapPoint::new(
                    route[i][0].as_f64().unwrap() as f32,
                    route[i][1].as_f64().unwrap() as f32,
                )
            };
            for (source, goal) in [(point(0), point(1)), (point(1), point(0))] {
                eprintln!("{file}: endpoint crossing {source:?} -> {goal:?}");
                tick_walkway_movement(
                    engine.clone(),
                    assets.clone(),
                    layer,
                    sector,
                    source,
                    goal,
                    !result["same_receiver_routes"].as_bool().unwrap_or(false),
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 0, "no endpoint crossings tested");
    eprintln!("{checked} directed endpoint crossings passed");
}

#[test]
#[ignore = "requires current exports via ROBIN_ASSET_MAP_DIAGNOSTICS"]
fn exported_receiving_seams_support_actor_crossings() {
    check_exported_receiver_crossings(false);
}

#[test]
#[ignore = "requires current exports via ROBIN_ASSET_MAP_DIAGNOSTICS"]
fn exported_ground_boundaries_support_actor_crossings() {
    check_exported_receiver_crossings(true);
}

fn check_exported_receiver_crossings(ground: bool) {
    let map_filter = std::env::var("ROBIN_RECEIVER_AUDIT_MAP").ok();
    let receiver_filter = std::env::var("ROBIN_RECEIVER_AUDIT_OBSTACLES")
        .ok()
        .map(|value| {
            value
                .split(',')
                .map(|index| {
                    index
                        .parse::<usize>()
                        .expect("receiver index must be an unsigned integer")
                })
                .collect::<std::collections::BTreeSet<_>>()
        });
    let directory = std::path::PathBuf::from(std::env::var("ROBIN_ASSET_MAP_DIAGNOSTICS").unwrap());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("diagnostics.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["complete"], true);
    let report_path = directory.join(if ground {
        "actor-ground-crossing-report.json"
    } else {
        "actor-receiver-crossing-report.json"
    });
    let mut report = serde_json::json!({
        "scope": if ground { "sampled-initial-state-actor-ground-crossings" }
            else { "sampled-initial-state-actor-receiver-crossings" },
        "map_filter": map_filter, "receiver_filter": receiver_filter,
        "complete": false, "results": []
    });
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    let mut total = 0;
    for result in manifest["results"].as_array().unwrap() {
        let file = result["file"].as_str().unwrap();
        if map_filter.as_ref().is_some_and(|filter| filter != file) {
            continue;
        }
        let bytes = std::fs::read(directory.join(file)).unwrap();
        let descriptor: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let dims = &descriptor["walkable_polygon"][2];
        let (engine, assets) = compiled_walkway_with_dimensions(
            &bytes,
            (
                dims[0].as_f64().unwrap() as f32 + 1.,
                dims[1].as_f64().unwrap() as f32 + 1.,
            ),
        );
        let grid = &engine.world.fast_grid;
        if let Some(indices) = &receiver_filter {
            assert!(
                indices
                    .iter()
                    .all(|index| *index < assets.environment.static_sight_obstacles.len()),
                "receiver filter includes an unknown obstacle in {file}"
            );
        }
        let half = grid.try_move_box_half_diagonal(0).unwrap();
        let mut candidates = vec![];
        let mut pairs = std::collections::BTreeSet::new();
        for line in grid.level.lines.iter().filter(|line| line.is_elevation) {
            let (left, right) = (line.left_obstacle_index, line.right_obstacle_index);
            if receiver_filter.as_ref().is_some_and(|indices| {
                ![left, right]
                    .into_iter()
                    .flatten()
                    .any(|index| indices.contains(&usize::from(index)))
            }) {
                continue;
            }
            if (left.is_none() || right.is_none()) != ground {
                continue;
            }
            let receiver = left.or(right).expect("elevation line must have a receiver");
            let Some(topology) = assets.environment.static_sight_obstacles[usize::from(receiver)]
                .projection_area_ref()
            else {
                continue;
            };
            let sector_index = usize::from(topology.sector);
            let sector = &grid.level.sectors[sector_index];
            let layer = u16::from(topology.layer);
            let handle = crate::position_interface::SectorHandle::from_number(sector.sector_number)
                .with_arena_index(topology.sector);
            let direction = line.b - line.a;
            let length = direction.length();
            if length < 16. {
                continue;
            }
            let midpoint = line.a + direction.scale(0.5);
            let offset =
                crate::coordinates::MapVec::new(-direction.y, direction.x).scale(12. / length);
            let a = midpoint + offset;
            let b = midpoint - offset;
            let receiver_a = engine.get_projection_area_index(&assets, handle, layer, a);
            let receiver_b = engine.get_projection_area_index(&assets, handle, layer, b);
            if receiver_a == receiver_b || (receiver_a.is_none() || receiver_b.is_none()) != ground
            {
                continue;
            }
            // Endpoints must have an unambiguous receiving plane; intermediate
            // actor ticks still exercise exact shared-edge positions.
            if receiver_a.is_some_and(|receiver| receiver_edge_contains(&assets, receiver, a))
                || receiver_b.is_some_and(|receiver| receiver_edge_contains(&assets, receiver, b))
            {
                continue;
            }
            let authorized = [a, b].into_iter().all(|point| {
                let bounds = crate::coordinates::MapBBox::from_corners(
                    MapPoint::new(point.x - half.x, point.y - half.y),
                    MapPoint::new(point.x + half.x, point.y + half.y),
                );
                matches!(grid.get_sector(point, point, layer), crate::fast_find_grid::SectorHit::Found { sector_number, .. }
                    if sector_number == sector.sector_number)
                    && grid.is_position_authorized(&bounds, layer)
            });
            if !authorized || !grid.is_reachable_thick(a, b, layer, half) {
                continue;
            }
            let pair = (left.min(right), left.max(right));
            if pairs.insert(pair) {
                candidates.push((layer, sector_index, a, b));
            }
        }
        let mut checked = 0;
        eprintln!(
            "{file}: testing {} eligible receiver pairs",
            candidates.len()
        );
        for &(layer, sector, a, b) in &candidates {
            for (source, goal) in [(a, b), (b, a)] {
                tick_walkway_crossing(engine.clone(), assets.clone(), layer, sector, source, goal);
                checked += 1;
            }
        }
        eprintln!(
            "{file}: {checked} actor crossings, {} eligible receiver pairs",
            candidates.len()
        );
        report["results"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "file": file, "eligible_pairs": candidates.len(), "directed_crossings": checked
            }));
        std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        total += checked;
    }
    assert!(total > 0, "no actor crossings sampled");
    report["complete"] = true.into();
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
}
