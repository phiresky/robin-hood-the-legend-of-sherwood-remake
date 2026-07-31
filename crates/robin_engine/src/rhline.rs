//! Line computation helpers.
//!
//! Pure-math helpers used by the repulsive-line and repulsive-point
//! anti-collision routines.

// ---------------------------------------------------------------------------
// Vec2 — 2D vector with float components.
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl std::ops::Add for Vec2 {
    type Output = Self;
    #[inline]
    fn add(self, o: Self) -> Self {
        Self {
            x: self.x + o.x,
            y: self.y + o.y,
        }
    }
}

impl std::ops::Sub for Vec2 {
    type Output = Self;
    #[inline]
    fn sub(self, o: Self) -> Self {
        Self {
            x: self.x - o.x,
            y: self.y - o.y,
        }
    }
}

impl std::ops::Neg for Vec2 {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self {
        Self {
            x: -self.x,
            y: -self.y,
        }
    }
}

#[allow(clippy::should_implement_trait)]
impl Vec2 {
    #[inline]
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    #[inline]
    pub fn sub(self, o: Self) -> Self {
        self - o
    }

    #[inline]
    pub fn add(self, o: Self) -> Self {
        self + o
    }

    /// Dot product.
    #[inline]
    pub fn dot(self, o: Self) -> f32 {
        self.x * o.x + self.y * o.y
    }

    /// 2D determinant / cross product (aspect ratio = 1).
    #[inline]
    pub fn det(self, o: Self) -> f32 {
        self.x * o.y - self.y * o.x
    }

    /// Euclidean norm (aspect ratio = 1)
    #[inline]
    pub fn norm(self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    /// Scalar multiply
    #[inline]
    pub fn scale(self, k: f32) -> Self {
        Self {
            x: k * self.x,
            y: k * self.y,
        }
    }

    /// Negate.
    #[inline]
    pub fn neg(self) -> Self {
        -self
    }

    /// Perpendicular vector (aspect ratio = 1).
    /// `direct=true` → `(-y, x)`, `direct=false` → `(y, -x)`.
    ///
    /// Callers consume the unscaled (aspect-ratio = 1) result and apply
    /// any iso-stretch by hand afterwards (e.g. `sidewards_vector.y *=
    /// ASPECT_RATIO` at the call sites). The inline `(-y, x)` copies in
    /// `engine/melee.rs`, `sound_source.rs`, `bow_shot.rs`, and
    /// `repulsive.rs` use the same formula.
    #[inline]
    pub fn get_normal(self, direct: bool) -> Self {
        if direct {
            Self {
                x: -self.y,
                y: self.x,
            }
        } else {
            Self {
                x: self.y,
                y: -self.x,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Conversions to/from geo2d types
// ---------------------------------------------------------------------------

impl From<crate::geo2d::GeoPoint2D> for Vec2 {
    #[inline]
    fn from(p: crate::geo2d::GeoPoint2D) -> Self {
        Vec2 { x: p.x, y: p.y }
    }
}

impl Vec2 {}

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
#[allow(clippy::too_many_arguments)]
pub fn repulsive_line_compute_deviation(
    movement: Vec2,
    origin: Vec2,
    movement_mag: f32,
    distance_destination: f32,
    radius: f32,
    // const members from the repulsive line
    self_radius: f32,
    self_action_radius: f32,
    self_force_a: f32,
    self_force_b: f32,
    normal: Vec2,
    vector: Vec2,
    seg_a: Vec2,
) -> Option<Vec2> {
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
            // Every operand in the C++ coefficient expression is FLOAT;
            // only its assignment target is DOUBLE. Preserve that f32
            // evaluation boundary before continuing with the double-width
            // distance update.
            let coeff =
                ((distance_destination - radius) * self_force_a + self_force_b) as f64;
            dist_origin -= coeff * dd + (1.0 - coeff) * dist_origin;
        } else {
            return None; // too far
        }

        if (movement_mag as f64) < dist_origin.abs() {
            return None;
        }

        // `fMovement * fMovement` is evaluated as FLOAT in the Original,
        // then promoted for the subtraction from the DOUBLE distance.
        let sq = (movement_mag * movement_mag) as f64 - dist_origin * dist_origin;
        let sqrt_val = sq.sqrt() as f32;
        let do_f32 = dist_origin as f32;

        if movement.dot(vector) > 0.0 {
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
            let coeff =
                ((-distance_destination - radius) * self_force_a + self_force_b) as f64;
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

        if movement.dot(vector) > 0.0 {
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
#[allow(clippy::too_many_arguments)]
pub fn repulsive_point_compute_deviation(
    movement: Vec2,
    origin: Vec2,
    movement_mag: f32,
    distance_destination: f32,
    mut radius: f32,
    // const members from the repulsive point
    self_pos: Vec2,
    self_radius: f32,
    self_action_radius: f32,
    self_force_a: f32,
    self_force_b: f32,
) -> Option<Vec2> {
    let mut v_rel_origin: Vec2;
    let dist_origin: f64;

    if distance_destination - radius - self_radius < 0.0 {
        v_rel_origin = self_pos.sub(origin);
        dist_origin = v_rel_origin.norm() as f64;

        if dist_origin as f32 - radius - self_radius < 0.0 {
            // Already inside the obstacle
            if (dist_origin as f32) < distance_destination {
                radius = dist_origin as f32 + 0.99 * movement_mag;
            } else {
                radius = distance_destination + 0.99 * movement_mag;
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
        dist_origin = v_rel_origin.norm() as f64;

        // The coefficient is a FLOAT expression assigned to DOUBLE in the
        // C++ routine. Promoting its individual operands changes crowded
        // anti-collision trajectories even though the final vector is f32.
        let coeff =
            ((distance_destination - radius) * self_force_a + self_force_b) as f64;
        radius = (coeff * distance_destination as f64 + (1.0 - coeff) * dist_origin) as f32;
    } else {
        return None; // too far
    }

    if dist_origin < 1e-15 {
        return None; // too near
    }

    v_rel_origin = v_rel_origin.scale(1.0 / dist_origin as f32);

    // Both squares are FLOAT expressions in the Original and are rounded
    // before their difference is promoted by division through dist_origin.
    let squared_difference = (movement_mag * movement_mag - radius * radius) as f64;
    let distance: f64 = 0.5 * (dist_origin + squared_difference / dist_origin);

    if (movement_mag as f64) < distance.abs() {
        return None;
    }

    let height = ((movement_mag * movement_mag) as f64 - distance * distance).sqrt();
    let d_f32 = distance as f32;
    let h_f32 = height as f32;

    if movement.det(v_rel_origin) > 0.0 {
        Some(
            v_rel_origin
                .scale(d_f32)
                .add(v_rel_origin.get_normal(false).scale(h_f32)),
        )
    } else {
        Some(
            v_rel_origin
                .scale(d_f32)
                .add(v_rel_origin.get_normal(true).scale(h_f32)),
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
        assert_eq!(r, 5.0);
        assert_eq!(ar, 15.0); // 10 + 5
        assert!((fa - 0.1).abs() < 1e-6); // 1/(15-5) = 0.1
        assert!((fb - (-0.5)).abs() < 1e-6); // -0.1 * 5
    }

    #[test]
    fn vec2_operations() {
        let a = Vec2::new(3.0, 4.0);
        let b = Vec2::new(1.0, 2.0);
        assert!((a.dot(b) - 11.0).abs() < 1e-6);
        assert!((a.det(b) - 2.0).abs() < 1e-6);
        assert!((a.norm() - 5.0).abs() < 1e-6);
        let n = a.get_normal(true);
        assert!((n.x - (-4.0)).abs() < 1e-6);
        assert!((n.y - 3.0).abs() < 1e-6);
        let n2 = a.get_normal(false);
        assert!((n2.x - 4.0).abs() < 1e-6);
        assert!((n2.y - (-3.0)).abs() < 1e-6);
    }

    #[test]
    fn line_deviation_too_far_returns_none() {
        // Object far from the repulsive line → None
        let result = repulsive_line_compute_deviation(
            Vec2::new(1.0, 0.0),   // movement
            Vec2::new(100.0, 0.0), // origin (far away)
            1.0,                   // movement_mag
            50.0,                  // distance_destination (far)
            1.0,                   // radius
            2.0,                   // self_radius
            5.0,                   // self_action_radius
            0.333,                 // self_force_a
            -0.666,                // self_force_b
            Vec2::new(0.0, 1.0),   // normal
            Vec2::new(1.0, 0.0),   // vector
            Vec2::new(0.0, 0.0),   // seg_a
        );
        assert!(result.is_none());
    }

    #[test]
    fn point_deviation_too_far_returns_none() {
        let result = repulsive_point_compute_deviation(
            Vec2::new(1.0, 0.0),
            Vec2::new(100.0, 0.0),
            1.0,
            50.0,
            1.0,
            Vec2::new(0.0, 0.0),
            2.0,
            5.0,
            0.333,
            -0.666,
        );
        assert!(result.is_none());
    }

    #[test]
    fn line_deviation_collision() {
        // Object inside total_radius → should produce a deviated movement
        let result = repulsive_line_compute_deviation(
            Vec2::new(1.0, 0.0), // movement along X
            Vec2::new(0.0, 1.0), // origin near line
            1.0,                 // movement_mag
            1.5,                 // distance_destination < total_radius(2+1=3)
            1.0,                 // radius
            2.0,                 // self_radius
            5.0,                 // self_action_radius
            1.0 / 3.0,           // self_force_a = 1/(5-2)
            -2.0 / 3.0,          // self_force_b = -fa*2
            Vec2::new(0.0, 1.0), // normal
            Vec2::new(1.0, 0.0), // vector
            Vec2::new(0.0, 0.0), // seg_a
        );
        assert!(result.is_some());
    }
}
