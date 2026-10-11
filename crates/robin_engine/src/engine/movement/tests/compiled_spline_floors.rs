use super::*;

#[test]
#[ignore = "requires authored spline floor probes and ROBIN_CLIMB_RHS"]
fn exported_spline_floor_clicks_follow_receivers_through_repeats() {
    check_exported_floor_clicks(16, (500., 500.), "spline-floor-actor-report.json");
}

#[test]
#[ignore = "requires overlapping floor exports and ROBIN_CLIMB_RHS"]
fn exported_overlapping_floors_keep_clicks_on_the_joined_upper_region() {
    check_exported_floor_clicks(64, (3000., 2000.), "joined-floor-actor-report.json");
}

fn check_exported_floor_clicks(expected: usize, dimensions: (f32, f32), report: &str) {
    let directory = std::path::PathBuf::from(std::env::var("ROBIN_ASSET_MAP_DIAGNOSTICS").unwrap());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("diagnostics.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["complete"], true);
    let sprite = exported_stairs::complete_climb_sprite();
    let mut walks = 0;
    for entry in manifest["results"].as_array().unwrap() {
        let file = entry["file"].as_str().unwrap();
        let bytes = std::fs::read(directory.join(file)).unwrap();
        let layer = entry["layer"].as_u64().unwrap() as u16;
        for probe in entry["probes"].as_array().unwrap() {
            let point = |i: usize| {
                MapPoint::new(
                    probe[i][0].as_f64().unwrap() as f32,
                    probe[i][1].as_f64().unwrap() as f32,
                )
            };
            for (start, goal) in [(point(0), point(1)), (point(1), point(0))] {
                let (mut engine, mut assets) = compiled_walkway_with_dimensions(&bytes, dimensions);
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
                for _ in 0..2000 {
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
                    if settled && (position - goal).length() < 0.1 {
                        arrived = true;
                        break;
                    }
                }
                assert!(
                    arrived,
                    "{file}: actor stopped at {previous:?}, expected {goal:?}"
                );
                assert!(
                    distance < (goal - start).length() * 2.,
                    "{file}: unexpected detour"
                );
                walks += 1;
            }
        }
    }
    assert_eq!(walks, expected);
    std::fs::write(
        directory.join(report),
        serde_json::to_vec(&serde_json::json!({"complete":true,"walks":walks})).unwrap(),
    )
    .unwrap();
}
