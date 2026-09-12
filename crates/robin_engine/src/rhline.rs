//! Line computation helpers.
//!
//! Pure-math helpers used by the repulsive-line and repulsive-point
//! anti-collision routines.

use crate::coordinates::{MapPoint, MapVec};
use crate::repulsive::{RepulsiveLine, RepulsivePoint};
use std::ops::{Add, Neg, Sub};

// ---------------------------------------------------------------------------
// Repulsive force helpers (shared by line and point)
// ---------------------------------------------------------------------------

/// Computes force parameters used by both repulsive-line and
/// repulsive-point construction.
///
/// Returns `(action_radius, radius, force_a, force_b)`.
#[inline]
pub fn repulsive_set_force(radius: f32, action_radius: f32) -> (f32, f32, f32, f32) {
    let ar = action_radius + radius;
    let fa = 1.0 / (ar - radius);
    let fb = -fa * radius;
    (ar, radius, fa, fb)
}

// ---------------------------------------------------------------------------
// Repulsive-line deviation
// ---------------------------------------------------------------------------

/// Computes the deviation movement around a repulsive line segment.
///
/// Uses `f64` for intermediate variables to preserve precision.
/// Returns `Some(deviated_movement)` on success, `None` when no deviation
/// is required.
pub fn repulsive_line_compute_deviation(
    line: &RepulsiveLine,
    movement: MapVec,
    origin: MapPoint,
    movement_mag: f32,
    distance_destination: f32,
    radius: f32,
) -> Option<MapVec> {
    let RepulsiveLine {
        radius: self_radius,
        action_radius: self_action_radius,
        force_a: self_force_a,
        force_b: self_force_b,
        normal,
        vector,
        a: seg_a,
        ..
    } = *line;
    let total_radius: f64 = self_radius as f64 + radius as f64;
    let dd = distance_destination as f64;

    let v_rel_origin = origin.sub(seg_a);
    let mut dist_origin: f64 = (v_rel_origin.x * normal.x + v_rel_origin.y * normal.y) as f64;

    if dd > 0.0 {
        if dd < total_radius {
            if dist_origin < total_radius {
                // Inside obstacle — try to escape
                dist_origin = -0.95 * movement_mag as f64;
            } else {
                // Collision
                dist_origin -= total_radius;
            }
        } else if dd < (radius + self_action_radius) as f64 {
            // Every operand in the original coefficient expression is single precision;
            // only its assignment target is double precision. Preserve that f32
            // evaluation boundary before continuing with the double-width
            // distance update.
            let coeff = ((distance_destination - radius) * self_force_a + self_force_b) as f64;
            dist_origin -= coeff * dd + (1.0 - coeff) * dist_origin;
        } else {
            return None; // too far
        }

        if (movement_mag as f64) < dist_origin.abs() {
            return None;
        }

        // The movement square is evaluated in single precision in the original
        // game, then promoted for subtraction from the double-precision distance.
        let sq = (movement_mag * movement_mag) as f64 - dist_origin * dist_origin;
        let sqrt_val = sq.sqrt() as f32;
        let do_f32 = dist_origin as f32;

        if crate::geo2d::dot(movement.to_geo(), vector.to_geo()) > 0.0 {
            Some(normal.scale(-do_f32).add(vector.scale(sqrt_val)))
        } else {
            Some(normal.scale(do_f32).add(vector.scale(sqrt_val)).neg())
        }
    } else {
        // Negative side — mirror
        if dd > -total_radius {
            if dist_origin > -total_radius {
                dist_origin = 0.95 * movement_mag as f64;
            } else {
                dist_origin += total_radius;
            }
        } else if -dd < (radius + self_action_radius) as f64 {
            let coeff = ((-distance_destination - radius) * self_force_a + self_force_b) as f64;
            dist_origin -= coeff * dd + (1.0 - coeff) * dist_origin;
        } else {
            return None;
        }

        if (movement_mag as f64) < dist_origin.abs() {
            return None;
        }

        let sq = (movement_mag * movement_mag) as f64 - dist_origin * dist_origin;
        let sqrt_val = sq.sqrt() as f32;
        let do_f32 = dist_origin as f32;

        if crate::geo2d::dot(movement.to_geo(), vector.to_geo()) > 0.0 {
            Some(normal.scale(-do_f32).add(vector.scale(sqrt_val)))
        } else {
            Some(normal.scale(do_f32).add(vector.scale(sqrt_val)).neg())
        }
    }
}

// ---------------------------------------------------------------------------
// Repulsive-point deviation
// ---------------------------------------------------------------------------

/// Computes the deviation movement around a repulsive point.
///
/// Uses `f64` for intermediate variables to preserve precision.
pub fn repulsive_point_compute_deviation(
    point: &RepulsivePoint,
    movement: MapVec,
    origin: MapPoint,
    movement_mag: f32,
    distance_destination: f32,
    mut radius: f32,
) -> Option<MapVec> {
    let RepulsivePoint {
        position: self_pos,
        radius: self_radius,
        action_radius: self_action_radius,
        force_a: self_force_a,
        force_b: self_force_b,
        ..
    } = *point;
    let mut v_rel_origin: MapVec;
    let dist_origin: f64;

    if distance_destination - radius - self_radius < 0.0 {
        v_rel_origin = self_pos.sub(origin);
        dist_origin = v_rel_origin.length() as f64;

        if dist_origin as f32 - radius - self_radius < 0.0 {
            // Already inside the obstacle
            if (dist_origin as f32) < distance_destination {
                // The original game's `0.99` is double precision, so
                // the multiply and add happen in double precision before the
                // result is stored back in a single-precision radius. Performing two
                // f32 operations here moves crowded avoidance trajectories by
                // one map-position ULP.
                radius = (dist_origin + 0.99 * movement_mag as f64) as f32;
            } else {
                radius = (distance_destination as f64 + 0.99 * movement_mag as f64) as f32;
            }
        } else {
            // Collision
            radius += self_radius;
            v_rel_origin = self_pos.sub(origin);
            // dist_origin stays the same since self_pos and origin haven't changed
        }
    } else if distance_destination - radius - self_action_radius < 0.0 {
        // Deviation zone
        v_rel_origin = self_pos.sub(origin);
        dist_origin = v_rel_origin.length() as f64;

        // The coefficient is a single-precision expression assigned to double precision in the
        // original calculation. Promoting its individual operands changes crowded
        // anti-collision trajectories even though the final vector is f32.
        let coeff = ((distance_destination - radius) * self_force_a + self_force_b) as f64;
        radius = (coeff * distance_destination as f64 + (1.0 - coeff) * dist_origin) as f32;
    } else {
        return None; // too far
    }

    if dist_origin < 1e-15 {
        return None; // too near
    }

    v_rel_origin = v_rel_origin.scale(1.0 / dist_origin as f32);

    // Both squares are single-precision expressions in the original game and are rounded
    // before their difference is promoted by division through dist_origin.
    let squared_difference = (movement_mag * movement_mag - radius * radius) as f64;
    let distance: f64 = 0.5 * (dist_origin + squared_difference / dist_origin);

    if (movement_mag as f64) < distance.abs() {
        return None;
    }

    let height = ((movement_mag * movement_mag) as f64 - distance * distance).sqrt();
    let d_f32 = distance as f32;
    let h_f32 = height as f32;

    if crate::geo2d::cross(movement.to_geo(), v_rel_origin.to_geo()) > 0.0 {
        Some(
            v_rel_origin
                .scale(d_f32)
                .add(MapVec::new(v_rel_origin.y, -v_rel_origin.x).scale(h_f32)),
        )
    } else {
        Some(
            v_rel_origin
                .scale(d_f32)
                .add(MapVec::new(-v_rel_origin.y, v_rel_origin.x).scale(h_f32)),
        )
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_force_basic() {
        let (ar, r, fa, fb) = repulsive_set_force(5.0, 10.0);
        assert_eq!((ar, r), (15.0, 5.0));
        assert_eq!((fa, fb), (0.1, -0.5));
    }

    #[test]
    fn point_escape_radius_preserves_original_double_expression() {
        // Save008 replay-009 frame 1068: preserve the Original f64 escape
        // expression and its f32 rounding boundary exactly.
        let movement = MapVec::new(f32::from_bits(3_209_131_520), f32::from_bits(3_218_059_776));
        let origin = MapPoint::new(f32::from_bits(1_136_387_169), f32::from_bits(1_143_137_685));
        let obstacle = MapPoint::new(f32::from_bits(1_136_466_202), f32::from_bits(1_143_023_002));
        let point = RepulsivePoint::new(obstacle, 4.0, 12.0);
        let result = point
            .compute_deviation(
                movement,
                origin,
                f32::from_bits(1_072_064_103),
                f32::from_bits(1_086_854_588),
                4.0,
            )
            .expect("overlapping actors must produce an escape deviation");
        assert_eq!(result.x.to_bits(), 3_219_492_766);
        assert_eq!(result.y.to_bits(), 3_189_581_806);
    }

    fn line() -> RepulsiveLine {
        let mut line = RepulsiveLine::new(MapPoint::ZERO, MapPoint::new(1.0, 0.0), 2.0, 3.0);
        line.normal = MapVec::new(0.0, 1.0);
        line.vector = MapVec::new(1.0, 0.0);
        line
    }

    #[test]
    fn line_deviation_too_far_returns_none() {
        assert!(
            line()
                .compute_deviation(
                    MapVec::new(1.0, 0.0),
                    MapPoint::new(100.0, 0.0),
                    1.0,
                    50.0,
                    1.0
                )
                .is_none()
        );
    }

    #[test]
    fn point_deviation_too_far_returns_none() {
        let point = RepulsivePoint::new(MapPoint::ZERO, 2.0, 3.0);
        assert!(
            point
                .compute_deviation(
                    MapVec::new(1.0, 0.0),
                    MapPoint::new(100.0, 0.0),
                    1.0,
                    50.0,
                    1.0
                )
                .is_none()
        );
    }

    #[test]
    fn line_deviation_collision() {
        assert!(
            line()
                .compute_deviation(
                    MapVec::new(1.0, 0.0),
                    MapPoint::new(0.0, 1.0),
                    1.0,
                    1.5,
                    1.0
                )
                .is_some()
        );
    }
}
