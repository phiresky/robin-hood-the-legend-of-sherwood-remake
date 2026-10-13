use super::*;
use crate::element::{ActionState, Command, Entity, Posture};
use crate::engine::{Engine, EngineArgs, LevelLoadArgs, SimConfig};
use crate::sequence::SequenceElement;
use std::sync::Arc;

mod exported_receivers {
    include!("exported_receivers.rs");
}

mod compiled_lifts {
    include!("compiled_lifts.rs");
}

mod exported_stairs {
    include!("exported_stairs.rs");
}

mod exported_jumps {
    include!("exported_jumps.rs");
}

mod compiled_posture_closure {
    include!("compiled_posture_closure.rs");
}

mod compiled_trigger_occupancy {
    include!("compiled_trigger_occupancy.rs");
}

mod compiled_sight_barriers {
    include!("compiled_sight_barriers.rs");
}

mod compiled_spline_floors {
    include!("compiled_spline_floors.rs");
}

mod compiled_building_clicks {
    include!("compiled_building_clicks.rs");
}

#[test]
fn exported_floor_flags_require_crouching_only_in_the_authored_room() {
    let bytes = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/asset-forced-crouch.level.json"
    ));
    for (sector_number, x, expected) in [(0, 320., Posture::Upright), (2, 450., Posture::Crouched)]
    {
        let (mut engine, mut assets) = compiled_walkway(bytes);
        let sector = crate::position_interface::SectorHandle::new(sector_number)
            .unwrap()
            .with_arena_index(
                crate::fast_find_grid::SectorIndex::new(u32::from(sector_number)).unwrap(),
            );
        let owner = walking_pc(&mut engine, &mut assets, MapPoint::new(x, 350.), 0, sector);
        let mut element = SequenceElement::new(1, Command::Wait, Some(owner));
        element.posture_after_transition = Posture::Upright;
        element.action_state_after_transition = ActionState::Moving;
        let sequence = engine.orders.sequence_manager.insert_element(element);
        engine
            .orders
            .sequence_manager
            .start_sequence_level(sequence);
        let sim = crate::sim_rng::test_context();
        assert!(engine.generate_transition(
            crate::engine::TickCtx::new(&sim, &assets),
            &mut Vec::new(),
            owner,
            crate::sequence::SequenceElementRef::new(sequence, 0),
        ));
        let element = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap();
        assert_eq!(element.posture_after_transition, expected);
        assert_eq!(
            element
                .orders
                .iter()
                .any(|order| order.order_type == OrderType::TransitionCrouchingDown),
            expected == Posture::Crouched,
        );
    }
}

#[test]
#[ignore = "requires ROBIN_CLIMB_RHS for complete character animations"]
fn actors_cross_generated_posture_socket_passages() {
    audit_posture_passages(
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/asset-joined-posture.level.json"
        )),
        40.,
        1,
    );
}

#[test]
#[ignore = "requires ROBIN_ASSET_MAP_DIAGNOSTICS and ROBIN_CLIMB_RHS"]
fn exported_posture_passage_zips_support_native_player_routes() {
    let directory = std::path::PathBuf::from(std::env::var("ROBIN_ASSET_MAP_DIAGNOSTICS").unwrap());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("diagnostics.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["complete"], true);
    let mut results = vec![];
    let mut crossings = 0;
    for entry in manifest["results"].as_array().unwrap() {
        let file = entry["file"].as_str().unwrap();
        let count = entry["expected_doors"].as_u64().unwrap() as usize;
        let height = entry["height"].as_f64().unwrap() as f32;
        let initially_closed = entry["initially_closed"] == true;
        let alternating = entry["alternating"] == true;
        let passed = audit_posture_passages_with_controls(
            &std::fs::read(directory.join(file)).unwrap(),
            height,
            count,
            initially_closed,
            alternating,
        );
        crossings += passed;
        eprintln!("{file}: {passed} complete posture crossings");
        results.push(
            serde_json::json!({"file":file, "crossings":passed, "doors":count, "height":height, "initially_closed":initially_closed, "alternating":alternating}),
        );
    }
    assert!(crossings > 0);
    std::fs::write(directory.join("actor-posture-report.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "complete":true, "crossings":crossings, "scope":"native-player-routes-from-editable-zips-not-rendering", "results":results,
    })).unwrap()).unwrap();
}

#[test]
#[ignore = "requires alternating posture ZIP diagnostics and ROBIN_CLIMB_RHS"]
fn exported_posture_openings_support_player_formations() {
    let directory = std::path::PathBuf::from(std::env::var("ROBIN_ASSET_MAP_DIAGNOSTICS").unwrap());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("diagnostics.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["complete"], true);
    let sprite = exported_stairs::complete_climb_sprite();
    let mut crossings = 0;
    for entry in manifest["results"].as_array().unwrap() {
        assert_eq!(entry["alternating"], true);
        let file = entry["file"].as_str().unwrap();
        let bytes = std::fs::read(directory.join(file)).unwrap();
        let descriptor: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let dimensions = (
            descriptor["walkable_polygon"][2][0].as_f64().unwrap() as f32 + 1.,
            descriptor["walkable_polygon"][2][1].as_f64().unwrap() as f32 + 1.,
        );
        for changed in [false, true] {
            for reverse in [false, true] {
                for copy in 0..2 {
                    let (mut engine, mut assets) =
                        compiled_walkway_with_dimensions(&bytes, dimensions);
                    let sim = crate::sim_rng::test_context();
                    if changed {
                        engine.apply_patch(
                            TickCtx::new(&sim, &assets),
                            crate::patch::PatchIndex::new(copy).unwrap(),
                        );
                    }
                    let doors = &engine.script_domains.interactables.doors;
                    let door = doors
                        .iter()
                        .find(|door| {
                            let other_copy = door.sector_out_index != doors[0].sector_out_index
                                || door.sector_in_index != doors[0].sector_in_index;
                            other_copy == (copy != 0) && !door.passage_blocked
                        })
                        .unwrap()
                        .clone();
                    let (start, end, source, destination, layer) = if reverse {
                        (
                            door.point_in,
                            door.point_out,
                            door.sector_in_index.unwrap(),
                            door.sector_out_index.unwrap(),
                            door.layer_in,
                        )
                    } else {
                        (
                            door.point_out,
                            door.point_in,
                            door.sector_out_index.unwrap(),
                            door.sector_in_index.unwrap(),
                            door.layer_out,
                        )
                    };
                    let direction = (end - start).scale(1. / (end - start).length());
                    let center = |index: crate::fast_find_grid::SectorIndex| {
                        let points =
                            &engine.world.fast_grid.level.sectors[usize::from(index)].points;
                        MapPoint::new(
                            points.iter().map(|p| p.x).sum::<f32>() / points.len() as f32,
                            points.iter().map(|p| p.y).sum::<f32>() / points.len() as f32,
                        )
                    };
                    let source_center = center(source);
                    let goal = center(destination);
                    let handle = crate::position_interface::SectorHandle::from_number(
                        engine.world.fast_grid.level.sectors[usize::from(source)].sector_number,
                    )
                    .with_arena_index(source);
                    let mut owners = vec![];
                    for offset in [-14., 0., 14.] {
                        let position = source_center + direction.scale(offset);
                        assert!(
                            engine.world.fast_grid.is_reachable_thick(
                                position,
                                position,
                                layer,
                                crate::coordinates::MoveBoxHalfDiagonal::new(6., 3.),
                            ),
                            "invalid formation start in {file}: {position:?}"
                        );
                        let owner = walking_pc(&mut engine, &mut assets, position, layer, handle);
                        let element = engine.ent_mut(owner).element_data_mut();
                        let iface = element.sprite.position_iface.clone();
                        element.sprite = sprite.clone();
                        element.sprite.position_iface = iface;
                        let receiver =
                            engine.get_projection_area_index(&assets, handle, layer, position);
                        engine.set_obstacle_and_material(&assets, owner, receiver);
                        owners.push(owner);
                    }
                    engine.perform_group_move(
                        TickCtx::new(&sim, &assets),
                        &owners,
                        goal,
                        false,
                        false,
                        None,
                        None,
                        None,
                        &[],
                        &[],
                    );
                    let expected = if engine.world.fast_grid.level.sectors[usize::from(destination)]
                        .force_crouched
                    {
                        Posture::Crouched
                    } else {
                        Posture::Upright
                    };
                    let mut arrived = false;
                    for _ in 0..1200 {
                        engine.control.frame_counter += 1;
                        engine.t_hourglass_phase_sequences(&assets);
                        engine.hourglass_phase_paths(TickCtx::new(&sim, &assets));
                        engine.t_tick_actor_owner_envelopes(&assets);
                        arrived = owners.iter().all(|&owner| {
                            let element = engine.ent(owner).element_data();
                            let settled = engine
                                .entities()
                                .current_element_for_actor(owner)
                                .and_then(|(id, index)| engine.seq().get_element(id, index))
                                .is_none_or(|selected| {
                                    selected.orders.is_empty() || selected.command == Command::Wait
                                });
                            element.sector().and_then(|sector| sector.arena_index())
                                == Some(destination)
                                && element.posture() == expected
                                && (element.position_map() - goal).length() < 25.
                                && settled
                        });
                        if arrived {
                            break;
                        }
                    }
                    assert!(
                        arrived,
                        "{file}: formation failed copy={copy} changed={changed} reverse={reverse}; positions={:?}",
                        owners
                            .iter()
                            .map(|&owner| engine.ent(owner).element_data().position_map())
                            .collect::<Vec<_>>()
                    );
                    // Obstacle authorization may shift individual formation slots.
                    // Require clear, separate footprints rather than exact offsets.
                    for (index, &owner) in owners.iter().enumerate() {
                        let position = engine.ent(owner).element_data().position_map();
                        assert!(engine.world.fast_grid.is_reachable_thick(
                            position,
                            position,
                            layer,
                            crate::coordinates::MoveBoxHalfDiagonal::new(6., 3.),
                        ));
                        for &other in &owners[index + 1..] {
                            let delta = position - engine.ent(other).element_data().position_map();
                            assert!(
                                delta.x.abs() >= 12. || delta.y.abs() >= 6.,
                                "formation footprints overlap in {file}"
                            );
                        }
                    }
                    for owner in owners {
                        assert!(
                            (engine.ent(owner).element_data().position().z
                                - entry["height"].as_f64().unwrap() as f32)
                                .abs()
                                < 0.001
                        );
                        crossings += 1;
                    }
                }
            }
        }
    }
    assert_eq!(crossings, 192);
    std::fs::write(directory.join("actor-formation-report.json"), serde_json::to_vec_pretty(&serde_json::json!({"complete":true,"crossings":crossings,"scope":"three-player-group-movement-through-alternating-posture-openings"})).unwrap()).unwrap();
}

fn audit_posture_passages(bytes: &[u8], height: f32, expected_doors: usize) -> usize {
    audit_posture_passages_with_controls(bytes, height, expected_doors, false, false)
}

#[test]
#[ignore = "requires ROBIN_CLIMB_RHS"]
fn initially_closed_posture_connections_support_player_routes_after_opening() {
    assert_eq!(
        audit_posture_passages_with_controls(
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/asset-closed-posture.level.json"
            )),
            40.,
            2,
            true,
            false
        ),
        4
    );
}

fn audit_posture_passages_with_controls(
    bytes: &[u8],
    height: f32,
    expected_doors: usize,
    open_controls: bool,
    alternate_controls: bool,
) -> usize {
    let sprite = exported_stairs::complete_climb_sprite();
    let descriptor: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    let dimensions = (
        descriptor["walkable_polygon"][2][0].as_f64().unwrap() as f32 + 1.,
        descriptor["walkable_polygon"][2][1].as_f64().unwrap() as f32 + 1.,
    );
    assert_eq!(
        compiled_walkway_with_dimensions(bytes, dimensions)
            .0
            .script_domains
            .interactables
            .doors
            .len(),
        expected_doors
    );
    for index in 0..expected_doors {
        for reverse in [false, true] {
            let (mut engine, mut assets) = compiled_walkway_with_dimensions(bytes, dimensions);
            if alternate_controls {
                let doors = &engine.script_domains.interactables.doors;
                let copy = |i: usize| {
                    usize::from(
                        doors[i].sector_out_index != doors[0].sector_out_index
                            || doors[i].sector_in_index != doors[0].sector_in_index,
                    )
                };
                let owner_copy = copy(index);
                let before: Vec<_> = doors
                    .iter()
                    .enumerate()
                    .map(|(i, door)| (copy(i), door.passage_blocked))
                    .collect();
                if before[index].1 {
                    let sim = crate::sim_rng::test_context();
                    engine.apply_patch(
                        TickCtx::new(&sim, &assets),
                        crate::patch::PatchIndex::new(owner_copy as u32).unwrap(),
                    );
                }
                assert!(!engine.script_domains.interactables.doors[index].passage_blocked);
                for (i, (copy, blocked)) in before.into_iter().enumerate() {
                    if copy != owner_copy {
                        assert_eq!(
                            engine.script_domains.interactables.doors[i].passage_blocked,
                            blocked
                        );
                    }
                }
            }
            if open_controls {
                assert!(
                    engine
                        .script_domains
                        .interactables
                        .doors
                        .iter()
                        .all(|door| door.passage_blocked)
                );
                let sim = crate::sim_rng::test_context();
                engine.apply_patch(
                    TickCtx::new(&sim, &assets),
                    crate::patch::PatchIndex::new(index as u32).unwrap(),
                );
                assert!(
                    engine
                        .script_domains
                        .interactables
                        .doors
                        .iter()
                        .enumerate()
                        .all(|(i, door)| door.passage_blocked == (i != index))
                );
            }
            let door = engine.script_domains.interactables.doors[index].clone();
            let (start, goal, source, destination, layer) = if reverse {
                (
                    door.point_in,
                    door.point_out,
                    door.sector_in_index.unwrap(),
                    door.sector_out_index.unwrap(),
                    door.layer_in,
                )
            } else {
                (
                    door.point_out,
                    door.point_in,
                    door.sector_out_index.unwrap(),
                    door.sector_in_index.unwrap(),
                    door.layer_out,
                )
            };
            let handle = crate::position_interface::SectorHandle::from_number(
                engine.world.fast_grid.level.sectors[usize::from(source)].sector_number,
            )
            .with_arena_index(source);
            let owner = walking_pc(&mut engine, &mut assets, start, layer, handle);
            let element = engine.ent_mut(owner).element_data_mut();
            let position = element.sprite.position_iface.clone();
            element.sprite = sprite.clone();
            element.sprite.position_iface = position;
            let receiver = engine.get_projection_area_index(&assets, handle, layer, start);
            engine.set_obstacle_and_material(&assets, owner, receiver);
            let sim = crate::sim_rng::test_context();
            engine.perform_group_move(
                crate::engine::TickCtx::new(&sim, &assets),
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
            let expected =
                if engine.world.fast_grid.level.sectors[usize::from(destination)].force_crouched {
                    Posture::Crouched
                } else {
                    Posture::Upright
                };
            let mut arrived = false;
            for _ in 0..600 {
                engine.control.frame_counter += 1;
                engine.t_hourglass_phase_sequences(&assets);
                engine.hourglass_phase_paths(crate::engine::TickCtx::new(&sim, &assets));
                engine.t_tick_actor_owner_envelopes(&assets);
                let element = engine.ent(owner).element_data();
                // A passage permits the footprint to straddle two floor sectors.
                // Check its prepared collision state and actual corridor, rather
                // than applying a single-floor reachability test at the seam.
                assert!(!engine.script_domains.interactables.doors[index].passage_blocked);
                let distance = |a: MapPoint, b: MapPoint| {
                    let p = element.position_map();
                    let dx = b.x - a.x;
                    let dy = b.y - a.y;
                    let squared = dx * dx + dy * dy;
                    let t = if squared > 0. {
                        (((p.x - a.x) * dx + (p.y - a.y) * dy) / squared).clamp(0., 1.)
                    } else {
                        0.
                    };
                    (p.x - a.x - dx * t).hypot(p.y - a.y - dy * t)
                };
                assert!(
                    distance(door.point_out, door.point_mid)
                        .min(distance(door.point_mid, door.point_in))
                        < 0.001,
                    "actor left its prepared passage corridor: {:?}",
                    element.position_map()
                );
                let settled = engine
                    .entities()
                    .current_element_for_actor(owner)
                    .and_then(|(id, index)| engine.seq().get_element(id, index))
                    .is_none_or(|selected| {
                        selected.orders.is_empty() || selected.command == Command::Wait
                    });
                if (element.position_map() - goal).length() < 0.1
                    && element.posture() == expected
                    && element.sector().and_then(|sector| sector.arena_index()) == Some(destination)
                    && settled
                {
                    arrived = true;
                    break;
                }
            }
            assert!(
                arrived,
                "generated posture passage failed in reverse={reverse}: {:?}",
                engine.ent(owner).element_data().position_map()
            );
            assert!((engine.ent(owner).element_data().position().z - height).abs() < 0.001);
        }
    }
    expected_doors * 2
}

fn compiled_walkway(bytes: &[u8]) -> (EngineInner, LevelAssets) {
    compiled_walkway_with_dimensions(bytes, (2000., 2000.))
}

fn compiled_walkway_with_dimensions(
    bytes: &[u8],
    dimensions: (f32, f32),
) -> (EngineInner, LevelAssets) {
    let loaded = crate::level_data::LoadedLevel::hackable_from_json(bytes).unwrap();
    let mut assets = LevelAssets::new();
    let mut profiles = crate::profiles::ProfileManager::new();
    let mut campaign = crate::campaign::Campaign::new();
    let mission = campaign
        .force_next_mission_by_name(&mut profiles, "walkway", "walkway", true)
        .unwrap();
    campaign.current_mission_idx = Some(mission);
    assets.profile_manager = Arc::new(profiles);
    let engine = Engine::new(EngineArgs {
        campaign,
        level: LevelLoadArgs {
            assets: &mut assets,
            level_directory: "",
            progress: &mut |_| {},
            loaded,
            bg_pixel_dims: dimensions,
        },
        ground_mark_sprite: None,
        titbit_row_frame_counts: vec![],
        rng_seed: 0,
        original_rng_replay: None,
        sim_config: SimConfig {
            script_enabled: false,
            ..Default::default()
        },
    })
    .unwrap();
    ((*engine).clone(), assets)
}

#[test]
fn actors_cross_sloped_terrain_sockets_after_rotation() {
    let cases: serde_json::Value = serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/asset-sloped-terrain-sockets.json"
    )))
    .unwrap();
    for case in cases.as_array().unwrap() {
        let bytes = serde_json::to_vec(&case["descriptor"]).unwrap();
        let (engine, assets) = compiled_walkway(&bytes);
        let point = |index: usize| {
            MapPoint::new(
                case["route"][index][0].as_f64().unwrap() as f32,
                case["route"][index][1].as_f64().unwrap() as f32,
            )
        };
        eprintln!("sloped socket rotation {}", case["rotation"]);
        for (source, goal) in [(point(0), point(1)), (point(1), point(0))] {
            tick_walkway_crossing(engine.clone(), assets.clone(), 0, 0, source, goal);
        }
    }
}

#[test]
fn actor_ticks_cross_compiled_walkway_seams_and_update_height() {
    for (layer, sector_index, source, goal) in [
        (0, 0, MapPoint::new(396., 320.), MapPoint::new(404., 290.)),
        (0, 0, MapPoint::new(404., 290.), MapPoint::new(396., 320.)),
        (1, 1, MapPoint::new(510., 325.), MapPoint::new(510., 280.)),
        (1, 1, MapPoint::new(510., 280.), MapPoint::new(510., 325.)),
    ] {
        let (engine, assets) = compiled_walkway(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/asset-navigation-copies.level.json"
        )));
        tick_walkway_crossing(engine, assets, layer, sector_index, source, goal);
    }
}

#[test]
fn actor_crosses_receivers_in_partial_edge_grid_cells() {
    let bytes = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/asset-navigation-copies.level.json"
    ));
    for (dimensions, layer, sector, a, b) in [
        (
            (420., 350.),
            0,
            0,
            MapPoint::new(396., 320.),
            MapPoint::new(404., 290.),
        ),
        (
            (620., 319.),
            1,
            1,
            MapPoint::new(510., 313.),
            MapPoint::new(510., 300.),
        ),
    ] {
        for (source, goal) in [(a, b), (b, a)] {
            let (engine, assets) = compiled_walkway_with_dimensions(bytes, dimensions);
            let grid = &engine.world.fast_grid.level;
            assert_eq!(grid.grid_width, (dimensions.0 as u16).div_ceil(64));
            assert_eq!(grid.grid_height, (dimensions.1 as u16).div_ceil(64) + 4);
            assert!(
                grid.map_bbox
                    .contains_point(MapPoint::new(dimensions.0 - 1., dimensions.1 - 1.))
            );
            assert!(
                !grid
                    .map_bbox
                    .contains_point(MapPoint::new(dimensions.0, dimensions.1 - 1.))
            );
            assert!(
                !grid
                    .map_bbox
                    .contains_point(MapPoint::new(dimensions.0 - 1., dimensions.1))
            );
            tick_walkway_crossing(engine, assets, layer, sector, source, goal);
        }
    }
}

#[test]
fn actor_steps_between_receiving_plane_and_uncovered_ground() {
    let mut descriptor: serde_json::Value = serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/asset-navigation-copies.level.json"
    )))
    .unwrap();
    let geometry = &mut descriptor["asset_geometry"];
    geometry["motion_data"]["layers"][0][0]["polygon"]["points"] =
        serde_json::json!([[0, 0], [100, 0], [100, 100], [0, 100]]);
    geometry["sight_obstacles"]
        .as_array_mut()
        .unwrap()
        .truncate(1);
    geometry["sight_obstacles"][0]["points"] = serde_json::json!(
        [(40, 20), (80, 20), (80, 80), (40, 80)]
            .map(|(x, y)| serde_json::json!({"x": x, "y": y + 16, "z_bottom": 16, "z_top": 16}))
    );
    geometry["sight_obstacles"][0]["projection_plane"] =
        serde_json::json!([[40, 36, 16], [80, 36, 16], [40, 96, 16]]);
    let bytes = serde_json::to_vec(&descriptor).unwrap();
    let a = MapPoint::new(28., 50.);
    let b = MapPoint::new(52., 50.);
    for (source, goal) in [(a, b), (b, a)] {
        let (engine, assets) = compiled_walkway_with_dimensions(&bytes, (100., 100.));
        let (receiver, height) = tick_walkway_crossing(engine, assets, 0, 0, source, goal);
        assert_eq!(receiver, if goal == b { Some(0) } else { None });
        assert_eq!(height, if goal == b { 16. } else { 0. });
    }
}

#[test]
fn actor_ticks_follow_compiled_routes_around_wall_ends() {
    let a = MapPoint::new(250., 150.);
    let b = MapPoint::new(250., 250.);
    for (source, goal) in [(a, b), (b, a)] {
        let samples = tick_compiled_route(
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/asset-spline-wall.level.json"
            )),
            0,
            0,
            source,
            goal,
        );
        assert!(
            samples.iter().any(|point| point.x < 101. || point.x > 399.),
            "actor never passed the wall end"
        );
    }
}

#[test]
fn actor_ticks_follow_curved_and_rising_compiled_walkways() {
    for (bytes, layer, sector, a, b) in [
        (
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/asset-spline-curved-walkway.level.json"
            ))
            .as_slice(),
            1,
            2,
            MapPoint::new(125.32, 158.32),
            MapPoint::new(394.32, 250.37),
        ),
        (
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/asset-spline-rising-walkway.level.json"
            ))
            .as_slice(),
            0,
            0,
            MapPoint::new(125.41611, 158.50731),
            MapPoint::new(394.36096, 232.79567),
        ),
    ] {
        for (source, goal) in [(a, b), (b, a)] {
            tick_compiled_route(bytes, layer, sector, source, goal);
        }
    }
}

fn tick_compiled_route(
    bytes: &[u8],
    layer: u16,
    sector: u16,
    source: MapPoint,
    goal: MapPoint,
) -> Vec<MapPoint> {
    tick_compiled_route_in(compiled_walkway(bytes), layer, sector, source, goal)
}

#[test]
#[ignore = "requires ROBIN_WYCHFORD_DESCRIPTOR pointing to a current editor export"]
fn exported_wychford_narrow_contour_actor_route() {
    let bytes = std::fs::read(std::env::var("ROBIN_WYCHFORD_DESCRIPTOR").unwrap()).unwrap();
    let document: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let corner = &document["walkable_polygon"][2];
    let dimensions = (
        corner[0].as_f64().unwrap() as f32 + 1.,
        corner[1].as_f64().unwrap() as f32 + 1.,
    );
    for (source, goal) in [
        (MapPoint::new(2520., 251.), MapPoint::new(2440., 331.)),
        (MapPoint::new(2440., 331.), MapPoint::new(2520., 251.)),
    ] {
        tick_compiled_route_in(
            compiled_walkway_with_dimensions(&bytes, dimensions),
            15,
            291,
            source,
            goal,
        );
    }
}

fn tick_compiled_route_in(
    (mut engine, mut assets): (EngineInner, LevelAssets),
    layer: u16,
    sector: u16,
    source: MapPoint,
    goal: MapPoint,
) -> Vec<MapPoint> {
    let grid = &engine.world.fast_grid;
    let half = grid.try_move_box_half_diagonal(0).unwrap();
    assert!(!grid.is_reachable_thick(source, goal, layer, half));
    let sector_index =
        grid.level.sector_number_map[&crate::sector::SectorNumber::new(sector as i16)];
    let handle = crate::position_interface::SectorHandle::new(sector)
        .unwrap()
        .with_arena_index(crate::fast_find_grid::SectorIndex::new(sector_index as u32).unwrap());
    let receiver = engine
        .get_projection_area_index(&assets, handle, layer, source)
        .expect("fixture source must have authored receiving geometry");
    let owner = walking_pc(&mut engine, &mut assets, source, layer, handle);
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
    assert!(
        matches!(outcome, MovePathOutcome::Pending),
        "dispatch {outcome:?}; actor position {:?}, box {:?}",
        engine.ent(owner).element_data().position_map(),
        engine.ent(owner).position_iface().get_move_box_map()
    );
    // Exercise the real queued request, path handoff and order postprocessing.
    for _ in 0..2 {
        engine.hourglass_phase_paths(TickCtx::new(&sim, &assets));
    }
    assert!(engine.orders.pending_path_requests.waiting.is_empty());
    assert!(engine.orders.pending_path_requests.in_flight.is_none());
    assert!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .orders
            .len()
            > 1
    );
    engine.select_sequence_element(owner, Some((sequence, 0)));
    let mut previous = source;
    let mut samples = vec![];
    let mut worst_height = (0., source, 0., 0., None, None);
    for _ in 0..2000 {
        engine.t_tick_actor_owner_envelopes(&assets);
        let element = engine.ent(owner).element_data();
        let position = element.position_map();
        assert!(
            engine
                .world
                .fast_grid
                .is_reachable_thick(previous, position, layer, half),
            "actor crossed collision: {previous:?} -> {position:?}"
        );
        assert_eq!(element.layer(), layer);
        let receiver = engine
            .get_projection_area_index(&assets, handle, layer, position)
            .expect("actor left authored receiving geometry");
        let height = assets.environment.static_sight_obstacles[usize::from(receiver)]
            .compute_top_z_from_projection(position.x, position.y);
        let delta = (element.position().z - height).abs();
        if delta > worst_height.0 {
            worst_height = (
                delta,
                position,
                element.position().z,
                height,
                engine.ent(owner).position_iface().get_obstacle(),
                Some(receiver),
            );
        }
        samples.push(position);
        previous = position;
        if (position - goal).length() < 0.01 {
            break;
        }
    }
    assert!(worst_height.0 < 0.001, "height mismatch: {worst_height:?}");
    assert!(
        (previous - goal).length() < 0.01,
        "actor stopped at {previous:?}, expected {goal:?}"
    );
    samples
}

fn walking_pc(
    engine: &mut EngineInner,
    assets: &mut LevelAssets,
    source: MapPoint,
    layer: u16,
    sector: crate::position_interface::SectorHandle,
) -> crate::element::EntityId {
    let action = OrderType::WalkingUpright;
    let script = crate::sprite_script::SpriteScript {
        action_id: action as u16,
        action_done: 2,
        average_speed: 1.,
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
    conversion[OrderType::WalkingStairs as usize] = 16;
    let mut scripts = vec![script.clone(); 16];
    let mut stairs = script;
    stairs.action_id = OrderType::WalkingStairs as u16;
    scripts.extend(vec![stairs; 16]);
    let mut pc = crate::engine::test_support::actors::unbound_pc(Posture::Upright);
    pc.element.sprite = crate::sprite::Sprite::new(Arc::new(scripts), Arc::new(conversion));
    pc.element.active = true;
    pc.element.set_sector(Some(sector));
    pc.element.set_layer(layer);
    pc.element.sprite.position_iface.configure_for_actor(
        crate::position_interface::PathfinderIndex::new(0).unwrap(),
        crate::coordinates::MoveBoxHalfDiagonal::new(6., 3.),
        source,
    );
    pc.actor.action_state = ActionState::Moving;
    let owner = engine.add_test_entity(Entity::Pc(pc));
    crate::engine::complete_test_runtime_fixture(engine, assets);
    owner
}

fn tick_walkway_crossing(
    engine: EngineInner,
    assets: LevelAssets,
    layer: u16,
    sector_index: usize,
    source: MapPoint,
    goal: MapPoint,
) -> (Option<u32>, f32) {
    tick_walkway_movement(engine, assets, layer, sector_index, source, goal, true)
}

fn tick_walkway_movement(
    mut engine: EngineInner,
    mut assets: LevelAssets,
    layer: u16,
    sector_index: usize,
    source: MapPoint,
    goal: MapPoint,
    change_receiver: bool,
) -> (Option<u32>, f32) {
    let sector = &engine.world.fast_grid.level.sectors[sector_index];
    let handle = crate::position_interface::SectorHandle::new(u16::from(sector.sector_number))
        .unwrap()
        .with_arena_index(crate::fast_find_grid::SectorIndex::new(sector_index as u32).unwrap());
    let start_receiver = engine.get_projection_area_index(&assets, handle, layer, source);
    let end_receiver = engine.get_projection_area_index(&assets, handle, layer, goal);
    assert_eq!(start_receiver != end_receiver, change_receiver);
    let action = OrderType::WalkingUpright;
    let owner = walking_pc(&mut engine, &mut assets, source, layer, handle);
    engine.set_obstacle_and_material(&assets, owner, start_receiver);
    let mut movement = SequenceElement::new_movement(1, Command::MoveOk, Some(owner), action);
    let order_id = engine.orders.allocate_order_id();
    movement
        .orders
        .push_back(crate::order::Order::new(action, goal.x, goal.y, order_id));
    let sequence = engine.t_launch_in_progress(&assets, movement);
    engine.select_sequence_element(owner, Some((sequence, 0)));
    let mut samples = Vec::new();
    // The fixture walks about one map unit per tick. Keep a finite allowance
    // for startup and slower movement when auditing an entire placed bridge.
    let tick_limit = 200.max(((goal - source).length() * 2.).ceil() as usize + 20);
    for _ in 0..tick_limit {
        engine.t_tick_actor_owner_envelopes(&assets);
        let position = engine.ent(owner).element_data().position_map();
        assert_actor_receiver(&engine, &assets, owner, handle, layer, position);
        samples.push(position);
        if (position - goal).length() < 0.01 {
            break;
        }
    }
    assert!(
        samples.iter().any(|p| *p != source),
        "actor never moved: {samples:?}"
    );
    let entity = engine.ent(owner);
    assert!(
        (entity.element_data().position_map() - goal).length() < 0.01,
        "actor did not arrive: {samples:?}"
    );
    assert_eq!(entity.position_iface().get_obstacle(), end_receiver);
    let arrived = entity.element_data().position_map();
    let height = end_receiver
        .map(|receiver| {
            assets.environment.static_sight_obstacles[usize::from(receiver)]
                .compute_top_z_from_projection(arrived.x, arrived.y)
        })
        .unwrap_or(0.);
    assert!((entity.element_data().position().z - height).abs() < 0.001);
    assert!(
        samples.len() > 2,
        "movement must advance over multiple actor ticks"
    );
    (end_receiver.map(u32::from), height)
}

fn point_on_edge(a: MapPoint, b: MapPoint, point: MapPoint) -> bool {
    let (ax, ay, bx, by, x, y) = (
        f64::from(a.x),
        f64::from(a.y),
        f64::from(b.x),
        f64::from(b.y),
        f64::from(point.x),
        f64::from(point.y),
    );
    (x - ax) * (by - ay) == (y - ay) * (bx - ax)
        && (ax.min(bx)..=ax.max(bx)).contains(&x)
        && (ay.min(by)..=ay.max(by)).contains(&y)
}

fn receiver_edge_contains(
    assets: &LevelAssets,
    receiver: crate::sight_obstacle::SightObstacleIndex,
    position: MapPoint,
) -> bool {
    let points = &assets.environment.static_sight_obstacles[usize::from(receiver)].obstacle_points;
    (0..points.len()).any(|index| {
        let a = &points[index];
        let b = &points[(index + 1) % points.len()];
        point_on_edge(
            MapPoint::new(a.x, a.y - a.z_top),
            MapPoint::new(b.x, b.y - b.z_top),
            position,
        )
    })
}

fn assert_actor_receiver(
    engine: &EngineInner,
    assets: &LevelAssets,
    owner: crate::element::EntityId,
    sector: crate::position_interface::SectorHandle,
    layer: u16,
    position: MapPoint,
) {
    actor_receiver_result(engine, assets, owner, sector, layer, position).unwrap();
}

fn actor_receiver_result(
    engine: &EngineInner,
    assets: &LevelAssets,
    owner: crate::element::EntityId,
    sector: crate::position_interface::SectorHandle,
    layer: u16,
    position: MapPoint,
) -> Result<(), String> {
    let queried = engine.get_projection_area_index(assets, sector, layer, position);
    let current = engine.ent(owner).position_iface().get_obstacle();
    // Crossing direction can select either side at an exact shared boundary.
    // At a fan vertex the lookup may select a nonadjacent triangle: both
    // contours must contain the point and their heights must agree there.
    // Away from these boundaries, receiver identity must agree too.
    let shared_junction = current.zip(queried).is_some_and(|(a, b)| {
        a != b
            && receiver_edge_contains(assets, a, position)
            && receiver_edge_contains(assets, b, position)
            && (assets.environment.static_sight_obstacles[usize::from(a)]
                .compute_top_z_from_projection(position.x, position.y)
                - assets.environment.static_sight_obstacles[usize::from(b)]
                    .compute_top_z_from_projection(position.x, position.y))
            .abs()
                < 0.001
    });
    let shared_boundary = current != queried
        && engine.world.fast_grid.level.lines.iter().any(|line| {
            line.is_elevation
                && ((line.left_obstacle_index == current && line.right_obstacle_index == queried)
                    || (line.right_obstacle_index == current
                        && line.left_obstacle_index == queried))
                && point_on_edge(line.a, line.b, position)
        });
    if current != queried && !shared_boundary && !shared_junction {
        return Err(format!(
            "actor receiver mismatch at {position:?}, layer {layer}, sector {sector:?}: current {current:?}, queried {queried:?}"
        ));
    }
    let expected_height = current
        .map(|receiver| {
            assets.environment.static_sight_obstacles[usize::from(receiver)]
                .compute_top_z_from_projection(position.x, position.y)
        })
        .unwrap_or(0.);
    let actual_height = engine.ent(owner).element_data().position().z;
    if (actual_height - expected_height).abs() >= 0.001 {
        return Err(format!(
            "actor height mismatch at {position:?}: {actual_height} != {expected_height}"
        ));
    }
    Ok(())
}

#[test]
fn receiving_audit_accepts_equal_height_fan_vertices_but_rejects_stale_interior_receivers() {
    let mut descriptor: serde_json::Value = serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/asset-navigation-copies.level.json"
    )))
    .unwrap();
    let corners = [(0., 0.), (100., 0.), (100., 100.), (0., 100.)];
    descriptor["asset_geometry"] = serde_json::json!({
        "motion_data": {"layers": [[{
            "is_lift": false, "state_id": 0, "flags": 0,
            "polygon": {"points": [[0,0],[100,0],[100,100],[0,100]]},
            "skeleton_segments": [], "obstacles": []
        }], []], "graph_bytes": []},
        "sight_obstacles": (0..4).map(|i| serde_json::json!({
            "points": ([(50.,50.), corners[i], corners[(i+1)%4]].map(|(x,y)| {
                let z = 10. + y * 0.4;
                serde_json::json!({"x":x,"y":y+z,"z_bottom":z,"z_top":z})
            })),
            "projection_area": [0,0], "solid":false, "opaque":false,
            "mouse":true,"show_shadow_polygon":false,"default_material":0,"material_indices":[]
        })).collect::<Vec<_>>(), "doors": []
    });
    let (mut engine, mut assets) = compiled_walkway(&serde_json::to_vec(&descriptor).unwrap());
    let sector = crate::position_interface::SectorHandle::new(0)
        .unwrap()
        .with_arena_index(crate::fast_find_grid::SectorIndex::new(0).unwrap());
    let center = MapPoint::new(50., 50.);
    let queried = engine
        .get_projection_area_index(&assets, sector, 0, center)
        .unwrap();
    let opposite =
        crate::sight_obstacle::SightObstacleIndex::new((u32::from(queried) + 2) % 4).unwrap();
    let owner = walking_pc(&mut engine, &mut assets, center, 0, sector);
    engine.set_obstacle_and_material(&assets, owner, Some(opposite));
    assert_eq!(
        actor_receiver_result(&engine, &assets, owner, sector, 0, center),
        Ok(())
    );
    // The same two coplanar receivers are not interchangeable inside a face.
    let interior = MapPoint::new(50., 20.);
    let queried = engine
        .get_projection_area_index(&assets, sector, 0, interior)
        .unwrap();
    let wrong =
        crate::sight_obstacle::SightObstacleIndex::new((u32::from(queried) + 2) % 4).unwrap();
    let owner = walking_pc(&mut engine, &mut assets, interior, 0, sector);
    engine.set_obstacle_and_material(&assets, owner, Some(wrong));
    assert!(actor_receiver_result(&engine, &assets, owner, sector, 0, interior).is_err());
    for (a, b) in [
        (MapPoint::new(50., 26.), MapPoint::new(50., 74.)),
        (MapPoint::new(26., 50.), MapPoint::new(74., 50.)),
    ] {
        for (source, goal) in [(a, b), (b, a)] {
            let (engine, assets) = compiled_walkway(&serde_json::to_vec(&descriptor).unwrap());
            tick_walkway_crossing(engine, assets, 0, 0, source, goal);
        }
    }
    // A coincident projected vertex on a different floor is not equivalent.
    for point in descriptor["asset_geometry"]["sight_obstacles"][2]["points"]
        .as_array_mut()
        .unwrap()
    {
        for key in ["y", "z_bottom", "z_top"] {
            point[key] = serde_json::json!(point[key].as_f64().unwrap() + 10.);
        }
    }
    let (mut engine, mut assets) = compiled_walkway(&serde_json::to_vec(&descriptor).unwrap());
    let queried = engine
        .get_projection_area_index(&assets, sector, 0, center)
        .unwrap();
    assert_eq!(u32::from(queried), 2);
    let owner = walking_pc(&mut engine, &mut assets, center, 0, sector);
    engine.set_obstacle_and_material(
        &assets,
        owner,
        Some(crate::sight_obstacle::SightObstacleIndex::new(0).unwrap()),
    );
    assert!(actor_receiver_result(&engine, &assets, owner, sector, 0, center).is_err());
}

#[test]
fn actor_crosses_receivers_independently_of_switch_visibility() {
    for replacement in [false, true] {
        let mut descriptor: serde_json::Value = serde_json::from_slice(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/asset-navigation-copies.level.json"
        )))
        .unwrap();
        let geometry = &mut descriptor["asset_geometry"];
        let (initial, applied, expected_receiver) = if replacement {
            let mut higher = geometry["sight_obstacles"][0].clone();
            for point in higher["points"].as_array_mut().unwrap() {
                for axis in ["y", "z_top", "z_bottom"] {
                    point[axis] = serde_json::json!(point[axis].as_f64().unwrap() + 10.);
                }
            }
            for point in higher["projection_plane"].as_array_mut().unwrap() {
                for axis in [1, 2] {
                    point[axis] = serde_json::json!(point[axis].as_f64().unwrap() + 10.);
                }
            }
            geometry["sight_obstacles"]
                .as_array_mut()
                .unwrap()
                .push(higher);
            (vec![0], vec![4], 4)
        } else {
            (vec![], vec![0], 0)
        };
        geometry["movement_transitions"] = serde_json::json!([{
            "id":"receiver-visibility", "waypoint":[396,320], "sector":0, "layer":0,
            "active":true, "definitive":false, "apply_polygon":{"points":[]},
            "no_apply_polygon":{"points":[]}, "motion_changes":[],
            "initial_sight":initial, "applied_sight":applied
        }]);
        let bytes = serde_json::to_vec(&descriptor).unwrap();
        let mut heights = Vec::new();
        for state in 0..3 {
            let (mut engine, assets) = compiled_walkway(&bytes);
            let sim = crate::sim_rng::test_context();
            let patch = crate::patch::PatchIndex::new(0).unwrap();
            if state >= 1 {
                engine.apply_patch(TickCtx::new(&sim, &assets), patch);
            }
            if state == 2 {
                engine.reset_patch(TickCtx::new(&sim, &assets), patch);
            }
            assert_eq!(
                engine.world.static_sight_obstacle_active[expected_receiver],
                state == 1
            );
            let (receiver, height) = tick_walkway_crossing(
                engine,
                assets,
                0,
                0,
                MapPoint::new(396., 320.),
                MapPoint::new(404., 290.),
            );
            assert_eq!(receiver, Some(expected_receiver as u32));
            heights.push(height);
        }
        assert!(
            heights
                .iter()
                .all(|height| (*height - heights[0]).abs() < 0.001)
        );
        assert!((heights[0] - if replacement { 74. } else { 64. }).abs() < 0.001);
    }
}
