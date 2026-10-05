//! Placed stair geometry in ground coordinates, retaining live collision ownership.

use geo::{Closest, ClosestPoint, Intersects, Validation};
use serde::{Deserialize, Serialize};

use crate::level_data::{RawLift, RawMotionArea};
use crate::stair_navigation::StairNavigationPlane;

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
        if !(1..=2).contains(&lift.lift_type) || !area.is_lift {
            return Err("physical navigation requires a stair or ladder motion area".into());
        }
        let plane = StairNavigationPlane::new(self.plane)?;
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
                if on_floor && !plane.contains_runtime_position(world) {
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
