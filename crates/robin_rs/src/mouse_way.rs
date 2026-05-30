//! Mouse-way gesture recognition for swordfight combat.
//!
//! While the player is swordfighting, dragging the left mouse button
//! records a polyline in screen space.  When the button is released,
//! the engine calls [`MouseWay::evaluate`] which classifies the stroke
//! into one of nine sword-strike patterns, an unrecognized "attempt",
//! or "none" (no stroke).

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::f32::consts::PI;

use crate::geo2d::{self, GeoPoint2D, Segment2D, Vec2D, segments_intersect};

/// Maximum number of points kept in the mouse-way polyline.
pub const MOUSEWAY_POINT_LIMIT: usize = 350;

/// Number of seconds a freshly-added trail sample stays at full alpha
/// before fading.
pub const TIME_TO_STAY: f32 = 0.5;

/// Initial alpha level for a new trail point: `100 + 25 * TIME_TO_STAY`.
pub const INITIAL_ALPHA: f32 = 100.0 + 25.0 * TIME_TO_STAY;

/// Recognized mouse-way patterns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MouseWayPattern {
    /// No usable polyline (too few points or no movement).
    None,
    /// Stroke was large enough to be intentional but didn't match any pattern.
    Attempt,
    /// Forward thrust, weak.
    ThrustA,
    /// Forward thrust, strong.
    ThrustB,
    /// Self-intersecting figure-8 / circle.
    ThrustC,
    /// Right-hand-side lateral.
    ThrustD,
    /// Left-hand-side lateral.
    ThrustE,
    /// Half-circle right.
    ThrustF,
    /// Half-circle left.
    ThrustG,
    /// Full circle, one direction.
    ThrustH,
    /// Full circle, opposite direction.
    ThrustI,
}

/// State for the swordfight mouse-way: the polyline being drawn and the
/// per-point alpha levels used by the on-screen trail.
///
/// The storage is a `VecDeque` so that when the polyline exceeds
/// `MOUSEWAY_POINT_LIMIT` the oldest sample can be dropped in O(1).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MouseWay {
    /// Screen-space points captured during the current drag.
    pub points: VecDeque<GeoPoint2D>,
    /// Per-point alpha used by the trail renderer.
    pub alpha: VecDeque<f32>,
}

impl MouseWay {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop all recorded points.
    pub fn clear(&mut self) {
        self.points.clear();
        self.alpha.clear();
    }

    /// Append a new mouse position to the polyline.
    ///
    /// Pushes the point and seeds the alpha at `INITIAL_ALPHA`, dropping
    /// the oldest sample if the polyline would exceed
    /// `MOUSEWAY_POINT_LIMIT`.
    pub fn add_point(&mut self, p: GeoPoint2D) {
        self.points.push_back(p);
        self.alpha.push_back(INITIAL_ALPHA);
        if self.points.len() > MOUSEWAY_POINT_LIMIT {
            self.points.pop_front();
            self.alpha.pop_front();
        }
    }

    /// Number of points currently in the polyline.
    pub fn len(&self) -> usize {
        self.points.len()
    }

    /// True when the polyline has no recorded points.
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// Classify the current polyline as a sword-strike pattern.
    ///
    /// * `pc_screen` — the swordfighter's screen-space position (the
    ///   reference point used by the directional checks).
    /// * `direction` — the swordfighter's facing direction in screen
    ///   space (the sector vector with isometric Y squish applied).
    pub fn evaluate(&self, pc_screen: GeoPoint2D, direction: Vec2D) -> MouseWayPattern {
        let n = self.points.len();
        if n <= 1 {
            return MouseWayPattern::None;
        }

        // ── Start / end of way (W / Z). ──
        let pt_w = self.points[0];
        let pt_z = self.points[n - 1];

        if n > 2 {
            // ── Find A/B/C/D extrema along the diagonals. ──
            //
            // Track four extremes:
            //   a = arg min (x + y)   (top-left in screen space)
            //   b = arg max (x + y)   (bottom-right)
            //   c = arg max (x - y)   (top-right)
            //   d = arg min (x - y)   (bottom-left)
            //
            // Also accumulate the maximum left/right deviation of the
            // polyline from the W→Z chord (used by the half-circle F/G
            // test).
            let lateral_chord = geo2d::pt(pt_z.x - pt_w.x, pt_z.y - pt_w.y);
            let lateral_normal = normalize_or_zero(perp_ccw(lateral_chord));

            let mut a_min = f32::INFINITY;
            let mut b_max = f32::NEG_INFINITY;
            let mut c_max = f32::NEG_INFINITY;
            let mut d_min = f32::INFINITY;
            let (mut ul_a, mut ul_b, mut ul_c, mut ul_d) = (0_usize, 0_usize, 0_usize, 0_usize);

            let mut max_left_deviation = 0.0_f32;
            let mut max_right_deviation = 0.0_f32;

            for (i, p) in self.points.iter().enumerate() {
                let sum = p.x + p.y;
                let diff = p.x - p.y;

                // Signed projection of (point - W) onto the chord normal.
                let signed = (p.x - pt_w.x) * lateral_normal.x + (p.y - pt_w.y) * lateral_normal.y;
                if signed > max_right_deviation {
                    max_right_deviation = signed;
                } else if signed < -max_left_deviation {
                    max_left_deviation = -signed;
                }

                if sum < a_min {
                    a_min = sum;
                    ul_a = i;
                }
                if sum > b_max {
                    b_max = sum;
                    ul_b = i;
                }
                if diff > c_max {
                    c_max = diff;
                    ul_c = i;
                }
                if diff < d_min {
                    d_min = diff;
                    ul_d = i;
                }
            }

            let self_intersecting = is_self_intersecting(&self.points);

            // ── Mid-point (axis-aligned bounding box centre). ──
            let mut x_lo = f32::INFINITY;
            let mut x_hi = f32::NEG_INFINITY;
            let mut y_lo = f32::INFINITY;
            let mut y_hi = f32::NEG_INFINITY;
            for p in &self.points {
                if p.x < x_lo {
                    x_lo = p.x;
                }
                if p.x > x_hi {
                    x_hi = p.x;
                }
                if p.y < y_lo {
                    y_lo = p.y;
                }
                if p.y > y_hi {
                    y_hi = p.y;
                }
            }
            let pt_q = geo2d::pt((x_lo + x_hi) * 0.5, (y_lo + y_hi) * 0.5);

            if !self_intersecting {
                // ── Non-self-intersecting branch. ──
                if let Some(p) =
                    check_thrust_hi(pc_screen, pt_w, pt_z, pt_q, ul_a, ul_b, ul_c, ul_d)
                {
                    return p;
                }
                if let Some(p) = check_thrust_fg(
                    pc_screen,
                    pt_w,
                    pt_z,
                    max_left_deviation,
                    max_right_deviation,
                ) {
                    return p;
                }
                if let Some(p) = check_thrust_ab(pc_screen, direction, pt_w, pt_z) {
                    return p;
                }
                if let Some(p) = check_thrust_de(pc_screen, direction, pt_w, pt_z) {
                    return p;
                }
            } else {
                // ── Self-intersecting branch. ──
                if let Some(p) = check_thrust_c(ul_a, ul_b, ul_c, ul_d) {
                    return p;
                }
                if let Some(p) =
                    check_thrust_hi(pc_screen, pt_w, pt_z, pt_q, ul_a, ul_b, ul_c, ul_d)
                {
                    return p;
                }
            }

            // Final size check: re-use the AABB we already computed.
            if (x_hi - x_lo) >= 10.0 || (y_hi - y_lo) >= 10.0 {
                return MouseWayPattern::Attempt;
            }
            return MouseWayPattern::None;
        }

        // n == 2: only the two-point bbox check applies.
        let x_lo = pt_w.x.min(pt_z.x);
        let x_hi = pt_w.x.max(pt_z.x);
        let y_lo = pt_w.y.min(pt_z.y);
        let y_hi = pt_w.y.max(pt_z.y);
        if (x_hi - x_lo) >= 10.0 || (y_hi - y_lo) >= 10.0 {
            MouseWayPattern::Attempt
        } else {
            MouseWayPattern::None
        }
    }
}

// ─── Helper checks ───────────────────────────────────────────────────────

/// Detect a single full circle (`THRUST_H`) or its mirror (`THRUST_I`).
#[allow(clippy::too_many_arguments)]
#[allow(clippy::nonminimal_bool)]
fn check_thrust_hi(
    _pt_p: GeoPoint2D,
    pt_w: GeoPoint2D,
    pt_z: GeoPoint2D,
    pt_q: GeoPoint2D,
    ul_a: usize,
    ul_b: usize,
    ul_c: usize,
    ul_d: usize,
) -> Option<MouseWayPattern> {
    let v_wp = geo2d::pt(pt_q.x - pt_w.x, pt_q.y - pt_w.y);
    let v_zp = geo2d::pt(pt_q.x - pt_z.x, pt_q.y - pt_z.y);
    let angle = vector_angle(v_wp, v_zp);

    if angle.abs() < (PI / 2.0) {
        // Cyclic ordering test.
        if (ul_a <= ul_c && ul_c <= ul_b && ul_b <= ul_d)
            || (ul_c <= ul_b && ul_b <= ul_d && ul_d <= ul_a)
            || (ul_b <= ul_d && ul_d <= ul_a && ul_a <= ul_c)
            || (ul_d <= ul_a && ul_a <= ul_c && ul_c <= ul_b)
        {
            Some(MouseWayPattern::ThrustH)
        } else {
            Some(MouseWayPattern::ThrustI)
        }
    } else {
        None
    }
}

/// Detect a one-sided half-circle (`THRUST_F` or `THRUST_G`).
fn check_thrust_fg(
    _pt_p: GeoPoint2D,
    pt_w: GeoPoint2D,
    pt_z: GeoPoint2D,
    max_left_deviation: f32,
    max_right_deviation: f32,
) -> Option<MouseWayPattern> {
    let dx = pt_z.x - pt_w.x;
    let dy = pt_z.y - pt_w.y;
    let distance = (dx * dx + dy * dy).sqrt();
    // If the chord is degenerate, neither side passes the 0.3 ratio test.
    if distance == 0.0 {
        return None;
    }

    let left_ratio = max_left_deviation / distance;
    let right_ratio = max_right_deviation / distance;

    if left_ratio > 0.3 {
        if right_ratio > 0.3 {
            // S curve — too wobbly.
            None
        } else {
            Some(MouseWayPattern::ThrustF)
        }
    } else if right_ratio > 0.3 {
        Some(MouseWayPattern::ThrustG)
    } else {
        // Curve too straight for an F/G match.
        None
    }
}

/// Detect a sideways slash (`THRUST_D` right, `THRUST_E` left).
fn check_thrust_de(
    _pt_p: GeoPoint2D,
    direction: Vec2D,
    pt_w: GeoPoint2D,
    pt_z: GeoPoint2D,
) -> Option<MouseWayPattern> {
    let v_zw = geo2d::pt(pt_z.x - pt_w.x, pt_z.y - pt_w.y);
    let v_revert = geo2d::pt(-direction.x, -direction.y);
    let angle = vector_angle(v_revert, v_zw);

    if angle > (PI / 4.0) && angle < (3.0 * PI / 4.0) {
        Some(MouseWayPattern::ThrustE)
    } else if angle < (-PI / 4.0) && angle > (-3.0 * PI / 4.0) {
        Some(MouseWayPattern::ThrustD)
    } else {
        None
    }
}

/// Detect a forward / backward thrust (`THRUST_A` weak, `THRUST_B` strong).
fn check_thrust_ab(
    _pt_p: GeoPoint2D,
    direction: Vec2D,
    pt_w: GeoPoint2D,
    pt_z: GeoPoint2D,
) -> Option<MouseWayPattern> {
    let v_zw = geo2d::pt(pt_w.x - pt_z.x, pt_w.y - pt_z.y);
    let v_revert = geo2d::pt(-direction.x, -direction.y);
    let angle = vector_angle(v_revert, v_zw);

    if angle.abs() < (PI / 4.0) {
        Some(MouseWayPattern::ThrustB)
    } else if angle.abs() > (3.0 * PI / 4.0) {
        Some(MouseWayPattern::ThrustA)
    } else {
        None
    }
}

/// Detect the figure-8 / monotone-ordering pattern (`THRUST_C`).
fn check_thrust_c(ul_a: usize, ul_b: usize, ul_c: usize, ul_d: usize) -> Option<MouseWayPattern> {
    let values = [ul_a, ul_b, ul_c, ul_d];

    let forward = ul_a < ul_b;
    let mut position = 0_usize;
    if forward {
        let mut min = usize::MAX;
        for (i, v) in values.iter().enumerate() {
            if *v < min {
                position = i;
                min = *v;
            }
        }
    } else {
        let mut max = 0_usize;
        for (i, v) in values.iter().enumerate() {
            if *v > max {
                position = i;
                max = *v;
            }
        }
    }

    // Walk the cycle starting at `position` and check that the indices
    // are monotone in the chosen direction.
    for offset in 0..3 {
        let i = (position + offset) % 4;
        let j = (position + offset + 1) % 4;
        if forward && values[i] > values[j] {
            return None;
        }
        if !forward && values[i] < values[j] {
            return None;
        }
    }

    Some(MouseWayPattern::ThrustC)
}

// ─── Geometry helpers ────────────────────────────────────────────────────

/// Counter-clockwise perpendicular of `v`.
fn perp_ccw(v: Vec2D) -> Vec2D {
    geo2d::pt(-v.y, v.x)
}

/// Normalize a vector; returns the zero vector when the input length
/// is below `geo2d::PRECISION`.
fn normalize_or_zero(v: Vec2D) -> Vec2D {
    let len = (v.x * v.x + v.y * v.y).sqrt();
    if len < geo2d::PRECISION {
        geo2d::pt(0.0, 0.0)
    } else {
        geo2d::pt(v.x / len, v.y / len)
    }
}

/// Signed angle between two vectors in `(-PI, PI]`.  Uses `atan2` of the
/// cross and dot products.
fn vector_angle(a: Vec2D, b: Vec2D) -> f32 {
    let cross = a.x * b.y - a.y * b.x;
    let dot = a.x * b.x + a.y * b.y;
    cross.atan2(dot)
}

/// Test whether the polyline crosses itself.
///
/// Walks every pair of non-adjacent polyline segments and returns `true`
/// on the first crossing.  Adjacent segments (sharing an endpoint) are
/// skipped.
pub fn is_self_intersecting(points: &VecDeque<GeoPoint2D>) -> bool {
    let n = points.len();
    if n < 4 {
        return false;
    }
    let n_segs = n - 1;
    for i in 0..n_segs {
        let s1 = Segment2D::new(points[i], points[i + 1]);
        for j in (i + 2)..n_segs {
            let s2 = Segment2D::new(points[j], points[j + 1]);
            if segments_intersect(s1, s2) {
                return true;
            }
        }
    }
    false
}

// ─── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo2d::pt;

    fn make_way(points: &[(f32, f32)]) -> MouseWay {
        let mut w = MouseWay::new();
        for &(x, y) in points {
            w.add_point(pt(x, y));
        }
        w
    }

    /// Reference point and direction used by the recognition tests.
    fn ref_point() -> GeoPoint2D {
        pt(320.0, 320.0)
    }
    fn ref_direction() -> Vec2D {
        pt(0.0, -10.0)
    }

    #[test]
    fn empty_polyline_returns_none() {
        let way = MouseWay::new();
        assert_eq!(
            way.evaluate(ref_point(), ref_direction()),
            MouseWayPattern::None
        );
    }

    #[test]
    fn single_point_returns_none() {
        let way = make_way(&[(100.0, 100.0)]);
        assert_eq!(
            way.evaluate(ref_point(), ref_direction()),
            MouseWayPattern::None
        );
    }

    /// Forward thrust → THRUST_B.
    #[test]
    fn straight_forward_is_thrust_b() {
        let mut points = Vec::new();
        let (mut x, mut y) = (320.0_f32, 300.0_f32);
        for _ in 0..10 {
            x += 1.0;
            y -= 4.0;
            points.push((x, y));
        }
        let way = make_way(&points);
        assert_eq!(
            way.evaluate(ref_point(), ref_direction()),
            MouseWayPattern::ThrustB
        );
    }

    /// Figure-8 → THRUST_C.
    #[test]
    fn figure_eight_is_thrust_c() {
        let way = make_way(&[
            (200.0, 200.0),
            (330.0, 330.0),
            (400.0, 400.0),
            (400.0, 200.0),
            (310.0, 310.0),
            (200.0, 400.0),
            (200.0, 300.0),
        ]);
        assert_eq!(
            way.evaluate(ref_point(), ref_direction()),
            MouseWayPattern::ThrustC
        );
    }

    /// Figure-8 (rotated start) → THRUST_C.
    #[test]
    fn figure_eight_rotated_is_thrust_c() {
        let way = make_way(&[
            (200.0, 300.0),
            (200.0, 400.0),
            (310.0, 310.0),
            (400.0, 200.0),
            (400.0, 400.0),
            (330.0, 330.0),
            (200.0, 200.0),
        ]);
        assert_eq!(
            way.evaluate(ref_point(), ref_direction()),
            MouseWayPattern::ThrustC
        );
    }

    /// Figure-8 (different start) → THRUST_C.
    #[test]
    fn figure_eight_third_rotation_is_thrust_c() {
        let way = make_way(&[
            (400.0, 200.0),
            (310.0, 310.0),
            (200.0, 400.0),
            (200.0, 300.0),
            (200.0, 200.0),
            (330.0, 330.0),
            (400.0, 400.0),
        ]);
        assert_eq!(
            way.evaluate(ref_point(), ref_direction()),
            MouseWayPattern::ThrustC
        );
    }

    /// Figure-8 (fourth rotation) → THRUST_C.
    #[test]
    fn figure_eight_fourth_rotation_is_thrust_c() {
        let way = make_way(&[
            (400.0, 200.0),
            (400.0, 400.0),
            (330.0, 330.0),
            (200.0, 200.0),
            (200.0, 300.0),
            (200.0, 400.0),
            (310.0, 310.0),
        ]);
        assert_eq!(
            way.evaluate(ref_point(), ref_direction()),
            MouseWayPattern::ThrustC
        );
    }

    /// Leftward horizontal stroke while facing north → THRUST_E.
    ///
    /// Tracing both `evaluate` / `check_thrust_de` (and the matching
    /// gamepad-stick recognizer) shows the implementation produces
    /// THRUST_E here, even though an old reference test asserted
    /// THRUST_D — that test block was never compiled by the shipping
    /// build, so the assertion drifted.  We test the actual
    /// implementation behaviour, which is what the game runs.
    #[test]
    fn leftward_stroke_is_thrust_e() {
        let mut points = Vec::new();
        let (mut x, mut y) = (360.0_f32, 360.0_f32);
        for i in 0..10 {
            x -= 8.0;
            // Deterministic small jitter — "almost-straight horizontal".
            y += (i % 3) as f32;
            points.push((x, y));
        }
        let way = make_way(&points);
        assert_eq!(
            way.evaluate(ref_point(), ref_direction()),
            MouseWayPattern::ThrustE
        );
    }

    /// Rightward horizontal stroke while facing north → THRUST_D.
    /// Same drift as the leftward case — see the leftward test comment.
    #[test]
    fn rightward_stroke_is_thrust_d() {
        let mut points = Vec::new();
        let (mut x, mut y) = (280.0_f32, 360.0_f32);
        for i in 0..10 {
            x += 8.0;
            y += (i % 3) as f32;
            points.push((x, y));
        }
        let way = make_way(&points);
        assert_eq!(
            way.evaluate(ref_point(), ref_direction()),
            MouseWayPattern::ThrustD
        );
    }

    /// Right-bulged half-circle → THRUST_F.
    #[test]
    fn right_half_circle_is_thrust_f() {
        let way = make_way(&[
            (320.0, 280.0),
            (360.0, 320.0),
            (360.0, 340.0),
            (320.0, 340.0),
            (300.0, 350.0),
        ]);
        assert_eq!(
            way.evaluate(ref_point(), ref_direction()),
            MouseWayPattern::ThrustF
        );
    }

    /// Left-bulged half-circle → THRUST_G.
    #[test]
    fn left_half_circle_is_thrust_g() {
        let way = make_way(&[
            (320.0, 280.0),
            (280.0, 320.0),
            (280.0, 340.0),
            (320.0, 340.0),
            (330.0, 350.0),
        ]);
        assert_eq!(
            way.evaluate(ref_point(), ref_direction()),
            MouseWayPattern::ThrustG
        );
    }

    /// Full circle → THRUST_H.
    #[test]
    fn full_circle_is_thrust_h() {
        let way = make_way(&[
            (320.0, 280.0),
            (360.0, 320.0),
            (360.0, 340.0),
            (320.0, 350.0),
            (300.0, 340.0),
            (280.0, 340.0),
            (280.0, 320.0),
            (320.0, 290.0),
        ]);
        assert_eq!(
            way.evaluate(ref_point(), ref_direction()),
            MouseWayPattern::ThrustH
        );
    }

    /// Reverse full circle → THRUST_H.
    #[test]
    fn reverse_full_circle_is_thrust_h() {
        let way = make_way(&[
            (320.0, 340.0),
            (300.0, 320.0),
            (280.0, 280.0),
            (320.0, 280.0),
            (360.0, 300.0),
            (360.0, 320.0),
            (325.0, 340.0),
        ]);
        assert_eq!(
            way.evaluate(ref_point(), ref_direction()),
            MouseWayPattern::ThrustH
        );
    }

    /// Mirror full circle → THRUST_I.
    #[test]
    fn mirror_full_circle_is_thrust_i() {
        let way = make_way(&[
            (320.0, 290.0),
            (280.0, 320.0),
            (280.0, 340.0),
            (300.0, 340.0),
            (320.0, 350.0),
            (360.0, 340.0),
            (360.0, 320.0),
            (320.0, 280.0),
        ]);
        assert_eq!(
            way.evaluate(ref_point(), ref_direction()),
            MouseWayPattern::ThrustI
        );
    }

    #[test]
    fn point_limit_drops_oldest() {
        let mut way = MouseWay::new();
        for i in 0..(MOUSEWAY_POINT_LIMIT + 50) {
            way.add_point(pt(i as f32, 0.0));
        }
        assert_eq!(way.len(), MOUSEWAY_POINT_LIMIT);
        // First point should be sample index 50, since the first 50 were
        // dropped.
        assert!((way.points[0].x - 50.0).abs() < 0.001);
    }

    #[test]
    fn clear_resets_state() {
        let mut way = make_way(&[(0.0, 0.0), (1.0, 1.0)]);
        way.clear();
        assert!(way.is_empty());
    }

    #[test]
    fn small_jitter_is_none() {
        // A two-point polyline with bbox under 10×10 is `None`
        // (the "no attempt" branch).
        let way = make_way(&[(100.0, 100.0), (105.0, 102.0)]);
        assert_eq!(
            way.evaluate(ref_point(), ref_direction()),
            MouseWayPattern::None
        );
    }

    /// Two-point stroke whose bbox spans more than 10 pixels but
    /// hits the n==2 fast path (the multi-point branch is skipped) →
    /// the bbox guard fires and returns `Attempt`.
    #[test]
    fn long_two_point_stroke_returns_attempt() {
        let way = make_way(&[(100.0, 100.0), (200.0, 100.0)]);
        assert_eq!(
            way.evaluate(ref_point(), ref_direction()),
            MouseWayPattern::Attempt
        );
    }
}
