use std::collections::VecDeque;

/// Collision flood-fill supplies expected connectivity independently of routing nodes.
/// This deliberately excludes inter-sector doors, jumps and traversal callbacks.
pub fn check_collision_routes(
    grid: &FastFindGrid,
    graph: &PathGraph,
    finder: &mut PathFinder,
    label: &str,
    include_sector: impl Fn(u16, u16) -> bool,
) -> usize {
    let half = grid.try_move_box_half_diagonal(0).unwrap();
    assert_eq!((half.x, half.y), (6., 3.));
    let mut checked = 0;
    let sampling_started = std::time::Instant::now();
    let mut routing_time = std::time::Duration::ZERO;
    for sector in &grid.level.sectors {
        if !sector
            .sector_type
            .contains(SectorType::AREA | SectorType::MOTION)
        {
            continue;
        }
        // The grid also carries unnumbered boundary sectors without routing nodes.
        if sector.sector_number.get() < 0 {
            continue;
        }
        let number = u16::try_from(sector.sector_number.get()).unwrap();
        let layer = sector.layer;
        if !include_sector(layer, number) {
            continue;
        }
        let min_x = sector
            .points
            .iter()
            .map(|p| p.x)
            .fold(f32::INFINITY, f32::min);
        let min_y = sector
            .points
            .iter()
            .map(|p| p.y)
            .fold(f32::INFINITY, f32::min);
        let max_x = sector
            .points
            .iter()
            .map(|p| p.x)
            .fold(f32::NEG_INFINITY, f32::max);
        let max_y = sector
            .points
            .iter()
            .map(|p| p.y)
            .fold(f32::NEG_INFINITY, f32::max);
        let step = ((max_x - min_x).max(max_y - min_y) / 64.).clamp(8., 64.);
        let width = ((max_x - min_x) / step).ceil() as usize;
        let height = ((max_y - min_y) / step).ceil() as usize;
        let mut samples = vec![None; width * height];
        for (index, slot) in samples.iter_mut().enumerate() {
            let point = MapPoint::new(
                min_x + (index % width) as f32 * step + step / 2.,
                min_y + (index / width) as f32 * step + step / 2.,
            );
            if !matches!(grid.get_sector(point, point, layer), SectorHit::Found { sector_number, .. }
                if sector_number == sector.sector_number)
            {
                continue;
            }
            let bounds = MapBBox::from_corners(
                MapPoint::new(point.x - half.x, point.y - half.y),
                MapPoint::new(point.x + half.x, point.y + half.y),
            );
            if grid.is_position_authorized(&bounds, layer) {
                *slot = Some(point);
            }
        }
        let mut visited = vec![false; samples.len()];
        for seed in 0..samples.len() {
            let Some(start) = samples[seed] else {
                continue;
            };
            if visited[seed] {
                continue;
            }
            visited[seed] = true;
            let mut queue = VecDeque::from([seed]);
            let mut members = vec![];
            while let Some(index) = queue.pop_front() {
                members.push(index);
                let x = index % width;
                let y = index / width;
                for (nx, ny) in [
                    (x as isize - 1, y as isize),
                    (x as isize + 1, y as isize),
                    (x as isize, y as isize - 1),
                    (x as isize, y as isize + 1),
                ] {
                    if nx < 0 || ny < 0 || nx >= width as isize || ny >= height as isize {
                        continue;
                    }
                    let next = ny as usize * width + nx as usize;
                    let Some(point) = samples[next] else {
                        continue;
                    };
                    if !visited[next]
                        && grid.is_reachable_thick(samples[index].unwrap(), point, layer, half)
                    {
                        visited[next] = true;
                        queue.push_back(next);
                    }
                }
            }
            if members.len() < 2 {
                continue;
            }
            let goal = samples[*members.last().unwrap()].unwrap();
            for (from, to) in [(start, goal), (goal, start)] {
                // Authorized samples allow the native direct-path check before A*.
                let query_started = std::time::Instant::now();
                let route = finder.find_path(graph, grid, layer, number, 0, from, to, true)
                    .unwrap_or_else(|| panic!("{label}: collision-connected samples have no route in sector {number}, layer {layer}: {from:?} -> {to:?}"));
                routing_time += query_started.elapsed();
                assert_eq!(route.last(), Some(&to), "{label}: incomplete route");
                let mut previous = from;
                for &point in &route {
                    assert!(
                        grid.is_reachable_thick(previous, point, layer, half),
                        "{label}: route crosses collision in sector {number}, layer {layer}: {previous:?} -> {point:?}; query {from:?} -> {to:?}; route {route:?}"
                    );
                    previous = point;
                }
                checked += 1;
            }
        }
    }
    println!(
        "{label}: {checked} routes; pathfinding {:.2}s, total sampling {:.2}s",
        routing_time.as_secs_f64(),
        sampling_started.elapsed().as_secs_f64()
    );
    checked
}
