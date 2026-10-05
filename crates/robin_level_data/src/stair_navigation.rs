//! Physical stair coordinates, independent of the rendering projection.
//!
//! A screen-space stair can collapse to a line while its physical walking
//! surface remains valid. Retain ground XY as the navigation coordinate and
//! evaluate height forward; never recover progress by inverting screen Y.

use serde::{Deserialize, Serialize};

/// A planar floor in game-world units: `z = a*x + b*y + c`.
///
/// This is only the coordinate conversion for physical stair navigation.
/// Bounds, collision, landing support and actor traversal are separate concerns.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct StairNavigationPlane {
    coefficients: [f64; 3],
}

/// One movement step along an already collision-checked physical route segment.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StairRouteStep {
    pub ground: [f64; 2],
    pub world: [f64; 3],
    pub screen: [f64; 2],
    pub reached: bool,
}

impl<'de> Deserialize<'de> for StairNavigationPlane {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Definition {
            coefficients: [f64; 3],
        }
        let definition = Definition::deserialize(deserializer)?;
        Self::new(definition.coefficients).map_err(serde::de::Error::custom)
    }
}

impl StairNavigationPlane {
    pub fn new(coefficients: [f64; 3]) -> Result<Self, &'static str> {
        if coefficients.iter().any(|value| !value.is_finite()) {
            return Err("stair plane coefficients must be finite");
        }
        Ok(Self { coefficients })
    }

    /// Resolve a route point without dividing by the projection determinant.
    pub fn world_position(&self, ground: [f64; 2]) -> Result<[f64; 3], &'static str> {
        let [x, y] = ground;
        let [a, b, c] = self.coefficients;
        let z = a * x + b * y + c;
        if !x.is_finite() || !y.is_finite() || !z.is_finite() {
            return Err("stair route point must produce a finite world position");
        }
        Ok([x, y, z])
    }

    pub fn screen_position(&self, ground: [f64; 2]) -> Result<[f64; 2], &'static str> {
        let [x, y, z] = self.world_position(ground)?;
        let screen_y = y - z;
        if !screen_y.is_finite() {
            return Err("stair route point must produce a finite screen position");
        }
        Ok([x, screen_y])
    }

    /// Physical progress remains measurable even when screen displacement is zero.
    pub fn route_distance(&self, from: [f64; 2], to: [f64; 2]) -> Result<f64, &'static str> {
        let from = self.world_position(from)?;
        let to = self.world_position(to)?;
        let distance = (to[0] - from[0])
            .hypot(to[1] - from[1])
            .hypot(to[2] - from[2]);
        if !distance.is_finite() {
            return Err("stair route distance must be finite");
        }
        Ok(distance)
    }

    /// Advance by world distance, including when the route projects to a point.
    /// The caller must validate the segment against physical navigation first.
    pub fn advance(
        &self,
        from: [f64; 2],
        to: [f64; 2],
        distance: f64,
    ) -> Result<StairRouteStep, &'static str> {
        if !distance.is_finite() || distance < 0.0 {
            return Err("stair step distance must be finite and nonnegative");
        }
        let remaining = self.route_distance(from, to)?;
        let reached = remaining <= distance;
        let ground = if reached {
            to
        } else {
            let fraction = distance / remaining;
            [
                from[0] + (to[0] - from[0]) * fraction,
                from[1] + (to[1] - from[1]) * fraction,
            ]
        };
        Ok(StairRouteStep {
            ground,
            world: self.world_position(ground)?,
            screen: self.screen_position(ground)?,
            reached,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_on_stair_retains_world_progress_when_screen_points_coincide() {
        let plane = StairNavigationPlane::new([0.0, 1.0, 40.0]).unwrap();
        let from = [12.0, 30.0];
        let to = [12.0, 80.0];
        assert_eq!(plane.screen_position(from).unwrap(), [12.0, -40.0]);
        assert_eq!(plane.screen_position(from), plane.screen_position(to));
        assert_eq!(plane.world_position(from).unwrap(), [12.0, 30.0, 70.0]);
        assert_eq!(plane.world_position(to).unwrap(), [12.0, 80.0, 120.0]);
        assert!((plane.route_distance(from, to).unwrap() - 50.0_f64.hypot(50.0)).abs() < 1e-12);
    }

    #[test]
    fn projected_direction_can_reverse_without_reversing_physical_progress() {
        for slope in [0.0, 1.0 - 1e-12, 1.0, 1.0 + 1e-12, 2.0] {
            let plane = StairNavigationPlane::new([0.25, slope, 80.0]).unwrap();
            let start = [20.0, 30.0];
            let end = [20.0, 60.0];
            assert!(
                (plane.route_distance(start, end).unwrap() - 30.0_f64.hypot(30.0 * slope)).abs()
                    < 1e-10
            );
            let delta =
                plane.screen_position(end).unwrap()[1] - plane.screen_position(start).unwrap()[1];
            assert!((delta - 30.0 * (1.0 - slope)).abs() < 1e-10);
            let bytes = serde_json::to_vec(&plane).unwrap();
            assert_eq!(
                serde_json::from_slice::<StairNavigationPlane>(&bytes).unwrap(),
                plane
            );
        }
    }

    #[test]
    fn invalid_or_overflowing_coordinates_are_errors() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(StairNavigationPlane::new([value, 0.0, 0.0]).is_err());
        }
        let plane = StairNavigationPlane::new([2.0, 1.0, 0.0]).unwrap();
        assert!(plane.world_position([f64::MAX, 0.0]).is_err());
        assert!(plane.world_position([0.0, f64::NAN]).is_err());
        assert!(
            plane
                .route_distance([-f64::MAX / 4.0, 0.0], [f64::MAX / 4.0, 0.0])
                .is_err()
        );
        assert!(
            serde_json::from_str::<StairNavigationPlane>(r#"{"coefficients":[0,1,0],"unused":1}"#)
                .is_err()
        );
    }

    #[test]
    fn steps_reach_both_edge_on_endpoints_without_screen_displacement() {
        let plane = StairNavigationPlane::new([0.0, 1.0, 40.0]).unwrap();
        for (from, to) in [([12.0, 30.0], [12.0, 80.0]), ([12.0, 80.0], [12.0, 30.0])] {
            let mut ground = from;
            let mut travelled = 0.0;
            let screen = plane.screen_position(from).unwrap();
            let mut finished = false;
            for _ in 0..100 {
                let step = plane.advance(ground, to, 3.0).unwrap();
                let distance = plane.route_distance(ground, step.ground).unwrap();
                assert!(distance > 0.0 && distance <= 3.0 + 1e-12);
                assert_eq!(step.screen, screen);
                travelled += distance;
                ground = step.ground;
                if step.reached {
                    assert_eq!(ground, to);
                    finished = true;
                    break;
                }
            }
            assert!(finished);
            assert!((travelled - plane.route_distance(from, to).unwrap()).abs() < 1e-10);
            assert!(plane.advance(to, to, 0.0).unwrap().reached);
            assert!(!plane.advance(from, to, 0.0).unwrap().reached);
            assert!(plane.advance(from, to, -1.0).is_err());
            assert!(plane.advance(from, to, f64::INFINITY).is_err());
        }
    }
}
