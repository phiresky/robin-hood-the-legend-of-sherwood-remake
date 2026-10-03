mod probes {
    use crate::coordinates::{MapBBox, MapPoint};
    use crate::fast_find_grid::{FastFindGrid, SectorHit};
    use crate::pathfinder::{PathFinder, PathGraph};
    use crate::sector::SectorType;
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/support/collision_routes.rs"
    ));
}

#[test]
fn changing_fixture_collision_routes_use_the_live_finder() {
    let (mut engine, assets) = load_compiled_transition(
        include_bytes!("../../tests/fixtures/asset-movement-transition.level.json"),
        (2000., 2000.),
    );
    let patch = crate::patch::PatchIndex::new(0).unwrap();
    let sim = crate::sim_rng::test_context();
    let mut counts = vec![];
    for state in ["initial", "applied", "reset"] {
        if state == "applied" {
            engine.apply_patch(TickCtx::new(&sim, &assets), patch);
        } else if state == "reset" {
            engine.reset_patch(TickCtx::new(&sim, &assets), patch);
        }
        let count = probes::check_collision_routes(
            &engine.world.fast_grid,
            &assets.navigation.pathfinder_graph,
            &mut engine.world.pathfinder,
            state,
            |layer, sector| layer == 0 && (sector == 0 || sector == 2),
        );
        assert!(count > 0);
        counts.push(count);
    }
    assert_eq!(counts[0], counts[2]);
}

#[test]
#[ignore = "requires current exports via ROBIN_ASSET_MAP_DIAGNOSTICS"]
fn exported_state_routes_match_live_collision_components() {
    check_exported_state_routes(false);
}

#[test]
#[ignore = "requires current exports via ROBIN_ASSET_MAP_DIAGNOSTICS"]
fn exported_combined_state_routes_match_live_collision_components() {
    check_exported_state_routes(true);
}

fn check_exported_state_routes(combined: bool) {
    let directory = std::path::PathBuf::from(std::env::var("ROBIN_ASSET_MAP_DIAGNOSTICS").unwrap());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("diagnostics.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["complete"], true);
    let report_path = directory.join(if combined {
        "combined-state-route-sampling-report.json"
    } else {
        "state-route-sampling-report.json"
    });
    let mut report = serde_json::json!({
        "scope": if combined { "ordinary-sector-all-applied-routing" }
            else { "ordinary-sector-independent-transition-routing" },
        "complete": false, "results": []
    });
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    let mut checked = 0;
    for result in manifest["results"].as_array().unwrap() {
        let file = result["file"].as_str().unwrap();
        let bytes = std::fs::read(directory.join(file)).unwrap();
        let descriptor: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let transitions: Vec<crate::level_data::CompiledMovementTransition> =
            serde_json::from_value(
                descriptor["asset_geometry"]
                    .get("movement_transitions")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!([])),
            )
            .unwrap();
        let dims = &descriptor["walkable_polygon"][2];
        let (mut engine, assets) = load_compiled_transition(
            &bytes,
            (
                dims[0].as_f64().unwrap() as f32 + 1.,
                dims[1].as_f64().unwrap() as f32 + 1.,
            ),
        );
        let sim = crate::sim_rng::test_context();
        let groups: Vec<Vec<usize>> = if combined {
            vec![(0..transitions.len()).collect()]
        } else {
            (0..transitions.len()).map(|index| vec![index]).collect()
        };
        for group in groups {
            let sectors: std::collections::BTreeSet<_> = group
                .iter()
                .flat_map(|&index| &transitions[index].motion_changes)
                .map(|change| (change.layer, change.sector))
                .collect();
            if sectors.is_empty() {
                continue;
            }
            let name = if combined {
                "all transitions"
            } else {
                &transitions[group[0]].id
            };
            for state in ["initial", "applied", "reset"] {
                if state == "applied" {
                    for &index in &group {
                        let patch = crate::patch::PatchIndex::new(index as u32).unwrap();
                        engine.apply_patch(TickCtx::new(&sim, &assets), patch);
                    }
                } else if state == "reset" {
                    for &index in group.iter().rev() {
                        let patch = crate::patch::PatchIndex::new(index as u32).unwrap();
                        engine.reset_patch(TickCtx::new(&sim, &assets), patch);
                    }
                }
                let started = std::time::Instant::now();
                let label = format!("{file}: {name} ({state})");
                // Keep the same live finder across all three states.
                let routes = probes::check_collision_routes(
                    &engine.world.fast_grid,
                    &assets.navigation.pathfinder_graph,
                    &mut engine.world.pathfinder,
                    &label,
                    |layer, sector| sectors.contains(&(layer, sector)),
                );
                checked += routes;
                report["results"].as_array_mut().unwrap().push(serde_json::json!({
                    "file": file, "transition": name, "state": state,
                    "members": group.iter().map(|&index| &transitions[index].id).collect::<Vec<_>>(),
                    "sectors": sectors, "routes": routes, "seconds": started.elapsed().as_secs_f64()
                }));
                std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
            }
        }
    }
    assert!(checked > 0, "No changing movement areas sampled");
    report["complete"] = true.into();
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
}
