use super::*;

#[test]
fn overlapping_stairs_on_separate_layers_do_not_block_each_other() {
    let mut document: serde_json::Value = serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/asset-lift.level.json"
    )))
    .unwrap();
    let mut geometry: crate::level_data::CompiledAssetGeometry =
        serde_json::from_value(document["asset_geometry"].clone()).unwrap();
    let mut area = geometry.motion_data.layers[2][0].clone();
    for point in &mut area.polygon.points {
        point.0 += 5;
    }
    geometry.motion_data.layers.push(vec![area]);
    let mut receiver = geometry.sight_obstacles[2].clone();
    for point in &mut receiver.points {
        point.x += 5.;
    }
    receiver.projection_area = Some((4, 3));
    geometry.sight_obstacles.push(receiver);
    let mut lift = geometry.lifts[0].clone();
    lift.motion_area_index = 4;
    for door in &mut lift.doors {
        door.sector_in = 4;
        door.layer_in = 3;
        door.point_in.0 += 5;
        door.point_out.0 += 5;
        door.point_mid.0 += 5;
    }
    geometry.lifts.push(lift);
    document["asset_geometry"] = serde_json::to_value(geometry).unwrap();
    let (engine, assets) = compiled_walkway(&serde_json::to_vec(&document).unwrap());
    for (entrance, exit) in [(0, 1), (1, 0), (2, 3), (3, 2)] {
        assert_eq!(
            walk_exported_stairs(engine.clone(), assets.clone(), entrance, exit),
            Ok(true)
        );
    }
}

#[test]
fn stair_passages_bridge_gaps_overlaps_and_ground_edges_after_placement() {
    let bytes = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/asset-lift.level.json"
    ));
    for gap in [-1, 1] {
        let mut document: serde_json::Value = serde_json::from_slice(bytes).unwrap();
        let mut geometry: crate::level_data::CompiledAssetGeometry =
            serde_json::from_value(document["asset_geometry"].clone()).unwrap();
        for point in &mut geometry.motion_data.layers[2][0].polygon.points {
            point.0 += if point.0 < 400 { gap } else { -gap };
        }
        for point in &mut geometry.sight_obstacles[2].points {
            point.x += if point.x < 400. {
                f32::from(gap)
            } else {
                -f32::from(gap)
            };
            // Keep a genuine receiving-polygon edge inside the navigation
            // boundary, exercising the doorway's replacement of that edge.
            point.x += if point.x < 400. { 0.25 } else { -0.25 };
        }
        document["asset_geometry"] = serde_json::to_value(geometry).unwrap();
        let bytes = serde_json::to_vec(&document).unwrap();
        for turn in 0..4 {
            let placed = super::compiled_lifts::placed_stair_fixture(&bytes, turn, 1);
            let (engine, assets) = compiled_walkway(&placed);
            for (entrance, exit) in [(0, 1), (1, 0)] {
                assert_eq!(
                    walk_exported_stairs(engine.clone(), assets.clone(), entrance, exit),
                    Ok(true),
                    "gap={gap}, turn={turn}, entrance={entrance}"
                );
            }
        }
    }
}

fn walk_exported_stairs(
    mut engine: EngineInner,
    mut assets: LevelAssets,
    entrance: usize,
    exit: usize,
) -> Result<bool, String> {
    let doors = &engine.script_domains.interactables.doors;
    let enter = doors[entrance].clone();
    let leave = doors[exit].clone();
    let source_sector = crate::position_interface::SectorHandle::from_number(enter.sector_out)
        .with_arena_index(enter.sector_out_index.unwrap());
    let destination_sector = crate::position_interface::SectorHandle::from_number(leave.sector_out)
        .with_arena_index(leave.sector_out_index.unwrap());
    let lift_sector = enter.sector_in_index.unwrap();
    assert_eq!(Some(lift_sector), leave.sector_in_index);
    let owner = walking_pc(
        &mut engine,
        &mut assets,
        enter.point_out,
        enter.layer_out,
        source_sector,
    );
    let auth = engine.ent(owner).actor_auth_info();
    if !enter.active
        || !leave.active
        || !enter.is_actor_authorized(true, &auth, true, false)
        || !leave.is_actor_authorized(false, &auth, true, false)
    {
        return Ok(false);
    }
    let receiver =
        engine.get_projection_area_index(&assets, source_sector, enter.layer_out, enter.point_out);
    engine.set_obstacle_and_material(&assets, owner, receiver);
    let sim = crate::sim_rng::test_context();
    let path = vec![
        crate::gate::GatePathStep {
            door_index: crate::gate::DoorIndex::new(entrance as u32).unwrap(),
            direct: true,
        },
        crate::gate::GatePathStep {
            door_index: crate::gate::DoorIndex::new(exit as u32).unwrap(),
            direct: false,
        },
    ];
    engine
        .launch_gate_movement_sequence(
            TickCtx::new(&sim, &assets),
            &mut vec![],
            crate::engine::movement::GateRouteRequest {
                entity_id: owner,
                source_sector: Some(source_sector),
                gate_path: path,
                goal: crate::engine::movement::GoalShape::Point {
                    point: leave.point_out,
                    tolerance: 0.,
                },
                goal_layer: leave.layer_out,
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
        .ok_or("could not construct walking stair route sequence")?;
    let distance = (enter.point_out - enter.point_mid).length()
        + (enter.point_mid - enter.point_in).length()
        + (enter.point_in - leave.point_in).length()
        + (leave.point_in - leave.point_mid).length()
        + (leave.point_mid - leave.point_out).length();
    let mut crossed = false;
    let mut previous = enter.point_out;
    let mut stationary = 0;
    for _ in 0..(distance.ceil() as usize * 4 + 1000) {
        engine.control.frame_counter += 1;
        engine.t_hourglass_phase_sequences(&assets);
        engine.hourglass_phase_paths(TickCtx::new(&sim, &assets));
        engine.t_tick_actor_owner_envelopes(&assets);
        let element = engine.ent(owner).element_data();
        let position = element.position_map();
        let sector = element.sector().ok_or("stair actor lost its sector")?;
        crossed |= sector.arena_index() == Some(lift_sector);
        // A passage straddles two receiving polygons while the sector changes
        // at its midpoint. Require ordinary lookup agreement once its explicit
        // approach movement has finished, including the destination.
        let passing = crate::engine::ai::selected_actor_is_passing_door(
            &engine.entities(),
            &engine.seq(),
            owner,
        );
        if !passing {
            actor_receiver_result(&engine, &assets, owner, sector, element.layer(), position)?;
        }
        if crossed
            && (position - leave.point_out).length() < 0.01
            && element.layer() == leave.layer_out
            && sector == destination_sector
        {
            actor_receiver_result(&engine, &assets, owner, sector, element.layer(), position)?;
            return Ok(true);
        }
        stationary = if position == previous {
            stationary + 1
        } else {
            0
        };
        previous = position;
        if stationary >= 200 {
            let bounds = *engine.ent(owner).position_iface().get_move_box_map();
            let blockers: Vec<_> = engine
                .world
                .fast_grid
                .get_active_motion_line_indices(element.layer(), &bounds)
                .iter()
                .map(|index| &engine.world.fast_grid.level.lines[usize::from(*index)])
                .filter(|line| line.intersects_bbox(&bounds))
                .map(|line| (line.a, line.b))
                .collect();
            let selected = engine
                .entities()
                .current_element_for_actor(owner)
                .and_then(|(id, index)| engine.seq().get_element(id, index))
                .map(|element| element.command);
            return Err(format!(
                "stair route stalled at {position:?}, layer {}, sector {sector:?}, goal {:?}, crossed={crossed}, bounds={bounds:?}, blockers={blockers:?}, selected={selected:?}",
                element.layer(),
                leave.point_out,
            ));
        }
    }
    Err(format!(
        "stair route exhausted its movement budget at {previous:?}, goal {:?}, crossed={crossed}",
        leave.point_out
    ))
}

#[test]
#[ignore = "requires current exports via ROBIN_ASSET_MAP_DIAGNOSTICS"]
fn exported_stairs_support_complete_actor_routes() {
    let directory = std::path::PathBuf::from(std::env::var("ROBIN_ASSET_MAP_DIAGNOSTICS").unwrap());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("diagnostics.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["complete"], true);
    let report_path = directory.join("actor-stair-route-report.json");
    let mut report = serde_json::json!({
        "scope": "initial-state-directed-stair-walks-between-every-entrance-pair",
        "complete": false, "audit_finished": false, "results": []
    });
    let mut total_checked = 0;
    let mut total_failed = 0;
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    for result in manifest["results"].as_array().unwrap() {
        let file = result["file"].as_str().unwrap();
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
        let mut checked = 0;
        let mut skipped = 0;
        let mut failures = vec![];
        let doors = &engine.script_domains.interactables.doors;
        for (sector_index, sector) in engine.world.fast_grid.level.sectors.iter().enumerate() {
            if sector.lift_type != Some(crate::sector::LiftType::Stairs) {
                continue;
            }
            let entrances: Vec<_> = doors
                .iter()
                .enumerate()
                .filter_map(|(index, door)| {
                    (door.sector_in_index.map(usize::from) == Some(sector_index)).then_some(index)
                })
                .collect();
            for &entrance in &entrances {
                for &exit in &entrances {
                    if entrance == exit {
                        continue;
                    }
                    let outcome =
                        walk_exported_stairs(engine.clone(), assets.clone(), entrance, exit);
                    match outcome {
                        Ok(true) => checked += 1,
                        Ok(false) => skipped += 1,
                        Err(message) => {
                            checked += 1;
                            failures.push(serde_json::json!({
                                "sector": sector.sector_number, "entrance": entrance, "exit": exit, "error": message
                            }));
                        }
                    }
                }
            }
        }
        total_checked += checked;
        total_failed += failures.len();
        eprintln!(
            "{file}: {checked} stair routes, {} failures, {skipped} forbidden routes",
            failures.len()
        );
        report["results"].as_array_mut().unwrap().push(serde_json::json!({
            "file": file, "checked": checked, "skipped_permissions": skipped, "failures": failures
        }));
        std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    report["audit_finished"] = true.into();
    report["complete"] = (total_checked > 0 && total_failed == 0).into();
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    assert!(total_checked > 0, "no stair routes tested");
    assert_eq!(total_failed, 0, "see {}", report_path.display());
}
