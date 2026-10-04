//! Receiving-plane boundaries rebuilt from placed, compiled asset geometry.
use crate::level_data::{CompiledAssetGeometry, RawElevationLine};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

type Point = [f64; 2];
type Edge = (Point, Point);
const EPS: f64 = 1e-7;

#[derive(Serialize, Deserialize)]
struct Receiver {
    index: u16,
    polygon: Vec<Point>,
    maximum_height: f64,
}

fn subtract(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}
fn cross(a: Point, b: Point) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn interpolate(a: Point, b: Point, t: f64) -> Point {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}
fn contains(polygon: &[Point], p: Point) -> bool {
    let mut inside = false;
    for i in 0..polygon.len() {
        let a = polygon[i];
        let b = polygon[(i + 1) % polygon.len()];
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            inside = !inside;
        }
    }
    inside
}

fn split_at(edge: Edge, other: Edge, cuts: &mut Vec<f64>) {
    if (0..2).any(|i| {
        edge.0[i].max(edge.1[i]) + EPS < other.0[i].min(other.1[i])
            || other.0[i].max(other.1[i]) + EPS < edge.0[i].min(edge.1[i])
    }) {
        return;
    }
    let direction = subtract(edge.1, edge.0);
    let other_direction = subtract(other.1, other.0);
    let offset = subtract(other.0, edge.0);
    let determinant = cross(direction, other_direction);
    let mut add = |t: f64| {
        if t > 0. && t < 1. {
            cuts.push(t);
        }
    };
    if determinant.abs() > EPS {
        let u = cross(offset, direction) / determinant;
        if (-EPS..=1. + EPS).contains(&u) {
            add(cross(offset, other_direction) / determinant);
        }
    } else if cross(offset, direction).abs() < EPS {
        let axis = usize::from(direction[1].abs() > direction[0].abs());
        if direction[axis].abs() > EPS {
            add((other.0[axis] - edge.0[axis]) / direction[axis]);
            add((other.1[axis] - edge.0[axis]) / direction[axis]);
        }
    }
}

fn rounded(point: Point) -> Result<(i16, i16), String> {
    if point.iter().any(|v| {
        !v.is_finite() || v.round() < f64::from(i16::MIN) || v.round() > f64::from(i16::MAX)
    }) {
        return Err("compiled elevation boundary exceeds native coordinate range".into());
    }
    Ok((point[0].round() as i16, point[1].round() as i16))
}

fn projected_coordinate(value: f32) -> f64 {
    f64::from(value)
}

fn side_probe_distance(middle: Point, normal: Point, edges: &[Edge]) -> f64 {
    // Stay inside even a subpixel overlap or gap. Crossing a neighboring edge
    // while sampling would emit duplicate receiver swaps at both boundaries.
    let mut distance = 1e-4f64;
    for &(a, b) in edges {
        if (0..2)
            .any(|i| middle[i] + distance < a[i].min(b[i]) || middle[i] - distance > a[i].max(b[i]))
        {
            continue;
        }
        let direction = subtract(b, a);
        let determinant = cross(normal, direction);
        if determinant.abs() <= EPS {
            continue;
        }
        let offset = subtract(a, middle);
        let along_edge = cross(offset, normal) / determinant;
        let crossing = (cross(offset, direction) / determinant).abs();
        if (-EPS..=1. + EPS).contains(&along_edge) && crossing > EPS {
            distance = distance.min(crossing * 0.25);
        }
    }
    distance
}

fn coordinate_key(value: f64) -> u32 {
    let value = value as f32;
    let bits = if value == 0. { 0 } else { value.to_bits() };
    if bits & 0x8000_0000 != 0 {
        !bits
    } else {
        bits ^ 0x8000_0000
    }
}

fn coordinate_from_key(key: u32) -> f32 {
    f32::from_bits(if key & 0x8000_0000 != 0 {
        key ^ 0x8000_0000
    } else {
        !key
    })
}

/// Only an actual receiver change inside one walkable area creates a bond.
/// Edges are split at every intersection so partial contacts and overlapping
/// planes cannot assign an unrelated receiver to the whole edge.
pub(crate) fn derive(geometry: &CompiledAssetGeometry) -> Result<Vec<RawElevationLine>, String> {
    let mut groups = BTreeMap::<(u16, u16), Vec<Receiver>>::new();
    for (index, obstacle) in geometry.sight_obstacles.iter().enumerate() {
        let Some(topology) = obstacle.projection_area else {
            continue;
        };
        let index = u16::try_from(index)
            .ok()
            .filter(|i| *i != u16::MAX)
            .ok_or("too many receiving surfaces for elevation boundaries")?;
        if obstacle.points.len() < 3
            || obstacle.points.iter().any(|p| {
                !p.x.is_finite()
                    || !p.y.is_finite()
                    || !p.z_top.is_finite()
                    || !p.z_bottom.is_finite()
            })
        {
            return Err(format!(
                "invalid receiving polygon for elevation boundary {index}"
            ));
        }
        groups.entry(topology).or_default().push(Receiver {
            index,
            polygon: obstacle
                .points
                .iter()
                .map(|p| {
                    [
                        projected_coordinate(p.x),
                        projected_coordinate(p.y - p.z_top),
                    ]
                })
                .collect(),
            maximum_height: obstacle
                .points
                .iter()
                .map(|p| f64::from(p.z_top.max(p.z_bottom)))
                .fold(f64::NEG_INFINITY, f64::max),
        });
    }
    let mut area_polygons = BTreeMap::new();
    let mut sector = 0u16;
    for (layer, areas) in geometry.motion_data.layers.iter().enumerate() {
        for area in areas {
            area_polygons.insert(
                (sector, layer as u16),
                area.polygon
                    .points
                    .iter()
                    .map(|p| [f64::from(p.0), f64::from(p.1)])
                    .collect::<Vec<_>>(),
            );
            sector = sector
                .checked_add(
                    u16::try_from(1 + area.obstacles.len())
                        .map_err(|_| "too many motion obstacles")?,
                )
                .ok_or("too many motion sectors")?;
        }
    }
    let mut output = BTreeMap::new();
    let mut ambiguous = BTreeSet::new();
    for ((sector, layer), receivers) in groups {
        // Sight activation controls collision and visibility, not receiving
        // height lookup. Keep bonds for all registered planes in every state;
        // doors and motion obstacles control access to switched surfaces.
        let area = area_polygons
            .get(&(sector, layer))
            .ok_or("elevation receiver has no motion area")?;
        let edges: Vec<Edge> = receivers
            .iter()
            .flat_map(|receiver| {
                (0..receiver.polygon.len()).map(|i| {
                    (
                        receiver.polygon[i],
                        receiver.polygon[(i + 1) % receiver.polygon.len()],
                    )
                })
            })
            .collect();
        let owner = |point| -> Option<u16> {
            if !contains(area, point) {
                return None;
            }
            let mut best: Option<&Receiver> = None;
            for receiver in &receivers {
                if contains(&receiver.polygon, point)
                    && best.is_none_or(|previous| receiver.maximum_height > previous.maximum_height)
                {
                    best = Some(receiver);
                }
            }
            Some(best.map_or(u16::MAX, |r| r.index))
        };
        for edge in &edges {
            let mut cuts = vec![0., 1.];
            for other in &edges {
                split_at(*edge, *other, &mut cuts);
            }
            for i in 0..area.len() {
                split_at(*edge, (area[i], area[(i + 1) % area.len()]), &mut cuts);
            }
            cuts.sort_by(f64::total_cmp);
            // A parameter-space epsilon can erase a distinct float32 endpoint
            // on a long edge, leaving overlapping copies of the shared seam.
            // Collapsed float32 intervals are discarded after interpolation.
            cuts.dedup_by(|a, b| *a == *b);
            for interval in cuts.windows(2) {
                let mut a = interpolate(edge.0, edge.1, interval[0]);
                let mut b = interpolate(edge.0, edge.1, interval[1]);
                if a[0] > b[0] || (a[0] == b[0] && a[1] > b[1]) {
                    std::mem::swap(&mut a, &mut b);
                }
                rounded(a)?;
                rounded(b)?;
                let point_a = a.map(coordinate_key);
                let point_b = b.map(coordinate_key);
                if point_a == point_b {
                    continue;
                }
                let direction = subtract(b, a);
                let length = direction[0].hypot(direction[1]);
                let unit_normal = [-direction[1] / length, direction[0] / length];
                let middle = interpolate(a, b, 0.5);
                // A fixed probe can jump across a thin overlap or gap and emit
                // duplicate receiver swaps for its two nearby boundaries.
                let distance = side_probe_distance(middle, unit_normal, &edges);
                let normal = unit_normal.map(|value| value * distance);
                let left = owner([middle[0] + normal[0], middle[1] + normal[1]]);
                let right = owner([middle[0] - normal[0], middle[1] - normal[1]]);
                let (Some(left), Some(right)) = (left, right) else {
                    continue;
                };
                if left == right {
                    continue;
                }
                let key = (layer, point_a, point_b);
                if ambiguous.contains(&key) {
                    continue;
                }
                if let Some(previous) = output.insert(key, (left, right)) {
                    if previous != (left, right) {
                        // Splitting almost-touching edges can collapse a ground
                        // sliver to one float32 segment. Compose its two swaps
                        // into the direct receiver transition instead of losing
                        // the entire boundary. Conflicting real receivers remain
                        // ambiguous and must not be guessed.
                        let merge = |a, b| {
                            if a == b || b == u16::MAX {
                                Some(a)
                            } else if a == u16::MAX {
                                Some(b)
                            } else {
                                None
                            }
                        };
                        if let (Some(left), Some(right)) =
                            (merge(previous.0, left), merge(previous.1, right))
                        {
                            output.insert(key, (left, right));
                        } else {
                            output.remove(&key);
                            ambiguous.insert(key);
                        }
                    }
                }
            }
        }
    }
    if !ambiguous.is_empty() {
        tracing::warn!(
            count = ambiguous.len(),
            "omitted ambiguous coincident elevation boundaries"
        );
    }
    Ok(output
        .into_iter()
        .filter(|(_, (left, right))| left != right)
        .map(
            |((layer, point_a, point_b), (left_obstacle_index, right_obstacle_index))| {
                let a = point_a.map(coordinate_from_key);
                let b = point_b.map(coordinate_from_key);
                let point_a = (a[0].round() as i16, a[1].round() as i16);
                let point_b = (b[0].round() as i16, b[1].round() as i16);
                let integral = a == [f32::from(point_a.0), f32::from(point_a.1)]
                    && b == [f32::from(point_b.0), f32::from(point_b.1)];
                RawElevationLine {
                    layer,
                    point_a,
                    point_b,
                    precise_points: (!integral).then_some([a, b]),
                    left_obstacle_index,
                    right_obstacle_index,
                }
            },
        )
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> CompiledAssetGeometry {
        let document: serde_json::Value = serde_json::from_slice(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../robin_engine/tests/fixtures/asset-multi-plane-region.level.json"
        )))
        .unwrap();
        serde_json::from_value(document["asset_geometry"].clone()).unwrap()
    }

    #[test]
    fn joined_planes_have_one_shared_elevation_boundary() {
        let lines = derive(&fixture()).unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].point_a, (400, 260));
        assert_eq!(lines[0].point_b, (400, 360));
        assert_eq!(
            (lines[0].left_obstacle_index, lines[0].right_obstacle_index),
            (1, 0)
        );
    }

    #[test]
    fn generated_seams_retain_fractional_endpoints() {
        let mut geometry = fixture();
        geometry.motion_data.layers[0][0].polygon.points =
            vec![(0, 0), (100, 0), (100, 100), (0, 100)];
        for (obstacle, (left, right)) in geometry
            .sight_obstacles
            .iter_mut()
            .zip([(0., 50.25), (50.25, 100.)])
        {
            obstacle.points = [(left, 0.), (right, 0.), (right, 100.), (left, 100.)]
                .into_iter()
                .map(|(x, y)| crate::level_data::RawObstaclePoint {
                    x,
                    y,
                    z_bottom: 0.,
                    z_top: 0.,
                })
                .collect();
        }
        let lines: Vec<_> = derive(&geometry)
            .unwrap()
            .into_iter()
            .filter(|line| {
                line.left_obstacle_index != u16::MAX && line.right_obstacle_index != u16::MAX
            })
            .collect();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].map_endpoints(), [[50.25, 0.], [50.25, 100.]]);
        assert_eq!(lines[0].point_a, (50, 0));
    }

    #[test]
    fn distinct_subpixel_seams_do_not_merge_or_disappear() {
        let mut geometry = fixture();
        geometry.motion_data.layers[0][0].polygon.points =
            vec![(0, 0), (100, 0), (100, 100), (0, 100)];
        geometry
            .sight_obstacles
            .push(geometry.sight_obstacles[0].clone());
        for (obstacle, (left, right)) in geometry.sight_obstacles.iter_mut().zip([
            (0., 50.125),
            (50.125, 50.375),
            (50.375, 100.),
        ]) {
            obstacle.points = [(left, 0.), (right, 0.), (right, 100.), (left, 100.)]
                .into_iter()
                .map(|(x, y)| crate::level_data::RawObstaclePoint {
                    x,
                    y,
                    z_bottom: 0.,
                    z_top: 0.,
                })
                .collect();
        }
        let lines = derive(&geometry).unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].point_a, lines[1].point_a);
        assert_eq!(lines[0].point_b, lines[1].point_b);
        assert_eq!(lines[0].map_endpoints(), [[50.125, 0.], [50.125, 100.]]);
        assert_eq!(lines[1].map_endpoints(), [[50.375, 0.], [50.375, 100.]]);
    }

    #[test]
    fn nearly_coincident_edges_preserve_overlap_and_gap_ownership() {
        for overlap in [false, true] {
            let mut geometry = fixture();
            geometry.motion_data.layers[0][0].polygon.points =
                vec![(0, 0), (100, 0), (100, 100), (0, 100)];
            let boundary = 50.00002;
            let ranges = if overlap {
                [(0., boundary, 2.), (50., 100., 1.)]
            } else {
                [(0., 50., 2.), (boundary, 100., 1.)]
            };
            for (obstacle, (left, right, height)) in geometry.sight_obstacles.iter_mut().zip(ranges)
            {
                obstacle.points = [(left, 0.), (right, 0.), (right, 100.), (left, 100.)]
                    .into_iter()
                    .map(|(x, y)| crate::level_data::RawObstaclePoint {
                        x,
                        y: y + height,
                        z_bottom: height,
                        z_top: height,
                    })
                    .collect();
            }
            let lines = derive(&geometry).unwrap();
            if overlap {
                assert_eq!(lines.len(), 1);
                assert_eq!(lines[0].map_endpoints(), [[boundary, 0.], [boundary, 100.]]);
                assert_ne!(lines[0].left_obstacle_index, u16::MAX);
                assert_ne!(lines[0].right_obstacle_index, u16::MAX);
            } else {
                assert_eq!(lines.len(), 2);
                for line in lines {
                    assert!(
                        line.left_obstacle_index == u16::MAX
                            || line.right_obstacle_index == u16::MAX
                    );
                }
            }
        }
    }

    #[test]
    fn nearly_coincident_endpoints_split_shared_seams_only_once() {
        let mut geometry = fixture();
        geometry.motion_data.layers[0][0].polygon.points =
            vec![(0, 0), (100, 0), (100, 100), (0, 100)];
        for (obstacle, (left, right, top)) in geometry
            .sight_obstacles
            .iter_mut()
            .zip([(0., 50., 0.), (50., 100., 0.000001)])
        {
            obstacle.points = [(left, top), (right, top), (right, 100.), (left, 100.)]
                .into_iter()
                .map(|(x, y)| crate::level_data::RawObstaclePoint {
                    x,
                    y,
                    z_bottom: 0.,
                    z_top: 0.,
                })
                .collect();
        }
        let lines = derive(&geometry).unwrap();
        let shared: Vec<_> = lines
            .iter()
            .filter(|line| {
                line.left_obstacle_index != u16::MAX && line.right_obstacle_index != u16::MAX
            })
            .collect();
        assert_eq!(shared.len(), 1);
        assert_eq!(shared[0].map_endpoints(), [[50., 0.000001], [50., 100.]]);
    }

    #[test]
    fn clipped_ground_sliver_composes_one_representable_receiver_transition() {
        let mut geometry = fixture();
        geometry.motion_data.layers[0][0].polygon.points =
            vec![(1600, 56), (1700, 56), (1700, 110), (1600, 110)];
        for (obstacle, points) in geometry.sight_obstacles.iter_mut().zip([
            [(1689., 90.), (1662., 103.), (1662., 56.)],
            [(1689., 79.), (1689., 90.), (1662., 55.999996)],
        ]) {
            obstacle.points = points
                .into_iter()
                .map(|(x, y)| crate::level_data::RawObstaclePoint {
                    x,
                    y,
                    z_bottom: 0.,
                    z_top: 0.,
                })
                .collect();
        }
        let lines = derive(&geometry).unwrap();
        let shared: Vec<_> = lines
            .iter()
            .filter(|line| {
                line.left_obstacle_index != u16::MAX && line.right_obstacle_index != u16::MAX
            })
            .collect();
        assert_eq!(shared.len(), 1);
        assert_eq!(shared[0].map_endpoints(), [[1662., 56.], [1689., 90.]]);
        assert_eq!(
            (
                shared[0].left_obstacle_index,
                shared[0].right_obstacle_index
            ),
            (0, 1)
        );
    }

    #[test]
    fn partial_contact_splits_receiver_and_ground_boundaries() {
        let mut geometry = fixture();
        geometry.motion_data.layers[0][0].polygon.points =
            vec![(0, 0), (100, 0), (100, 100), (0, 100)];
        for (obstacle, points) in geometry.sight_obstacles.iter_mut().zip([
            vec![(10., 10.), (50., 10.), (50., 90.), (10., 90.)],
            vec![(50., 30.), (90., 30.), (90., 60.), (50., 60.)],
        ]) {
            obstacle.points = points
                .into_iter()
                .map(|(x, y)| crate::level_data::RawObstaclePoint {
                    x,
                    y,
                    z_bottom: 0.,
                    z_top: 0.,
                })
                .collect();
        }
        let lines = derive(&geometry).unwrap();
        let shared: Vec<_> = lines
            .iter()
            .filter(|l| l.left_obstacle_index != u16::MAX && l.right_obstacle_index != u16::MAX)
            .collect();
        assert_eq!(shared.len(), 1);
        assert_eq!((shared[0].point_a, shared[0].point_b), ((50, 30), (50, 60)));
        for endpoints in [((50, 10), (50, 30)), ((50, 60), (50, 90))] {
            assert!(lines.iter().any(|l| (l.point_a, l.point_b) == endpoints
                && (l.left_obstacle_index == u16::MAX || l.right_obstacle_index == u16::MAX)));
        }
    }

    #[test]
    fn sight_switches_do_not_remove_receiving_plane_boundaries() {
        let mut geometry = fixture();
        let expected = serde_json::to_value(derive(&geometry).unwrap()).unwrap();
        // Independent switches may share a navigation area after assets are
        // joined. Neither changes the set of registered receiving planes.
        for index in 0..2 {
            geometry.movement_transitions.push(
                serde_json::from_value(serde_json::json!({
                    "id": format!("walkway/{index}/switch"),
                    "waypoint": [400, 300], "sector": 0, "layer": 0,
                    "active": true, "definitive": false,
                    "apply_polygon": {"points": []}, "no_apply_polygon": {"points": []},
                    "motion_changes": [], "applied_sight": [index]
                }))
                .unwrap(),
            );
            assert_eq!(
                serde_json::to_value(derive(&geometry).unwrap()).unwrap(),
                expected
            );
        }
    }
}
