//! Continuous physical movement across connected planar stair flights.

use geo::{
    Area, BooleanOps, Closest, ClosestPoint, Intersects, Line, LineString, Polygon, Validation,
};
use serde::{Deserialize, Serialize};

use crate::stair_navigation::{StairNavigationPlane, StairRouteStep};

#[derive(
    Debug,
    Clone,
    PartialEq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(deny_unknown_fields)]
pub struct StairFloorPatch {
    pub plane: [f64; 3],
    pub boundary: Vec<[f64; 2]>,
}

/// Height conversion only. Live collision and actor-sized route authorization
/// remain the owning stair's responsibility.
#[derive(Debug, Clone, Serialize)]
pub struct StairNavigationFloor {
    patches: Vec<StairFloorPatch>,
}

impl<'de> Deserialize<'de> for StairNavigationFloor {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Definition {
            patches: Vec<StairFloorPatch>,
        }
        let definition = Definition::deserialize(deserializer)?;
        Self::new(definition.patches).map_err(serde::de::Error::custom)
    }
}

fn polygon(points: &[[f64; 2]]) -> Polygon<f64> {
    Polygon::new(LineString::from(points.to_vec()), Vec::new())
}

fn plane(patch: &StairFloorPatch) -> StairNavigationPlane {
    StairNavigationPlane::new(patch.plane).expect("validated stair floor plane")
}

fn edges(points: &[[f64; 2]]) -> impl Iterator<Item = Line<f64>> + '_ {
    points
        .iter()
        .enumerate()
        .map(|(index, point)| Line::new(*point, points[(index + 1) % points.len()]))
}

fn intersection_points(a: Line<f64>, b: Line<f64>) -> Vec<[f64; 2]> {
    use geo::line_intersection::{LineIntersection, line_intersection};
    match line_intersection(a, b) {
        Some(LineIntersection::SinglePoint { intersection, .. }) => {
            vec![[intersection.x, intersection.y]]
        }
        Some(LineIntersection::Collinear { intersection }) => vec![
            [intersection.start.x, intersection.start.y],
            [intersection.end.x, intersection.end.y],
        ],
        None => Vec::new(),
    }
}

impl StairNavigationFloor {
    pub fn new(patches: Vec<StairFloorPatch>) -> Result<Self, String> {
        if patches.is_empty() {
            return Err("physical stair needs at least one floor patch".into());
        }
        let mut polygons = Vec::with_capacity(patches.len());
        for patch in &patches {
            let plane = StairNavigationPlane::new(patch.plane)?;
            if patch.boundary.len() < 3 {
                return Err("stair floor patch needs at least three vertices".into());
            }
            for &point in &patch.boundary {
                plane.world_position(point)?;
            }
            let polygon = polygon(&patch.boundary);
            let area = polygon.unsigned_area();
            if !polygon.is_valid() || !area.is_finite() || area <= 1e-9 {
                return Err("stair floor patch must be a simple nonempty polygon".into());
            }
            polygons.push(polygon);
        }
        for (index, patch) in patches.iter().enumerate() {
            for (other_index, other) in patches[..index].iter().enumerate() {
                // A linear height difference reaches its extremes at intersection
                // vertices: test crossing edges and vertices inside the overlap.
                let points =
                    patch
                        .boundary
                        .iter()
                        .copied()
                        .filter(|point| polygons[other_index].intersects(&geo::Point::from(*point)))
                        .chain(
                            other.boundary.iter().copied().filter(|point| {
                                polygons[index].intersects(&geo::Point::from(*point))
                            }),
                        )
                        .chain(edges(&patch.boundary).flat_map(|a| {
                            edges(&other.boundary).flat_map(move |b| intersection_points(a, b))
                        }));
                for point in points {
                    let height = plane(patch).world_position(point)?[2];
                    let other_height = plane(other).world_position(point)?[2];
                    if (height - other_height).abs() > 1e-4 {
                        return Err(
                            "stair floor patches disagree at their shared boundary or overlap"
                                .into(),
                        );
                    }
                }
            }
        }
        let mut joined = geo::MultiPolygon::from(vec![polygons[0].clone()]);
        for polygon in &polygons[1..] {
            joined = joined.union(polygon);
        }
        if joined.0.len() != 1 {
            return Err("stair floor patches must form one connected floor".into());
        }
        Ok(Self { patches })
    }

    fn patch_at(&self, point: [f64; 2]) -> Result<&StairFloorPatch, &'static str> {
        self.patch_at_with_rounding(point, false)
    }

    fn patch_at_with_rounding(
        &self,
        point: [f64; 2],
        runtime: bool,
    ) -> Result<&StairFloorPatch, &'static str> {
        if point.iter().any(|value| !value.is_finite()) {
            return Err("stair floor query must be finite");
        }
        let query = geo::Point::from(point);
        if let Some(patch) = self
            .patches
            .iter()
            .find(|patch| polygon(&patch.boundary).intersects(&query))
        {
            return Ok(patch);
        }
        if runtime {
            let tolerance =
                point[0].abs().max(point[1].abs()).max(1.0) * f64::from(f32::EPSILON) * 2.0;
            if let Some(patch) = self.patches.iter().find(|patch| {
                let Closest::SinglePoint(nearest) = polygon(&patch.boundary).closest_point(&query)
                else {
                    return false;
                };
                (nearest.x() - point[0]).hypot(nearest.y() - point[1]) <= tolerance
            }) {
                return Ok(patch);
            }
        }
        Err("stair route point has no floor support")
    }

    pub fn runtime_plane_at(&self, point: [f32; 2]) -> Result<[f64; 3], &'static str> {
        Ok(self
            .patch_at_with_rounding(point.map(f64::from), true)?
            .plane)
    }

    pub fn footprint(&self) -> geo::MultiPolygon<f64> {
        let mut joined = geo::MultiPolygon::from(vec![polygon(&self.patches[0].boundary)]);
        for patch in &self.patches[1..] {
            joined = joined.union(&polygon(&patch.boundary));
        }
        joined
    }

    /// Candidate inverses remain ambiguous when different flights overlap in
    /// screen space, even if each plane individually has an inverse.
    pub fn world_point_from_screen(&self, screen: [f32; 2]) -> Option<[f32; 3]> {
        let mut result: Option<[f32; 3]> = None;
        for patch in &self.patches {
            let [a, b, c] = patch.plane;
            let x = f64::from(screen[0]);
            if (1.0 - b).abs() < 1e-8 {
                let min_x = patch
                    .boundary
                    .iter()
                    .map(|p| p[0])
                    .fold(f64::INFINITY, f64::min);
                let max_x = patch
                    .boundary
                    .iter()
                    .map(|p| p[0])
                    .fold(f64::NEG_INFINITY, f64::max);
                if x >= min_x && x <= max_x && (f64::from(screen[1]) + a * x + c).abs() <= 0.001 {
                    return None;
                }
                continue;
            }
            let y = (f64::from(screen[1]) + a * x + c) / (1.0 - b);
            if !polygon(&patch.boundary).intersects(&geo::Point::new(x, y)) {
                continue;
            }
            let world = plane(patch).world_position([x, y]).ok()?.map(|n| n as f32);
            if world.iter().any(|n| !n.is_finite()) {
                return None;
            }
            if let Some(previous) = result {
                if previous
                    .iter()
                    .zip(world)
                    .any(|(a, b)| (*a - b).abs() > 0.001)
                {
                    return None;
                }
            }
            result = Some(world);
        }
        result
    }

    pub fn world_position(&self, point: [f64; 2]) -> Result<[f64; 3], &'static str> {
        plane(self.patch_at(point)?).world_position(point)
    }

    /// Split at every crossed patch edge, including collinear shared boundaries.
    /// Never authorize a shortcut across a hole merely because its ends are valid.
    fn segments(
        &self,
        from: [f64; 2],
        to: [f64; 2],
        runtime: bool,
    ) -> Result<Vec<(StairNavigationPlane, [f64; 2], [f64; 2])>, &'static str> {
        self.patch_at_with_rounding(from, runtime)?;
        self.patch_at_with_rounding(to, runtime)?;
        if from == to {
            return Ok(Vec::new());
        }
        let delta = [to[0] - from[0], to[1] - from[1]];
        if delta.iter().any(|value| !value.is_finite()) {
            return Err("stair route displacement must be finite");
        }
        let axis = usize::from(delta[1].abs() > delta[0].abs());
        let route = Line::new(from, to);
        let mut cuts = vec![0.0, 1.0];
        for patch in &self.patches {
            for edge in edges(&patch.boundary) {
                for point in intersection_points(route, edge) {
                    cuts.push(((point[axis] - from[axis]) / delta[axis]).clamp(0.0, 1.0));
                }
            }
        }
        cuts.sort_by(f64::total_cmp);
        cuts.dedup();
        let at = |t: f64| {
            if t == 0.0 {
                from
            } else if t == 1.0 {
                to
            } else {
                [from[0] + t * delta[0], from[1] + t * delta[1]]
            }
        };
        cuts.windows(2)
            .map(|pair| {
                let patch = self.patch_at_with_rounding(at((pair[0] + pair[1]) * 0.5), runtime)?;
                Ok((plane(patch), at(pair[0]), at(pair[1])))
            })
            .collect()
    }

    pub fn route_distance(&self, from: [f64; 2], to: [f64; 2]) -> Result<f64, &'static str> {
        self.route_distance_with_rounding(from, to, false)
    }

    pub fn route_distance_runtime(
        &self,
        from: [f32; 2],
        to: [f32; 2],
    ) -> Result<f64, &'static str> {
        self.route_distance_with_rounding(from.map(f64::from), to.map(f64::from), true)
    }

    fn route_distance_with_rounding(
        &self,
        from: [f64; 2],
        to: [f64; 2],
        runtime: bool,
    ) -> Result<f64, &'static str> {
        self.segments(from, to, runtime)?
            .into_iter()
            .try_fold(0.0, |sum, (plane, from, to)| {
                let distance = sum + plane.route_distance(from, to)?;
                if !distance.is_finite() {
                    return Err("stair route distance must be finite");
                }
                Ok(distance)
            })
    }

    /// Spend the movement budget on each flight's actual 3D length, carrying any
    /// remaining distance across the seam without snapping or losing a tick.
    pub fn advance(
        &self,
        from: [f64; 2],
        to: [f64; 2],
        distance: f64,
    ) -> Result<StairRouteStep, &'static str> {
        self.advance_with_rounding(from, to, distance, false)
    }

    pub fn advance_runtime(
        &self,
        from: [f32; 2],
        to: [f32; 2],
        distance: f64,
    ) -> Result<StairRouteStep, &'static str> {
        self.advance_with_rounding(from.map(f64::from), to.map(f64::from), distance, true)
    }

    fn advance_with_rounding(
        &self,
        from: [f64; 2],
        to: [f64; 2],
        distance: f64,
        runtime: bool,
    ) -> Result<StairRouteStep, &'static str> {
        if !distance.is_finite() || distance < 0.0 {
            return Err("stair step distance must be finite and nonnegative");
        }
        let mut remaining = distance;
        for (plane, start, end) in self.segments(from, to, runtime)? {
            let length = plane.route_distance(start, end)?;
            if remaining < length {
                let mut step = plane.advance(start, end, remaining)?;
                step.reached = false;
                return Ok(step);
            }
            remaining -= length;
        }
        let world = plane(self.patch_at_with_rounding(to, runtime)?).world_position(to)?;
        let screen = [world[0], world[1] - world[2]];
        if screen.iter().any(|value| !value.is_finite()) {
            return Err("stair route point must produce a finite screen position");
        }
        Ok(StairRouteStep {
            ground: to,
            world,
            screen,
            reached: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(y0: f64, y1: f64, slope: f64, offset: f64) -> StairFloorPatch {
        StairFloorPatch {
            plane: [0.0, slope, offset],
            boundary: vec![[0.0, y0], [20.0, y0], [20.0, y1], [0.0, y1]],
        }
    }

    #[test]
    fn steps_preserve_distance_and_height_across_different_slopes_in_both_directions() {
        let floor = StairNavigationFloor::new(vec![
            patch(0.0, 10.0, 1.0, 0.0),
            patch(10.0, 20.0, 2.0, -10.0),
        ])
        .unwrap();
        let expected = 8.0 * 2.0_f64.sqrt() + 8.0 * 5.0_f64.sqrt();
        for (from, to) in [([10.0, 2.0], [10.0, 18.0]), ([10.0, 18.0], [10.0, 2.0])] {
            assert!((floor.route_distance(from, to).unwrap() - expected).abs() < 1e-10);
            let mut at = from;
            let mut travelled = 0.0;
            for _ in 0..100 {
                let step = floor.advance(at, to, 3.0).unwrap();
                let distance = floor.route_distance(at, step.ground).unwrap();
                assert!(distance > 0.0 && distance <= 3.0 + 1e-10);
                travelled += distance;
                assert_eq!(step.world, floor.world_position(step.ground).unwrap());
                at = step.ground;
                if step.reached {
                    break;
                }
            }
            assert_eq!(at, to);
            assert!((travelled - expected).abs() < 1e-10);
        }
        let step = floor
            .advance([10.0, 9.0], [10.0, 18.0], 2.0_f64.sqrt() + 5.0_f64.sqrt())
            .unwrap();
        assert_eq!(step.world, [10.0, 11.0, 12.0]);
        assert!(!step.reached);
    }

    #[test]
    fn floor_rejects_disconnected_discontinuous_and_ambiguous_patches() {
        for second in [
            patch(11.0, 20.0, 2.0, -12.0),
            patch(10.0, 20.0, 2.0, -9.0),
            patch(5.0, 20.0, 2.0, -10.0),
        ] {
            assert!(StairNavigationFloor::new(vec![patch(0.0, 10.0, 1.0, 0.0), second]).is_err());
        }
        assert!(StairNavigationFloor::new(Vec::new()).is_err());
        assert!(StairNavigationFloor::new(vec![patch(0.0, 10.0, f64::NAN, 0.0)]).is_err());
    }

    #[test]
    fn rotated_elevated_copies_preserve_the_same_piecewise_distance() {
        for angle in [0.0_f64, 37.0, 90.0, 180.0] {
            let (sin, cos) = angle.to_radians().sin_cos();
            for (dx, dy, dz) in [(1000.25, 2300.125, 0.0), (-1700.5, 3200.75, 40.0)] {
                let place = |[x, y]: [f64; 2]| [cos * x - sin * y + dx, sin * x + cos * y + dy];
                let patches = [patch(0.0, 10.0, 1.0, 0.0), patch(10.0, 20.0, 2.0, -10.0)]
                    .into_iter()
                    .map(|patch| {
                        let [a, b, c] = patch.plane;
                        let a1 = a * cos - b * sin;
                        let b1 = a * sin + b * cos;
                        StairFloorPatch {
                            plane: [a1, b1, c + dz - a1 * dx - b1 * dy],
                            boundary: patch.boundary.into_iter().map(place).collect(),
                        }
                    })
                    .collect();
                let floor = StairNavigationFloor::new(patches).unwrap();
                for (start, end) in [([10.0, 2.0], [10.0, 18.0]), ([10.0, 18.0], [10.0, 2.0])] {
                    let from = place(start);
                    let to = place(end);
                    let length = floor.route_distance(from, to).unwrap();
                    let expected = 8.0 * (2.0_f64.sqrt() + 5.0_f64.sqrt());
                    assert!((length - expected).abs() < 1e-8, "angle {angle}: {length}");
                    let mut at = from;
                    let mut travelled = 0.0;
                    for _ in 0..100 {
                        let step = floor.advance(at, to, 3.0).unwrap();
                        let distance = floor.route_distance(at, step.ground).unwrap();
                        assert!(distance > 0.0 && distance <= 3.0 + 1e-8);
                        travelled += distance;
                        at = step.ground;
                        if step.reached {
                            break;
                        }
                    }
                    assert_eq!(at, to);
                    assert!((travelled - expected).abs() < 1e-8);
                }
            }
        }
    }

    #[test]
    fn collinear_seam_queries_and_overlapping_coplanar_patches_are_consistent() {
        let floor = StairNavigationFloor::new(vec![
            patch(0.0, 10.0, 1.0, 0.0),
            patch(5.0, 10.0, 1.0, 0.0),
            patch(10.0, 20.0, 2.0, -10.0),
        ])
        .unwrap();
        assert_eq!(
            floor.route_distance([2.0, 10.0], [18.0, 10.0]).unwrap(),
            16.0
        );
        assert_eq!(
            floor.advance([2.0, 10.0], [18.0, 10.0], 3.0).unwrap().world,
            [5.0, 10.0, 10.0]
        );
        let step = floor
            .advance([10.0, 2.0], [10.0, 18.0], 8.0 * 2.0_f64.sqrt())
            .unwrap();
        assert_eq!(step.world, [10.0, 10.0, 10.0]);
        assert!(!step.reached);
        assert!(
            floor
                .advance([10.0, 2.0], [10.0, 18.0], f64::INFINITY)
                .is_err()
        );
    }

    #[test]
    fn inverse_rejects_overlapping_and_edge_on_flights() {
        let floor = StairNavigationFloor::new(vec![
            patch(0.0, 10.0, 0.5, 0.0),
            patch(10.0, 20.0, 1.5, -10.0),
        ])
        .unwrap();
        assert!(floor.world_point_from_screen([10.0, 2.0]).is_none());
        let floor = StairNavigationFloor::new(vec![
            patch(0.0, 10.0, 1.0, 0.0),
            patch(10.0, 20.0, 0.5, 5.0),
        ])
        .unwrap();
        assert!(floor.world_point_from_screen([10.0, 0.0]).is_none());
        assert_eq!(
            floor.world_point_from_screen([10.0, 2.0]),
            Some([10.0, 14.0, 12.0])
        );
        assert!(floor.runtime_plane_at([25.0, 15.0]).is_err());
    }

    #[test]
    fn concave_floor_does_not_authorize_a_segment_across_missing_support() {
        let floor = StairNavigationFloor::new(vec![StairFloorPatch {
            plane: [0.0, 0.0, 0.0],
            boundary: vec![
                [0.0, 0.0],
                [20.0, 0.0],
                [20.0, 10.0],
                [10.0, 10.0],
                [10.0, 20.0],
                [0.0, 20.0],
            ],
        }])
        .unwrap();
        assert!(floor.advance([19.0, 9.0], [9.0, 19.0], 100.0).is_err());
        assert!(floor.world_position([15.0, 15.0]).is_err());
        assert!(floor.advance([5.0, 5.0], [5.0, 5.0], 0.0).unwrap().reached);
        assert!(!floor.advance([5.0, 5.0], [5.0, 15.0], 0.0).unwrap().reached);
        assert!(floor.advance([5.0, 5.0], [5.0, 15.0], -1.0).is_err());
        let encoded = serde_json::to_vec(&floor).unwrap();
        let decoded: StairNavigationFloor = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(
            decoded.world_position([5.0, 5.0]),
            floor.world_position([5.0, 5.0])
        );
        assert!(serde_json::from_str::<StairNavigationFloor>(r#"{"patches":[]}"#).is_err());
    }
}
