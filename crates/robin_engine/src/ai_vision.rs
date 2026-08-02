//! NPC AI perception — the cone/halfcircle visibility test, the
//! distance-sharpness curve, and the per-frame view-parameter refresh.
//!
//! `compute_visibility(...)` returns a sharpness in
//! `[0.0, RUNNING_DETECTION_FACTOR]` — 0.0 means "not visible this
//! frame", non-zero means the target is inside the perception cone /
//! halfcircle and the line of sight is clear.  The caller multiplies
//! this by `BASE_VIEW_SPEED` and accumulates into the NPC's
//! `detection_suspects[ENEMY]` to drive detection events.
//!
//! # Deferred short-circuits
//!
//! A few short-circuits cannot be evaluated in this module because the
//! underlying state lives outside it — callers pass the resolved values
//! through [`VisibilityQuery`] / [`ObjectVisibilityQuery`]:
//!
//!   * Per-NPC eye-state — `viewer_eye_status` (Closed /
//!     DieOrGetUnconscious are blind).
//!   * Viewer / target inside a building sector — `viewer_in_building`
//!     and `target_in_same_building`.
//!   * Target "active and outside building" —
//!     `target_is_active_and_outside_building`.
//!   * Forest-level merry-men 180° special case —
//!     `forest_180_degree_view`.
//!   * Effective view radius after the ground-plane sphere projection
//!     and night/fog modulation — computed by [`compute_view_radius`]
//!     and passed in via `effective_view_radius`.
//!
//! [`los_clear_spatial`] runs the line-of-sight check by collecting
//! candidate obstacles from the FastFindGrid cells crossed by the ray,
//! then checking opaque blockers on that smaller candidate list.

use crate::coordinates::{GroundBBox, GroundPoint, MapPoint, WorldPoint3D};
use crate::element::{ActionState, EntityId, EyeStatus, NpcData, Posture};
use crate::order::OrderType;
use crate::position_interface::{ASPECT_RATIO, INVERSE_ASPECT_RATIO, ObstacleHandle};
use crate::sight_obstacle::ObstacleList;
use crate::sight_obstacle::SightObstacle;

/// One surface-owned `ComputeViewRadius` result, corresponding to Original's
/// last-viewer, last-radius, and universal-frame fields.
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    serde::Deserialize,
    serde::Serialize,
    robin_state_hash_derive::StateHash,
)]
pub(crate) struct ViewRadiusCacheEntry {
    pub(crate) viewer: EntityId,
    pub(crate) frame: u32,
    pub(crate) radius: f32,
}

/// Persistent surface-owned view-radius memo table.
///
/// Each surface has only one last writer. A different viewer in the same
/// frame therefore overwrites the prior entry, exactly as `RHSightObstacle`
/// does. Stale entries remain stored but do not hit; Original also leaves them
/// in place and gates them by the universal frame counter.
#[derive(
    Clone,
    Debug,
    Default,
    PartialEq,
    serde::Deserialize,
    serde::Serialize,
    robin_state_hash_derive::StateHash,
)]
pub(crate) struct ViewRadiusCache {
    pub(crate) ground: Option<ViewRadiusCacheEntry>,
    pub(crate) obstacles: Vec<Option<ViewRadiusCacheEntry>>,
}

impl ViewRadiusCache {
    pub(crate) fn is_empty(&self) -> bool {
        self.ground.is_none() && self.obstacles.iter().all(Option::is_none)
    }

    pub(crate) fn get(
        &self,
        surface: Option<ObstacleHandle>,
        viewer: EntityId,
        frame: u32,
    ) -> Option<f32> {
        let entry = match surface {
            None => self.ground,
            Some(handle) => self.obstacles.get(usize::from(handle)).copied().flatten(),
        }?;
        (entry.viewer == viewer && entry.frame == frame && entry.radius != 0.0)
            .then_some(entry.radius)
    }

    pub(crate) fn set(
        &mut self,
        surface: Option<ObstacleHandle>,
        viewer: EntityId,
        frame: u32,
        radius: f32,
    ) {
        let entry = Some(ViewRadiusCacheEntry {
            viewer,
            frame,
            radius,
        });
        match surface {
            None => self.ground = entry,
            Some(handle) => {
                let index = usize::from(handle);
                if self.obstacles.len() <= index {
                    self.obstacles.resize(index + 1, None);
                }
                self.obstacles[index] = entry;
            }
        }
    }
}

// ─── Constants ───────────────────────────────────────────────────

/// Default view radius the engine hands out at level start before any
/// per-NPC mutation.  Used as the `view_radius` seed for freshly-spawned
/// NPCs.
pub const DEFAULT_VIEW_RADIUS: u16 = 400;

/// Reduced view radius for Fog/Night ambiances, installed at mission
/// load.
pub const NIGHT_VIEW_RADIUS: u16 = 300;

/// Squared radius below which the close-range halfcircle test applies
/// instead of the narrow forward cone.
pub const SQR_HALFCIRCLE_VIEW_RADIUS: f32 = 60.0 * 60.0;

/// Default half-aperture of the vision cone.
pub const NORMAL_HALF_APERTURE: f32 = 0.5;

pub const CROUCHED_DETECTION_FACTOR: f32 = 0.5;
pub const WALKING_DETECTION_FACTOR: f32 = 3.0;
pub const RUNNING_DETECTION_FACTOR: f32 = 20.0;
pub const SWORD_DETECTION_FACTOR: f32 = 1.5;
pub const BOW_DETECTION_FACTOR: f32 = 2.0;

/// Distance-curve control points.
pub const DETECTION_CURVE_X_1: f32 = 0.3;
pub const DETECTION_CURVE_Y_1: f32 = 0.95;
pub const DETECTION_CURVE_X_2: f32 = 0.7;
pub const DETECTION_CURVE_Y_2: f32 = 0.25;
pub const DETECTION_CURVE_Y_3: f32 = 0.15;

/// Multiplier applied to the per-frame visibility to produce the
/// sharpness accumulated into `detection_suspects[ENEMY]`.
pub const BASE_VIEW_SPEED: u16 = 20;

/// 10× faster detection when leaning out from a wall (the NPC is
/// looking straight down with a wide cone, so targets are spotted
/// quickly).
pub const LOOK_DOWN_BASE_VIEW_SPEED: u16 = 200;

/// Detection frequency for PC targets — an NPC only re-runs the full
/// visibility test against each PC every 2 frames (times a per-NPC
/// phase offset).
pub const DETECTION_FREQUENCY_ENEMY_PC: u32 = 2;

/// Detection frequency for enemy NPCs (Lacklandists seen by Royalist
/// mercenaries / Sheriffs).  Slower than the PC variant because enemy
/// NPCs re-scan less aggressively.
pub const DETECTION_FREQUENCY_ENEMY_NPC: u32 = 16;

/// Frames between visibility refreshes for `DetectableType::Body`
/// entries.
pub const DETECTION_FREQUENCY_BODY: u32 = 8;

/// Frames between visibility refreshes for `DetectableType::Object`
/// entries (coins, ales, money bags).
pub const DETECTION_FREQUENCY_OBJECT: u32 = 4;

/// Frames between visibility refreshes for `DetectableType::Friend`
/// entries.
pub const DETECTION_FREQUENCY_FRIEND: u32 = 8;

/// Frames between visibility refreshes for `DetectableType::MissedFriend`
/// entries (the "Charly" mechanic).
pub const DETECTION_FREQUENCY_MISSED_FRIEND: u32 = 8;

/// Frames between visibility refreshes for `DetectableType::Beggar`
/// entries.
pub const DETECTION_FREQUENCY_BEGGAR: u32 = 8;

/// Frame period at which `detection_suspects[type]` is decremented when
/// nothing is visible, so an NPC "cools down" after a false alarm.
pub const UNSUSPECT_FREQUENCY: u32 = 20;

/// Threshold at which accumulated detection sharpness commits a
/// detection event.
pub const DETECTION_SUSPECT_THRESHOLD: u32 = 1000;

/// When `detection_suspects >= 100` and the target is visible this
/// frame, fire `EVENT_SEES_SHADOW` for the "what's that?" pre-detection
/// warning.  Edge-triggered: only fires on the frame the threshold is
/// first crossed.
pub const SHADOW_DETECTION_THRESHOLD: u32 = 100;

// ─── refresh_view constants ─────────────────────────────────────

pub const NORMAL_HALF_ANGLE_RANGE: f32 = 0.8;

pub const STARE_HALF_ANGLE_RANGE: f32 = 1.3;

/// π/8.
// RHparametersai.h's active definition is PI/8. The PI/16 value immediately
// below it is inside the commented "old values" block.
pub const NORMAL_ANGLE_STEP: f32 = 0.3927;

/// π/80.
pub const NORMAL_ANGLE_ITERATOR_STEP: f32 = 0.03927;

pub const STARE_APERTURE_FACTOR: f32 = 0.7;

pub const STARE_RANGE_FACTOR: f32 = 1.4;

pub const RIDER_VIEW_RADIUS_FACTOR: f32 = 1.4;

pub const ALPHA_START: u16 = 154;

// ─── Public API ──────────────────────────────────────────────────

/// Context the caller passes in for a full `compute_visibility` run.
/// Everything the test reads from the viewer's view parameters, the
/// viewer's sector, and the target's human data lives in here.
#[derive(Debug, Clone, Copy)]
pub struct VisibilityQuery<'a> {
    /// Projected eye point used only for opaque line of sight.
    pub viewer_los: MapPoint,
    /// Original world-space eye point used for distance and cone geometry.
    pub viewer_world: WorldPoint3D,
    /// Viewer body direction, 16-sector compass (0 = north, CW).
    /// Used only for the close-range halfcircle test.
    pub viewer_direction: i16,
    /// Pre-computed view forward direction from `refresh_view`.
    /// The body direction rotated by the current `view_angle` (head
    /// turning, stare, etc.).  Used for the cone test and the
    /// forward-half rejection.
    pub view_forward: (f32, f32),
    /// World-units view radius.
    pub view_radius: u16,
    /// Eye status.  When set to a blind value (Closed /
    /// DieOrGetUnconscious) the visibility pipeline returns 0
    /// immediately.
    pub viewer_eye_status: EyeStatus,
    /// Half-angle of the vision cone after all modifiers (stare,
    /// drunk, lean-out).
    pub real_half_aperture: f32,
    /// `true` when the viewer's current sector has the BUILDING flag.
    pub viewer_in_building: bool,
    /// `true` when both viewer and target are in the SAME building
    /// sector — used for the "same building" half-visibility
    /// short-circuit:
    ///     if (viewer_in_building) {
    ///         if (target not in same building) return 0.0;
    ///         else                              return 0.5;
    ///     }
    /// Only consulted when `viewer_in_building == true`.
    pub target_in_same_building: bool,
    /// `true` when (a) the level is a forest level AND (b) the viewer
    /// is in `Camp::Royalists` — i.e. a merry man.  Triggers the 180°
    /// vision cone special case.  Only Royalist NPCs use this; the
    /// enemy AI tick (which iterates Lacklandists) leaves it false,
    /// but the field is here so the function is API-complete for
    /// future friendly-AI use.
    pub forest_180_degree_view: bool,
    /// Global `GoldenEyeMode` cheat — when on, NPCs are blind to PCs.
    pub golden_eye_mode: bool,

    /// Effective view radius after [`compute_view_radius`] — reduced
    /// by the ground-plane sphere projection (`sqrt(R² − Z²)`) and
    /// night/fog light-sector modulation.  When neither effect
    /// applies, equals `view_radius as f32`.
    pub effective_view_radius: f32,

    /// `mbActive && (sector == 0 || !sector_is_building)` for the
    /// target.  When false, the test returns 0.
    pub target_is_active_and_outside_building: bool,

    /// Projected detection point used only for opaque line of sight.
    pub target_los: MapPoint,
    /// Original world-space detection point used for distance and cone geometry.
    pub target_world: WorldPoint3D,
    /// Target posture (`Posture::Crouched`, `Posture::Spy`, etc).
    pub target_posture: Posture,
    /// Target action state (affects sharpness modifier).
    pub target_action_state: ActionState,
    /// Is the target a PC?  Drives the golden-eye check.
    pub target_is_pc: bool,

    /// Sight obstacle list plus FastFindGrid for the LOS check.
    pub sight_obstacles: crate::sight_obstacle::ObstacleList<'a>,
    pub fast_grid: &'a crate::fast_find_grid::FastFindGrid,
    pub layer: u16,
    /// `true` when the target is unconscious.  Short-circuits the
    /// same-building branch to 0.  Callers filter dead upstream, so
    /// only the unconscious part is exposed here.
    pub target_unconscious: bool,
    /// `true` when the target is mid-`Command::PassDoor` — used
    /// alongside `target_unconscious` to short-circuit the
    /// same-building branch to 0.
    pub target_passing_door: bool,
}

/// Returns a sharpness in `[0.0, RUNNING_DETECTION_FACTOR]` — 0.0 means
/// "not visible this frame", non-zero means the target is inside the
/// perception cone / halfcircle and the line of sight is clear.  The
/// caller multiplies this by `BASE_VIEW_SPEED` and accumulates into
/// the NPC's `detection_suspects[ENEMY]`.
pub fn compute_visibility(q: &VisibilityQuery<'_>) -> f32 {
    compute_visibility_with_effective_radius(q, || q.effective_view_radius)
}

/// Run the human visibility pipeline with a call-site-owned effective-radius
/// calculation. Original `ComputeVisibility(human)` calls `ComputeViewRadius`
/// only after all cheap semantic/range exits and the very-close auto-visible
/// branch. Keeping the calculation behind `FnOnce` preserves that observable
/// LOS/cache timing while the fixed-radius wrapper above remains convenient
/// for unit tests and callers without light-sector modulation.
pub fn compute_visibility_with_effective_radius(
    q: &VisibilityQuery<'_>,
    effective_view_radius: impl FnOnce() -> f32,
) -> f32 {
    // Golden-eye cheat: blind to PCs only.
    if q.golden_eye_mode && q.target_is_pc {
        return 0.0;
    }

    // PC disguises.
    match q.target_posture {
        Posture::Spy | Posture::Tree | Posture::AnonymousArcher => return 0.0,
        _ => {}
    }

    if q.viewer_eye_status.is_blind() {
        return 0.0;
    }

    // "I'm inside a building" short-circuit:
    //   if (viewer in building) {
    //     if (target not in same building
    //         || target dead || target unconscious
    //         || target passing door) return 0.0;
    //     else                         return 0.5;
    //   }
    if q.viewer_in_building {
        if !q.target_in_same_building || q.target_unconscious || q.target_passing_door {
            return 0.0;
        }
        return 0.5;
    }

    // If the target is inactive or inside a building, the NPC can't
    // see them from outside.
    if !q.target_is_active_and_outside_building {
        return 0.0;
    }

    // Forest-level 180° merry-men special case.  When the level is a
    // forest AND viewer is Royalist, vision becomes a flat 180°
    // detection (no narrow cone, no distance curve).  Currently only
    // consulted by friendly NPCs which the enemy-AI tick doesn't
    // process; the branch is wired anyway so future friendly-AI
    // ports get correct behaviour for free.
    if q.forest_180_degree_view {
        return if is_detecting_180_degrees(q, effective_view_radius) {
            1.0
        } else {
            0.0
        };
    }

    // Original `ComputeVisibility(human)` subtracts the world-space eye and
    // detection points, then stretches world Y. Projected map Y is reserved
    // for the later `IsReachable` call because elevation changes it.
    let dx = q.target_world.x - q.viewer_world.x;
    let dy = q.target_world.y - q.viewer_world.y;
    let sy = dy * INVERSE_ASPECT_RATIO;
    let sqr_distance = dx * dx + sy * sy;

    let (fx, fy) = q.view_forward;

    // Reject when far OR (behind AND beyond the close-range halfcircle).
    let view_radius = q.view_radius as f32;
    let view_dot_dir = dx * fx + sy * fy;
    if sqr_distance > view_radius * view_radius
        || (view_dot_dir < 0.0 && sqr_distance > SQR_HALFCIRCLE_VIEW_RADIUS)
    {
        return 0.0;
    }

    // Very-close auto-visible.  Uses the 3D eye-to-target vector
    // (viewer eye-point to target detection-point).  Adding the Z
    // component tightens the gate so a target well above or below
    // the viewer isn't auto-spotted at short horizontal range.
    let dz = q.target_world.z - q.viewer_world.z;
    let sqr_distance_3d = sqr_distance + dz * dz;
    if sqr_distance_3d < 400.0 {
        return 1.0;
    }

    // Secondary distance check using the effective view radius from
    // [`compute_view_radius`].  Callers pass the per-target effective
    // radius, including ground/obstacle sphere projection and
    // night/fog light-sector modulation.
    let effective_view_radius = effective_view_radius();
    let sqr_effective = effective_view_radius * effective_view_radius;
    if sqr_distance > sqr_effective {
        return 0.0;
    }

    // Cone + LOS test.
    // IsDetecting uses the stretched 3D norm to choose its close-range
    // branch, but its cone determinants consume the raw world-space XY
    // vector.  Do not leak the distance-only Y stretch into that geometry.
    if !is_detecting(q, dx, dy, sqr_distance_3d, fx, fy, false) {
        return 0.0;
    }

    let sharpness = distance_sharpness(sqr_distance_3d, view_radius);

    // Posture / action-state sharpness modifier.
    if q.target_posture == Posture::Crouched {
        return CROUCHED_DETECTION_FACTOR * sharpness;
    }
    match q.target_action_state {
        ActionState::Moving | ActionState::MovingShield | ActionState::MovingSword => {
            WALKING_DETECTION_FACTOR * sharpness
        }
        ActionState::MovingFast | ActionState::MovingFastSword => {
            RUNNING_DETECTION_FACTOR * sharpness
        }
        ActionState::AimingWithBow
        | ActionState::AimingWithBowUp
        | ActionState::AimingWithBowDown => BOW_DETECTION_FACTOR * sharpness,
        ActionState::WaitingSword | ActionState::ParryingSword | ActionState::ParryingSwordLow => {
            SWORD_DETECTION_FACTOR * sharpness
        }
        _ => sharpness,
    }
}

/// Context the caller passes in for a `compute_object_visibility` run.
/// The object path is simpler than the human path — no posture /
/// action-state multipliers, no same-building half-vis, no golden-eye,
/// no "active and outside" gate, no forest-180°, no `< 400`
/// auto-visible, no effective-radius shrink; only the eye-status /
/// in-building / `belongs_to_beggar` short-circuits and the radius +
/// forward-halfplane + cone + LOS gate.
#[derive(Debug, Clone, Copy)]
pub struct ObjectVisibilityQuery<'a> {
    /// Projected eye point used only for opaque line of sight.
    pub viewer_los: MapPoint,
    /// Original world-space eye point used for distance and cone geometry.
    pub viewer_world: WorldPoint3D,
    /// Viewer body direction, 16-sector compass (0 = north, CW).
    /// Used only for the close-range halfcircle test inside
    /// `is_detecting` — though note the object path rejects any
    /// `view_dot_dir < 0` before that test runs, so the wrap-around
    /// sectors {13,14,15} can never trigger.
    pub viewer_direction: i16,
    /// Pre-computed view forward direction from `refresh_view`.
    pub view_forward: (f32, f32),
    pub view_radius: u16,
    pub viewer_eye_status: EyeStatus,
    pub real_half_aperture: f32,
    /// `true` when the viewer's current sector has the BUILDING flag.
    /// Unlike the human path there is no "same building → 0.5"
    /// branch: an object inside a building is always 0 for an indoor
    /// viewer.
    pub viewer_in_building: bool,
    /// `true` for beggar-dropped coins, which are invisible to
    /// soldiers.
    pub object_belongs_to_beggar: bool,
    /// Projected object point used only for opaque line of sight.
    pub target_los: MapPoint,
    /// Original world-space object point (`GetPosition()`, with Z + 1).
    pub target_world: WorldPoint3D,
    /// Sight obstacle list plus FastFindGrid for the LOS check.
    pub sight_obstacles: crate::sight_obstacle::ObstacleList<'a>,
    pub fast_grid: &'a crate::fast_find_grid::FastFindGrid,
    pub layer: u16,
}

/// Returns a sharpness in `[0.0, 1.0]` — 0.0 means "not visible this
/// frame", non-zero is the distance-sharpness curve result with no
/// posture or action-state multiplier (objects are inanimate).
pub fn compute_object_visibility(q: &ObjectVisibilityQuery<'_>) -> f32 {
    if q.viewer_eye_status.is_blind() {
        return 0.0;
    }

    // Viewer indoors.  Unlike the human path, no "same building →
    // 0.5" branch: objects inside a building are always invisible to
    // a viewer who is also in one.
    if q.viewer_in_building {
        return 0.0;
    }

    // Beggar-dropped items are invisible to soldiers.
    // `belongs_to_beggar` is set on beggar-simulating PC coin drops
    // by `engine::beggar::simulate_beggar_drop` so guards don't
    // confiscate the handouts.
    if q.object_belongs_to_beggar {
        return 0.0;
    }

    // Original object visibility uses the complete stretched world-space
    // vector for radius, half-circle, and sharpness checks.
    let dx = q.target_world.x - q.viewer_world.x;
    let dy = q.target_world.y - q.viewer_world.y;
    let sy = dy * INVERSE_ASPECT_RATIO;
    let dz = q.target_world.z - q.viewer_world.z;
    let sqr_distance = dx * dx + sy * sy + dz * dz;

    let (fx, fy) = q.view_forward;
    let view_radius = q.view_radius as f32;
    let view_dot_dir = dx * fx + sy * fy;

    // Reject if far OR behind.  Note the difference from the human
    // path, which forgives "behind" for targets within the close-range
    // halfcircle: the object path unconditionally rejects any target
    // whose 2D view vector has a negative dot with the view direction.
    // Objects therefore get no peripheral close-range detection.
    if sqr_distance > view_radius * view_radius || view_dot_dir < 0.0 {
        return 0.0;
    }

    // Cone + LOS test.  The radius / forward-halfplane rejection
    // above already covers the `check_distance=true` branch inside
    // `is_detecting`, so we go straight to the cone + LOS core; the
    // eye-status and viewer-in-building short-circuits inside
    // `is_detecting` are redundant with the ones we already ran.
    if !is_detecting_cone_and_los(
        q.viewer_los,
        q.viewer_direction,
        q.real_half_aperture,
        q.layer,
        q.sight_obstacles,
        q.fast_grid,
        q.target_los,
        Some((q.viewer_world, q.target_world)),
        dx,
        dy,
        sqr_distance,
        fx,
        fy,
    ) {
        return 0.0;
    }

    // Raw distance-sharpness curve — objects are inanimate, so no
    // posture or action-state multiplier.
    distance_sharpness(sqr_distance, view_radius)
}

/// Cone / halfcircle test plus the line-of-sight raycast.
///
/// `check_distance` is the "re-apply distance gate" toggle: when true,
/// the "normal detection" (outside the close-range halfcircle) branch
/// re-applies the radius + forward-halfplane reject before the triangle
/// test.  `compute_visibility` passes false because it already ran an
/// equivalent reject upstream.
fn is_detecting(
    q: &VisibilityQuery<'_>,
    view_x: f32,
    view_y: f32,
    sqr_distance: f32,
    fx: f32,
    fy: f32,
    check_distance: bool,
) -> bool {
    if q.viewer_eye_status.is_blind() {
        return false;
    }

    if q.viewer_in_building {
        return false;
    }

    // `check_distance` re-check in the normal-detection (outside
    // halfcircle) branch.  Inside the halfcircle the test ignores
    // `check_distance` entirely, so gate this on
    // `sqr > SQR_HALFCIRCLE_VIEW_RADIUS`.
    if check_distance && sqr_distance > SQR_HALFCIRCLE_VIEW_RADIUS {
        let view_radius = q.view_radius as f32;
        let view_dot_dir = view_x * fx + view_y * fy;
        if sqr_distance > view_radius * view_radius || view_dot_dir < 0.0 {
            return false;
        }
    }

    is_detecting_cone_and_los(
        q.viewer_los,
        q.viewer_direction,
        q.real_half_aperture,
        q.layer,
        q.sight_obstacles,
        q.fast_grid,
        q.target_los,
        Some((q.viewer_world, q.target_world)),
        view_x,
        view_y,
        sqr_distance,
        fx,
        fy,
    )
}

/// The bare cone/halfcircle + LOS test, with the eye-status /
/// viewer-in-building short-circuits already handled by the caller.
/// Shared by `compute_visibility` (human target) and
/// `compute_object_visibility` (object target), both of which check
/// those short-circuits earlier in their own bodies.
#[allow(clippy::too_many_arguments)]
fn is_detecting_cone_and_los(
    viewer: MapPoint,
    viewer_direction: i16,
    real_half_aperture: f32,
    layer: u16,
    sight_obstacles: ObstacleList<'_>,
    fast_grid: &crate::fast_find_grid::FastFindGrid,
    target: MapPoint,
    los_world: Option<(WorldPoint3D, WorldPoint3D)>,
    view_x: f32,
    view_y: f32,
    sqr_distance: f32,
    fx: f32,
    fy: f32,
) -> bool {
    if sqr_distance > SQR_HALFCIRCLE_VIEW_RADIUS {
        // ── Normal detection ─────────────────────────────────────
        //
        // `check_distance` is false in the `compute_visibility`
        // path, so skip the inner radius / forward-half-plane
        // re-check and go straight to the triangle test:
        //     left.Det(viewVector) < 0 || right.Det(viewVector) > 0
        // where `left` and `right` start as unit vectors set to the view
        // direction rotated by ±real_half_aperture, then have their Y
        // component multiplied by ASPECT_RATIO. Sides include stare, drunk,
        // and lean-out modifiers from `refresh_view`.
        //
        // The original constructs `viewVector` directly from the raw
        // world-space X/Y delta for these determinants. Its separately
        // stretched 3D norm is used only to select this branch; the isometric
        // correction is encoded in the precomputed side vectors instead.
        let ha = real_half_aperture;
        let (lx, mut ly) = rotate_unit(fx, fy, -ha);
        let (rx, mut ry) = rotate_unit(fx, fy, ha);
        ly *= ASPECT_RATIO;
        ry *= ASPECT_RATIO;
        let det_left = lx * view_y - ly * view_x;
        let det_right = rx * view_y - ry * view_x;
        if det_left < 0.0 || det_right > 0.0 {
            return false;
        }
        los_clear_for_detection(viewer, target, layer, sight_obstacles, fast_grid, los_world)
    } else {
        // ── Close-range halfcircle ───────────────────────────────
        //
        //     diff = (viewVector.sector_0_to_15(ASPECT_RATIO)
        //             + 16 - body_direction) % 16
        //     if diff in {12..4} (with wrap): LOS
        //     else:                           false
        //
        // The sector lookup applies ASPECT_RATIO to the raw view vector,
        // matching `viewVector.GetSector0to15(ASPECT_RATIO)`.
        let view_sector = crate::position_interface::vector_to_sector_0_to_15_iso(view_x, view_y);
        let diff = (view_sector - viewer_direction).rem_euclid(16);
        if matches!(diff, 0 | 1 | 2 | 3 | 4 | 12 | 13 | 14 | 15) {
            los_clear_for_detection(viewer, target, layer, sight_obstacles, fast_grid, los_world)
        } else {
            false
        }
    }
}

fn los_clear_for_detection(
    viewer: MapPoint,
    target: MapPoint,
    layer: u16,
    sight_obstacles: ObstacleList<'_>,
    fast_grid: &crate::fast_find_grid::FastFindGrid,
    los_world: Option<(WorldPoint3D, WorldPoint3D)>,
) -> bool {
    if let Some((viewer_world, target_world)) = los_world {
        return crate::sight_obstacle::is_reachable_3d(
            sight_obstacles,
            [viewer_world.x, viewer_world.y, viewer_world.z],
            [target_world.x, target_world.y, target_world.z],
            crate::sight_obstacle::SIGHTOBSTACLE_OPAQUE,
        );
    }
    los_clear_spatial(viewer, target, layer, sight_obstacles, fast_grid)
}

/// Merry-men forest-level 180° view: vision is a flat 180° forward
/// half-plane bounded by the view radius — no narrow cone, no distance
/// curve.
fn is_detecting_180_degrees(
    q: &VisibilityQuery<'_>,
    effective_view_radius: impl FnOnce() -> f32,
) -> bool {
    let dx = q.target_world.x - q.viewer_world.x;
    let dy = q.target_world.y - q.viewer_world.y;
    let sy = dy * INVERSE_ASPECT_RATIO;
    let sqr_distance = dx * dx + sy * sy;
    let view_radius = q.view_radius as f32;
    if sqr_distance > view_radius * view_radius {
        return false;
    }

    // Very-near perpendicular detection.  Within 50 units, check if
    // the target is more to the side than forward/backward — "beside"
    // the viewer.  If so, return detected immediately, with no LOS
    // check.
    if sqr_distance < 50.0 * 50.0 {
        // Original uses GetDirectionVector(), not the independently mutable
        // RefreshView direction. GetDirectionVector's map-space Y is already
        // compressed by ASPECT_RATIO, then this function expands it again,
        // leaving the ordinary unit body vector in stretched coordinates.
        let (body_x, body_y) = sector_to_forward(q.viewer_direction);
        let fwd_len = dx * body_x + sy * body_y;
        let fc_x = body_x * fwd_len;
        let fc_y = body_y * fwd_len;
        let perp_sq = (dx - fc_x) * (dx - fc_x) + (sy - fc_y) * (sy - fc_y);
        if perp_sq >= fwd_len {
            return true;
        }
    }

    // 180° forward: dot product with view direction ≥ 0.
    let (body_x, body_y) = sector_to_forward(q.viewer_direction);
    if dx * body_x + sy * body_y < 0.0 {
        return false;
    }

    // Second, tighter radius gate using the effective radius
    // (night/fog modulation + obstacle-projection shrink).  Without
    // this, a Royalist NPC on a dark/foggy forest level would detect
    // Lacklandist targets at the full unmodulated radius.
    // Keep this lazy: Original does not call ComputeViewRadius for a target
    // rejected by the full-radius, near-side, or forward-halfplane gates.
    let effective_view_radius = effective_view_radius();
    if sqr_distance > effective_view_radius * effective_view_radius {
        return false;
    }

    crate::sight_obstacle::is_reachable_3d(
        q.sight_obstacles,
        [q.viewer_world.x, q.viewer_world.y, q.viewer_world.z],
        [q.target_world.x, q.target_world.y, q.target_world.z],
        crate::sight_obstacle::SIGHTOBSTACLE_OPAQUE,
    )
}

/// Three-segment distance sharpness curve.
///
/// Called after the cone test with the already-computed squared
/// distance and per-NPC view radius, to avoid re-doing the same
/// subtraction.
pub fn distance_sharpness(sqr_distance: f32, view_radius: f32) -> f32 {
    // Close-detection short-circuit.  Redundant with the `< 400`
    // early-return in `compute_visibility`, but kept for direct use.
    if sqr_distance <= SQR_HALFCIRCLE_VIEW_RADIUS {
        return 1.0;
    }
    let rel_dist = sqr_distance.sqrt() / view_radius;
    let sharpness = if rel_dist < DETECTION_CURVE_X_1 {
        1.0 + (DETECTION_CURVE_Y_1 - 1.0) * rel_dist / DETECTION_CURVE_X_1
    } else if rel_dist < DETECTION_CURVE_X_2 {
        DETECTION_CURVE_Y_1
            - (DETECTION_CURVE_Y_1 - DETECTION_CURVE_Y_2) * (rel_dist - DETECTION_CURVE_X_1)
                / (DETECTION_CURVE_X_2 - DETECTION_CURVE_X_1)
    } else {
        DETECTION_CURVE_Y_2
            - (DETECTION_CURVE_Y_2 - DETECTION_CURVE_Y_3) * (rel_dist - DETECTION_CURVE_X_2)
                / (1.0 - DETECTION_CURVE_X_2)
    };
    sharpness.max(0.0)
}

// ─── compute_view_radius (night/fog) ────────────────────────────

/// Computes the effective view radius after:
/// 1. Ground-plane sphere projection: `sqrt(R² − Z²)` — converts the
///    3D view sphere to a ground-plane circle given the eye height.
/// 2. Night/fog light-sector modulation: nearby SHADOW sectors act as
///    light sources at night; the view radius is scaled down when far
///    from any light.
///
/// `target_obstacle` is the target human's projection obstacle, e.g.
/// an elevated platform the target is standing on.  When `Some`, the
/// base radius is computed by slicing the view sphere with the
/// obstacle's top plane — if the eye is farther than R from the plane,
/// the sphere doesn't cross it and the radius is 0.  The night/fog
/// block then scopes the shadow-sector lookup to the obstacle's layer
/// and subtracts the obstacle's top-plane Z from the screen-space
/// reference points so distance falloff stays consistent.
///
/// `eye_world` must be the full world-space point returned by
/// `ComputeEyesPoint`. Obstacle planes and shadow-sector barycentres use
/// world coordinates; passing the isometrically projected map Y here mixes
/// coordinate spaces whenever the viewer is elevated.
///
/// The per-frame cache from the legacy implementation is
/// intentionally skipped — we recompute once per call.
#[allow(clippy::too_many_arguments)]
pub fn compute_view_radius(
    eye_world: WorldPoint3D,
    view_radius: u16,
    view_forward: (f32, f32),
    half_aperture: f32,
    is_night_or_fog: bool,
    level: &crate::fast_find_grid::LevelGrid,
    sight_obstacles: ObstacleList<'_>,
    target_obstacle: Option<&SightObstacle>,
) -> f32 {
    let r = view_radius as f32;

    // Base radius.  Without an obstacle, slice the view sphere with
    // the ground plane (`sqrt(R² − Z²)`).  With an obstacle, slice it
    // with the obstacle's top plane: project the eye onto the plane
    // and compute `sqrt(R² − d²)` where `d` is the signed plane
    // distance.  If the eye is farther than R from the plane, the
    // sphere doesn't intersect it and the radius is 0.
    let base_radius = if let Some(obs) = target_obstacle {
        let origin = obs.top_plane_origin();
        let normal = obs.top_plane_normal();
        let rel = [
            eye_world.x - origin[0],
            eye_world.y - origin[1],
            eye_world.z - origin[2],
        ];
        let f_distance = rel[0] * normal[0] + rel[1] * normal[1] + rel[2] * normal[2];
        if f_distance.abs() >= r {
            return 0.0;
        }
        (r * r - f_distance * f_distance).sqrt()
    } else {
        (r * r - eye_world.z * eye_world.z).abs().sqrt()
    };

    if !is_night_or_fog {
        return base_radius;
    }

    // Night/fog light-sector modulation.  Three reference points
    // sample the light field: one along the view forward direction,
    // and two along the left/right cone edges.
    let [mut pt_ref, mut pt_left, mut pt_right] =
        night_fog_light_reference_points(eye_world, base_radius, view_forward, half_aperture);

    // When the target sits on a projection obstacle, subtract the
    // obstacle's top-plane Z from each reference point's Y.  This is
    // a screen-space (iso) correction: an elevated ref point projects
    // "higher" on screen, so its 2D distance to a shadow barycentre
    // (which lives in screen-space here) reduces by the Z component
    // along the iso Y axis.
    if let Some(obs) = target_obstacle {
        pt_ref.y -= obs.compute_top_z(pt_ref.x, pt_ref.y);
        pt_left.y -= obs.compute_top_z(pt_left.x, pt_left.y);
        pt_right.y -= obs.compute_top_z(pt_right.x, pt_right.y);
    }

    // Shadow-sector lookup is scoped to the obstacle's layer when
    // present, else layer 0.
    let shadow_layer: u16 = target_obstacle.map(|o| o.layer).unwrap_or(0);

    let mut factor_result: f32 = 0.5;

    for (sector_idx, gs) in level.sectors.iter().enumerate() {
        if !gs.sector_type.is_shadow() || gs.points.is_empty() {
            continue;
        }
        if gs.layer != shadow_layer {
            continue;
        }

        // Use the precomputed centroid + 3D barycentre from
        // `RHSectorShadow::Initialize` (`level.shadow_data` populated
        // in the post-load pass at
        // `level_loading.rs::initialize_motion_from_level_data`).  The
        // 3D barycentre is needed for the `is_reachable` ray; if it
        // is missing, the load pipeline is incomplete and this
        // sector cannot be sampled correctly.
        let Some(shadow) = level.shadow_data.get(&(sector_idx as u32)) else {
            tracing::warn!(
                sector_idx,
                "shadow sector missing precomputed barycentre; skipping compute_view_radius sample"
            );
            continue;
        };
        let bary = shadow.barycentre_2d;
        let bary_3d = [
            shadow.barycentre_3d_x,
            shadow.barycentre_3d_y,
            shadow.barycentre_3d_z,
        ];

        // Quick 400-unit bounding-box reject.
        let dx = pt_ref.x - bary.x;
        let dy = pt_ref.y - bary.y;
        if dx * dx + dy * dy > 400.0 * 400.0 {
            continue;
        }

        // 3D LOS check to the light source — segment-vs-obstacle test
        // using the precomputed 3D barycentre.  Using the 3D variant
        // ensures elevated shadow barycentres blocked by a ramp are
        // correctly rejected instead of passing a 2D-only test.
        let los_ok = crate::sight_obstacle::is_reachable_3d(
            sight_obstacles,
            [eye_world.x, eye_world.y, eye_world.z],
            bary_3d,
            crate::sight_obstacle::SIGHTOBSTACLE_OPAQUE,
        );
        if !los_ok {
            continue;
        }

        // Distance-based factor for each reference point.
        for ref_pt in [pt_ref, pt_left, pt_right] {
            let dist = ((ref_pt.x - bary.x).powi(2) + (ref_pt.y - bary.y).powi(2)).sqrt();
            let f = if dist < 10.0 {
                1.0
            } else if dist > 110.0 {
                0.5
            } else {
                1.05 - 0.005 * dist
            };
            if f > factor_result {
                factor_result = f;
            }
        }
    }

    // Clamp to [0.5, 1.0], remap to [0.0, 1.0].
    factor_result = factor_result.clamp(0.5, 1.0);
    factor_result = 2.0 * factor_result - 1.0;

    // Blend between base_radius and DEFAULT_VIEW_RADIUS.
    base_radius * (1.0 - factor_result) + DEFAULT_VIEW_RADIUS as f32 * factor_result
}

/// Original stores the cone sides with their map-space Y compressed by
/// `ASPECT_RATIO`, while the central view direction remains an ordinary unit
/// vector. ComputeViewRadius samples exactly those three differently scaled
/// vectors when looking for a nearby light sector at night or in fog.
fn night_fog_light_reference_points(
    eye_world: WorldPoint3D,
    radius: f32,
    view_forward: (f32, f32),
    half_aperture: f32,
) -> [MapPoint; 3] {
    let (fx, fy) = view_forward;
    let half_r = 0.5 * radius;
    let forward = MapPoint {
        x: eye_world.x + half_r * fx,
        y: eye_world.y + half_r * fy,
    };

    let (lx, ly) = rotate_unit(fx, fy, -half_aperture);
    let (rx, ry) = rotate_unit(fx, fy, half_aperture);
    let left = MapPoint {
        x: eye_world.x + half_r * lx,
        y: eye_world.y + half_r * ly * ASPECT_RATIO,
    };
    let right = MapPoint {
        x: eye_world.x + half_r * rx,
        y: eye_world.y + half_r * ry * ASPECT_RATIO,
    };
    [forward, left, right]
}

// ─── Helpers ─────────────────────────────────────────────────────

/// Unit forward vector for a 16-sector compass direction (0 = north,
/// CW).  Plain unit vector in true world space (no aspect-ratio
/// stretch).
pub fn sector_to_forward(sector: i16) -> (f32, f32) {
    let s = sector.rem_euclid(16);
    let angle = (s as f32) * (std::f32::consts::TAU / 16.0) - std::f32::consts::FRAC_PI_2;
    (angle.cos(), angle.sin())
}

/// Rotate the unit vector `(x, y)` by `theta` radians.
/// CCW for positive θ in standard math convention, which corresponds
/// to screen-clockwise since the game's Y axis points down.
fn rotate_unit(x: f32, y: f32, theta: f32) -> (f32, f32) {
    let (s, c) = theta.sin_cos();
    (x * c - y * s, x * s + y * c)
}

/// LOS query: ask the grid for obstacles crossed by the ray, then run
/// the opaque / layer / polygon checks on that smaller candidate list.
pub fn los_clear_spatial(
    viewer: MapPoint,
    target: MapPoint,
    layer: u16,
    obstacles: ObstacleList<'_>,
    fast_grid: &crate::fast_find_grid::FastFindGrid,
) -> bool {
    // The obstacle grid is keyed by `SightObstacle::box_ground`.
    // Legacy visibility passes projected 2D positions here; preserve
    // that parity explicitly at the conversion boundary.
    let viewer_ground = GroundPoint::new(viewer.x, viewer.y);
    let target_ground = GroundPoint::new(target.x, target.y);
    let bbox = GroundBBox::from_corners(
        GroundPoint::new(
            viewer_ground.x.min(target_ground.x),
            viewer_ground.y.min(target_ground.y),
        ),
        GroundPoint::new(
            viewer_ground.x.max(target_ground.x),
            viewer_ground.y.max(target_ground.y),
        ),
    );

    let mut candidates = fast_grid.get_obstacle_indices(0, &bbox);
    if layer != 0 {
        for idx in fast_grid.get_obstacle_indices(layer, &bbox) {
            if !candidates.contains(&idx) {
                candidates.push(idx);
            }
        }
    }

    for idx in candidates {
        let idx = usize::from(idx);
        let Some(obs) = obstacles.get(idx) else {
            tracing::warn!(
                obstacle_index = idx,
                "fast grid returned missing sight obstacle index"
            );
            continue;
        };
        if !obstacles.is_active(idx) || !obs.is_opaque() {
            continue;
        }
        if obs.layer != u16::MAX && obs.layer != layer {
            continue;
        }
        if obs.is_blocking_sight(viewer_ground, target_ground) {
            return false;
        }
    }

    let static_len = obstacles.static_obstacles.len();
    for (dyn_offset, obs) in obstacles.dynamic_obstacles.iter().enumerate() {
        let idx = static_len + dyn_offset;
        if !obstacles.is_active(idx) || !obs.is_opaque() {
            continue;
        }
        if obs.layer != u16::MAX && obs.layer != layer {
            continue;
        }
        if obs.is_blocking_sight(
            GroundPoint::new(viewer.x, viewer.y),
            GroundPoint::new(target.x, target.y),
        ) {
            return false;
        }
    }

    true
}

/// Standalone radius + cone (or close-range halfcircle) + opaque-LOS
/// check from raw 2D inputs — no [`VisibilityQuery`] required.  Used
/// by AI populators that snapshot the result on per-tick records
/// (e.g. `CampSoldierInfo::is_detecting_cone`) so per-call sites
/// don't redo the geometry per fighter pair.
///
/// Caller is responsible for the eye-blind / viewer-in-building /
/// target-in-building short-circuits; those depend on state outside
/// the raw inputs.
#[allow(clippy::too_many_arguments)]
pub fn is_detecting_target(
    viewer_los: MapPoint,
    viewer_world: GroundPoint,
    viewer_direction: i16,
    view_forward: (f32, f32),
    real_half_aperture: f32,
    view_radius: u16,
    target_los: MapPoint,
    target_world: GroundPoint,
    layer: u16,
    obstacles: ObstacleList<'_>,
    fast_grid: &crate::fast_find_grid::FastFindGrid,
) -> bool {
    let dx = target_world.x - viewer_world.x;
    let dy = target_world.y - viewer_world.y;
    let sy = dy * INVERSE_ASPECT_RATIO;
    let sqr_distance = dx * dx + sy * sy;
    let view_radius_f = view_radius as f32;
    if sqr_distance > view_radius_f * view_radius_f {
        return false;
    }
    let (fx, fy) = view_forward;
    // Forward-halfplane reject — applies only outside the close-range
    // halfcircle.
    let view_dot_dir = dx * fx + sy * fy;
    if view_dot_dir < 0.0 && sqr_distance > SQR_HALFCIRCLE_VIEW_RADIUS {
        return false;
    }
    is_detecting_cone_and_los(
        viewer_los,
        viewer_direction,
        real_half_aperture,
        layer,
        obstacles,
        fast_grid,
        target_los,
        None,
        dx,
        dy,
        sqr_distance,
        fx,
        fy,
    )
}

// ─── refresh_view ───────────────────────────────────────────────

/// Read-only context gathered from the entity each frame, passed to
/// [`refresh_view`] alongside the mutable [`NpcData`].
#[derive(Debug, Clone)]
pub struct RefreshViewContext {
    /// Current body direction sector (0-15).
    pub body_direction: i16,
    /// Current posture.
    pub posture: Posture,
    /// Current animation (order type), for look-left/right validation.
    pub animation: Option<OrderType>,
    /// Whether the NPC is unconscious.
    pub is_unconscious: bool,
    /// Whether the NPC is tied up.
    pub is_tied: bool,
    /// Whether the NPC is dead.
    pub is_dead: bool,
    /// Whether the NPC is active and outside any building sector.
    pub is_active_and_outside_building: bool,
    /// Whether this NPC is mounted on a horse.
    pub is_rider: bool,
    /// Current blood-alcohol level (0-255), sourced from the AI brain
    /// each frame.  Drives the drunken vision-cone wobble.  Passed
    /// through the context (rather than read from `NpcData`) because
    /// the canonical value lives on `AiController` and mutates at
    /// runtime via script natives and the swordfight drunk-roll path
    /// — a cached copy on `NpcData` would go stale.
    pub blood_alcohol: u8,
    /// NPC's own projected map position (for stare/follow vector).
    /// Original `GetPositionGround()`: world-space `(position.x, position.y)`.
    pub own_position: GroundPoint,
    /// Projected map position of the follow target, if [`EyeStatus::Follow`]
    /// and the target is alive.
    /// Original followed element's `GetPositionGround()`.
    pub follow_target_position: Option<GroundPoint>,
}

/// Per-frame view parameter update.
///
/// Mutates the NPC's view state fields (direction, radius, aperture,
/// eye status) based on posture, animation, drunkenness, stare
/// targets, etc.  The caller must invoke this once per frame for every
/// active NPC, then read back `view_direction`, `view_radius`,
/// `real_half_aperture`, and `eye_status` when constructing
/// [`VisibilityQuery`] values or rendering the shadow polygon.
///
/// Note: the [`EyeStatus::LookToTheLeft`] / [`EyeStatus::LookToTheRight`]
/// transitions that the bored head-turn animations drive are
/// event-driven from the animation motion-state dispatcher in
/// [`crate::engine::animation::apply_soldier_execute_side_effects`]
/// / [`crate::engine::animation::apply_npc_execute_side_effects`].
/// They fire `set_view_status` exactly once per motion start/done
/// edge, so `refresh_view` does not re-assert them each frame.  The
/// defensive reset inside the `LookToTheLeft` / `LookToTheRight` arm
/// of [`refresh_view_look`] still clears a stale look-status the
/// moment the animation is no longer playing.
pub fn refresh_view(npc: &mut NpcData, ctx: &RefreshViewContext) {
    // Symptom therapy: unconscious/tied/dead NPCs that somehow have
    // a non-closed status get forced to Closed.
    if npc.eye_status != EyeStatus::DieOrGetUnconscious
        && (ctx.is_unconscious || ctx.is_tied || ctx.is_dead)
    {
        npc.eye_status = EyeStatus::Closed;
    }

    // Body direction as a unit vector.
    let body_dir = ctx.body_direction;
    let (vdx, vdy) = sector_to_forward(body_dir);

    // Direction-change tracking.  When the body rotates, compensate
    // the view angle so the cone doesn't snap instantly; instead it
    // transitions smoothly.
    if npc.direction_old != body_dir {
        let mut steps = (body_dir - npc.direction_old) as i8;
        // Set steps between -7 and 8.
        steps = (((steps as i16 + 23) & 15) - 7) as i8;

        npc.view_angle -= steps as f32 * std::f32::consts::FRAC_PI_8;
        // When the body rotation clamps the angle at ±π/2, also
        // immediately set the view direction to the body direction's
        // normal (±π/2 rotation).  This way EyeStatus branches that
        // do not rerun the direction computation post-switch
        // (DieOrGetUnconscious, Closed) still observe the
        // perpendicular cone facing.
        if npc.view_angle < -std::f32::consts::FRAC_PI_2 {
            npc.view_angle = -std::f32::consts::FRAC_PI_2;
            npc.view_direction = [vdy, -vdx];
        } else if npc.view_angle > std::f32::consts::FRAC_PI_2 {
            npc.view_angle = std::f32::consts::FRAC_PI_2;
            npc.view_direction = [-vdy, vdx];
        }

        npc.view_transition = true;
        npc.direction_old = body_dir;
    }

    // Lean-out posture.
    if ctx.posture == Posture::LeaningOut {
        set_view_status(npc, EyeStatus::LookDownwards);
        npc.view_lean_out = true;
    } else if npc.eye_status == EyeStatus::LookDownwards {
        set_view_status(npc, EyeStatus::LookForward);
        npc.view_lean_out = false;
    }

    // Main per-status logic (only when not closed AND active outside
    // a building).
    if npc.eye_status != EyeStatus::Closed && ctx.is_active_and_outside_building {
        match npc.eye_status {
            EyeStatus::ViewconeGrow => {
                // Grow radius by 8 per frame.
                npc.view_radius_base = npc.view_radius_base.saturating_add(8);
                if npc.view_radius_base >= npc.view_radius_goal {
                    npc.view_radius_base = npc.view_radius_goal;
                    npc.eye_status = EyeStatus::LookForward;
                }
                // Fall through to LookForward.
                refresh_view_look(npc, ctx, vdx, vdy);
            }

            EyeStatus::LookForward | EyeStatus::LookToTheLeft | EyeStatus::LookToTheRight => {
                refresh_view_look(npc, ctx, vdx, vdy);
            }

            EyeStatus::LookDownwards => {
                // Lean-out: wide aperture, no angle.
                npc.view_angle = 0.0;
                npc.view_angle_iterator = 0.0;
                npc.view_transition = false;
                npc.half_aperture = std::f32::consts::FRAC_PI_2 - 0.05;
                npc.view_direction = [vdx, vdy];
            }

            EyeStatus::DieOrGetUnconscious => {
                // Shrink view cone on death.
                npc.view_alpha_start = npc.view_alpha_start.saturating_sub(5);
                let new_goal = npc.view_radius_goal as i16 - npc.view_radius_step as i16;
                npc.view_radius_step = npc.view_radius_step.saturating_add(5);

                if new_goal < 0 {
                    npc.view_radius_goal = 0;
                    set_view_status(npc, EyeStatus::Closed);
                } else {
                    npc.view_radius_goal = new_goal as u16;
                }
            }

            EyeStatus::Follow => {
                // Update stare point from target.
                if let Some(pos) = ctx.follow_target_position {
                    npc.stare_point = pos;
                }
                // Fall through to Stare.
                refresh_view_stare(npc, vdx, vdy, &ctx.own_position);
            }

            EyeStatus::Stare => {
                refresh_view_stare(npc, vdx, vdy, &ctx.own_position);
            }

            EyeStatus::Closed => {}
        }

        // Post-processing: real values from base.
        npc.real_half_aperture = npc.half_aperture;
        npc.view_radius = (npc.view_radius_base as f32 * npc.view_longrange_radius_factor) as u16;

        // Stare/follow: narrow aperture, extend range.
        if matches!(npc.eye_status, EyeStatus::Follow | EyeStatus::Stare) {
            npc.real_half_aperture *= STARE_APERTURE_FACTOR;
            npc.view_radius = (npc.view_radius as f32 * STARE_RANGE_FACTOR) as u16;
        }

        // Rider modifier.
        if ctx.is_rider {
            npc.view_radius = (npc.view_radius as f32 * RIDER_VIEW_RADIUS_FACTOR) as u16;
        }

        // Drunken cone wobble.
        if ctx.blood_alcohol > 0 {
            const DRUNKEN_SPEEDS: [f32; 4] = [0.1000, 0.07634, 0.12321, 0.04546];
            for (iter, &speed) in npc.drunken_cone_iterators.iter_mut().zip(&DRUNKEN_SPEEDS) {
                *iter += speed;
                if *iter > std::f32::consts::TAU {
                    *iter -= std::f32::consts::TAU;
                }
            }

            let drunk_factor = 0.01 * ctx.blood_alcohol as f32;
            let radius_factor = 1.0
                - drunk_factor
                    * (0.6
                        + 0.1 * npc.drunken_cone_iterators[0].cos()
                        + 0.1 * npc.drunken_cone_iterators[1].sin());
            let aperture_factor = 1.0
                - drunk_factor
                    * (0.2
                        + 0.1 * npc.drunken_cone_iterators[2].sin()
                        + 0.1 * npc.drunken_cone_iterators[3].cos());

            npc.view_radius = (npc.view_radius as f32 * radius_factor) as u16;
            npc.real_half_aperture *= aperture_factor;

            if npc.real_half_aperture > 0.95 {
                npc.real_half_aperture = 0.95;
            }
        }

        let (left_x, mut left_y) = rotate_unit(
            npc.view_direction[0],
            npc.view_direction[1],
            -npc.real_half_aperture,
        );
        left_y *= crate::position_interface::ASPECT_RATIO;
        npc.view_left_side = [left_x, left_y];
        let (right_x, mut right_y) = rotate_unit(
            npc.view_direction[0],
            npc.view_direction[1],
            npc.real_half_aperture,
        );
        right_y *= crate::position_interface::ASPECT_RATIO;
        npc.view_right_side = [right_x, right_y];
    }
}

/// Sets the transition flag when the status actually changes.
pub fn set_view_status(npc: &mut NpcData, status: EyeStatus) {
    npc.view_transition = npc.eye_status != status;
    npc.eye_status = status;
}

pub fn focus_entity(npc: &mut NpcData, target: EntityId) {
    npc.follow_target = Some(target);
    npc.eye_status = EyeStatus::Follow;
    npc.view_half_angle_range = STARE_HALF_ANGLE_RANGE;
}

pub fn focus_point(npc: &mut NpcData, point: GroundPoint) {
    npc.stare_point = point;
    npc.eye_status = EyeStatus::Stare;
    npc.view_half_angle_range = STARE_HALF_ANGLE_RANGE;
}

pub fn unfocus(npc: &mut NpcData) {
    npc.eye_status = EyeStatus::LookForward;
    npc.view_half_angle_range = NORMAL_HALF_ANGLE_RANGE;
    npc.follow_target = None;
}

/// Common handler for `LookForward`, `LookToTheLeft`, `LookToTheRight`,
/// and `ViewconeGrow` (after the grow step).
fn refresh_view_look(npc: &mut NpcData, ctx: &RefreshViewContext, vdx: f32, vdy: f32) {
    // Per-status angle goal + animation validation.
    let angle_goal = match npc.eye_status {
        EyeStatus::LookForward | EyeStatus::ViewconeGrow => 0.0f32,

        EyeStatus::LookToTheLeft => {
            match ctx.animation {
                Some(OrderType::LookingLeft | OrderType::LookingLeftAlerted) => {}
                _ => set_view_status(npc, EyeStatus::LookForward),
            }
            -std::f32::consts::FRAC_PI_4
        }

        EyeStatus::LookToTheRight => {
            match ctx.animation {
                Some(
                    OrderType::LookingRight
                    | OrderType::WaitingUprightBoredRandom
                    | OrderType::LookingRightAlerted,
                ) => {}
                _ => set_view_status(npc, EyeStatus::LookForward),
            }
            std::f32::consts::FRAC_PI_4
        }

        _ => 0.0,
    };

    // Common: reset aperture + range.
    npc.half_aperture = NORMAL_HALF_APERTURE;
    npc.view_half_angle_range = NORMAL_HALF_ANGLE_RANGE;

    // Transition toward goal angle.
    if npc.view_transition {
        if npc.view_angle > angle_goal {
            if npc.view_angle <= angle_goal + npc.view_angle_step {
                npc.view_angle = angle_goal;
                npc.view_angle_iterator = 0.0;
                npc.view_transition = false;
            } else {
                npc.view_angle -= npc.view_angle_step;
            }
        } else if npc.view_angle >= angle_goal - npc.view_angle_step {
            npc.view_angle = angle_goal;
            npc.view_angle_iterator = 0.0;
            npc.view_transition = false;
        } else {
            npc.view_angle += npc.view_angle_step;
        }
    }

    // direction = body_direction rotated by view_angle.
    let (rx, ry) = rotate_unit(vdx, vdy, npc.view_angle);
    npc.view_direction = [rx, ry];
}

/// Handler for `Stare` and `Follow` (after stare-point update).
fn refresh_view_stare(npc: &mut NpcData, vdx: f32, vdy: f32, own_position: &GroundPoint) {
    // Stare vector from own position to stare point.
    let mut svx = npc.stare_point.x - own_position.x;
    let mut svy = npc.stare_point.y - own_position.y;

    // Zero-length guard: fall back to body direction.
    if svx.abs().max(svy.abs()) == 0.0 {
        svx = vdx;
        svy = vdy;
    }

    // Stretch stare vector X by ASPECT_RATIO (0.5736).  An earlier
    // revision used INVERSE_ASPECT_RATIO (1.7434) based on a wrong
    // equivalence claim — that produced a view direction
    // perpendicular to the view vector's stretched frame in
    // `compute_visibility`, which spuriously rejected the target and
    // fired `EventOutOfView` a few frames into the reactiontime
    // window.
    svx *= crate::position_interface::ASPECT_RATIO;

    // Is the stare vector to the right of the view direction?
    let dir = npc.view_direction;
    let det_right = dir[0] * svy - dir[1] * svx;
    let vector_right = det_right > 0.0;

    // Is the stare target in the forward half-plane?
    let dot_forward = vdx * svx + vdy * svy;

    if dot_forward >= 0.0 {
        // Target in front: rotate toward it.
        let half_range = npc.view_half_angle_range;
        let step = npc.view_angle_step;

        if vector_right {
            if npc.view_angle < half_range {
                npc.view_angle += step;
                let (rx, ry) = rotate_unit(vdx, vdy, npc.view_angle);
                npc.view_direction = [rx, ry];

                // Overshoot check: if we passed the stare vector,
                // snap to it.
                let new_det = npc.view_direction[0] * svy - npc.view_direction[1] * svx;
                if new_det < 0.0 {
                    let new_dot = npc.view_direction[0] * svx + npc.view_direction[1] * svy;
                    if new_dot > 0.0 {
                        npc.view_direction = [svx, svy];
                        npc.view_angle = -vec_angle(svx, svy, vdx, vdy);
                    }
                }
            }
        } else if npc.view_angle > -half_range {
            npc.view_angle -= step;
            let (rx, ry) = rotate_unit(vdx, vdy, npc.view_angle);
            npc.view_direction = [rx, ry];

            // Overshoot check (mirrored).
            let new_det = npc.view_direction[0] * svy - npc.view_direction[1] * svx;
            if new_det > 0.0 {
                let new_dot = npc.view_direction[0] * svx + npc.view_direction[1] * svy;
                if new_dot > 0.0 {
                    npc.view_direction = [svx, svy];
                    npc.view_angle = -vec_angle(svx, svy, vdx, vdy);
                }
            }
        }
    } else {
        // Target behind: return to forward.
        if npc.view_angle == 0.0 {
            npc.view_direction = [vdx, vdy];
        } else if npc.view_angle > 0.0 {
            if npc.view_angle <= npc.view_angle_step {
                npc.view_angle = 0.0;
                npc.view_angle_iterator = 0.0;
                npc.view_direction = [vdx, vdy];
                npc.view_transition = false;
            } else {
                npc.view_angle -= npc.view_angle_step;
                let (rx, ry) = rotate_unit(vdx, vdy, npc.view_angle);
                npc.view_direction = [rx, ry];
            }
        } else if npc.view_angle >= npc.view_angle_step {
            // Bug-compatible: the comparison is `>= angle_step`
            // (positive) but `view_angle` is negative here, so this
            // is always false and the `else` branch runs.  The NPC
            // overshoots past 0 by one step, then gets caught by the
            // `view_angle > 0` branch on the next frame.
            npc.view_angle = 0.0;
            npc.view_angle_iterator = 0.0;
            npc.view_direction = [vdx, vdy];
            npc.view_transition = false;
        } else {
            npc.view_angle += npc.view_angle_step;
            let (rx, ry) = rotate_unit(vdx, vdy, npc.view_angle);
            npc.view_direction = [rx, ry];
        }
    }
}

/// Signed angle between two 2D vectors.
fn vec_angle(ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let dot = ax * bx + ay * by;
    let det = ax * by - ay * bx;
    if det == 0.0 {
        return if dot > 0.0 { 0.0 } else { std::f32::consts::PI };
    }
    let angle = (det / dot).atan();
    if dot >= 0.0 {
        angle
    } else if det > 0.0 {
        angle + std::f32::consts::PI
    } else {
        angle - std::f32::consts::PI
    }
}

// ─── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_radius_cache_is_one_last_writer_per_surface_and_frame() {
        let first = EntityId::from(crate::entity_id::SoldierId(7));
        let second = EntityId::from(crate::entity_id::SoldierId(9));
        let obstacle = ObstacleHandle::new(3).unwrap();
        let mut cache = ViewRadiusCache::default();

        cache.set(Some(obstacle), first, 40, 125.0);
        assert_eq!(cache.get(Some(obstacle), first, 40), Some(125.0));
        assert_eq!(cache.get(Some(obstacle), first, 41), None);

        cache.set(Some(obstacle), second, 40, 90.0);
        assert_eq!(cache.get(Some(obstacle), first, 40), None);
        assert_eq!(cache.get(Some(obstacle), second, 40), Some(90.0));
    }

    #[test]
    fn view_radius_cache_zero_is_a_miss_but_remains_last_writer_state() {
        let viewer = EntityId::from(crate::entity_id::CivilianId(12));
        let mut cache = ViewRadiusCache::default();

        cache.set(None, viewer, 55, 0.0);

        assert_eq!(cache.get(None, viewer, 55), None);
        assert_eq!(cache.ground.unwrap().viewer, viewer);
    }

    const NO_OBSTACLES: &[SightObstacle] = &[];

    fn pt(x: f32, y: f32) -> MapPoint {
        MapPoint::new(x, y)
    }

    fn empty_grid() -> &'static [SightObstacle] {
        NO_OBSTACLES
    }

    fn fast_grid_for_obstacles(
        obstacles: &[SightObstacle],
    ) -> &'static crate::fast_find_grid::FastFindGrid {
        let mut grid = crate::fast_find_grid::FastFindGrid::new();
        grid.size_map(64, 64);
        grid.allocate_layers(4);
        for (idx, obs) in obstacles.iter().enumerate() {
            let idx = crate::sight_obstacle::SightObstacleIndex::new(idx as u32).unwrap();
            grid.add_obstacle_index(idx, obs.layer, &obs.box_ground);
        }
        Box::leak(Box::new(grid))
    }

    fn query<'a>(
        viewer: MapPoint,
        dir: i16,
        target: MapPoint,
        obstacles: &'a [SightObstacle],
    ) -> VisibilityQuery<'a> {
        VisibilityQuery {
            viewer_los: viewer,
            viewer_world: WorldPoint3D::new(viewer.x, viewer.y, 30.0),
            viewer_direction: dir,
            view_forward: sector_to_forward(dir),
            view_radius: DEFAULT_VIEW_RADIUS,
            viewer_eye_status: EyeStatus::LookForward,
            real_half_aperture: NORMAL_HALF_APERTURE,
            viewer_in_building: false,
            target_in_same_building: false,
            forest_180_degree_view: false,
            golden_eye_mode: false,
            effective_view_radius: DEFAULT_VIEW_RADIUS as f32,
            target_is_active_and_outside_building: true,
            target_los: target,
            target_world: WorldPoint3D::new(target.x, target.y, 30.0),
            target_posture: Posture::Upright,
            target_action_state: ActionState::Waiting,
            target_is_pc: true,
            sight_obstacles: crate::sight_obstacle::ObstacleList::from_slice_all_active(obstacles),
            fast_grid: fast_grid_for_obstacles(obstacles),
            layer: 0,
            target_unconscious: false,
            target_passing_door: false,
        }
    }

    #[test]
    fn sector_forward_cardinals() {
        let (nx, ny) = sector_to_forward(0);
        assert!(nx.abs() < 1e-5 && ny < -0.99);
        let (ex, ey) = sector_to_forward(4);
        assert!(ex > 0.99 && ey.abs() < 1e-5);
        let (sx, sy) = sector_to_forward(8);
        assert!(sx.abs() < 1e-5 && sy > 0.99);
        let (wx, wy) = sector_to_forward(12);
        assert!(wx < -0.99 && wy.abs() < 1e-5);
    }

    #[test]
    fn auto_visible_within_20_units() {
        // Inside the 3D <400 check — always 1.0.
        let grid = empty_grid();
        let q = query(pt(0.0, 0.0), 4, pt(10.0, 0.0), grid);
        assert_eq!(compute_visibility(&q), 1.0);
    }

    #[test]
    fn effective_radius_is_requested_only_after_original_early_exits() {
        let calls = std::cell::Cell::new(0_u32);
        let radius = || {
            calls.set(calls.get() + 1);
            DEFAULT_VIEW_RADIUS as f32
        };

        let mut disguised = query(pt(0.0, 0.0), 4, pt(100.0, 0.0), empty_grid());
        disguised.target_posture = Posture::Spy;
        assert_eq!(
            compute_visibility_with_effective_radius(&disguised, radius),
            0.0
        );
        assert_eq!(calls.get(), 0);

        let close = query(pt(0.0, 0.0), 4, pt(10.0, 0.0), empty_grid());
        assert_eq!(
            compute_visibility_with_effective_radius(&close, radius),
            1.0
        );
        assert_eq!(calls.get(), 0);

        let eligible = query(pt(0.0, 0.0), 4, pt(100.0, 0.0), empty_grid());
        assert!(compute_visibility_with_effective_radius(&eligible, radius) > 0.0);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn cross_elevation_visibility_uses_world_horizontal_distance() {
        // Original `GetPosition()` points share world Y here, while their
        // projected map Y differs by the 300-unit ground elevation. Feeding
        // projected Y into the radius test would manufacture a ~523-unit
        // horizontal separation and reject this target outside radius 400.
        let grid = empty_grid();
        let mut q = query(pt(0.0, 0.0), 4, pt(0.0, -300.0), grid);
        q.viewer_world = WorldPoint3D::new(0.0, 0.0, 45.0);
        q.target_world = WorldPoint3D::new(0.0, 0.0, 345.0);

        assert!(compute_visibility(&q) > 0.0);
    }

    #[test]
    fn vertical_separation_contributes_to_human_distance_sharpness() {
        let grid = empty_grid();
        let mut q = query(pt(0.0, 0.0), 4, pt(0.0, 0.0), grid);
        q.view_radius = 200;
        q.effective_view_radius = 200.0;
        q.viewer_world = WorldPoint3D::new(0.0, 0.0, 45.0);
        q.target_world = WorldPoint3D::new(0.0, 0.0, 145.0);

        assert_eq!(
            compute_visibility(&q),
            distance_sharpness(100.0 * 100.0, 200.0)
        );
    }

    #[test]
    fn spy_disguise_invisible() {
        let grid = empty_grid();
        let mut q = query(pt(0.0, 0.0), 4, pt(100.0, 0.0), grid);
        q.target_posture = Posture::Spy;
        assert_eq!(compute_visibility(&q), 0.0);
    }

    #[test]
    fn tree_and_anonymous_archer_invisible() {
        let grid = empty_grid();
        for p in [Posture::Tree, Posture::AnonymousArcher] {
            let mut q = query(pt(0.0, 0.0), 4, pt(100.0, 0.0), grid);
            q.target_posture = p;
            assert_eq!(compute_visibility(&q), 0.0);
        }
    }

    #[test]
    fn golden_eye_mode_blind_to_pc() {
        let grid = empty_grid();
        let mut q = query(pt(0.0, 0.0), 4, pt(100.0, 0.0), grid);
        q.golden_eye_mode = true;
        assert_eq!(compute_visibility(&q), 0.0);
        // NPC target still visible under golden eye.
        q.target_is_pc = false;
        assert!(compute_visibility(&q) > 0.0);
    }

    #[test]
    fn same_building_returns_half() {
        // Viewer in building + target in same building → 0.5.
        let mut q = query(pt(0.0, 0.0), 4, pt(100.0, 0.0), &[]);
        q.viewer_in_building = true;
        q.target_in_same_building = true;
        assert_eq!(compute_visibility(&q), 0.5);
    }

    #[test]
    fn different_building_returns_zero() {
        // Viewer in building, target NOT in same building → 0.
        let mut q = query(pt(0.0, 0.0), 4, pt(100.0, 0.0), &[]);
        q.viewer_in_building = true;
        q.target_in_same_building = false;
        assert_eq!(compute_visibility(&q), 0.0);
    }

    #[test]
    fn forest_180_degree_view_sees_to_the_side() {
        // 180° forward half-plane: target perpendicular to nose at
        // mid-range should be visible (the narrow cone would reject
        // it).
        let mut q = query(pt(0.0, 0.0), 4, pt(0.0, -200.0), &[]);
        q.forest_180_degree_view = true;
        assert_eq!(compute_visibility(&q), 1.0);
        // But still blind to anything behind.
        let mut q = query(pt(0.0, 0.0), 4, pt(-100.0, 0.0), &[]);
        q.forest_180_degree_view = true;
        assert_eq!(compute_visibility(&q), 0.0);
    }

    #[test]
    fn forest_180_degree_view_uses_body_not_mutable_view_direction() {
        let mut q = query(pt(0.0, 0.0), 4, pt(100.0, 0.0), &[]);
        q.forest_180_degree_view = true;
        // The body faces east while RefreshView's independently mutable cone
        // faces west. Original IsDetecting180Degrees uses GetDirectionVector,
        // so the eastward target remains in front.
        q.view_forward = sector_to_forward(12);
        assert_eq!(compute_visibility(&q), 1.0);
    }

    #[test]
    fn forest_180_degree_view_computes_effective_radius_at_original_boundary() {
        use std::cell::Cell;

        let calls = Cell::new(0);
        let mut behind = query(pt(0.0, 0.0), 4, pt(-100.0, 0.0), &[]);
        behind.forest_180_degree_view = true;
        assert_eq!(
            compute_visibility_with_effective_radius(&behind, || {
                calls.set(calls.get() + 1);
                50.0
            }),
            0.0
        );
        assert_eq!(calls.get(), 0, "behind targets skip ComputeViewRadius");

        let mut ahead = query(pt(0.0, 0.0), 4, pt(100.0, 0.0), &[]);
        ahead.forest_180_degree_view = true;
        assert_eq!(
            compute_visibility_with_effective_radius(&ahead, || {
                calls.set(calls.get() + 1);
                // Models a night/fog or obstacle-projection radius shrink.
                50.0
            }),
            0.0
        );
        assert_eq!(calls.get(), 1, "forward targets compute the radius once");
    }

    #[test]
    fn diagonal_night_fog_side_samples_compress_only_their_y_component() {
        let eye = WorldPoint3D::new(17.0, 29.0, 30.0);
        let radius = 400.0;
        let forward = sector_to_forward(2);
        let aperture = 0.5;
        let [center, left, right] =
            night_fog_light_reference_points(eye, radius, forward, aperture);
        let half_r = radius * 0.5;
        let (left_x, left_y) = rotate_unit(forward.0, forward.1, -aperture);
        let (right_x, right_y) = rotate_unit(forward.0, forward.1, aperture);

        assert!((center.x - (eye.x + half_r * forward.0)).abs() < 1e-5);
        assert!((center.y - (eye.y + half_r * forward.1)).abs() < 1e-5);
        assert!((left.x - (eye.x + half_r * left_x)).abs() < 1e-5);
        assert!((left.y - (eye.y + half_r * left_y * ASPECT_RATIO)).abs() < 1e-5);
        assert!((right.x - (eye.x + half_r * right_x)).abs() < 1e-5);
        assert!((right.y - (eye.y + half_r * right_y * ASPECT_RATIO)).abs() < 1e-5);

        assert!(
            (left.y - (eye.y + half_r * left_y)).abs() > 1.0,
            "diagonal side sample must not retain uncompressed world Y"
        );
        assert!(
            (right.y - (eye.y + half_r * right_y)).abs() > 1.0,
            "diagonal side sample must not retain uncompressed world Y"
        );
    }

    #[test]
    fn closed_eyes_blind() {
        let mut q = query(pt(0.0, 0.0), 4, pt(100.0, 0.0), &[]);
        q.viewer_eye_status = EyeStatus::Closed;
        assert_eq!(compute_visibility(&q), 0.0);
        q.viewer_eye_status = EyeStatus::DieOrGetUnconscious;
        assert_eq!(compute_visibility(&q), 0.0);
        q.viewer_eye_status = EyeStatus::LookForward;
        assert!(compute_visibility(&q) > 0.0);
    }

    #[test]
    fn viewer_in_building_blind() {
        let grid = empty_grid();
        let mut q = query(pt(0.0, 0.0), 4, pt(100.0, 0.0), grid);
        q.viewer_in_building = true;
        assert_eq!(compute_visibility(&q), 0.0);
    }

    #[test]
    fn sees_target_inside_cone() {
        // 100 units east, facing east.  Should give something on
        // the first leg of the distance curve.
        let grid = empty_grid();
        let q = query(pt(0.0, 0.0), 4, pt(100.0, 0.0), grid);
        let v = compute_visibility(&q);
        assert!(v > 0.0 && v <= 1.0, "got {}", v);
    }

    #[test]
    fn cone_sides_apply_original_isometric_aspect_ratio() {
        // This is the frame-38 Soldier83 -> Soldier82 geometry from the
        // original parity trace. In raw screen X/Y it appears to be about
        // 21 degrees left of west and inside the 0.5-radian cone. Original
        // scales each cone side's Y by ASPECT_RATIO, which puts it outside.
        let grid = empty_grid();
        let mut q = query(pt(655.01, 1576.0), 12, pt(560.0, 1539.0), grid);
        q.view_forward = (-1.0, 0.0);
        q.real_half_aperture = 0.5;

        assert_eq!(compute_visibility(&q), 0.0);
    }

    #[test]
    fn blind_behind() {
        let grid = empty_grid();
        let q = query(pt(0.0, 0.0), 4, pt(-200.0, 0.0), grid);
        assert_eq!(compute_visibility(&q), 0.0);
    }

    #[test]
    fn out_of_range_east_west() {
        let grid = empty_grid();
        let q = query(pt(0.0, 0.0), 4, pt(1000.0, 0.0), grid);
        assert_eq!(compute_visibility(&q), 0.0);
    }

    #[test]
    fn isometric_stretch_shortens_north_south_range() {
        // N-S distance is stretched by ~1.74, so a 300-unit north
        // target (facing north) sees an effective distance of 522,
        // which exceeds the 400 view radius → invisible.
        let grid = empty_grid();
        let q = query(pt(0.0, 0.0), 0, pt(0.0, -300.0), grid);
        assert_eq!(compute_visibility(&q), 0.0);
        // 150 units north: effective 261 < 400 → visible.
        let q = query(pt(0.0, 0.0), 0, pt(0.0, -150.0), grid);
        assert!(compute_visibility(&q) > 0.0);
    }

    #[test]
    fn running_target_is_much_more_visible() {
        let grid = empty_grid();
        let mut q = query(pt(0.0, 0.0), 4, pt(200.0, 0.0), grid);
        let idle = compute_visibility(&q);
        q.target_action_state = ActionState::MovingFast;
        let running = compute_visibility(&q);
        assert!(
            (running - RUNNING_DETECTION_FACTOR * idle).abs() < 1e-5,
            "running {running} vs idle {idle}"
        );
    }

    #[test]
    fn crouched_target_is_half_as_visible() {
        let grid = empty_grid();
        let mut q = query(pt(0.0, 0.0), 4, pt(200.0, 0.0), grid);
        let upright = compute_visibility(&q);
        q.target_posture = Posture::Crouched;
        let crouched = compute_visibility(&q);
        assert!(
            (crouched - CROUCHED_DETECTION_FACTOR * upright).abs() < 1e-5,
            "crouched {crouched} vs upright {upright}"
        );
    }

    #[test]
    fn distance_curve_monotonic_and_positive() {
        // Same direction, increasing distance → sharpness should
        // monotonically decrease from 1.0 toward ~DETECTION_CURVE_Y_3.
        let grid = empty_grid();
        let mut last = f32::INFINITY;
        for d in [21.0f32, 60.0, 100.0, 150.0, 200.0, 300.0, 380.0] {
            let q = query(pt(0.0, 0.0), 4, pt(d, 0.0), grid);
            let v = compute_visibility(&q);
            assert!(v > 0.0, "d={d} → 0");
            assert!(v <= last, "d={d} v={v} last={last}");
            last = v;
        }
    }

    #[test]
    fn close_halfcircle_sees_to_the_side() {
        // Target 40 units north-ish, facing east. 40 world-units Y
        // stretches to ≈ 70 effective; inside the halfcircle radius
        // of 60? barely not — let's pick a closer one.
        // At 30 units north: stretched to 52.3 < 60 → halfcircle
        // branch. diff = sector(0,52.3) - 4 → should be in halfcircle
        // set {12..4}.
        let grid = empty_grid();
        let q = query(pt(0.0, 0.0), 4, pt(0.0, -30.0), grid);
        assert!(compute_visibility(&q) > 0.0);
    }

    #[test]
    fn tall_opaque_obstacle_blocks_los() {
        use crate::sight_obstacle::ObstaclePoint;
        // Build a wall blocking the line from (0,0) to (200,0): a
        // box around (100, -10)..(110, 10).  Vertices are CCW.
        let mut wall = SightObstacle::new_default(0);
        wall.obstacle_points = vec![
            ObstaclePoint {
                x: 100.0,
                y: -10.0,
                z_top: 80.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 110.0,
                y: -10.0,
                z_top: 80.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 110.0,
                y: 10.0,
                z_top: 80.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 100.0,
                y: 10.0,
                z_top: 80.0,
                z_bottom: 0.0,
            },
        ];
        wall.top_plane_points = [
            [100.0, -10.0, 80.0],
            [110.0, -10.0, 80.0],
            [100.0, 10.0, 80.0],
        ];
        wall.bottom_plane_points = [[100.0, -10.0, 0.0], [110.0, -10.0, 0.0], [100.0, 10.0, 0.0]];
        wall.rebuild_geometry();
        let obstacles = vec![wall];

        let q = query(pt(0.0, 0.0), 4, pt(200.0, 0.0), &obstacles);
        assert_eq!(
            compute_visibility(&q),
            0.0,
            "wall at x=100..110 should block sight to target at x=200"
        );
    }

    #[test]
    fn obstacle_off_to_the_side_does_not_block() {
        use crate::sight_obstacle::ObstaclePoint;
        // Wall at (50, 100)..(60, 110) — far from the line of sight.
        let mut wall = SightObstacle::new_default(0);
        wall.obstacle_points = vec![
            ObstaclePoint {
                x: 50.0,
                y: 100.0,
                z_top: 5.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 60.0,
                y: 100.0,
                z_top: 5.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 60.0,
                y: 110.0,
                z_top: 5.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 50.0,
                y: 110.0,
                z_top: 5.0,
                z_bottom: 0.0,
            },
        ];
        wall.rebuild_geometry();
        let obstacles = vec![wall];

        let q = query(pt(0.0, 0.0), 4, pt(200.0, 0.0), &obstacles);
        assert!(compute_visibility(&q) > 0.0);
    }

    #[test]
    fn non_opaque_obstacle_does_not_block() {
        use crate::sight_obstacle::{ObstaclePoint, SIGHTOBSTACLE_SOLID};
        // SOLID-only wall (e.g. invisible collider) should NOT block
        // sight, only movement.
        let mut wall = SightObstacle::new(0, SIGHTOBSTACLE_SOLID);
        wall.obstacle_points = vec![
            ObstaclePoint {
                x: 100.0,
                y: -10.0,
                z_top: 5.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 110.0,
                y: -10.0,
                z_top: 5.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 110.0,
                y: 10.0,
                z_top: 5.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 100.0,
                y: 10.0,
                z_top: 5.0,
                z_bottom: 0.0,
            },
        ];
        wall.rebuild_geometry();
        let obstacles = vec![wall];

        let q = query(pt(0.0, 0.0), 4, pt(200.0, 0.0), &obstacles);
        assert!(compute_visibility(&q) > 0.0);
    }

    #[test]
    fn per_npc_radius_respected() {
        // Short-sighted NPC: radius 100, target at 150 → blind.
        let grid = empty_grid();
        let mut q = query(pt(0.0, 0.0), 4, pt(150.0, 0.0), grid);
        q.view_radius = 100;
        assert_eq!(compute_visibility(&q), 0.0);
    }

    // ─── refresh_view behavioural tests ──────────────────────────

    fn default_npc() -> NpcData {
        NpcData {
            eye_status: EyeStatus::LookForward,
            view_direction: [1.0, 0.0], // facing east (sector 4)
            direction_old: 4,
            ..Default::default()
        }
    }

    fn ctx(animation: Option<OrderType>, posture: Posture) -> RefreshViewContext {
        RefreshViewContext {
            body_direction: 4,
            posture,
            animation,
            is_unconscious: false,
            is_tied: false,
            is_dead: false,
            is_active_and_outside_building: true,
            is_rider: false,
            blood_alcohol: 0,
            own_position: GroundPoint::new(0.0, 0.0),
            follow_target_position: None,
        }
    }

    #[test]
    fn bored_looking_left_sets_eye_status_and_rotates_cone() {
        // The look-status is set by
        // `engine::animation::apply_soldier_execute_side_effects` on
        // the LookingLeft motion-start edge.  For this per-frame
        // `refresh_view` behavioural test we simulate the START edge
        // by calling `set_view_status` directly, then drive the view
        // angle toward its goal.
        let mut npc = default_npc();
        set_view_status(&mut npc, EyeStatus::LookToTheLeft);
        let c = ctx(Some(OrderType::LookingLeft), Posture::Upright);

        // Drive the transition over several frames so the view angle
        // walks toward its -π/4 goal.
        for _ in 0..20 {
            refresh_view(&mut npc, &c);
        }
        assert_eq!(npc.eye_status, EyeStatus::LookToTheLeft);
        // View angle should have settled at -π/4 (head turned left).
        assert!(
            (npc.view_angle + std::f32::consts::FRAC_PI_4).abs() < 1e-4,
            "view_angle {} ≠ -π/4",
            npc.view_angle
        );
    }

    #[test]
    fn bored_looking_right_sets_eye_status() {
        // The look-status is set on the LookingRight motion-start
        // edge by `apply_soldier_execute_side_effects`.  We simulate
        // the START edge here, then let `refresh_view` rotate the
        // cone.
        let mut npc = default_npc();
        set_view_status(&mut npc, EyeStatus::LookToTheRight);
        let c = ctx(Some(OrderType::LookingRight), Posture::Upright);
        for _ in 0..20 {
            refresh_view(&mut npc, &c);
        }
        assert_eq!(npc.eye_status, EyeStatus::LookToTheRight);
        assert!(
            (npc.view_angle - std::f32::consts::FRAC_PI_4).abs() < 1e-4,
            "view_angle {} ≠ π/4",
            npc.view_angle
        );
    }

    #[test]
    fn looking_left_done_restores_look_forward() {
        // On the LookingLeft motion-done edge,
        // `apply_soldier_execute_side_effects` resets eye status to
        // LookForward.  `refresh_view` must then walk the view_angle
        // back to 0 — not leave it stuck at -π/4.
        let mut npc = default_npc();
        set_view_status(&mut npc, EyeStatus::LookToTheLeft);
        let c_hold = ctx(Some(OrderType::LookingLeft), Posture::Upright);
        for _ in 0..20 {
            refresh_view(&mut npc, &c_hold);
        }
        assert_eq!(npc.eye_status, EyeStatus::LookToTheLeft);
        assert!((npc.view_angle + std::f32::consts::FRAC_PI_4).abs() < 1e-4);

        // Simulate the MS::Done event edge.
        set_view_status(&mut npc, EyeStatus::LookForward);
        // Animation ticks keep flowing until the sprite advances;
        // `ctx.animation` can still briefly be LookingLeft on the
        // same frame as the DONE event, and `refresh_view` must not
        // reassert LookToTheLeft.
        refresh_view(&mut npc, &c_hold);
        assert_eq!(npc.eye_status, EyeStatus::LookForward);
        // Subsequent frames drive the cone back to center.
        let c_after = ctx(Some(OrderType::WaitingUpright), Posture::Upright);
        for _ in 0..20 {
            refresh_view(&mut npc, &c_after);
        }
        assert!(
            npc.view_angle.abs() < 1e-4,
            "view_angle {} did not return to 0 after DONE",
            npc.view_angle
        );
    }

    #[test]
    fn officer_bored_random_event_sets_right_look() {
        // Only officers trigger the right-look during
        // WaitingUprightBoredRandom.  This is plumbed via
        // `apply_npc_execute_side_effects` on the motion-start edge,
        // which inspects `enemy_ai.soldier_profile_rank == Officer`.
        // `refresh_view` itself no longer knows about rank — so here
        // we just confirm that once the look-right status has been
        // set (by the animation dispatcher), `refresh_view`
        // preserves it while the animation keeps playing.
        let mut officer = default_npc();
        set_view_status(&mut officer, EyeStatus::LookToTheRight);
        let c = ctx(Some(OrderType::WaitingUprightBoredRandom), Posture::Upright);
        refresh_view(&mut officer, &c);
        assert_eq!(officer.eye_status, EyeStatus::LookToTheRight);

        // A grunt that never received the START event stays on
        // LookForward across the same animation.
        let mut grunt = default_npc();
        refresh_view(&mut grunt, &c);
        assert_eq!(grunt.eye_status, EyeStatus::LookForward);
    }

    #[test]
    fn look_status_resets_when_animation_leaves() {
        // `refresh_view` must reset LookToTheLeft back to LookForward
        // when the sprite has moved to any other animation.
        let mut npc = default_npc();
        npc.eye_status = EyeStatus::LookToTheLeft;
        npc.view_angle = -std::f32::consts::FRAC_PI_4;
        // Next frame the NPC is playing a waiting animation, not
        // LookingLeft.  `refresh_view` must clear the look state.
        let c = ctx(Some(OrderType::WaitingUpright), Posture::Upright);
        refresh_view(&mut npc, &c);
        assert_eq!(npc.eye_status, EyeStatus::LookForward);
    }

    #[test]
    fn drunken_wobble_shrinks_radius_and_aperture() {
        // The drunken cone wobble multiplies the real radius and
        // aperture by a blood-alcohol-driven factor on every frame.
        let mut sober = default_npc();
        sober.view_radius_base = 400;
        let mut drunk = sober.clone();
        refresh_view(&mut sober, &ctx(None, Posture::Upright));

        let mut c = ctx(None, Posture::Upright);
        c.blood_alcohol = 80;
        refresh_view(&mut drunk, &c);

        // Drunken NPC sees a shorter distance through a narrower
        // aperture, and the phase iterators have advanced.
        assert!(drunk.view_radius < sober.view_radius);
        assert!(drunk.real_half_aperture < sober.real_half_aperture);
        assert!(drunk.drunken_cone_iterators.iter().any(|&v| v > 0.0));
    }

    #[test]
    fn lean_out_widens_cone_and_faces_forward() {
        // LeaningOut posture forces LookDownwards with a π/2-wide
        // aperture and no view-angle offset.
        let mut npc = default_npc();
        refresh_view(&mut npc, &ctx(None, Posture::LeaningOut));
        assert_eq!(npc.eye_status, EyeStatus::LookDownwards);
        assert!(npc.view_lean_out);
        // Half-aperture close to π/2 (minus the 0.05 safety margin).
        assert!((npc.real_half_aperture - (std::f32::consts::FRAC_PI_2 - 0.05)).abs() < 1e-4);
        // View direction equals body direction (no angle offset).
        assert_eq!(npc.view_angle, 0.0);

        // Dropping the posture restores LookForward and NORMAL_HALF_APERTURE.
        refresh_view(&mut npc, &ctx(None, Posture::Upright));
        assert_eq!(npc.eye_status, EyeStatus::LookForward);
        assert!(!npc.view_lean_out);
        assert!((npc.real_half_aperture - NORMAL_HALF_APERTURE).abs() < 1e-4);
    }

    #[test]
    fn death_fades_alpha_and_eventually_closes_eyes() {
        // DieOrGetUnconscious decrements alpha by 5 each frame and
        // shrinks the radius goal with an accelerating step, then
        // sets eye status to Closed once the goal falls below zero.
        let mut npc = default_npc();
        npc.eye_status = EyeStatus::DieOrGetUnconscious;
        npc.view_alpha_start = ALPHA_START;
        npc.view_radius_goal = 400;
        let initial_alpha = npc.view_alpha_start;

        // One tick: alpha drops by 5, step accelerates from 0 → 5.
        refresh_view(&mut npc, &ctx(None, Posture::Upright));
        assert_eq!(npc.view_alpha_start, initial_alpha - 5);
        assert_eq!(npc.view_radius_step, 5);

        // Run many frames — quadratic shrink must eventually close.
        for _ in 0..200 {
            if npc.eye_status == EyeStatus::Closed {
                break;
            }
            refresh_view(&mut npc, &ctx(None, Posture::Upright));
        }
        assert_eq!(npc.eye_status, EyeStatus::Closed);
        assert_eq!(npc.view_radius_goal, 0);
    }

    #[test]
    fn follow_target_gaze_rotates_toward_moving_point() {
        // Follow reads the target position each frame, computes the
        // stare vector, and rotates the view cone toward it by
        // `view_angle_step`.
        let mut npc = default_npc();
        focus_entity(&mut npc, EntityId::Pc(crate::entity_id::PcId(42)));
        assert_eq!(npc.eye_status, EyeStatus::Follow);

        // Target off to the NPC's right (body facing east, target south-east).
        let mut c = ctx(None, Posture::Upright);
        c.follow_target_position = Some(GroundPoint::new(200.0, 200.0));

        for _ in 0..30 {
            refresh_view(&mut npc, &c);
        }
        // View angle should have rotated to a positive value (toward target).
        assert!(
            npc.view_angle > 0.0,
            "view_angle {} did not rotate toward right-side target",
            npc.view_angle
        );
        // Real half-aperture is narrowed by STARE_APERTURE_FACTOR.
        assert!(
            (npc.real_half_aperture - NORMAL_HALF_APERTURE * STARE_APERTURE_FACTOR).abs() < 1e-4
        );
    }

    #[test]
    fn focus_point_enters_stare_state() {
        // `focus_point` sets eye status to Stare and the view cone
        // rotates toward the stored stare point.
        let mut npc = default_npc();
        focus_point(&mut npc, GroundPoint::new(0.0, 200.0));
        assert_eq!(npc.eye_status, EyeStatus::Stare);
        assert_eq!(npc.view_half_angle_range, STARE_HALF_ANGLE_RANGE);
    }

    #[test]
    fn unfocus_returns_to_look_forward() {
        let mut npc = default_npc();
        focus_entity(&mut npc, EntityId::Pc(crate::entity_id::PcId(7)));
        unfocus(&mut npc);
        assert_eq!(npc.eye_status, EyeStatus::LookForward);
        assert_eq!(npc.view_half_angle_range, NORMAL_HALF_ANGLE_RANGE);
        assert!(npc.follow_target.is_none());
    }

    #[test]
    fn dead_npc_closes_eyes() {
        // Symptom therapy forces Closed on unconscious/tied/dead
        // NPCs (unless already in the DIE fade path).
        let mut npc = default_npc();
        let mut c = ctx(None, Posture::Upright);
        c.is_dead = true;
        refresh_view(&mut npc, &c);
        assert_eq!(npc.eye_status, EyeStatus::Closed);
    }

    // ── compute_object_visibility ─────────────────────────────────

    fn obj_query<'a>(
        viewer: MapPoint,
        dir: i16,
        target: MapPoint,
        obstacles: &'a [SightObstacle],
    ) -> ObjectVisibilityQuery<'a> {
        ObjectVisibilityQuery {
            viewer_los: viewer,
            viewer_world: WorldPoint3D::new(viewer.x, viewer.y, 0.0),
            viewer_direction: dir,
            view_forward: sector_to_forward(dir),
            view_radius: DEFAULT_VIEW_RADIUS,
            viewer_eye_status: EyeStatus::LookForward,
            real_half_aperture: NORMAL_HALF_APERTURE,
            viewer_in_building: false,
            object_belongs_to_beggar: false,
            target_los: target,
            target_world: WorldPoint3D::new(target.x, target.y, 0.0),
            sight_obstacles: crate::sight_obstacle::ObstacleList::from_slice_all_active(obstacles),
            fast_grid: fast_grid_for_obstacles(obstacles),
            layer: 0,
        }
    }

    #[test]
    fn object_visible_inside_cone() {
        // Object 100 east, viewer facing east: cleanly inside cone,
        // distance curve returns something on the first-leg ramp.
        let q = obj_query(pt(0.0, 0.0), 4, pt(100.0, 0.0), &[]);
        let v = compute_object_visibility(&q);
        assert!(v > 0.0 && v <= 1.0, "got {}", v);
    }

    #[test]
    fn cross_elevation_object_visibility_uses_world_3d_vector() {
        let grid = empty_grid();
        let mut q = obj_query(pt(0.0, 0.0), 4, pt(0.0, -300.0), grid);
        q.viewer_world = WorldPoint3D::new(0.0, 0.0, 45.0);
        q.target_world = WorldPoint3D::new(0.0, 0.0, 301.0);

        assert!(compute_object_visibility(&q) > 0.0);
    }

    #[test]
    fn object_belongs_to_beggar_invisible() {
        // Beggar-dropped coins short-circuit to 0.
        let mut q = obj_query(pt(0.0, 0.0), 4, pt(100.0, 0.0), &[]);
        q.object_belongs_to_beggar = true;
        assert_eq!(compute_object_visibility(&q), 0.0);
    }

    #[test]
    fn object_viewer_in_building_always_zero() {
        // Unlike the human path there is no "same building → 0.5"
        // branch; viewer indoors is always 0.
        let mut q = obj_query(pt(0.0, 0.0), 4, pt(100.0, 0.0), &[]);
        q.viewer_in_building = true;
        assert_eq!(compute_object_visibility(&q), 0.0);
    }

    #[test]
    fn object_closed_eyes_invisible() {
        let mut q = obj_query(pt(0.0, 0.0), 4, pt(100.0, 0.0), &[]);
        q.viewer_eye_status = EyeStatus::Closed;
        assert_eq!(compute_object_visibility(&q), 0.0);
        q.viewer_eye_status = EyeStatus::DieOrGetUnconscious;
        assert_eq!(compute_object_visibility(&q), 0.0);
    }

    #[test]
    fn object_behind_viewer_invisible_even_close_range() {
        // The object path rejects any `view_dot_dir < 0`
        // unconditionally, so even a close-range (< halfcircle)
        // backwards object is invisible — the human path's peripheral
        // near-vision does not apply to inanimate objects.
        let q = obj_query(pt(0.0, 0.0), 4, pt(-40.0, 0.0), &[]);
        assert_eq!(compute_object_visibility(&q), 0.0);
    }

    #[test]
    fn object_out_of_range_invisible() {
        let q = obj_query(pt(0.0, 0.0), 4, pt(1000.0, 0.0), &[]);
        assert_eq!(compute_object_visibility(&q), 0.0);
    }

    #[test]
    fn object_no_posture_multiplier() {
        // The object path returns the raw distance curve — unlike
        // the human path, no RUNNING / WALKING / BOW / SWORD
        // scaling.  Sanity check: at ~100 east the return equals the
        // curve value directly.
        let q = obj_query(pt(0.0, 0.0), 4, pt(100.0, 0.0), &[]);
        let got = compute_object_visibility(&q);
        let dx = 100.0_f32;
        let sy = 0.0_f32;
        let sqr = dx * dx + sy * sy;
        let expected = distance_sharpness(sqr, DEFAULT_VIEW_RADIUS as f32);
        assert!((got - expected).abs() < 1e-5, "got {} vs {}", got, expected);
    }
}
