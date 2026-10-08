use super::*;

#[test]
fn prepared_passage_state_preserves_script_activation_and_validates_bindings() {
    let fixtures: Vec<serde_json::Value> = serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/asset-climb-entrance-barriers.levels.json"
    )))
    .unwrap();
    let (mut engine, assets) = compiled_walkway(&serde_json::to_vec(&fixtures[0]).unwrap());
    assert_eq!(assets.navigation.passage_states.len(), 1);
    let sim = crate::sim_rng::test_context();
    let patch = crate::patch::PatchIndex::new(0).unwrap();
    assert!(!engine.script_domains.interactables.doors[0].passage_blocked);
    engine.script_domains.interactables.doors[0].active = false;
    engine.apply_patch(TickCtx::new(&sim, &assets), patch);
    assert!(engine.script_domains.interactables.doors[0].passage_blocked);
    assert!(!engine.script_domains.interactables.doors[1].passage_blocked);
    engine.reset_patch(TickCtx::new(&sim, &assets), patch);
    assert!(!engine.script_domains.interactables.doors[0].passage_blocked);
    assert!(!engine.script_domains.interactables.doors[0].active);
    for (field, value) in [
        ("layer", serde_json::json!(99)),
        ("area", serde_json::json!(99)),
        ("allowed_states", serde_json::json!([3])),
        ("allowed_states", serde_json::json!([4])),
    ] {
        let mut invalid = fixtures[0].clone();
        invalid["asset_geometry"]["lifts"][0]["doors"][0]["passage_states"][0][field] = value;
        let error = crate::level_data::LoadedLevel::hackable_from_json(
            &serde_json::to_vec(&invalid).unwrap(),
        )
        .err()
        .expect("invalid prepared passage must fail to load");
        assert!(error.to_string().contains("passage"), "{error}");
    }
}

pub(super) fn walk_exported_building_round_trip(
    engine: EngineInner,
    assets: LevelAssets,
    door: usize,
    sprite: &crate::sprite::Sprite,
) -> Result<bool, String> {
    let entrance = &engine.script_domains.interactables.doors[door];
    assert_eq!(entrance.door_type, crate::gate::DoorType::Building);
    let inside = entrance.sector_in_index.expect("building interior sector");
    assert!(
        engine.world.fast_grid.level.sectors[usize::from(inside)]
            .sector_type
            .is_building()
    );
    walk_exported_lift_with_tick(engine, assets, door, door, Some(sprite), |_, _, _| {})
}

#[test]
fn compiled_stair_barriers_stop_actor_traversal_and_reset() {
    let fixtures: Vec<serde_json::Value> = serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/asset-changing-lifts.levels.json"
    )))
    .unwrap();
    for (rotation, fixture) in fixtures.iter().enumerate() {
        let (mut engine, assets) = compiled_walkway(&serde_json::to_vec(fixture).unwrap());
        let sim = crate::sim_rng::test_context();
        let patch = crate::patch::PatchIndex::new(0).unwrap();
        for (step, applied) in [false, true, false].into_iter().enumerate() {
            if step > 0 {
                if applied {
                    engine.apply_patch(TickCtx::new(&sim, &assets), patch);
                } else {
                    engine.reset_patch(TickCtx::new(&sim, &assets), patch);
                }
            }
            for (entrance, exit) in [(0, 1), (1, 0)] {
                let result = walk_exported_stairs(engine.clone(), assets.clone(), entrance, exit);
                if applied {
                    if engine.script_domains.interactables.doors[entrance].passage_blocked
                        || engine.script_domains.interactables.doors[exit].passage_blocked
                    {
                        assert_eq!(
                            result,
                            Ok(false),
                            "blocked prepared passage at rotation {rotation}"
                        );
                    } else {
                        assert!(
                            result
                                .as_ref()
                                .is_err_and(|error| error.starts_with("lift route stalled")),
                            "closed stair, rotation {rotation}, entrance {entrance}: {result:?}"
                        );
                    }
                } else {
                    assert_eq!(
                        result,
                        Ok(true),
                        "open stair, rotation {rotation}, entrance {entrance}"
                    );
                }
            }
        }
    }
}

#[test]
fn compiled_stair_barriers_remain_independent_after_copying() {
    let bytes = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/asset-changing-lifts-copied.level.json"
    ));
    audit_copied_lift_controls(bytes, None);
}

#[test]
#[ignore = "requires ROBIN_CLIMB_RHS"]
fn compiled_climb_barriers_remain_independent_after_copying() {
    let sprite = complete_climb_sprite();
    let fixtures: Vec<serde_json::Value> = serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/asset-changing-climbs-copied.levels.json"
    )))
    .unwrap();
    for fixture in fixtures {
        audit_copied_lift_controls(&serde_json::to_vec(&fixture).unwrap(), Some(&sprite));
    }
}

fn audit_copied_lift_controls(bytes: &[u8], sprite: Option<&crate::sprite::Sprite>) {
    let document: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    let geometry: crate::level_data::CompiledAssetGeometry =
        serde_json::from_value(document["asset_geometry"].clone()).unwrap();
    assert_eq!(geometry.lifts.len(), 2);
    assert_eq!(geometry.movement_transitions.len(), 2);
    let (mut engine, assets) = compiled_walkway(bytes);
    let sim = crate::sim_rng::test_context();
    let mut closed = [false; 2];
    for (changed, applied) in [(0, true), (1, true), (0, false), (1, false)] {
        let sector = geometry.lifts[changed].motion_area_index;
        let patch_index = geometry
            .movement_transitions
            .iter()
            .position(|transition| transition.motion_changes.iter().any(|c| c.sector == sector))
            .expect("copied stair must have its own control");
        let patch = crate::patch::PatchIndex::new(patch_index as u32).unwrap();
        if applied {
            engine.apply_patch(TickCtx::new(&sim, &assets), patch);
        } else {
            engine.reset_patch(TickCtx::new(&sim, &assets), patch);
        }
        closed[changed] = applied;
        for (index, &blocked) in closed.iter().enumerate() {
            for (entrance, exit) in [(index * 2, index * 2 + 1), (index * 2 + 1, index * 2)] {
                let result =
                    walk_exported_lift(engine.clone(), assets.clone(), entrance, exit, sprite);
                if blocked {
                    if engine.script_domains.interactables.doors[entrance].passage_blocked
                        || engine.script_domains.interactables.doors[exit].passage_blocked
                    {
                        assert_eq!(
                            result,
                            Ok(false),
                            "blocked prepared passage of copy {index}"
                        );
                    } else {
                        assert!(
                            result
                                .as_ref()
                                .is_err_and(|error| error.starts_with("lift route stalled")),
                            "copied stair {index}, changed {changed}, applied {applied}: {result:?}"
                        );
                    }
                } else {
                    assert_eq!(
                        result,
                        Ok(true),
                        "copied stair {index}, changed {changed}, applied {applied}"
                    );
                }
            }
        }
    }
}

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
    if let Some(anchors) = &mut receiver.projection_plane {
        for point in anchors {
            point[0] += 5.;
        }
    }
    receiver.projection_area = Some((4, 3));
    geometry.sight_obstacles.push(receiver);
    let mut lift = geometry.lifts[0].clone();
    lift.motion_area_index = 4;
    // This fixture offsets only projected geometry; retain its legacy layer test.
    lift.physical_navigation = None;
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
        if let Some(anchors) = &mut geometry.sight_obstacles[2].projection_plane {
            for point in anchors {
                point[0] += if point[0] < 400. {
                    f32::from(gap) + 0.25
                } else {
                    -f32::from(gap) - 0.25
                };
            }
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

#[test]
fn arbitrarily_rotated_stairs_support_complete_actor_routes() {
    let bytes = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/asset-lift.level.json"
    ));
    for degrees in (0..360).map(|angle| angle as f32).chain([22.5]) {
        let placed = super::compiled_lifts::angled_lift_fixture(bytes, degrees, 1);
        let (engine, assets) = compiled_walkway(&placed);
        for (entrance, exit) in [(0, 1), (1, 0)] {
            assert_eq!(
                walk_exported_stairs(engine.clone(), assets.clone(), entrance, exit),
                Ok(true),
                "degrees={degrees}, entrance={entrance}"
            );
        }
    }
}

fn walk_exported_stairs(
    engine: EngineInner,
    assets: LevelAssets,
    entrance: usize,
    exit: usize,
) -> Result<bool, String> {
    walk_exported_lift(engine, assets, entrance, exit, None)
}

fn walk_exported_lift(
    engine: EngineInner,
    assets: LevelAssets,
    entrance: usize,
    exit: usize,
    sprite: Option<&crate::sprite::Sprite>,
) -> Result<bool, String> {
    walk_exported_lift_with_tick(engine, assets, entrance, exit, sprite, |_, _, _| {})
}

fn walk_exported_lift_with_tick(
    mut engine: EngineInner,
    mut assets: LevelAssets,
    entrance: usize,
    exit: usize,
    sprite: Option<&crate::sprite::Sprite>,
    mut tick: impl FnMut(&mut EngineInner, &LevelAssets, crate::element::EntityId),
) -> Result<bool, String> {
    let doors = &engine.script_domains.interactables.doors;
    let enter = doors[entrance].clone();
    let leave = doors[exit].clone();
    let source_sector = crate::position_interface::SectorHandle::from_number(enter.sector_out)
        .with_arena_index(enter.sector_out_index.unwrap());
    let destination_sector = crate::position_interface::SectorHandle::from_number(leave.sector_out)
        .with_arena_index(leave.sector_out_index.unwrap());
    let lift_sector = enter.sector_in_index.unwrap();
    let virtual_room = engine.world.fast_grid.level.sectors[usize::from(lift_sector)]
        .sector_type
        .is_building();
    let climbing = matches!(
        engine.world.fast_grid.level.sectors[usize::from(lift_sector)].lift_type,
        Some(crate::sector::LiftType::Ladder | crate::sector::LiftType::Wall)
    );
    assert_eq!(Some(lift_sector), leave.sector_in_index);
    let owner = walking_pc(
        &mut engine,
        &mut assets,
        enter.point_out,
        enter.layer_out,
        source_sector,
    );
    if let Some(sprite) = sprite {
        engine.ent_mut(owner).pc_data_mut().unwrap().has_climb = true;
        let element = engine.ent_mut(owner).element_data_mut();
        let position = element.sprite.position_iface.clone();
        element.sprite = sprite.clone();
        element.sprite.position_iface = position;
    }
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
    let route = engine
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
    let mut previous_receiver = receiver;
    let trace = std::env::var_os("ROBIN_LIFT_TRACE").is_some();
    for _ in 0..(distance.ceil() as usize * 4 + 1000) {
        engine.control.frame_counter += 1;
        engine.t_hourglass_phase_sequences(&assets);
        engine.hourglass_phase_paths(TickCtx::new(&sim, &assets));
        engine.t_tick_actor_owner_envelopes(&assets);
        tick(&mut engine, &assets, owner);
        let element = engine.ent(owner).element_data();
        let position = element.position_map();
        let sector = element.sector().ok_or("stair actor lost its sector")?;
        let receiver = engine.ent(owner).position_iface().get_obstacle();
        if trace && receiver != previous_receiver {
            eprintln!(
                "route {entrance}->{exit}: {position:?} layer {} receiver {previous_receiver:?}->{receiver:?}",
                element.layer()
            );
        }
        previous_receiver = receiver;
        crossed |= sector.arena_index() == Some(lift_sector);
        // A passage straddles two receiving polygons while the sector changes
        // at its midpoint. Require ordinary lookup agreement once its explicit
        // approach movement has finished, including the destination.
        let passing = crate::engine::ai::selected_actor_is_passing_door(
            &engine.entities(),
            &engine.seq(),
            owner,
        );
        // Climbing preserves its approach receiver while animation motion
        // changes altitude; ordinary receiving lookup applies after landing.
        // Virtual rooms have no receiving plane; check it again after exit.
        if !passing && !((climbing || virtual_room) && sector.arena_index() == Some(lift_sector)) {
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
            let route_states = engine.seq().get_sequence(route).map(|sequence| {
                sequence
                    .elements
                    .iter()
                    .map(|element| (element.command, element.state))
                    .collect::<Vec<_>>()
            });
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
                "lift route stalled at {position:?}, layer {}, sector {sector:?}, goal {:?}, crossed={crossed}, bounds={bounds:?}, blockers={blockers:?}, selected={selected:?}, route={route_states:?}",
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
    audit_exported_lifts(
        &[crate::sector::LiftType::Stairs],
        None,
        "actor-stair-route-report.json",
    );
}

#[test]
#[ignore = "requires ROBIN_ASSET_MAP_DIAGNOSTICS and ROBIN_CLIMB_RHS"]
fn exported_stairs_support_complete_sprite_actor_routes() {
    let sprite = complete_climb_sprite();
    audit_exported_lifts(
        &[crate::sector::LiftType::Stairs],
        Some(&sprite),
        "actor-stair-sprite-route-report.json",
    );
}

#[test]
#[ignore = "requires ROBIN_ASSET_MAP_DIAGNOSTICS and ROBIN_CLIMB_RHS"]
fn exported_climbs_support_complete_actor_routes() {
    let sprite = complete_climb_sprite();
    audit_exported_lifts(
        &[
            crate::sector::LiftType::Ladder,
            crate::sector::LiftType::Wall,
        ],
        Some(&sprite),
        "actor-climb-route-report.json",
    );
}

#[test]
#[ignore = "requires ROBIN_CLIMB_RHS"]
fn placed_climbs_support_complete_actor_routes() {
    let sprite = complete_climb_sprite();
    let bytes = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/asset-lift.level.json"
    ));
    for (lift_type, high_type) in [(2, 4), (3, 4), (3, 6)] {
        for degrees in (0..360).map(|angle| angle as f32).chain([22.5]) {
            let mut document: serde_json::Value = serde_json::from_slice(
                &super::compiled_lifts::angled_lift_fixture(bytes, degrees, lift_type),
            )
            .unwrap();
            document["asset_geometry"]["lifts"][0]["doors"][1]["door_type"] = high_type.into();
            let (engine, assets) = compiled_walkway(&serde_json::to_vec(&document).unwrap());
            for (entrance, exit) in [(0, 1), (1, 0)] {
                let result = walk_exported_lift(
                    engine.clone(),
                    assets.clone(),
                    entrance,
                    exit,
                    Some(&sprite),
                );
                assert_eq!(
                    result,
                    Ok(true),
                    "lift={lift_type}, high={high_type}, degrees={degrees}, entrance={entrance}"
                );
            }
        }
    }
}

pub(super) fn complete_climb_sprite() -> crate::sprite::Sprite {
    if std::env::var_os("ROBIN_LIFT_TRACE").is_some() {
        use tracing_subscriber::prelude::*;
        let _ = tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer().with_test_writer())
            .with(
                tracing_subscriber::filter::Targets::new()
                    .with_default(tracing::Level::DEBUG)
                    .with_target("robin_engine::elevation_crossing", tracing::Level::TRACE)
                    .with_target(
                        "robin_engine::engine::movement::elevation",
                        tracing::Level::TRACE,
                    ),
            )
            .try_init();
    }
    use crate::sprite_script::{FrameKind, MissionResourceEnvironment, SpriteScriptor};
    use robin_util::asset_fs::{AssetVfs, Bundle};
    let path = std::env::var("ROBIN_CLIMB_RHS").expect("path to a complete RobinTown RHS sprite");
    let bytes = std::fs::read(path).unwrap();
    let signature = u32::from_le_bytes(bytes[..4].try_into().unwrap());
    let vfs = Arc::new(AssetVfs::new());
    vfs.mount_bundle(Arc::new(Bundle::from([(
        "characters/audit.rhs".into(),
        bytes.into(),
    )])))
    .unwrap();
    let files = crate::sbfile::SbFileSystem::new(vfs);
    let mut scriptor =
        SpriteScriptor::with_resources(Arc::new(MissionResourceEnvironment::from_files(&files)));
    let mut sprite = crate::sprite::Sprite::default();
    sprite
        .load_frame_info(
            &mut scriptor,
            FrameKind::Character,
            "Data/Characters",
            "audit",
            "Robin des bois",
            signature,
            None,
        )
        .unwrap();
    sprite
}

#[test]
fn changing_climbs_update_collision_and_routes_after_reset() {
    audit_changing_climbs(None);
}

#[test]
#[ignore = "requires ROBIN_CLIMB_RHS"]
fn changing_climbs_stop_actor_traversal_and_reset() {
    let sprite = complete_climb_sprite();
    audit_changing_climbs(Some(&sprite));
}

#[test]
#[ignore = "requires ROBIN_CLIMB_RHS"]
fn changing_climb_barriers_stop_an_actor_already_climbing() {
    let sprite = complete_climb_sprite();
    let fixtures: Vec<serde_json::Value> = serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/asset-changing-climbs.levels.json"
    )))
    .unwrap();
    for (index, fixture) in fixtures.iter().enumerate() {
        let (engine, assets) = compiled_walkway(&serde_json::to_vec(fixture).unwrap());
        for (entrance, exit) in [(0, 1), (1, 0)] {
            let mut applied = false;
            let sim = crate::sim_rng::test_context();
            let goal = engine.script_domains.interactables.doors[exit].point_in;
            let lift_sector = engine.script_domains.interactables.doors[entrance].sector_in_index;
            let result = walk_exported_lift_with_tick(
                engine.clone(),
                assets.clone(),
                entrance,
                exit,
                Some(&sprite),
                |engine, assets, owner| {
                    let element = engine.ent(owner).element_data();
                    if applied
                        || !matches!(element.posture(), Posture::OnLadder | Posture::OnWall)
                        || element.sector().and_then(|sector| sector.arena_index()) != lift_sector
                    {
                        return;
                    }
                    let position = element.position_map();
                    let layer = element.layer();
                    engine.apply_patch(
                        TickCtx::new(&sim, assets),
                        crate::patch::PatchIndex::new(0).unwrap(),
                    );
                    assert!(
                        !engine
                            .world
                            .fast_grid
                            .is_reachable_thin(position, goal, layer),
                        "barrier must appear ahead of the climbing actor, fixture={index}, entrance={entrance}"
                    );
                    applied = true;
                },
            );
            assert!(
                applied,
                "actor never began climbing, fixture={index}, entrance={entrance}: {result:?}"
            );
            assert!(
                result
                    .as_ref()
                    .is_err_and(|error| error.starts_with("lift route stalled")),
                "mid-climb barrier fixture={index}, entrance={entrance}: {result:?}"
            );
        }
    }
}

#[test]
#[ignore = "requires ROBIN_CLIMB_RHS"]
fn changing_climb_barriers_reopen_before_or_after_path_failure() {
    let sprite = complete_climb_sprite();
    let fixtures: Vec<serde_json::Value> = serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/asset-changing-climbs.levels.json"
    )))
    .unwrap();
    let mut checked = 0;
    for hold_ticks in [0, 8, 120] {
        for (index, fixture) in fixtures.iter().enumerate() {
            let (engine, assets) = compiled_walkway(&serde_json::to_vec(fixture).unwrap());
            for (entrance, exit) in [(0, 1), (1, 0)] {
                let mut applied = false;
                let mut reopened = false;
                let mut stationary = 0;
                let mut previous = None;
                let mut route = None;
                let sim = crate::sim_rng::test_context();
                let patch = crate::patch::PatchIndex::new(0).unwrap();
                let goal = engine.script_domains.interactables.doors[exit].point_in;
                let lift_sector =
                    engine.script_domains.interactables.doors[entrance].sector_in_index;
                let result = walk_exported_lift_with_tick(
                    engine.clone(),
                    assets.clone(),
                    entrance,
                    exit,
                    Some(&sprite),
                    |engine, assets, owner| {
                        if reopened {
                            return;
                        }
                        let element = engine.ent(owner).element_data();
                        if !matches!(element.posture(), Posture::OnLadder | Posture::OnWall)
                            || element.sector().and_then(|sector| sector.arena_index())
                                != lift_sector
                        {
                            return;
                        }
                        let position = element.position_map();
                        let layer = element.layer();
                        if !applied {
                            let Some((sequence, element_index)) =
                                engine.entities().current_element_for_actor(owner)
                            else {
                                return;
                            };
                            if engine
                                .seq()
                                .get_element(sequence, element_index)
                                .unwrap()
                                .command
                                != crate::element::Command::MoveOk
                            {
                                return;
                            }
                            if crate::engine::ai::selected_actor_is_passing_door(
                                &engine.entities(),
                                &engine.seq(),
                                owner,
                            ) {
                                return;
                            }
                            assert!(
                                engine
                                    .world
                                    .fast_grid
                                    .is_reachable_thin(position, goal, layer),
                                "open climb before closure, fixture={index}, entrance={entrance}, hold={hold_ticks}"
                            );
                            route = Some(sequence);
                            engine.apply_patch(TickCtx::new(&sim, assets), patch);
                            applied = true;
                        }
                        assert!(
                            !engine
                                .world
                                .fast_grid
                                .is_reachable_thin(position, goal, layer),
                            "closed barrier must remain ahead, fixture={index}, entrance={entrance}"
                        );
                        stationary = if previous == Some(position) {
                            stationary + 1
                        } else {
                            0
                        };
                        previous = Some(position);
                        if stationary == hold_ticks {
                            assert_eq!(
                                engine
                                    .orders
                                    .failed_path_requests
                                    .iter()
                                    .any(|request| request.owner == owner),
                                hold_ticks == 8,
                                "failed request before reopening, fixture={index}, entrance={entrance}, hold={hold_ticks}"
                            );
                            let aborted = engine
                                .seq()
                                .get_sequence(route.unwrap())
                                .unwrap()
                                .elements
                                .iter()
                                .any(|element| {
                                    element.state == crate::sequence::SequenceState::Impossible
                                });
                            assert_eq!(
                                aborted,
                                hold_ticks == 120,
                                "route state before reopening, fixture={index}, entrance={entrance}, hold={hold_ticks}"
                            );
                            // Reopening is a normal activation. A forced reset restores
                            // geometry without notifying actors to replan their routes.
                            engine.apply_patch(TickCtx::new(&sim, assets), patch);
                            assert!(!engine.script_domains.interactables.patches[0].applied);
                            assert!(
                                engine
                                    .world
                                    .fast_grid
                                    .is_reachable_thin(position, goal, layer),
                                "open climb after toggle, fixture={index}, entrance={entrance}, hold={hold_ticks}"
                            );
                            reopened = true;
                        }
                    },
                );
                assert!(
                    applied && reopened,
                    "actor must wait before reopening, fixture={index}, entrance={entrance}: {result:?}"
                );
                if hold_ticks == 0 {
                    assert_eq!(
                        result,
                        Ok(true),
                        "reopened before path failure, fixture={index}, entrance={entrance}"
                    );
                } else {
                    assert!(
                        result
                            .as_ref()
                            .is_err_and(|error| error.starts_with("lift route stalled")),
                        "a failed request must not redispatch when geometry opens, fixture={index}, entrance={entrance}, hold={hold_ticks}: {result:?}"
                    );
                }
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 72);
    eprintln!(
        "{checked} mid-climb reopening checks passed: early reopening completes; failed requests retain their timeout"
    );
}

#[test]
#[ignore = "requires ROBIN_CLIMB_RHS"]
fn changing_climb_barrier_near_entrance_blocks_actor_approach() {
    let sprite = complete_climb_sprite();
    let fixtures: Vec<serde_json::Value> = serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/asset-climb-entrance-barriers.levels.json"
    )))
    .unwrap();
    assert_eq!(fixtures.len(), 24);
    for (placement, fixture) in fixtures.iter().enumerate() {
        let lift_type = &fixture["asset_geometry"]["lifts"][0]["lift_type"];
        let (mut engine, assets) = compiled_walkway(&serde_json::to_vec(&fixture).unwrap());
        let sim = crate::sim_rng::test_context();
        let patch = crate::patch::PatchIndex::new(0).unwrap();
        for (step, applied) in [false, true, false].into_iter().enumerate() {
            if step > 0 {
                if applied {
                    engine.apply_patch(TickCtx::new(&sim, &assets), patch);
                } else {
                    engine.reset_patch(TickCtx::new(&sim, &assets), patch);
                }
            }
            for (entrance, exit) in [(0, 1), (1, 0)] {
                // Inner endpoints can lie on the same side after clearance
                // adjustment. Test the complete approach and exit animations.
                let result = walk_exported_lift(
                    engine.clone(),
                    assets.clone(),
                    entrance,
                    exit,
                    Some(&sprite),
                );
                if applied {
                    // Prepared connection conditions reject the route before
                    // an animation can carry the actor through the barrier.
                    assert_eq!(
                        result,
                        Ok(false),
                        "closed entrance placement={placement}, type={lift_type}, entrance={entrance}"
                    );
                } else {
                    assert_eq!(
                        result,
                        Ok(true),
                        "open entrance placement={placement}, type={lift_type}, entrance={entrance}"
                    );
                }
            }
        }
    }
}

#[test]
#[ignore = "requires ROBIN_CLIMB_RHS"]
fn changing_climb_entry_barrier_closes_during_animation() {
    let sprite = complete_climb_sprite();
    let fixtures: Vec<serde_json::Value> = serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/asset-climb-entrance-barriers.levels.json"
    )))
    .unwrap();
    for (placement, fixture) in fixtures.iter().enumerate() {
        let (engine, assets) = compiled_walkway(&serde_json::to_vec(fixture).unwrap());
        let entrance = usize::from(placement >= 12);
        for reopen_after in [None, Some(20), Some(120)] {
            let sim = crate::sim_rng::test_context();
            let mut applied = false;
            let mut reopened = false;
            let mut held_ticks = 0;
            let mut paused = None;
            let result = walk_exported_lift_with_tick(
                engine.clone(),
                assets.clone(),
                entrance,
                1 - entrance,
                Some(&sprite),
                |engine, assets, owner| {
                    let element = engine.ent(owner).element_data();
                    let cursor = (
                        element.position_map(),
                        element.sprite.current_row,
                        element.sprite.current_frame,
                        element.sprite.frame_count,
                    );
                    if reopened {
                        return;
                    }
                    if applied {
                        assert_eq!(
                            Some(cursor),
                            paused,
                            "blocked entry must retain position and animation cursor"
                        );
                        held_ticks += 1;
                        if reopen_after == Some(held_ticks) {
                            engine.apply_patch(
                                TickCtx::new(&sim, assets),
                                crate::patch::PatchIndex::new(0).unwrap(),
                            );
                            reopened = true;
                        }
                        return;
                    }
                    if !matches!(
                        element.sprite.last_action,
                        OrderType::TransitionWaitingUprightClimbingLadderUp
                            | OrderType::TransitionWaitingUprightClimbingWallUp
                            | OrderType::TransitionWaitingCrouchedClimbingLadderDown
                            | OrderType::TransitionWaitingCrouchedClimbingWallDown
                            | OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel
                    ) {
                        return;
                    }
                    assert!(crate::engine::ai::selected_actor_is_passing_door(
                        &engine.entities(),
                        &engine.seq(),
                        owner
                    ));
                    engine.apply_patch(
                        TickCtx::new(&sim, assets),
                        crate::patch::PatchIndex::new(0).unwrap(),
                    );
                    applied = true;
                    paused = Some(cursor);
                },
            );
            assert!(
                applied,
                "entry animation never started at placement {placement}: {result:?}"
            );
            if reopen_after.is_some() {
                assert!(reopened);
                assert_eq!(
                    result,
                    Ok(true),
                    "entry resumes at placement {placement}, hold {reopen_after:?}"
                );
            } else {
                assert!(
                    result
                        .as_ref()
                        .is_err_and(|error| error.starts_with("lift route stalled")),
                    "entry barrier closed during animation at placement {placement}: {result:?}"
                );
            }
        }
    }
}

fn audit_changing_climbs(sprite: Option<&crate::sprite::Sprite>) {
    let fixtures: Vec<serde_json::Value> = serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/asset-changing-climbs.levels.json"
    )))
    .unwrap();
    for (rotation, fixture) in fixtures.iter().enumerate() {
        let lift_type = &fixture["asset_geometry"]["lifts"][0]["lift_type"];
        let (mut engine, assets) = compiled_walkway(&serde_json::to_vec(fixture).unwrap());
        let sim = crate::sim_rng::test_context();
        let patch = crate::patch::PatchIndex::new(0).unwrap();
        for (step, applied) in [false, true, false].into_iter().enumerate() {
            if step > 0 {
                if applied {
                    engine.apply_patch(TickCtx::new(&sim, &assets), patch);
                } else {
                    engine.reset_patch(TickCtx::new(&sim, &assets), patch);
                }
            }
            for (entrance, exit) in [(0, 1), (1, 0)] {
                let enter = &engine.script_domains.interactables.doors[entrance];
                let leave = &engine.script_domains.interactables.doors[exit];
                let layer = enter.layer_in;
                let sector = enter.sector_in;
                assert_eq!(
                    engine
                        .world
                        .fast_grid
                        .is_reachable_thin(enter.point_in, leave.point_in, layer),
                    !applied,
                    "climb collision type={lift_type}, fixture={rotation}, applied={applied}, enter={:?}, leave={:?}",
                    enter.point_in,
                    leave.point_in
                );
                let route = engine.world.pathfinder.find_path(
                    &assets.navigation.pathfinder_graph,
                    &engine.world.fast_grid,
                    layer,
                    sector.into(),
                    0,
                    enter.point_in,
                    leave.point_in,
                    true,
                );
                assert_eq!(
                    route.is_some(),
                    !applied,
                    "climb planning type={lift_type}, rotation={rotation}, applied={applied}"
                );
                let Some(sprite) = sprite else { continue };
                let result = walk_exported_lift(
                    engine.clone(),
                    assets.clone(),
                    entrance,
                    exit,
                    Some(sprite),
                );
                if applied {
                    assert!(
                        result
                            .as_ref()
                            .is_err_and(|error| error.starts_with("lift route stalled")),
                        "closed climb type={lift_type}, rotation={rotation}, entrance={entrance}: {result:?}"
                    );
                } else {
                    assert_eq!(
                        result,
                        Ok(true),
                        "open climb type={lift_type}, entrance={entrance}"
                    );
                }
            }
        }
    }
}

fn audit_exported_lifts(
    types: &[crate::sector::LiftType],
    sprite: Option<&crate::sprite::Sprite>,
    report_name: &str,
) {
    let directory = std::path::PathBuf::from(std::env::var("ROBIN_ASSET_MAP_DIAGNOSTICS").unwrap());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("diagnostics.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["complete"], true);
    let report_path = directory.join(report_name);
    let mut report = serde_json::json!({
        "scope": "initial-state-directed-lift-walks-between-every-entrance-pair",
        "lift_types": types, "complete_sprite": sprite.is_some(),
        "input_snapshot_notes": manifest.get("snapshot_notes"),
        "map_filter": std::env::var("ROBIN_LIFT_AUDIT_MAP").ok(),
        "complete": false, "audit_finished": false, "results": []
    });
    let mut total_checked = 0;
    let mut total_failed = 0;
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    for result in manifest["results"].as_array().unwrap() {
        let file = result["file"].as_str().unwrap();
        if std::env::var("ROBIN_LIFT_AUDIT_MAP").is_ok_and(|selected| selected != file) {
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
        let mut checked = 0;
        let mut skipped = 0;
        let mut failures = vec![];
        let doors = &engine.script_domains.interactables.doors;
        for (sector_index, sector) in engine.world.fast_grid.level.sectors.iter().enumerate() {
            if !sector.lift_type.is_some_and(|kind| types.contains(&kind)) {
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
                        walk_exported_lift(engine.clone(), assets.clone(), entrance, exit, sprite);
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
            "{file}: {checked} lift routes, {} failures, {skipped} forbidden routes",
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
    assert!(total_checked > 0, "no lift routes tested");
    assert_eq!(total_failed, 0, "see {}", report_path.display());
}
