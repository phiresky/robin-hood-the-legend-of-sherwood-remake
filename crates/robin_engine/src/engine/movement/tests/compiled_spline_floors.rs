use super::*;

#[test]
fn compiled_ordinary_passage_receivers_are_bound_at_load_only_for_explicit_endpoints() {
    let bytes = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/asset-joined-posture.level.json"
    ));
    let (engine, assets) = compiled_walkway(bytes);
    let door = &engine.script_domains.interactables.doors[0];
    let pair = assets.navigation.passage_receivers[0].expect("prepared passage receivers");
    for (expected, number, arena, layer, point) in [
        (
            pair[0],
            door.sector_out,
            door.sector_out_index,
            door.layer_out,
            door.point_out,
        ),
        (
            pair[1],
            door.sector_in,
            door.sector_in_index,
            door.layer_in,
            door.point_in,
        ),
    ] {
        let sector = crate::position_interface::SectorHandle::from_number(number)
            .with_arena_index(arena.unwrap());
        assert_eq!(
            engine.get_projection_area_index(&assets, sector, layer, point),
            expected
        );
    }
    let mut without_endpoints: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    without_endpoints["asset_geometry"]["doors"][0]
        .as_object_mut()
        .unwrap()
        .remove("world_endpoints");
    let (_, assets) = compiled_walkway(&serde_json::to_vec(&without_endpoints).unwrap());
    assert_eq!(assets.navigation.passage_receivers, vec![None]);
}

#[test]
fn compiled_ordinary_passage_missing_receivers_are_rejected_at_load() {
    let bytes = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/asset-joined-posture.level.json"
    ));
    let loaded = crate::level_data::LoadedLevel::hackable_from_json(bytes).unwrap();
    let (engine, mut assets) = compiled_walkway(bytes);
    Arc::make_mut(&mut assets.environment.static_sight_obstacles).clear();
    let error = engine
        .bind_compiled_passage_receivers(&mut assets, &loaded)
        .unwrap_err();
    assert!(error.to_string().contains("compiled passage receivers"));
}

#[test]
fn compiled_zero_height_passages_bind_the_implicit_ground_plane() {
    let (_, assets) = compiled_walkway(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/asset-ordinary-passage-states.level.json"
    )));
    assert_eq!(
        assets.navigation.passage_receivers,
        vec![Some([None, None]); 2]
    );
}

#[test]
#[ignore = "requires authored spline floor probes and ROBIN_CLIMB_RHS"]
fn exported_spline_floor_clicks_follow_receivers_through_repeats() {
    check_exported_floor_clicks(16, (500., 500.), "spline-floor-actor-report.json");
}

#[test]
#[ignore = "requires overlapping floor exports and ROBIN_CLIMB_RHS"]
fn exported_overlapping_floors_respect_prepared_layers_and_clearance() {
    check_exported_floor_clicks(64, (3000., 2000.), "joined-floor-actor-report.json");
}

#[test]
#[ignore = "requires partitioned loop exports and ROBIN_CLIMB_RHS"]
fn exported_loop_floors_cross_generated_sector_passages() {
    check_exported_loop_floors(false, false);
}

#[test]
#[ignore = "requires partitioned loop exports and ROBIN_CLIMB_RHS"]
fn exported_partitioned_loops_transfer_receivers_at_generated_passages() {
    check_exported_loop_floors(true, false);
}

#[test]
#[ignore = "requires copied folded-floor passage exports and ROBIN_CLIMB_RHS"]
fn exported_folded_floors_cross_prepared_passages() {
    check_exported_loop_floors(false, true);
}

fn check_exported_loop_floors(partitioned_only: bool, folded: bool) {
    let directory = std::path::PathBuf::from(std::env::var("ROBIN_ASSET_MAP_DIAGNOSTICS").unwrap());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("diagnostics.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["complete"], true);
    assert_eq!(
        manifest[if folded { "foldedFloor" } else { "loopFloor" }],
        true
    );
    let sprite = exported_stairs::complete_climb_sprite();
    let mut walks = 0;
    for entry in manifest["results"].as_array().unwrap() {
        if partitioned_only && !matches!(entry["rotation"].as_u64(), Some(0 | 37)) {
            continue;
        }
        let file = entry["file"].as_str().unwrap();
        let bytes = std::fs::read(directory.join(file)).unwrap();
        let descriptor: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        for (probe_index, probe) in entry["probes"].as_array().unwrap().iter().enumerate() {
            for (start_index, goal_index) in [(0, 1), (1, 0)] {
                let point = |index: usize| {
                    MapPoint::new(
                        probe[index][0].as_f64().unwrap() as f32,
                        probe[index][1].as_f64().unwrap() as f32,
                    )
                };
                let start = point(start_index);
                let goal = point(goal_index);
                let layer_at = |index: usize| {
                    entry["probe_layers"][probe_index][index].as_u64().unwrap() as u16
                };
                let layer = layer_at(start_index);
                let goal_layer = layer_at(goal_index);
                let (mut engine, mut assets) =
                    compiled_walkway_with_dimensions(&bytes, (3000., 2000.));
                let sector_at = |point, layer| {
                    let crate::fast_find_grid::SectorHit::Found {
                        sector_idx,
                        sector_number,
                        ..
                    } = engine.world.fast_grid.get_sector(point, point, layer)
                    else {
                        panic!("{file}: missing endpoint at {point:?} on layer {layer}")
                    };
                    crate::position_interface::SectorHandle::from_number(sector_number)
                        .with_arena_index(sector_idx)
                };
                let handle = sector_at(start, layer);
                let goal_handle = sector_at(goal, goal_layer);
                if handle == goal_handle {
                    let mut grid = (*engine.world.fast_grid).clone();
                    let graph = &assets.navigation.pathfinder_graph;
                    let mut finder = crate::pathfinder::PathFinder::new();
                    finder.initialize_from_graph(graph, &mut grid);
                    let path = finder
                        .find_path(graph, &grid, layer, handle.get(), 0, start, goal, true)
                        .unwrap_or_else(|| {
                            panic!("{file}: prepared graph has no route {start:?} -> {goal:?}")
                        });
                    assert_eq!(path.last(), Some(&goal));
                    let click = engine.world.fast_grid.get_sector_screen(goal, start);
                    assert_eq!(
                        click.sector_idx,
                        goal_handle.arena_index(),
                        "{file}: click selects a different floor"
                    );
                }
                let owner = walking_pc(&mut engine, &mut assets, start, layer, handle);
                let element = engine.ent_mut(owner).element_data_mut();
                let position = element.sprite.position_iface.clone();
                element.sprite = sprite.clone();
                element.sprite.position_iface = position;
                let receiver = engine.get_projection_area_index(&assets, handle, layer, start);
                assert!(receiver.is_some());
                engine.set_obstacle_and_material(&assets, owner, receiver);
                let sim = crate::sim_rng::test_context();
                if folded {
                    let planned = engine.plan_recorded_group_move(
                        &assets,
                        &[owner],
                        goal,
                        Some((goal_handle.number(), goal_layer)),
                        goal_handle.arena_index(),
                        None,
                    );
                    assert!(
                        matches!(
                            planned.as_slice(),
                            [crate::engine::movement::formation::PlannedRecordedGroupMoveOutcome::Resolved(_)]
                        ),
                        "{file}: passage {probe_index} direction {start_index}->{goal_index} rejects destination {goal:?} on layer {goal_layer} before movement; result {planned:?}, map box {:?}, world box {:?}",
                        engine.ent(owner).position_iface().get_move_box_map(),
                        engine.ent(owner).position_iface().get_move_box(),
                    );
                }
                engine.perform_group_move(
                    TickCtx::new(&sim, &assets),
                    &[owner],
                    goal,
                    false,
                    false,
                    folded.then_some((goal_handle.number(), goal_layer)),
                    folded.then_some(goal_handle.arena_index()).flatten(),
                    None,
                    &[],
                    &[],
                );
                let half = engine
                    .world
                    .fast_grid
                    .try_move_box_half_diagonal(0)
                    .unwrap();
                let mut previous = start;
                let mut previous_world = engine.ent(owner).element_data().position();
                let world_floors: Vec<geo::Polygon<f32>> = if folded {
                    entry["world_floors"][probe_index]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|polygon| {
                            geo::Polygon::new(
                                geo::LineString::from(
                                    polygon
                                        .as_array()
                                        .unwrap()
                                        .iter()
                                        .map(|p| {
                                            (
                                                p[0].as_f64().unwrap() as f32,
                                                p[1].as_f64().unwrap() as f32,
                                            )
                                        })
                                        .collect::<Vec<_>>(),
                                ),
                                vec![],
                            )
                        })
                        .collect()
                } else {
                    vec![]
                };
                let mut previous_sector = handle;
                let mut distance = 0.;
                let mut crossings = 0;
                let mut arrived = false;
                for _ in 0..2000 {
                    engine.control.frame_counter += 1;
                    engine.t_hourglass_phase_sequences(&assets);
                    engine.hourglass_phase_paths(TickCtx::new(&sim, &assets));
                    engine.t_tick_actor_owner_envelopes(&assets);
                    let element = engine.ent(owner).element_data();
                    let position = element.position_map();
                    let current = element.sector().expect("actor sector");
                    assert!(
                        current == handle || current == goal_handle,
                        "{file}: entered unrelated floor"
                    );
                    let current_layer = if current == handle { layer } else { goal_layer };
                    assert_eq!(element.layer(), current_layer);
                    let receiver_result = actor_receiver_result(
                        &engine,
                        &assets,
                        owner,
                        current,
                        current_layer,
                        position,
                    );
                    if let Err(error) = receiver_result {
                        // While standing at the passage midpoint, its integer position can
                        // lie just beyond the exact floor contour. Require its
                        // load-bound plane, correct height and subpixel edge proximity.
                        let receiver = engine
                            .ent(owner)
                            .position_iface()
                            .get_obstacle()
                            .unwrap_or_else(|| panic!("{file}: {error}"));
                        let prepared =
                            engine
                                .script_domains
                                .interactables
                                .doors
                                .iter()
                                .enumerate()
                                .any(|(index, door)| {
                                    if (door.point_mid - position).length() > 0.001 {
                                        return false;
                                    }
                                    let side = if door.sector_in_index == current.arena_index()
                                        && door.layer_in == current_layer
                                    {
                                        1
                                    } else if door.sector_out_index == current.arena_index()
                                        && door.layer_out == current_layer
                                    {
                                        0
                                    } else {
                                        return false;
                                    };
                                    assets.navigation.passage_receivers.get(index).is_some_and(
                                        |pair| {
                                            pair.is_some_and(|pair| pair[side] == Some(receiver))
                                        },
                                    )
                                });
                        assert!(prepared, "{file}: {error}");
                        let obstacle =
                            &assets.environment.static_sight_obstacles[usize::from(receiver)];
                        let points = &obstacle.obstacle_points;
                        assert!(
                            (0..points.len()).any(|index| {
                                let a = &points[index];
                                let b = &points[(index + 1) % points.len()];
                                let ax = a.x;
                                let ay = a.y - a.z_top;
                                let dx = b.x - ax;
                                let dy = b.y - b.z_top - ay;
                                let length2 = dx * dx + dy * dy;
                                if length2 == 0. {
                                    return false;
                                }
                                let t = (((position.x - ax) * dx + (position.y - ay) * dy)
                                    / length2)
                                    .clamp(0., 1.);
                                (position.x - ax - t * dx).hypot(position.y - ay - t * dy)
                                    <= std::f32::consts::FRAC_1_SQRT_2 + 0.001
                            }),
                            "{file}: {error}"
                        );
                        let height = obstacle.compute_top_z_from_projection(position.x, position.y);
                        assert!(
                            (element.position().z - height).abs() < 0.001,
                            "{file}: {error}"
                        );
                    }
                    let step = (position - previous).length();
                    assert!(step < 20., "{file}: teleported {step} units");
                    let near_passage = descriptor["asset_geometry"]["doors"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|door| {
                            let mid = MapPoint::new(
                                door["point_mid"][0].as_f64().unwrap() as f32,
                                door["point_mid"][1].as_f64().unwrap() as f32,
                            );
                            (position - mid).length() < 20. && (previous - mid).length() < 20.
                        });
                    let crosses_collision = current != previous_sector
                        || !engine.world.fast_grid.is_reachable_thick(
                            previous,
                            position,
                            current_layer,
                            half,
                        );
                    if crosses_collision {
                        assert!(
                            near_passage,
                            "{file}: crossed collision away from a passage {previous:?} -> {position:?}"
                        );
                    }
                    if folded || crosses_collision {
                        // Near a door, the footprint legitimately straddles sectors before
                        // the actor center changes owner. Both floors must support it.
                        for t in [0., 0.25, 0.5, 0.75, 1.] {
                            let center = MapPoint::new(
                                previous.x + (position.x - previous.x) * t,
                                previous.y + (position.y - previous.y) * t,
                            );
                            for dx in [-half.x, 0., half.x] {
                                for dy in [-half.y, 0., half.y] {
                                    let p = MapPoint::new(center.x + dx, center.y + dy);
                                    let supported = if folded {
                                        // Projection-compensated movement contours are not
                                        // physical floors. Check the authored world footprint.
                                        let world = element.position();
                                        let corner = geo::Point::new(
                                            previous_world.x
                                                + (world.x - previous_world.x) * t
                                                + dx,
                                            previous_world.y
                                                + (world.y - previous_world.y) * t
                                                + dy,
                                        );
                                        world_floors.iter().any(|floor| {
                                            geo::Intersects::intersects(floor, &corner)
                                        })
                                    } else {
                                        [(layer, handle), (goal_layer, goal_handle)].iter().any(|&(floor_layer, floor)| {
                                        matches!(engine.world.fast_grid.get_sector(p, p, floor_layer),
                                            crate::fast_find_grid::SectorHit::Found { sector_idx, sector_number, .. }
                                            if crate::position_interface::SectorHandle::from_number(sector_number).with_arena_index(sector_idx) == floor)
                                    })
                                    };
                                    assert!(
                                        supported,
                                        "{file}: unsupported footprint at passage {p:?}"
                                    );
                                }
                            }
                        }
                    }
                    if current != previous_sector {
                        crossings += 1;
                    }
                    distance += step;
                    previous = position;
                    previous_world = element.position();
                    previous_sector = current;
                    if (position - goal).length() < 0.1 && current == goal_handle {
                        let expected_z = entry["world_probes"][probe_index][goal_index][2]
                            .as_f64()
                            .unwrap() as f32;
                        assert!((element.position().z - expected_z).abs() < 0.01);
                        arrived = true;
                        break;
                    }
                }
                assert!(
                    arrived,
                    "{file}: actor stopped at {previous:?}, goal {goal:?}"
                );
                assert_eq!(
                    crossings,
                    usize::from(handle != goal_handle),
                    "{file}: wrong number of sector passages"
                );
                let minimum_distance = if folded {
                    let middle = &descriptor["asset_geometry"]["doors"][probe_index]["point_mid"];
                    let middle = MapPoint::new(
                        middle[0].as_f64().unwrap() as f32,
                        middle[1].as_f64().unwrap() as f32,
                    );
                    (middle - start).length() + (goal - middle).length() - 0.1
                } else {
                    (goal - start).length() * 2.
                };
                assert!(
                    distance > minimum_distance,
                    "{file}: shortcut across stacked floors: travelled {distance}, required {minimum_distance}"
                );
                walks += 1;
            }
        }
    }
    assert_eq!(
        walks,
        if folded {
            manifest["expectedWalks"].as_u64().unwrap() as usize
        } else if partitioned_only {
            32
        } else {
            64
        }
    );
    assert!(walks > 0);
    std::fs::write(
        directory.join(if folded {
            "folded-floor-actor-report.json"
        } else if partitioned_only {
            "partitioned-loop-actor-report.json"
        } else {
            "loop-floor-actor-report.json"
        }),
        serde_json::to_vec(&serde_json::json!({"complete": true, "walks": walks})).unwrap(),
    )
    .unwrap();
}

fn check_exported_floor_clicks(expected: usize, dimensions: (f32, f32), report: &str) {
    let directory = std::path::PathBuf::from(std::env::var("ROBIN_ASSET_MAP_DIAGNOSTICS").unwrap());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("diagnostics.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["complete"], true);
    let sprite = exported_stairs::complete_climb_sprite();
    let mut walks = 0;
    let mut blocked_moves = 0;
    for entry in manifest["results"].as_array().unwrap() {
        let file = entry["file"].as_str().unwrap();
        let bytes = std::fs::read(directory.join(file)).unwrap();
        let layer = entry["layer"].as_u64().unwrap() as u16;
        let blocked = entry["blocked"].as_bool().unwrap_or(false);
        for probe in entry["probes"].as_array().unwrap() {
            let point = |i: usize| {
                MapPoint::new(
                    probe[i][0].as_f64().unwrap() as f32,
                    probe[i][1].as_f64().unwrap() as f32,
                )
            };
            for (start, goal) in [(point(0), point(1)), (point(1), point(0))] {
                let (mut engine, mut assets) = compiled_walkway_with_dimensions(&bytes, dimensions);
                if blocked {
                    let point = &entry["barrier"];
                    let barrier = MapPoint::new(
                        point[0].as_f64().unwrap() as f32,
                        point[1].as_f64().unwrap() as f32,
                    );
                    assert!(
                        !matches!(
                            engine.world.fast_grid.get_sector(barrier, barrier, layer),
                            crate::fast_find_grid::SectorHit::Found { .. }
                        ),
                        "{file}: low ceiling has no prepared exclusion"
                    );
                }
                let crate::fast_find_grid::SectorHit::Found {
                    sector_idx,
                    sector_number,
                    ..
                } = engine.world.fast_grid.get_sector(start, start, layer)
                else {
                    panic!("missing start")
                };
                let handle = crate::position_interface::SectorHandle::from_number(sector_number)
                    .with_arena_index(sector_idx);
                let owner = walking_pc(&mut engine, &mut assets, start, layer, handle);
                let element = engine.ent_mut(owner).element_data_mut();
                let position = element.sprite.position_iface.clone();
                element.sprite = sprite.clone();
                element.sprite.position_iface = position;
                let receiver = engine.get_projection_area_index(&assets, handle, layer, start);
                assert!(receiver.is_some());
                engine.set_obstacle_and_material(&assets, owner, receiver);
                let sim = crate::sim_rng::test_context();
                engine.perform_group_move(
                    TickCtx::new(&sim, &assets),
                    &[owner],
                    goal,
                    false,
                    false,
                    None,
                    None,
                    None,
                    &[],
                    &[],
                );
                let half = engine
                    .world
                    .fast_grid
                    .try_move_box_half_diagonal(0)
                    .unwrap();
                let mut previous = start;
                let mut distance = 0.;
                let mut arrived = false;
                let mut settled_at_end = false;
                for tick in 0..2000 {
                    engine.control.frame_counter += 1;
                    engine.t_hourglass_phase_sequences(&assets);
                    engine.hourglass_phase_paths(TickCtx::new(&sim, &assets));
                    engine.t_tick_actor_owner_envelopes(&assets);
                    let element = engine.ent(owner).element_data();
                    let position = element.position_map();
                    assert_eq!(element.layer(), layer);
                    assert_eq!(element.sector(), Some(handle));
                    assert!(
                        engine
                            .get_projection_area_index(&assets, handle, layer, position)
                            .is_some(),
                        "{file}: actor left the authored receiving floor at {position:?}"
                    );
                    assert!(
                        engine
                            .world
                            .fast_grid
                            .is_reachable_thick(previous, position, layer, half),
                        "{file}: actor crossed collision {previous:?} -> {position:?}"
                    );
                    actor_receiver_result(&engine, &assets, owner, handle, layer, position)
                        .unwrap_or_else(|error| panic!("{file}: {error}"));
                    distance += (position - previous).length();
                    previous = position;
                    let settled = engine
                        .entities()
                        .current_element_for_actor(owner)
                        .and_then(|(id, index)| engine.seq().get_element(id, index))
                        .is_none_or(|selected| {
                            selected.orders.is_empty() || selected.command == Command::Wait
                        });
                    if settled && ((position - goal).length() < 0.1 || (blocked && tick >= 119)) {
                        arrived = (position - goal).length() < 0.1;
                        settled_at_end = true;
                        break;
                    }
                }
                assert!(settled_at_end, "{file}: move order did not settle");
                assert_eq!(
                    arrived, !blocked,
                    "{file}: actor stopped at {previous:?}, goal {goal:?}, blocked {blocked}"
                );
                assert!(
                    distance < (goal - start).length() * 2.,
                    "{file}: unexpected detour"
                );
                walks += 1;
                blocked_moves += usize::from(blocked);
            }
        }
    }
    assert_eq!(walks, expected);
    std::fs::write(
        directory.join(report),
        serde_json::to_vec(&serde_json::json!({"complete":true,"moves":walks,"walks":walks-blocked_moves,"blocked":blocked_moves})).unwrap(),
    )
    .unwrap();
}
