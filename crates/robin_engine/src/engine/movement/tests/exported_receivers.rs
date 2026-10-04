use super::*;

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
        "complete": false, "results": []
    });
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    let mut total = 0;
    for result in manifest["results"].as_array().unwrap() {
        let file = result["file"].as_str().unwrap();
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
        let half = grid.try_move_box_half_diagonal(0).unwrap();
        let mut candidates = vec![];
        let mut pairs = std::collections::BTreeSet::new();
        for line in grid.level.lines.iter().filter(|line| line.is_elevation) {
            let (left, right) = (line.left_obstacle_index, line.right_obstacle_index);
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
