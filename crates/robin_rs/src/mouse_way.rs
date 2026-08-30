//! Mouse-way gesture recognition for swordfight combat.
//!
//! While the player is swordfighting, dragging the left mouse button
//! records a polyline in screen space.  When the button is released,
//! the engine calls [`MouseWay::evaluate`] which classifies the stroke
//! into one of nine sword-strike patterns, an unrecognized "attempt",
//! or "none" (no stroke).

use robin_engine::coordinates::{ScreenPoint, ScreenVec};
use robin_engine::geo2d::{PRECISION, Segment2D, segments_intersect};
use robin_engine::player_command::{CompositeSwordTechnique, GestureQuality};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::f32::consts::PI;

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
    /// Optional template-recognized two-strike technique.
    Composite(CompositeSwordTechnique),
}

/// Original A-I patterns in guide and recognition order.
pub const LEGACY_PATTERNS: [MouseWayPattern; 9] = [
    MouseWayPattern::ThrustA,
    MouseWayPattern::ThrustB,
    MouseWayPattern::ThrustC,
    MouseWayPattern::ThrustD,
    MouseWayPattern::ThrustE,
    MouseWayPattern::ThrustF,
    MouseWayPattern::ThrustG,
    MouseWayPattern::ThrustH,
    MouseWayPattern::ThrustI,
];

/// Rich host-side result. Only the quantized quality and technique enter a
/// [`robin_engine::player_command::PlayerCommand`]; the floating score and
/// nearest template are presentation diagnostics.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GestureEvaluation {
    pub pattern: MouseWayPattern,
    pub quality: GestureQuality,
    pub similarity: f32,
    pub nearest_composite: Option<CompositeSwordTechnique>,
}

/// Short-lived, host-only result consumed by the optional coach overlay.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GestureCoachFeedback {
    pub pattern: MouseWayPattern,
    pub quality: GestureQuality,
    pub bounds: (ScreenPoint, ScreenPoint),
    /// Rotation that turns the north-facing teaching template into the
    /// selected fighter's screen-space orientation. Only A/B/D/E are actor
    /// relative in the original classifier; all other patterns store zero.
    pub template_rotation: f32,
    pub created_at_ms: u32,
}

const TEMPLATE_SAMPLE_COUNT: usize = 64;
const COMPOSITE_ACCEPT_SIMILARITY: f32 = 0.88;
const COMPOSITE_ACCEPT_MARGIN: f32 = 0.055;

// Each extension template deliberately self-crosses while failing the
// original figure-eight/circle checks. That makes the legacy classifier
// return Attempt for the entire fixed vocabulary, which is what lets the
// extension preserve A-I recognition without heuristic precedence tricks.
const RISING_FEINT: &[(f32, f32)] = &[
    (-1.0, -1.0),
    (1.0, 1.0),
    (-1.0, 1.0),
    (1.0, -1.0),
    (1.0, 1.0),
];
const FALLING_FEINT: &[(f32, f32)] = &[
    (-1.0, 1.0),
    (1.0, -1.0),
    (-1.0, -1.0),
    (1.0, 1.0),
    (1.0, -1.0),
];
const LIGHTNING: &[(f32, f32)] = &[
    (-1.0, -1.0),
    (1.0, 1.0),
    (-1.0, 0.0),
    (1.0, -1.0),
    (1.0, 1.0),
];
const BACKSLASH: &[(f32, f32)] = &[
    (1.0, -1.0),
    (-1.0, -1.0),
    (1.0, 1.0),
    (1.0, 0.0),
    (-1.0, 1.0),
];
const TRIAD: &[(f32, f32)] = &[
    (-1.0, -1.0),
    (1.0, 1.0),
    (0.0, -1.0),
    (-1.0, 1.0),
    (1.0, -1.0),
    (1.0, 1.0),
];
const RAMPART: &[(f32, f32)] = &[
    (1.0, -1.0),
    (-1.0, -1.0),
    (1.0, 1.0),
    (0.0, -1.0),
    (-1.0, 1.0),
];
const VORTEX: &[(f32, f32)] = &[
    (-1.0, -1.0),
    (1.0, 1.0),
    (-0.5, 1.0),
    (0.5, -1.0),
    (-1.0, 1.0),
    (1.0, -1.0),
    (1.0, 1.0),
];
const STAG: &[(f32, f32)] = &[
    (1.0, -1.0),
    (-1.0, -1.0),
    (1.0, 1.0),
    (0.6, -0.4),
    (-1.0, 1.0),
];
const SERPENT: &[(f32, f32)] = &[
    (-1.0, -1.0),
    (1.0, -1.0),
    (-1.0, 1.0),
    (-1.0, -0.5),
    (1.0, 1.0),
];

/// Authored, normalized display/recognition path for a composite technique.
pub fn composite_template(technique: CompositeSwordTechnique) -> &'static [(f32, f32)] {
    match technique {
        CompositeSwordTechnique::RisingFeint => RISING_FEINT,
        CompositeSwordTechnique::FallingFeint => FALLING_FEINT,
        CompositeSwordTechnique::Lightning => LIGHTNING,
        CompositeSwordTechnique::Backslash => BACKSLASH,
        CompositeSwordTechnique::Triad => TRIAD,
        CompositeSwordTechnique::Rampart => RAMPART,
        CompositeSwordTechnique::Vortex => VORTEX,
        CompositeSwordTechnique::Stag => STAG,
        CompositeSwordTechnique::Serpent => SERPENT,
    }
}

/// State for the swordfight mouse-way: the polyline being drawn and the
/// per-point alpha levels used by the on-screen trail.
///
/// The storage is a `VecDeque` so that when the polyline exceeds
/// `MOUSEWAY_POINT_LIMIT` the oldest sample can be dropped in O(1).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MouseWay {
    /// Screen-space points captured during the current drag.
    pub points: VecDeque<ScreenPoint>,
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
    pub fn add_point(&mut self, p: ScreenPoint) {
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
    pub fn evaluate(&self, pc_screen: ScreenPoint, direction: ScreenVec) -> MouseWayPattern {
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
            let lateral_chord = ScreenVec::new(pt_z.x - pt_w.x, pt_z.y - pt_w.y);
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
            let pt_q = ScreenPoint::new((x_lo + x_hi) * 0.5, (y_lo + y_hi) * 0.5);

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

    /// Preserve the original classifier, add fixed-template scoring, and —
    /// when enabled — allow a high-confidence composite template to override
    /// the broad legacy heuristics. Existing A-I reference paths are guarded
    /// by tests against all composite templates.
    pub fn evaluate_detailed(
        &self,
        pc_screen: ScreenPoint,
        direction: ScreenVec,
        allow_composites: bool,
    ) -> GestureEvaluation {
        let legacy = self.evaluate(pc_screen, direction);
        if self.points.len() <= 1 {
            return GestureEvaluation {
                pattern: legacy,
                quality: GestureQuality::PERFECT,
                similarity: 0.0,
                nearest_composite: None,
            };
        }

        let input = normalized_resampled(self.points.iter().map(|point| (point.x, point.y)));
        let legacy_similarity = score_legacy_pattern(legacy, &input, direction).unwrap_or(0.0);
        let best_legacy_similarity = LEGACY_PATTERNS
            .into_iter()
            .filter_map(|pattern| score_legacy_pattern(pattern, &input, direction))
            .fold(0.0_f32, f32::max);

        let mut ranked = CompositeSwordTechnique::ALL
            .into_iter()
            .map(|technique| {
                let template = normalized_resampled(composite_template(technique).iter().copied());
                (technique, path_similarity(&input, &template, false, false))
            })
            .collect::<Vec<_>>();
        ranked.sort_by(|left, right| right.1.total_cmp(&left.1));
        let (nearest_composite, best_composite) = ranked[0];
        let second_composite = ranked[1].1;

        // The original classifier remains authoritative for every A-I
        // stroke. New techniques are considered only after it has rejected
        // an intentional stroke, so enabling this feature cannot reinterpret
        // any legacy input -- including unusual but valid A-I variants that
        // are not represented by the teaching templates below.
        if allow_composites
            && matches!(legacy, MouseWayPattern::Attempt)
            && best_composite >= COMPOSITE_ACCEPT_SIMILARITY
            && best_composite - second_composite >= COMPOSITE_ACCEPT_MARGIN
            && best_composite - best_legacy_similarity >= COMPOSITE_ACCEPT_MARGIN
        {
            return GestureEvaluation {
                pattern: MouseWayPattern::Composite(nearest_composite),
                quality: quality_from_similarity(best_composite),
                similarity: best_composite,
                nearest_composite: Some(nearest_composite),
            };
        }

        GestureEvaluation {
            pattern: legacy,
            quality: if matches!(legacy, MouseWayPattern::None | MouseWayPattern::Attempt) {
                GestureQuality::new(0).expect("zero gesture quality is valid")
            } else {
                quality_from_similarity(legacy_similarity)
            },
            similarity: legacy_similarity,
            nearest_composite: Some(nearest_composite),
        }
    }

    pub fn bounds(&self) -> Option<(ScreenPoint, ScreenPoint)> {
        let first = self.points.front()?;
        let mut lo = *first;
        let mut hi = *first;
        for point in &self.points {
            lo.x = lo.x.min(point.x);
            lo.y = lo.y.min(point.y);
            hi.x = hi.x.max(point.x);
            hi.y = hi.y.max(point.y);
        }
        Some((lo, hi))
    }
}

const THRUST_A_TEMPLATE: &[(f32, f32)] = &[(0.0, -1.0), (0.0, 1.0)];
const THRUST_B_TEMPLATE: &[(f32, f32)] = &[(0.0, 1.0), (0.0, -1.0)];
const THRUST_C_TEMPLATE: &[(f32, f32)] = &[
    (-1.0, 0.0),
    (-0.5, -0.8),
    (0.5, 0.8),
    (1.0, 0.0),
    (0.5, -0.8),
    (-0.5, 0.8),
    (-1.0, 0.0),
];
const THRUST_D_TEMPLATE: &[(f32, f32)] = &[(-1.0, 0.0), (1.0, 0.0)];
const THRUST_E_TEMPLATE: &[(f32, f32)] = &[(1.0, 0.0), (-1.0, 0.0)];
const THRUST_F_TEMPLATE: &[(f32, f32)] =
    &[(0.0, -1.0), (0.8, -0.5), (1.0, 0.2), (0.5, 0.8), (0.0, 1.0)];
const THRUST_G_TEMPLATE: &[(f32, f32)] = &[
    (0.0, -1.0),
    (-0.8, -0.5),
    (-1.0, 0.2),
    (-0.5, 0.8),
    (0.0, 1.0),
];
const THRUST_H_TEMPLATE: &[(f32, f32)] = &[
    (0.0, -1.0),
    (0.7, -0.7),
    (1.0, 0.0),
    (0.7, 0.7),
    (0.0, 1.0),
    (-0.7, 0.7),
    (-1.0, 0.0),
    (-0.7, -0.7),
    (0.0, -1.0),
];
const THRUST_I_TEMPLATE: &[(f32, f32)] = &[
    (0.0, -1.0),
    (-0.7, -0.7),
    (-1.0, 0.0),
    (-0.7, 0.7),
    (0.0, 1.0),
    (0.7, 0.7),
    (1.0, 0.0),
    (0.7, -0.7),
    (0.0, -1.0),
];

fn legacy_template(pattern: MouseWayPattern) -> Option<&'static [(f32, f32)]> {
    Some(match pattern {
        MouseWayPattern::ThrustA => THRUST_A_TEMPLATE,
        MouseWayPattern::ThrustB => THRUST_B_TEMPLATE,
        MouseWayPattern::ThrustC => THRUST_C_TEMPLATE,
        MouseWayPattern::ThrustD => THRUST_D_TEMPLATE,
        MouseWayPattern::ThrustE => THRUST_E_TEMPLATE,
        MouseWayPattern::ThrustF => THRUST_F_TEMPLATE,
        MouseWayPattern::ThrustG => THRUST_G_TEMPLATE,
        MouseWayPattern::ThrustH => THRUST_H_TEMPLATE,
        MouseWayPattern::ThrustI => THRUST_I_TEMPLATE,
        MouseWayPattern::None | MouseWayPattern::Attempt | MouseWayPattern::Composite(_) => {
            return None;
        }
    })
}

/// Fixed normalized path used by the guide/coach overlay.
pub fn display_template(pattern: MouseWayPattern) -> Option<&'static [(f32, f32)]> {
    match pattern {
        MouseWayPattern::Composite(technique) => Some(composite_template(technique)),
        legacy => legacy_template(legacy),
    }
}

/// Screen-space rotation for a teaching template. The original A/B/D/E
/// recognizers are relative to the fighter's facing direction, while C and
/// F-I (and all new fixed templates) are authored in screen space.
pub fn display_template_rotation(pattern: MouseWayPattern, direction: ScreenVec) -> f32 {
    if !matches!(
        pattern,
        MouseWayPattern::ThrustA
            | MouseWayPattern::ThrustB
            | MouseWayPattern::ThrustD
            | MouseWayPattern::ThrustE
    ) {
        return 0.0;
    }
    if direction.x.abs() < PRECISION && direction.y.abs() < PRECISION {
        tracing::warn!("combat gesture guide received a zero actor-facing vector");
        return 0.0;
    }
    direction.y.atan2(direction.x) + PI / 2.0
}

pub fn pattern_label(pattern: MouseWayPattern) -> &'static str {
    match pattern {
        MouseWayPattern::None => "None",
        MouseWayPattern::Attempt => "Try again",
        MouseWayPattern::ThrustA => "A Jab",
        MouseWayPattern::ThrustB => "B Thrust",
        MouseWayPattern::ThrustC => "C Execute",
        MouseWayPattern::ThrustD => "D Right",
        MouseWayPattern::ThrustE => "E Left",
        MouseWayPattern::ThrustF => "F Half",
        MouseWayPattern::ThrustG => "G Half",
        MouseWayPattern::ThrustH => "H Circle",
        MouseWayPattern::ThrustI => "I Circle",
        MouseWayPattern::Composite(technique) => technique.label(),
    }
}

fn quality_from_similarity(similarity: f32) -> GestureQuality {
    match similarity.clamp(0.0, 1.0) {
        similarity if similarity >= 0.97 => GestureQuality::PERFECT,
        similarity if similarity >= 0.80 => GestureQuality::GOOD,
        similarity if similarity >= 0.55 => GestureQuality::FAIR,
        _ => GestureQuality::MINIMUM,
    }
}

fn score_legacy_pattern(
    pattern: MouseWayPattern,
    input: &[(f32, f32)],
    direction: ScreenVec,
) -> Option<f32> {
    let template = normalized_resampled(legacy_template(pattern)?.iter().copied());
    let actor_relative = matches!(
        pattern,
        MouseWayPattern::ThrustA
            | MouseWayPattern::ThrustB
            | MouseWayPattern::ThrustD
            | MouseWayPattern::ThrustE
    );
    let scored_input = if actor_relative {
        rotate_to_north(input, direction)
    } else {
        input.to_vec()
    };
    let cyclic = matches!(
        pattern,
        MouseWayPattern::ThrustC | MouseWayPattern::ThrustH | MouseWayPattern::ThrustI
    );
    let rotation_invariant = matches!(pattern, MouseWayPattern::ThrustF | MouseWayPattern::ThrustG);
    Some(path_similarity(
        &scored_input,
        &template,
        cyclic,
        rotation_invariant,
    ))
}

fn rotate_to_north(points: &[(f32, f32)], direction: ScreenVec) -> Vec<(f32, f32)> {
    if direction.x.abs() < PRECISION && direction.y.abs() < PRECISION {
        tracing::warn!("gesture quality received a zero actor-facing vector");
        return points.to_vec();
    }
    let facing_angle = direction.y.atan2(direction.x);
    let target_angle = -PI / 2.0;
    let angle = target_angle - facing_angle;
    let (sin, cos) = angle.sin_cos();
    points
        .iter()
        .map(|&(x, y)| (x * cos - y * sin, x * sin + y * cos))
        .collect()
}

fn normalized_resampled(points: impl IntoIterator<Item = (f32, f32)>) -> Vec<(f32, f32)> {
    let points = points.into_iter().collect::<Vec<_>>();
    if points.len() < 2 {
        return points;
    }
    let mut cumulative = Vec::with_capacity(points.len());
    cumulative.push(0.0);
    for pair in points.windows(2) {
        let dx = pair[1].0 - pair[0].0;
        let dy = pair[1].1 - pair[0].1;
        cumulative.push(cumulative.last().copied().unwrap_or(0.0) + dx.hypot(dy));
    }
    let total = cumulative.last().copied().unwrap_or(0.0);
    if total < PRECISION {
        return vec![(0.0, 0.0); TEMPLATE_SAMPLE_COUNT];
    }

    let mut sampled = Vec::with_capacity(TEMPLATE_SAMPLE_COUNT);
    let mut segment = 0;
    for index in 0..TEMPLATE_SAMPLE_COUNT {
        let distance = total * index as f32 / (TEMPLATE_SAMPLE_COUNT - 1) as f32;
        while segment + 1 < cumulative.len() - 1 && cumulative[segment + 1] < distance {
            segment += 1;
        }
        let span = cumulative[segment + 1] - cumulative[segment];
        let t = if span < PRECISION {
            0.0
        } else {
            (distance - cumulative[segment]) / span
        };
        sampled.push((
            points[segment].0 + (points[segment + 1].0 - points[segment].0) * t,
            points[segment].1 + (points[segment + 1].1 - points[segment].1) * t,
        ));
    }

    let centroid = sampled
        .iter()
        .fold((0.0, 0.0), |acc, point| (acc.0 + point.0, acc.1 + point.1));
    let centroid = (
        centroid.0 / sampled.len() as f32,
        centroid.1 / sampled.len() as f32,
    );
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
    );
    for point in &sampled {
        min_x = min_x.min(point.0);
        max_x = max_x.max(point.0);
        min_y = min_y.min(point.1);
        max_y = max_y.max(point.1);
    }
    let scale = (max_x - min_x).max(max_y - min_y).max(PRECISION);
    sampled
        .into_iter()
        .map(|point| {
            (
                (point.0 - centroid.0) / scale,
                (point.1 - centroid.1) / scale,
            )
        })
        .collect()
}

fn path_similarity(
    input: &[(f32, f32)],
    template: &[(f32, f32)],
    cyclic: bool,
    rotation_invariant: bool,
) -> f32 {
    if input.len() != template.len() || input.is_empty() {
        return 0.0;
    }
    let offset_count = if cyclic { input.len() } else { 1 };
    let best = (0..offset_count)
        .map(|offset| {
            let angle = if rotation_invariant {
                let (cross, dot) = input.iter().enumerate().fold(
                    (0.0_f32, 0.0_f32),
                    |(cross, dot), (index, input)| {
                        let template = template[(index + offset) % template.len()];
                        (
                            cross + input.0 * template.1 - input.1 * template.0,
                            dot + input.0 * template.0 + input.1 * template.1,
                        )
                    },
                );
                cross.atan2(dot)
            } else {
                0.0
            };
            let (sin, cos) = angle.sin_cos();
            input
                .iter()
                .enumerate()
                .map(|(index, input)| {
                    let template = template[(index + offset) % template.len()];
                    let aligned = (input.0 * cos - input.1 * sin, input.0 * sin + input.1 * cos);
                    (aligned.0 - template.0).hypot(aligned.1 - template.1)
                })
                .sum::<f32>()
                / input.len() as f32
        })
        .fold(f32::INFINITY, f32::min);
    (1.0 - best / std::f32::consts::FRAC_1_SQRT_2).clamp(0.0, 1.0)
}

// ─── Helper checks ───────────────────────────────────────────────────────

/// Detect a single full circle (`THRUST_H`) or its mirror (`THRUST_I`).
#[allow(clippy::too_many_arguments)]
#[allow(clippy::nonminimal_bool)]
fn check_thrust_hi(
    _pt_p: ScreenPoint,
    pt_w: ScreenPoint,
    pt_z: ScreenPoint,
    pt_q: ScreenPoint,
    ul_a: usize,
    ul_b: usize,
    ul_c: usize,
    ul_d: usize,
) -> Option<MouseWayPattern> {
    let v_wp = ScreenVec::new(pt_q.x - pt_w.x, pt_q.y - pt_w.y);
    let v_zp = ScreenVec::new(pt_q.x - pt_z.x, pt_q.y - pt_z.y);
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
    _pt_p: ScreenPoint,
    pt_w: ScreenPoint,
    pt_z: ScreenPoint,
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
    _pt_p: ScreenPoint,
    direction: ScreenVec,
    pt_w: ScreenPoint,
    pt_z: ScreenPoint,
) -> Option<MouseWayPattern> {
    let v_zw = ScreenVec::new(pt_z.x - pt_w.x, pt_z.y - pt_w.y);
    let v_revert = ScreenVec::new(-direction.x, -direction.y);
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
    _pt_p: ScreenPoint,
    direction: ScreenVec,
    pt_w: ScreenPoint,
    pt_z: ScreenPoint,
) -> Option<MouseWayPattern> {
    let v_zw = ScreenVec::new(pt_w.x - pt_z.x, pt_w.y - pt_z.y);
    let v_revert = ScreenVec::new(-direction.x, -direction.y);
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
fn perp_ccw(v: ScreenVec) -> ScreenVec {
    ScreenVec::new(-v.y, v.x)
}

/// Normalize a vector; returns the zero vector when the input length
/// is below the shared geometry precision.
fn normalize_or_zero(v: ScreenVec) -> ScreenVec {
    let len = (v.x * v.x + v.y * v.y).sqrt();
    if len < PRECISION {
        ScreenVec::ZERO
    } else {
        ScreenVec::new(v.x / len, v.y / len)
    }
}

/// Signed angle between two vectors in `(-PI, PI]`.  Uses `atan2` of the
/// cross and dot products.
fn vector_angle(a: ScreenVec, b: ScreenVec) -> f32 {
    let cross = a.x * b.y - a.y * b.x;
    let dot = a.x * b.x + a.y * b.y;
    cross.atan2(dot)
}

/// Test whether the polyline crosses itself.
///
/// Walks every pair of non-adjacent polyline segments and returns `true`
/// on the first crossing.  Adjacent segments (sharing an endpoint) are
/// skipped.
pub fn is_self_intersecting(points: &VecDeque<ScreenPoint>) -> bool {
    let n = points.len();
    if n < 4 {
        return false;
    }
    let n_segs = n - 1;
    for i in 0..n_segs {
        let s1 = Segment2D::new(points[i].to_geo(), points[i + 1].to_geo());
        for j in (i + 2)..n_segs {
            let s2 = Segment2D::new(points[j].to_geo(), points[j + 1].to_geo());
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

    fn make_way(points: &[(f32, f32)]) -> MouseWay {
        let mut w = MouseWay::new();
        for &(x, y) in points {
            w.add_point(ScreenPoint::new(x, y));
        }
        w
    }

    fn make_template_way(points: &[(f32, f32)]) -> MouseWay {
        let mut way = MouseWay::new();
        let transform = |(x, y): (f32, f32)| (320.0 + x * 90.0, 320.0 + y * 90.0);
        let first = transform(points[0]);
        way.add_point(ScreenPoint::new(first.0, first.1));
        for segment in points.windows(2) {
            for step in 1..=8 {
                let t = step as f32 / 8.0;
                let point = (
                    segment[0].0 + (segment[1].0 - segment[0].0) * t,
                    segment[0].1 + (segment[1].1 - segment[0].1) * t,
                );
                let point = transform(point);
                way.add_point(ScreenPoint::new(point.0, point.1));
            }
        }
        way
    }

    /// Reference point and direction used by the recognition tests.
    fn ref_point() -> ScreenPoint {
        ScreenPoint::new(320.0, 320.0)
    }
    fn ref_direction() -> ScreenVec {
        ScreenVec::new(0.0, -10.0)
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
            way.add_point(ScreenPoint::new(i as f32, 0.0));
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

    #[test]
    fn all_composite_golden_paths_are_distinct_and_perfect() {
        for technique in CompositeSwordTechnique::ALL {
            let way = make_template_way(composite_template(technique));
            assert_eq!(
                way.evaluate(ref_point(), ref_direction()),
                MouseWayPattern::Attempt,
                "composite template for {technique:?} collides with the original A-I classifier"
            );
            let evaluation = way.evaluate_detailed(ref_point(), ref_direction(), true);
            assert_eq!(
                evaluation.pattern,
                MouseWayPattern::Composite(technique),
                "golden path for {technique:?} was not recognized"
            );
            assert_eq!(evaluation.quality, GestureQuality::PERFECT);
        }
    }

    #[test]
    fn composite_templates_are_translation_scale_and_small_noise_tolerant() {
        for technique in CompositeSwordTechnique::ALL {
            let template = composite_template(technique);
            let mut way = MouseWay::new();
            let mut sample_index = 0_u32;
            for segment in template.windows(2) {
                for step in 0..8 {
                    let t = step as f32 / 8.0;
                    let x = segment[0].0 + (segment[1].0 - segment[0].0) * t;
                    let y = segment[0].1 + (segment[1].1 - segment[0].1) * t;
                    let noise_x = ((sample_index * 17 + 3) % 7) as f32 - 3.0;
                    let noise_y = ((sample_index * 11 + 5) % 7) as f32 - 3.0;
                    way.add_point(ScreenPoint::new(
                        147.0 + x * 76.0 + noise_x * 0.35,
                        211.0 + y * 76.0 + noise_y * 0.35,
                    ));
                    sample_index += 1;
                }
            }
            let last = *template.last().expect("nonempty composite template");
            way.add_point(ScreenPoint::new(
                147.0 + last.0 * 76.0,
                211.0 + last.1 * 76.0,
            ));

            let evaluation = way.evaluate_detailed(ref_point(), ref_direction(), true);
            assert_eq!(
                evaluation.pattern,
                MouseWayPattern::Composite(technique),
                "slightly noisy path for {technique:?} was not recognized (legacy={:?}, similarity={})",
                way.evaluate(ref_point(), ref_direction()),
                evaluation.similarity,
            );
            assert!(evaluation.quality.is_strike_quality());
        }
    }

    #[test]
    fn composite_extension_never_reclassifies_any_recognized_legacy_stroke() {
        let mut seed = 0xA17E_5EED_u32;
        for case in 0..512 {
            let mut way = MouseWay::new();
            let mut point = ScreenPoint::new(320.0, 320.0);
            for _ in 0..(4 + case % 20) {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                point.x += ((seed >> 16) as i16 as f32) / 2_048.0;
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                point.y += ((seed >> 16) as i16 as f32) / 2_048.0;
                way.add_point(point);
            }
            let legacy = way.evaluate(ref_point(), ref_direction());
            let extended = way
                .evaluate_detailed(ref_point(), ref_direction(), true)
                .pattern;
            if !matches!(legacy, MouseWayPattern::Attempt) {
                assert_eq!(
                    extended, legacy,
                    "extension reclassified legacy stroke in generated case {case}"
                );
            }
        }
    }

    #[test]
    fn disabling_composites_preserves_the_original_classifier() {
        for technique in CompositeSwordTechnique::ALL {
            let way = make_template_way(composite_template(technique));
            assert_eq!(
                way.evaluate_detailed(ref_point(), ref_direction(), false)
                    .pattern,
                way.evaluate(ref_point(), ref_direction()),
                "disabled extension changed the legacy result for {technique:?}"
            );
        }
    }

    #[test]
    fn enabling_composites_preserves_canonical_a_through_i_paths() {
        let golden: &[(MouseWayPattern, &[(f32, f32)])] = &[
            (
                MouseWayPattern::ThrustA,
                &[(320.0, 260.0), (320.0, 300.0), (320.0, 340.0)],
            ),
            (
                MouseWayPattern::ThrustB,
                &[(320.0, 340.0), (320.0, 300.0), (320.0, 260.0)],
            ),
            (
                MouseWayPattern::ThrustC,
                &[
                    (200.0, 200.0),
                    (330.0, 330.0),
                    (400.0, 400.0),
                    (400.0, 200.0),
                    (310.0, 310.0),
                    (200.0, 400.0),
                    (200.0, 300.0),
                ],
            ),
            (
                MouseWayPattern::ThrustD,
                &[(260.0, 320.0), (320.0, 320.0), (380.0, 320.0)],
            ),
            (
                MouseWayPattern::ThrustE,
                &[(380.0, 320.0), (320.0, 320.0), (260.0, 320.0)],
            ),
            (
                MouseWayPattern::ThrustF,
                &[
                    (320.0, 280.0),
                    (360.0, 320.0),
                    (360.0, 340.0),
                    (320.0, 340.0),
                    (300.0, 350.0),
                ],
            ),
            (
                MouseWayPattern::ThrustG,
                &[
                    (320.0, 280.0),
                    (280.0, 320.0),
                    (280.0, 340.0),
                    (320.0, 340.0),
                    (330.0, 350.0),
                ],
            ),
            (
                MouseWayPattern::ThrustH,
                &[
                    (320.0, 280.0),
                    (360.0, 320.0),
                    (360.0, 340.0),
                    (320.0, 350.0),
                    (300.0, 340.0),
                    (280.0, 340.0),
                    (280.0, 320.0),
                    (320.0, 290.0),
                ],
            ),
            (
                MouseWayPattern::ThrustI,
                &[
                    (320.0, 290.0),
                    (280.0, 320.0),
                    (280.0, 340.0),
                    (300.0, 340.0),
                    (320.0, 350.0),
                    (360.0, 340.0),
                    (360.0, 320.0),
                    (320.0, 280.0),
                ],
            ),
        ];

        for &(expected, points) in golden {
            let way = make_way(points);
            assert_eq!(way.evaluate(ref_point(), ref_direction()), expected);
            assert_eq!(
                way.evaluate_detailed(ref_point(), ref_direction(), true)
                    .pattern,
                expected,
                "composite recognition stole canonical {expected:?} input"
            );
        }
    }

    #[test]
    fn gesture_quality_uses_fixed_deterministic_tiers() {
        assert_eq!(quality_from_similarity(1.0), GestureQuality::PERFECT);
        assert_eq!(quality_from_similarity(0.97), GestureQuality::PERFECT);
        assert_eq!(quality_from_similarity(0.96), GestureQuality::GOOD);
        assert_eq!(quality_from_similarity(0.80), GestureQuality::GOOD);
        assert_eq!(quality_from_similarity(0.79), GestureQuality::FAIR);
        assert_eq!(quality_from_similarity(0.55), GestureQuality::FAIR);
        assert_eq!(quality_from_similarity(0.54), GestureQuality::MINIMUM);
    }

    #[test]
    fn actor_relative_teaching_templates_follow_facing() {
        assert_eq!(
            display_template_rotation(MouseWayPattern::ThrustA, ref_direction()),
            0.0
        );
        let east = display_template_rotation(MouseWayPattern::ThrustD, ScreenVec::new(10.0, 0.0));
        assert!((east - PI / 2.0).abs() < 0.0001);
        assert_eq!(
            display_template_rotation(MouseWayPattern::ThrustH, ScreenVec::new(10.0, 0.0)),
            0.0
        );
    }
}
