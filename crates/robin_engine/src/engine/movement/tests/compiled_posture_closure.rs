use super::*;

#[test]
#[ignore = "requires initially closed posture ZIP diagnostics and ROBIN_CLIMB_RHS"]
fn exported_posture_barriers_crush_overlapping_crossings() {
    let directory = std::path::PathBuf::from(std::env::var("ROBIN_ASSET_MAP_DIAGNOSTICS").unwrap());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("diagnostics.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["complete"], true);
    let sprite = exported_stairs::complete_climb_sprite();
    let mut cases = 0;
    for entry in manifest["results"].as_array().unwrap() {
        assert_eq!(entry["initially_closed"], true);
        let file = entry["file"].as_str().unwrap();
        let bytes = std::fs::read(directory.join(file)).unwrap();
        let descriptor: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let dimensions = (
            descriptor["walkable_polygon"][2][0].as_f64().unwrap() as f32 + 1.,
            descriptor["walkable_polygon"][2][1].as_f64().unwrap() as f32 + 1.,
        );
        for copy in 0..2 {
            for reverse in [false, true] {
                let (mut engine, mut assets) = compiled_walkway_with_dimensions(&bytes, dimensions);
                let sim = crate::sim_rng::test_context();
                let patch = crate::patch::PatchIndex::new(copy as u32).unwrap();
                engine.apply_patch(TickCtx::new(&sim, &assets), patch);
                let door = engine.script_domains.interactables.doors[copy].clone();
                assert!(!door.passage_blocked);
                let (start, goal, sector, layer) = if reverse {
                    (
                        door.point_in,
                        door.point_out,
                        door.sector_in_index.unwrap(),
                        door.layer_in,
                    )
                } else {
                    (
                        door.point_out,
                        door.point_in,
                        door.sector_out_index.unwrap(),
                        door.layer_out,
                    )
                };
                let handle = crate::position_interface::SectorHandle::from_number(
                    engine.world.fast_grid.level.sectors[usize::from(sector)].sector_number,
                )
                .with_arena_index(sector);
                let owner = walking_pc(&mut engine, &mut assets, start, layer, handle);
                let element = engine.ent_mut(owner).element_data_mut();
                let iface = element.sprite.position_iface.clone();
                element.sprite = sprite.clone();
                element.sprite.position_iface = iface;
                let receiver = engine.get_projection_area_index(&assets, handle, layer, start);
                engine.set_obstacle_and_material(&assets, owner, receiver);
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
                let mut crossed = false;
                for _ in 0..600 {
                    engine.control.frame_counter += 1;
                    engine.t_hourglass_phase_sequences(&assets);
                    engine.hourglass_phase_paths(TickCtx::new(&sim, &assets));
                    engine.t_tick_actor_owner_envelopes(&assets);
                    let position = engine.ent(owner).element_data().position_map();
                    if (position - start).length() > 0.01 && (position - goal).length() > 0.1 {
                        assert!(!engine.ent(owner).element_data().unreachable);
                        let other_before =
                            engine.script_domains.interactables.doors[1 - copy].passage_blocked;
                        // Operate the reversible control again. A forced reset
                        // restores state without actor invalidation or damage.
                        engine.apply_patch(TickCtx::new(&sim, &assets), patch);
                        assert!(engine.script_domains.interactables.doors[copy].passage_blocked);
                        assert_eq!(
                            engine.script_domains.interactables.doors[1 - copy].passage_blocked,
                            other_before
                        );
                        assert!(
                            !engine.world.fast_grid.is_position_authorized(
                                engine.ent(owner).position_iface().get_move_box_map(),
                                layer,
                            ),
                            "fixture closure did not overlap the crossing in {file}"
                        );
                        assert!(
                            engine.ent(owner).element_data().unreachable,
                            "closing solid barrier did not crush crossing actor in {file}, copy={copy}, reverse={reverse}"
                        );
                        crossed = true;
                        break;
                    }
                }
                assert!(
                    crossed,
                    "crossing never started in {file}, copy={copy}, reverse={reverse}"
                );
                cases += 1;
            }
        }
    }
    assert_eq!(cases, 32);
    std::fs::write(directory.join("actor-posture-crushing-report.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "complete":true, "cases":cases, "scope":"solid-barrier-appearance-during-posture-crossings-not-safe-pause-resume"
    })).unwrap()).unwrap();
}
