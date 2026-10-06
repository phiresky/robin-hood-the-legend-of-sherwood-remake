//! Placed stair geometry in ground coordinates, retaining live collision ownership.

use geo::{Area, BooleanOps, Buffer, Closest, ClosestPoint, Intersects, Validation};
use serde::{Deserialize, Serialize};

use crate::level_data::{RawLift, RawMotionArea};
use crate::stair_navigation::StairNavigationPlane;
use crate::stair_navigation_floor::{StairFloorPatch, StairNavigationFloor};

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(deny_unknown_fields)]
pub struct PhysicalStairNavigation {
    /// World floor coefficients: z = a*x + b*y + c.
    pub plane: [f64; 3],
    /// Authoritative piecewise floor when present; `plane` is retained for older
    /// planar descriptors and must not be extrapolated across these patches.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub floor_patches: Vec<StairFloorPatch>,
    pub boundary: Vec<[f32; 2]>,
    pub obstacles: Vec<PhysicalStairObstacle>,
    /// Same local order as the owning lift's doors. Never infer these heights
    /// by inverting the screen projection.
    pub doors: Vec<PhysicalStairDoor>,
}

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(deny_unknown_fields)]
pub struct PhysicalStairObstacle {
    /// Index in the owning motion area's obstacle list. Its state word remains
    /// authoritative; several physical pieces may share one obstacle identity.
    pub motion_obstacle: u16,
    pub polygon: Vec<[f32; 2]>,
}

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(deny_unknown_fields)]
pub struct PhysicalStairDoor {
    pub inside: [f32; 3],
    pub middle: [f32; 3],
    pub outside: [f32; 3],
}

impl PhysicalStairNavigation {
    /// Check ownership and forward projection before runtime geometry binding.
    pub fn validate(&self, lift: &RawLift, area: &RawMotionArea) -> Result<(), String> {
        if !(1..=3).contains(&lift.lift_type) || !area.is_lift {
            return Err("physical navigation requires a stair, ladder or wall motion area".into());
        }
        let plane = StairNavigationPlane::new(self.plane)?;
        let floor = if self.floor_patches.is_empty() {
            None
        } else {
            if lift.lift_type != 1 {
                return Err("piecewise physical floors require a stair".into());
            }
            Some(StairNavigationFloor::new(self.floor_patches.clone())?)
        };
        let check_polygon = |points: &[[f32; 2]]| -> Result<(), String> {
            if points.len() < 3 {
                return Err("physical stair polygon needs at least three vertices".into());
            }
            let mut twice_area = 0.0_f64;
            for (index, &[x, y]) in points.iter().enumerate() {
                let position = plane.world_position([f64::from(x), f64::from(y)])?;
                if position.iter().any(|value| !(*value as f32).is_finite()) {
                    return Err("physical stair position exceeds runtime coordinates".into());
                }
                let [nx, ny] = points[(index + 1) % points.len()];
                twice_area += f64::from(x) * f64::from(ny) - f64::from(nx) * f64::from(y);
            }
            if !twice_area.is_finite() || twice_area.abs() <= 1e-9 {
                return Err("physical stair polygon has no finite area".into());
            }
            let polygon = geo::Polygon::new(
                geo::LineString::from(
                    points
                        .iter()
                        .map(|point| (point[0], point[1]))
                        .collect::<Vec<_>>(),
                ),
                Vec::new(),
            );
            if !polygon.is_valid() {
                return Err("physical stair polygon is not simple".into());
            }
            Ok(())
        };
        check_polygon(&self.boundary)?;
        let mut covered = vec![false; area.obstacles.len()];
        for obstacle in &self.obstacles {
            let covered = covered
                .get_mut(usize::from(obstacle.motion_obstacle))
                .ok_or("physical stair references a missing motion obstacle")?;
            *covered = true;
            check_polygon(&obstacle.polygon)?;
        }
        if covered.contains(&false) {
            return Err("physical stair omits a motion obstacle".into());
        }
        if self.doors.len() != lift.doors.len() {
            return Err("physical stair door identities do not match its lift".into());
        }
        let [low, high] = lift
            .endpoint_doors
            .ok_or("physical stair needs explicit low/high door identities")?;
        let low = self
            .doors
            .get(usize::from(low))
            .ok_or("physical stair low door is missing")?;
        let high = self
            .doors
            .get(usize::from(high))
            .ok_or("physical stair high door is missing")?;
        if low.outside[2] > high.outside[2] {
            return Err("physical stair endpoint heights contradict low/high identities".into());
        }
        let boundary = geo::Polygon::new(
            geo::LineString::from(
                self.boundary
                    .iter()
                    .map(|point| (f64::from(point[0]), f64::from(point[1])))
                    .collect::<Vec<_>>(),
            ),
            Vec::new(),
        );
        if let Some(floor) = &floor {
            let coverage = floor.footprint();
            let scale = self
                .boundary
                .iter()
                .flatten()
                .map(|n| f64::from(*n).abs())
                .fold(1.0_f64, f64::max);
            let tolerance = scale * f64::from(f32::EPSILON) * 2.0;
            let mut required = geo::MultiPolygon::from(vec![boundary.clone()]);
            for obstacle in &self.obstacles {
                if area.obstacles[usize::from(obstacle.motion_obstacle)].state_id != 0 {
                    continue;
                }
                let blocked = geo::Polygon::new(
                    geo::LineString::from(
                        obstacle
                            .polygon
                            .iter()
                            .map(|p| (f64::from(p[0]), f64::from(p[1])))
                            .collect::<Vec<_>>(),
                    ),
                    Vec::new(),
                );
                required = required.difference(&blocked);
            }
            // Allow coordinate rounding only beside actual edges. A total-area
            // allowance could hide a genuine narrow missing-floor notch.
            if required
                .difference(&coverage.buffer(tolerance))
                .unsigned_area()
                > 1e-8
                || coverage
                    .difference(&boundary.buffer(tolerance))
                    .unsigned_area()
                    > 1e-8
            {
                return Err("physical stair patches do not cover its walking boundary".into());
            }
        }
        for (physical, door) in self.doors.iter().zip(&lift.doors) {
            for (world, screen, on_floor) in [
                (physical.inside, door.point_in, true),
                (physical.middle, door.point_mid, true),
                (physical.outside, door.point_out, false),
            ] {
                if world.iter().any(|value| !value.is_finite()) {
                    return Err("physical stair door coordinates must be finite".into());
                }
                let [x, y, z] = world.map(f64::from);
                if (x - f64::from(screen.0)).abs() > 1.0
                    || (y - z - f64::from(screen.1)).abs() > 1.0
                {
                    return Err("physical stair door differs from its projected identity".into());
                }
                let on_plane = if let Some(floor) = &floor {
                    floor
                        .runtime_plane_at([world[0], world[1]])
                        .ok()
                        .and_then(|coefficients| StairNavigationPlane::new(coefficients).ok())
                        .is_some_and(|plane| plane.contains_runtime_position(world))
                } else {
                    plane.contains_runtime_position(world)
                };
                if on_floor && !on_plane {
                    return Err("physical stair door is not on its floor".into());
                }
                if on_floor && !contains_rounded_anchor(&boundary, [world[0], world[1]]) {
                    return Err("physical stair door is outside its boundary".into());
                }
            }
        }
        Ok(())
    }
}

fn contains_rounded_anchor(boundary: &geo::Polygon<f64>, point: [f32; 2]) -> bool {
    let point = geo::Point::new(f64::from(point[0]), f64::from(point[1]));
    if boundary.intersects(&point) {
        return true;
    }
    let Closest::SinglePoint(closest) = boundary.closest_point(&point) else {
        return false;
    };
    // Independently encoded f32 vertices and anchors can round to opposite
    // sides of the same edge. Match the runtime route query's error budget.
    let tolerance = point.x().abs().max(point.y().abs()).max(1.0) * f64::from(f32::EPSILON) * 2.0;
    (closest.x() - point.x()).hypot(closest.y() - point.y()) <= tolerance
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundary_validation_accepts_coordinate_rounding_but_not_missing_floor() {
        let boundary = geo::Polygon::new(
            geo::LineString::from(vec![
                (1000., 2000.),
                (1100., 2033.),
                (1100., 2100.),
                (1000., 2100.),
            ]),
            vec![],
        );
        let x = 1037.1234_f32;
        let y = (2000. + (f64::from(x) - 1000.) * 0.33) as f32;
        assert!(contains_rounded_anchor(&boundary, [x, y]));
        assert!(contains_rounded_anchor(&boundary, [x, y - 0.000244140625]));
        assert!(!contains_rounded_anchor(&boundary, [x, y - 0.01]));
        assert!(!contains_rounded_anchor(&boundary, [x, y - 0.125]));
    }
}
