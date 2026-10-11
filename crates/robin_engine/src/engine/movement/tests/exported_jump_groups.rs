use super::*;

#[test]
#[ignore = "requires roof ZIP diagnostics and ROBIN_CLIMB_RHS"]
fn exported_roof_jumps_support_group_clicks() {
    let directory = std::path::PathBuf::from(std::env::var("ROBIN_ASSET_MAP_DIAGNOSTICS").unwrap());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("diagnostics.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["complete"], true);
    assert_eq!(manifest["zip_roundtrip"], true);
    assert_eq!(manifest["results"].as_array().unwrap().len(), 8);
    let sprite = exported_stairs::complete_climb_sprite();
    let mut results = Vec::new();
    for entry in manifest["results"].as_array().unwrap() {
        let file = entry["file"].as_str().unwrap();
        let bytes = std::fs::read(directory.join(file)).unwrap();
        let (base, base_assets) = compiled_walkway_with_dimensions(&bytes, (2000., 2000.));
        let depth = entry["approach_depth"].as_f64().unwrap() as f32 / 2.;
        for index in 0..base.world.fast_grid.level.jump_lines.len() {
            let mut engine = base.clone();
            let mut assets = base_assets.clone();
            let source = engine.world.fast_grid.level.jump_lines[index].clone();
            let destination = engine.world.fast_grid.level.jump_lines
                [source.associated_line_index.unwrap() as usize]
                .clone();
            let handle = |line: &crate::jump_line::JumpLine| {
                let index = line.sector_index.unwrap();
                crate::position_interface::SectorHandle::from_number(
                    engine.world.fast_grid.level.sectors[usize::from(index)].sector_number,
                )
                .with_arena_index(index)
            };
            let source_sector = handle(&source);
            let destination_sector = handle(&destination);
            let landing_posture = if engine.world.fast_grid.level.sectors
                [usize::from(destination.sector_index.unwrap())]
            .force_crouched
            {
                Posture::Crouched
            } else {
                Posture::Upright
            };
            let vector = source.vector();
            let inset =
                crate::coordinates::MapVec::new(-vector.y, vector.x).scale(depth / vector.length());
            let goal = destination.point_b + vector.scale(0.5) + inset;
            let mut owners = Vec::new();
            for t in [0.05, 0.5, 0.95] {
                let start = source.point_a + vector.scale(t) - inset;
                let owner =
                    walking_pc(&mut engine, &mut assets, start, source.layer, source_sector);
                engine.ent_mut(owner).pc_data_mut().unwrap().has_jump = true;
                let element = engine.ent_mut(owner).element_data_mut();
                let position = element.sprite.position_iface.clone();
                element.sprite = sprite.clone();
                element.sprite.position_iface = position;
                let receiver =
                    engine.get_projection_area_index(&assets, source_sector, source.layer, start);
                engine.set_obstacle_and_material(&assets, owner, receiver);
                resolve_jump_click(&engine, owner, start, goal, index, destination_sector).unwrap();
                owners.push(owner);
            }
            for (i, &owner) in owners.iter().enumerate() {
                let point = engine.ent(owner).element_data().position_map();
                for &other in &owners[..i] {
                    let delta = point - engine.ent(other).element_data().position_map();
                    assert!(
                        delta.x.abs() >= 12. || delta.y.abs() >= 6.,
                        "overlapping launch formation"
                    );
                }
            }
            let sim = crate::sim_rng::test_context();
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
            let mut flew = [false; 3];
            let mut arrived = false;
            for _ in 0..2000 {
                engine.control.frame_counter += 1;
                engine.t_hourglass_phase_sequences(&assets);
                engine.hourglass_phase_paths(TickCtx::new(&sim, &assets));
                engine.t_tick_actor_owner_envelopes(&assets);
                arrived = true;
                for (actor, &owner) in owners.iter().enumerate() {
                    let element = engine.ent(owner).element_data();
                    flew[actor] |= element.sprite.last_action == OrderType::JumpingLong;
                    let settled = engine
                        .entities()
                        .current_element_for_actor(owner)
                        .and_then(|(id, index)| engine.seq().get_element(id, index))
                        .is_none_or(|selected| {
                            selected.orders.is_empty() || selected.command == Command::Wait
                        });
                    arrived &= flew[actor]
                        && settled
                        && element.sector() == Some(destination_sector)
                        && element.layer() == destination.layer
                        && element.posture() == landing_posture
                        && (element.position_map() - goal).length() < vector.length() / 2.;
                }
                if arrived {
                    break;
                }
            }
            let positions: Vec<_> = owners.iter().map(|&owner| {
                let element = engine.ent(owner).element_data();
                serde_json::json!({"position": [element.position_map().x, element.position_map().y],
                    "sector": element.sector().map(u16::from), "posture": format!("{:?}", element.posture())})
            }).collect();
            assert!(
                arrived,
                "{file}: line={index}, flew={flew:?}, positions={positions:?}"
            );
            for (i, &owner) in owners.iter().enumerate() {
                actor_receiver_result(
                    &engine,
                    &assets,
                    owner,
                    destination_sector,
                    destination.layer,
                    engine.ent(owner).element_data().position_map(),
                )
                .unwrap();
                let point = engine.ent(owner).element_data().position_map();
                for &other in &owners[..i] {
                    let delta = point - engine.ent(other).element_data().position_map();
                    assert!(
                        delta.x.abs() >= 12. || delta.y.abs() >= 6.,
                        "{file}: overlapping landing formation on line {index}"
                    );
                }
            }
            results.push(serde_json::json!({"file":file,"line":index,"actors":positions}));
        }
    }
    assert!(!results.is_empty());
    std::fs::write(
        directory.join("actor-jump-group-report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"complete":true,"groups":results.len(),"results":results}),
        )
        .unwrap(),
    )
    .unwrap();
}
