use super::*;

#[test]
#[ignore = "requires automatic trigger ZIP diagnostics and ROBIN_CLIMB_RHS"]
fn exported_asset_triggers_track_players_until_last_exit() {
    let directory = std::path::PathBuf::from(std::env::var("ROBIN_ASSET_MAP_DIAGNOSTICS").unwrap());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("diagnostics.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["complete"], true);
    assert_eq!(manifest["scope"], "editable-asset-occupancy-triggers");
    let sprite = exported_stairs::complete_climb_sprite();
    let mut walks = 0;
    for entry in manifest["results"].as_array().unwrap() {
        let file = entry["file"].as_str().unwrap();
        let bytes = std::fs::read(directory.join(file)).unwrap();
        for copy in 0..2 {
            let (mut engine, mut assets) = compiled_walkway_with_dimensions(&bytes, (3000., 2000.));
            assert_eq!(engine.script_domains.interactables.patches.len(), 2);
            let patch = engine.script_domains.interactables.patches[copy].clone();
            let points = engine.world.fast_grid.level.sectors
                [patch.apply_sector_index.unwrap() as usize]
                .points
                .clone();
            assert_eq!(points.len(), 4);
            let origin = points[0];
            let x = points[1] - origin;
            let y = points[3] - origin;
            let point = |u, v| origin + x.scale(u) + y.scale(v);
            let handle = crate::position_interface::SectorHandle::new(patch.sector).unwrap();
            let sector = engine
                .world
                .fast_grid
                .level
                .sectors
                .iter()
                .position(|sector| {
                    sector.sector_number == handle.number() && sector.layer == patch.layer
                })
                .unwrap();
            let handle = handle
                .with_arena_index(crate::fast_find_grid::SectorIndex::new(sector as u32).unwrap());
            let mut owners = vec![];
            for v in [0.25, 0.75] {
                let start = point(-0.35, v);
                let owner = walking_pc(&mut engine, &mut assets, start, patch.layer, handle);
                let element = engine.ent_mut(owner).element_data_mut();
                let iface = element.sprite.position_iface.clone();
                element.sprite = sprite.clone();
                element.sprite.position_iface = iface;
                let receiver =
                    engine.get_projection_area_index(&assets, handle, patch.layer, start);
                engine.set_obstacle_and_material(&assets, owner, receiver);
                owners.push(owner);
            }
            let initial_sight = engine.world.static_sight_obstacle_active.clone();
            let sim = crate::sim_rng::test_context();
            for (actor, entering, expected_count) in
                [(0, true, 1), (1, true, 2), (0, false, 1), (1, false, 0)]
            {
                let owner = owners[actor];
                let goal = point(
                    if entering { 0.5 } else { -0.35 },
                    if actor == 0 { 0.25 } else { 0.75 },
                );
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
                let mut arrived = false;
                for _ in 0..600 {
                    engine.control.frame_counter += 1;
                    engine.t_hourglass_phase_sequences(&assets);
                    engine.hourglass_phase_paths(TickCtx::new(&sim, &assets));
                    engine.t_tick_actor_owner_envelopes(&assets);
                    let element = engine.ent(owner).element_data();
                    let settled = engine
                        .entities()
                        .current_element_for_actor(owner)
                        .and_then(|(id, index)| engine.seq().get_element(id, index))
                        .is_none_or(|selected| {
                            selected.orders.is_empty() || selected.command == Command::Wait
                        });
                    if (element.position_map() - goal).length() < 1. && settled {
                        arrived = true;
                        break;
                    }
                }
                assert!(
                    arrived,
                    "{file}: copy={copy} actor={actor} entering={entering} did not arrive"
                );
                let current = &engine.script_domains.interactables.patches[copy];
                assert_eq!(
                    current.occupants.len(),
                    expected_count,
                    "{file}: copy={copy} actor={actor} entering={entering}"
                );
                assert_eq!(current.applied, expected_count > 0);
                assert_eq!(
                    current.is_inside(crate::patch::OccupantId(owner.index())),
                    entering
                );
                assert!(!engine.script_domains.interactables.patches[1 - copy].applied);
                assert!(
                    engine.script_domains.interactables.patches[1 - copy]
                        .occupants
                        .is_empty()
                );
                let mut expected_sight = initial_sight.clone();
                for index in &patch.old_sight_obstacle_indices {
                    expected_sight[usize::from(*index)] = expected_count == 0;
                }
                for index in &patch.new_sight_obstacle_indices {
                    expected_sight[usize::from(*index)] = expected_count > 0;
                }
                assert_eq!(engine.world.static_sight_obstacle_active, expected_sight);
                assert!(
                    (engine.ent(owner).element_data().position().z
                        - entry["elevation"].as_f64().unwrap() as f32)
                        .abs()
                        < 0.001
                );
                walks += 1;
            }
        }
    }
    assert_eq!(walks, 64);
    std::fs::write(directory.join("actor-trigger-report.json"), serde_json::to_vec_pretty(&serde_json::json!({"complete":true,"walks":walks,"scope":"native-player-trigger-occupancy-and-sight-states-from-editable-zips"})).unwrap()).unwrap();
}
