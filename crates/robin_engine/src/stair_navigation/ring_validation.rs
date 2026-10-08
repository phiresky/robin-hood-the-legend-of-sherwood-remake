//! Accelerate large single-ring validation without changing intersection rules.

use geo::{GeoFloat, Intersects, Polygon, RemoveRepeatedPoints, Validation};

pub(super) fn is_valid<T: GeoFloat>(polygon: &Polygon<T>) -> bool {
    let ring = polygon.exterior();
    if ring.0.len() < 64 || !polygon.interiors().is_empty() {
        return polygon.is_valid();
    }
    if ring.0.iter().any(|p| !p.x.is_finite() || !p.y.is_finite())
        || ring.remove_repeated_points().0.len() < 4
    {
        return false;
    }
    let mut edges = ring
        .lines()
        .map(|line| {
            (
                line.start.x.min(line.end.x),
                line.start.x.max(line.end.x),
                line.start.y.min(line.end.y),
                line.start.y.max(line.end.y),
                line,
            )
        })
        .collect::<Vec<_>>();
    edges.sort_by(|a, b| a.0.partial_cmp(&b.0).expect("finite ring bounds"));
    for (index, &(_, max_x, min_y, max_y, line)) in edges.iter().enumerate() {
        for &(other_min_x, _, other_min_y, other_max_y, other) in &edges[index + 1..] {
            if other_min_x > max_x {
                break;
            }
            if other_min_y > max_y || other_max_y < min_y {
                continue;
            }
            // The exact predicate and shared-endpoint exclusions match polygon
            // validation. Bounds prune only pairs that cannot intersect.
            if line.intersects(&other) && line.start != other.end && line.end != other.start {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dense_rings_match_polygon_validation() {
        for count in [64, 129, 530] {
            let points = (0..count)
                .map(|i| {
                    let angle = i as f64 * std::f64::consts::TAU / count as f64;
                    (angle.cos() * 100. + 1600., angle.sin() * 100. + 1400.)
                })
                .collect::<Vec<_>>();
            for variant in 0..8 {
                let mut points = points.clone();
                match variant {
                    0 => {}
                    1 => points.reverse(),
                    2 => points.swap(1, count / 2),
                    3 => points[2] = points[1],
                    4 => points[2] = points[count / 2],
                    5 => points[1].0 = f64::NAN,
                    6 => points.fill((0., 0.)),
                    7 => points[1].1 = f64::INFINITY,
                    _ => unreachable!(),
                }
                let polygon = Polygon::new(points.into(), vec![]);
                assert_eq!(is_valid(&polygon), polygon.is_valid(), "{count}/{variant}");
                let polygon = geo::MapCoords::map_coords(&polygon, |p| geo::Coord {
                    x: p.x as f32,
                    y: p.y as f32,
                });
                assert_eq!(
                    is_valid(&polygon),
                    polygon.is_valid(),
                    "f32 {count}/{variant}"
                );
            }
        }
    }
}
