use super::{Engine, LevelAssets, LoadedLevel, construct_with_dimensions};
mod probes {
    use robin_engine::coordinates::{MapBBox, MapPoint};
    use robin_engine::fast_find_grid::{FastFindGrid, SectorHit};
    use robin_engine::pathfinder::{PathFinder, PathGraph};
    use robin_engine::sector::SectorType;
    include!("../support/collision_routes.rs");
}

fn check_routes(engine: &Engine, assets: &LevelAssets, label: &str) -> usize {
    let mut grid = engine.fast_grid().clone();
    let graph = &assets.navigation.pathfinder_graph;
    let mut finder = robin_engine::pathfinder::PathFinder::new();
    finder.initialize_from_graph(graph, &mut grid);
    probes::check_collision_routes(&grid, graph, &mut finder, label, |_, _| true)
}

#[test]
fn authored_terrain_routes_agree_with_collision_connected_samples() {
    let mut assets = LevelAssets::new();
    let engine = super::construct(
        include_bytes!("../fixtures/grid-terrain.level.json"),
        &mut assets,
    );
    assert!(check_routes(&engine, &assets, "authored terrain") > 0);
}

#[test]
#[ignore = "requires current exports via ROBIN_ASSET_MAP_DIAGNOSTICS"]
fn library_exports_route_between_collision_connected_samples() {
    let directory = std::path::PathBuf::from(std::env::var("ROBIN_ASSET_MAP_DIAGNOSTICS").unwrap());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("diagnostics.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["complete"], true);
    assert!(!manifest["results"].as_array().unwrap().is_empty());
    let report_path = directory.join("route-sampling-report.json");
    let mut report = serde_json::json!({
        "scope": "ordinary-sector-initial-state-routing",
        "complete": false,
        "results": []
    });
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    for result in manifest["results"].as_array().unwrap() {
        let started = std::time::Instant::now();
        let file = result["file"].as_str().unwrap();
        let bytes = std::fs::read(directory.join(file)).unwrap();
        let descriptor: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let dims = &descriptor["walkable_polygon"][2];
        let mut assets = LevelAssets::new();
        let engine = construct_with_dimensions(
            LoadedLevel::hackable_from_json(&bytes).unwrap(),
            &mut assets,
            (
                dims[0].as_f64().unwrap() as f32 + 1.,
                dims[1].as_f64().unwrap() as f32 + 1.,
            ),
        );
        let count = check_routes(&engine, &assets, file);
        assert!(count > 0, "{file}: no route coverage");
        println!("{file}: {count} sampled routes");
        report["results"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "file": file, "routes": count, "seconds": started.elapsed().as_secs_f64()
            }));
        std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    report["complete"] = serde_json::json!(true);
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
}
