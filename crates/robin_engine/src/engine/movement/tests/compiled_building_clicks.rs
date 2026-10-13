use super::*;

#[test]
#[ignore = "requires building_approaches with source_layer and ROBIN_CLIMB_RHS"]
fn exported_building_clicks_enter_from_terrain_and_return() {
    let directory = std::path::PathBuf::from(std::env::var("ROBIN_ASSET_MAP_DIAGNOSTICS").unwrap());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("diagnostics.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["complete"], true);
    let sprite = super::exported_stairs::complete_climb_sprite();
    let mut results = vec![];
    let mut failures = 0;
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
        for probe in result["building_approaches"].as_array().unwrap() {
            let outcome = building_click_round_trip(engine.clone(), assets.clone(), probe, &sprite);
            failures += usize::from(outcome.is_err());
            results.push(serde_json::json!({"file":file,"probe":probe,"error":outcome.err()}));
        }
    }
    let report = directory.join("actor-building-terrain-click-report.json");
    std::fs::write(&report, serde_json::to_vec_pretty(&serde_json::json!({
        "scope":"player-click-entry-from-terrain-and-return", "complete":!results.is_empty() && failures == 0,
        "audit_finished":true,"results":results,
    })).unwrap()).unwrap();
    assert!(!results.is_empty());
    assert_eq!(
        failures,
        0,
        "{failures}/{} terrain building clicks failed; see {}",
        results.len(),
        report.display()
    );
}

fn building_click_round_trip(
    mut engine: EngineInner,
    mut assets: LevelAssets,
    probe: &serde_json::Value,
    sprite: &crate::sprite::Sprite,
) -> Result<(), String> {
    let doors = engine
        .script_domains
        .interactables
        .doors
        .iter()
        .enumerate()
        .filter(|(_, door)| {
            door.door_type == crate::gate::DoorType::Building
                && u64::from(door.layer_out) == probe["layer"].as_u64().unwrap()
                && u64::from(u16::from(door.sector_out)) == probe["sector"].as_u64().unwrap()
                && f64::from(door.point_out.x) == probe["point_out"][0].as_f64().unwrap()
                && f64::from(door.point_out.y) == probe["point_out"][1].as_f64().unwrap()
        })
        .collect::<Vec<_>>();
    if doors.len() != 1 {
        return Err(format!("expected one entrance, found {}", doors.len()));
    }
    let (door_index, door) = doors[0];
    let door = door.clone();
    let world = &probe["source_world"];
    let source = MapPoint::new(
        world[0].as_f64().unwrap() as f32,
        (world[1].as_f64().unwrap() - world[2].as_f64().unwrap()) as f32,
    );
    let layer = u16::try_from(
        probe["source_layer"]
            .as_u64()
            .expect("explicit terrain source layer"),
    )
    .unwrap();
    let crate::fast_find_grid::SectorHit::Found {
        sector_idx,
        sector_number,
        ..
    } = engine.world.fast_grid.get_sector(source, source, layer)
    else {
        return Err(format!(
            "terrain source has no sector: {source:?} layer {layer}"
        ));
    };
    let handle = crate::position_interface::SectorHandle::from_number(sector_number)
        .with_arena_index(sector_idx);
    let owner = walking_pc(&mut engine, &mut assets, source, layer, handle);
    let element = engine.ent_mut(owner).element_data_mut();
    let position = element.sprite.position_iface.clone();
    element.sprite = sprite.clone();
    element.sprite.position_iface = position;
    let receiver = engine
        .get_projection_area_index(&assets, handle, layer, source)
        .ok_or("terrain source has no receiver")?;
    engine.set_obstacle_and_material(&assets, owner, Some(receiver));
    if !engine
        .world
        .fast_grid
        .is_position_authorized(engine.ent(owner).position_iface().get_move_box_map(), layer)
    {
        return Err(format!(
            "terrain source does not fit the standing character: {source:?}"
        ));
    }
    let sim = crate::sim_rng::test_context();
    let selects_entrance = |point| {
        engine
            .world
            .fast_grid
            .get_sector_screen(point, source)
            .sector_idx
            .is_some_and(|index| {
                engine.world.fast_grid.level.sectors[usize::from(index)]
                    .door_index
                    .is_some_and(|index| usize::try_from(index).unwrap() == door_index)
            })
    };
    // The animation handoff may lie outside the clickable door polygon.
    // Resolve an actual visible click in that polygon instead of assuming the
    // handoff itself is clickable or injecting the expected gate identity.
    let mut clicks = vec![door.point_mid];
    if !door.click_polygon.is_empty() {
        let xs = door.click_polygon.iter().map(|p| p.0).collect::<Vec<_>>();
        let ys = door.click_polygon.iter().map(|p| p.1).collect::<Vec<_>>();
        let min_x = xs.iter().copied().fold(f32::INFINITY, f32::min).ceil() as i32;
        let max_x = xs.iter().copied().fold(f32::NEG_INFINITY, f32::max).floor() as i32;
        let min_y = ys.iter().copied().fold(f32::INFINITY, f32::min).ceil() as i32;
        let max_y = ys.iter().copied().fold(f32::NEG_INFINITY, f32::max).floor() as i32;
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                clicks.push(MapPoint::new(x as f32, y as f32));
            }
        }
    }
    let entry_click = clicks
        .into_iter()
        .find(|point| selects_entrance(*point))
        .ok_or("no visible click selects the authored entrance polygon")?;
    for (phase, goal) in [("enter", entry_click), ("return", source)] {
        let current = engine.ent(owner).element_data().position_map();
        let selected = engine.world.fast_grid.get_sector_screen(goal, current);
        let selected_sector = selected
            .sector_idx
            .map(|index| &engine.world.fast_grid.level.sectors[usize::from(index)]);
        if phase == "enter" {
            if !selected_sector.is_some_and(|sector| {
                sector
                    .door_index
                    .is_some_and(|index| usize::try_from(index).unwrap() == door_index)
            }) {
                return Err(format!(
                    "entrance click selects a different sector: {selected:?}"
                ));
            }
        } else if selected.sector_idx != Some(sector_idx) || selected.layer != layer {
            return Err(format!(
                "return click selects a different terrain sector: {selected:?}"
            ));
        }
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
        for _ in 0..1500 {
            engine.control.frame_counter += 1;
            engine.t_hourglass_phase_sequences(&assets);
            engine.hourglass_phase_paths(TickCtx::new(&sim, &assets));
            engine.t_tick_actor_owner_envelopes(&assets);
            let element = engine.ent(owner).element_data();
            let sector = element.sector().ok_or("actor lost its sector")?;
            let passing = crate::engine::ai::selected_actor_is_passing_door(
                &engine.entities(),
                &engine.seq(),
                owner,
            );
            let inside = sector.arena_index() == door.sector_in_index;
            if !passing && !inside {
                actor_receiver_result(
                    &engine,
                    &assets,
                    owner,
                    sector,
                    element.layer(),
                    element.position_map(),
                )?;
            }
            if !passing
                && ((phase == "enter" && inside)
                    || (phase == "return"
                        && sector == handle
                        && element.layer() == layer
                        && (element.position_map() - source).length() < 0.02))
            {
                arrived = true;
                break;
            }
        }
        if !arrived {
            let element = engine.ent(owner).element_data();
            return Err(format!(
                "{phase} stalled at {:?}, layer {}, sector {:?}",
                element.position_map(),
                element.layer(),
                element.sector()
            ));
        }
    }
    Ok(())
}
