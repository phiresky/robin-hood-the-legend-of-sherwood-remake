//! Actor-sized approach points for passages in compiled asset geometry.
use crate::level_data::{CompiledAssetGeometry, RawMotionArea};
use std::collections::BTreeMap;

type Point = (i16, i16);

fn contains(polygon: &[Point], point: [f64; 2]) -> bool {
    let mut inside = false;
    for i in 0..polygon.len() {
        let a = polygon[i];
        let b = polygon[(i + 1) % polygon.len()];
        let (ax, ay, bx, by) = (
            f64::from(a.0),
            f64::from(a.1),
            f64::from(b.0),
            f64::from(b.1),
        );
        if (ay > point[1]) != (by > point[1])
            && point[0] < (bx - ax) * (point[1] - ay) / (by - ay) + ax
        {
            inside = !inside;
        }
    }
    inside
}

fn edge_hits_box(a: Point, b: Point, center: [f64; 2]) -> bool {
    let a = [f64::from(a.0), f64::from(a.1)];
    let b = [f64::from(b.0), f64::from(b.1)];
    let mut low = 0f64;
    let mut high = 1f64;
    for axis in 0..2 {
        let radius = [6., 3.][axis];
        let delta = b[axis] - a[axis];
        let minimum = center[axis] - radius - a[axis];
        let maximum = center[axis] + radius - a[axis];
        if delta == 0. {
            if minimum > 0. || maximum < 0. {
                return false;
            }
        } else {
            low = low.max((minimum / delta).min(maximum / delta));
            high = high.min((minimum / delta).max(maximum / delta));
        }
    }
    low <= high
}

fn touches_boundary(polygon: &[Point], point: [f64; 2]) -> bool {
    (0..polygon.len()).any(|i| edge_hits_box(polygon[i], polygon[(i + 1) % polygon.len()], point))
}

fn fits(area: &RawMotionArea, point: [f64; 2]) -> bool {
    contains(&area.polygon.points, point)
        && !touches_boundary(&area.polygon.points, point)
        && area.obstacles.iter().all(|obstacle| {
            !contains(&obstacle.polygon.points, point)
                && !touches_boundary(&obstacle.polygon.points, point)
        })
}

fn crosses_boundary(polygon: &[Point], source: [f64; 2], goal: [f64; 2]) -> bool {
    let cross = |a: [f64; 2], b: [f64; 2]| a[0] * b[1] - a[1] * b[0];
    let direction = [goal[0] - source[0], goal[1] - source[1]];
    (0..polygon.len()).any(|i| {
        let a = polygon[i];
        let b = polygon[(i + 1) % polygon.len()];
        let edge = [
            f64::from(b.0) - f64::from(a.0),
            f64::from(b.1) - f64::from(a.1),
        ];
        let offset = [f64::from(a.0) - source[0], f64::from(a.1) - source[1]];
        let determinant = cross(direction, edge);
        if determinant == 0. {
            return false;
        }
        let t = cross(offset, edge) / determinant;
        let u = cross(offset, direction) / determinant;
        t > 0. && t <= 1. && (0. ..=1.).contains(&u)
    })
}

fn approach(area: &RawMotionArea, point: Point, middle: Point) -> Option<Point> {
    let source = [f64::from(point.0), f64::from(point.1)];
    if fits(area, source) {
        return Some(point);
    }
    let delta = [
        source[0] - f64::from(middle.0),
        source[1] - f64::from(middle.1),
    ];
    let length = delta[0].hypot(delta[1]);
    if length == 0. {
        return None;
    }
    // Prefer extending the authored direction, retaining established approach
    // alignment wherever it provides sufficient clearance.
    for distance in 0..=64 {
        let candidate = [
            (source[0] + delta[0] * f64::from(distance) / length).round(),
            (source[1] + delta[1] * f64::from(distance) / length).round(),
        ];
        if candidate
            .iter()
            .any(|v| *v < f64::from(i16::MIN) || *v > f64::from(i16::MAX))
            || !contains(&area.polygon.points, candidate)
            || crosses_boundary(&area.polygon.points, source, candidate)
            || area.obstacles.iter().any(|obstacle| {
                contains(&obstacle.polygon.points, candidate)
                    || touches_boundary(&obstacle.polygon.points, candidate)
            })
        {
            break;
        }
        if fits(area, candidate) {
            return Some((candidate[0] as i16, candidate[1] as i16));
        }
    }
    if !contains(&area.polygon.points, source) {
        return None;
    }
    // A slanted narrow stair may never fit an actor on that ray. Find the
    // nearest forward/sideways adjustment with a clear center segment in the
    // same polygon. Never tunnel through a wall or another part of the area.
    let mut offsets: Vec<_> = (-64i32..=64)
        .flat_map(|x| {
            (-64i32..=64).filter_map(move |y| {
                let distance = x * x + y * y;
                (distance > 0
                    && distance <= 64 * 64
                    && f64::from(x) * delta[0] + f64::from(y) * delta[1] >= 0.)
                    .then_some((distance, x, y))
            })
        })
        .collect();
    offsets.sort_unstable();
    for (_, x, y) in offsets {
        let candidate = [source[0] + f64::from(x), source[1] + f64::from(y)];
        if candidate
            .iter()
            .any(|v| *v < f64::from(i16::MIN) || *v > f64::from(i16::MAX))
            || !fits(area, candidate)
            || crosses_boundary(&area.polygon.points, source, candidate)
            || area.obstacles.iter().any(|obstacle| {
                contains(&obstacle.polygon.points, source)
                    || crosses_boundary(&obstacle.polygon.points, source, candidate)
            })
        {
            continue;
        }
        return Some((candidate[0] as i16, candidate[1] as i16));
    }
    None
}

/// Passage animations can reach points too close to a boundary for ordinary
/// walking to resume. Give walking lifts a stock 6-by-3 actor footprint on both
/// sides, retaining the authored midpoint and every permission/state link.
pub(crate) fn derive(geometry: &mut CompiledAssetGeometry) {
    let mut areas = BTreeMap::new();
    let mut sector = 0usize;
    for (layer, entries) in geometry.motion_data.layers.iter().enumerate() {
        for area in entries {
            areas.insert((sector, layer), area);
            sector += 1 + area.obstacles.len();
        }
    }
    for (lift_index, lift) in geometry.lifts.iter_mut().enumerate() {
        if !matches!(lift.lift_type, 0..=3) {
            continue;
        }
        for (door_index, door) in lift.doors.iter_mut().enumerate() {
            for (side, point, sector, layer) in [
                ("inside", &mut door.point_in, door.sector_in, door.layer_in),
                (
                    "outside",
                    &mut door.point_out,
                    door.sector_out,
                    door.layer_out,
                ),
            ] {
                let Some(area) = areas.get(&(usize::from(sector), usize::from(layer))) else {
                    continue; // Topology validation reports missing areas separately.
                };
                if side == "inside" && lift.lift_type == 3 && matches!(door.door_type, 4 | 6) {
                    // Wall-top approaches are fixed by the climb animation.
                    // Extending the authored ray would be undone at runtime.
                    let adapted = crate::level_data::offset_door_approach(
                        [f32::from(point.0), f32::from(point.1)],
                        [f32::from(door.point_mid.0), f32::from(door.point_mid.1)],
                        if door.door_type == 6 { 65. } else { 60. },
                    );
                    if !fits(area, adapted.map(f64::from)) {
                        tracing::warn!(
                            lift_index,
                            door_index,
                            ?adapted,
                            "compiled wall has no actor-sized clearance at its animation approach"
                        );
                    }
                    continue;
                }
                if let Some(adjusted) = approach(area, *point, door.point_mid) {
                    *point = adjusted;
                } else {
                    tracing::warn!(
                        lift_index,
                        door_index,
                        side,
                        ?point,
                        "compiled lift has no actor-sized approach near its authored passage"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn climbing_approaches_leave_room_for_the_actor() {
        let document: serde_json::Value = serde_json::from_slice(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../robin_engine/tests/fixtures/asset-lift.level.json"
        )))
        .unwrap();
        for kind in [2, 3] {
            let mut geometry: CompiledAssetGeometry =
                serde_json::from_value(document["asset_geometry"].clone()).unwrap();
            geometry.lifts[0].lift_type = kind;
            let high_inside = geometry.lifts[0].doors[1].point_in;
            derive(&mut geometry);
            let low = &geometry.lifts[0].doors[0];
            assert!(fits(
                &geometry.motion_data.layers[2][0],
                [f64::from(low.point_in.0), f64::from(low.point_in.1)]
            ));
            assert!(fits(
                &geometry.motion_data.layers[0][0],
                [f64::from(low.point_out.0), f64::from(low.point_out.1)]
            ));
            let high = geometry.lifts[0].doors[1].point_in;
            if kind == 2 {
                assert!(fits(
                    &geometry.motion_data.layers[2][0],
                    [f64::from(high.0), f64::from(high.1)]
                ));
            } else {
                assert_eq!(
                    high, high_inside,
                    "wall-top animation direction remains authored"
                );
            }
        }
    }

    #[test]
    fn narrow_slanted_stair_has_an_actor_sized_approach() {
        let area: RawMotionArea = serde_json::from_value(serde_json::json!({
            "is_lift": true, "state_id": 0, "flags": 0,
            "polygon": {"points": [[2441,301],[2460,313],[2472,349],[2496,419],[2490,415],[2477,407]]},
            "skeleton_segments": [], "obstacles": []
        })).unwrap();
        let adjusted = approach(&area, (2454, 314), (2452, 307)).expect("nearby clearance");
        assert_ne!(adjusted, (2454, 314));
        assert!(fits(&area, [f64::from(adjusted.0), f64::from(adjusted.1)]));
    }

    #[test]
    fn approach_stays_in_its_area_and_does_not_skip_blockers() {
        let mut area: RawMotionArea = serde_json::from_value(serde_json::json!({
            "is_lift": true, "state_id": 0, "flags": 0,
            "polygon": {"points": [[0,0],[100,0],[100,100],[0,100]]},
            "skeleton_segments": [], "obstacles": []
        }))
        .unwrap();
        assert_eq!(approach(&area, (2, 50), (0, 50)), Some((7, 50)));
        assert_eq!(approach(&area, (20, 50), (0, 50)), Some((20, 50)));
        assert_eq!(approach(&area, (2, 2), (0, 2)), Some((7, 4)));
        area.obstacles.push(
            serde_json::from_value(serde_json::json!({
                "state_id": 0, "polygon": {"points": [[10,0],[11,0],[11,100],[10,100]]}
            }))
            .unwrap(),
        );
        assert_eq!(approach(&area, (2, 50), (0, 50)), None);
        assert_eq!(approach(&area, (2, 2), (0, 2)), None);
    }
}
