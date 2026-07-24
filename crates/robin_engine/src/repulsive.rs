//! Repulsive objects used by actor-vs-actor anti-collision.
//!
//! These are the "personal space" markers each actor contributes to
//! the anti-collision system — when another actor is about to step
//! onto them, `PositionInterface::update_position_anti_collision`
//! (see [`crate::position_interface`]) deviates the moving actor
//! around them.
//!
//! The heavy deviation math already lives in [`crate::rhline`] as pure
//! functions — the structs here are just data holders that wire the
//! right members into those functions.

use serde::{Deserialize, Serialize};

use crate::coordinates::{MapPoint, MapVec};
use crate::geo2d;
use crate::rhline;

/// Repulsive point — a circular (or wedge-limited) zone of influence
/// centred on `position`.
///
/// `radius`, `action_radius`, `force_a`, `force_b` come from
/// `SetForce` — see [`rhline::repulsive_set_force`].  The stored
/// `action_radius` **includes** `radius`
/// (`action_radius = radius + action_radius_input`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RepulsivePoint {
    pub position: MapPoint,
    pub radius: f32,
    /// Total action radius: `radius + action_radius_input`.
    pub action_radius: f32,
    pub force_a: f32,
    pub force_b: f32,

    // Action field. Default: total circle.
    pub is_total: bool,
    pub is_concave: bool,
    pub limit_left: MapVec,
    pub limit_right: MapVec,
}

impl RepulsivePoint {
    /// Build a `RepulsivePoint` with the given `SetForce` parameters.
    /// `radius` is the inner (hard) radius; `action_radius_input` is the
    /// soft falloff distance beyond it.
    pub fn new(position: MapPoint, radius: f32, action_radius_input: f32) -> Self {
        let (ar, r, fa, fb) = rhline::repulsive_set_force(radius, action_radius_input);
        Self {
            position,
            radius: r,
            action_radius: ar,
            force_a: fa,
            force_b: fb,
            is_total: true,
            is_concave: false,
            limit_left: MapVec::ZERO,
            limit_right: MapVec::ZERO,
        }
    }

    /// Restrict the action field to an angular wedge.
    pub fn set_action_field(&mut self, limit_left: MapVec, limit_right: MapVec) {
        self.is_total = false;
        self.limit_left = limit_left;
        self.limit_right = limit_right;
        self.is_concave = limit_left.x * limit_right.y - limit_left.y * limit_right.x < 0.0;
    }

    /// Returns `Some(distance_destination)` if the future position
    /// `destination` is close enough to warrant a deviation check.
    pub fn is_deviating(&self, destination: MapPoint) -> Option<f32> {
        let rel = destination - self.position;
        let distance = rel.length();
        if distance > self.action_radius {
            return None;
        }
        if !self.is_total {
            let left = self.limit_left.x * rel.y - self.limit_left.y * rel.x;
            let right = self.limit_right.x * rel.y - self.limit_right.y * rel.x;
            if self.is_concave {
                if left < 0.0 && right >= 0.0 {
                    return None;
                }
            } else if left < 0.0 || right >= 0.0 {
                return None;
            }
        }
        Some(distance)
    }

    /// Compute the deviated movement around this point.  Returns
    /// `Some(new_movement)` when the actor should deviate; `None`
    /// means "too far" (continue straight).
    pub fn compute_deviation(
        &self,
        movement: MapVec,
        origin: MapPoint,
        movement_mag: f32,
        distance_destination: f32,
        actor_radius: f32,
    ) -> Option<MapVec> {
        let r = rhline::repulsive_point_compute_deviation(
            rhline::Vec2::new(movement.x, movement.y),
            rhline::Vec2::new(origin.x, origin.y),
            movement_mag,
            distance_destination,
            actor_radius,
            rhline::Vec2::new(self.position.x, self.position.y),
            self.radius,
            // RHRepulsivePoint::ComputeDeviation reads the stored
            // mfActionRadius, which SetForce has already expanded by
            // the inner radius.
            self.action_radius,
            self.force_a,
            self.force_b,
        )?;
        Some(MapVec::new(r.x, r.y))
    }
}

/// Repulsive line segment — a directed line with an outward normal
/// and a repulsion zone extending perpendicular to it.
///
/// `action_radius` **includes** `radius` (same convention as
/// [`RepulsivePoint`]).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RepulsiveLine {
    pub a: MapPoint,
    pub b: MapPoint,
    pub normal: MapVec,
    pub vector: MapVec,
    pub radius: f32,
    /// Total action radius: `radius + action_radius_input`.
    pub action_radius: f32,
    pub force_a: f32,
    pub force_b: f32,
    /// `POINT_TOTAL` flag — when false, only positive-normal side deflects.
    pub is_total: bool,
    /// True when the segment is an "area" (two-sided repulsion); selects
    /// the direct-sense normal.
    pub is_area: bool,
}

impl RepulsiveLine {
    /// Build a `RepulsiveLine` from endpoints and `SetForce` parameters.
    pub fn new(a: MapPoint, b: MapPoint, radius: f32, action_radius_input: f32) -> Self {
        let (ar, r, fa, fb) = rhline::repulsive_set_force(radius, action_radius_input);
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let len = (dx * dx + dy * dy).sqrt();
        let (vx, vy) = if len > 0.0 {
            (dx / len, dy / len)
        } else {
            (0.0, 0.0)
        };
        // Default: non-area line uses `get_normal(false)` = (y, -x).
        let nx = vy;
        let ny = -vx;
        Self {
            a,
            b,
            normal: MapVec::new(nx, ny),
            vector: MapVec::new(vx, vy),
            radius: r,
            action_radius: ar,
            force_a: fa,
            force_b: fb,
            is_total: true,
            is_area: false,
        }
    }

    /// True when the `destination` lies between the segment endpoints
    /// along the segment's projection axis.  Delegates to
    /// [`geo2d::point_in_segment_slab`].
    fn is_between(&self, destination: MapPoint) -> bool {
        geo2d::point_in_segment_slab(
            destination.to_geo(),
            geo2d::Segment2D::new(self.a.to_geo(), self.b.to_geo()),
        )
    }

    /// Returns `Some(signed_distance_destination)` if the future
    /// position `destination` is close enough to warrant a deviation
    /// check.
    pub fn is_deviating(&self, destination: MapPoint) -> Option<f32> {
        let rel = destination - self.a;
        let distance = rel.x * self.normal.x + rel.y * self.normal.y;
        if !self.is_total && distance < 0.0 {
            return None;
        }
        if distance.abs() < self.action_radius && self.is_between(destination) {
            Some(distance)
        } else {
            None
        }
    }

    /// Compute the deviated movement around this line.
    pub fn compute_deviation(
        &self,
        movement: MapVec,
        origin: MapPoint,
        movement_mag: f32,
        distance_destination: f32,
        actor_radius: f32,
    ) -> Option<MapVec> {
        let r = rhline::repulsive_line_compute_deviation(
            rhline::Vec2::new(movement.x, movement.y),
            rhline::Vec2::new(origin.x, origin.y),
            movement_mag,
            distance_destination,
            actor_radius,
            self.radius,
            self.action_radius,
            self.force_a,
            self.force_b,
            rhline::Vec2::new(self.normal.x, self.normal.y),
            rhline::Vec2::new(self.vector.x, self.vector.y),
            rhline::Vec2::new(self.a.x, self.a.y),
        )?;
        Some(MapVec::new(r.x, r.y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinates::map_pt;

    #[test]
    fn point_is_deviating_total() {
        let p = RepulsivePoint::new(map_pt(0.0, 0.0), 4.0, 12.0);
        // Inside action radius (16 total) → Some
        assert!(p.is_deviating(map_pt(5.0, 0.0)).is_some());
        // Outside action radius → None
        assert!(p.is_deviating(map_pt(50.0, 0.0)).is_none());
    }

    #[test]
    fn point_new_stores_total_action_radius() {
        let p = RepulsivePoint::new(map_pt(0.0, 0.0), 4.0, 12.0);
        assert!((p.radius - 4.0).abs() < 1e-6);
        assert!((p.action_radius - 16.0).abs() < 1e-6);
    }

    #[test]
    fn corpse_point_deviation_matches_frame_404_geometry() {
        // Original parity replay, frame 404: Robin's naive four-unit
        // walking step enters a dead soldier's outer repulsion zone.
        // RHRepulsivePoint::ComputeDeviation uses the stored total action
        // radius (10 + 15), producing this exact deflection.
        let origin = map_pt(f32::from_bits(1_142_634_601), f32::from_bits(1_155_129_408));
        let movement = MapVec::new(f32::from_bits(1_079_899_904), f32::from_bits(1_073_682_432));
        let future = origin + movement;
        let corpse = RepulsivePoint::new(
            map_pt(f32::from_bits(1_143_098_668), f32::from_bits(1_155_128_658)),
            10.0,
            15.0,
        );
        let distance_destination = (future - corpse.position).length();

        let deviated = corpse
            .compute_deviation(movement, origin, 4.0, distance_destination, 4.0)
            .expect("corpse must deflect the approaching actor");
        let committed = origin + deviated;

        assert_eq!(committed.x.to_bits(), 1_142_678_042);
        assert_eq!(committed.y.to_bits(), 1_155_153_943);
    }

    #[test]
    fn line_is_deviating_between() {
        let l = RepulsiveLine::new(map_pt(0.0, 0.0), map_pt(10.0, 0.0), 2.0, 5.0);
        // Point near midpoint, on +normal side → Some
        assert!(l.is_deviating(map_pt(5.0, 3.0)).is_some());
        // Point past the segment endpoints → None
        assert!(l.is_deviating(map_pt(20.0, 3.0)).is_none());
    }

    #[test]
    fn line_deviation_uses_stored_total_action_radius() {
        // radius=2 and input action radius=5 stores mfActionRadius=7.
        // With actor radius 4, a destination ten units from the line is
        // inside the C++ threshold (4 + 7) but outside the old, incorrectly
        // subtracted threshold (4 + 5).
        let line = RepulsiveLine::new(map_pt(0.0, 0.0), map_pt(100.0, 0.0), 2.0, 5.0);
        let movement = MapVec::new(0.0, 4.0);
        let deviated = line
            .compute_deviation(movement, map_pt(50.0, -14.0), 4.0, 10.0, 4.0)
            .expect("stored total action radius must keep the line active");

        assert_eq!(deviated.x.to_bits(), (-2.3999999_f32).to_bits());
        assert_eq!(deviated.y.to_bits(), 3.2_f32.to_bits());
    }
}
