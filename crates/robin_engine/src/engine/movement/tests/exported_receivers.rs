use super::*;

#[test]
#[ignore = "requires exported geometry and ray probes via ROBIN_ASSET_MAP_DIAGNOSTICS"]
fn exported_geometry_preserves_authored_sight_and_projectile_gaps() {
    use crate::sight_obstacle::{SIGHTOBSTACLE_OPAQUE, SIGHTOBSTACLE_SOLID, is_reachable_3d};
    let directory = std::path::PathBuf::from(std::env::var("ROBIN_ASSET_MAP_DIAGNOSTICS").unwrap());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("diagnostics.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["complete"], true);
    let report_path = directory.join("ray-probe-report.json");
    let mut report = serde_json::json!({"complete": false, "scope": "authored-initial-state-ray-probes", "results": []});
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    let mut checked = 0;
    let mut failures = 0;
    for result in manifest["results"].as_array().unwrap() {
        let Some(probes) = result["ray_probes"].as_array() else {
            continue;
        };
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
        for probe in probes {
            let endpoints: [[f32; 3]; 2] =
                serde_json::from_value(probe["endpoints"].clone()).unwrap();
            for (kind, mask) in [
                ("sight", SIGHTOBSTACLE_OPAQUE),
                ("projectile", SIGHTOBSTACLE_SOLID),
            ] {
                let expected = probe["clear"].as_bool().unwrap();
                for reverse in [false, true] {
                    let (a, b) = if reverse {
                        (endpoints[1], endpoints[0])
                    } else {
                        (endpoints[0], endpoints[1])
                    };
                    let actual = is_reachable_3d(engine.sight_obstacles(&assets), a, b, mask);
                    checked += 1;
                    failures += usize::from(actual != expected);
                    report["results"]
                        .as_array_mut()
                        .unwrap()
                        .push(serde_json::json!({
                            "file": file, "probe": probe["name"], "kind": kind, "reverse": reverse,
                            "expected_clear": expected, "actual_clear": actual
                        }));
                }
            }
        }
    }
    report["complete"] = true.into();
    report["checked"] = checked.into();
    report["failures"] = failures.into();
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    assert!(checked > 0, "no sight/projectile probes tested");
    assert_eq!(failures, 0, "see {}", report_path.display());
    eprintln!("{checked} sight/projectile ray probes passed");
}

#[test]
fn loaded_movement_obstacles_reject_mouse_positions_inside_and_on_boundary() {
    let mut descriptor: serde_json::Value = serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/asset-navigation-copies.level.json"
    )))
    .unwrap();
    descriptor["asset_geometry"]["motion_data"]["layers"][1][0]["obstacles"] = serde_json::json!([{
        "state_id": 0,
        "polygon": { "points": [[480, 280], [520, 280], [520, 320], [480, 320]] }
    }]);
    let (engine, _) = compiled_walkway(&serde_json::to_vec(&descriptor).unwrap());
    let grid = &engine.world.fast_grid;
    for point in [
        MapPoint::new(500., 300.),
        MapPoint::new(480., 300.),
        MapPoint::new(480., 280.),
    ] {
        assert!(
            matches!(
                grid.get_sector(point, point, 1),
                crate::fast_find_grid::SectorHit::Blocked
            ),
            "movement obstacle must reject mouse position {point:?}"
        );
    }
    let outside = MapPoint::new(550., 300.);
    assert!(matches!(
        grid.get_sector(outside, outside, 1),
        crate::fast_find_grid::SectorHit::Found { .. }
    ));
}

#[test]
#[ignore = "requires assembly exports and endpoint routes via ROBIN_ASSET_MAP_DIAGNOSTICS"]
fn exported_endpoint_routes_support_actor_crossings() {
    let directory = std::path::PathBuf::from(std::env::var("ROBIN_ASSET_MAP_DIAGNOSTICS").unwrap());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("diagnostics.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["complete"], true);
    let mut checked = 0;
    for result in manifest["results"].as_array().unwrap() {
        let file = result["file"].as_str().unwrap();
        let bytes = std::fs::read(directory.join(file)).unwrap();
        let descriptor: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let layers = descriptor["asset_geometry"]["motion_data"]["layers"]
            .as_array()
            .unwrap();
        let (layer, sector) = if let (Some(layer), Some(sector)) =
            (result["layer"].as_u64(), result["sector"].as_u64())
        {
            (
                u16::try_from(layer).unwrap(),
                usize::try_from(sector).unwrap(),
            )
        } else {
            assert_eq!(
                layers
                    .iter()
                    .map(|layer| layer.as_array().unwrap().len())
                    .sum::<usize>(),
                1,
                "endpoint audit requires a single navigation region: {file}"
            );
            assert_eq!(layers[0].as_array().unwrap().len(), 1);
            (0, 0)
        };
        let dimensions = &descriptor["walkable_polygon"][2];
        let (engine, assets) = compiled_walkway_with_dimensions(
            &bytes,
            (
                dimensions[0].as_f64().unwrap() as f32 + 1.,
                dimensions[1].as_f64().unwrap() as f32 + 1.,
            ),
        );
        let routes = result["routes"]
            .as_array()
            .expect("endpoint routes are required");
        assert!(!routes.is_empty(), "no endpoint routes in {file}");
        if let Some(points) = result["blocked_points"].as_array() {
            let grid = &engine.world.fast_grid;
            for point in points {
                let x = point[0].as_f64().unwrap() as f32;
                let y = point[1].as_f64().unwrap() as f32;
                let position = MapPoint::new(x, y);
                let hit = grid.get_sector(position, position, layer);
                assert!(
                    matches!(hit, crate::fast_find_grid::SectorHit::Blocked),
                    "{file}: expected blocked position [{x}, {y}], got {hit:?}"
                );
            }
        }
        for route in routes {
            let point = |i: usize| {
                MapPoint::new(
                    route[i][0].as_f64().unwrap() as f32,
                    route[i][1].as_f64().unwrap() as f32,
                )
            };
            for (source, goal) in [(point(0), point(1)), (point(1), point(0))] {
                eprintln!("{file}: endpoint crossing {source:?} -> {goal:?}");
                tick_walkway_movement(
                    engine.clone(),
                    assets.clone(),
                    layer,
                    sector,
                    source,
                    goal,
                    !result["same_receiver_routes"].as_bool().unwrap_or(false),
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 0, "no endpoint crossings tested");
    eprintln!("{checked} directed endpoint crossings passed");
}

#[test]
#[ignore = "requires current exports via ROBIN_ASSET_MAP_DIAGNOSTICS"]
fn exported_receiving_seams_support_actor_crossings() {
    check_exported_receiver_crossings(false);
}

#[test]
#[ignore = "requires current exports via ROBIN_ASSET_MAP_DIAGNOSTICS"]
fn exported_ground_boundaries_support_actor_crossings() {
    check_exported_receiver_crossings(true);
}

fn check_exported_receiver_crossings(ground: bool) {
    let map_filter = std::env::var("ROBIN_RECEIVER_AUDIT_MAP").ok();
    let receiver_filter = std::env::var("ROBIN_RECEIVER_AUDIT_OBSTACLES")
        .ok()
        .map(|value| {
            value
                .split(',')
                .map(|index| {
                    index
                        .parse::<usize>()
                        .expect("receiver index must be an unsigned integer")
                })
                .collect::<std::collections::BTreeSet<_>>()
        });
    let directory = std::path::PathBuf::from(std::env::var("ROBIN_ASSET_MAP_DIAGNOSTICS").unwrap());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("diagnostics.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["complete"], true);
    let report_path = directory.join(if ground {
        "actor-ground-crossing-report.json"
    } else {
        "actor-receiver-crossing-report.json"
    });
    let mut report = serde_json::json!({
        "scope": if ground { "sampled-initial-state-actor-ground-crossings" }
            else { "sampled-initial-state-actor-receiver-crossings" },
        "map_filter": map_filter, "receiver_filter": receiver_filter,
        "complete": false, "results": []
    });
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    let mut total = 0;
    for result in manifest["results"].as_array().unwrap() {
        let file = result["file"].as_str().unwrap();
        if map_filter.as_ref().is_some_and(|filter| filter != file) {
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
        let grid = &engine.world.fast_grid;
        if let Some(indices) = &receiver_filter {
            assert!(
                indices
                    .iter()
                    .all(|index| *index < assets.environment.static_sight_obstacles.len()),
                "receiver filter includes an unknown obstacle in {file}"
            );
        }
        let half = grid.try_move_box_half_diagonal(0).unwrap();
        let mut candidates = vec![];
        let mut pairs = std::collections::BTreeSet::new();
        for line in grid.level.lines.iter().filter(|line| line.is_elevation) {
            let (left, right) = (line.left_obstacle_index, line.right_obstacle_index);
            if receiver_filter.as_ref().is_some_and(|indices| {
                ![left, right]
                    .into_iter()
                    .flatten()
                    .any(|index| indices.contains(&usize::from(index)))
            }) {
                continue;
            }
            if (left.is_none() || right.is_none()) != ground {
                continue;
            }
            let receiver = left.or(right).expect("elevation line must have a receiver");
            let Some(topology) = assets.environment.static_sight_obstacles[usize::from(receiver)]
                .projection_area_ref()
            else {
                continue;
            };
            let sector_index = usize::from(topology.sector);
            let sector = &grid.level.sectors[sector_index];
            let layer = u16::from(topology.layer);
            let handle = crate::position_interface::SectorHandle::from_number(sector.sector_number)
                .with_arena_index(topology.sector);
            let direction = line.b - line.a;
            let length = direction.length();
            if length < 16. {
                continue;
            }
            let midpoint = line.a + direction.scale(0.5);
            let offset =
                crate::coordinates::MapVec::new(-direction.y, direction.x).scale(12. / length);
            let a = midpoint + offset;
            let b = midpoint - offset;
            let receiver_a = engine.get_projection_area_index(&assets, handle, layer, a);
            let receiver_b = engine.get_projection_area_index(&assets, handle, layer, b);
            if receiver_a == receiver_b || (receiver_a.is_none() || receiver_b.is_none()) != ground
            {
                continue;
            }
            // Endpoints must have an unambiguous receiving plane; intermediate
            // actor ticks still exercise exact shared-edge positions.
            if receiver_a.is_some_and(|receiver| receiver_edge_contains(&assets, receiver, a))
                || receiver_b.is_some_and(|receiver| receiver_edge_contains(&assets, receiver, b))
            {
                continue;
            }
            let authorized = [a, b].into_iter().all(|point| {
                let bounds = crate::coordinates::MapBBox::from_corners(
                    MapPoint::new(point.x - half.x, point.y - half.y),
                    MapPoint::new(point.x + half.x, point.y + half.y),
                );
                matches!(grid.get_sector(point, point, layer), crate::fast_find_grid::SectorHit::Found { sector_number, .. }
                    if sector_number == sector.sector_number)
                    && grid.is_position_authorized(&bounds, layer)
            });
            if !authorized || !grid.is_reachable_thick(a, b, layer, half) {
                continue;
            }
            let pair = (left.min(right), left.max(right));
            if pairs.insert(pair) {
                candidates.push((layer, sector_index, a, b));
            }
        }
        let mut checked = 0;
        eprintln!(
            "{file}: testing {} eligible receiver pairs",
            candidates.len()
        );
        for &(layer, sector, a, b) in &candidates {
            for (source, goal) in [(a, b), (b, a)] {
                tick_walkway_crossing(engine.clone(), assets.clone(), layer, sector, source, goal);
                checked += 1;
            }
        }
        eprintln!(
            "{file}: {checked} actor crossings, {} eligible receiver pairs",
            candidates.len()
        );
        report["results"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "file": file, "eligible_pairs": candidates.len(), "directed_crossings": checked
            }));
        std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        total += checked;
    }
    assert!(total > 0, "no actor crossings sampled");
    report["complete"] = true.into();
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
}
