// Exercise authored ordinary passages before activation, after activation and after reset.
#[test]
#[ignore = "requires passage_controls probes in ROBIN_ASSET_MAP_DIAGNOSTICS and ROBIN_CLIMB_RHS"]
fn exported_ordinary_passage_controls_support_actor_round_trips() {
    let directory = std::path::PathBuf::from(std::env::var("ROBIN_ASSET_MAP_DIAGNOSTICS").unwrap());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("diagnostics.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["complete"], true);
    let sprite = complete_climb_sprite();
    let sim = crate::sim_rng::test_context();
    let mut results = Vec::new();
    for entry in manifest["results"].as_array().unwrap() {
        let file = entry["file"].as_str().unwrap();
        let bytes = std::fs::read(directory.join(file)).unwrap();
        let descriptor: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let dims = &descriptor["walkable_polygon"][2];
        for probe in entry["passage_controls"].as_array().unwrap() {
            let (mut engine, assets) = compiled_walkway_with_dimensions(
                &bytes,
                (
                    dims[0].as_f64().unwrap() as f32 + 1.,
                    dims[1].as_f64().unwrap() as f32 + 1.,
                ),
            );
            let patches: Vec<_> = probe["patches"]
                .as_array()
                .unwrap()
                .iter()
                .map(|patch| {
                    let index = patch["index"].as_u64().unwrap();
                    assert_eq!(
                        descriptor["asset_geometry"]["movement_transitions"][index as usize]["id"],
                        patch["id"]
                    );
                    crate::patch::PatchIndex::new(u32::try_from(index).unwrap()).unwrap()
                })
                .collect();
            assert!(!patches.is_empty());
            let doors = probe["doors"].as_array().unwrap();
            assert!(!doors.is_empty());
            for state in ["closed", "open", "reset"] {
                for &patch in &patches {
                    if state == "open" {
                        engine.apply_patch(TickCtx::new(&sim, &assets), patch);
                    } else if state == "reset" {
                        engine.reset_patch(TickCtx::new(&sim, &assets), patch);
                    }
                }
                for door in doors {
                    let index = usize::try_from(door.as_u64().unwrap()).unwrap();
                    let runtime = &engine.script_domains.interactables.doors[index];
                    assert_eq!(runtime.door_type, crate::gate::DoorType::Default);
                    let outcome = walk_exported_lift_with_tick(
                        engine.clone(),
                        assets.clone(),
                        index,
                        index,
                        Some(&sprite),
                        |_, _, _| {},
                    );
                    let expected = state == "open";
                    results.push(serde_json::json!({
                        "file": file, "door": index, "state": state,
                        "expected_round_trip": expected, "passed": outcome == Ok(expected),
                        "outcome": outcome,
                    }));
                }
            }
        }
    }
    let complete = !results.is_empty() && results.iter().all(|result| result["passed"] == true);
    let report = directory.join("actor-passage-control-report.json");
    std::fs::write(&report, serde_json::to_vec_pretty(&serde_json::json!({
        "scope": "prepared-ordinary-passage-round-trips-and-lock-reset-not-player-selection-or-bypass-proof",
        "complete": complete, "results": results,
    })).unwrap()).unwrap();
    assert!(complete, "see {}", report.display());
}
