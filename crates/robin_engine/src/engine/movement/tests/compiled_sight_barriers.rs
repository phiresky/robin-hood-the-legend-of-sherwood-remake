use super::*;

#[test]
#[ignore = "requires check-automatic-triggers.mjs --sight-barriers diagnostics"]
fn exported_sight_barriers_change_live_guard_detection() {
    let directory = std::path::PathBuf::from(std::env::var("ROBIN_ASSET_MAP_DIAGNOSTICS").unwrap());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("diagnostics.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["complete"], true);
    assert_eq!(manifest["scope"], "editable-asset-sight-barriers");
    assert_eq!(manifest["results"].as_array().unwrap().len(), 8);
    let mut checks = 0;
    for entry in manifest["results"].as_array().unwrap() {
        let file = entry["file"].as_str().unwrap();
        let bytes = std::fs::read(directory.join(file)).unwrap();
        let (mut engine, mut assets) = compiled_walkway_with_dimensions(&bytes, (3000., 2000.));
        let mut actors = Vec::new();
        for (copy, applied_position) in [(0, false), (0, true), (1, false), (1, true)] {
            let patch = &engine.script_domains.interactables.patches[copy];
            let index = if applied_position {
                patch.new_sight_obstacle_indices[0]
            } else {
                patch.old_sight_obstacle_indices[0]
            };
            let obstacle = &assets.environment.static_sight_obstacles[usize::from(index)];
            let a = &obstacle.obstacle_points[0];
            let b = &obstacle.obstacle_points[1];
            let c = &obstacle.obstacle_points[2];
            let center = MapPoint::new((a.x + c.x) / 2., (a.y + c.y) / 2.);
            let delta = MapPoint::new(b.x, b.y) - MapPoint::new(a.x, a.y);
            let offset = delta.scale(40. / delta.length());
            let viewer =
                engine.add_test_entity(crate::engine::test_support::actors::make_test_ai_soldier(
                    crate::element_kinds::Camp::Lacklandists,
                ));
            let target = engine.add_test_entity(crate::engine::test_support::actors::make_test_pc(
                Posture::Upright,
            ));
            for (id, point) in [(viewer, center - offset), (target, center + offset)] {
                let element = engine.ent_mut(id).element_data_mut();
                element.active = true;
                element.set_position(crate::coordinates::WorldPoint3D::new(
                    point.x,
                    point.y,
                    entry["elevation"].as_f64().unwrap() as f32,
                ));
            }
            let npc = engine.ent_mut(viewer).ai_actor_data_mut().unwrap();
            npc.view_radius = 400;
            npc.view_direction = [1., 0.];
            npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
            npc.eye_status = crate::element::EyeStatus::LookForward;
            actors.push((copy, applied_position, viewer, target));
        }
        crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
        let sim = crate::sim_rng::test_context();
        for (operation, expected) in [
            (None, [false, false]),
            (Some((0, true)), [true, false]),
            (Some((1, true)), [true, true]),
            (Some((0, false)), [false, true]),
            (Some((1, false)), [false, false]),
        ] {
            if let Some((copy, apply)) = operation {
                let patch = crate::patch::PatchIndex::new(copy).unwrap();
                if apply {
                    engine.apply_patch(TickCtx::new(&sim, &assets), patch);
                } else {
                    engine.reset_patch(TickCtx::new(&sim, &assets), patch);
                }
            }
            for &(copy, applied_position, viewer, target) in &actors {
                let mut visible = false;
                let mut standard_visible = false;
                crate::sight_obstacle::begin_parity_visibility_capture();
                for direction in 0..16 {
                    engine.control.frame_counter += 1;
                    engine
                        .ent_mut(viewer)
                        .element_data_mut()
                        .set_direction_instantly(direction);
                    engine.ent_mut(viewer).ai_actor_data_mut().unwrap().view_direction =
                        crate::position_interface::sector_to_vector_iso(direction);
                    visible |= engine.live_ai_detects_180(&assets, viewer, target);
                    standard_visible |= engine.npc_is_detecting_human(
                        &assets,
                        viewer,
                        target,
                        engine.control.frame_counter,
                    );
                }
                assert!(
                    !crate::sight_obstacle::take_parity_visibility_capture().is_empty(),
                    "{file}: copy={copy} must reach the obstacle visibility query"
                );
                assert_eq!(
                    visible,
                    expected[copy] != applied_position,
                    "{file}: copy={copy} applied_position={applied_position} operation={operation:?}"
                );
                assert_eq!(
                    standard_visible,
                    expected[copy] != applied_position,
                    "{file}: standard view copy={copy} applied_position={applied_position} operation={operation:?}"
                );
                checks += 2;
            }
        }
    }
    std::fs::write(
        directory.join("guard-sight-report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({"checks": checks})).unwrap(),
    )
    .unwrap();
}
