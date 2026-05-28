//! Bow-shot execution — dispatch logic for `Command::ShootBow`.
//!
//! Implements the end-to-end flow for firing an arrow at a target:
//!
//! 1. [`begin_bow_shot`] is called by the engine when a
//!    `Command::ShootBow` sequence element is dispatched to a shooter.
//!    It sets the shooter into the appropriate aiming action state,
//!    pushes aim-transition and shoot orders onto the order queue,
//!    and marks the `ActiveShot` in-progress.
//!
//! 2. [`tick_bow_shots`] runs every engine tick and, for each actor with an
//!    [`ActiveShot`], drives the sprite through transition animations
//!    and the shoot animation.  On the frame the shoot animation reports
//!    [`SpriteMotionState::Done`], the tick returns a
//!    [`ShotTickResult`] for each completed shot so the engine layer
//!    can compute the trajectory and spawn the arrow.
//!
//! 3. The engine layer (`EngineInner::tick_bow_shots`) receives the result,
//!    looks up the shooter's bow profile, rolls the hit chance,
//!    computes a ballistic trajectory via [`compute_initial_throw_velocity`]
//!    and [`compute_trajectory_ballistic`], and spawns the arrow via
//!    [`spawn_arrow`].
//!
//! 4. [`tick_arrows`] runs every engine tick and advances each arrow
//!    along its precomputed ballistic trajectory (popping waypoints
//!    from the trajectory list, interpolating between them).  When the
//!    arrow comes within [`HIT_DISTANCE`] of any human, or the
//!    trajectory runs out, the hit is applied via [`apply_arrow_hit`].
//!
//! ## UI action-slot refresh
//!
//! When ammo reaches 0, the ammo decrement path
//! (`engine/combat.rs::decrement_bow_ammo`) calls
//! `EngineInner::disable_pc_action`, which sets
//! `PcData::disabled_actions[Bow] = true`.  The HUD action-slot strip is
//! immediate-mode (see `ui_panel.rs`) and re-reads `disabled_actions`
//! each frame, so no messenger notification is needed — the next frame
//! shows the disabled bow slot automatically.

use crate::combat::{self, ConcussionContext};
use crate::element::{
    ActionState, Animation, Command, ElementData, ElementKind, ElementProjectile, Entity, EntityId,
    ObjectData, ObjectType, Point2D as ElemPoint2D, Point3D, Posture, ProjectileData, TargetFilter,
    TrajectoryPoint,
};
use crate::movement::ActiveShot;
use crate::order::{Order, OrderType};
use crate::profiles::Action;
use crate::sequence::{SequenceElement, SequenceElementData, SequenceId, SequenceManager};
use crate::sprite::MotionState as SpriteMotionState;
use crate::weapons::ShootMode;

// ═══════════════════════════════════════════════════════════════════
//  Physics constants
// ═══════════════════════════════════════════════════════════════════

/// Gravitational acceleration (negative = downward).
pub const GRAVITY: f32 = -8.01;

/// Arrow mass for flat (normal / down) shots.
pub const MASS_ARROW_FLAT: f32 = 0.1;

/// Arrow mass for high (long) shots — heavier for a steeper arc.
pub const MASS_ARROW_HIGH: f32 = 0.9;

// Throwable projectile masses.
pub const MASS_APPLE: f32 = 0.8;
pub const MASS_PURSE: f32 = 0.2;
pub const MASS_WASP_NEST: f32 = 0.5;
pub const MASS_NET: f32 = 0.6;
pub const MASS_STONE: f32 = 0.1;

// Throwable apex heights.
pub const APEX_APPLE: f32 = 15.0;
pub const APEX_PURSE: f32 = 15.0;
pub const APEX_WASP_NEST: f32 = 50.0;
pub const APEX_NET: f32 = 30.0;
// Stone is thrown with flight_time = 1 (see `spawn_stone`), so
// `compute_initial_throw_velocity` takes the `v = 0.5 * direction` branch
// and the apex value is never consulted.  Keep at 0.001 to preserve
// replay determinism if the flight-time path ever changes.
pub const APEX_STONE: f32 = 0.001;

/// Number of game frames per trajectory segment.
pub const TIME_FLYSEGMENT: u16 = 4;

/// Distance (map units) at which an arrow can hit a victim.
pub const HIT_DISTANCE: f32 = 15.0;

/// Experience points awarded for a bow kill.
pub const BOW_KILL_EXPERIENCE_POINTS: u32 = 20;

/// Fallback for the post-impact `OBJECT_BURSTING` countdown when the
/// projectile's sprite has no script loaded (e.g. headless tests with
/// `Sprite::default()`).  See `burst_ticks_for_proj` for the live path.
/// 8 ticks is the typical length of the burst strip across apple/stone
/// sprites.
pub const BURST_ANIMATION_FRAMES: u16 = 8;

/// How many ticks the projectile's `OBJECT_BURSTING` animation will
/// take to play out — the total tick count is the sum of every
/// burst-row frame's delay.  Falls back to the
/// [`BURST_ANIMATION_FRAMES`] constant when the sprite has no script
/// (headless tests).
fn burst_ticks_for_proj(proj: &ElementProjectile) -> u16 {
    let dynamic = proj
        .element
        .sprite
        .total_ticks_for_anim(Animation::ObjectBursting);
    if dynamic == 0 {
        BURST_ANIMATION_FRAMES
    } else {
        dynamic
    }
}

/// Z offset added to the bow point for long (high) shots.
const BOW_Z_OFFSET_LONG: f32 = 50.0;

/// Z offset added to the bow point for normal (flat) shots.
const BOW_Z_OFFSET_NORMAL: f32 = 40.0;

// Sprite order ids for bow shots are allocated from `EngineInner::next_order_id`
// (passed in by the caller as `&mut u32`) so rollback / replay reproduces
// the same id sequence.

// ═══════════════════════════════════════════════════════════════════
//  Shield obstacle — arrow blocking geometry
// ═══════════════════════════════════════════════════════════════════

use crate::position_interface::{ASPECT_RATIO, INVERSE_ASPECT_RATIO};

/// Shield dimensions and positioning parameters.
///
/// Derived from entity type and weapon profile.
#[derive(Debug, Clone, Copy)]
pub struct ShieldParams {
    /// Forward offset applied before building the shield box (map-space
    /// direction).  PC uses 10, Soldier uses 20.
    pub pre_offset: f32,
    /// Perpendicular (horizontal) extent of the shield quad.
    pub width: f32,
    /// Forward (depth) extent of the shield quad (always 5).
    pub depth: f32,
    /// Vertical (Z) extent of the shield quad.
    pub height: f32,
    /// Additional Z offset above actor position.
    pub z_offset: f32,
}

/// Shield params for a PC actor.
pub fn shield_params_for_pc(has_big_shield: bool) -> ShieldParams {
    if has_big_shield {
        ShieldParams {
            pre_offset: 10.0,
            width: 40.0,
            depth: 5.0,
            height: 50.0,
            z_offset: 10.0,
        }
    } else {
        ShieldParams {
            pre_offset: 10.0,
            width: 30.0,
            depth: 5.0,
            height: 40.0,
            z_offset: 20.0,
        }
    }
}

/// Shield params for a soldier, derived from the weapon profile.
///
/// Note: the profile's "width" and "height" fields are swapped relative
/// to their geometric meaning:
///   profile "width"  → Z extent (height)
///   profile "height" → horizontal extent (width)
pub fn shield_params_for_soldier(
    profile_shield_width: u16,
    profile_shield_height: u16,
) -> ShieldParams {
    let z_height = profile_shield_width as f32;
    let horiz_width = profile_shield_height as f32;
    let z_offset = if z_height < 40.0 {
        50.0 - z_height
    } else {
        10.0
    };
    ShieldParams {
        pre_offset: 20.0,
        width: horiz_width,
        depth: 5.0,
        height: z_height,
        z_offset,
    }
}

/// Compute a shield obstacle (4-point bounding box) positioned in front
/// of an actor.
///
/// The obstacle is a thin rectangular quad oriented perpendicular to the
/// actor's facing direction, offset forward from the actor's position.
/// Coordinate system: (x, y) in map-space, z in world height.
pub fn compute_shield_obstacle(
    position_map: ElemPoint2D,
    z: f32,
    direction_sector: i16,
    params: &ShieldParams,
) -> crate::sight_obstacle::SightObstacle {
    use crate::element::direction_vector_16;
    use crate::sight_obstacle::{
        ObstaclePoint, SIGHTOBSTACLE_SHIELD, SIGHTOBSTACLE_SOLID, SightObstacle,
    };

    let (dir_x, dir_y) = direction_vector_16(direction_sector);

    // Pre-offset: move position forward in map-space direction
    // before constructing the shield box.
    let px = position_map.x + params.pre_offset * dir_x;
    let py = position_map.y + params.pre_offset * dir_y;
    let pz = z + params.z_offset;

    // Box construction: direction is already unit from
    // `direction_vector_16`; compute perpendicular, apply aspect ratio,
    // then offset another 20 units forward.
    let fwd_x = dir_x;
    let fwd_y = dir_y;

    // Perpendicular (Normal): (-y, x)
    let side_x = -fwd_y;
    let side_y = fwd_x;

    // Apply aspect ratio to both directions.
    let side_x_ar = side_x;
    let side_y_ar = side_y * ASPECT_RATIO;
    let fwd_x_ar = fwd_x;
    let fwd_y_ar = fwd_y * ASPECT_RATIO;

    // Additional 20-unit forward offset.
    let cx = px + 20.0 * fwd_x_ar;
    let cy = py + 20.0 * fwd_y_ar;

    let w = params.width;
    let d = params.depth;
    let h = params.height;

    // Build 4 corner points of the shield quad.
    let points = [
        // point1: left-back
        ObstaclePoint {
            x: cx - 0.5 * w * side_x_ar - 0.5 * d * fwd_x_ar,
            y: cy - 0.5 * w * side_y_ar - 0.5 * d * fwd_y_ar,
            z_bottom: pz,
            z_top: pz + h,
        },
        // point2: left-front
        ObstaclePoint {
            x: cx - 0.5 * w * side_x_ar + 0.5 * d * fwd_x_ar,
            y: cy - 0.5 * w * side_y_ar + 0.5 * d * fwd_y_ar,
            z_bottom: pz,
            z_top: pz + h,
        },
        // point3: right-front
        ObstaclePoint {
            x: cx + 0.5 * w * side_x_ar + 0.5 * d * fwd_x_ar,
            y: cy + 0.5 * w * side_y_ar + 0.5 * d * fwd_y_ar,
            z_bottom: pz,
            z_top: pz + h,
        },
        // point4: right-back
        ObstaclePoint {
            x: cx + 0.5 * w * side_x_ar - 0.5 * d * fwd_x_ar,
            y: cy + 0.5 * w * side_y_ar - 0.5 * d * fwd_y_ar,
            z_bottom: pz,
            z_top: pz + h,
        },
    ];

    // The shield stays SOLID|SHIELD for its entire lifetime. The SOLID
    // bit matters: arrow collision filters on
    // SIGHTOBSTACLE_SOLID|SIGHTOBSTACLE_OPAQUE — without SOLID, arrows
    // pass through shields.
    let mut obstacle = SightObstacle::new(0, SIGHTOBSTACLE_SOLID | SIGHTOBSTACLE_SHIELD);
    obstacle.obstacle_points = points.to_vec();
    obstacle.on_ground = false;

    // Set plane points for 3D ray intersection.
    obstacle.top_plane_points = [
        [points[0].x, points[0].y, points[0].z_top],
        [points[1].x, points[1].y, points[1].z_top],
        [points[2].x, points[2].y, points[2].z_top],
    ];
    obstacle.bottom_plane_points = [
        [points[0].x, points[0].y, points[0].z_bottom],
        [points[1].x, points[1].y, points[1].z_bottom],
        [points[2].x, points[2].y, points[2].z_bottom],
    ];

    obstacle.rebuild_geometry();
    obstacle
}

// ═══════════════════════════════════════════════════════════════════
//  Shoot-mode helpers
// ═══════════════════════════════════════════════════════════════════

/// Determine the shoot mode from the shooter's current action state.
pub fn shoot_mode_from_action_state(state: ActionState) -> ShootMode {
    match state {
        ActionState::AimingWithBowUp => ShootMode::Long,
        ActionState::AimingWithBowDown => ShootMode::Down,
        _ => ShootMode::Normal,
    }
}

/// Whether the shot uses a flat trajectory (low mass, fast).
pub fn is_flat_shot(mode: ShootMode) -> bool {
    matches!(mode, ShootMode::Normal | ShootMode::Down)
}

/// Arrow mass for the given shoot mode.
pub fn arrow_mass(mode: ShootMode) -> f32 {
    if is_flat_shot(mode) {
        MASS_ARROW_FLAT
    } else {
        MASS_ARROW_HIGH
    }
}

/// Determine the appropriate `OrderType` for the shoot animation.
fn shoot_order_type_for_mode(mode: ShootMode, anonymous: bool) -> OrderType {
    match (mode, anonymous) {
        (ShootMode::Normal, true) => OrderType::ShootingWithBowAnonymous,
        (ShootMode::Long, true) => OrderType::ShootingWithBowUpAnonymous,
        (ShootMode::Normal, false) => OrderType::ShootingWithBow,
        (ShootMode::Long, false) => OrderType::ShootingWithBowUp,
        (ShootMode::Down, _) => OrderType::ShootingWithBowLeaningOut,
    }
}

/// C++ `RHElementActorHuman::ComputeBowPoint` selects these non-anonymous
/// animation ids for hotspot lookup even when the active shoot animation is
/// an anonymous archer variant.
pub(crate) fn bow_point_order_type_for_mode(mode: ShootMode) -> OrderType {
    match mode {
        ShootMode::Normal => OrderType::ShootingWithBow,
        ShootMode::Long => OrderType::ShootingWithBowUp,
        ShootMode::Down => OrderType::ShootingWithBowLeaningOut,
    }
}

/// Whether this order type is a bow shoot animation.
fn is_shoot_order(ot: OrderType) -> bool {
    matches!(
        ot,
        OrderType::ShootingWithBow
            | OrderType::ShootingWithBowUp
            | OrderType::ShootingWithBowLeaningOut
            | OrderType::ShootingWithBowAnonymous
            | OrderType::ShootingWithBowUpAnonymous
    )
}

/// Whether this order type is a bow transition animation.
fn is_bow_transition_order(ot: OrderType) -> bool {
    matches!(
        ot,
        OrderType::TransitionRaisingBow
            | OrderType::TransitionLoweringBow
            | OrderType::TransitionRaisingBowLeaningOut
            | OrderType::TransitionLoweringBowLeaningOut
            | OrderType::TransitionLoadingBow
            | OrderType::TransitionUnequipBow
            | OrderType::TransitionRaisingBowAnonymous
            | OrderType::TransitionLoweringBowAnonymous
            | OrderType::TransitionLoadingBowAnonymous
            | OrderType::TransitionUnequipBowAnonymous
    )
}

fn apply_bow_transition_state_side_effect(
    entity: &mut Entity,
    order_type: OrderType,
    motion: SpriteMotionState,
) {
    let action_state = match order_type {
        OrderType::TransitionLoweringBow | OrderType::TransitionLoweringBowAnonymous
            if matches!(
                motion,
                SpriteMotionState::Done | SpriteMotionState::Terminated
            ) =>
        {
            Some(ActionState::AimingWithBow)
        }
        OrderType::TransitionRaisingBow | OrderType::TransitionRaisingBowAnonymous
            if matches!(
                motion,
                SpriteMotionState::Done | SpriteMotionState::Terminated
            ) =>
        {
            Some(ActionState::AimingWithBowUp)
        }
        OrderType::TransitionLoweringBowLeaningOut
            if matches!(
                motion,
                SpriteMotionState::Done | SpriteMotionState::Terminated
            ) =>
        {
            entity.element_data_mut().posture = Posture::LeaningOut;
            Some(ActionState::AimingWithBowDown)
        }
        OrderType::TransitionRaisingBowLeaningOut
            if matches!(
                motion,
                SpriteMotionState::Done | SpriteMotionState::Terminated
            ) =>
        {
            entity.element_data_mut().posture = Posture::Upright;
            Some(ActionState::AimingWithBow)
        }
        OrderType::TransitionUnequipBow | OrderType::TransitionUnequipBowAnonymous
            if matches!(
                motion,
                SpriteMotionState::Start | SpriteMotionState::Done | SpriteMotionState::Terminated
            ) =>
        {
            if entity.element_data().posture != Posture::AnonymousArcher {
                entity.element_data_mut().posture = Posture::Upright;
            }
            Some(ActionState::Waiting)
        }
        _ => None,
    };

    if let Some(action_state) = action_state
        && let Some(actor) = entity.actor_data_mut()
    {
        actor.action_state = action_state;
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Ballistic trajectory computation
// ═══════════════════════════════════════════════════════════════════

/// Compute the initial velocity for a ballistic trajectory.
///
/// - `thrower_to_target`: 3D vector from launch point to target point.
/// - `apex_height`: desired parabola apex height (0 for near-flat shots).
/// - `mass`: projectile mass (determines gravity influence).
/// - `flight_time`: if non-zero, use this fixed flight time (frames);
///   if zero, compute flight time from apex height.
/// - `target_forecasted_movement`: if `Some`, the per-second velocity
///   of the *target* that the shot is leading.  Adds
///   `movement * 0.5 * TIME_FLYSEGMENT` to the velocity to lead a
///   moving target.  Pass `None` for fixed targets to add nothing.
///
/// Returns the initial velocity vector.
pub fn compute_initial_throw_velocity(
    thrower_to_target: Point3D,
    apex_height: f32,
    mass: f32,
    mut flight_time: u16,
    target_forecasted_movement: Option<Point3D>,
) -> Point3D {
    debug_assert!(!(apex_height > 0.0 && mass == 0.0));
    debug_assert!(!(apex_height == 0.0 && mass > 0.0));

    if flight_time == 0 {
        // Estimate flight time from apex height.
        let apex_factor = -mass * GRAVITY * 2.0;
        debug_assert!(apex_factor >= 0.0);
        let mut current_apex = 0.0_f32;
        while current_apex < apex_height {
            flight_time += 1;
            current_apex += flight_time as f32 * apex_factor;
        }
        // Full trajectory = 2× time to apex.
        flight_time *= 2;
    }

    let mut velocity = if flight_time == 1 {
        Point3D {
            x: 0.5 * thrower_to_target.x,
            y: 0.5 * thrower_to_target.y,
            z: 0.5 * thrower_to_target.z,
        }
    } else {
        // Zero-gravity velocity.
        let denom = 0.5 / (flight_time as f32 + 1.0);
        let mut vx = thrower_to_target.x * denom;
        let mut vy = thrower_to_target.y * denom;
        let mut vz = thrower_to_target.z * denom;
        // Correct Z for gravity: vZ -= mass * GRAVITY * flightTime.
        vz -= mass * GRAVITY * flight_time as f32;
        // Clamp any NaN/Inf to zero (safety).
        if !vx.is_finite() {
            vx = 0.0;
        }
        if !vy.is_finite() {
            vy = 0.0;
        }
        if !vz.is_finite() {
            vz = 0.0;
        }
        Point3D {
            x: vx,
            y: vy,
            z: vz,
        }
    };

    // Lead a moving target: add the target's forecasted movement scaled
    // by 0.5 * TIME_FLYSEGMENT.
    if let Some(movement) = target_forecasted_movement {
        let lead_factor = 0.5 * TIME_FLYSEGMENT as f32;
        velocity.x += movement.x * lead_factor;
        velocity.y += movement.y * lead_factor;
        velocity.z += movement.z * lead_factor;
    }

    velocity
}

/// Parameters for trajectory obstacle collision checking.
pub struct TrajectoryObstacleCheck<'a> {
    pub fast_find_grid: &'a crate::fast_find_grid::FastFindGrid,
    pub layer: u16,
    /// Sight obstacles for 3D ray-obstacle intersection.
    /// When provided, each trajectory segment is also checked against
    /// these in full 3D (height-aware), allowing arrows to arc over
    /// walls whose top is below the arrow's trajectory.
    pub sight_obstacles: crate::sight_obstacle::ObstacleList<'a>,
    /// Water / hole zones.  When provided, a landing on a hole sector
    /// triggers the fall-into-hole extension — the projectile slides
    /// to the far edge of the hole polygon before disappearing
    /// (cosmetic polish).  `None` in tests and in callsites that don't
    /// carry the zone list.
    pub water_zones: Option<&'a crate::water_zones::WaterZones>,
}

/// Precompute a ballistic trajectory as a list of waypoints.
///
/// This is the non-bouncing branch used by arrows.
///
/// When `obstacle_check` is provided, each trajectory segment is tested
/// against the obstacle grid (2D motion lines) AND sight obstacles (3D
/// height-aware).  If blocked, the trajectory ends at an approximate
/// impact point.
///
/// Each waypoint stores a 3D position and a duration in frames
/// (`TIME_FLYSEGMENT`).  The arrow advances linearly between consecutive
/// points, giving the visual gravity arc.
pub fn compute_trajectory_ballistic(
    start: Point3D,
    initial_velocity: Point3D,
    mass: f32,
    flat_shot: bool,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
) -> Vec<TrajectoryPoint> {
    compute_trajectory_ballistic_impl(
        start,
        initial_velocity,
        mass,
        flat_shot,
        obstacle_check,
        None,
    )
}

/// Ballistic trajectory with bounce.
///
/// `bounce_factors` is `(vertical, horizontal)` — the projectile's own
/// damping coefficients, further multiplied by the struck obstacle's
/// per-material bounce coefficients on wall / top impacts.  Nets use
/// `(0.1, 0.1)` and wasp nests use the coin bounce factors.
pub fn compute_trajectory_ballistic_bounce(
    start: Point3D,
    initial_velocity: Point3D,
    mass: f32,
    flat_shot: bool,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
    bounce_factors: (f32, f32),
) -> Vec<TrajectoryPoint> {
    compute_trajectory_ballistic_impl(
        start,
        initial_velocity,
        mass,
        flat_shot,
        obstacle_check,
        Some(bounce_factors),
    )
}

/// Impact classifications used by trajectory bounce dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImpactKind {
    Ground,
    Wall,
    Top,
}

/// Result of classifying a trajectory-segment impact.
#[derive(Debug, Clone, Copy)]
struct ImpactInfo {
    kind: ImpactKind,
    /// Unit impact normal (world-space).  For `Ground`: (0,0,1).  For
    /// `Top`: cross(top_plane.v1, top_plane.v2) normalised.  For `Wall`:
    /// the outward XY normal of the crossed polygon edge.
    normal: Point3D,
    /// Per-obstacle bounce factors (vertical, horizontal).
    /// Defaults to `(1.0, 1.0)` for ground (no material factor).
    obstacle_bounce_v: f32,
    obstacle_bounce_h: f32,
}

/// Classify a trajectory impact against an obstacle (or ground if `None`).
fn classify_impact(
    impact: Point3D,
    segment_from: Point3D,
    segment_to: Point3D,
    obstacle: Option<&crate::sight_obstacle::SightObstacle>,
) -> ImpactInfo {
    let Some(obs) = obstacle else {
        return ImpactInfo {
            kind: ImpactKind::Ground,
            normal: Point3D {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            obstacle_bounce_v: 1.0,
            obstacle_bounce_h: 1.0,
        };
    };

    // Is the impact on the top plane of the obstacle?  The raycast
    // returns an impact on either a wall edge or the top/bottom plane;
    // compare impact.z to the top-plane Z at (impact.x, impact.y).
    let top_z = obs.compute_top_z(impact.x, impact.y);
    const TOP_EPS: f32 = 0.5;
    let is_top = (impact.z - top_z).abs() <= TOP_EPS
        && obs.box_ground.contains_point(crate::geo2d::Point2D {
            x: impact.x,
            y: impact.y,
        });

    if is_top {
        // Top-plane normal from the three plane-defining points.
        let [p0, p1, p2] = obs.top_plane_points;
        let v1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        let v2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
        let mut nx = v1[1] * v2[2] - v1[2] * v2[1];
        let mut ny = v1[2] * v2[0] - v1[0] * v2[2];
        let mut nz = v1[0] * v2[1] - v1[1] * v2[0];
        // Ensure the normal points upward (positive Z).
        if nz < 0.0 {
            nx = -nx;
            ny = -ny;
            nz = -nz;
        }
        let norm = (nx * nx + ny * ny + nz * nz).sqrt().max(1e-6);
        return ImpactInfo {
            kind: ImpactKind::Top,
            normal: Point3D {
                x: nx / norm,
                y: ny / norm,
                z: nz / norm,
            },
            obstacle_bounce_v: obs.bounce_vertical,
            obstacle_bounce_h: obs.bounce_horizontal,
        };
    }

    // Wall impact: find the polygon edge closest to the impact point
    // (in 2D map space) and use its outward normal.  Derived from the
    // convex ground polygon which is equivalent for axis-aligned walls
    // (the only obstacle geometry the engine ships).
    let pts = &obs.obstacle_points;
    let (mut best_nx, mut best_ny, mut best_d2) = (1.0_f32, 0.0_f32, f32::INFINITY);
    if !pts.is_empty() {
        let n = pts.len();
        let mut prev = (pts[n - 1].x, pts[n - 1].y);
        for pt_i in pts.iter() {
            let cur = (pt_i.x, pt_i.y);
            let ex = cur.0 - prev.0;
            let ey = cur.1 - prev.1;
            let len2 = ex * ex + ey * ey;
            if len2 > 1e-6 {
                // Closest point on the edge to the impact (2D).
                let px = impact.x - prev.0;
                let py = impact.y - prev.1;
                let t = ((px * ex + py * ey) / len2).clamp(0.0, 1.0);
                let cx = prev.0 + t * ex;
                let cy = prev.1 + t * ey;
                let dx = impact.x - cx;
                let dy = impact.y - cy;
                let d2 = dx * dx + dy * dy;
                if d2 < best_d2 {
                    best_d2 = d2;
                    // Outward normal = perpendicular to edge, pointing
                    // away from the polygon interior.  For a CCW
                    // polygon, the outward normal of edge (prev→cur)
                    // is (ey, -ex).
                    let nx = ey;
                    let ny = -ex;
                    let nlen = (nx * nx + ny * ny).sqrt().max(1e-6);
                    best_nx = nx / nlen;
                    best_ny = ny / nlen;
                }
            }
            prev = cur;
        }
    }

    // Ensure the normal points against the incoming velocity so the
    // reflection is outward.  If it's aligned with the segment
    // direction, flip it.
    let seg_dx = segment_to.x - segment_from.x;
    let seg_dy = segment_to.y - segment_from.y;
    if best_nx * seg_dx + best_ny * seg_dy > 0.0 {
        best_nx = -best_nx;
        best_ny = -best_ny;
    }

    ImpactInfo {
        kind: ImpactKind::Wall,
        normal: Point3D {
            x: best_nx,
            y: best_ny,
            z: 0.0,
        },
        obstacle_bounce_v: obs.bounce_vertical,
        obstacle_bounce_h: obs.bounce_horizontal,
    }
}

/// Apply the bounce reflection for an impact.  Three branches:
/// ground, wall, and top-of-obstacle.
///
/// `velocity` is the pre-impact velocity; `new_vz` is what `vz` would
/// have been after gravity for this step (used on top impacts so the
/// bounce integrates the fractional-step gravity correction).
fn apply_bounce_reflection(
    velocity: Point3D,
    new_vz: f32,
    ratio: f32,
    info: ImpactInfo,
    projectile_bounce: (f32, f32),
) -> Point3D {
    let (proj_bv, proj_bh) = projectile_bounce;
    match info.kind {
        ImpactKind::Ground => Point3D {
            x: velocity.x * proj_bh,
            y: velocity.y * proj_bh,
            z: -velocity.z * proj_bv,
        },
        ImpactKind::Wall => {
            // The wall normal is stored in screen-compressed Y;
            // un-compress before reflecting (so the reflection is
            // geometrically correct in world space), then re-compress
            // the reflected Y.
            let mut n = info.normal;
            n.x *= INVERSE_ASPECT_RATIO;
            let inv_norm = 1.0 / (n.x * n.x + n.y * n.y).sqrt().max(1e-6);
            n.x *= inv_norm;
            n.y *= inv_norm;

            let dot = velocity.x * n.x + velocity.y * n.y;
            let comp_x = -2.0 * dot * n.x;
            let comp_y = -2.0 * dot * n.y;

            Point3D {
                x: info.obstacle_bounce_h * proj_bh * (velocity.x + comp_x),
                y: ASPECT_RATIO * (info.obstacle_bounce_h * proj_bh * (velocity.y + comp_y)),
                z: velocity.z,
            }
        }
        ImpactKind::Top => {
            // Full 3D normal reflection with aspect-ratio correction
            // on X and Z.  Use the fractional-step gravity-corrected
            // vz, interpolated via `(1-ratio) * vz + ratio * new_vz`.
            let interp_vz = (1.0 - ratio) * velocity.z + ratio * new_vz;
            let mut n = info.normal;
            n.x *= INVERSE_ASPECT_RATIO;
            n.z *= INVERSE_ASPECT_RATIO;
            let inv_norm = 1.0 / (n.x * n.x + n.y * n.y + n.z * n.z).sqrt().max(1e-6);
            n.x *= inv_norm;
            n.y *= inv_norm;
            n.z *= inv_norm;

            let dot = velocity.x * n.x + velocity.y * n.y + interp_vz * n.z;
            let comp_x = -2.0 * dot * n.x;
            let comp_y = -2.0 * dot * n.y;
            let comp_z = -2.0 * dot * n.z;

            Point3D {
                x: info.obstacle_bounce_h * proj_bh * (velocity.x + comp_x),
                y: ASPECT_RATIO * info.obstacle_bounce_h * proj_bh * (velocity.y + comp_y),
                z: info.obstacle_bounce_v * proj_bv * (interp_vz + comp_z),
            }
        }
    }
}

fn compute_trajectory_ballistic_impl(
    start: Point3D,
    initial_velocity: Point3D,
    mass: f32,
    _flat_shot: bool,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
    bounce: Option<(f32, f32)>,
) -> Vec<TrajectoryPoint> {
    /// Top-impact termination speed threshold (`||v|| < 5`).
    /// Only applies when the previous iteration hit an obstacle's top
    /// plane; ground bounces use the gravity-chained termination
    /// (`new_vz <= 0` after a Ground bounce), and wall bounces never
    /// terminate at the top of the loop.
    const BOUNCE_TOP_MIN_SPEED: f32 = 5.0;

    let fg = GRAVITY * mass;
    let mut trajectory = Vec::new();
    let mut velocity = initial_velocity;
    let mut position = start;

    // Last impact kind carried across loop iterations (reset to None
    // on a clear free-flight step).  The kind plus the impact's surface
    // normal drive the gravity-chained termination at the top of the
    // *next* iteration.
    let mut last_impact: Option<ImpactInfo> = None;

    for _ in 0..50 {
        let new_vz = fg * 2.0 + velocity.z;

        // Termination: bouncing projectiles settle when (a) the previous
        // bounce was on the ground and the reflected vz can't overcome
        // 2*g this step, or (b) the previous bounce was on an obstacle
        // top plane and the reflected velocity has either gone back
        // into the surface or dropped below the speed floor.  Wall
        // bounces never terminate here — the projectile keeps flying
        // until it hits the ground or another obstacle.  Non-bounce
        // projectiles use the simpler z<0 shortcut.
        if bounce.is_some() {
            let was_ground = matches!(
                last_impact,
                Some(ImpactInfo {
                    kind: ImpactKind::Ground,
                    ..
                })
            );
            if (was_ground || position.z < 0.0) && new_vz <= 0.0 {
                break;
            }
            if let Some(info) = last_impact
                && info.kind == ImpactKind::Top
            {
                let dot = velocity.x * info.normal.x
                    + velocity.y * info.normal.y
                    + velocity.z * info.normal.z;
                let speed_sq =
                    velocity.x * velocity.x + velocity.y * velocity.y + velocity.z * velocity.z;
                if dot < 0.0 || speed_sq < BOUNCE_TOP_MIN_SPEED * BOUNCE_TOP_MIN_SPEED {
                    break;
                }
            }
        } else if position.z < 0.0 && new_vz <= 0.0 {
            break;
        }

        // Pre-emptive ground bounce when a previous free-flight step
        // pushed the projectile below z=0.  `is_reachable_impact_3d`
        // does not test the z=0 plane explicitly, so reflect inline
        // here.  Tagging `last_impact = Ground` lets the next iteration's
        // gravity-chained termination above stop the bounce when the
        // reflected vz can't overcome 2*g.
        if let Some((bv, bh)) = bounce
            && position.z < 0.0
        {
            velocity = Point3D {
                x: velocity.x * bh,
                y: velocity.y * bh,
                z: -velocity.z * bv,
            };
            position.z = 0.0;
            last_impact = Some(ImpactInfo {
                kind: ImpactKind::Ground,
                normal: Point3D {
                    x: 0.0,
                    y: 0.0,
                    z: 1.0,
                },
                obstacle_bounce_v: 1.0,
                obstacle_bounce_h: 1.0,
            });
            continue;
        }

        let new_position = Point3D {
            x: velocity.x * 2.0 + position.x,
            y: velocity.y * 2.0 + position.y,
            z: velocity.z * 2.0 + position.z,
        };

        if let Some(check) = obstacle_check {
            // 3D raycast: finds the first blocking obstacle (or ground
            // crossing) and returns the impact point plus the obstacle
            // index (None for ground).
            // C++ `RHElementProjectile::ComputeTrajectory` checks
            // projectile flight with `SIGHTOBSTACLE_SOLID` only.  The
            // earlier reachability gate may use opaque obstacles to
            // select long-shot mode, but the arrow itself must not be
            // clipped by opaque-only sight blockers.
            let impact_3d = crate::sight_obstacle::is_reachable_impact_3d(
                crate::position_interface::Point3D {
                    x: position.x,
                    y: position.y,
                    z: position.z,
                },
                crate::position_interface::Point3D {
                    x: new_position.x,
                    y: new_position.y,
                    z: new_position.z,
                },
                crate::sight_obstacle::SIGHTOBSTACLE_SOLID,
                check.sight_obstacles,
                Some(check.fast_find_grid.level.map_bbox),
            );

            // Compute the 3D impact ratio relative to the current
            // segment.  C++ projectile trajectory integration uses
            // `RHFastFindGrid::IsReachableImpact` over sight obstacles;
            // it does not collide with pathfinding/motion-line
            // geometry.  Keep this path sight-obstacle-only.
            let ratio_3d = impact_3d.as_ref().map(|r| {
                let seg_dx = new_position.x - position.x;
                let seg_dy = new_position.y - position.y;
                let seg_dz = new_position.z - position.z;
                let seg_len_sq = seg_dx * seg_dx + seg_dy * seg_dy + seg_dz * seg_dz;
                if seg_len_sq <= 1e-9 {
                    0.0
                } else {
                    let ix = r.impact.x - position.x;
                    let iy = r.impact.y - position.y;
                    let iz = r.impact.z - position.z;
                    ((ix * seg_dx + iy * seg_dy + iz * seg_dz) / seg_len_sq).clamp(0.0, 1.0)
                }
            });

            let Some(ratio) = ratio_3d else {
                trajectory.push(TrajectoryPoint {
                    position: new_position,
                    time: TIME_FLYSEGMENT,
                });
                velocity.z = new_vz;
                position = new_position;
                last_impact = None;
                continue;
            };
            let r = impact_3d.unwrap();
            let obstacle = r
                .obstacle_index
                .and_then(|i| check.sight_obstacles.get(i as usize));
            let impact = Point3D {
                x: r.impact.x,
                y: r.impact.y,
                z: r.impact.z,
            };
            tracing::trace!(
                ratio,
                ?position,
                ?new_position,
                ?impact,
                obstacle_type = obstacle.map(|o| o.obstacle_type),
                obstacle_layer = obstacle.map(|o| o.layer),
                "Projectile trajectory clipped by obstacle"
            );

            let impact_time = ((TIME_FLYSEGMENT as f32 * ratio + 0.5) as u16).max(1);
            trajectory.push(TrajectoryPoint {
                position: impact,
                time: impact_time,
            });

            if let Some(proj_bounce) = bounce {
                let info = classify_impact(impact, position, new_position, obstacle);
                let new_vel = apply_bounce_reflection(velocity, new_vz, ratio, info, proj_bounce);

                // Water mid-trajectory: if a bouncing projectile
                // impacts a water / hole sector, it dives instead of
                // continuing the bounce integration.  Stop here;
                // `maybe_splash_on_landing` marks `dive`.
                if let Some(water_zones) = check.water_zones {
                    let map_pt = crate::geo2d::Point2D {
                        x: impact.x,
                        y: impact.y - impact.z,
                    };
                    if water_zones.determine_water_hole(map_pt).is_some() {
                        break;
                    }
                }

                velocity = new_vel;
                position = impact;
                // Record the impact kind so the next iteration's
                // termination check can fire.
                last_impact = Some(info);
                continue;
            }
            break;
        }

        trajectory.push(TrajectoryPoint {
            position: new_position,
            time: TIME_FLYSEGMENT,
        });

        velocity.z = new_vz;
        position = new_position;
        last_impact = None;
    }

    // Fall-into-hole extension: if the landing point is inside a
    // hole zone, slide the projectile to the far edge of the hole
    // before it disappears.  Visual polish only; a projectile
    // stopping at the hole's near lip would otherwise float in
    // mid-air (since holes have no back-wall to catch it).
    if let Some(check) = obstacle_check
        && let Some(water_zones) = check.water_zones
        && trajectory.len() >= 2
    {
        let last = trajectory[trajectory.len() - 1].position;
        let prev = trajectory[trajectory.len() - 2].position;
        let landing_map = crate::geo2d::Point2D {
            x: last.x,
            y: last.y - last.z,
        };
        let prev_map = crate::geo2d::Point2D {
            x: prev.x,
            y: prev.y - prev.z,
        };
        if let Some(exit) = water_zones.find_hole_far_exit(prev_map, landing_map) {
            // Duration proportional to the 2D distance from the
            // landing point to the exit.
            let dx = exit.x - landing_map.x;
            let dy = exit.y - landing_map.y;
            let extension_dist = (dx * dx + dy * dy).sqrt();
            let prev_seg_dist = {
                let sdx = landing_map.x - prev_map.x;
                let sdy = landing_map.y - prev_map.y;
                (sdx * sdx + sdy * sdy).sqrt()
            };
            let time = if prev_seg_dist > 0.0 {
                let speed = trajectory.last().unwrap().time as f32 / prev_seg_dist;
                ((extension_dist * speed) as u16).max(1)
            } else {
                1
            };
            // Keep the landing world-Z; the extension slides in map
            // space so screen depth advances while world height stays
            // flat.
            trajectory.push(TrajectoryPoint {
                position: Point3D {
                    x: exit.x,
                    y: exit.y + last.z,
                    z: last.z,
                },
                time,
            });
        }
    }

    trajectory
}

// ═══════════════════════════════════════════════════════════════════
//  Bow point — hand anchor offset
// ═══════════════════════════════════════════════════════════════════

/// Compute the 3D launch point for an arrow.
///
/// Takes the shooter's 3D entity position (`Entity::element_data().position`)
/// where `.z` is the ground elevation and `.y` already includes elevation.
///
/// `sprite_hand_point`: absolute 2D position of the hand anchor, computed as
/// `sprite_position + hotspot_offset` by the caller.  When the hotspot is
/// missing, the caller falls back to `sprite_position + (0,0)`.  The
/// `None` case here is only for headless tests with no sprite at all.
///
/// For down shots, the bow point is shifted laterally by 20 units along
/// the facing direction vector.
pub fn compute_bow_point(
    position: Point3D,
    shoot_mode: ShootMode,
    direction: i16,
    sprite_hand_point: Option<crate::geo2d::Point2D>,
) -> Point3D {
    let elevation = position.z;

    // Use the absolute hand point for X/Y; fall back to entity position
    // only when there is no sprite at all (headless tests).
    let (hand_x, hand_y) = match sprite_hand_point {
        Some(pt) => (pt.x, pt.y),
        None => (position.x, position.y),
    };

    // Isometric projection: elevation shifts the sprite upward on screen,
    // so add it into the hand Y.
    let hand_y = hand_y + elevation;

    match shoot_mode {
        ShootMode::Long => Point3D {
            x: hand_x,
            y: hand_y,
            z: elevation + BOW_Z_OFFSET_LONG,
        },
        ShootMode::Normal => Point3D {
            x: hand_x,
            y: hand_y,
            z: elevation + BOW_Z_OFFSET_NORMAL,
        },
        ShootMode::Down => {
            // Leaning-out soldiers shift the bow point by 20 units
            // along the direction vector.
            let (dx, dy) = crate::element::direction_vector_16(direction);
            Point3D {
                x: hand_x + dx * 20.0,
                y: hand_y + dy * 20.0,
                z: elevation + BOW_Z_OFFSET_NORMAL,
            }
        }
    }
}

/// Apply projectile landing membership (obstacle / layer / sector)
/// to a projectile element after its trajectory has settled.
pub fn apply_projectile_landing_resolution(
    element: &mut ElementData,
    resolution: crate::fast_find_grid::ProjectileLandingResolution,
    obstacle_plane: Option<crate::position_interface::PlaneZCoeffs>,
) {
    element.set_obstacle_index(
        resolution.obstacle_index,
        obstacle_plane.or(resolution.obstacle_plane),
    );
    element.set_sector(resolution.sector);
    if !resolution.blocked_by_motion_obstacle {
        element.set_layer(resolution.layer);
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Hit-chance & miss bias
// ═══════════════════════════════════════════════════════════════════

/// Roll the hit chance and compute a miss-bias velocity offset.
///
/// ```text
/// if ((rand() % 100 + 1) > hit_chance) {
///     bias = (rand()%5-2, rand()%5-2, rand()%5-2);
///     bias *= 1 - (skill / 100.0);
///     velocity += bias;
/// }
/// ```
///
/// Returns `Some(bias)` if the shot misses (caller adds to velocity),
/// or `None` if the shot hits.
pub fn roll_hit_and_compute_bias(hit_chance: u32, bow_skill_capacity: u32) -> Option<Point3D> {
    let roll: u32 = crate::sim_rng::u32(1..=100);

    if roll <= hit_chance {
        // Hit!
        return None;
    }

    // Miss — compute random bias, scaled by inverse skill.
    let skill_factor = 1.0 - (bow_skill_capacity.min(100) as f32 / 100.0);
    let bx = (crate::sim_rng::u32(0..5) as f32 - 2.0) * skill_factor;
    let by = (crate::sim_rng::u32(0..5) as f32 - 2.0) * skill_factor;
    let bz = (crate::sim_rng::u32(0..5) as f32 - 2.0) * skill_factor;

    Some(Point3D {
        x: bx,
        y: by,
        z: bz,
    })
}

/// Check if a precomputed trajectory will hit a target point.
///
/// Walks consecutive trajectory waypoints and checks if the target
/// point is within [`HIT_DISTANCE`] of any segment.  Uses the 3D
/// point-to-line-segment perpendicular distance.
///
/// Seeds `last` from `trajectory[0]` and walks indices `1..N`,
/// inspecting the N-1 inter-waypoint segments. The launch segment
/// (from the shooter's hand/bow anchor to the first stored waypoint)
/// is intentionally not tested — the first stored waypoint already
/// sits one `TIME_FLYSEGMENT` integration step downrange.
pub fn will_hit_target(trajectory: &[TrajectoryPoint], _start: Point3D, target: Point3D) -> bool {
    if trajectory.len() < 2 {
        return false;
    }

    let mut last = trajectory[0].position;

    for tp in &trajectory[1..] {
        let current = tp.position;
        // Segment vector (A→B)
        let abx = current.x - last.x;
        let aby = current.y - last.y;
        let abz = current.z - last.z;
        let seg_len_sq = abx * abx + aby * aby + abz * abz;
        let seg_len = seg_len_sq.sqrt();

        if seg_len < 0.001 {
            last = current;
            continue;
        }

        // Segment must be longer than distance from start to target.
        // This ensures the target is "in front of" or "alongside" the
        // segment, not behind the start point.
        let apx = target.x - last.x;
        let apy = target.y - last.y;
        let apz = target.z - last.z;
        let dist_to_target = (apx * apx + apy * apy + apz * apz).sqrt();

        if seg_len > dist_to_target {
            // Perpendicular distance from target to the line through (last, current).
            // ||AP × AB|| / ||AB||
            let cx = apy * abz - apz * aby;
            let cy = apz * abx - apx * abz;
            let cz = apx * aby - apy * abx;
            let cross_len = (cx * cx + cy * cy + cz * cz).sqrt();
            let perp_dist = cross_len / seg_len;

            if perp_dist <= HIT_DISTANCE {
                return true;
            }
        }

        last = current;
    }

    false
}

/// Compute velocity and trajectory parameters for a shot.
///
/// Returns `(initial_velocity, flight_time_hint, apex_height)`.
///
/// `target_forecasted_movement`: when `Some`, the target's per-second
/// movement vector (from `PositionInterface::get_forecasted_movement`)
/// — used to lead the shot.  Pass `None` for stationary FX targets.
pub fn compute_shot_velocity_params(
    bow_point: Point3D,
    target_point: Point3D,
    shoot_mode: ShootMode,
    target_forecasted_movement: Option<Point3D>,
) -> (Point3D, u16, f32) {
    let to_target = Point3D {
        x: target_point.x - bow_point.x,
        y: target_point.y - bow_point.y,
        z: target_point.z - bow_point.z,
    };
    let hit_distance =
        (to_target.x * to_target.x + to_target.y * to_target.y + to_target.z * to_target.z).sqrt();
    let mass = arrow_mass(shoot_mode);

    match shoot_mode {
        ShootMode::Normal | ShootMode::Down => {
            // Flat shot: fixed flight time.
            let flight_time = (0.003 * hit_distance) as u16 + 1;
            let velocity = compute_initial_throw_velocity(
                to_target,
                0.001,
                mass,
                flight_time,
                target_forecasted_movement,
            );
            (velocity, flight_time, 0.001)
        }
        ShootMode::Long => {
            // High shot: compute flight time from apex height.
            // Apex = distance / 10.0, with a minimum of 1.0.
            let apex_height = (hit_distance / 10.0).max(1.0);
            let velocity = compute_initial_throw_velocity(
                to_target,
                apex_height,
                mass,
                0,
                target_forecasted_movement,
            );
            (velocity, 0, apex_height)
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Aim transition helpers
// ═══════════════════════════════════════════════════════════════════

/// Compute the transition orders needed to move from the current aim
/// state to the desired shoot mode.
///
/// Returns 0–2 transition `OrderType`s that should be pushed before the
/// shoot animation order.
fn aim_transition_orders(
    current_state: ActionState,
    desired_mode: ShootMode,
    anonymous: bool,
) -> Vec<OrderType> {
    let mut transitions = Vec::new();

    match desired_mode {
        ShootMode::Normal => {
            match current_state {
                ActionState::AimingWithBowUp => {
                    // Lower from up → normal
                    transitions.push(if anonymous {
                        OrderType::TransitionLoweringBowAnonymous
                    } else {
                        OrderType::TransitionLoweringBow
                    });
                }
                ActionState::AimingWithBowDown => {
                    // Raise from leaning-out → normal
                    transitions.push(OrderType::TransitionRaisingBowLeaningOut);
                }
                _ => {} // Already in correct position or first shot
            }
        }
        ShootMode::Long => {
            match current_state {
                ActionState::AimingWithBow => {
                    // Raise from normal → up
                    transitions.push(if anonymous {
                        OrderType::TransitionRaisingBowAnonymous
                    } else {
                        OrderType::TransitionRaisingBow
                    });
                }
                ActionState::AimingWithBowDown => {
                    // Raise from leaning-out → normal → up
                    transitions.push(OrderType::TransitionRaisingBowLeaningOut);
                    transitions.push(if anonymous {
                        OrderType::TransitionRaisingBowAnonymous
                    } else {
                        OrderType::TransitionRaisingBow
                    });
                }
                _ => {} // Already up or first shot
            }
        }
        ShootMode::Down => {
            match current_state {
                ActionState::AimingWithBow => {
                    // Lower to leaning-out
                    transitions.push(OrderType::TransitionLoweringBowLeaningOut);
                }
                ActionState::AimingWithBowUp => {
                    // Lower from up → normal → leaning-out
                    transitions.push(if anonymous {
                        OrderType::TransitionLoweringBowAnonymous
                    } else {
                        OrderType::TransitionLoweringBow
                    });
                    transitions.push(OrderType::TransitionLoweringBowLeaningOut);
                }
                _ => {} // Already down or first shot
            }
        }
    }

    transitions
}

// ═══════════════════════════════════════════════════════════════════
//  Public dispatch
// ═══════════════════════════════════════════════════════════════════

/// Outcome of attempting to start a bow shot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeginShotResult {
    /// Shooter will play the shoot animation; arrow will spawn on the
    /// action-done frame.  The sequence element is now `InProgress`.
    Started,
    /// Shooter or target no longer valid (dead, despawned, wrong kind).
    /// The sequence element should be marked `Impossible`.
    Impossible,
}

/// Begin a bow shot on behalf of a `Command::ShootBow` sequence element.
///
/// Called from the engine's sequence-action dispatch when it sees a
/// `Command::ShootBow` instruction for an actor owner.
///
/// The function determines the required shoot mode based on target
/// distance, inserts any necessary aim-transition orders, the shoot
/// animation order, and a reload/unequip order after the shot.
///
/// Returns [`BeginShotResult::Started`] if the shooter has been queued
/// to play the shoot animation; [`BeginShotResult::Impossible`] if the
/// shooter or target is not in a valid state.
#[allow(clippy::too_many_arguments)]
pub fn begin_bow_shot(
    entities: &mut [Option<Entity>],
    sequence_manager: &mut SequenceManager,
    shooter_id: EntityId,
    target_id: EntityId,
    seq_id: SequenceId,
    elem_idx: usize,
    shoot_once: bool,
    ammo_count: u32,
    // Shoot mode determined by the engine via `can_shoot_with_bow_at`.
    // `None` means the engine couldn't determine a mode; falls back
    // to the current action state or Normal.
    resolved_shoot_mode: Option<ShootMode>,
    next_order_id: &mut u32,
) -> BeginShotResult {
    // Validate target: must exist, be shootable, and not be the shooter.
    if shooter_id == target_id {
        return BeginShotResult::Impossible;
    }
    let target_valid = match entities.get(target_id.0 as usize).and_then(|s| s.as_ref()) {
        Some(e) if e.is_human() => !e.is_dead() && e.is_active(),
        Some(Entity::Target(t)) => {
            t.element.active && t.target.action_filter.contains(TargetFilter::ARROW)
        }
        None => false,
        Some(_) => false,
    };
    if !target_valid {
        return BeginShotResult::Impossible;
    }

    // Read target ground position for direction/order selection.
    let (tx, ty) = match entities.get(target_id.0 as usize).and_then(|s| s.as_ref()) {
        Some(e) if e.is_fx_target() => {
            let pos = e.element_data().position();
            // Rust 3D sprite points include elevation in Y; C++ bow launch
            // direction uses the target projected back onto the ground plane.
            (pos.x, pos.y - pos.z)
        }
        Some(e) => {
            let position_map = e.element_data().position_map();
            (position_map.x, position_map.y)
        }
        None => return BeginShotResult::Impossible,
    };

    // Validate shooter.  Read posture before the mutable borrow.
    let (shooter_valid, shooter_posture, current_state) =
        match entities.get(shooter_id.0 as usize).and_then(|s| s.as_ref()) {
            Some(e) if e.is_human() && !e.is_dead() => {
                let posture = e.element_data().posture;
                let action = e.actor_data().map(|a| a.action_state);
                let active = e.actor_data().map(|a| a.active_shot.is_active());
                if active == Some(true) {
                    (false, posture, ActionState::Waiting)
                } else {
                    (true, posture, action.unwrap_or(ActionState::Waiting))
                }
            }
            _ => return BeginShotResult::Impossible,
        };
    if !shooter_valid {
        return BeginShotResult::Impossible;
    }

    let shooter = match entities
        .get_mut(shooter_id.0 as usize)
        .and_then(|s| s.as_mut())
    {
        Some(e) => e,
        None => return BeginShotResult::Impossible,
    };
    let actor = match shooter.actor_data_mut() {
        Some(a) => a,
        None => return BeginShotResult::Impossible,
    };

    // Determine the desired shoot mode.  The engine resolves the mode
    // up front via `can_shoot_with_bow_at` and passes it in; we
    // override for leaning-out, then fall back to the current action
    // state or Normal.
    let desired_mode = if shooter_posture == Posture::LeaningOut {
        ShootMode::Down
    } else if let Some(mode) = resolved_shoot_mode {
        mode
    } else if current_state.is_bow() {
        shoot_mode_from_action_state(current_state)
    } else {
        ShootMode::Normal
    };

    // Set the action state to the matching aim state.
    actor.action_state = match desired_mode {
        ShootMode::Normal => ActionState::AimingWithBow,
        ShootMode::Long => ActionState::AimingWithBowUp,
        ShootMode::Down => ActionState::AimingWithBowDown,
    };
    let order_id = crate::order::alloc_order_id(next_order_id);
    actor.active_shot = ActiveShot {
        sequence_id: Some(seq_id),
        element_index: elem_idx,
        target: Some(target_id),
        order_id: Some(order_id),
        released: false,
    };
    actor.clear_path();

    // Push aim-transition orders if needed.  Orders live on the owning
    // `SequenceElement.orders` — when the element is cancelled, its
    // orders go with it.
    let _ = actor; // actor mutable borrow ends here; below we borrow sequence_manager instead
    let anonymous = shooter_posture == Posture::AnonymousArcher;
    let transitions = aim_transition_orders(current_state, desired_mode, anonymous);
    for t in &transitions {
        let mut order = Order::new(*t, tx, ty, crate::order::alloc_order_id(next_order_id));
        order.compute_direction = false;
        sequence_manager.push_order_on(seq_id, elem_idx, order);
    }

    // Push the shoot animation order.
    let shoot_ot = shoot_order_type_for_mode(desired_mode, anonymous);
    let mut order = Order::new(shoot_ot, tx, ty, order_id);
    order.target_actor = Some(target_id.0);
    order.compute_direction = false;
    order.lock_ai = true;
    sequence_manager.push_order_on(seq_id, elem_idx, order);

    // Push reload or unequip order after the shot.
    // If ammo > 1 and not a one-shot command → LOADING_BOW, else UNEQUIP_BOW.
    if ammo_count > 1 && !shoot_once {
        // Reload — keep aiming.  Anonymous archers use the anonymous
        // variant of the transition.
        let reload_ot = if shooter_posture == Posture::AnonymousArcher {
            OrderType::TransitionLoadingBowAnonymous
        } else {
            OrderType::TransitionLoadingBow
        };
        let mut reload_order = Order::new(
            reload_ot,
            tx,
            ty,
            crate::order::alloc_order_id(next_order_id),
        );
        reload_order.compute_direction = false;
        sequence_manager.push_order_on(seq_id, elem_idx, reload_order);

        // DownShoot needs an extra lowering transition after reload.
        if desired_mode == ShootMode::Down {
            let mut lower = Order::new(
                OrderType::TransitionLoweringBowLeaningOut,
                tx,
                ty,
                crate::order::alloc_order_id(next_order_id),
            );
            lower.compute_direction = false;
            sequence_manager.push_order_on(seq_id, elem_idx, lower);
        }
    } else {
        // Unequip — last arrow or one-shot command.  Anonymous archers
        // use the anonymous variant of the transition.
        let unequip_ot = if shooter_posture == Posture::AnonymousArcher {
            OrderType::TransitionUnequipBowAnonymous
        } else {
            OrderType::TransitionUnequipBow
        };
        let mut unequip_order = Order::new(
            unequip_ot,
            tx,
            ty,
            crate::order::alloc_order_id(next_order_id),
        );
        unequip_order.compute_direction = false;
        sequence_manager.push_order_on(seq_id, elem_idx, unequip_order);
    }

    // Face the target via the normal direction-goal path.  C++ sets the
    // shoot animation's facing at initialization, then freezes the first
    // frame while Turn() rotates toward it; do not snap instantly here.
    let shooter = entities
        .get_mut(shooter_id.0 as usize)
        .and_then(|s| s.as_mut())
        .unwrap();
    let shooter_pos = shooter.element_data().position_map();
    let dx = tx - shooter_pos.x;
    let dy = ty - shooter_pos.y;
    shooter.element_data_mut().set_direction_goal(
        crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy),
    );

    BeginShotResult::Started
}

// ═══════════════════════════════════════════════════════════════════
//  Per-frame bow-shot tick
// ═══════════════════════════════════════════════════════════════════

/// Per-actor outcome returned from the bow-shot tick.  The engine uses
/// this to compute the trajectory, spawn the arrow, and notify the
/// sequence manager *after* the mutable borrow on `entities` is released.
pub struct ShotTickResult {
    pub shooter: EntityId,
    pub target: EntityId,
    pub seq_id: SequenceId,
    pub elem_idx: usize,
    /// Shooter's 3D entity position (`.z` = ground elevation).
    pub shooter_position: Point3D,
    /// Target's 2D map position (for arrow direction / spawn).
    pub target_pos: ElemPoint2D,
    /// Target's 3D belt point (for trajectory computation).
    pub target_point: Point3D,
    /// The action state at the moment the shot was released —
    /// determines shoot mode, arrow mass, and damage lookup.
    pub action_state: ActionState,
    /// Shooter's facing direction (0–15) for bow-point computation.
    pub shooter_direction: i16,
    /// Sprite hand anchor point (sprite position + hotspot), if available.
    pub sprite_hand_point: Option<crate::geo2d::Point2D>,
    /// Target's forecasted movement vector for leading shots.
    pub target_forecasted_movement: Point3D,
}

#[derive(Default)]
pub struct BowTickEvents {
    pub fired: Vec<ShotTickResult>,
    pub completed: Vec<(SequenceId, usize)>,
}

/// Advance the shoot animation for every actor with an [`ActiveShot`].
///
/// Returns a list of results for actors whose shoot animation reached
/// `MotionState::Done` this frame — the engine computes the trajectory,
/// spawns an arrow, and notifies the sequence manager for each.  When
/// the shoot animation completes the actor returns to the
/// AimingWithBow action state.
pub fn tick_bow_shots(
    entities: &mut [Option<Entity>],
    sequence_manager: &mut SequenceManager,
) -> BowTickEvents {
    let mut events = BowTickEvents::default();
    let target_ground_positions: Vec<Option<ElemPoint2D>> = entities
        .iter()
        .map(|slot| {
            slot.as_ref()
                .map(|entity| entity.element_data().position_map())
        })
        .collect();

    for (idx, slot) in entities.iter_mut().enumerate() {
        let entity = match slot {
            Some(e) => e,
            None => continue,
        };
        let actor = match entity.actor_data() {
            Some(a) => a,
            None => continue,
        };
        if !actor.active_shot.is_active() {
            continue;
        }
        let shot = actor.active_shot;
        let direction = entity.element_data().direction();
        // Build 3D shooter position: X/Y from the live map position,
        // Z from the position interface's elevation.
        // The element_data().position field is a dead field; the live
        // ground position is in position_map.
        let elevation = entity.position_iface().get_elevation();
        let shooter_position = Point3D {
            x: entity.element_data().position_map().x,
            y: entity.element_data().position_map().y,
            z: elevation,
        };
        let _action_state = actor.action_state;

        // Peek at the current order to determine what animation to drive.
        // Orders live on the owning `SequenceElement.orders` (looked up via
        // the `active_shot` handle), not `actor.order_queue`.
        let (shot_seq_id, shot_elem_idx) = match (shot.sequence_id, shot.element_index) {
            (Some(id), ix) => (id, ix),
            _ => continue,
        };
        let (current_order_type, current_order_id) = match sequence_manager
            .get_element(shot_seq_id, shot_elem_idx)
            .and_then(|e| e.current_order())
        {
            Some(o) => (o.order_type, Some(o.order_id)),
            None => continue,
        };

        let mut direction = direction;
        let mut frame_progression = crate::sprite::FrameProgression::Default;
        if is_shoot_order(current_order_type)
            && let Some(target_id) = shot.target
            && let Some(Some(target_pos)) = target_ground_positions.get(target_id.0 as usize)
        {
            let shooter_pos = entity.element_data().position_map();
            let dx = target_pos.x - shooter_pos.x;
            let dy = target_pos.y - shooter_pos.y;
            entity.element_data_mut().set_direction_goal(
                crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy),
            );
            if entity.element_data_mut().sprite.position_iface.turn() {
                frame_progression = crate::sprite::FrameProgression::FrozenFirstFrame;
            }
            direction = entity.element_data().direction();
        }
        let Ok(dir_u16) = u16::try_from(direction) else {
            tracing::warn!(
                shooter = idx,
                direction,
                "Bow shot tick skipped: invalid shooter direction"
            );
            continue;
        };

        // Drive the current animation through the sprite.
        //
        // Bow shots have a different headless contract than every
        // other animation site: the test in
        // `tick_bow_shots_fires_arrow_and_returns_to_aiming` (and any
        // future headless flow) expects each transition order to
        // resolve immediately when no real sprite is bound, instead of
        // staying in-progress forever.  `Sprite::perform_action`'s
        // generic empty-conversion fallback returns `InProgress`
        // (the right default for animation-tick callers, who can't
        // mark an order Impossible while scripts are still loading);
        // here we explicitly opt out and synthesize `Done`.
        let elem = entity.element_data_mut();
        let motion = if elem.sprite.scripts.is_empty() {
            if frame_progression == crate::sprite::FrameProgression::FrozenFirstFrame {
                SpriteMotionState::InProgress
            } else {
                SpriteMotionState::Done
            }
        } else {
            elem.sprite.perform_action(
                current_order_id,
                current_order_type,
                dir_u16,
                frame_progression,
                false,
            )
        };

        let transition_start =
            is_bow_transition_order(current_order_type) && motion == SpriteMotionState::Start;
        if !transition_start
            && !matches!(
                motion,
                SpriteMotionState::Done
                    | SpriteMotionState::Terminated
                    | SpriteMotionState::Aborted
            )
        {
            continue;
        }

        // Animation for current order reached a state with bow side
        // effects or queue-completion behavior.

        if is_bow_transition_order(current_order_type) {
            // Update action state on the action-done pulse, matching the
            // C++ execute arm.  Keep the visual transition order alive until
            // Terminated so it does not collapse to a one-frame animation.
            apply_bow_transition_state_side_effect(entity, current_order_type, motion);
            let actor = entity.actor_data_mut().unwrap();
            if matches!(
                motion,
                SpriteMotionState::Terminated | SpriteMotionState::Aborted
            ) {
                let remaining = if let Some(elem) =
                    sequence_manager.get_element_mut(shot_seq_id, shot_elem_idx)
                {
                    elem.orders.pop_front();
                    elem.orders.is_empty()
                } else {
                    true
                };
                if remaining {
                    actor.active_shot.clear();
                    events.completed.push((shot_seq_id, shot_elem_idx));
                }
            }
            continue;
        }

        let actor = entity.actor_data_mut().unwrap();
        if is_shoot_order(current_order_type) {
            if matches!(
                motion,
                SpriteMotionState::Terminated | SpriteMotionState::Aborted
            ) {
                let remaining = if let Some(elem) =
                    sequence_manager.get_element_mut(shot_seq_id, shot_elem_idx)
                {
                    elem.orders.pop_front();
                    elem.orders.is_empty()
                } else {
                    true
                };
                if remaining {
                    actor.active_shot.clear();
                    events.completed.push((shot_seq_id, shot_elem_idx));
                }
                continue;
            }

            if actor.active_shot.released {
                continue;
            }
            actor.active_shot.released = true;

            // Shoot action-done pulse — arrow is released, but the
            // animation continues until Terminated.
            let shot_action_state = actor.action_state;
            actor.action_state = ActionState::AimingWithBow;
            if current_order_type == OrderType::ShootingWithBowLeaningOut {
                entity.element_data_mut().posture = Posture::LeaningOut;
            } else if entity.element_data().posture != Posture::AnonymousArcher {
                entity.element_data_mut().posture = Posture::Upright;
            }

            // Compute sprite hand anchor point for accurate bow origin.
            // The sprite returns a relative hotspot offset; we add the
            // entity's map position to get absolute coordinates.
            // (immutable borrow — safe now that the mutable borrow above is done)
            let sprite_hand_point = {
                let shoot_anim =
                    bow_point_order_type_for_mode(shoot_mode_from_action_state(shot_action_state));
                let sprite_pos = entity.element_data().position_map();
                let hotspot = entity.element_data().sprite.get_point(shoot_anim, dir_u16);
                match hotspot {
                    Some(offset) => {
                        // Hotspot found — add sprite position to get absolute coords.
                        Some(crate::geo2d::Point2D {
                            x: sprite_pos.x + offset.x,
                            y: sprite_pos.y + offset.y,
                        })
                    }
                    None => {
                        tracing::warn!(
                            shooter = idx,
                            ?shoot_anim,
                            dir_u16,
                            "Bow release skipped: missing bow-point sprite hotspot"
                        );
                        continue;
                    }
                }
            };

            let Some(target) = shot.target else {
                tracing::warn!(
                    shooter = idx,
                    "Bow release skipped: active shot missing target"
                );
                continue;
            };

            events.fired.push(ShotTickResult {
                shooter: EntityId(idx as u32),
                target,
                seq_id: shot_seq_id,
                elem_idx: shot.element_index,
                shooter_position,
                target_pos: ElemPoint2D::default(),
                target_point: Point3D::default(),
                action_state: shot_action_state,
                shooter_direction: direction,
                sprite_hand_point,
                target_forecasted_movement: Point3D::default(),
            });
            continue;
        }

        // Unknown order type in the queue while we have an active shot.
        // Pop it to avoid stalling.
        if let Some(elem) = sequence_manager.get_element_mut(shot_seq_id, shot_elem_idx) {
            elem.orders.pop_front();
        }
    }

    // Resolve target positions, 3D body points and forecasted movement (immutable re-borrow).
    events.fired.retain_mut(|result| {
        let Some(Some(target_entity)) = entities.get(result.target.0 as usize) else {
            tracing::warn!(
                shooter = ?result.shooter,
                target = ?result.target,
                "Bow release skipped: target entity missing"
            );
            return false;
        };
        result.target_pos = target_entity.element_data().position_map();
        result.target_point = if target_entity.is_human() {
            let Some(point) = target_entity.compute_belt_point() else {
                tracing::warn!(
                    shooter = ?result.shooter,
                    target = ?result.target,
                    "Bow release skipped: human target missing belt hotspot"
                );
                return false;
            };
            point
        } else if target_entity.is_fx_target() {
            let Some(point) = target_entity.compute_target_center() else {
                tracing::warn!(
                    shooter = ?result.shooter,
                    target = ?result.target,
                    "Bow release skipped: FX target missing center hotspot"
                );
                return false;
            };
            point
        } else {
            tracing::warn!(
                shooter = ?result.shooter,
                target = ?result.target,
                kind = ?target_entity.kind(),
                "Bow release skipped: unsupported target kind"
            );
            return false;
        };
        result.target_forecasted_movement = target_entity
            .position_iface()
            .get_forecasted_movement()
            .into();
        true
    });

    events
}

// ═══════════════════════════════════════════════════════════════════
//  Arrow spawn
// ═══════════════════════════════════════════════════════════════════

/// Parameters for spawning an arrow projectile.
pub struct SpawnArrowParams {
    pub shooter: EntityId,
    pub bow_point: Point3D,
    /// C++ `ShootWithBowAt` stores the shooter map position as
    /// `mposStartOfTrajectory` after the arrow's first `Hourglass()`.
    /// AI reactions use this origin, not the bow hand hotspot.
    pub trajectory_origin: ElemPoint2D,
    pub target: EntityId,
    pub target_pos: ElemPoint2D,
    pub trajectory: Vec<TrajectoryPoint>,
    pub damage: u16,
    pub layer: u16,
    /// Initial 3D velocity — `compute_initial_throw_velocity` output
    /// (after any target-leading correction).  The sprite facing is
    /// seeded from the XY of this vector, not from `target - bow` —
    /// the two diverge once leading is applied to moving targets.
    ///
    pub initial_velocity: Point3D,
    /// Whether the precomputed trajectory ends inside a hole zone
    /// (before any far-edge fall-into-hole extension).  Pre-flags
    /// `ProjectileData::disappear` so `maybe_splash_on_landing` can
    /// route to the silent-disappear branch even if the extended final
    /// position tests outside the polygon due to boundary ray-cast
    /// tiebreaking.
    pub lands_in_hole: bool,
}

/// Build a new arrow projectile `Entity` for a fired shot.
///
/// Unlike the previous straight-line version, this takes a precomputed
/// ballistic trajectory and stores it on the projectile for per-frame
/// advancement in [`tick_arrows`].
pub fn spawn_arrow(params: SpawnArrowParams) -> Entity {
    let SpawnArrowParams {
        shooter,
        bow_point,
        trajectory_origin,
        target,
        target_pos,
        trajectory,
        damage,
        layer,
        lands_in_hole,
        initial_velocity,
    } = params;
    let map_pos = ElemPoint2D {
        x: bow_point.x,
        y: bow_point.y,
    };
    let end_pos = trajectory.last().map(|tp| tp.position).unwrap_or(Point3D {
        x: target_pos.x,
        y: target_pos.y,
        z: 0.0,
    });

    let mut element = ElementData {
        kind: ElementKind::ObjectProjectile,
        active: true,
        posture: Posture::Undefined,
        ..ElementData::default()
    };
    element.set_position_map(map_pos);
    element.set_position(bow_point);
    element.set_layer(layer);
    // Flight direction comes from the initial velocity's XY component
    // (after leading correction), not from the raw target-minus-bow
    // displacement.  Leading shifts the velocity vector off the
    // line-of-sight and occasionally lands on a different sector index.
    // Apply the isometric Y-stretch via `vector_to_sector_0_to_15_iso`.
    element.set_direction_instantly(crate::position_interface::vector_to_sector_0_to_15_iso(
        initial_velocity.x,
        initial_velocity.y,
    ));
    let mut object = ObjectData {
        associated_action: Action::Bow,
        object_type: ObjectType::Arrow,
        animation: Animation::ObjectFlying,
        quantity: 1,
        ..ObjectData::default()
    };
    object.reference = Some(target);

    let projectile = ProjectileData {
        start: bow_point,
        end: end_pos,
        start_of_trajectory_x: trajectory_origin.x,
        start_of_trajectory_y: trajectory_origin.y,
        shooter: Some(shooter),
        flying: true,
        disappear: lands_in_hole,
        trajectory,
        damage,
        ..ProjectileData::default()
    };

    let mut arrow = ElementProjectile {
        element,
        object,
        projectile,
    };
    arrow.advance_trajectory_one_frame();
    arrow.projectile.launch_segment_start = Some(bow_point);
    Entity::Projectile(arrow)
}

/// Spawn a net projectile entity flying toward `target_pos`.
///
/// Creates an `Entity::Net` with a precomputed ballistic trajectory
/// using `MASS_NET` / `APEX_NET`.
pub fn spawn_net(
    thrower: EntityId,
    throw_pos: Point3D,
    target_pos: Point3D,
    layer: u16,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
) -> Entity {
    let dx = target_pos.x - throw_pos.x;
    let dy = target_pos.y - throw_pos.y;
    let dz = target_pos.z - throw_pos.z;
    let direction_vec = Point3D {
        x: dx,
        y: dy,
        z: dz,
    };

    let velocity = compute_initial_throw_velocity(direction_vec, APEX_NET, MASS_NET, 0, None);
    // Nets bounce with `(0.1, 0.1)` — heavily damped, so the net skips
    // once and settles.
    let trajectory = compute_trajectory_ballistic_bounce(
        throw_pos,
        velocity,
        MASS_NET,
        false,
        obstacle_check,
        (0.1, 0.1),
    );
    let end_pos = trajectory
        .last()
        .map(|tp| tp.position)
        .unwrap_or(target_pos);

    let map_pos = ElemPoint2D {
        x: throw_pos.x,
        y: throw_pos.y,
    };
    let mut element = ElementData {
        kind: ElementKind::ObjectNet,
        active: true,
        posture: Posture::Undefined,
        ..ElementData::default()
    };
    element.set_position_map(map_pos);
    element.set_position(throw_pos);
    element.set_layer(layer);
    element.set_direction_instantly(crate::position_interface::vector_to_sector_0_to_15_iso(
        dx, dy,
    ));
    let object = ObjectData {
        associated_action: Action::Net,
        object_type: ObjectType::BonusNet,
        animation: Animation::ObjectFlying,
        quantity: 1,
        ..ObjectData::default()
    };

    // Sum the precomputed waypoint times for the net's frames-left
    // counter at spawn.  Time-till-unfolding is `frames_left - 15`,
    // clamped at a minimum of 1.
    let total_trajectory_frames: u32 = trajectory.iter().map(|p| p.time as u32).sum();
    let time_till_unfolding = (total_trajectory_frames as i32 - 15).max(1);

    let projectile = ProjectileData {
        start: throw_pos,
        end: end_pos,
        start_of_trajectory_x: throw_pos.x,
        start_of_trajectory_y: throw_pos.y,
        shooter: Some(thrower),
        frame_count: 0,
        flying: true,
        trajectory,
        damage: 0,
        ..ProjectileData::default()
    };

    let net = crate::element::NetData {
        crumpled: false,
        was_flying: true,
        time_till_unfolding,
        ..Default::default()
    };

    let mut net_entity = crate::element::ElementNet {
        element,
        object,
        projectile,
        net,
    };
    // Advance one trajectory step before handing the net to the engine
    // so it's already one step in when the engine picks it up.
    // `detect_initial_net_crumple` runs against `projectile.end` (the
    // trajectory's last waypoint), which this primer does not modify —
    // only the first waypoint is consumed.
    net_entity.advance_trajectory_one_frame();
    Entity::Net(net_entity)
}

/// Spawn a wasp nest projectile entity flying toward `target_pos`.
///
/// Creates an `Entity::Projectile` with a ballistic trajectory using
/// `MASS_WASP_NEST` / `APEX_WASP_NEST`.
pub fn spawn_wasp_nest(
    thrower: EntityId,
    throw_pos: Point3D,
    target_pos: Point3D,
    layer: u16,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
) -> Entity {
    let dx = target_pos.x - throw_pos.x;
    let dy = target_pos.y - throw_pos.y;
    let dz = target_pos.z - throw_pos.z;
    let direction_vec = Point3D {
        x: dx,
        y: dy,
        z: dz,
    };

    let velocity =
        compute_initial_throw_velocity(direction_vec, APEX_WASP_NEST, MASS_WASP_NEST, 0, None);
    // Wasp nests bounce with the coin bounce factors `(0.33, 0.3)`.
    let trajectory = compute_trajectory_ballistic_bounce(
        throw_pos,
        velocity,
        MASS_WASP_NEST,
        false,
        obstacle_check,
        (0.33, 0.3),
    );
    let end_pos = trajectory
        .last()
        .map(|tp| tp.position)
        .unwrap_or(target_pos);

    let map_pos = ElemPoint2D {
        x: throw_pos.x,
        y: throw_pos.y,
    };
    let mut element = ElementData {
        kind: ElementKind::ObjectProjectile,
        active: true,
        posture: Posture::Undefined,
        ..ElementData::default()
    };
    element.set_position_map(map_pos);
    element.set_position(throw_pos);
    element.set_layer(layer);
    element.set_direction_instantly(crate::position_interface::vector_to_sector_0_to_15_iso(
        dx, dy,
    ));
    let object = ObjectData {
        associated_action: Action::WaspNest,
        object_type: ObjectType::BonusWaspNest,
        animation: Animation::ObjectFlying,
        quantity: 1,
        ..ObjectData::default()
    };

    let projectile = ProjectileData {
        start: throw_pos,
        end: end_pos,
        start_of_trajectory_x: throw_pos.x,
        start_of_trajectory_y: throw_pos.y,
        shooter: Some(thrower),
        frame_count: 0,
        flying: true,
        trajectory,
        damage: 0,
        ..ProjectileData::default()
    };

    let mut wasp_nest = ElementProjectile {
        element,
        object,
        projectile,
    };
    // Advance one trajectory step before handing the wasp nest to the
    // engine so it's already one step in when it joins the active
    // element list.
    wasp_nest.advance_trajectory_one_frame();
    Entity::Projectile(wasp_nest)
}

/// Number of wasps a wasp nest bursts into on impact.
pub const NUMBER_OF_WASPS: u16 = 20;

/// Spawn a wasp at `position`, attached to `nest_id`.
///
/// Copies the nest's position into the wasp and queues the
/// `BonusOne` animation.  Per-frame AI (direction change / victim
/// choice / sting) lives in `EngineInner::tick_wasp_nests`.
pub fn spawn_wasp(nest_id: EntityId, position: Point3D, layer: u16) -> Entity {
    let mut element = ElementData {
        kind: ElementKind::ObjectProjectile,
        active: true,
        posture: Posture::Undefined,
        ..ElementData::default()
    };
    element.set_position_map(ElemPoint2D {
        x: position.x,
        y: position.y - position.z,
    });
    element.set_position(position);
    element.set_layer(layer);

    let object = ObjectData {
        associated_action: Action::NoAction,
        object_type: ObjectType::Wasp,
        animation: Animation::BonusOne,
        quantity: 1,
        ..ObjectData::default()
    };

    let mut projectile = ProjectileData {
        start: position,
        end: position,
        start_of_trajectory_x: position.x,
        start_of_trajectory_y: position.y,
        shooter: None,
        frame_count: 0,
        // Inert projectile flag: wasps don't consume ballistic
        // trajectories (they fly under AI control in
        // `EngineInner::tick_wasp_nests`).
        flying: false,
        damage: 0,
        ..ProjectileData::default()
    };
    projectile.wasp.source_nest = Some(nest_id);

    Entity::Projectile(ElementProjectile {
        element,
        object,
        projectile,
    })
}

/// Spawn an apple projectile flying toward `target_pos`.
///
/// Creates an `Entity::Projectile` with a ballistic trajectory using
/// `MASS_APPLE` / `APEX_APPLE`.
///
/// `target_forecasted_movement`: when the victim is an NPC, callers
/// should look up `PositionInterface::get_forecasted_movement()` on
/// the NPC so the shot leads the target's current motion; pass `None`
/// for FX / static targets.
#[allow(clippy::too_many_arguments)]
pub fn spawn_apple(
    thrower: EntityId,
    throw_pos: Point3D,
    target_pos: Point3D,
    target: Option<EntityId>,
    target_forecasted_movement: Option<Point3D>,
    layer: u16,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
) -> Entity {
    spawn_throwable(
        thrower,
        throw_pos,
        target_pos,
        target,
        target_forecasted_movement,
        layer,
        MASS_APPLE,
        APEX_APPLE,
        0,
        Action::Apple,
        ObjectType::Apple,
        obstacle_check,
    )
}

/// Spawn a stone projectile flying toward `target_pos`.
///
/// Creates an `Entity::Projectile` with a fast near-flat ballistic
/// trajectory.  Unlike the other throwables, stones use `flight_time = 1`
/// in `compute_initial_throw_velocity`, which skips the apex-driven
/// branch and sets `velocity = 0.5 * direction` directly — so
/// `APEX_STONE` is effectively unused, but `MASS_STONE` still drives
/// the gravity applied during trajectory integration.
///
/// `target_forecasted_movement`: see `spawn_apple` for how callers
/// supply this.
#[allow(clippy::too_many_arguments)]
pub fn spawn_stone(
    thrower: EntityId,
    throw_pos: Point3D,
    target_pos: Point3D,
    target: Option<EntityId>,
    target_forecasted_movement: Option<Point3D>,
    layer: u16,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
) -> Entity {
    spawn_throwable(
        thrower,
        throw_pos,
        target_pos,
        target,
        target_forecasted_movement,
        layer,
        MASS_STONE,
        APEX_STONE,
        1,
        Action::Stone,
        ObjectType::Stone,
        obstacle_check,
    )
}

/// Shared spawn path for non-bouncing small throwables (apple, stone).
/// Bounce-on-landing projectiles (net, purse, wasp nest) use the
/// dedicated bounce-trajectory path.
///
/// `flight_time` is forwarded to `compute_initial_throw_velocity`.
/// Apple passes `0` (compute from apex), stone passes `1` (fast flat
/// throw, apex unused).
#[allow(clippy::too_many_arguments)]
fn spawn_throwable(
    thrower: EntityId,
    throw_pos: Point3D,
    target_pos: Point3D,
    target: Option<EntityId>,
    target_forecasted_movement: Option<Point3D>,
    layer: u16,
    mass: f32,
    apex: f32,
    flight_time: u16,
    action: Action,
    object_type: ObjectType,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
) -> Entity {
    let dx = target_pos.x - throw_pos.x;
    let dy = target_pos.y - throw_pos.y;
    let dz = target_pos.z - throw_pos.z;
    let direction_vec = Point3D {
        x: dx,
        y: dy,
        z: dz,
    };

    let velocity = compute_initial_throw_velocity(
        direction_vec,
        apex,
        mass,
        flight_time,
        target_forecasted_movement,
    );
    let trajectory = compute_trajectory_ballistic(throw_pos, velocity, mass, false, obstacle_check);
    let end_pos = trajectory
        .last()
        .map(|tp| tp.position)
        .unwrap_or(target_pos);

    let map_pos = ElemPoint2D {
        x: throw_pos.x,
        y: throw_pos.y,
    };
    let mut element = ElementData {
        kind: ElementKind::ObjectProjectile,
        active: true,
        posture: Posture::Undefined,
        ..ElementData::default()
    };
    element.set_position_map(map_pos);
    element.set_position(throw_pos);
    element.set_layer(layer);
    element.set_direction_instantly(crate::position_interface::vector_to_sector_0_to_15_iso(
        dx, dy,
    ));
    let object = ObjectData {
        associated_action: action,
        object_type,
        animation: Animation::ObjectFlying,
        quantity: 1,
        reference: target,
        ..ObjectData::default()
    };

    let projectile = ProjectileData {
        start: throw_pos,
        end: end_pos,
        start_of_trajectory_x: throw_pos.x,
        start_of_trajectory_y: throw_pos.y,
        shooter: Some(thrower),
        flying: true,
        trajectory,
        damage: 0,
        ..ProjectileData::default()
    };

    let mut throwable = ElementProjectile {
        element,
        object,
        projectile,
    };
    // Advance one trajectory step before handing the projectile to
    // the engine so it's already one step in when it joins the active
    // element list.  Without this, the projectile would wait an extra
    // frame.
    throwable.advance_trajectory_one_frame();
    Entity::Projectile(throwable)
}

// ═══════════════════════════════════════════════════════════════════
//  Purse / coin spawn
// ═══════════════════════════════════════════════════════════════════

/// Number of coins ejected on impact.  Aliased to
/// `crate::inventory::COINS_PER_PURSE` so the burst routine reads with
/// the same name as the projectile-settings constant.
pub const NUMBER_OF_COINS_IN_PURSE: u16 = crate::inventory::COINS_PER_PURSE;

/// Mass for a single coin's ballistic ejection (same as arrow-flat /
/// stone — 0.1).
pub const MASS_COIN: f32 = 0.1;

/// Coin bounce factors `(vertical, horizontal)`.
pub const BOUNCE_COIN: (f32, f32) = (0.33, 0.3);

/// Maximum random horizontal scatter for a coin's landing point, in
/// map units.  The goal vector is `unit_sector * (10 + rand() & 31)` —
/// a `[10..=41]` random magnitude before multiplying by the unit
/// sector vector.
pub const COIN_SCATTER_MIN: f32 = 10.0;
pub const COIN_SCATTER_RANGE: f32 = 32.0;

/// Apex height for a tossed coin.  The coin scatter trajectory uses
/// the small fixed apex of 3.
pub const APEX_COIN: f32 = 3.0;

/// Apex used by civilians tossing a coin to a PC-beggar — a gentler
/// arc than the purse-burst scatter.
pub const APEX_BEGGAR_COIN: f32 = 1.0;

/// Number of attempts the scatter loop makes when picking each coin's
/// landing point.
pub const COIN_SCATTER_ATTEMPTS: u32 = 7;

/// Spawn a thrown-purse projectile.
///
/// Creates an `Entity::Projectile` with `ObjectType::Purse` whose
/// ballistic trajectory uses `MASS_PURSE` / `APEX_PURSE`.  When the
/// trajectory finishes, the purse-handling tick
/// (`EngineInner::tick_purses_and_coins`) detects the impact and calls
/// into the burst routine to eject coins.
pub fn spawn_purse(
    thrower: EntityId,
    throw_pos: Point3D,
    target_pos: Point3D,
    layer: u16,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
) -> Entity {
    let dx = target_pos.x - throw_pos.x;
    let dy = target_pos.y - throw_pos.y;
    let dz = target_pos.z - throw_pos.z;
    let direction_vec = Point3D {
        x: dx,
        y: dy,
        z: dz,
    };

    let velocity = compute_initial_throw_velocity(direction_vec, APEX_PURSE, MASS_PURSE, 0, None);
    let trajectory =
        compute_trajectory_ballistic(throw_pos, velocity, MASS_PURSE, false, obstacle_check);
    let end_pos = trajectory
        .last()
        .map(|tp| tp.position)
        .unwrap_or(target_pos);

    let map_pos = ElemPoint2D {
        x: throw_pos.x,
        y: throw_pos.y,
    };
    let mut element = ElementData {
        kind: ElementKind::ObjectProjectile,
        active: true,
        posture: Posture::Undefined,
        ..ElementData::default()
    };
    element.set_position_map(map_pos);
    element.set_position(throw_pos);
    element.set_layer(layer);
    element.set_direction_instantly(crate::position_interface::vector_to_sector_0_to_15_iso(
        dx, dy,
    ));
    let object = ObjectData {
        associated_action: Action::Purse,
        object_type: ObjectType::Purse,
        animation: Animation::ObjectFlying,
        // The per-purse value for inventory accounting is one purse,
        // not the coin count.
        quantity: 1,
        ..ObjectData::default()
    };

    let mut projectile = ProjectileData {
        start: throw_pos,
        end: end_pos,
        start_of_trajectory_x: throw_pos.x,
        start_of_trajectory_y: throw_pos.y,
        shooter: Some(thrower),
        frame_count: 0,
        flying: true,
        trajectory,
        damage: 0,
        ..ProjectileData::default()
    };
    // Populate the purse's coin count from the bonus master during
    // creation; the impact handler later asserts
    // `>= NUMBER_OF_COINS_IN_PURSE` and decrements.
    projectile.purse.number_of_coins = NUMBER_OF_COINS_IN_PURSE;

    let mut purse = ElementProjectile {
        element,
        object,
        projectile,
    };
    // Advance one trajectory step before handing the purse to the
    // engine so it's already one step in when the engine picks it up.
    purse.advance_trajectory_one_frame();
    Entity::Projectile(purse)
}

/// Spawn one coin projectile.
///
/// Two call sites share this:
///
/// * Purse-burst coins — `source_purse` is `Some(purse_id)` and `apex`
///   is [`APEX_COIN`].
/// * Civilian-tossed coins (give-money-to-beggar) — `source_purse` is
///   `None` and `apex` is [`APEX_BEGGAR_COIN`].
///
/// `target_pos` is the landing point; the trajectory uses the
/// damped-bounce parameters from `BOUNCE_COIN`.  The goal layer/sector
/// are stored on the projectile so the coin can snap to them on
/// landing — see [`PurseData::layer_goal`] and
/// [`PurseData::sector_goal`].
#[allow(clippy::too_many_arguments)]
pub fn spawn_coin(
    source_purse: Option<EntityId>,
    source_pos: Point3D,
    target_pos: Point3D,
    layer: u16,
    layer_goal: u16,
    sector_goal: Option<crate::position_interface::SectorHandle>,
    apex: f32,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
) -> Entity {
    let dx = target_pos.x - source_pos.x;
    let dy = target_pos.y - source_pos.y;
    let dz = target_pos.z - source_pos.z;
    let direction_vec = Point3D {
        x: dx,
        y: dy,
        z: dz,
    };

    let velocity = compute_initial_throw_velocity(direction_vec, apex, MASS_COIN, 0, None);
    let trajectory = compute_trajectory_ballistic_bounce(
        source_pos,
        velocity,
        MASS_COIN,
        false,
        obstacle_check,
        BOUNCE_COIN,
    );
    let end_pos = trajectory
        .last()
        .map(|tp| tp.position)
        .unwrap_or(target_pos);

    let map_pos = ElemPoint2D {
        x: source_pos.x,
        y: source_pos.y,
    };
    let mut element = ElementData {
        kind: ElementKind::ObjectProjectile,
        active: true,
        posture: Posture::Undefined,
        ..ElementData::default()
    };
    element.set_position_map(map_pos);
    element.set_position(source_pos);
    element.set_layer(layer);
    element.set_direction_instantly(crate::position_interface::vector_to_sector_0_to_15_iso(
        dx, dy,
    ));
    let object = ObjectData {
        associated_action: Action::NoAction,
        object_type: ObjectType::Coin,
        animation: Animation::ObjectFlying,
        quantity: 1,
        ..ObjectData::default()
    };

    let mut projectile = ProjectileData {
        start: source_pos,
        end: end_pos,
        start_of_trajectory_x: source_pos.x,
        start_of_trajectory_y: source_pos.y,
        // Burst-spawned coins carry no shooter; their owner identity
        // flows through `source_purse` instead.  Beggar coins have
        // neither a shooter nor a source purse.
        shooter: None,
        frame_count: 0,
        flying: true,
        trajectory,
        damage: 0,
        ..ProjectileData::default()
    };
    projectile.purse.source_purse = source_purse;
    projectile.purse.layer_goal = layer_goal;
    projectile.purse.sector_goal = sector_goal;

    let mut coin = ElementProjectile {
        element,
        object,
        projectile,
    };
    // Advance one trajectory step before handing the coin to the
    // engine so it's already one step in when it joins the active
    // element list.  Without this, fresh coins visually pop on frame 0.
    coin.advance_trajectory_one_frame();
    Entity::Projectile(coin)
}

// ═══════════════════════════════════════════════════════════════════
//  Per-frame arrow tick
// ═══════════════════════════════════════════════════════════════════

/// Outcome of an arrow tick — the engine applies damage and despawn
/// decisions after the mutable-borrow loop releases.
pub struct ArrowTickResult {
    pub arrow: EntityId,
    pub hit_target: Option<EntityId>,
    /// Entity whose shield blocked the arrow (mutually exclusive with
    /// `hit_target`).  When set, the engine should trigger a parry-shield
    /// animation instead of applying damage.
    pub shield_hit: Option<EntityId>,
    /// FX-target the projectile connected with (mutually exclusive
    /// with `hit_target`/`shield_hit`), paired with the activation
    /// command to dispatch.  Different projectile types launch
    /// different activation commands: arrows → `Command::ActivateArrow`,
    /// apples → `Command::ActivateApple`, stones →
    /// `Command::ActivateStone`.
    pub fx_target_hit: Option<(EntityId, Command)>,
    pub despawn: bool,
    /// Damage to apply if there's a hit.  Precomputed at spawn time
    /// from the shooter's bow profile.
    pub damage: u16,
    /// Impact sound to play at [`Self::impact_pos`] on this tick.  Set
    /// on the tick a projectile first stops flying; the engine routes
    /// it through `pending_side_effects.sounds`.  Per-type FX ids:
    /// arrow 510, apple 509, stone 508.
    pub impact_fx: Option<u32>,
    /// Map-space position of the projectile at impact, for locating
    /// the impact FX sound.  Only meaningful when `impact_fx.is_some()`.
    pub impact_pos: ElemPoint2D,
}

fn current_arrow_orientation_sector(proj: &ElementProjectile) -> i16 {
    let vx = proj.projectile.velocity_increment.x;
    let vy = proj.projectile.velocity_increment.y;
    if vx != 0.0 || vy != 0.0 {
        crate::position_interface::vector_to_sector_0_to_15_iso(vx, vy)
    } else {
        proj.element.direction()
    }
}

fn apply_arrow_falling_sprite_visual(proj: &mut ElementProjectile) {
    // C++ `RHElementArrow::Refresh` renders falling arrows with
    // `ForceSpriteRow(mubFallingDirection)`,
    // `ForceSprite(mubFallingDirection, (rand() % 3) + 3)`, then rotates
    // the row by -2 sectors for the next refresh.
    let row = proj.projectile.falling_direction;
    let frame = (crate::sim_rng::u32(0..3) as u16) + 3;
    proj.element.sprite.force_sprite_row_raw(row);
    proj.element.sprite.force_sprite(row, frame);
    proj.projectile.falling_direction = (row + 14) % 16;
}

pub(crate) fn make_arrow_falling_down(proj: &mut ElementProjectile, thrown_away_by_shield: bool) {
    let sector = current_arrow_orientation_sector(proj);
    proj.projectile.falling = true;
    proj.projectile.flying = true;

    let (falling_direction, velocity) = if thrown_away_by_shield {
        let direction = (sector + 4) & 15;
        let (dx, dy) = crate::element::direction_vector_16(direction);
        (
            direction as u16,
            Point3D {
                x: dx * 30.0,
                y: dy * ASPECT_RATIO * 30.0,
                z: -20.0,
            },
        )
    } else {
        let direction = sector ^ 8;
        let (dx, dy) = crate::element::direction_vector_16(direction);
        (
            direction as u16,
            Point3D {
                x: dx * 30.0,
                y: dy * ASPECT_RATIO * 10.0,
                z: 0.0,
            },
        )
    };

    proj.projectile.falling_direction = falling_direction;
    proj.projectile.trajectory = compute_trajectory_ballistic(
        proj.element.position(),
        velocity,
        MASS_ARROW_HIGH,
        false,
        None,
    );
    proj.projectile.trajectory_frame_count = 0;
    proj.projectile.launch_segment_start = None;

    // C++ `RHElementArrow::MakeFallingDown` calls `Hourglass()` after
    // recomputing the trajectory, so the ricochet visibly advances on
    // the same tick as the shield/target impact.
    proj.advance_trajectory_one_frame();
    apply_arrow_falling_sprite_visual(proj);
}

/// Advance every arrow projectile by one frame along its precomputed
/// ballistic trajectory.
///
/// Pops waypoints from the trajectory list, interpolates position
/// between them, and checks for victim proximity each frame.
///
/// When the arrow comes within [`HIT_DISTANCE`] of any living human,
/// or the trajectory runs out, the arrow is flagged for despawn and
/// the engine applies damage.
pub fn tick_arrows(
    entities: &mut [Option<Entity>],
    sight_obstacles: crate::sight_obstacle::ObstacleList<'_>,
) -> Vec<ArrowTickResult> {
    let mut results = Vec::new();

    // Snapshot living humans for line-segment hit detection.  Computes
    // the perpendicular distance from the target's 3D belt (or eyes
    // for stones) to the arrow's movement line, and filters by posture.
    //
    // Excluded postures: `Lying`, `Carried`, `Dead`, `DeadBack`,
    // `StuckUnderNet`, `Tied`, `Tree` — targets in these states are
    // un-hittable (on the ground, restrained, or camouflaged).
    // `LeaningOut` falls through to the default branch but gets a
    // second belt→eyes pass for arrows.
    struct HumanSnapshot {
        id: EntityId,
        /// Belt point (arrows + apples) and eyes point (stones) in 3D.
        /// Pre-computed so the per-projectile loop stays cheap.
        belt: Point3D,
        eyes: Point3D,
        /// True when posture == LeaningOut — arrows get an eye-level
        /// re-check after the belt miss.
        leaning_out: bool,
        is_pc: bool,
        is_soldier: bool,
        is_civilian: bool,
        camp: Option<crate::element::Camp>,
        holding_shield: bool,
    }
    let human_snapshots: Vec<HumanSnapshot> = entities
        .iter()
        .enumerate()
        .filter_map(|(idx, slot)| {
            let e = slot.as_ref()?;
            if !e.is_human() || !e.is_active() {
                return None;
            }
            // Posture filter: skip targets that can't be hit by
            // arrows (lying on ground, carried, dead, netted, tied,
            // hiding in a tree).
            let posture = e.element_data().posture;
            use crate::element::Posture::*;
            if matches!(
                posture,
                Lying | Carried | Dead | DeadBack | StuckUnderNet | Tied | Tree
            ) {
                return None;
            }
            let Some(belt) = e.compute_belt_point() else {
                tracing::warn!(
                    entity = idx,
                    "Projectile hit snapshot skipped: human missing belt hotspot"
                );
                return None;
            };
            let Some(eyes) = e.compute_eyes_point(None) else {
                tracing::warn!(
                    entity = idx,
                    "Projectile hit snapshot skipped: human missing eyes hotspot"
                );
                return None;
            };
            let camp = match e {
                Entity::Pc(_) => Some(crate::element::Camp::Royalists),
                Entity::Soldier(s) => Some(s.soldier.cached_camp),
                Entity::Civilian(c) => Some(c.civilian.cached_camp),
                _ => None,
            };
            let holding_shield = e
                .actor_data()
                .map(|a| a.action_state.is_shield())
                .unwrap_or(false);
            Some(HumanSnapshot {
                id: EntityId(idx as u32),
                belt,
                eyes,
                leaning_out: posture == crate::element::Posture::LeaningOut,
                is_pc: e.is_pc(),
                is_soldier: e.is_soldier(),
                is_civilian: e.is_civilian(),
                camp,
                holding_shield,
            })
        })
        .collect();

    // Snapshot FX targets that can be activated by a passing
    // projectile.  Each projectile type checks a specific filter bit
    // and launches a dedicated activation command — the per-projectile
    // loop below matches the projectile's `ObjectType` against the
    // target's filter bits.  The hit test uses the target's 3D center
    // and the perpendicular distance to the arrow's movement line.
    struct FxTargetSnapshot {
        id: EntityId,
        center: Point3D,
        action_filter: crate::element::TargetFilter,
    }
    let fx_target_snapshots: Vec<FxTargetSnapshot> = entities
        .iter()
        .enumerate()
        .filter_map(|(idx, slot)| {
            let e = slot.as_ref()?;
            if !e.kind().is_fx_target() || !e.is_active() {
                return None;
            }
            let Entity::Target(t) = e else { return None };
            let filter = t.target.action_filter;
            // Projectile-activation filters only — keeps the per-tick
            // inner loop small.
            if !filter.intersects(
                crate::element::TargetFilter::ARROW
                    | crate::element::TargetFilter::APPLE
                    | crate::element::TargetFilter::STONE,
            ) {
                return None;
            }
            let Some(center) = e.compute_target_center() else {
                tracing::warn!(
                    entity = idx,
                    "Projectile hit snapshot skipped: FX target missing center hotspot"
                );
                return None;
            };
            Some(FxTargetSnapshot {
                id: EntityId(idx as u32),
                center,
                action_filter: filter,
            })
        })
        .collect();

    // Snapshot shield holders for arrow-shield intersection — iterates
    // all actors holding a shield and checks their shield obstacle
    // geometry against each projectile's path.
    struct ShieldSnapshot {
        holder_id: EntityId,
        /// Look direction with Y un-compressed by inverse aspect ratio,
        /// for the dot-product "arrow from front" check.
        look_dir: (f32, f32),
        obstacle: crate::sight_obstacle::SightObstacle,
    }
    let shield_snapshots: Vec<ShieldSnapshot> = entities
        .iter()
        .enumerate()
        .filter_map(|(idx, slot)| {
            let e = slot.as_ref()?;
            if !e.is_human() || !e.is_active() || e.is_dead() {
                return None;
            }
            let actor = e.actor_data()?;
            if !actor.action_state.is_shield() {
                return None;
            }
            let obstacle = actor.shield_obstacle.as_ref()?.clone();
            let (dx, dy) = crate::element::direction_vector_16(e.element_data().direction());
            // Un-compress Y for angular comparison.
            let look_dir = (dx, dy * INVERSE_ASPECT_RATIO);
            Some(ShieldSnapshot {
                holder_id: EntityId(idx as u32),
                look_dir,
                obstacle,
            })
        })
        .collect();

    for (idx, slot) in entities.iter_mut().enumerate() {
        let entity = match slot {
            Some(e) => e,
            None => continue,
        };
        let is_arrow = matches!(entity, Entity::Projectile(_)) && entity.element_data().active;
        if !is_arrow {
            continue;
        }
        let proj = match entity {
            Entity::Projectile(p) => p,
            _ => continue,
        };
        let arrow_id = EntityId(idx as u32);
        // `Entity::Projectile` is shared by arrows, apples, stones,
        // purses, coins, nets, wasp nests, and wasps.  Purses, coins,
        // wasp nests, and wasps follow their own per-tick update paths
        // (`EngineInner::tick_purses_and_coins`, `EngineInner::tick_wasp_nests`)
        // — skip them here so the proximity / shield / FX-target paths
        // below don't misfire.
        if matches!(
            proj.object.object_type,
            ObjectType::Purse
                | ObjectType::Coin
                | ObjectType::WaspNest
                | ObjectType::BonusWaspNest
                | ObjectType::Wasp
        ) {
            continue;
        }

        let is_burster = matches!(
            proj.object.object_type,
            ObjectType::Apple | ObjectType::Stone
        );

        // ── Post-impact burst decay (apple/stone only) ─────────────
        // Once a burster stops flying, advance the `ObjectBursting`
        // sprite and deactivate when its animation finishes.  Modeled
        // with a frame counter set on impact; when it hits zero, we
        // despawn.
        if !proj.projectile.flying {
            if proj.projectile.burst_countdown > 0 {
                proj.projectile.burst_countdown -= 1;
                if proj.projectile.burst_countdown == 0 {
                    results.push(ArrowTickResult {
                        arrow: arrow_id,
                        hit_target: None,
                        shield_hit: None,
                        fx_target_hit: None,
                        despawn: true,
                        damage: 0,
                        impact_fx: None,
                        impact_pos: proj.element.position_map(),
                    });
                }
            }
            continue;
        }

        // Distinct impact FX ids per projectile type.  Arrows play
        // their 510 only on shield deflection (which has its own
        // path), so non-shield arrow impacts stay silent.
        let impact_fx = match proj.object.object_type {
            ObjectType::Apple => Some(509u32),
            ObjectType::Stone => Some(508u32),
            _ => None,
        };

        let _target_id = proj.object.reference;
        let damage = proj.projectile.damage;
        let shooter_id = proj.projectile.shooter;
        let primed_segment_start = proj.projectile.launch_segment_start.take();

        // ── Trajectory advancement ────────────────────────────────

        if primed_segment_start.is_some() {
            // Spawn already advanced this first segment to match C++
            // `pArrow->Hourglass()` before engine insertion.  Still run
            // collision below against `primed_segment_start -> current`.
        } else if proj.projectile.trajectory_frame_count == 0 {
            if !proj.projectile.trajectory.is_empty() {
                // Pop the next trajectory waypoint.
                let point = proj.projectile.trajectory.remove(0);
                let time = point.time.max(1);
                proj.projectile.trajectory_frame_count = time - 1;

                // Compute per-frame increment toward this waypoint.
                let current = proj.element.position();
                let factor = 1.0 / time as f32;
                proj.projectile.velocity_increment = Point3D {
                    x: (point.position.x - current.x) * factor,
                    y: (point.position.y - current.y) * factor,
                    z: (point.position.z - current.z) * factor,
                };

                // Update end position.
                proj.projectile.end = point.position;
            } else {
                // Trajectory exhausted — projectile lands / impacts
                // terrain.  Apples and stones force the burst
                // animation and keep the sprite alive for a few
                // frames; arrows just despawn.
                //
                // Elevation snap on landing.  Branch on the obstacle
                // at the landing point:
                //   * No obstacle → snap elevation to 0.001 (absolute
                //     ground), unconditionally.
                //   * Obstacle present → snap elevation to
                //     `top_plane_z + 0.001`, gated on
                //     `layer != 0xFFFF && object_type != Arrow`
                //     (arrows stuck in walls and unassigned-layer
                //     projectiles keep their trajectory-end elevation).
                //
                // Active in-flight projectiles don't carry a cached
                // obstacle reference, so re-derive the obstacle here
                // by scanning active projection-area obstacles whose
                // screen polygon contains the landing point — the same
                // lookup `FastFindGrid::resolve_projectile_landing`
                // runs after tick to populate the cached obstacle.
                let pos = proj.element.position();
                let landing_screen = crate::geo2d::pt(pos.x, pos.y - pos.z);
                let mut top_plane_z: Option<f32> = None;
                for (obs_idx, obstacle) in sight_obstacles.iter_indexed() {
                    if !sight_obstacles.is_active(obs_idx as usize)
                        || !obstacle.is_projection_area()
                        || obstacle.layer == u16::MAX
                        || !obstacle.contains_point_screen(landing_screen)
                    {
                        continue;
                    }
                    let plane = crate::position_interface::PlaneZCoeffs::from_plane_points(
                        &obstacle.top_plane_points,
                    );
                    top_plane_z = Some(plane.compute_z(pos.x, pos.y));
                    break;
                }

                let new_z = match top_plane_z {
                    None => Some(0.001),
                    Some(z) => {
                        if !matches!(proj.object.object_type, ObjectType::Arrow)
                            && proj.element.layer() != 0xFFFF
                        {
                            Some(z + 0.001)
                        } else {
                            None
                        }
                    }
                };
                if let Some(z) = new_z {
                    let mut p = proj.element.position();
                    p.z = z;
                    proj.element.set_position(p);
                    let mut m = proj.element.position_map();
                    m.y = p.y - p.z;
                    proj.element.set_position_map_preserving_3d(m);
                }
                let impact_pos = proj.element.position_map();
                proj.projectile.flying = false;
                let despawn = if is_burster {
                    proj.object.animation = Animation::ObjectBursting;
                    proj.projectile.burst_countdown = burst_ticks_for_proj(proj);
                    false
                } else {
                    true
                };
                results.push(ArrowTickResult {
                    arrow: arrow_id,
                    hit_target: None,
                    shield_hit: None,
                    fx_target_hit: None,
                    despawn,
                    damage,
                    impact_fx,
                    impact_pos,
                });
                continue;
            }
        } else {
            proj.projectile.trajectory_frame_count -= 1;
        }

        // Apply the per-frame increment to position.
        let mut p = proj.element.position();
        p.x += proj.projectile.velocity_increment.x;
        p.y += proj.projectile.velocity_increment.y;
        p.z += proj.projectile.velocity_increment.z;
        proj.element.set_position(p);

        // Update the 2D map position from 3D (project Z onto Y for
        // isometric display: map.y = pos.y - pos.z).
        proj.element.set_position_map_preserving_3d(ElemPoint2D {
            x: proj.element.position().x,
            y: proj.element.position().y - proj.element.position().z,
        });
        // Update facing direction from velocity increment.  Use the
        // isometric Y-stretch so the flight-direction sector agrees
        // with the one set at spawn.
        let vx = proj.projectile.velocity_increment.x;
        let vy = proj.projectile.velocity_increment.y;
        let vz = proj.projectile.velocity_increment.z;
        if vx != 0.0 || vy != 0.0 {
            proj.element.set_direction_instantly(
                crate::position_interface::vector_to_sector_0_to_15_iso(vx, vy),
            );
        }

        // Cache the sector + vertical-pitch azimut into `last_sector`
        // / `last_azimut` whenever the trajectory is non-empty so
        // later callers with an empty trajectory can still read the
        // last known orientation.  Computed from the per-frame
        // velocity increment.
        //
        // After the per-frame position + sector update, push row +
        // frame into the sprite so the arrow renders with the
        // directional sector row and the vertical-pitch frame.
        if matches!(proj.object.object_type, ObjectType::Arrow) {
            if proj.projectile.falling {
                apply_arrow_falling_sprite_visual(proj);
            } else if vx != 0.0 || vy != 0.0 {
                let norm_sq = vx * vx + vy * vy + vz * vz;
                if norm_sq > 0.0 {
                    let inv_norm = 1.0 / norm_sq.sqrt();
                    let nx = vx * inv_norm;
                    let ny = vy * inv_norm;
                    let nz = vz * inv_norm;
                    // Ground-projection norm = sqrt(nx² + ny²) ≤ 1.
                    let ground_norm = (nx * nx + ny * ny).sqrt().min(1.0);
                    // `acos(ground_norm) * 180/PI`, clamped ≤ 60 then
                    // signed by the sign of Z.
                    let mut azimut_deg =
                        (ground_norm.acos() * 180.0 / std::f32::consts::PI).min(60.0) as i16;
                    if nz < 0.0 {
                        azimut_deg = -azimut_deg;
                    }
                    let sector = proj.element.direction() as u16 & 15;
                    // row = sector, frame = `(azimut * 0.0666…) + 0.5`
                    // rounded + 4 (nine vertical-pitch frames centred
                    // on 4 for horizontal flight).
                    let azimut_frame =
                        ((azimut_deg as f32 * 0.066_666_67_f32 + 0.5_f32) as i32 + 4) as u16;
                    proj.element.sprite.force_sprite_row_raw(sector);
                    proj.element.sprite.force_sprite(sector, azimut_frame);
                }
            }
        }

        // Increment lifetime counter for diagnostics/replay state. C++
        // projectile lifetime is governed by trajectory exhaustion and
        // impact side effects; it has no hard timeout.
        proj.projectile.frame_count = proj.projectile.frame_count.saturating_add(1);

        // ── Falling arrows skip all collision checks ──────────────
        // Once an arrow is deflected (by a shield or target), it
        // tumbles to the ground without hitting anything.  It
        // continues advancing along its deflected trajectory until it
        // runs out (handled by the trajectory advancement code above).
        if proj.projectile.falling {
            continue;
        }

        let arrow_new = proj.element.position();
        let arrow_old = primed_segment_start.unwrap_or(Point3D {
            x: arrow_new.x - proj.projectile.velocity_increment.x,
            y: arrow_new.y - proj.projectile.velocity_increment.y,
            z: arrow_new.z - proj.projectile.velocity_increment.z,
        });

        // ── Shield intersection check ─────────────────────────────
        // Before checking victim proximity, check if any shield blocks
        // the projectile path.  This check runs for **every**
        // projectile type, but only arrows are deflected into the
        // falling state on a shield hit.  Apples and stones keep
        // flying along their existing trajectory; the caller plays
        // the per-type impact FX and launches a `ParryShield` on the
        // holder, then this frame terminates early — the apple/stone
        // carries on its trajectory next tick.
        let arrow_map = proj.element.position_map();
        let vx = proj.projectile.velocity_increment.x;
        let vy = proj.projectile.velocity_increment.y;

        let mut shield_blocker = None;
        if !proj.projectile.trajectory.is_empty() || proj.projectile.trajectory_frame_count > 0 {
            let old_pos = [arrow_old.x, arrow_old.y - arrow_old.z, arrow_old.z];
            let new_pos = [arrow_new.x, arrow_map.y, arrow_new.z];

            // Flight direction with Y un-compressed.
            let flight_dir = (vx, vy * INVERSE_ASPECT_RATIO);

            for shield in &shield_snapshots {
                if Some(shield.holder_id) == shooter_id {
                    continue;
                }
                // (a) Arrow from front: dot(look_dir, flight_dir) < 0.
                let dot = shield.look_dir.0 * flight_dir.0 + shield.look_dir.1 * flight_dir.1;
                if dot >= 0.0 {
                    continue;
                }
                // (b) Arrow path intersects shield geometry.
                if shield.obstacle.is_blocking_ray_3d(new_pos, old_pos) {
                    shield_blocker = Some(shield.holder_id);
                    break;
                }
            }
        }

        if let Some(holder) = shield_blocker {
            // Arrow path: deflect 90° right, set falling=true,
            // recompute trajectory.
            //
            // Apple/Stone path: keep flying along the existing
            // trajectory.  The per-type FX (509/508) plays at the
            // shield holder's position.  This frame terminates early
            // (no human / FX-target check), so `continue` after
            // reporting.
            if matches!(proj.object.object_type, ObjectType::Arrow) {
                make_arrow_falling_down(proj, true);

                results.push(ArrowTickResult {
                    arrow: arrow_id,
                    hit_target: None,
                    shield_hit: Some(holder),
                    fx_target_hit: None,
                    despawn: false, // Don't despawn — arrow falls to ground.
                    damage,
                    // Silent — see note above.
                    impact_fx: None,
                    impact_pos: proj.element.position_map(),
                });
            } else {
                // Apple / stone: keep flying on current trajectory; the
                // engine plays the per-type FX at the holder's position
                // and launches ParryShield.  `impact_pos` is the
                // projectile's current position — the engine caller
                // replaces it with the holder's position using
                // `shield_hit` as the anchor.
                results.push(ArrowTickResult {
                    arrow: arrow_id,
                    hit_target: None,
                    shield_hit: Some(holder),
                    fx_target_hit: None,
                    despawn: false,
                    damage,
                    impact_fx,
                    impact_pos: proj.element.position_map(),
                });
            }
            continue;
        }

        // ── Victim / FX-target hit detection ──────────────────────
        // For each living human / FX target, compute the perpendicular
        // distance from the target's 3D anchor point to the arrow's
        // movement line (old_pos → new_pos).  A target is hit when
        //   (a) perpendicular distance ≤ HIT_DISTANCE, and
        //   (b) the segment is long enough to reach it from old_pos
        //       (so a slow-moving arrow doesn't "teleport" onto a
        //       distant target that happens to be near its final line).
        // This catches fast arrows that would otherwise tunnel past a
        // target between frames, which the old 2D point check missed.
        /// Perpendicular distance from `p` to the line through `a→b`,
        /// computed via `||ap × ab|| / ||ab||`.  Returns `f32::MAX`
        /// when the segment has zero length (caller should fall back
        /// to a point-to-point distance).
        fn point_to_line_distance(p: Point3D, a: Point3D, b: Point3D) -> f32 {
            let abx = b.x - a.x;
            let aby = b.y - a.y;
            let abz = b.z - a.z;
            let ab_len_sq = abx * abx + aby * aby + abz * abz;
            if ab_len_sq < 1e-6 {
                return f32::MAX;
            }
            let apx = p.x - a.x;
            let apy = p.y - a.y;
            let apz = p.z - a.z;
            let cx = apy * abz - apz * aby;
            let cy = apz * abx - apx * abz;
            let cz = apx * aby - apy * abx;
            let cross_sq = cx * cx + cy * cy + cz * cz;
            (cross_sq / ab_len_sq).sqrt()
        }
        fn distance(a: Point3D, b: Point3D) -> f32 {
            let dx = b.x - a.x;
            let dy = b.y - a.y;
            let dz = b.z - a.z;
            (dx * dx + dy * dy + dz * dz).sqrt()
        }
        fn projectile_victim_prefilter_allows(
            shooter: Option<&HumanSnapshot>,
            victim: &HumanSnapshot,
        ) -> bool {
            let Some(shooter) = shooter else {
                return true;
            };
            let same_camp = matches!(
                (shooter.camp, victim.camp),
                (Some(shooter_camp), Some(victim_camp)) if shooter_camp == victim_camp
            );

            // C++ filters these candidates inside `FindHumanVictim`
            // before geometric hit selection:
            //   - soldier projectiles do not hit civilians
            //   - soldier projectiles do not hit same-camp humans
            //   - PC projectiles do not hit shield-holding PCs
            // The separate forest GoodSoldier rule is covered by the
            // same-camp soldier branch for Royalist soldiers.
            !(shooter.is_soldier && (victim.is_civilian || same_camp)
                || shooter.is_pc && victim.is_pc && victim.holding_shield)
        }

        // Segment length (range of this frame's movement).
        let range = distance(arrow_old, arrow_new);

        // Pick the aim anchor by projectile type — arrows and apples
        // aim for the belt, stones aim for the eyes.
        let uses_eyes_anchor = matches!(proj.object.object_type, ObjectType::Stone);

        // C++ `FindHumanVictim` returns no human victim when the
        // projectile is not moving.  FX target handling is separate in
        // `FindTargetVictim` and keeps the point fallback below.
        let use_range_gate = range > 0.1;

        let mut hit_victim = None;
        let shooter_snapshot =
            shooter_id.and_then(|id| human_snapshots.iter().find(|snap| snap.id == id));
        // C++ `RHElementProjectile::FindHumanVictim` returns null when
        // `mpShooter == NULL`; only FX target collision still runs.
        if let Some(shooter_snapshot) = shooter_snapshot {
            for snap in &human_snapshots {
                if Some(snap.id) == shooter_id {
                    continue;
                }
                if !projectile_victim_prefilter_allows(Some(shooter_snapshot), snap) {
                    continue;
                }
                let anchor = if uses_eyes_anchor {
                    snap.eyes
                } else {
                    snap.belt
                };
                let hit = if use_range_gate {
                    // Range gate: the old_pos→target distance must be
                    // within this frame's reach (segment length).
                    let old_to_target = distance(arrow_old, anchor);
                    old_to_target <= range
                        && point_to_line_distance(anchor, arrow_old, arrow_new) <= HIT_DISTANCE
                } else {
                    false
                };
                if hit {
                    hit_victim = Some(snap.id);
                    break;
                }

                // Leaning-out re-check for arrows only.  If the arrow
                // missed the belt by less than 100 (max-component
                // distance), try again at the eye level — catches arrows
                // that sail just above the belt of a soldier leaning out
                // of a battlement.
                if snap.leaning_out
                    && matches!(proj.object.object_type, ObjectType::Arrow)
                    && use_range_gate
                {
                    // Use max-component distance ≤ 100 as the coarse gate
                    // for the re-check.
                    let ddx = (anchor.x - arrow_new.x).abs();
                    let ddy = (anchor.y - arrow_new.y).abs();
                    let ddz = (anchor.z - arrow_new.z).abs();
                    let max_norm = ddx.max(ddy).max(ddz);
                    if max_norm <= 100.0 {
                        let eyes = snap.eyes;
                        // C++ uses the arrow's current position for the
                        // leaning-out eye retry, unlike the primary human
                        // belt check's "deguillaumized" old-position gate.
                        let new_to_eyes = distance(arrow_new, eyes);
                        if new_to_eyes <= range
                            && point_to_line_distance(eyes, arrow_old, arrow_new) <= HIT_DISTANCE
                        {
                            hit_victim = Some(snap.id);
                            break;
                        }
                    }
                }
            }
        }

        // FX-target segment/point check (same semantics).  C++
        // `FindTargetVictim` only returns FX targets whose action
        // filter matches the projectile type, so nonmatching targets
        // are ignored before `HitTarget` can run.
        let (required_filter, activation_command) = match proj.object.object_type {
            ObjectType::Arrow => (crate::element::TargetFilter::ARROW, Command::ActivateArrow),
            ObjectType::Apple => (crate::element::TargetFilter::APPLE, Command::ActivateApple),
            ObjectType::Stone => (crate::element::TargetFilter::STONE, Command::ActivateStone),
            _ => (
                crate::element::TargetFilter::empty(),
                Command::ActivateArrow,
            ),
        };
        let mut fx_target_hit: Option<(EntityId, Command)> = None;
        if !required_filter.is_empty() {
            for snap in &fx_target_snapshots {
                if !snap.action_filter.contains(required_filter) {
                    continue;
                }
                let hit = if use_range_gate {
                    // C++ `FindTargetVictim` gates FX targets by
                    // distance from the projectile's current position
                    // to target center, not by old position.  This
                    // catches the scripted-target case where the
                    // current frame reaches the center exactly.
                    let new_to_target = distance(arrow_new, snap.center);
                    new_to_target <= range
                        && point_to_line_distance(snap.center, arrow_old, arrow_new) <= HIT_DISTANCE
                } else {
                    // With no movement C++ still evaluates
                    // `vtRange.Norm() <= range`, so only an exact center
                    // overlap can activate the target. Do not use the
                    // normal hit-radius fallback here.
                    distance(arrow_new, snap.center) <= 0.01
                };
                if hit {
                    fx_target_hit = Some((snap.id, activation_command));
                    break;
                }
            }
        }

        if let Some(victim) = hit_victim {
            let impact_pos = proj.element.position_map();
            proj.projectile.flying = false;
            let despawn = if is_burster {
                proj.object.animation = Animation::ObjectBursting;
                proj.projectile.burst_countdown = burst_ticks_for_proj(proj);
                false
            } else {
                true
            };
            results.push(ArrowTickResult {
                arrow: arrow_id,
                hit_target: Some(victim),
                shield_hit: None,
                fx_target_hit: None,
                despawn,
                damage,
                impact_fx,
                impact_pos,
            });
        } else if let Some(fx_hit) = fx_target_hit {
            let impact_pos = proj.element.position_map();
            proj.projectile.flying = false;
            let despawn = if is_burster {
                proj.object.animation = Animation::ObjectBursting;
                proj.projectile.burst_countdown = burst_ticks_for_proj(proj);
                false
            } else {
                true
            };
            results.push(ArrowTickResult {
                arrow: arrow_id,
                hit_target: None,
                shield_hit: None,
                fx_target_hit: Some(fx_hit),
                despawn,
                damage,
                impact_fx,
                impact_pos,
            });
        }
    }

    results
}

// ═══════════════════════════════════════════════════════════════════
//  Hit application
// ═══════════════════════════════════════════════════════════════════

/// Apply an arrow impact to the target human.
///
/// Returns `true` if the victim died from the hit.
pub fn apply_arrow_hit(
    entities: &mut [Option<Entity>],
    victim_id: EntityId,
    shooter_id: EntityId,
    damage: u16,
    arrow_flight_direction: i16,
) -> bool {
    // Arrows pass `concussion = damage` — the arrow damage element
    // uses a single value for both fields.
    apply_projectile_hit(
        entities,
        victim_id,
        shooter_id,
        damage,
        damage,
        arrow_flight_direction,
    )
}

/// Apply a generic projectile hit (piercing damage + concussion) to a
/// human.  Factored from [`apply_arrow_hit`] so stones can pass a
/// distinct concussion (e.g. damage=10, concussion=100 for stones —
/// much higher KO potential than arrows).
pub fn apply_projectile_hit(
    entities: &mut [Option<Entity>],
    victim_id: EntityId,
    shooter_id: EntityId,
    damage: u16,
    concussion: u16,
    arrow_flight_direction: i16,
) -> bool {
    // Resolve shooter PC-ness before the victim mutable borrow.
    // The flag is read only when the victim transitions to
    // unconscious; missing slot ⇒ false.
    let shooter_is_pc = entities
        .get(shooter_id.0 as usize)
        .and_then(|s| s.as_ref())
        .map(|e| e.is_pc())
        .unwrap_or(false);

    let victim = match entities
        .get_mut(victim_id.0 as usize)
        .and_then(|s| s.as_mut())
    {
        Some(e) => e,
        None => return false,
    };

    // Snap the victim to face the arrow's opposite direction (toward
    // the shooter) when struck.
    victim
        .element_data_mut()
        .set_direction_instantly(arrow_flight_direction ^ 8);

    let ctx = ConcussionContext {
        is_invulnerable: victim.is_immortal(),
        ..ConcussionContext::default()
    };
    // Read actual max HP from the entity.
    let max_hp: i16 = match &*victim {
        Entity::Pc(_) => 100,
        Entity::Soldier(s) => {
            use crate::element::Human;
            Human::max_life_points(s)
        }
        Entity::Civilian(_) => 100,
        _ => 100,
    };

    // Snapshot the pre-hit unconscious state so we can detect the KO
    // transition triggered by the concussion add and forward the
    // shooter attribution into `inform_my_friends`.
    let was_unconscious = victim.human_data().map(|h| h.unconscious).unwrap_or(false);

    let died = match victim {
        Entity::Pc(pc) => combat::receive_piercing_damage(
            &mut pc.human,
            &mut pc.pc.life_points,
            damage,
            concussion,
            max_hp,
            &ctx,
        ),
        Entity::Soldier(s) => combat::receive_piercing_damage(
            &mut s.human,
            &mut s.npc.life_points,
            damage,
            concussion,
            max_hp,
            &ctx,
        ),
        Entity::Civilian(c) => combat::receive_piercing_damage(
            &mut c.human,
            &mut c.npc.life_points,
            damage,
            concussion,
            max_hp,
            &ctx,
        ),
        _ => return false,
    };

    // Detect a fresh KO transition (was conscious, now unconscious).
    // Set `inform_my_friends` only on this transition; the flag is
    // consumed on the next tick by `tick_inform_my_friends`, which
    // broadcasts the body to nearby NPCs.  Without this, a stone-KO'd
    // soldier would not be detected by his friends, breaking witness
    // wiring for PC-thrown stones.
    let now_unconscious = victim.human_data().map(|h| h.unconscious).unwrap_or(false);
    if !was_unconscious
        && now_unconscious
        && let Some(npc) = victim.npc_data_mut()
    {
        npc.inform_my_friends = shooter_is_pc;
    }

    if died {
        victim.set_posture(Posture::Dead);
    }
    died
}

// ═══════════════════════════════════════════════════════════════════
//  Helper — launching the sequence element
// ═══════════════════════════════════════════════════════════════════

/// Build a `Command::ShootBow` sequence element on the given shooter,
/// targeting the given entity. The caller is expected to launch it via
/// `EngineInner::launch_element` so the priority is resolved eagerly.
pub fn build_shoot_bow_element(shooter: EntityId, target: EntityId) -> SequenceElement {
    let mut element = SequenceElement::new(1, Command::ShootBow, Some(shooter));
    element.data = SequenceElementData::Interaction {
        antagonist: Some(target),
    };
    element
}

// ═══════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{
        ActorData, ElementKind, ElementTarget, FxData, HumanData, TargetData, TargetFilter,
    };
    use crate::element::{ActorPc, ActorSoldier, NpcData, PcData, SoldierData};
    use crate::geo2d::{Point2D as GeoPoint2D, Vec2D};
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    fn make_pc(x: f32, y: f32) -> Entity {
        let mut element = ElementData {
            kind: ElementKind::ActorPc,
            active: true,
            ..ElementData::default()
        };
        element.set_position_map(ElemPoint2D { x, y });
        Entity::Pc(ActorPc {
            element,
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        })
    }

    fn make_anonymous_pc(x: f32, y: f32) -> Entity {
        let mut pc = make_pc(x, y);
        pc.element_data_mut().posture = Posture::AnonymousArcher;
        pc
    }

    fn make_soldier(x: f32, y: f32) -> Entity {
        make_soldier_with_camp(x, y, crate::element::Camp::Royalists)
    }

    fn make_soldier_with_camp(x: f32, y: f32, camp: crate::element::Camp) -> Entity {
        let mut element = ElementData {
            kind: ElementKind::ActorSoldier,
            active: true,
            ..ElementData::default()
        };
        element.set_position_map(ElemPoint2D { x, y });
        let npc = NpcData {
            life_points: 100,
            ..Default::default()
        };
        Entity::Soldier(ActorSoldier {
            element,
            actor: ActorData::default(),
            human: HumanData::default(),
            npc,
            soldier: SoldierData {
                cached_camp: camp,
                ..SoldierData::default()
            },
        })
    }

    fn make_arrow_target(x: f32, y: f32) -> Entity {
        let mut element = ElementData {
            kind: ElementKind::Target,
            active: true,
            ..ElementData::default()
        };
        element.set_position_map(ElemPoint2D { x, y });
        element.set_position(Point3D { x, y, z: 0.0 });
        Entity::Target(ElementTarget {
            element,
            fx: FxData::default(),
            target: TargetData {
                action_filter: TargetFilter::ARROW,
                ..TargetData::default()
            },
        })
    }

    /// Test helper — launch a `ShootBow` sequence element and return
    /// `(sequence_manager, seq_id, elem_idx)` so tests can hand the
    /// triple to `begin_bow_shot` / `tick_bow_shots`.
    fn launch_test_shoot_element(
        shooter: EntityId,
        target: EntityId,
    ) -> (SequenceManager, SequenceId, usize) {
        let mut sm = SequenceManager::new();
        let elem = build_shoot_bow_element(shooter, target);
        let seq_id = sm.launch_element(elem);
        // Transition the element to InProgress so `current_element_for_actor`
        // finds it — the engine does this as part of the hourglass dispatch,
        // which the tests skip.
        sm.element_in_progress(seq_id, 0);
        (sm, seq_id, 0)
    }

    fn bind_test_bow_release_rows(entity: &mut Entity, order_type: OrderType) {
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        let base_row = 0u16;
        conversion[order_type as usize] = base_row;

        let mut scripts = Vec::with_capacity(16);
        for _direction in 0..16 {
            scripts.push(SpriteScript {
                action_id: order_type as u16,
                action_done: 1,
                average_speed: 0.0,
                hotspot: GeoPoint2D { x: 2.0, y: 3.0 },
                sum_distance: 0,
                frame_ids: vec![1, 2, 3],
                delays: vec![0, 0, 0],
                distances: vec![0, 0, 0],
                offsets: vec![Vec2D { x: 0.0, y: 0.0 }; 3],
                sound_ids: vec![0, 0, 0],
            });
        }

        let sprite = &mut entity.element_data_mut().sprite;
        sprite.scripts = std::sync::Arc::new(scripts);
        sprite.conversion = std::sync::Arc::new(conversion);
    }

    #[test]
    fn begin_bow_shot_sets_shooter_state() {
        let mut entities: Vec<Option<Entity>> =
            vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))];
        let (mut sm, seq_id, elem_idx) = launch_test_shoot_element(EntityId(0), EntityId(1));
        let result = begin_bow_shot(
            &mut entities,
            &mut sm,
            EntityId(0),
            EntityId(1),
            seq_id,
            elem_idx,
            false,
            10,
            None,
            &mut 1u32,
        );
        assert_eq!(result, BeginShotResult::Started);

        let actor = entities[0].as_ref().unwrap().actor_data().unwrap();
        assert_eq!(actor.action_state, ActionState::AimingWithBow);
        assert!(actor.active_shot.is_active());
        assert_eq!(actor.active_shot.target, Some(EntityId(1)));
        // Should have: shoot order + reload order (and possibly transition orders)
        assert!(sm.get_element(seq_id, elem_idx).unwrap().orders.len() >= 2);
    }

    #[test]
    fn begin_bow_shot_rejects_dead_target() {
        let mut entities: Vec<Option<Entity>> =
            vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))];
        if let Some(Some(Entity::Soldier(s))) = entities.get_mut(1) {
            s.npc.life_points = 0; // dead
        }
        let (mut sm, seq_id, elem_idx) = launch_test_shoot_element(EntityId(0), EntityId(1));
        let result = begin_bow_shot(
            &mut entities,
            &mut sm,
            EntityId(0),
            EntityId(1),
            seq_id,
            elem_idx,
            false,
            10,
            None,
            &mut 1u32,
        );
        assert_eq!(result, BeginShotResult::Impossible);
    }

    #[test]
    fn begin_bow_shot_accepts_arrow_fx_target() {
        let mut entities: Vec<Option<Entity>> =
            vec![Some(make_pc(0.0, 0.0)), Some(make_arrow_target(50.0, 0.0))];
        let (mut sm, seq_id, elem_idx) = launch_test_shoot_element(EntityId(0), EntityId(1));
        let result = begin_bow_shot(
            &mut entities,
            &mut sm,
            EntityId(0),
            EntityId(1),
            seq_id,
            elem_idx,
            false,
            10,
            None,
            &mut 1u32,
        );
        assert_eq!(result, BeginShotResult::Started);
        assert_eq!(
            entities[0]
                .as_ref()
                .unwrap()
                .actor_data()
                .unwrap()
                .active_shot
                .target,
            Some(EntityId(1))
        );
    }

    #[test]
    fn begin_bow_shot_uses_anonymous_shoot_orders() {
        let mut entities: Vec<Option<Entity>> = vec![
            Some(make_anonymous_pc(0.0, 0.0)),
            Some(make_soldier(50.0, 0.0)),
        ];
        let (mut sm, seq_id, elem_idx) = launch_test_shoot_element(EntityId(0), EntityId(1));

        let result = begin_bow_shot(
            &mut entities,
            &mut sm,
            EntityId(0),
            EntityId(1),
            seq_id,
            elem_idx,
            false,
            10,
            Some(ShootMode::Normal),
            &mut 1u32,
        );

        assert_eq!(result, BeginShotResult::Started);
        let orders: Vec<OrderType> = sm
            .get_element(seq_id, elem_idx)
            .unwrap()
            .orders
            .iter()
            .map(|o| o.order_type)
            .collect();
        assert_eq!(orders[0], OrderType::ShootingWithBowAnonymous);
        assert_eq!(orders[1], OrderType::TransitionLoadingBowAnonymous);
    }

    #[test]
    fn begin_bow_shot_faces_arrow_fx_target_ground_projection() {
        let mut target = make_arrow_target(50.0, 120.0);
        target.element_data_mut().set_position(Point3D {
            x: 50.0,
            y: 120.0,
            z: 100.0,
        });
        let mut entities: Vec<Option<Entity>> = vec![Some(make_pc(0.0, 100.0)), Some(target)];
        let (mut sm, seq_id, elem_idx) = launch_test_shoot_element(EntityId(0), EntityId(1));

        let result = begin_bow_shot(
            &mut entities,
            &mut sm,
            EntityId(0),
            EntityId(1),
            seq_id,
            elem_idx,
            false,
            10,
            None,
            &mut 1u32,
        );

        assert_eq!(result, BeginShotResult::Started);
        let direction_goal = i16::from(
            entities[0]
                .as_ref()
                .unwrap()
                .element_data()
                .sprite
                .position_iface
                .get_direction_goal(),
        );
        assert_eq!(
            direction_goal,
            crate::position_interface::vector_to_sector_0_to_15_iso(50.0, -80.0)
        );
        assert_ne!(
            direction_goal,
            crate::position_interface::vector_to_sector_0_to_15_iso(50.0, 20.0)
        );
    }

    #[test]
    fn tick_bow_shots_fires_arrow_and_returns_to_aiming() {
        let mut entities: Vec<Option<Entity>> =
            vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))];
        bind_test_bow_release_rows(entities[0].as_mut().unwrap(), OrderType::ShootingWithBow);
        let (mut sm, seq_id, elem_idx) = launch_test_shoot_element(EntityId(0), EntityId(1));

        begin_bow_shot(
            &mut entities,
            &mut sm,
            EntityId(0),
            EntityId(1),
            seq_id,
            elem_idx,
            false,
            10,
            None,
            &mut 1u32,
        );

        // Tick through the facing freeze, then the shoot row's action-done pulse.
        let mut fired = Vec::new();
        let mut completed = Vec::new();
        for _ in 0..24 {
            let events = tick_bow_shots(&mut entities, &mut sm);
            fired.extend(events.fired);
            completed.extend(events.completed);
            if !fired.is_empty() {
                break;
            }
        }
        assert_eq!(fired.len(), 1, "expected one fired shot");
        assert!(
            completed.is_empty(),
            "release should not terminate the sequence before the visual orders finish"
        );
        let r = &fired[0];
        assert_eq!(r.shooter, EntityId(0));
        assert_eq!(r.target, EntityId(1));
        assert_eq!(r.target_pos.x, 50.0);

        // Shooter should now be in AimingWithBow (sustained aim).
        let actor = entities[0].as_ref().unwrap().actor_data().unwrap();
        assert_eq!(actor.action_state, ActionState::AimingWithBow);
        assert!(actor.active_shot.is_active());
        assert!(actor.active_shot.released);
    }

    #[test]
    fn compute_initial_throw_velocity_flat_shot() {
        let to_target = Point3D {
            x: 100.0,
            y: 0.0,
            z: 0.0,
        };
        // Flat shot: flight_time = (0.003 * 100) + 1 = 1
        let vel = compute_initial_throw_velocity(to_target, 0.001, MASS_ARROW_FLAT, 1, None);
        // With flight_time == 1: velocity = 0.5 * to_target
        assert!((vel.x - 50.0).abs() < 0.01);
    }

    #[test]
    fn compute_initial_throw_velocity_high_shot() {
        let to_target = Point3D {
            x: 100.0,
            y: 0.0,
            z: 0.0,
        };
        let apex = 10.0; // distance / 10
        let vel = compute_initial_throw_velocity(to_target, apex, MASS_ARROW_HIGH, 0, None);
        // Should have a positive Z component (upward arc).
        assert!(vel.z > 0.0, "high shot should arc upward, got z={}", vel.z);
        // X should be positive (toward target).
        assert!(vel.x > 0.0);
    }

    #[test]
    fn compute_trajectory_produces_arc() {
        let start = Point3D {
            x: 0.0,
            y: 0.0,
            z: 40.0,
        };
        let vel = compute_initial_throw_velocity(
            Point3D {
                x: 100.0,
                y: 0.0,
                z: -10.0,
            },
            10.0,
            MASS_ARROW_HIGH,
            0,
            None,
        );
        let traj = compute_trajectory_ballistic(start, vel, MASS_ARROW_HIGH, false, None);
        assert!(!traj.is_empty(), "trajectory should have waypoints");
        // All points should have time == TIME_FLYSEGMENT.
        for pt in &traj {
            assert_eq!(pt.time, TIME_FLYSEGMENT);
        }
        // First point should be ahead of start in X.
        assert!(traj[0].position.x > start.x);
    }

    #[test]
    fn spawn_arrow_creates_flying_projectile_with_trajectory() {
        let traj = vec![
            TrajectoryPoint {
                position: Point3D {
                    x: 25.0,
                    y: 0.0,
                    z: 45.0,
                },
                time: 4,
            },
            TrajectoryPoint {
                position: Point3D {
                    x: 50.0,
                    y: 0.0,
                    z: 40.0,
                },
                time: 4,
            },
        ];
        let arrow = spawn_arrow(SpawnArrowParams {
            shooter: EntityId(0),
            bow_point: Point3D {
                x: 0.0,
                y: 0.0,
                z: 40.0,
            },
            trajectory_origin: ElemPoint2D { x: 0.0, y: 0.0 },
            target: EntityId(1),
            target_pos: ElemPoint2D { x: 50.0, y: 0.0 },
            trajectory: traj,
            damage: 30,
            layer: 0,
            lands_in_hole: false,
            initial_velocity: Point3D {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        });
        match arrow {
            Entity::Projectile(p) => {
                assert!(p.projectile.flying);
                assert_eq!(p.projectile.trajectory.len(), 1);
                assert_eq!(p.projectile.launch_segment_start.map(|p| p.x), Some(0.0));
                assert_eq!(p.projectile.damage, 30);
                assert_eq!(p.object.object_type, ObjectType::Arrow);
            }
            _ => panic!("expected ElementProjectile"),
        }
    }

    #[test]
    fn spawn_arrow_stores_shooter_map_position_as_trajectory_origin() {
        let arrow = spawn_arrow(SpawnArrowParams {
            shooter: EntityId(0),
            bow_point: Point3D {
                x: 100.0,
                y: 40.0,
                z: 40.0,
            },
            trajectory_origin: ElemPoint2D { x: 100.0, y: 0.0 },
            target: EntityId(1),
            target_pos: ElemPoint2D { x: 50.0, y: 0.0 },
            trajectory: vec![TrajectoryPoint {
                position: Point3D {
                    x: 50.0,
                    y: 40.0,
                    z: 40.0,
                },
                time: 2,
            }],
            damage: 30,
            layer: 0,
            lands_in_hole: false,
            initial_velocity: Point3D {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        });

        let Entity::Projectile(p) = arrow else {
            panic!("spawn_arrow should create projectile");
        };
        assert_eq!(p.projectile.start_of_trajectory_x, 100.0);
        assert_eq!(p.projectile.start_of_trajectory_y, 0.0);
    }

    #[test]
    fn tick_arrows_follows_trajectory_and_hits() {
        // Place a soldier at (50, 0) (belt lives at Z=25, the
        // default belt elevation for an upright human).  The
        // trajectory arcs from the bow height down to belt height at
        // the soldier's XY — the per-segment 3D hit check picks the
        // soldier up on the final waypoint.
        let traj = vec![
            TrajectoryPoint {
                position: Point3D {
                    x: 20.0,
                    y: 0.0,
                    z: 35.0,
                },
                time: 2,
            },
            TrajectoryPoint {
                position: Point3D {
                    x: 40.0,
                    y: 0.0,
                    z: 30.0,
                },
                time: 2,
            },
            TrajectoryPoint {
                position: Point3D {
                    x: 50.0,
                    y: 0.0,
                    z: 25.0,
                },
                time: 2,
            },
        ];
        let mut entities: Vec<Option<Entity>> = vec![
            Some(make_pc(0.0, 0.0)),
            Some(make_soldier(50.0, 0.0)),
            Some(spawn_arrow(SpawnArrowParams {
                shooter: EntityId(0),
                bow_point: Point3D {
                    x: 0.0,
                    y: 0.0,
                    z: 40.0,
                },
                trajectory_origin: ElemPoint2D { x: 0.0, y: 0.0 },
                target: EntityId(1),
                target_pos: ElemPoint2D { x: 50.0, y: 0.0 },
                trajectory: traj,
                damage: 30,
                layer: 0,
                lands_in_hole: false,
                initial_velocity: Point3D {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
            })),
        ];

        let mut hit = None;
        for _ in 0..20 {
            let results = tick_arrows(&mut entities, crate::sight_obstacle::ObstacleList::empty());
            for r in &results {
                if r.hit_target.is_some() {
                    hit = r.hit_target;
                    assert_eq!(r.damage, 30);
                    break;
                }
            }
            if hit.is_some() {
                break;
            }
        }
        assert_eq!(hit, Some(EntityId(1)), "arrow should reach target");
    }

    #[test]
    fn tick_arrows_prefilters_friendly_candidate_before_selecting_victim() {
        let arrow = spawn_arrow(SpawnArrowParams {
            shooter: EntityId(0),
            bow_point: Point3D {
                x: 0.0,
                y: 0.0,
                z: 25.0,
            },
            trajectory_origin: ElemPoint2D { x: 0.0, y: 0.0 },
            target: EntityId(2),
            target_pos: ElemPoint2D { x: 50.0, y: 0.0 },
            trajectory: vec![TrajectoryPoint {
                position: Point3D {
                    x: 50.0,
                    y: 0.0,
                    z: 25.0,
                },
                time: 1,
            }],
            damage: 30,
            layer: 0,
            lands_in_hole: false,
            initial_velocity: Point3D {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        });
        let mut entities: Vec<Option<Entity>> = vec![
            Some(make_soldier_with_camp(
                0.0,
                0.0,
                crate::element::Camp::Royalists,
            )),
            Some(make_soldier_with_camp(
                20.0,
                0.0,
                crate::element::Camp::Royalists,
            )),
            Some(make_soldier_with_camp(
                40.0,
                0.0,
                crate::element::Camp::Lacklandists,
            )),
            Some(arrow),
        ];

        let results = tick_arrows(&mut entities, crate::sight_obstacle::ObstacleList::empty());
        assert!(
            results.iter().all(|r| r.hit_target != Some(EntityId(1))),
            "same-camp soldier must be filtered before hit selection"
        );
        assert!(
            results.iter().any(|r| r.hit_target == Some(EntityId(2))),
            "arrow should continue to the valid victim behind the filtered candidate"
        );
    }

    #[test]
    fn tick_arrows_stationary_projectile_does_not_hit_human() {
        let mut arrow_element = ElementData {
            kind: ElementKind::ObjectProjectile,
            active: true,
            ..ElementData::default()
        };
        arrow_element.set_position_map(ElemPoint2D { x: 50.0, y: -25.0 });
        arrow_element.set_position(Point3D {
            x: 50.0,
            y: 0.0,
            z: 25.0,
        });
        let arrow = Entity::Projectile(ElementProjectile {
            element: arrow_element,
            object: ObjectData {
                associated_action: Action::Bow,
                object_type: ObjectType::Arrow,
                animation: Animation::ObjectFlying,
                quantity: 1,
                reference: Some(EntityId(1)),
                ..ObjectData::default()
            },
            projectile: ProjectileData {
                shooter: Some(EntityId(0)),
                flying: true,
                trajectory: vec![TrajectoryPoint {
                    position: Point3D {
                        x: 50.0,
                        y: 0.0,
                        z: 25.0,
                    },
                    time: 1,
                }],
                damage: 30,
                ..ProjectileData::default()
            },
        });
        let mut entities: Vec<Option<Entity>> = vec![
            Some(make_pc(0.0, 0.0)),
            Some(make_soldier_with_camp(
                50.0,
                0.0,
                crate::element::Camp::Lacklandists,
            )),
            Some(arrow),
        ];

        let results = tick_arrows(&mut entities, crate::sight_obstacle::ObstacleList::empty());
        assert!(
            results.iter().all(|r| r.hit_target.is_none()),
            "C++ FindHumanVictim returns no victim when projectile is not moving"
        );
    }

    #[test]
    fn tick_arrows_without_shooter_does_not_hit_human() {
        let mut arrow = spawn_arrow(SpawnArrowParams {
            shooter: EntityId(0),
            bow_point: Point3D {
                x: 0.0,
                y: 0.0,
                z: 40.0,
            },
            trajectory_origin: ElemPoint2D { x: 0.0, y: 0.0 },
            target: EntityId(1),
            target_pos: ElemPoint2D { x: 50.0, y: 0.0 },
            trajectory: vec![TrajectoryPoint {
                position: Point3D {
                    x: 50.0,
                    y: 0.0,
                    z: 25.0,
                },
                time: 1,
            }],
            damage: 30,
            layer: 0,
            lands_in_hole: false,
            initial_velocity: Point3D {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        });
        if let Entity::Projectile(proj) = &mut arrow {
            proj.projectile.shooter = None;
        }
        let mut entities: Vec<Option<Entity>> =
            vec![None, Some(make_soldier(50.0, 0.0)), Some(arrow)];

        let results = tick_arrows(&mut entities, crate::sight_obstacle::ObstacleList::empty());
        assert!(
            results.iter().all(|r| r.hit_target.is_none()),
            "C++ FindHumanVictim returns no victim when projectile has no shooter"
        );
    }

    /// Apple projectile flying through an APPLE-filtered FX target
    /// yields a `Command::ActivateApple` activation on tick.
    #[test]
    fn tick_arrows_apple_projectile_activates_apple_target() {
        use crate::element::{ElementKind, ElementTarget, FxData, TargetData, TargetFilter};

        let target_pos = ElemPoint2D { x: 50.0, y: 0.0 };
        let mut target_element = ElementData {
            kind: ElementKind::Target,
            active: true,
            ..ElementData::default()
        };
        target_element.set_position_map(target_pos);
        // `compute_target_center` reads the 3D position; real loaded
        // targets set both, but `ElementData::default()` leaves position
        // at origin so we mirror position_map.
        target_element.set_position(Point3D {
            x: target_pos.x,
            y: target_pos.y,
            z: 0.0,
        });
        let target = Entity::Target(ElementTarget {
            element: target_element,
            fx: FxData::default(),
            target: TargetData {
                action_filter: TargetFilter::APPLE,
                ..TargetData::default()
            },
        });

        let trajectory = vec![
            TrajectoryPoint {
                position: Point3D {
                    x: 25.0,
                    y: 0.0,
                    z: 10.0,
                },
                time: 2,
            },
            TrajectoryPoint {
                position: Point3D {
                    x: 50.0,
                    y: 0.0,
                    z: 0.0,
                },
                time: 2,
            },
        ];
        let mut apple_element = ElementData {
            kind: ElementKind::ObjectProjectile,
            active: true,
            ..ElementData::default()
        };
        apple_element.set_position_map(ElemPoint2D { x: 0.0, y: 0.0 });
        apple_element.set_position(Point3D {
            x: 0.0,
            y: 0.0,
            z: 20.0,
        });
        let apple = Entity::Projectile(ElementProjectile {
            element: apple_element,
            object: ObjectData {
                associated_action: Action::Apple,
                object_type: ObjectType::Apple,
                animation: Animation::ObjectFlying,
                quantity: 1,
                reference: Some(EntityId(0)),
                ..ObjectData::default()
            },
            projectile: ProjectileData {
                shooter: Some(EntityId(2)),
                flying: true,
                trajectory,
                ..ProjectileData::default()
            },
        });

        let mut entities: Vec<Option<Entity>> =
            vec![Some(target), Some(apple), Some(make_pc(0.0, 0.0))];

        let mut activation = None;
        for _ in 0..20 {
            for r in tick_arrows(&mut entities, crate::sight_obstacle::ObstacleList::empty()) {
                if let Some(hit) = r.fx_target_hit {
                    activation = Some(hit);
                    break;
                }
            }
            if activation.is_some() {
                break;
            }
        }
        assert_eq!(
            activation,
            Some((EntityId(0), Command::ActivateApple)),
            "apple projectile should activate APPLE-filter target with ActivateApple"
        );
    }

    /// C++ `FindTargetVictim` uses current-position range gating for
    /// FX targets: a target just beyond the old->new segment endpoint
    /// can still be hit when it is within one movement length of the
    /// arrow's current position.  This catches short final segments
    /// that would otherwise land without activating scripted targets.
    #[test]
    fn tick_arrows_arrow_target_uses_current_position_range_gate() {
        use crate::element::{ElementKind, ElementTarget, FxData, TargetData, TargetFilter};

        let mut target_element = ElementData {
            kind: ElementKind::Target,
            active: true,
            ..ElementData::default()
        };
        target_element.set_position_map(ElemPoint2D { x: 40.0, y: 0.0 });
        target_element.set_position(Point3D {
            x: 40.0,
            y: 0.0,
            z: 0.0,
        });
        let target = Entity::Target(ElementTarget {
            element: target_element,
            fx: FxData::default(),
            target: TargetData {
                action_filter: TargetFilter::ARROW,
                ..TargetData::default()
            },
        });

        let mut arrow_element = ElementData {
            kind: ElementKind::ObjectProjectile,
            active: true,
            ..ElementData::default()
        };
        arrow_element.set_position_map(ElemPoint2D { x: 0.0, y: 0.0 });
        arrow_element.set_position(Point3D {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        });
        let arrow = Entity::Projectile(ElementProjectile {
            element: arrow_element,
            object: ObjectData {
                associated_action: Action::Bow,
                object_type: ObjectType::Arrow,
                animation: Animation::ObjectFlying,
                quantity: 1,
                reference: Some(EntityId(0)),
                ..ObjectData::default()
            },
            projectile: ProjectileData {
                shooter: Some(EntityId(2)),
                flying: true,
                trajectory: vec![TrajectoryPoint {
                    position: Point3D {
                        x: 25.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    time: 1,
                }],
                damage: 30,
                ..ProjectileData::default()
            },
        });

        let mut entities: Vec<Option<Entity>> =
            vec![Some(target), Some(arrow), Some(make_pc(0.0, 0.0))];
        let results = tick_arrows(&mut entities, crate::sight_obstacle::ObstacleList::empty());

        assert!(
            results.iter().any(|r| {
                r.fx_target_hit == Some((EntityId(0), Command::ActivateArrow)) && r.despawn
            }),
            "arrow should activate target using C++ current-position range gate"
        );
    }

    /// C++ stationary FX-target checks still require
    /// `vtRange.Norm() <= range`, so a projectile with zero movement
    /// cannot activate a nearby target unless it is exactly centered on
    /// it. Rust used to fall back to `HIT_DISTANCE`, which could fire
    /// scripted targets from a stopped projectile.
    #[test]
    fn tick_arrows_stationary_projectile_does_not_radius_hit_fx_target() {
        use crate::element::{ElementKind, ElementTarget, FxData, TargetData, TargetFilter};

        let mut target_element = ElementData {
            kind: ElementKind::Target,
            active: true,
            ..ElementData::default()
        };
        target_element.set_position_map(ElemPoint2D { x: 10.0, y: 0.0 });
        target_element.set_position(Point3D {
            x: 10.0,
            y: 0.0,
            z: 0.0,
        });
        let target = Entity::Target(ElementTarget {
            element: target_element,
            fx: FxData::default(),
            target: TargetData {
                action_filter: TargetFilter::ARROW,
                ..TargetData::default()
            },
        });

        let mut arrow_element = ElementData {
            kind: ElementKind::ObjectProjectile,
            active: true,
            ..ElementData::default()
        };
        arrow_element.set_position_map(ElemPoint2D { x: 0.0, y: 0.0 });
        arrow_element.set_position(Point3D {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        });
        let arrow = Entity::Projectile(ElementProjectile {
            element: arrow_element,
            object: ObjectData {
                associated_action: Action::Bow,
                object_type: ObjectType::Arrow,
                animation: Animation::ObjectFlying,
                quantity: 1,
                reference: Some(EntityId(0)),
                ..ObjectData::default()
            },
            projectile: ProjectileData {
                shooter: Some(EntityId(2)),
                flying: true,
                trajectory: vec![TrajectoryPoint {
                    position: Point3D {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    time: 1,
                }],
                damage: 30,
                ..ProjectileData::default()
            },
        });

        let mut entities: Vec<Option<Entity>> =
            vec![Some(target), Some(arrow), Some(make_pc(0.0, 0.0))];
        let results = tick_arrows(&mut entities, crate::sight_obstacle::ObstacleList::empty());

        assert!(
            results.iter().all(|r| r.fx_target_hit.is_none()),
            "stationary projectile must not activate nearby FX target by radius"
        );
    }

    #[test]
    fn tick_arrows_has_no_artificial_lifetime_timeout() {
        let trajectory = (1..=320)
            .map(|i| TrajectoryPoint {
                position: Point3D {
                    x: i as f32 * 10.0,
                    y: 0.0,
                    z: 40.0,
                },
                time: 1,
            })
            .collect();
        let arrow = spawn_arrow(SpawnArrowParams {
            shooter: EntityId(0),
            bow_point: Point3D {
                x: 0.0,
                y: 0.0,
                z: 40.0,
            },
            trajectory_origin: ElemPoint2D { x: 0.0, y: 0.0 },
            target: EntityId(1),
            target_pos: ElemPoint2D { x: 3200.0, y: 0.0 },
            trajectory,
            damage: 30,
            layer: 0,
            lands_in_hole: false,
            initial_velocity: Point3D {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        });
        let mut entities: Vec<Option<Entity>> = vec![Some(make_pc(0.0, -100.0)), Some(arrow)];

        let mut despawn_frame = None;
        for frame in 0..260 {
            let results = tick_arrows(&mut entities, crate::sight_obstacle::ObstacleList::empty());
            if results.iter().any(|r| r.despawn) {
                despawn_frame = Some(frame);
                break;
            }
        }

        assert_eq!(
            despawn_frame, None,
            "C++ projectile lifetime is trajectory-driven, not capped at 250 frames"
        );
        match entities[1].as_ref().unwrap() {
            Entity::Projectile(p) => assert!(p.projectile.flying),
            _ => panic!("expected projectile"),
        }
    }

    /// Apple projectile flying through a target that does NOT have the
    /// APPLE filter leaves `fx_target_hit` unset — no activation is
    /// launched.
    #[test]
    fn tick_arrows_apple_projectile_ignores_non_apple_target() {
        use crate::element::{ElementKind, ElementTarget, FxData, TargetData, TargetFilter};

        let mut target_element = ElementData {
            kind: ElementKind::Target,
            active: true,
            ..ElementData::default()
        };
        target_element.set_position_map(ElemPoint2D { x: 50.0, y: 0.0 });
        target_element.set_position(Point3D {
            x: 50.0,
            y: 0.0,
            z: 0.0,
        });
        let target = Entity::Target(ElementTarget {
            element: target_element,
            fx: FxData::default(),
            target: TargetData {
                action_filter: TargetFilter::ARROW,
                ..TargetData::default()
            },
        });

        let trajectory = vec![TrajectoryPoint {
            position: Point3D {
                x: 50.0,
                y: 0.0,
                z: 0.0,
            },
            time: 2,
        }];
        let apple = Entity::Projectile(ElementProjectile {
            element: ElementData {
                kind: ElementKind::ObjectProjectile,
                active: true,
                ..ElementData::default()
            },
            object: ObjectData {
                associated_action: Action::Apple,
                object_type: ObjectType::Apple,
                animation: Animation::ObjectFlying,
                ..ObjectData::default()
            },
            projectile: ProjectileData {
                shooter: Some(EntityId(2)),
                flying: true,
                trajectory,
                ..ProjectileData::default()
            },
        });

        let mut entities: Vec<Option<Entity>> = vec![Some(target), Some(apple)];
        let results = tick_arrows(&mut entities, crate::sight_obstacle::ObstacleList::empty());
        assert!(
            results.is_empty(),
            "C++ FindTargetVictim ignores nonmatching target filters before HitTarget can burst"
        );
    }

    /// Apple impact on an FX target sets the burst animation + decay
    /// counter and despawns after `BURST_ANIMATION_FRAMES` ticks.
    #[test]
    fn tick_arrows_apple_bursts_and_despawns_after_frames() {
        use crate::element::{ElementKind, ElementTarget, FxData, TargetData, TargetFilter};

        let mut target_element = ElementData {
            kind: ElementKind::Target,
            active: true,
            ..ElementData::default()
        };
        target_element.set_position_map(ElemPoint2D { x: 10.0, y: 0.0 });
        let target = Entity::Target(ElementTarget {
            element: target_element,
            fx: FxData::default(),
            target: TargetData {
                action_filter: TargetFilter::APPLE,
                ..TargetData::default()
            },
        });
        let apple = Entity::Projectile(ElementProjectile {
            element: ElementData {
                kind: ElementKind::ObjectProjectile,
                active: true,
                ..ElementData::default()
            },
            object: ObjectData {
                object_type: ObjectType::Apple,
                animation: Animation::ObjectFlying,
                ..ObjectData::default()
            },
            projectile: ProjectileData {
                shooter: Some(EntityId(2)),
                flying: true,
                trajectory: vec![TrajectoryPoint {
                    position: Point3D {
                        x: 10.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    time: 1,
                }],
                ..ProjectileData::default()
            },
        });
        let mut entities: Vec<Option<Entity>> =
            vec![Some(target), Some(apple), Some(make_pc(0.0, 0.0))];

        // First tick: apple reaches target, bursts.
        let impact_results =
            tick_arrows(&mut entities, crate::sight_obstacle::ObstacleList::empty());
        assert!(
            impact_results
                .iter()
                .any(|r| r.fx_target_hit.is_some() && !r.despawn),
            "apple must NOT despawn on impact frame — it bursts first"
        );
        let proj_after = entities[1].as_ref().unwrap();
        match proj_after {
            Entity::Projectile(p) => {
                assert!(!p.projectile.flying);
                assert_eq!(p.object.animation, Animation::ObjectBursting);
                assert_eq!(p.projectile.burst_countdown, BURST_ANIMATION_FRAMES);
            }
            _ => panic!("expected apple projectile"),
        }

        // Subsequent ticks: decrement burst_countdown; despawn on the
        // tick it reaches 0.
        let mut ticks_until_despawn = 0;
        for _ in 0..20 {
            ticks_until_despawn += 1;
            let results = tick_arrows(&mut entities, crate::sight_obstacle::ObstacleList::empty());
            if results.iter().any(|r| r.despawn) {
                break;
            }
        }
        assert_eq!(
            ticks_until_despawn, BURST_ANIMATION_FRAMES as usize,
            "burst should last exactly BURST_ANIMATION_FRAMES ticks"
        );
    }

    /// Apple impact yields impact FX 509; stone yields 508; arrow hit
    /// without shield yields no impact FX (silent).
    #[test]
    fn tick_arrows_impact_fx_per_projectile_type() {
        fn spawn_projectile_at_impact(obj: ObjectType) -> Entity {
            let mut element = ElementData {
                kind: ElementKind::ObjectProjectile,
                active: true,
                ..ElementData::default()
            };
            element.set_position_map(ElemPoint2D { x: 0.0, y: 0.0 });
            element.set_position(Point3D {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            });
            Entity::Projectile(ElementProjectile {
                element,
                object: ObjectData {
                    object_type: obj,
                    animation: Animation::ObjectFlying,
                    ..ObjectData::default()
                },
                projectile: ProjectileData {
                    shooter: Some(EntityId(1)),
                    flying: true,
                    // Empty trajectory → immediate "trajectory exhausted".
                    trajectory: Vec::new(),
                    ..ProjectileData::default()
                },
            })
        }

        let fx_for = |obj: ObjectType| -> Option<u32> {
            let mut entities: Vec<Option<Entity>> = vec![
                Some(spawn_projectile_at_impact(obj)),
                Some(make_pc(100.0, 0.0)),
            ];
            let results = tick_arrows(&mut entities, crate::sight_obstacle::ObstacleList::empty());
            results.into_iter().find_map(|r| r.impact_fx)
        };
        assert_eq!(fx_for(ObjectType::Apple), Some(509));
        assert_eq!(fx_for(ObjectType::Stone), Some(508));
        assert_eq!(fx_for(ObjectType::Arrow), None);
    }

    /// `spawn_apple` builds a flying apple projectile with Apple
    /// object_type and a ballistic trajectory.
    #[test]
    fn spawn_apple_creates_flying_apple_projectile() {
        let start = Point3D {
            x: 0.0,
            y: 0.0,
            z: 40.0,
        };
        let end = Point3D {
            x: 100.0,
            y: 0.0,
            z: 20.0,
        };
        let apple = spawn_apple(EntityId(0), start, end, Some(EntityId(1)), None, 0, None);
        match apple {
            Entity::Projectile(p) => {
                assert!(p.projectile.flying);
                assert_eq!(p.object.object_type, ObjectType::Apple);
                assert_eq!(p.object.associated_action, Action::Apple);
                assert_eq!(p.object.animation, Animation::ObjectFlying);
                assert_eq!(p.projectile.shooter, Some(EntityId(0)));
                assert_eq!(p.object.reference, Some(EntityId(1)));
                assert!(!p.projectile.trajectory.is_empty());
            }
            _ => panic!("expected apple projectile"),
        }
    }

    #[test]
    fn apply_arrow_hit_wounds_soldier() {
        let mut entities: Vec<Option<Entity>> =
            vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))];
        let died = apply_arrow_hit(&mut entities, EntityId(1), EntityId(0), 30, 0);
        assert!(!died, "30 damage shouldn't kill a 100hp soldier");

        let life = match entities[1].as_ref().unwrap() {
            Entity::Soldier(s) => s.npc.life_points,
            _ => unreachable!(),
        };
        assert_eq!(life, 70);
    }

    #[test]
    fn apply_arrow_hit_kills_soldier_at_low_hp() {
        let mut entities: Vec<Option<Entity>> =
            vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))];
        if let Some(Some(Entity::Soldier(s))) = entities.get_mut(1) {
            s.npc.life_points = 5;
        }
        let died = apply_arrow_hit(&mut entities, EntityId(1), EntityId(0), 30, 0);
        assert!(died);
        let posture = entities[1].as_ref().unwrap().element_data().posture;
        assert_eq!(posture, Posture::Dead);
    }

    #[test]
    fn build_shoot_bow_element_produces_interaction_element() {
        let elem = build_shoot_bow_element(EntityId(0), EntityId(1));
        assert_eq!(elem.command, Command::ShootBow);
        match &elem.data {
            SequenceElementData::Interaction { antagonist } => {
                assert_eq!(*antagonist, Some(EntityId(1)));
            }
            other => panic!("expected Interaction, got {:?}", other),
        }
    }

    #[test]
    fn hit_chance_bias_scales_with_skill() {
        // RNG is seeded via the sim_rng scope — deterministic across runs.
        crate::sim_rng::with_seed(1, || {
            if let Some(bias) = roll_hit_and_compute_bias(0, 90) {
                // Miss with 90 skill → very small bias.
                assert!(bias.x.abs() < 1.0);
                assert!(bias.y.abs() < 1.0);
                assert!(bias.z.abs() < 1.0);
            }
        });
    }

    #[test]
    fn shoot_mode_from_action_state_mapping() {
        assert!(matches!(
            shoot_mode_from_action_state(ActionState::AimingWithBow),
            ShootMode::Normal
        ));
        assert!(matches!(
            shoot_mode_from_action_state(ActionState::AimingWithBowUp),
            ShootMode::Long
        ));
        assert!(matches!(
            shoot_mode_from_action_state(ActionState::AimingWithBowDown),
            ShootMode::Down
        ));
    }

    #[test]
    fn bow_point_order_types_are_non_anonymous_cxx_compute_bow_point_ids() {
        assert_eq!(
            bow_point_order_type_for_mode(ShootMode::Normal),
            OrderType::ShootingWithBow
        );
        assert_eq!(
            bow_point_order_type_for_mode(ShootMode::Long),
            OrderType::ShootingWithBowUp
        );
        assert_eq!(
            bow_point_order_type_for_mode(ShootMode::Down),
            OrderType::ShootingWithBowLeaningOut
        );
    }

    #[test]
    fn aim_transitions_from_up_to_normal() {
        let t = aim_transition_orders(ActionState::AimingWithBowUp, ShootMode::Normal, false);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0], OrderType::TransitionLoweringBow);
    }

    #[test]
    fn aim_transitions_from_down_to_long() {
        let t = aim_transition_orders(ActionState::AimingWithBowDown, ShootMode::Long, false);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0], OrderType::TransitionRaisingBowLeaningOut);
        assert_eq!(t[1], OrderType::TransitionRaisingBow);
    }

    #[test]
    fn aim_transitions_use_anonymous_raise_lower_orders() {
        let normal = aim_transition_orders(ActionState::AimingWithBowUp, ShootMode::Normal, true);
        assert_eq!(normal, vec![OrderType::TransitionLoweringBowAnonymous]);

        let long = aim_transition_orders(ActionState::AimingWithBow, ShootMode::Long, true);
        assert_eq!(long, vec![OrderType::TransitionRaisingBowAnonymous]);
    }

    #[test]
    fn unequip_bow_sets_waiting_on_animation_start() {
        let mut pc = make_pc(0.0, 0.0);
        pc.actor_data_mut().unwrap().action_state = ActionState::AimingWithBow;

        apply_bow_transition_state_side_effect(
            &mut pc,
            OrderType::TransitionUnequipBow,
            SpriteMotionState::Start,
        );

        assert_eq!(
            pc.actor_data().unwrap().action_state,
            ActionState::Waiting,
            "C++ TransitionUnequipBow sets Waiting on RHMOTION_START"
        );
    }

    #[test]
    fn leaning_out_bow_transitions_update_posture_like_soldier_execute() {
        let mut soldier = make_soldier(0.0, 0.0);
        soldier.actor_data_mut().unwrap().action_state = ActionState::AimingWithBow;

        apply_bow_transition_state_side_effect(
            &mut soldier,
            OrderType::TransitionLoweringBowLeaningOut,
            SpriteMotionState::Done,
        );
        assert_eq!(soldier.element_data().posture, Posture::LeaningOut);
        assert_eq!(
            soldier.actor_data().unwrap().action_state,
            ActionState::AimingWithBowDown
        );

        apply_bow_transition_state_side_effect(
            &mut soldier,
            OrderType::TransitionRaisingBowLeaningOut,
            SpriteMotionState::Done,
        );
        assert_eq!(soldier.element_data().posture, Posture::Upright);
        assert_eq!(
            soldier.actor_data().unwrap().action_state,
            ActionState::AimingWithBow
        );
    }

    #[test]
    fn down_bow_shot_release_keeps_leaning_out_posture() {
        let mut pc = make_pc(0.0, 0.0);
        pc.element_data_mut().posture = Posture::LeaningOut;
        bind_test_bow_release_rows(&mut pc, OrderType::ShootingWithBowLeaningOut);
        let mut target = make_soldier(50.0, 0.0);
        target.element_data_mut().posture = Posture::LeaningOut;
        let mut entities: Vec<Option<Entity>> = vec![Some(pc), Some(target)];
        let (mut sm, seq_id, elem_idx) = launch_test_shoot_element(EntityId(0), EntityId(1));

        begin_bow_shot(
            &mut entities,
            &mut sm,
            EntityId(0),
            EntityId(1),
            seq_id,
            elem_idx,
            false,
            10,
            Some(ShootMode::Down),
            &mut 1u32,
        );

        let mut fired = Vec::new();
        for _ in 0..16 {
            fired.extend(tick_bow_shots(&mut entities, &mut sm).fired);
            if !fired.is_empty() {
                break;
            }
        }
        assert_eq!(fired.len(), 1);
        assert_eq!(
            entities[0].as_ref().unwrap().element_data().posture,
            Posture::LeaningOut
        );
        assert_eq!(
            entities[0]
                .as_ref()
                .unwrap()
                .actor_data()
                .unwrap()
                .action_state,
            ActionState::AimingWithBow
        );
    }

    #[test]
    fn compute_bow_point_offsets() {
        // 3D position: x=10, y=20 (map_y + elevation), z=0 (ground level)
        let pos = Point3D {
            x: 10.0,
            y: 20.0,
            z: 0.0,
        };
        let pt = compute_bow_point(pos, ShootMode::Normal, 0, None);
        assert_eq!(pt.z, BOW_Z_OFFSET_NORMAL);
        assert_eq!(pt.x, 10.0); // no lateral shift for normal

        let pt_long = compute_bow_point(pos, ShootMode::Long, 0, None);
        assert_eq!(pt_long.z, BOW_Z_OFFSET_LONG);

        // Down shot should shift laterally by 20 units in direction.
        let pt_down = compute_bow_point(pos, ShootMode::Down, 4, None);
        assert_eq!(pt_down.z, BOW_Z_OFFSET_NORMAL);
        // Sector 4 = east (+x), so x shifts by ~20
        assert!(pt_down.x > pos.x + 15.0, "down-shot should shift x");

        // With non-zero elevation, Z should be elevation + offset,
        // and Y should have elevation added (isometric projection
        // adds elevation into the hand Y).
        let elevated_pos = Point3D {
            x: 10.0,
            y: 50.0,
            z: 30.0,
        };
        let pt_elev = compute_bow_point(elevated_pos, ShootMode::Normal, 0, None);
        assert_eq!(pt_elev.z, 30.0 + BOW_Z_OFFSET_NORMAL);
        assert_eq!(pt_elev.y, 50.0 + 30.0); // map_y + elevation
    }

    // ═══════════════════════════════════════════════════════════════
    //  Projectile pipeline parity tests
    //
    //  Verification of the projectile-tick branches: hit-an-actor,
    //  hit-a-shield (deflect + fall), miss-and-fall, and the wasp-nest
    //  throw impact path.
    // ═══════════════════════════════════════════════════════════════

    /// A projectile that passes close to a target on the ground (not
    /// airborne) still misses when the target's posture is one of the
    /// "untargetable" postures.  Spot-check one of them
    /// (`Posture::Lying`) to confirm the filter actually prunes the
    /// snapshot.
    #[test]
    fn tick_arrows_skips_lying_victim() {
        use crate::element::Posture;

        let mut soldier = make_soldier(50.0, 0.0);
        soldier.set_posture(Posture::Lying);

        // Arrow trajectory aimed directly at where the belt would be
        // if the soldier were upright — but since it's lying, no hit.
        let trajectory = vec![TrajectoryPoint {
            position: Point3D {
                x: 50.0,
                y: 0.0,
                z: 25.0,
            },
            time: 2,
        }];
        let arrow = spawn_arrow(SpawnArrowParams {
            shooter: EntityId(0),
            bow_point: Point3D {
                x: 0.0,
                y: 0.0,
                z: 25.0,
            },
            trajectory_origin: ElemPoint2D { x: 0.0, y: 0.0 },
            target: EntityId(1),
            target_pos: ElemPoint2D { x: 50.0, y: 0.0 },
            trajectory,
            damage: 30,
            layer: 0,
            lands_in_hole: false,
            initial_velocity: Point3D {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        });

        let mut entities: Vec<Option<Entity>> =
            vec![Some(make_pc(0.0, 0.0)), Some(soldier), Some(arrow)];

        let mut any_hit = None;
        for _ in 0..10 {
            for r in tick_arrows(&mut entities, crate::sight_obstacle::ObstacleList::empty()) {
                if r.hit_target.is_some() {
                    any_hit = r.hit_target;
                    break;
                }
            }
        }
        assert!(
            any_hit.is_none(),
            "arrow must not hit a lying soldier (posture filter)"
        );
    }

    /// Arrow that sails past a target in 3D does not hit it even when
    /// their 2D projections coincide.  Previously the 2D point check
    /// falsely reported a hit on any arrow passing directly over a
    /// target; the 3D line-segment check does not.  Regression test
    /// for that gap.
    #[test]
    fn tick_arrows_does_not_hit_when_arcing_overhead() {
        // Arrow stays well above the soldier's belt (Z=25).
        let trajectory = vec![
            TrajectoryPoint {
                position: Point3D {
                    x: 30.0,
                    y: 0.0,
                    z: 80.0,
                },
                time: 2,
            },
            TrajectoryPoint {
                position: Point3D {
                    x: 60.0,
                    y: 0.0,
                    z: 78.0,
                },
                time: 2,
            },
            TrajectoryPoint {
                position: Point3D {
                    x: 90.0,
                    y: 0.0,
                    z: 76.0,
                },
                time: 2,
            },
        ];
        let arrow = spawn_arrow(SpawnArrowParams {
            shooter: EntityId(0),
            bow_point: Point3D {
                x: 0.0,
                y: 0.0,
                z: 82.0,
            },
            trajectory_origin: ElemPoint2D { x: 0.0, y: 0.0 },
            target: EntityId(1),
            target_pos: ElemPoint2D { x: 90.0, y: 0.0 },
            trajectory,
            damage: 30,
            layer: 0,
            lands_in_hole: false,
            initial_velocity: Point3D {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        });

        let mut entities: Vec<Option<Entity>> = vec![
            Some(make_pc(0.0, 0.0)),
            Some(make_soldier(50.0, 0.0)),
            Some(arrow),
        ];

        let mut any_hit = None;
        for _ in 0..20 {
            for r in tick_arrows(&mut entities, crate::sight_obstacle::ObstacleList::empty()) {
                if r.hit_target.is_some() {
                    any_hit = r.hit_target;
                }
            }
            if any_hit.is_some() {
                break;
            }
        }
        assert!(
            any_hit.is_none(),
            "arrow arcing 50+ units above a soldier's belt must not register a hit"
        );
    }

    /// Arrow that shares the soldier's 2D column but passes at belt
    /// height hits; trajectory comes down to the belt then continues
    /// past.  Complement to [`tick_arrows_does_not_hit_when_arcing_overhead`].
    #[test]
    fn tick_arrows_hits_through_belt_column() {
        let trajectory = vec![
            TrajectoryPoint {
                position: Point3D {
                    x: 50.0,
                    y: 0.0,
                    z: 25.0,
                },
                time: 2,
            },
            TrajectoryPoint {
                position: Point3D {
                    x: 80.0,
                    y: 0.0,
                    z: 20.0,
                },
                time: 2,
            },
        ];
        let arrow = spawn_arrow(SpawnArrowParams {
            shooter: EntityId(0),
            bow_point: Point3D {
                x: 0.0,
                y: 0.0,
                z: 30.0,
            },
            trajectory_origin: ElemPoint2D { x: 0.0, y: 0.0 },
            target: EntityId(1),
            target_pos: ElemPoint2D { x: 80.0, y: 0.0 },
            trajectory,
            damage: 30,
            layer: 0,
            lands_in_hole: false,
            initial_velocity: Point3D {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        });
        let mut entities: Vec<Option<Entity>> = vec![
            Some(make_pc(0.0, 0.0)),
            Some(make_soldier(20.0, 0.0)),
            Some(arrow),
        ];
        let mut hit = None;
        for _ in 0..20 {
            for r in tick_arrows(&mut entities, crate::sight_obstacle::ObstacleList::empty()) {
                if r.hit_target.is_some() {
                    hit = r.hit_target;
                }
            }
            if hit.is_some() {
                break;
            }
        }
        assert_eq!(hit, Some(EntityId(1)));
    }

    /// Shield intersection flips the projectile into the falling state
    /// and emits a `shield_hit` result.  The projectile keeps flying
    /// on a new deflected trajectory toward the ground — it must not
    /// despawn on the same tick.
    #[test]
    fn tick_arrows_shield_hit_deflects_and_keeps_flying() {
        crate::sim_rng::with_seed(1, || {
            use crate::element::ActionState;

            // Shield holder facing east (sector 4 = +X), toward the arrow
            // which is flying westward from bow_point (100,…) to target
            // (50,…).  The shield quad projects forward in the holder's
            // facing direction, so the arrow's path intersects it.
            let mut shield_holder = make_soldier(50.0, 0.0);
            {
                let actor = shield_holder.actor_data_mut().unwrap();
                actor.action_state = ActionState::HoldingShield;
                let params = shield_params_for_soldier(20, 40);
                let obs = compute_shield_obstacle(ElemPoint2D { x: 50.0, y: 0.0 }, 0.0, 4, &params);
                actor.shield_obstacle = Some(obs);
            }
            shield_holder.element_data_mut().set_direction_instantly(4);

            // Arrow flying from +X toward the shield holder at Z=40 —
            // mid-shield height for `shield_params_for_soldier(20, 40)`
            // which places the quad between Z=30 and Z=50.  The isometric
            // projection requires `world.y = map_y + z`, so for the arrow
            // to render at the holder's map-space column (map_y=0) it
            // needs world.y = 40 — see `compute_bow_point` which adds
            // elevation into the hand Y the same way.
            let trajectory = vec![TrajectoryPoint {
                position: Point3D {
                    x: 50.0,
                    y: 40.0,
                    z: 40.0,
                },
                time: 2,
            }];
            let arrow = spawn_arrow(SpawnArrowParams {
                shooter: EntityId(0),
                bow_point: Point3D {
                    x: 100.0,
                    y: 40.0,
                    z: 40.0,
                },
                trajectory_origin: ElemPoint2D { x: 100.0, y: 0.0 },
                target: EntityId(1),
                target_pos: ElemPoint2D { x: 50.0, y: 0.0 },
                trajectory,
                damage: 30,
                layer: 0,
                lands_in_hole: false,
                initial_velocity: Point3D {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
            });

            let mut entities: Vec<Option<Entity>> =
                vec![Some(make_pc(100.0, 0.0)), Some(shield_holder), Some(arrow)];

            // Advance ticks until the shield_hit fires.
            let mut shield_hit = None;
            let mut despawn_seen = false;
            for _ in 0..10 {
                for r in tick_arrows(&mut entities, crate::sight_obstacle::ObstacleList::empty()) {
                    if let Some(holder) = r.shield_hit {
                        shield_hit = Some(holder);
                        despawn_seen = r.despawn;
                    }
                }
                if shield_hit.is_some() {
                    break;
                }
            }
            assert_eq!(
                shield_hit,
                Some(EntityId(1)),
                "arrow must report shield hit on the holder"
            );
            assert!(
                !despawn_seen,
                "shield-hit arrow keeps flying (falling) on same tick"
            );

            // The projectile should be flagged as falling, and the hit
            // check must now skip (falling arrows pass through bodies).
            match entities[2].as_ref().unwrap() {
                Entity::Projectile(p) => {
                    assert!(
                        p.projectile.falling,
                        "shield deflection flips arrow into falling state"
                    );
                    assert!(
                        p.projectile.flying,
                        "falling arrow still visually flying (arcs to ground)"
                    );
                    assert_ne!(
                        p.element.position(),
                        (Point3D {
                            x: 50.0,
                            y: 40.0,
                            z: 40.0
                        }),
                        "C++ MakeFallingDown advances the falling trajectory immediately"
                    );
                }
                _ => panic!("expected projectile"),
            }
        });
    }

    #[test]
    fn non_shield_arrow_ricochet_advances_immediately() {
        crate::sim_rng::with_seed(1, || {
            let trajectory = vec![TrajectoryPoint {
                position: Point3D {
                    x: 50.0,
                    y: 0.0,
                    z: 0.0,
                },
                time: 2,
            }];
            let arrow = spawn_arrow(SpawnArrowParams {
                shooter: EntityId(0),
                bow_point: Point3D {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                trajectory_origin: ElemPoint2D { x: 0.0, y: 0.0 },
                target: EntityId(1),
                target_pos: ElemPoint2D { x: 50.0, y: 0.0 },
                trajectory,
                damage: 30,
                layer: 0,
                lands_in_hole: false,
                initial_velocity: Point3D {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
            });
            let mut projectile = match arrow {
                Entity::Projectile(p) => p,
                _ => panic!("expected arrow projectile"),
            };
            projectile.element.set_direction_instantly(4);
            let impact_position = projectile.element.position();

            make_arrow_falling_down(&mut projectile, false);

            assert!(projectile.projectile.falling);
            assert!(projectile.projectile.flying);
            assert_eq!(
                projectile.element.sprite.current_row, 12,
                "impact-frame render uses the first falling sector"
            );
            assert!((3..=5).contains(&projectile.element.sprite.current_frame));
            assert_eq!(
                projectile.projectile.falling_direction, 10,
                "falling refresh rotates the next tumble sector by -2"
            );
            assert_ne!(
                projectile.element.position(),
                impact_position,
                "C++ MakeFallingDown calls Hourglass for armor ricochets too"
            );
        });
    }

    /// An arrow that runs out of trajectory without hitting anything
    /// stops flying on the landing tick and despawns.
    #[test]
    fn tick_arrows_miss_and_land_despawns() {
        let trajectory = vec![TrajectoryPoint {
            position: Point3D {
                x: 10.0,
                y: 0.0,
                z: 0.0,
            },
            time: 1,
        }];
        let arrow = spawn_arrow(SpawnArrowParams {
            shooter: EntityId(0),
            bow_point: Point3D {
                x: 0.0,
                y: 0.0,
                z: 5.0,
            },
            trajectory_origin: ElemPoint2D { x: 0.0, y: 0.0 },
            target: EntityId(0),
            target_pos: ElemPoint2D { x: 10.0, y: 0.0 },
            trajectory,
            damage: 30,
            layer: 0,
            lands_in_hole: false,
            initial_velocity: Point3D {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        });
        // No other humans in range — arrow will fly out and land.
        let mut entities: Vec<Option<Entity>> = vec![Some(make_pc(0.0, 0.0)), Some(arrow)];

        let mut despawn = false;
        for _ in 0..10 {
            for r in tick_arrows(&mut entities, crate::sight_obstacle::ObstacleList::empty()) {
                if r.despawn && r.hit_target.is_none() && r.shield_hit.is_none() {
                    despawn = true;
                }
            }
            if despawn {
                break;
            }
        }
        assert!(
            despawn,
            "arrow that misses should land and despawn without hit_target / shield_hit"
        );
    }

    /// Wasp nest thrown at a ground target bursts (`flying == false`)
    /// once its bounce trajectory is exhausted.  Unlike arrows, the
    /// nest keeps a projectile slot for the post-impact wasp swarm
    /// spawn — here we just assert it stops flying.
    #[test]
    fn spawn_wasp_nest_lands_and_stops_flying() {
        let throw_pos = Point3D {
            x: 0.0,
            y: 0.0,
            z: 50.0,
        };
        let target_pos = Point3D {
            x: 80.0,
            y: 0.0,
            z: 0.0,
        };
        let nest = spawn_wasp_nest(EntityId(0), throw_pos, target_pos, 0, None);

        match &nest {
            Entity::Projectile(p) => {
                assert!(p.projectile.flying, "nest starts flying");
                assert_eq!(p.object.object_type, ObjectType::BonusWaspNest);
                assert!(
                    !p.projectile.trajectory.is_empty(),
                    "wasp nest must produce a ballistic trajectory"
                );
            }
            _ => panic!("expected projectile"),
        }

        let mut entities: Vec<Option<Entity>> = vec![Some(make_pc(0.0, 0.0)), Some(nest)];
        // Wasp nests are skipped by `tick_arrows` (their impact burst +
        // swarm spawn lives on the engine in `tick_wasp_nests`).  Drive
        // the trajectory directly here via `advance_trajectory_one_frame`;
        // bouncing nests can produce the full 50-waypoint trajectory
        // (~100 ticks at TIME_FLYSEGMENT=2), so 300 iterations is a
        // generous bound.
        for _ in 0..300 {
            if let Some(Entity::Projectile(p)) = entities[1].as_mut() {
                if !p.projectile.flying {
                    break;
                }
                p.advance_trajectory_one_frame();
            }
        }
        let p = match entities[1].as_ref().unwrap() {
            Entity::Projectile(p) => p,
            _ => panic!("nest entity lost"),
        };
        assert!(
            !p.projectile.flying,
            "wasp nest must stop flying once its trajectory is exhausted"
        );
    }
}
