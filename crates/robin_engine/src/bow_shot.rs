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
//!    trajectory runs out, it returns an [`ArrowTickResult`].  The
//!    engine layer turns that into a `ReceiveArrowDamage` sequence
//!    element so damage and death animations follow the normal
//!    sequence-manager path.
//!
//! ## UI action-slot refresh
//!
//! When ammo reaches 0, the ammo decrement path
//! (`engine/combat.rs::decrement_bow_ammo`) calls
//! `EngineInner::disable_pc_action`, which resolves Bow through the
//! PC's profile action list and sets that portrait slot in
//! `PcData::disabled_actions`.  The HUD action-slot strip is
//! immediate-mode (see `ui_panel.rs`) and re-reads `disabled_actions`
//! each frame, so no messenger notification is needed; the next frame
//! shows the disabled bow slot automatically.

use crate::combat::{self, ConcussionContext};
use crate::coordinates::{GroundPoint, MapPoint, WorldPoint3D, WorldVec3D};
use crate::element::{
    ActionState, Animation, Command, ElementData, ElementKind, ElementProjectile, Entity, EntityId,
    ObjectData, ObjectType, Posture, ProjectileData, TargetFilter, TrajectoryPoint,
};
use crate::entities::{Entities, EntitySlots};
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

fn set_projectile_animation(proj: &mut ElementProjectile, animation: Animation) {
    proj.object.animation = animation;
    if proj.element.sprite.current_conversion().is_empty() {
        return;
    }
    assert!(
        proj.element.sprite.has_animation(animation),
        "projectile {:?} is missing required animation {animation:?}",
        proj.object.object_type
    );
    // Original Apple/Stone HitObstacle/HitHuman/HitTarget use the
    // directionless ForceAnimation overload, whose default direction is 0.
    proj.element.sprite.force_animation(animation, 0);
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
///
/// `position_ground` is the holder's ground-space (world) XY, not its
/// screen-projected map position: sight obstacles live in ground space, and
/// every ray query against them supplies ground coordinates.  For a holder
/// standing above z = 0 the two differ by the holder's elevation, and folding
/// that elevation into Y here would then be counted a second time by the
/// obstacle's own Z extent.
pub fn compute_shield_obstacle(
    position_ground: MapPoint,
    z: f32,
    direction_sector: i16,
    params: &ShieldParams,
) -> crate::sight_obstacle::SightObstacle {
    use crate::element::direction_vector_16;
    use crate::sight_obstacle::{
        ObstaclePoint, SIGHTOBSTACLE_SHIELD, SIGHTOBSTACLE_SOLID, SightObstacle,
    };

    let (dir_x, dir_y) = direction_vector_16(direction_sector);

    // Pre-offset: move position forward in the facing direction
    // before constructing the shield box.
    let px = position_ground.x + params.pre_offset * dir_x;
    let py = position_ground.y + params.pre_offset * dir_y;
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

/// Recover the authored shoot mode from a concrete shooting order. Used when
/// rebuilding Rust's derived active-shot latch after loading an Original save.
pub(crate) fn shoot_mode_for_order(order: OrderType) -> Option<ShootMode> {
    match order {
        OrderType::ShootingWithBow | OrderType::ShootingWithBowAnonymous => Some(ShootMode::Normal),
        OrderType::ShootingWithBowUp | OrderType::ShootingWithBowUpAnonymous => {
            Some(ShootMode::Long)
        }
        OrderType::ShootingWithBowLeaningOut => Some(ShootMode::Down),
        _ => None,
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

/// Absolute projected bow hotspot used by C++ `ComputeBowPoint`:
/// `GetPositionSprite() + sprite.GetPoint(shoot_animation, direction)`.
pub(crate) fn bow_sprite_hand_point(
    entity: &Entity,
    mode: ShootMode,
    direction: i16,
) -> Option<MapPoint> {
    let dir = u16::try_from(direction).ok()?;
    let sprite_pos = entity.cxx_position_sprite();
    let offset = entity
        .element_data()
        .sprite
        .get_point(bow_point_order_type_for_mode(mode), dir)?;
    Some(MapPoint::new(
        sprite_pos.x + offset.x,
        sprite_pos.y + offset.y,
    ))
}

/// Canonical order set accepted by the selected active-bow owner.
pub(crate) const ACTIVE_BOW_ORDERS: &[OrderType] = &[
    OrderType::ShootingWithBow,
    OrderType::ShootingWithBowUp,
    OrderType::ShootingWithBowLeaningOut,
    OrderType::ShootingWithBowAnonymous,
    OrderType::ShootingWithBowUpAnonymous,
    OrderType::TransitionEquipBow,
    OrderType::TransitionRaisingBow,
    OrderType::TransitionLoweringBow,
    OrderType::TransitionRaisingBowLeaningOut,
    OrderType::TransitionLoweringBowLeaningOut,
    OrderType::TransitionLoadingBow,
    OrderType::TransitionUnloadBow,
    OrderType::TransitionUnequipBow,
    OrderType::TransitionEquipBowAnonymous,
    OrderType::TransitionRaisingBowAnonymous,
    OrderType::TransitionLoweringBowAnonymous,
    OrderType::TransitionLoadingBowAnonymous,
    OrderType::TransitionUnloadBowAnonymous,
    OrderType::TransitionUnequipBowAnonymous,
];

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
        OrderType::TransitionEquipBow
            | OrderType::TransitionRaisingBow
            | OrderType::TransitionLoweringBow
            | OrderType::TransitionRaisingBowLeaningOut
            | OrderType::TransitionLoweringBowLeaningOut
            | OrderType::TransitionLoadingBow
            | OrderType::TransitionUnloadBow
            | OrderType::TransitionUnequipBow
            | OrderType::TransitionEquipBowAnonymous
            | OrderType::TransitionRaisingBowAnonymous
            | OrderType::TransitionLoweringBowAnonymous
            | OrderType::TransitionLoadingBowAnonymous
            | OrderType::TransitionUnloadBowAnonymous
            | OrderType::TransitionUnequipBowAnonymous
    )
}

pub(crate) fn is_active_bow_order(ot: OrderType) -> bool {
    ACTIVE_BOW_ORDERS.contains(&ot)
}

fn has_active_bow_order(element: &crate::sequence::SequenceElement) -> bool {
    element
        .orders
        .iter()
        .any(|order| is_active_bow_order(order.order_type))
}

fn apply_bow_transition_state_side_effect(
    entity: &mut Entity,
    order_type: OrderType,
    motion: SpriteMotionState,
) {
    let action_state = match order_type {
        OrderType::TransitionEquipBow | OrderType::TransitionEquipBowAnonymous
            if motion == SpriteMotionState::Start =>
        {
            if entity.element_data().posture != Posture::AnonymousArcher {
                entity.element_data_mut().posture = Posture::Upright;
            }
            Some(ActionState::AimingWithBow)
        }
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
        OrderType::TransitionUnloadBow | OrderType::TransitionUnloadBowAnonymous
            if motion == SpriteMotionState::Start =>
        {
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
    thrower_to_target: WorldVec3D,
    apex_height: f32,
    mass: f32,
    mut flight_time: u16,
    target_forecasted_movement: Option<WorldVec3D>,
) -> WorldVec3D {
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
        WorldVec3D {
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
        WorldVec3D {
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
    start: WorldPoint3D,
    initial_velocity: WorldVec3D,
    mass: f32,
    flat_shot: bool,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
) -> Vec<TrajectoryPoint> {
    compute_trajectory_ballistic_with_terminal_obstacle(
        start,
        initial_velocity,
        mass,
        flat_shot,
        obstacle_check,
    )
    .0
}

/// Build an arrow trajectory and retain the exact obstacle hit by its
/// terminal segment.
///
/// Original `RHElementProjectile::ComputeTrajectory` keeps the
/// `RHSightObstacle*` returned by `IsReachableImpact` and uses that exact
/// object when assigning prospective landing layer/sector membership.  The
/// plain trajectory API intentionally exposes only waypoints; arrow spawning
/// uses this companion API so an overlapping projection polygon cannot replace
/// a non-projection impact obstacle during the later membership lookup.
pub fn compute_trajectory_ballistic_with_terminal_obstacle(
    start: WorldPoint3D,
    initial_velocity: WorldVec3D,
    mass: f32,
    flat_shot: bool,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
) -> (
    Vec<TrajectoryPoint>,
    Option<crate::position_interface::ObstacleHandle>,
) {
    let (trajectory, terminal_obstacle, _) = compute_trajectory_ballistic_impl(
        start,
        initial_velocity,
        mass,
        flat_shot,
        obstacle_check,
        None,
    );
    (trajectory, terminal_obstacle)
}

/// Companion used by projectile membership initialization, where an exact
/// bare-ground impact must remain distinguishable from a trajectory that
/// simply exhausted its integration without hitting anything.
pub fn compute_trajectory_ballistic_with_terminal_impact(
    start: WorldPoint3D,
    initial_velocity: WorldVec3D,
    mass: f32,
    flat_shot: bool,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
) -> (
    Vec<TrajectoryPoint>,
    Option<crate::position_interface::ObstacleHandle>,
    bool,
) {
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
    start: WorldPoint3D,
    initial_velocity: WorldVec3D,
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
    .0
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
    normal: WorldVec3D,
    /// Per-obstacle bounce factors (vertical, horizontal).
    /// Defaults to `(1.0, 1.0)` for ground (no material factor).
    obstacle_bounce_v: f32,
    obstacle_bounce_h: f32,
}

/// Classify a trajectory impact against an obstacle (or ground if `None`).
fn classify_impact(
    impact: WorldPoint3D,
    segment_from: WorldPoint3D,
    segment_to: WorldPoint3D,
    obstacle: Option<&crate::sight_obstacle::SightObstacle>,
) -> ImpactInfo {
    let Some(obs) = obstacle else {
        return ImpactInfo {
            kind: ImpactKind::Ground,
            normal: WorldVec3D {
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
        && obs
            .box_ground
            .contains_point(GroundPoint::new(impact.x, impact.y));

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
            normal: WorldVec3D {
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
        normal: WorldVec3D {
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
    velocity: WorldVec3D,
    new_vz: f32,
    ratio: f32,
    info: ImpactInfo,
    projectile_bounce: (f32, f32),
) -> WorldVec3D {
    let (proj_bv, proj_bh) = projectile_bounce;
    match info.kind {
        ImpactKind::Ground => WorldVec3D {
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

            WorldVec3D {
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

            WorldVec3D {
                x: info.obstacle_bounce_h * proj_bh * (velocity.x + comp_x),
                y: ASPECT_RATIO * info.obstacle_bounce_h * proj_bh * (velocity.y + comp_y),
                z: info.obstacle_bounce_v * proj_bv * (interp_vz + comp_z),
            }
        }
    }
}

fn compute_trajectory_ballistic_impl(
    start: WorldPoint3D,
    initial_velocity: WorldVec3D,
    mass: f32,
    _flat_shot: bool,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
    bounce: Option<(f32, f32)>,
) -> (
    Vec<TrajectoryPoint>,
    Option<crate::position_interface::ObstacleHandle>,
    bool,
) {
    /// Top-impact termination speed threshold (`||v|| < 5`).
    /// Only applies when the previous iteration hit an obstacle's top
    /// plane; ground bounces use the gravity-chained termination
    /// (`new_vz <= 0` after a Ground bounce), and wall bounces never
    /// terminate at the top of the loop.
    const BOUNCE_TOP_MIN_SPEED: f32 = 5.0;

    let fg = GRAVITY * mass;
    let mut trajectory = Vec::new();
    let mut terminal_obstacle = None;
    let mut terminal_impact = false;
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
            velocity = WorldVec3D {
                x: velocity.x * bh,
                y: velocity.y * bh,
                z: -velocity.z * bv,
            };
            position.z = 0.0;
            last_impact = Some(ImpactInfo {
                kind: ImpactKind::Ground,
                normal: WorldVec3D {
                    x: 0.0,
                    y: 0.0,
                    z: 1.0,
                },
                obstacle_bounce_v: 1.0,
                obstacle_bounce_h: 1.0,
            });
            continue;
        }

        let new_position = WorldPoint3D {
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
                crate::coordinates::WorldPoint3D {
                    x: position.x,
                    y: position.y,
                    z: position.z,
                },
                crate::coordinates::WorldPoint3D {
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
            let impact = WorldPoint3D {
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
                    let map_pt = MapPoint::from_world_xyz(impact.x, impact.y, impact.z);
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
            terminal_obstacle = r
                .obstacle_index
                .and_then(|index| u16::try_from(index).ok())
                .and_then(crate::position_interface::ObstacleHandle::new);
            terminal_impact = true;
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
        let landing_map = MapPoint::from_world_xyz(last.x, last.y, last.z);
        let prev_map = MapPoint::from_world_xyz(prev.x, prev.y, prev.z);
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
                position: WorldPoint3D {
                    x: exit.x,
                    y: crate::coordinates::GroundPoint::from_map_and_z(exit, last.z).y,
                    z: last.z,
                },
                time,
            });
        }
    }

    (trajectory, terminal_obstacle, terminal_impact)
}

// ═══════════════════════════════════════════════════════════════════
//  Bow point — hand anchor offset
// ═══════════════════════════════════════════════════════════════════

/// Compute the 3D launch point for an arrow.
///
/// Takes the shooter's 3D entity position (`Entity::element_data().position`)
/// where `.z` is the ground elevation and `.y` already includes elevation.
///
/// `sprite_hand_point`: projected map position of the hand anchor, computed as
/// `sprite_position + hotspot_offset` by the caller.
///
/// For down shots, the bow point is shifted laterally by 20 units along
/// the facing direction vector.
pub fn compute_bow_point(
    position: WorldPoint3D,
    shoot_mode: ShootMode,
    direction: i16,
    sprite_hand_point: MapPoint,
) -> WorldPoint3D {
    let elevation = position.z;

    // Isometric projection: elevation shifts the sprite upward on screen,
    // so add it into the hand Y.
    let hand_y = sprite_hand_point.y + elevation;

    match shoot_mode {
        ShootMode::Long => WorldPoint3D {
            x: sprite_hand_point.x,
            y: hand_y,
            z: elevation + BOW_Z_OFFSET_LONG,
        },
        ShootMode::Normal => WorldPoint3D {
            x: sprite_hand_point.x,
            y: hand_y,
            z: elevation + BOW_Z_OFFSET_NORMAL,
        },
        ShootMode::Down => {
            // Leaning-out soldiers shift the bow point by 20 units
            // along RHElement::GetDirectionVector(), whose Y component is
            // scaled by the isometric aspect ratio.
            let [dx, dy] = crate::position_interface::sector_to_vector_iso(direction);
            WorldPoint3D {
                x: sprite_hand_point.x + dx * 20.0,
                y: hand_y + dy * 20.0,
                z: elevation + BOW_Z_OFFSET_NORMAL,
            }
        }
    }
}

/// Top-plane coefficients of a trajectory's terminal obstacle, resolved
/// against the list the trajectory builder searched.
pub fn terminal_obstacle_plane(
    obstacle: Option<crate::position_interface::ObstacleHandle>,
    sight_obstacles: crate::sight_obstacle::ObstacleList<'_>,
) -> Option<crate::position_interface::PlaneZCoeffs> {
    obstacle.map(|handle| {
        let index = usize::from(u16::from(handle));
        let obstacle = sight_obstacles.get(index).unwrap_or_else(|| {
            panic!("projectile terminal obstacle {index} is absent from its source list")
        });
        crate::position_interface::PlaneZCoeffs::from_plane_points(&obstacle.top_plane_points)
    })
}

/// Bind the obstacle a freshly computed trajectory terminates on.
///
/// The arc builder resolves the impact obstacle while integrating the
/// trajectory — long before the projectile has flown a single frame — and
/// stores it on the projectile. A ground or motion-line impact stores
/// nothing, which is exactly the signal the landing code needs to tell
/// "came to rest on the ground" from "stuck in / on a piece of geometry".
///
/// Binding an obstacle also binds its top plane, which would otherwise pull
/// the projectile's 3D point down onto that plane; the trajectory's own 3D
/// point stays authoritative, so it is re-asserted afterwards.
pub fn bind_trajectory_obstacle(
    element: &mut ElementData,
    obstacle: Option<crate::position_interface::ObstacleHandle>,
    plane: Option<crate::position_interface::PlaneZCoeffs>,
) {
    let position = element.position();
    element.set_obstacle_index(obstacle, plane);
    element.set_position(position);
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
    if resolution.sector.is_some() && !resolution.blocked_by_motion_obstacle {
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
pub fn roll_hit_and_compute_bias(
    sim: &crate::sim_rng::SimulationContext,
    hit_chance: u32,
    bow_skill_capacity: u32,
) -> Option<WorldVec3D> {
    let roll: u32 = crate::sim_rng::u32(sim, crate::sim_rng::RngSite::BowAccuracy, 1..=100);

    if roll <= hit_chance {
        // Hit!
        return None;
    }

    // Miss — compute random bias, scaled by inverse skill.
    let skill_factor = 1.0 - (bow_skill_capacity.min(100) as f32 / 100.0);
    // Original spells this as `SBGeoVector3D(rand(), rand(), rand())`.
    // The shipped/compiler-supported builds evaluate those constructor
    // arguments right-to-left: the first global-stream draw becomes Z and
    // the last becomes X. Keep the draw order visible here because swapping
    // it rotates every inaccurate shot while leaving the RNG cursor aligned.
    let bz = (crate::sim_rng::u32(sim, crate::sim_rng::RngSite::BowAccuracy, 0..5) as f32 - 2.0)
        * skill_factor;
    let by = (crate::sim_rng::u32(sim, crate::sim_rng::RngSite::BowAccuracy, 0..5) as f32 - 2.0)
        * skill_factor;
    let bx = (crate::sim_rng::u32(sim, crate::sim_rng::RngSite::BowAccuracy, 0..5) as f32 - 2.0)
        * skill_factor;

    Some(WorldVec3D {
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
pub fn will_hit_target(
    trajectory: &[TrajectoryPoint],
    _start: WorldPoint3D,
    target: WorldPoint3D,
) -> bool {
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
    bow_point: WorldPoint3D,
    target_point: WorldPoint3D,
    shoot_mode: ShootMode,
    target_forecasted_movement: Option<WorldVec3D>,
) -> (WorldVec3D, u16, f32) {
    let to_target = WorldVec3D {
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

fn bow_target_ground_position(entity: &Entity) -> MapPoint {
    if entity.is_fx_target() {
        entity
            .compute_target_center()
            .map(|pos| MapPoint { x: pos.x, y: pos.y })
            .unwrap_or_else(|| {
                let pos = entity.element_data().position();
                MapPoint { x: pos.x, y: pos.y }
            })
    } else if entity.is_human() {
        entity
            .compute_belt_point()
            .map(|pos| MapPoint { x: pos.x, y: pos.y })
            .unwrap_or_else(|| {
                let pos = entity.element_data().position();
                MapPoint { x: pos.x, y: pos.y }
            })
    } else {
        let pos = entity.element_data().position();
        MapPoint { x: pos.x, y: pos.y }
    }
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
    /// Shooter or target no longer valid (inactive, despawned, wrong kind).
    /// The sequence element should be marked `Impossible`.
    Impossible,
}

/// Forget Rust's execution-side bow latch before retranslating the same
/// postponed Original sequence element.
///
/// C++ stores the active shot entirely in the selected sequence element and
/// its current order. When an injury postpones that element, resuming it calls
/// `Instruct`/`Translate` again with no separate "shot already active" state.
/// Rust needs the separate [`ActiveShot`] driver while an order is executing,
/// but that driver must not reject re-instruction of its own postponed owner.
pub(crate) fn clear_matching_retranslated_shot(
    entities: &mut Entities,
    owner: EntityId,
    seq_id: SequenceId,
    elem_idx: usize,
) {
    let actor = entities
        .get_mut(owner)
        .unwrap_or_else(|| panic!("postponed bow owner {owner:?} disappeared"))
        .actor_data_mut()
        .unwrap_or_else(|| panic!("postponed bow owner {owner:?} has no actor data"));
    if actor.active_shot.sequence_id == Some(seq_id) && actor.active_shot.element_index == elem_idx
    {
        actor.active_shot.clear();
    }
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
    entities: &mut Entities,
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
    // Validate target: must exist, be active/shootable, and not be the shooter.
    // A human may have died since aiming began. The original still launches
    // the queued shot at that retained target rather than rejecting the
    // interaction and sending AI through its lost-enemy path.
    if shooter_id == target_id {
        return BeginShotResult::Impossible;
    }
    let target_valid = match entities.get(target_id) {
        Some(e) if e.is_human() => e.is_active(),
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
    let (tx, ty) = match entities.get(target_id) {
        Some(e) => {
            let position = bow_target_ground_position(e);
            (position.x, position.y)
        }
        None => return BeginShotResult::Impossible,
    };

    // Validate shooter.  Read posture before the mutable borrow.
    let (shooter_valid, shooter_posture, current_state) = match entities.get(shooter_id) {
        Some(e) if e.is_human() && !e.is_dead() => {
            let posture = e.element_data().posture;
            let Some(actor) = e.actor_data() else {
                tracing::warn!(
                    shooter = ?shooter_id,
                    "Begin bow shot rejected: human shooter missing actor data"
                );
                return BeginShotResult::Impossible;
            };
            if actor.active_shot.is_active() {
                (false, posture, ActionState::Waiting)
            } else {
                (true, posture, actor.action_state)
            }
        }
        _ => return BeginShotResult::Impossible,
    };
    if !shooter_valid {
        return BeginShotResult::Impossible;
    }

    let shooter = match entities.get_mut(shooter_id) {
        Some(e) => e,
        None => return BeginShotResult::Impossible,
    };
    let actor = match shooter.actor_data_mut() {
        Some(a) => a,
        None => return BeginShotResult::Impossible,
    };

    // C++ `RHElementActorHuman::Translate(RHCOMMAND_SHOOT_BOW)` chooses
    // raise/lower setup from the sequence element's
    // ActionStateAfterTransition, not from the actor's live state.  That
    // matters when the same element already queued equip/load orders:
    // the live state is still Waiting, but the shoot body must see
    // AimingWithBow and add a raise order for a first long shot.
    let action_state_after_transition = sequence_manager
        .get_element(seq_id, elem_idx)
        .map(|elem| elem.action_state_after_transition)
        .unwrap_or(current_state);

    // Determine the desired shoot mode.  The engine resolves the mode
    // up front via `can_shoot_with_bow_at` and passes it in; we
    // override for leaning-out, then fall back to the post-transition
    // bow attitude or the current action state.
    let desired_mode = if shooter_posture == Posture::LeaningOut {
        ShootMode::Down
    } else if let Some(mode) = resolved_shoot_mode {
        mode
    } else if action_state_after_transition.is_bow() {
        shoot_mode_from_action_state(action_state_after_transition)
    } else if current_state.is_bow() {
        shoot_mode_from_action_state(current_state)
    } else {
        ShootMode::Normal
    };

    let order_id = crate::order::alloc_order_id(next_order_id);
    actor.active_shot = ActiveShot {
        sequence_id: Some(seq_id),
        element_index: elem_idx,
        target: Some(target_id),
        order_id: Some(order_id),
        released: false,
        shoot_mode: Some(desired_mode),
    };
    actor.clear_path();

    // Push aim-transition orders if needed.  Orders live on the owning
    // `SequenceElement.orders` — when the element is cancelled, its
    // orders go with it.
    let _ = actor; // actor mutable borrow ends here; below we borrow sequence_manager instead
    let anonymous = shooter_posture == Posture::AnonymousArcher;
    let transitions = aim_transition_orders(action_state_after_transition, desired_mode, anonymous);
    for t in &transitions {
        let mut order = Order::new(*t, tx, ty, crate::order::alloc_order_id(next_order_id));
        order.compute_direction = false;
        sequence_manager.push_order_on(seq_id, elem_idx, order);
    }

    // Push the shoot animation order.
    let shoot_ot = shoot_order_type_for_mode(desired_mode, anonymous);
    let mut order = Order::new(shoot_ot, tx, ty, order_id);
    order.target_actor = Some(target_id.index());
    order.compute_direction = false;

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
    pub shooter_position: WorldPoint3D,
    /// Target's 2D map position (for arrow direction / spawn).
    pub target_pos: MapPoint,
    /// Target's 3D belt point (for trajectory computation).
    pub target_point: WorldPoint3D,
    /// Shoot mode selected when the command was translated.
    pub shoot_mode: ShootMode,
    /// Shooter's facing direction (0–15) for bow-point computation.
    pub shooter_direction: i16,
    /// Projected sprite hand anchor point (sprite position + hotspot).
    pub sprite_hand_point: MapPoint,
    /// Target's forecasted movement vector for leading shots.
    pub target_forecasted_movement: WorldVec3D,
}

struct PendingShotTickResult {
    shooter: EntityId,
    target: EntityId,
    seq_id: SequenceId,
    elem_idx: usize,
    shooter_position: WorldPoint3D,
    shoot_mode: ShootMode,
    shooter_direction: i16,
    sprite_hand_point: MapPoint,
}

#[derive(Default)]
pub struct BowTickEvents {
    pub fired: Vec<ShotTickResult>,
    pub completed: Vec<(SequenceId, usize)>,
}

#[cfg(test)]
thread_local! {
    static CROSS_ACTOR_SHOT_REPLACEMENT: std::cell::Cell<Option<(EntityId, ActiveShot)>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn apply_cross_actor_shot_replacement(entities: &mut Entities) {
    let Some((actor_id, replacement)) = CROSS_ACTOR_SHOT_REPLACEMENT.take() else {
        return;
    };
    entities
        .get_mut(actor_id)
        .and_then(Entity::actor_data_mut)
        .expect("cross-actor replacement target must remain an actor")
        .active_shot = replacement;
}

/// Advance the shoot animation for every actor with an [`ActiveShot`].
///
/// Returns a list of results for actors whose shoot animation reached
/// `MotionState::Done` this frame — the engine computes the trajectory,
/// spawns an arrow, and notifies the sequence manager for each.  When
/// the shoot animation completes the actor returns to the
/// AimingWithBow action state.
pub fn tick_bow_shots(
    sim: &crate::sim_rng::SimulationContext,
    entities: &mut Entities,
    sequence_manager: &mut SequenceManager,
) -> BowTickEvents {
    tick_bow_shots_matching(sim, entities, sequence_manager, None, None, false)
}

pub fn tick_bow_shot_for_owner(
    sim: &crate::sim_rng::SimulationContext,
    entities: &mut Entities,
    sequence_manager: &mut SequenceManager,
    owner: EntityId,
    expected_order_id: std::num::NonZeroU32,
    sprite_frozen: bool,
) -> BowTickEvents {
    tick_bow_shots_matching(
        sim,
        entities,
        sequence_manager,
        Some(owner),
        Some(expected_order_id),
        sprite_frozen,
    )
}

fn tick_bow_shots_matching(
    sim: &crate::sim_rng::SimulationContext,
    entities: &mut Entities,
    sequence_manager: &mut SequenceManager,
    only_owner: Option<EntityId>,
    expected_order_id: Option<std::num::NonZeroU32>,
    sprite_frozen: bool,
) -> BowTickEvents {
    let mut events = BowTickEvents::default();
    let mut pending_fired = Vec::new();
    let mut target_ground_positions = EntitySlots::filled(entities.len(), None);
    let mut target_map_positions = EntitySlots::filled(entities.len(), None);
    for (entity_id, entity) in entities.occupied() {
        target_ground_positions[entity_id] = Some(bow_target_ground_position(entity));
        target_map_positions[entity_id] = Some(entity.element_data().position_map());
    }

    #[cfg(test)]
    apply_cross_actor_shot_replacement(entities);

    for (actor_id, entity) in entities.actors_mut() {
        let shooter_id: EntityId = actor_id.into();
        if only_owner.is_some_and(|owner| owner != shooter_id) {
            continue;
        }
        let actor = match entity.actor_data() {
            Some(a) => a,
            None => continue,
        };
        if !actor.active_shot.is_active() {
            continue;
        }
        let shot = actor.active_shot;
        let order_initialising = actor.execute_order_initialising;
        let direction = entity.element_data().direction();
        // Build 3D shooter position: X/Y from the live map position,
        // Z from the position interface's elevation.
        // The element_data().position field is a dead field; the live
        // ground position is in position_map.
        let elevation = entity.position_iface().get_elevation();
        let shooter_position = WorldPoint3D {
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
        if expected_order_id.is_some() && expected_order_id != current_order_id {
            continue;
        }
        if !is_active_bow_order(current_order_type) {
            let bow_order_pending = sequence_manager
                .get_element(shot_seq_id, shot_elem_idx)
                .map(has_active_bow_order)
                .unwrap_or(false);
            if bow_order_pending {
                // C++ `Translate(SHOOT_BOW)` appends bow orders after
                // any pre-command setup transitions already owned by the
                // same sequence element. The active shot has been
                // registered, but the actor must finish those setup orders
                // before the bow runner starts driving the shoot body.
                continue;
            }
            tracing::debug!(
                shooter = shooter_id.index(),
                ?shot_seq_id,
                shot_elem_idx,
                ?current_order_type,
                "Bow shot driver detached after sequence advanced past bow orders"
            );
            entity.actor_data_mut().unwrap().active_shot.clear();
            continue;
        }

        let mut direction = direction;
        let mut frame_progression = crate::sprite::FrameProgression::Default;
        if is_shoot_order(current_order_type) {
            // Original samples the live target only when the shooting order
            // initializes. Translation and bow-preparation transitions retain
            // the actor's old facing, and later target movement does not
            // rewrite this shot's goal.
            if order_initialising {
                let target_id = shot.target.unwrap_or_else(|| {
                    panic!("active bow shot for {shooter_id:?} has no target at initialization")
                });
                // The soldier override for a downward leaning-out shot uses
                // GetPositionMap on both participants. Ordinary and high bow
                // shots remain in the Human arm, which uses PositionGround.
                let leaning_out = current_order_type == OrderType::ShootingWithBowLeaningOut;
                let target_pos = (if leaning_out {
                    target_map_positions.get(target_id)
                } else {
                    target_ground_positions.get(target_id)
                })
                .and_then(|position| *position)
                .unwrap_or_else(|| {
                    panic!(
                        "active bow shot for {shooter_id:?} lost target {target_id:?} at initialization"
                    )
                });
                let shooter_pos = if leaning_out {
                    entity.element_data().position_map()
                } else {
                    let position = entity.element_data().position();
                    MapPoint::new(position.x, position.y)
                };
                let dx = target_pos.x - shooter_pos.x;
                let dy = target_pos.y - shooter_pos.y;
                entity.element_data_mut().set_direction_goal(
                    crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy),
                );
            }
            if entity.element_data_mut().sprite.position_iface.turn() {
                frame_progression = crate::sprite::FrameProgression::FrozenFirstFrame;
            }
            direction = entity.element_data().direction();
        }
        let Ok(dir_u16) = u16::try_from(direction) else {
            tracing::warn!(
                shooter = shooter_id.index(),
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
        let motion = if sprite_frozen {
            SpriteMotionState::InProgress
        } else if elem.sprite.scripts.is_empty() {
            if frame_progression == crate::sprite::FrameProgression::FrozenFirstFrame {
                SpriteMotionState::InProgress
            } else {
                SpriteMotionState::Done
            }
        } else {
            elem.sprite.perform_action(
                sim,
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
                let (remaining, bow_remaining, installed_order) = if let Some(elem) =
                    sequence_manager.get_element_mut(shot_seq_id, shot_elem_idx)
                {
                    elem.orders.pop_front();
                    let installed_order =
                        elem.current_order()
                            .map(|order| crate::element::InstalledActorOrder {
                                order_id: order.order_id,
                                order_type: order.order_type,
                            });
                    (
                        elem.orders.is_empty(),
                        has_active_bow_order(elem),
                        installed_order,
                    )
                } else {
                    (true, false, None)
                };
                // Actor::Hourglass routes a TERMINATED bow Execute through
                // DoNextOrder, whose Proceed() result immediately replaces
                // mpOrder.  This specialized driver pops the same queue
                // directly, so publish that exact successor (or NULL) here.
                actor.installed_order = installed_order;
                if remaining || !bow_remaining {
                    actor.active_shot.clear();
                }
                if remaining {
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
                let (remaining, bow_remaining, installed_order) = if let Some(elem) =
                    sequence_manager.get_element_mut(shot_seq_id, shot_elem_idx)
                {
                    elem.orders.pop_front();
                    let installed_order =
                        elem.current_order()
                            .map(|order| crate::element::InstalledActorOrder {
                                order_id: order.order_id,
                                order_type: order.order_type,
                            });
                    (
                        elem.orders.is_empty(),
                        has_active_bow_order(elem),
                        installed_order,
                    )
                } else {
                    (true, false, None)
                };
                // See the transition branch above: direct retirement must
                // mirror DoNextOrder's synchronous mpOrder publication.
                actor.installed_order = installed_order;
                if remaining || !bow_remaining {
                    actor.active_shot.clear();
                }
                if remaining {
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
            let Some(shot_mode) = shot.shoot_mode else {
                panic!(
                    "active bow shot missing resolved shoot mode at release: shooter={} seq_id={shot_seq_id:?} elem_idx={shot_elem_idx} order={current_order_type:?}",
                    shooter_id.index()
                );
            };
            actor.action_state = ActionState::AimingWithBow;
            if current_order_type == OrderType::ShootingWithBowLeaningOut {
                entity.element_data_mut().posture = Posture::LeaningOut;
            } else if entity.element_data().posture != Posture::AnonymousArcher {
                entity.element_data_mut().posture = Posture::Upright;
            }

            let Some(sprite_hand_point) = bow_sprite_hand_point(entity, shot_mode, direction)
            else {
                tracing::warn!(
                    shooter = shooter_id.index(),
                    ?shot_mode,
                    dir_u16,
                    "Bow release skipped: missing bow-point sprite hotspot"
                );
                continue;
            };

            let Some(target) = shot.target else {
                tracing::warn!(
                    shooter = shooter_id.index(),
                    "Bow release skipped: active shot missing target"
                );
                continue;
            };

            pending_fired.push(PendingShotTickResult {
                shooter: shooter_id,
                target,
                seq_id: shot_seq_id,
                elem_idx: shot.element_index,
                shooter_position,
                shoot_mode: shot_mode,
                shooter_direction: direction,
                sprite_hand_point,
            });
            continue;
        }

        panic!(
            "active bow shot reached unhandled active bow order: shooter={} seq_id={shot_seq_id:?} elem_idx={shot_elem_idx} order={current_order_type:?}",
            shooter_id.index()
        );
    }

    // Resolve target positions, 3D body points and forecasted movement (immutable re-borrow).
    for result in pending_fired {
        let Some(target_entity) = entities.get(result.target) else {
            tracing::warn!(
                shooter = ?result.shooter,
                target = ?result.target,
                "Bow release skipped: target entity missing"
            );
            continue;
        };
        let target_pos = target_entity.element_data().position_map();
        let target_point = if target_entity.is_human() {
            let Some(point) = target_entity.compute_belt_point() else {
                tracing::warn!(
                    shooter = ?result.shooter,
                    target = ?result.target,
                    "Bow release skipped: human target missing belt hotspot"
                );
                continue;
            };
            point
        } else if target_entity.is_fx_target() {
            let Some(point) = target_entity.compute_target_center() else {
                tracing::warn!(
                    shooter = ?result.shooter,
                    target = ?result.target,
                    "Bow release skipped: FX target missing center hotspot"
                );
                continue;
            };
            point
        } else {
            tracing::warn!(
                shooter = ?result.shooter,
                target = ?result.target,
                kind = ?target_entity.kind(),
                "Bow release skipped: unsupported target kind"
            );
            continue;
        };
        let target_forecasted_movement = target_entity.position_iface().get_forecasted_movement();
        events.fired.push(ShotTickResult {
            shooter: result.shooter,
            target: result.target,
            seq_id: result.seq_id,
            elem_idx: result.elem_idx,
            shooter_position: result.shooter_position,
            target_pos,
            target_point,
            shoot_mode: result.shoot_mode,
            shooter_direction: result.shooter_direction,
            sprite_hand_point: result.sprite_hand_point,
            target_forecasted_movement,
        });
    }

    events
}

// ═══════════════════════════════════════════════════════════════════
//  Arrow spawn
// ═══════════════════════════════════════════════════════════════════

/// Parameters for spawning an arrow projectile.
pub struct SpawnArrowParams {
    pub shooter: EntityId,
    pub bow_point: WorldPoint3D,
    /// C++ `ShootWithBowAt` stores the shooter map position as
    /// `mposStartOfTrajectory` after the arrow's first `Hourglass()`.
    /// AI reactions use this origin, not the bow hand hotspot.
    pub trajectory_origin: MapPoint,
    pub target: EntityId,
    pub target_pos: MapPoint,
    pub trajectory: Vec<TrajectoryPoint>,
    pub damage: u16,
    pub layer: u16,
    /// Initial 3D velocity — `compute_initial_throw_velocity` output
    /// (after any target-leading correction).  The sprite facing is
    /// seeded from the XY of this vector, not from `target - bow` —
    /// the two diverge once leading is applied to moving targets.
    ///
    pub initial_velocity: WorldVec3D,
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
        target_pos: _,
        trajectory,
        damage,
        layer: _trajectory_layer,
        lands_in_hole,
        initial_velocity,
    } = params;
    let map_pos = MapPoint {
        x: bow_point.x,
        y: bow_point.y,
    };
    let end_pos = trajectory_end_or_start(&trajectory, bow_point, "arrow");

    let mut element = ElementData {
        kind: ElementKind::ObjectProjectile,
        active: true,
        posture: Posture::Undefined,
        ..ElementData::default()
    };
    element.set_position_map(map_pos);
    element.set_position(bow_point);
    // ComputeTrajectory detaches a projectile from world membership before
    // constructing its flight: layer = 0xFFFF, sector = NULL, obstacle =
    // NULL. It only restores a layer when the resolved landing point belongs
    // to a motion sector on that layer.
    element.clear_layer();
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
    arrow.projectile.flight_direction = crate::position_interface::vector_to_sector_0_to_15_iso(
        initial_velocity.x,
        initial_velocity.y,
    ) as u16;
    arrow.projectile.launch_segment_start = Some(bow_point);
    Entity::Projectile(arrow)
}

fn trajectory_end_or_start(
    trajectory: &[TrajectoryPoint],
    start: WorldPoint3D,
    projectile_kind: &'static str,
) -> WorldPoint3D {
    match trajectory.last() {
        Some(tp) => tp.position,
        None => {
            tracing::warn!(
                projectile_kind,
                ?start,
                "projectile spawn produced empty trajectory; keeping end at start"
            );
            start
        }
    }
}

/// Spawn a net projectile entity flying toward `target_pos`.
///
/// Creates an `Entity::Net` with a precomputed ballistic trajectory
/// using `MASS_NET` / `APEX_NET`.
pub fn spawn_net(
    thrower: EntityId,
    throw_pos: WorldPoint3D,
    target_pos: WorldPoint3D,
    layer: u16,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
) -> Entity {
    let dx = target_pos.x - throw_pos.x;
    let dy = target_pos.y - throw_pos.y;
    let dz = target_pos.z - throw_pos.z;
    let direction_vec = WorldVec3D {
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
    let end_pos = trajectory_end_or_start(&trajectory, throw_pos, "net");

    let map_pos = MapPoint {
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
    let time_till_unfolding = total_trajectory_frames.saturating_sub(15).max(1) as u32;

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
    throw_pos: WorldPoint3D,
    target_pos: WorldPoint3D,
    layer: u16,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
) -> Entity {
    let dx = target_pos.x - throw_pos.x;
    let dy = target_pos.y - throw_pos.y;
    let dz = target_pos.z - throw_pos.z;
    let direction_vec = WorldVec3D {
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
    let end_pos = trajectory_end_or_start(&trajectory, throw_pos, "wasp_nest");

    let map_pos = MapPoint {
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
pub fn spawn_wasp(nest_id: EntityId, position: WorldPoint3D, layer: u16) -> Entity {
    let mut element = ElementData {
        kind: ElementKind::ObjectProjectile,
        active: true,
        posture: Posture::Undefined,
        ..ElementData::default()
    };
    element.set_position_map(MapPoint::from_world_xyz(position.x, position.y, position.z));
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
    throw_pos: WorldPoint3D,
    target_pos: WorldPoint3D,
    target: Option<EntityId>,
    target_forecasted_movement: Option<WorldVec3D>,
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
    throw_pos: WorldPoint3D,
    target_pos: WorldPoint3D,
    target: Option<EntityId>,
    target_forecasted_movement: Option<WorldVec3D>,
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
    throw_pos: WorldPoint3D,
    target_pos: WorldPoint3D,
    target: Option<EntityId>,
    target_forecasted_movement: Option<WorldVec3D>,
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
    let direction_vec = WorldVec3D {
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
    let (trajectory, terminal_obstacle) = compute_trajectory_ballistic_with_terminal_obstacle(
        throw_pos,
        velocity,
        mass,
        false,
        obstacle_check,
    );
    let end_pos = trajectory_end_or_start(&trajectory, throw_pos, "throwable");

    let map_pos = MapPoint {
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
    if let Some(check) = obstacle_check {
        let plane = terminal_obstacle_plane(terminal_obstacle, check.sight_obstacles);
        bind_trajectory_obstacle(&mut element, terminal_obstacle, plane);
    }
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
    throw_pos: WorldPoint3D,
    target_pos: WorldPoint3D,
    layer: u16,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
) -> Entity {
    let dx = target_pos.x - throw_pos.x;
    let dy = target_pos.y - throw_pos.y;
    let dz = target_pos.z - throw_pos.z;
    let direction_vec = WorldVec3D {
        x: dx,
        y: dy,
        z: dz,
    };

    let velocity = compute_initial_throw_velocity(direction_vec, APEX_PURSE, MASS_PURSE, 0, None);
    let (trajectory, terminal_obstacle) = compute_trajectory_ballistic_with_terminal_obstacle(
        throw_pos,
        velocity,
        MASS_PURSE,
        false,
        obstacle_check,
    );
    let end_pos = trajectory_end_or_start(&trajectory, throw_pos, "purse");

    let map_pos = MapPoint {
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
    if let Some(check) = obstacle_check {
        let plane = terminal_obstacle_plane(terminal_obstacle, check.sight_obstacles);
        bind_trajectory_obstacle(&mut element, terminal_obstacle, plane);
    }
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
    source_pos: WorldPoint3D,
    target_pos: WorldPoint3D,
    layer: u16,
    layer_goal: u16,
    sector_goal: Option<crate::position_interface::SectorHandle>,
    apex: f32,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
) -> Entity {
    let dx = target_pos.x - source_pos.x;
    let dy = target_pos.y - source_pos.y;
    let dz = target_pos.z - source_pos.z;
    let direction_vec = WorldVec3D {
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
    let end_pos = trajectory_end_or_start(&trajectory, source_pos, "coin");

    let map_pos = MapPoint {
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
    pub impact_pos: MapPoint,
    /// Previous 3D projectile position for C++ `SetPosition(old)` on
    /// human hits whose `HitHuman` handler returns true.  Arrows only
    /// use this in the damage branch; pass-through and ricochet keep
    /// their current position/falling trajectory.
    pub human_hit_old_position: Option<WorldPoint3D>,
}

/// Original `RHElementArrow::GetOrientations` points from the arrow's current
/// position to the *next queued trajectory point*. The current per-frame
/// increment is not equivalent: `Hourglass` has already removed the point
/// which produced that increment, and the next ballistic segment can have a
/// different vertical pitch.
fn current_arrow_orientation(proj: &mut ElementProjectile) -> (u16, i16) {
    let Some(next) = proj.projectile.trajectory.first() else {
        return (
            proj.projectile.last_orientation_sector,
            proj.projectile.last_orientation_azimuth,
        );
    };
    let current = proj.element.position();
    let dx = next.position.x - current.x;
    let dy = next.position.y - current.y;
    let dz = next.position.z - current.z;
    let norm_sq = dx * dx + dy * dy + dz * dz;
    if norm_sq == 0.0 {
        return (
            proj.projectile.last_orientation_sector,
            proj.projectile.last_orientation_azimuth,
        );
    }

    let inv_norm = 1.0 / norm_sq.sqrt();
    let nx = dx * inv_norm;
    let ny = dy * inv_norm;
    let nz = dz * inv_norm;
    let sector = crate::position_interface::vector_to_sector_0_to_15_iso(nx, ny) as u16 & 15;
    let ground_norm = (nx * nx + ny * ny).sqrt().min(1.0);
    let mut azimuth = (ground_norm.acos() * 180.0 / std::f32::consts::PI).min(60.0) as i16;
    if nz < 0.0 {
        azimuth = -azimuth;
    }
    proj.projectile.last_orientation_sector = sector;
    proj.projectile.last_orientation_azimuth = azimuth;
    (sector, azimuth)
}

fn apply_arrow_falling_sprite_visual(
    sim: &crate::sim_rng::SimulationContext,
    proj: &mut ElementProjectile,
) {
    // C++ `RHElementArrow::Refresh` renders falling arrows with
    // `ForceSpriteRow(mubFallingDirection)`,
    // `ForceSprite(mubFallingDirection, (rand() % 3) + 3)`, then rotates
    // the row by -2 sectors for the next refresh.
    let row = proj.projectile.falling_direction;
    let frame =
        (crate::sim_rng::u32(sim, crate::sim_rng::RngSite::ArrowFallingFrame, 0..3) as u16) + 3;
    proj.element.sprite.force_sprite_row_raw(row);
    proj.element.sprite.force_sprite(row, frame);
    proj.projectile.falling_direction = (row + 14) % 16;
}

/// Apply the presentation pass which Original runs after the parity snapshot.
/// The engine calls this immediately before the next `PerformHourglass`, which
/// exposes the same row/frame at the next snapshot and puts falling-arrow RNG
/// before that frame's simulation draws.
pub(crate) fn refresh_arrow_after_previous_hourglass(
    sim: &crate::sim_rng::SimulationContext,
    proj: &mut ElementProjectile,
) {
    if !proj.element.active {
        return;
    }
    let trajectory_empty = proj.projectile.trajectory.is_empty();
    let flight_at_endpoint = proj.projectile.trajectory_frame_count == 0;
    let world_position_is_moving = proj.element.sprite.position_iface.is_moving();
    let retire_loaded_stopped = !proj.projectile.flying && !flight_at_endpoint;
    let retire_live_stopped =
        !proj.projectile.flying && flight_at_endpoint && !world_position_is_moving;
    let retire_live_flying = proj.projectile.flying
        && proj.projectile.falling
        && flight_at_endpoint
        && !proj
            .element
            .sprite
            .position_iface
            .raw_sprite_position_is_moving();
    if trajectory_empty && (retire_loaded_stopped || retire_live_stopped || retire_live_flying) {
        // The endpoint frame itself is still presented once. Falling arrows
        // therefore consume their final tumble draw before the settled cache
        // retires them; an already-stopped loaded arrow retires immediately.
        if proj.projectile.flying && proj.projectile.falling {
            apply_arrow_falling_sprite_visual(sim, proj);
        }
        // Refresh retires the arrow before another Projectile::Hourglass can
        // call NewMove. Settle the exposed movement snapshot at the endpoint
        // as Original's retired sprite state records it.
        proj.element.sprite.position_iface.new_move();
        proj.element.active = false;
        return;
    }

    if trajectory_empty && !proj.projectile.flying && flight_at_endpoint {
        // Projectile::Hourglass calls NewMove even after flight stops. Rust's
        // landed-projectile tick is otherwise skipped at that point, so cross
        // the same movement-snapshot boundary here before the next Refresh.
        proj.element.sprite.position_iface.new_move();
    }

    if proj.projectile.falling {
        apply_arrow_falling_sprite_visual(sim, proj);
    } else {
        let (sector, azimuth) = current_arrow_orientation(proj);
        let frame = ((azimuth as f32 * 0.066_666_67_f32 + 0.5_f32) as i32 + 4) as u16;
        proj.element.sprite.force_sprite_row_raw(sector);
        proj.element.sprite.force_sprite(sector, frame);
    }
}

pub(crate) fn make_arrow_falling_down(
    _sim: &crate::sim_rng::SimulationContext,
    proj: &mut ElementProjectile,
    thrown_away_by_shield: bool,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
) {
    let (sector, _) = current_arrow_orientation(proj);
    proj.projectile.falling = true;
    proj.projectile.flying = true;

    let (falling_direction, velocity) = if thrown_away_by_shield {
        let direction = (sector + 4) & 15;
        let (dx, dy) = crate::element::direction_vector_16(
            i16::try_from(direction).expect("arrow direction sector fits in i16"),
        );
        (
            direction as u16,
            WorldVec3D {
                x: dx * 30.0,
                y: dy * ASPECT_RATIO * 30.0,
                z: -20.0,
            },
        )
    } else {
        let direction = sector ^ 8;
        let (dx, dy) = crate::element::direction_vector_16(
            i16::try_from(direction).expect("arrow direction sector fits in i16"),
        );
        (
            direction as u16,
            WorldVec3D {
                x: dx * 30.0,
                y: dy * ASPECT_RATIO * 10.0,
                z: 0.0,
            },
        )
    };

    proj.projectile.falling_direction = falling_direction;
    // The deflection trajectory is integrated through the same
    // solid-sight-obstacle clipping as a launched shot, so a deflected arrow
    // stops at the wall or floor it is thrown into instead of sailing through
    // it. Segment clipping also shortens the first waypoint's frame count,
    // which is directly observable as this tick's movement increment.
    let (trajectory, terminal_obstacle, terminal_impact) =
        compute_trajectory_ballistic_with_terminal_impact(
            proj.element.position(),
            velocity,
            MASS_ARROW_HIGH,
            false,
            obstacle_check,
        );
    proj.projectile.trajectory = trajectory;
    proj.projectile.trajectory_frame_count = 0;
    proj.projectile.launch_segment_start = None;
    if let Some(water_zones) = obstacle_check.and_then(|check| check.water_zones) {
        preserve_falling_hole_disappearance(proj, water_zones);
    }

    // Recomputing a trajectory drops the projectile's current membership and
    // re-derives it from where the new trajectory ends, so the deflected
    // arrow reports the layer and sector it is about to land in for the whole
    // of its fall rather than only once it settles.
    proj.element.clear_layer();
    proj.element.set_sector(None);
    if let Some(check) = obstacle_check {
        let plane = terminal_obstacle_plane(terminal_obstacle, check.sight_obstacles);
        bind_trajectory_obstacle(&mut proj.element, terminal_obstacle, plane);
    }
    if terminal_impact
        && let Some(check) = obstacle_check
        && let Some(end) = proj.projectile.trajectory.last().map(|tp| tp.position)
    {
        let resolution = if let Some(obstacle) = terminal_obstacle {
            check
                .fast_find_grid
                .resolve_projectile_landing_with_obstacle(
                    end.to_map(),
                    Some(obstacle),
                    check.sight_obstacles,
                )
        } else {
            check
                .fast_find_grid
                .resolve_projectile_ground_landing(end.to_map())
        };
        proj.element.set_sector(resolution.sector);
        if resolution.sector.is_some() && !resolution.blocked_by_motion_obstacle {
            proj.element.set_layer(resolution.layer);
        }
    }

    // C++ `RHElementArrow::MakeFallingDown` calls `Hourglass()` after
    // recomputing the trajectory, so the ricochet visibly advances on
    // the same tick as the shield/target impact.  That nested hourglass
    // opens with its own `NewMove`, which re-anchors the old position onto
    // the impact point before the deflection step is applied.
    proj.element.sprite.position_iface.new_move();
    proj.advance_trajectory_one_frame();
}

fn preserve_falling_hole_disappearance(
    proj: &mut ElementProjectile,
    water_zones: &crate::water_zones::WaterZones,
) {
    // `ComputeTrajectory` does not clear `mbDisappear`, and
    // `AddTrajectoryFallIntoHole` sets it before checking whether there are
    // enough waypoints to append the visual far-edge extension. A short
    // ricochet can therefore have only one terminal waypoint and must still
    // disappear silently when that point lies in a hole.
    proj.projectile.disappear |= proj
        .projectile
        .trajectory
        .iter()
        .rev()
        .take(2)
        .any(|point| water_zones.landing_is_in_hole(point.position.to_map()));
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
    sim: &crate::sim_rng::SimulationContext,

    entities: &mut Entities,
    sight_obstacles: crate::sight_obstacle::ObstacleList<'_>,
) -> Vec<ArrowTickResult> {
    tick_arrows_matching(sim, entities, sight_obstacles, None, None, false, &[])
}

/// Advance every projectile except the ones listed in `skip_arrow_ids`.
///
/// Used for bow arrows released from the sequence-manager phase: C++
/// already called `pArrow->Hourglass()` before insertion, and the global
/// element hourglass pass for that frame has already finished.
pub fn tick_arrows_excluding(
    sim: &crate::sim_rng::SimulationContext,

    entities: &mut Entities,
    sight_obstacles: crate::sight_obstacle::ObstacleList<'_>,
    skip_arrow_ids: &[EntityId],
) -> Vec<ArrowTickResult> {
    tick_arrows_matching(
        sim,
        entities,
        sight_obstacles,
        None,
        None,
        false,
        skip_arrow_ids,
    )
}

/// Advance and resolve collision for one active projectile.
///
/// Used immediately after spawning a bow arrow to match C++
/// `ShootWithBowAt`, which calls `pArrow->Hourglass()` before the
/// arrow enters the engine element list.
pub fn tick_arrow(
    sim: &crate::sim_rng::SimulationContext,

    entities: &mut Entities,
    sight_obstacles: crate::sight_obstacle::ObstacleList<'_>,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
    arrow_id: EntityId,
) -> Vec<ArrowTickResult> {
    tick_arrows_matching(
        sim,
        entities,
        sight_obstacles,
        obstacle_check,
        Some(arrow_id),
        true,
        &[],
    )
}

/// Advance one projectile already present in the engine element array.
///
/// Unlike [`tick_arrow`], this does not treat the projectile's spawn-time
/// priming step as its current-frame advancement. It is used by the engine's
/// creation-ordered entity pass so projectile and PC hourglasses can retain
/// their relative element-array order.
pub fn tick_existing_projectile(
    sim: &crate::sim_rng::SimulationContext,

    entities: &mut Entities,
    sight_obstacles: crate::sight_obstacle::ObstacleList<'_>,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
    projectile_id: EntityId,
) -> Vec<ArrowTickResult> {
    tick_arrows_matching(
        sim,
        entities,
        sight_obstacles,
        obstacle_check,
        Some(projectile_id),
        false,
        &[],
    )
}

fn tick_arrows_matching(
    sim: &crate::sim_rng::SimulationContext,

    entities: &mut Entities,
    sight_obstacles: crate::sight_obstacle::ObstacleList<'_>,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
    only_arrow_id: Option<EntityId>,
    primed_segment_already_advanced: bool,
    skip_arrow_ids: &[EntityId],
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
        belt: WorldPoint3D,
        eyes: WorldPoint3D,
        /// True when posture == LeaningOut — arrows get an eye-level
        /// re-check after the belt miss.
        leaning_out: bool,
        is_pc: bool,
        is_soldier: bool,
        is_civilian: bool,
        camp: Option<crate::element::Camp>,
        holding_shield: bool,
        position_map: MapPoint,
    }
    let human_snapshots: Vec<HumanSnapshot> = entities
        .humans()
        .filter_map(|(human_id, e)| {
            let entity_id: EntityId = human_id.into();
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
                    entity = entity_id.index(),
                    "Projectile hit snapshot skipped: human missing belt hotspot"
                );
                return None;
            };
            let Some(eyes) = e.compute_eyes_point(None) else {
                tracing::warn!(
                    entity = entity_id.index(),
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
            let Some(actor) = e.actor_data() else {
                tracing::warn!(
                    entity = entity_id.index(),
                    "Projectile hit snapshot skipped: human missing actor data"
                );
                return None;
            };
            let holding_shield = actor.action_state.is_shield();
            Some(HumanSnapshot {
                id: entity_id,
                belt,
                eyes,
                leaning_out: posture == crate::element::Posture::LeaningOut,
                is_pc: e.is_pc(),
                is_soldier: e.is_soldier(),
                is_civilian: e.is_civilian(),
                camp,
                holding_shield,
                position_map: e.element_data().position_map(),
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
        center: WorldPoint3D,
        position_map: MapPoint,
        action_filter: crate::element::TargetFilter,
    }
    let fx_target_snapshots: Vec<FxTargetSnapshot> = entities
        .targets()
        .filter_map(|(target_id, e)| {
            let entity_id = EntityId::Target(target_id);
            if !e.element.active {
                return None;
            }
            let filter = e.target.action_filter;
            // Projectile-activation filters only — keeps the per-tick
            // inner loop small.
            if !filter.intersects(
                crate::element::TargetFilter::ARROW
                    | crate::element::TargetFilter::APPLE
                    | crate::element::TargetFilter::STONE,
            ) {
                return None;
            }
            let Some(center) = Entity::Target(e.clone()).compute_target_center() else {
                tracing::warn!(
                    entity = entity_id.index(),
                    "Projectile hit snapshot skipped: FX target missing center hotspot"
                );
                return None;
            };
            Some(FxTargetSnapshot {
                id: entity_id,
                center,
                position_map: e.element.position_map(),
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
        .actors()
        .filter_map(|(actor_id, e)| {
            let entity_id = actor_id.into();
            if !e.is_active() || e.is_dead() {
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
                holder_id: entity_id,
                look_dir,
                obstacle,
            })
        })
        .collect();

    tracing::trace!(
        holders = ?shield_snapshots.iter().map(|s| s.holder_id).collect::<Vec<_>>(),
        "Projectile tick shield-holder snapshot"
    );

    for (projectile_id, entity) in entities.projectiles_mut() {
        let idx = projectile_id.0 as usize;
        let arrow_id = EntityId::Projectile(crate::entity_id::ProjectileId(idx as u32));
        if let Some(only_arrow_id) = only_arrow_id
            && only_arrow_id != arrow_id
        {
            continue;
        }
        if skip_arrow_ids.contains(&arrow_id) {
            continue;
        }
        if !entity.element.active {
            continue;
        }
        let proj = entity;
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

        // Grounded Apple/Stone work belongs to the derived virtual owner
        // path. Projectile::Hourglass returns without advancing or removing
        // them; the caller must run the landed sprite tail before applying
        // that saved base result.
        if !proj.projectile.flying {
            continue;
        }
        // RHElementProjectile::Hourglass calls NewMove before advancing its
        // trajectory. Spawned projectiles have already consumed their primer
        // step, so this snapshots that primer position exactly as Original's
        // explicit pre-add Hourglass does.
        proj.element.sprite.position_iface.new_move();

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
        let has_primed_segment = primed_segment_start.is_some();
        let already_advanced_this_segment = has_primed_segment && primed_segment_already_advanced;

        // ── Trajectory advancement ────────────────────────────────

        if has_primed_segment {
            // Spawn primed the first segment. The immediate single-arrow
            // path has already advanced it to match C++ `pArrow->Hourglass()`;
            // the general tick path keeps the old public behavior and applies
            // the stored increment below. Both paths still run collision
            // against `primed_segment_start -> current/new`.
        } else if proj.projectile.trajectory_frame_count == 0 {
            if !proj.projectile.trajectory.is_empty() {
                // Pop the next trajectory waypoint.
                let point = proj.projectile.trajectory.remove(0);
                let time = point.time.max(1);
                proj.projectile.trajectory_frame_count = time - 1;

                // Compute per-frame increment toward this waypoint.
                let current = proj.element.position();
                let factor = 1.0 / time as f32;
                proj.projectile.velocity_increment = WorldVec3D {
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
                // The obstacle is the one the trajectory builder struck
                // when it terminated the arc, bound onto the projectile at
                // launch and carried through the flight. It is emphatically
                // not "whichever projection polygon happens to cover the
                // landing point on screen": an arrow that lands on open
                // ground in front of a building is under that building's
                // projection polygon yet hit no obstacle at all, and gets
                // the flat 0.001 ground snap.
                let pos = proj.element.position();
                let (top_plane_z, new_z) = if proj.projectile.disappear {
                    // Original tests `mbDisappear` before `HitObstacle`.
                    // Reaching a hole therefore preserves the trajectory-end
                    // elevation instead of applying the ordinary +0.001
                    // ground/obstacle snap.
                    (None, None)
                } else {
                    let top_plane_z = proj.element.obstacle_index().map(|handle| {
                        let index = usize::from(u16::from(handle));
                        let obstacle = sight_obstacles.get(index).unwrap_or_else(|| {
                            panic!(
                                "landed projectile obstacle {index} is absent from its source list"
                            )
                        });
                        crate::position_interface::PlaneZCoeffs::from_plane_points(
                            &obstacle.top_plane_points,
                        )
                        .compute_z(pos.x, pos.y)
                    });

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
                    (top_plane_z, new_z)
                };
                tracing::trace!(
                    target: "arrow_landing",
                    arrow = arrow_id.index(),
                    object_type = ?proj.object.object_type,
                    obstacle = ?proj.element.obstacle_index().map(u16::from),
                    layer = proj.element.layer(),
                    ?pos,
                    ?top_plane_z,
                    ?new_z,
                    "projectile landing elevation snap"
                );
                if let Some(z) = new_z {
                    let mut p = proj.element.position();
                    p.z = z;
                    proj.element.set_position(p);
                    proj.element.set_position_map_preserving_3d(p.to_map());
                }
                let impact_pos = proj.element.position_map();
                proj.projectile.flying = false;
                let despawn = if is_burster {
                    set_projectile_animation(proj, Animation::ObjectBursting);
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
                    human_hit_old_position: None,
                });
                continue;
            }
        } else {
            proj.projectile.trajectory_frame_count -= 1;
        }

        if !already_advanced_this_segment {
            // Apply the per-frame increment to position.
            let mut p = proj.element.position();
            p.x += proj.projectile.velocity_increment.x;
            p.y += proj.projectile.velocity_increment.y;
            p.z += proj.projectile.velocity_increment.z;
            proj.element.set_position(p);

            // Update the 2D map position from 3D (project Z onto Y for
            // isometric display: map.y = pos.y - pos.z).
            proj.element
                .set_position_map_preserving_3d(MapPoint::from_world_xyz(
                    proj.element.position().x,
                    proj.element.position().y,
                    proj.element.position().z,
                ));
        }
        // Increment lifetime counter for diagnostics/replay state. C++
        // projectile lifetime is governed by trajectory exhaustion and
        // impact side effects; it has no hard timeout.
        if !already_advanced_this_segment {
            proj.projectile.frame_count = proj.projectile.frame_count.saturating_add(1);
        }

        // ── Falling arrows skip all collision checks ──────────────
        // Once an arrow is deflected (by a shield or target), it
        // tumbles to the ground without hitting anything.  It
        // continues advancing along its deflected trajectory until it
        // runs out (handled by the trajectory advancement code above).
        if proj.projectile.falling {
            continue;
        }

        let arrow_new = proj.element.position();
        let arrow_old = primed_segment_start.unwrap_or(WorldPoint3D {
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
        let vx = proj.projectile.velocity_increment.x;
        let vy = proj.projectile.velocity_increment.y;

        let mut shield_blocker = None;
        if already_advanced_this_segment
            || !proj.projectile.trajectory.is_empty()
            || proj.projectile.trajectory_frame_count > 0
            || vx != 0.0
            || vy != 0.0
        {
            // Shield obstacle geometry lives in ground/world space, the same
            // space every other 3D sight-ray query uses, so the projectile
            // endpoints go in as world XYZ rather than screen-projected map
            // coordinates.
            let old_pos = [arrow_old.x, arrow_old.y, arrow_old.z];
            let new_pos = [arrow_new.x, arrow_new.y, arrow_new.z];

            // Flight direction with Y un-compressed.
            let flight_dir = (vx, vy * INVERSE_ASPECT_RATIO);

            for shield in &shield_snapshots {
                if Some(shield.holder_id) == shooter_id {
                    continue;
                }
                // (a) Arrow from front: dot(look_dir, flight_dir) < 0.
                let dot = shield.look_dir.0 * flight_dir.0 + shield.look_dir.1 * flight_dir.1;
                // (b) Arrow path intersects shield geometry.
                let blocking = dot < 0.0 && shield.obstacle.is_blocking_ray_3d(new_pos, old_pos);
                tracing::trace!(
                    ?arrow_id,
                    holder = ?shield.holder_id,
                    ?dot,
                    ?old_pos,
                    ?new_pos,
                    obstacle_id = shield.obstacle.id,
                    obstacle_points = ?shield.obstacle.obstacle_points,
                    blocking,
                    "Projectile shield-holder intersection test"
                );
                if blocking {
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
                make_arrow_falling_down(sim, proj, true, obstacle_check);

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
                    human_hit_old_position: None,
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
                    human_hit_old_position: None,
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
        /// Perpendicular offset from `p` to the line through `a→b`.
        ///
        /// C++ uses `SBGeoPoint3D::DistanceVectorToLine` for both the
        /// hit-radius norm and the leaning-out arrow retry's coarse
        /// `MaxNorm` gate. Return an infinite vector when the segment
        /// has zero length so callers naturally reject the line test.
        fn point_to_line_delta(p: WorldPoint3D, a: WorldPoint3D, b: WorldPoint3D) -> WorldVec3D {
            let abx = b.x - a.x;
            let aby = b.y - a.y;
            let abz = b.z - a.z;
            let ab_len_sq = abx * abx + aby * aby + abz * abz;
            if ab_len_sq < 1e-6 {
                return WorldVec3D {
                    x: f32::MAX,
                    y: f32::MAX,
                    z: f32::MAX,
                };
            }
            let apx = p.x - a.x;
            let apy = p.y - a.y;
            let apz = p.z - a.z;
            let t = (apx * abx + apy * aby + apz * abz) / ab_len_sq;
            WorldVec3D {
                x: p.x - (a.x + t * abx),
                y: p.y - (a.y + t * aby),
                z: p.z - (a.z + t * abz),
            }
        }
        fn point_to_line_distance(p: WorldPoint3D, a: WorldPoint3D, b: WorldPoint3D) -> f32 {
            let delta = point_to_line_delta(p, a, b);
            (delta.x * delta.x + delta.y * delta.y + delta.z * delta.z).sqrt()
        }
        fn distance(a: WorldPoint3D, b: WorldPoint3D) -> f32 {
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
        // C++ compares floating-point segment endpoint distances exactly
        // (`vtRange.Norm() <= range`).  Rust's f32 integration can land
        // a target a tiny fraction past the nominal endpoint on flat
        // shots, so allow a sub-pixel tolerance while keeping the same
        // old/current range gate.

        // Pick the aim anchor by projectile type — arrows and apples
        // aim for the belt, stones aim for the eyes.
        let uses_eyes_anchor = matches!(proj.object.object_type, ObjectType::Stone);

        // C++ `FindHumanVictim` returns no human victim when the
        // projectile is not moving.  FX target handling is separate in
        // `FindTargetVictim` and keeps the point fallback below.
        let use_range_gate = range > 0.0;

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
                    let line_distance = point_to_line_distance(anchor, arrow_old, arrow_new);
                    old_to_target <= range && line_distance <= HIT_DISTANCE
                } else {
                    false
                };
                if hit {
                    hit_victim = Some((snap.id, snap.position_map));
                    break;
                }

                // Leaning-out re-check for arrows only.  If the belt's
                // perpendicular distance-to-flight-line has max component
                // <= 100, try again at eye level. This matches the C++
                // `vtDistance.MaxNorm()` gate after the belt check.
                if snap.leaning_out
                    && matches!(proj.object.object_type, ObjectType::Arrow)
                    && use_range_gate
                {
                    let belt_line_delta = point_to_line_delta(anchor, arrow_old, arrow_new);
                    let max_norm = belt_line_delta
                        .x
                        .abs()
                        .max(belt_line_delta.y.abs())
                        .max(belt_line_delta.z.abs());
                    if max_norm <= 100.0 {
                        let eyes = snap.eyes;
                        // C++ uses the arrow's current position for the
                        // leaning-out eye retry, unlike the primary human
                        // belt check's "deguillaumized" old-position gate.
                        let new_to_eyes = distance(arrow_new, eyes);
                        if new_to_eyes <= range
                            && point_to_line_distance(eyes, arrow_old, arrow_new) <= HIT_DISTANCE
                        {
                            hit_victim = Some((snap.id, snap.position_map));
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
        let mut fx_target_hit: Option<(EntityId, Command, MapPoint)> = None;
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
                    fx_target_hit = Some((snap.id, activation_command, snap.position_map));
                    break;
                }
            }
        }

        if let Some((victim, victim_position_map)) = hit_victim {
            let impact_pos = victim_position_map;
            proj.projectile.flying = false;
            let despawn = if is_burster {
                set_projectile_animation(proj, Animation::ObjectBursting);
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
                human_hit_old_position: Some(arrow_old),
            });
        } else if let Some((fx_id, fx_command, fx_position_map)) = fx_target_hit {
            let impact_pos = fx_position_map;
            proj.projectile.flying = false;
            // RHElementProjectile::Hourglass handles a successful HitTarget
            // synchronously: stop flight and DeleteTrajectory before the
            // frame snapshot. Unlike HitHuman it does not rewind to the old
            // position, so the following Refresh keeps the arrow for this
            // moving frame; next Hourglass calls NewMove and the subsequent
            // Refresh retires the now-stationary empty arrow.
            proj.projectile.trajectory.clear();
            let despawn = if is_burster {
                set_projectile_animation(proj, Animation::ObjectBursting);
                false
            } else {
                true
            };
            results.push(ArrowTickResult {
                arrow: arrow_id,
                hit_target: None,
                shield_hit: None,
                fx_target_hit: Some((fx_id, fx_command)),
                despawn,
                damage,
                impact_fx,
                impact_pos,
                human_hit_old_position: None,
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
    entities: &mut Entities,
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
    entities: &mut Entities,
    victim_id: EntityId,
    shooter_id: EntityId,
    damage: u16,
    concussion: u16,
    arrow_flight_direction: i16,
) -> bool {
    // Resolve shooter PC-ness before the victim mutable borrow. C++
    // projectile damage carries a real origin pointer; missing shooter
    // state is invalid and must not become "not a PC" silently.
    let Some(shooter) = entities.get(shooter_id) else {
        tracing::warn!(
            ?victim_id,
            ?shooter_id,
            "projectile hit skipped: missing shooter before damage"
        );
        return false;
    };
    let shooter_is_pc = shooter.is_pc();

    let victim = match entities.get_mut(victim_id) {
        Some(e) => e,
        None => {
            tracing::warn!(
                ?victim_id,
                ?shooter_id,
                "projectile hit skipped: missing victim before damage"
            );
            return false;
        }
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
    let Some(human) = victim.human_data() else {
        tracing::warn!(
            ?victim_id,
            "projectile hit skipped: human victim missing human data before damage"
        );
        return false;
    };
    let was_unconscious = human.unconscious;

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
    // consumed at that NPC's next owner slot by
    // `tick_inform_my_friends_for_npc`, which broadcasts the body to
    // nearby NPCs. Without this, a stone-KO'd soldier would not be
    // detected by his friends, breaking witness wiring for PC-thrown
    // stones.
    let Some(human) = victim.human_data() else {
        tracing::warn!(
            ?victim_id,
            "projectile hit skipped: human victim missing human data after damage"
        );
        return false;
    };
    let now_unconscious = human.unconscious;
    if !was_unconscious
        && now_unconscious
        && let Some(npc) = victim.npc_data_mut()
    {
        npc.inform_my_friends = shooter_is_pc;
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
    use crate::coordinates::{SpriteFrameOffset, SpriteLocalPoint};
    use crate::element::{
        ActorData, ElementKind, ElementTarget, FxData, HumanData, TargetData, TargetFilter,
    };
    use crate::element::{ActorPc, ActorSoldier, NpcData, PcData, SoldierData};
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    trait TestEntityIndexAccess {
        fn get_at_index(&self, index: u32) -> Option<(EntityId, &Entity)>;
        fn get_mut_at_index(&mut self, index: u32) -> Option<(EntityId, &mut Entity)>;
    }

    impl TestEntityIndexAccess for Entities {
        fn get_at_index(&self, index: u32) -> Option<(EntityId, &Entity)> {
            self.get_legacy_slot(index)
        }

        fn get_mut_at_index(&mut self, index: u32) -> Option<(EntityId, &mut Entity)> {
            self.get_legacy_slot_mut(index)
        }
    }

    fn entity_table(slots: Vec<Option<Entity>>) -> Entities {
        let mut entities = Entities::new();
        for slot in slots {
            entities.push(slot);
        }
        entities
    }

    fn make_pc(x: f32, y: f32) -> Entity {
        let mut element = ElementData {
            kind: ElementKind::ActorPc,
            active: true,
            ..ElementData::default()
        };
        element.set_position_map(MapPoint { x, y });
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
        element.set_position_map(MapPoint { x, y });
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
        element.set_position_map(MapPoint { x, y });
        element.set_position(WorldPoint3D { x, y, z: 0.0 });
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

    fn set_test_action_state_after_transition(
        sm: &mut SequenceManager,
        seq_id: SequenceId,
        elem_idx: usize,
        action_state: ActionState,
    ) {
        sm.get_element_mut(seq_id, elem_idx)
            .unwrap()
            .action_state_after_transition = action_state;
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
                hotspot: SpriteLocalPoint::new(2.0, 3.0),
                sum_distance: 0,
                frame_ids: vec![1, 2, 3],
                delays: vec![0, 0, 0],
                distances: vec![0, 0, 0],
                offsets: vec![SpriteFrameOffset::ZERO; 3],
                sound_ids: vec![0, 0, 0],
            });
        }

        let sprite = &mut entity.element_data_mut().sprite;
        sprite.scripts = std::sync::Arc::new(scripts);
        sprite.conversion = std::sync::Arc::new(conversion);
    }

    #[test]
    fn begin_bow_shot_sets_shooter_state() {
        let mut entities =
            entity_table(vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))]);
        let target_id = EntityId::Soldier(crate::entity_id::SoldierId(1));
        let (mut sm, seq_id, elem_idx) =
            launch_test_shoot_element(EntityId::Pc(crate::entity_id::PcId(0)), target_id);
        let result = begin_bow_shot(
            &mut entities,
            &mut sm,
            EntityId::Pc(crate::entity_id::PcId(0)),
            target_id,
            seq_id,
            elem_idx,
            false,
            10,
            None,
            &mut 1u32,
        );
        assert_eq!(result, BeginShotResult::Started);

        let actor = entities
            .get_at_index(0)
            .map(|(_, entity)| entity)
            .unwrap()
            .actor_data()
            .unwrap();
        assert_eq!(
            actor.action_state,
            ActionState::Waiting,
            "C++ ShootBow translation must not force the actor's action state before queued bow orders run"
        );
        assert!(actor.active_shot.is_active());
        assert_eq!(actor.active_shot.target, Some(target_id));
        assert_eq!(actor.active_shot.shoot_mode, Some(ShootMode::Normal));
        // Should have: shoot order + reload order (and possibly transition orders)
        assert!(sm.get_element(seq_id, elem_idx).unwrap().orders.len() >= 2);
    }

    #[test]
    fn todo_shot_retranslation_clears_only_its_own_execution_latch() {
        let owner = EntityId::Pc(crate::entity_id::PcId(0));
        let target_id = EntityId::Soldier(crate::entity_id::SoldierId(1));
        let mut entities =
            entity_table(vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))]);
        let mut sm = SequenceManager::new();
        let seq_id = sm.launch_element(build_shoot_bow_element(owner, target_id));
        let elem_idx = 0;
        assert_eq!(
            sm.get_element(seq_id, elem_idx).unwrap().state,
            crate::sequence::SequenceState::Todo,
            "cross-postponed elements are refreshed to Todo before dispatch"
        );
        entities
            .get_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .active_shot = ActiveShot {
            sequence_id: Some(seq_id),
            element_index: elem_idx,
            target: Some(target_id),
            order_id: Some(std::num::NonZeroU32::new(77).unwrap()),
            released: true,
            shoot_mode: Some(ShootMode::Normal),
        };

        clear_matching_retranslated_shot(&mut entities, owner, SequenceId(999), elem_idx);
        assert!(
            entities
                .get(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .active_shot
                .is_active(),
            "an unrelated sequence must retain its active shot"
        );

        clear_matching_retranslated_shot(&mut entities, owner, seq_id, elem_idx);
        assert!(
            !entities
                .get(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .active_shot
                .is_active(),
            "the resumed element must release its stale execution latch before translation"
        );

        let mut next_order_id = 100;
        assert_eq!(
            begin_bow_shot(
                &mut entities,
                &mut sm,
                owner,
                target_id,
                seq_id,
                elem_idx,
                false,
                1,
                Some(ShootMode::Normal),
                &mut next_order_id,
            ),
            BeginShotResult::Started,
            "the resumed Original element must translate as a fresh execution"
        );
    }

    #[test]
    fn tick_bow_shots_detaches_when_sequence_has_advanced_past_bow_orders() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut entities =
            entity_table(vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))]);
        let target_id = EntityId::Soldier(crate::entity_id::SoldierId(1));
        let (mut sm, seq_id, elem_idx) =
            launch_test_shoot_element(EntityId::Pc(crate::entity_id::PcId(0)), target_id);
        let result = begin_bow_shot(
            &mut entities,
            &mut sm,
            EntityId::Pc(crate::entity_id::PcId(0)),
            target_id,
            seq_id,
            elem_idx,
            false,
            10,
            None,
            &mut 1u32,
        );
        assert_eq!(result, BeginShotResult::Started);
        let orders = &mut sm.get_element_mut(seq_id, elem_idx).unwrap().orders;
        orders.clear();
        let mut next_order_id = 1000;
        orders.push_back(Order::new(
            OrderType::WalkingUpright,
            0.0,
            0.0,
            crate::order::alloc_order_id(&mut next_order_id),
        ));

        let events = tick_bow_shots(sim, &mut entities, &mut sm);

        assert!(events.fired.is_empty());
        assert!(events.completed.is_empty());
        assert!(
            !entities
                .get_at_index(0)
                .map(|(_, entity)| entity)
                .unwrap()
                .actor_data()
                .unwrap()
                .active_shot
                .is_active(),
            "C++ shoot-list ownership ends once the sequence has no bow orders left"
        );
    }

    #[test]
    fn single_owner_tick_preserves_replaced_other_actor_shot() {
        let sim_context = crate::sim_rng::test_context();
        let mut entities = entity_table(vec![
            Some(make_pc(0.0, 0.0)),
            Some(make_pc(5.0, 0.0)),
            Some(make_soldier(50.0, 0.0)),
        ]);
        let first = EntityId::Pc(crate::entity_id::PcId(0));
        let other = EntityId::Pc(crate::entity_id::PcId(1));
        let target = EntityId::Soldier(crate::entity_id::SoldierId(2));
        let mut sm = SequenceManager::new();
        let first_seq = sm.launch_element(build_shoot_bow_element(first, target));
        sm.element_in_progress(first_seq, 0);
        let other_seq = sm.launch_element(build_shoot_bow_element(other, target));
        sm.element_in_progress(other_seq, 0);
        let mut next_order_id = 1;
        assert_eq!(
            begin_bow_shot(
                &mut entities,
                &mut sm,
                first,
                target,
                first_seq,
                0,
                false,
                10,
                None,
                &mut next_order_id,
            ),
            BeginShotResult::Started
        );
        assert_eq!(
            begin_bow_shot(
                &mut entities,
                &mut sm,
                other,
                target,
                other_seq,
                0,
                false,
                10,
                None,
                &mut next_order_id,
            ),
            BeginShotResult::Started
        );
        let mut replacement = entities
            .get(other)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_shot;
        replacement.released = true;
        let selected_order = sm
            .current_order_for_actor(first)
            .expect("first bow order selected")
            .2
            .order_id;

        CROSS_ACTOR_SHOT_REPLACEMENT.set(Some((other, replacement)));

        tick_bow_shot_for_owner(
            &sim_context,
            &mut entities,
            &mut sm,
            first,
            selected_order,
            false,
        );

        assert_eq!(
            entities
                .get(other)
                .unwrap()
                .actor_data()
                .unwrap()
                .active_shot,
            replacement,
            "single-owner bow execution must preserve a synchronous cross-actor replacement"
        );
    }

    #[test]
    fn frozen_owner_bow_initialises_direction_without_advancing_sprite_or_order() {
        let sim = crate::sim_rng::test_context();
        let mut entities =
            entity_table(vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(40.0, 0.0))]);
        let shooter = EntityId::Pc(crate::entity_id::PcId(0));
        let target = EntityId::Soldier(crate::entity_id::SoldierId(1));
        entities
            .get_mut(shooter)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .action_state = ActionState::AimingWithBow;
        let mut sm = SequenceManager::new();
        let seq = sm.launch_element(build_shoot_bow_element(shooter, target));
        sm.element_in_progress(seq, 0);
        let mut next_order_id = 1;
        assert_eq!(
            begin_bow_shot(
                &mut entities,
                &mut sm,
                shooter,
                target,
                seq,
                0,
                false,
                10,
                None,
                &mut next_order_id
            ),
            BeginShotResult::Started
        );
        let order = sm.current_order_for_actor(shooter).unwrap().2.clone();
        let before_sprite = entities.get(shooter).unwrap().sprite().clone();
        // The shoot order samples its target only while the owner slot has
        // the execute order in its initialising window; arm it the way the
        // production Execute path does.
        entities
            .get_mut(shooter)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .execute_order_initialising = true;

        let events =
            tick_bow_shot_for_owner(&sim, &mut entities, &mut sm, shooter, order.order_id, true);

        assert!(events.fired.is_empty());
        assert!(events.completed.is_empty());
        let entity = entities.get(shooter).unwrap();
        assert_eq!(
            i16::from(entity.position_iface().get_direction_goal()),
            crate::position_interface::vector_to_sector_0_to_15_iso(40.0, 0.0)
        );
        assert_eq!(
            entity.sprite().last_processed_order_id,
            before_sprite.last_processed_order_id
        );
        assert_eq!(
            sm.current_order_for_actor(shooter).unwrap().2.order_id,
            order.order_id
        );
    }

    #[test]
    fn tick_bow_shots_waits_behind_pre_shoot_setup_order() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut entities =
            entity_table(vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))]);
        let target_id = EntityId::Soldier(crate::entity_id::SoldierId(1));
        let (mut sm, seq_id, elem_idx) =
            launch_test_shoot_element(EntityId::Pc(crate::entity_id::PcId(0)), target_id);
        let result = begin_bow_shot(
            &mut entities,
            &mut sm,
            EntityId::Pc(crate::entity_id::PcId(0)),
            target_id,
            seq_id,
            elem_idx,
            false,
            10,
            None,
            &mut 1u32,
        );
        assert_eq!(result, BeginShotResult::Started);
        let mut next_order_id = 1000;
        sm.get_element_mut(seq_id, elem_idx)
            .unwrap()
            .orders
            .push_front(Order::new(
                OrderType::TransitionWaitingUprightBoredWaitingUpright,
                0.0,
                0.0,
                crate::order::alloc_order_id(&mut next_order_id),
            ));

        let events = tick_bow_shots(sim, &mut entities, &mut sm);

        assert!(events.fired.is_empty());
        assert!(events.completed.is_empty());
        assert!(
            entities
                .get_at_index(0)
                .map(|(_, entity)| entity)
                .unwrap()
                .actor_data()
                .unwrap()
                .active_shot
                .is_active(),
            "pre-shoot setup orders should not cancel the pending bow shot"
        );
    }

    #[test]
    fn tick_bow_shots_detaches_before_trailing_non_bow_order() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut entities =
            entity_table(vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))]);
        let target_id = EntityId::Soldier(crate::entity_id::SoldierId(1));
        bind_test_bow_release_rows(
            entities
                .get_mut_at_index(0)
                .map(|(_, entity)| entity)
                .unwrap(),
            OrderType::ShootingWithBow,
        );
        let (mut sm, seq_id, elem_idx) =
            launch_test_shoot_element(EntityId::Pc(crate::entity_id::PcId(0)), target_id);
        let result = begin_bow_shot(
            &mut entities,
            &mut sm,
            EntityId::Pc(crate::entity_id::PcId(0)),
            target_id,
            seq_id,
            elem_idx,
            false,
            10,
            Some(ShootMode::Normal),
            &mut 1u32,
        );
        assert_eq!(result, BeginShotResult::Started);

        let mut next_order_id = 1000;
        let orders = &mut sm.get_element_mut(seq_id, elem_idx).unwrap().orders;
        orders.clear();
        orders.push_back(Order::new(
            OrderType::ShootingWithBow,
            0.0,
            0.0,
            crate::order::alloc_order_id(&mut next_order_id),
        ));
        orders.push_back(Order::new(
            OrderType::TransitionWaitingUprightBoredWaitingUpright,
            0.0,
            0.0,
            crate::order::alloc_order_id(&mut next_order_id),
        ));

        let mut fired = Vec::new();
        let mut completed = Vec::new();
        for _ in 0..64 {
            let events = tick_bow_shots(sim, &mut entities, &mut sm);
            fired.extend(events.fired);
            completed.extend(events.completed);
            if !entities
                .get_at_index(0)
                .map(|(_, entity)| entity)
                .unwrap()
                .actor_data()
                .unwrap()
                .active_shot
                .is_active()
            {
                break;
            }
        }

        assert_eq!(fired.len(), 1);
        assert!(completed.is_empty());
        assert!(
            !entities
                .get_at_index(0)
                .map(|(_, entity)| entity)
                .unwrap()
                .actor_data()
                .unwrap()
                .active_shot
                .is_active(),
            "active bow-shot driver should detach after the final bow order"
        );
        assert_eq!(
            sm.get_element(seq_id, elem_idx)
                .unwrap()
                .current_order()
                .unwrap()
                .order_type,
            OrderType::TransitionWaitingUprightBoredWaitingUpright
        );
    }

    #[test]
    #[should_panic(expected = "active bow shot missing resolved shoot mode")]
    fn tick_bow_shots_panics_on_missing_resolved_shoot_mode() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut entities =
            entity_table(vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))]);
        let target_id = EntityId::Soldier(crate::entity_id::SoldierId(1));
        let (mut sm, seq_id, elem_idx) =
            launch_test_shoot_element(EntityId::Pc(crate::entity_id::PcId(0)), target_id);
        let result = begin_bow_shot(
            &mut entities,
            &mut sm,
            EntityId::Pc(crate::entity_id::PcId(0)),
            target_id,
            seq_id,
            elem_idx,
            false,
            10,
            None,
            &mut 1u32,
        );
        assert_eq!(result, BeginShotResult::Started);
        let facing = crate::position_interface::vector_to_sector_0_to_15_iso(50.0, 0.0);
        let shooter = entities
            .get_mut_at_index(0)
            .map(|(_, entity)| entity)
            .unwrap();
        shooter.element_data_mut().set_direction_instantly(facing);
        shooter.actor_data_mut().unwrap().active_shot.shoot_mode = None;

        let _ = tick_bow_shots(sim, &mut entities, &mut sm);
    }

    #[test]
    fn begin_bow_shot_keeps_current_aim_state_until_transition_pulse() {
        let mut entities =
            entity_table(vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))]);
        let target_id = EntityId::Soldier(crate::entity_id::SoldierId(1));
        entities
            .get_mut_at_index(0)
            .map(|(_, entity)| entity)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .action_state = ActionState::AimingWithBow;
        let (mut sm, seq_id, elem_idx) =
            launch_test_shoot_element(EntityId::Pc(crate::entity_id::PcId(0)), target_id);
        set_test_action_state_after_transition(
            &mut sm,
            seq_id,
            elem_idx,
            ActionState::AimingWithBow,
        );

        let result = begin_bow_shot(
            &mut entities,
            &mut sm,
            EntityId::Pc(crate::entity_id::PcId(0)),
            target_id,
            seq_id,
            elem_idx,
            false,
            10,
            Some(ShootMode::Long),
            &mut 1u32,
        );

        assert_eq!(result, BeginShotResult::Started);
        let actor = entities
            .get_at_index(0)
            .map(|(_, entity)| entity)
            .unwrap()
            .actor_data()
            .unwrap();
        assert_eq!(actor.action_state, ActionState::AimingWithBow);
        assert_eq!(actor.active_shot.shoot_mode, Some(ShootMode::Long));
        let orders: Vec<OrderType> = sm
            .get_element(seq_id, elem_idx)
            .unwrap()
            .orders
            .iter()
            .map(|o| o.order_type)
            .collect();
        assert_eq!(orders[0], OrderType::TransitionRaisingBow);
        assert_eq!(orders[1], OrderType::ShootingWithBowUp);
    }

    #[test]
    fn begin_bow_shot_uses_action_state_after_transition_for_setup_orders() {
        let mut entities =
            entity_table(vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))]);
        let target_id = EntityId::Soldier(crate::entity_id::SoldierId(1));
        let (mut sm, seq_id, elem_idx) =
            launch_test_shoot_element(EntityId::Pc(crate::entity_id::PcId(0)), target_id);
        set_test_action_state_after_transition(
            &mut sm,
            seq_id,
            elem_idx,
            ActionState::AimingWithBow,
        );

        let result = begin_bow_shot(
            &mut entities,
            &mut sm,
            EntityId::Pc(crate::entity_id::PcId(0)),
            target_id,
            seq_id,
            elem_idx,
            false,
            10,
            Some(ShootMode::Long),
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
        assert_eq!(
            orders[0],
            OrderType::TransitionRaisingBow,
            "C++ uses ActionStateAfterTransition, so a first long shot after equip/load still raises the bow before shooting"
        );
        assert_eq!(orders[1], OrderType::ShootingWithBowUp);
    }

    #[test]
    fn begin_bow_shot_accepts_active_target_that_died_while_aiming() {
        let mut entities =
            entity_table(vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))]);
        let target_id = EntityId::Soldier(crate::entity_id::SoldierId(1));
        if let Some((_, Entity::Soldier(s))) = entities.get_mut_at_index(1) {
            s.npc.life_points = 0; // dead
        }
        let (mut sm, seq_id, elem_idx) =
            launch_test_shoot_element(EntityId::Pc(crate::entity_id::PcId(0)), target_id);
        let result = begin_bow_shot(
            &mut entities,
            &mut sm,
            EntityId::Pc(crate::entity_id::PcId(0)),
            target_id,
            seq_id,
            elem_idx,
            false,
            10,
            None,
            &mut 1u32,
        );
        assert_eq!(result, BeginShotResult::Started);
        assert!(
            sm.get_element(seq_id, elem_idx)
                .unwrap()
                .orders
                .iter()
                .any(|order| matches!(
                    order.order_type,
                    OrderType::ShootingWithBow | OrderType::ShootingWithBowUp
                ))
        );
    }

    #[test]
    fn begin_bow_shot_accepts_arrow_fx_target() {
        let mut entities = entity_table(vec![
            Some(make_pc(0.0, 0.0)),
            Some(make_arrow_target(50.0, 0.0)),
        ]);
        let target_id = EntityId::Target(crate::entity_id::TargetId(1));
        let (mut sm, seq_id, elem_idx) =
            launch_test_shoot_element(EntityId::Pc(crate::entity_id::PcId(0)), target_id);
        let result = begin_bow_shot(
            &mut entities,
            &mut sm,
            EntityId::Pc(crate::entity_id::PcId(0)),
            target_id,
            seq_id,
            elem_idx,
            false,
            10,
            None,
            &mut 1u32,
        );
        assert_eq!(result, BeginShotResult::Started);
        assert_eq!(
            entities
                .get_at_index(0)
                .map(|(_, entity)| entity)
                .unwrap()
                .actor_data()
                .unwrap()
                .active_shot
                .target,
            Some(target_id)
        );
    }

    #[test]
    fn begin_bow_shot_uses_anonymous_shoot_orders() {
        let mut entities = entity_table(vec![
            Some(make_anonymous_pc(0.0, 0.0)),
            Some(make_soldier(50.0, 0.0)),
        ]);
        let target_id = EntityId::Soldier(crate::entity_id::SoldierId(1));
        let (mut sm, seq_id, elem_idx) =
            launch_test_shoot_element(EntityId::Pc(crate::entity_id::PcId(0)), target_id);

        let result = begin_bow_shot(
            &mut entities,
            &mut sm,
            EntityId::Pc(crate::entity_id::PcId(0)),
            target_id,
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
    fn begin_bow_shot_preserves_facing_until_shoot_order_initialization() {
        let mut target = make_arrow_target(50.0, 120.0);
        target.element_data_mut().set_position(WorldPoint3D {
            x: 50.0,
            y: 120.0,
            z: 100.0,
        });
        let mut entities = entity_table(vec![Some(make_pc(0.0, 100.0)), Some(target)]);
        entities
            .get_mut_at_index(0)
            .map(|(_, entity)| entity)
            .unwrap()
            .element_data_mut()
            .set_direction_goal(7);
        let target_id = EntityId::Target(crate::entity_id::TargetId(1));
        let (mut sm, seq_id, elem_idx) =
            launch_test_shoot_element(EntityId::Pc(crate::entity_id::PcId(0)), target_id);

        let result = begin_bow_shot(
            &mut entities,
            &mut sm,
            EntityId::Pc(crate::entity_id::PcId(0)),
            target_id,
            seq_id,
            elem_idx,
            false,
            10,
            None,
            &mut 1u32,
        );

        assert_eq!(result, BeginShotResult::Started);
        let direction_goal = i16::from(
            entities
                .get_at_index(0)
                .map(|(_, entity)| entity)
                .unwrap()
                .element_data()
                .sprite
                .position_iface
                .get_direction_goal(),
        );
        assert_eq!(direction_goal, 7);
    }

    #[test]
    fn shoot_initialization_samples_fx_target_cxx_ground_y_once() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut target = make_arrow_target(50.0, 120.0);
        target.element_data_mut().set_position(WorldPoint3D {
            x: 50.0,
            y: 120.0,
            z: 100.0,
        });
        let mut entities = entity_table(vec![Some(make_pc(0.0, 100.0)), Some(target)]);
        let target_id = EntityId::Target(crate::entity_id::TargetId(1));
        bind_test_bow_release_rows(
            entities
                .get_mut_at_index(0)
                .map(|(_, entity)| entity)
                .unwrap(),
            OrderType::ShootingWithBow,
        );
        let (mut sm, seq_id, elem_idx) =
            launch_test_shoot_element(EntityId::Pc(crate::entity_id::PcId(0)), target_id);

        let result = begin_bow_shot(
            &mut entities,
            &mut sm,
            EntityId::Pc(crate::entity_id::PcId(0)),
            target_id,
            seq_id,
            elem_idx,
            false,
            10,
            Some(ShootMode::Normal),
            &mut 1u32,
        );
        assert_eq!(result, BeginShotResult::Started);
        entities
            .get_mut_at_index(0)
            .map(|(_, entity)| entity)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .execute_order_initialising = true;

        tick_bow_shots(sim, &mut entities, &mut sm);

        let direction_goal = i16::from(
            entities
                .get_at_index(0)
                .map(|(_, entity)| entity)
                .unwrap()
                .element_data()
                .sprite
                .position_iface
                .get_direction_goal(),
        );
        assert_eq!(
            direction_goal,
            crate::position_interface::vector_to_sector_0_to_15_iso(50.0, 20.0)
        );
        assert_ne!(
            direction_goal,
            crate::position_interface::vector_to_sector_0_to_15_iso(50.0, -80.0)
        );

        entities
            .get_mut_at_index(0)
            .map(|(_, entity)| entity)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .execute_order_initialising = false;
        entities
            .get_mut(target_id)
            .unwrap()
            .element_data_mut()
            .set_position(WorldPoint3D {
                x: -100.0,
                y: -100.0,
                z: 0.0,
            });
        tick_bow_shots(sim, &mut entities, &mut sm);
        assert_eq!(
            i16::from(
                entities
                    .get_at_index(0)
                    .map(|(_, entity)| entity)
                    .unwrap()
                    .position_iface()
                    .get_direction_goal()
            ),
            direction_goal,
            "the live target is sampled once per shooting order"
        );
    }

    #[test]
    fn leaning_out_shot_initializes_from_live_map_positions_and_holds_while_turning() {
        let sim = crate::sim_rng::test_context();
        let mut target = make_pc(50.0, 20.0);
        target.element_data_mut().set_position(WorldPoint3D {
            x: 50.0,
            y: 120.0,
            z: 100.0,
        });
        let mut shooter = make_soldier(0.0, 0.0);
        shooter.element_data_mut().posture = Posture::LeaningOut;
        shooter.actor_data_mut().unwrap().action_state = ActionState::AimingWithBowDown;
        shooter.element_data_mut().set_direction_instantly(14);
        bind_test_bow_release_rows(&mut shooter, OrderType::ShootingWithBowLeaningOut);

        let shooter_id = EntityId::Soldier(crate::entity_id::SoldierId(0));
        let target_id = EntityId::Pc(crate::entity_id::PcId(1));
        let mut entities = entity_table(vec![Some(shooter), Some(target)]);
        let (mut sm, seq_id, elem_idx) = launch_test_shoot_element(shooter_id, target_id);
        assert_eq!(
            begin_bow_shot(
                &mut entities,
                &mut sm,
                shooter_id,
                target_id,
                seq_id,
                elem_idx,
                false,
                10,
                Some(ShootMode::Down),
                &mut 1u32,
            ),
            BeginShotResult::Started
        );
        entities
            .get_mut(shooter_id)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .execute_order_initialising = true;

        let events = tick_bow_shots(&sim, &mut entities, &mut sm);
        assert!(events.fired.is_empty());

        let shooter = entities.get(shooter_id).unwrap();
        let expected_goal = crate::position_interface::vector_to_sector_0_to_15_iso(50.0, 20.0);
        assert_ne!(
            expected_goal,
            crate::position_interface::vector_to_sector_0_to_15_iso(50.0, 120.0),
            "the fixture must distinguish PositionMap from PositionGround"
        );
        assert_eq!(
            i16::from(shooter.position_iface().get_direction_goal()),
            expected_goal
        );
        assert_eq!(i16::from(shooter.position_iface().get_direction()), 13);
        assert_eq!(shooter.sprite().current_row, 13);
        assert_eq!(shooter.sprite().current_frame, 0);
    }

    #[test]
    fn tick_bow_shots_fires_arrow_and_returns_to_aiming() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut entities =
            entity_table(vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))]);
        let target_id = EntityId::Soldier(crate::entity_id::SoldierId(1));
        bind_test_bow_release_rows(
            entities
                .get_mut_at_index(0)
                .map(|(_, entity)| entity)
                .unwrap(),
            OrderType::ShootingWithBow,
        );
        let (mut sm, seq_id, elem_idx) =
            launch_test_shoot_element(EntityId::Pc(crate::entity_id::PcId(0)), target_id);

        begin_bow_shot(
            &mut entities,
            &mut sm,
            EntityId::Pc(crate::entity_id::PcId(0)),
            target_id,
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
            let events = tick_bow_shots(sim, &mut entities, &mut sm);
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
        assert_eq!(r.shooter, EntityId::Pc(crate::entity_id::PcId(0)));
        assert_eq!(r.target, target_id);
        assert_eq!(r.target_pos.x, 50.0);

        // Shooter should now be in AimingWithBow (sustained aim).
        let actor = entities
            .get_at_index(0)
            .map(|(_, entity)| entity)
            .unwrap()
            .actor_data()
            .unwrap();
        assert_eq!(actor.action_state, ActionState::AimingWithBow);
        assert!(actor.active_shot.is_active());
        assert!(actor.active_shot.released);
    }

    #[test]
    fn compute_initial_throw_velocity_flat_shot() {
        let to_target = WorldVec3D {
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
        let to_target = WorldVec3D {
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
        let start = WorldPoint3D {
            x: 0.0,
            y: 0.0,
            z: 40.0,
        };
        let vel = compute_initial_throw_velocity(
            WorldVec3D {
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
                position: WorldPoint3D {
                    x: 25.0,
                    y: 0.0,
                    z: 45.0,
                },
                time: 4,
            },
            TrajectoryPoint {
                position: WorldPoint3D {
                    x: 50.0,
                    y: 0.0,
                    z: 40.0,
                },
                time: 4,
            },
        ];
        let arrow = spawn_arrow(SpawnArrowParams {
            shooter: EntityId::Pc(crate::entity_id::PcId(0)),
            bow_point: WorldPoint3D {
                x: 0.0,
                y: 0.0,
                z: 40.0,
            },
            trajectory_origin: MapPoint { x: 0.0, y: 0.0 },
            target: EntityId::Soldier(crate::entity_id::SoldierId(1)),
            target_pos: MapPoint { x: 50.0, y: 0.0 },
            trajectory: traj,
            damage: 30,
            layer: 0,
            lands_in_hole: false,
            initial_velocity: WorldVec3D {
                x: 0.0,
                y: 1.0,
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
                assert_eq!(
                    p.element.direction(),
                    0,
                    "projectile sprite facing stays at its element-constructor default"
                );
                assert_ne!(
                    p.projectile.flight_direction, 0,
                    "gameplay flight direction is stored separately from sprite facing"
                );
            }
            _ => panic!("expected ElementProjectile"),
        }
    }

    #[test]
    fn spawn_arrow_stores_shooter_map_position_as_trajectory_origin() {
        let arrow = spawn_arrow(SpawnArrowParams {
            shooter: EntityId::Pc(crate::entity_id::PcId(0)),
            bow_point: WorldPoint3D {
                x: 100.0,
                y: 40.0,
                z: 40.0,
            },
            trajectory_origin: MapPoint { x: 100.0, y: 0.0 },
            target: EntityId::Pc(crate::entity_id::PcId(1)),
            target_pos: MapPoint { x: 50.0, y: 0.0 },
            trajectory: vec![TrajectoryPoint {
                position: WorldPoint3D {
                    x: 50.0,
                    y: 40.0,
                    z: 40.0,
                },
                time: 2,
            }],
            damage: 30,
            layer: 0,
            lands_in_hole: false,
            initial_velocity: WorldVec3D {
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
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        // Place a soldier at (50, 0) (belt lives at Z=25, the
        // default belt elevation for an upright human).  The
        // trajectory arcs from the bow height down to belt height at
        // the soldier's XY — the per-segment 3D hit check picks the
        // soldier up on the final waypoint.
        let traj = vec![
            TrajectoryPoint {
                position: WorldPoint3D {
                    x: 20.0,
                    y: 0.0,
                    z: 35.0,
                },
                time: 2,
            },
            TrajectoryPoint {
                position: WorldPoint3D {
                    x: 40.0,
                    y: 0.0,
                    z: 30.0,
                },
                time: 2,
            },
            TrajectoryPoint {
                position: WorldPoint3D {
                    x: 50.0,
                    y: 0.0,
                    z: 25.0,
                },
                time: 2,
            },
        ];
        let mut entities = entity_table(vec![
            Some(make_pc(0.0, 0.0)),
            Some(make_soldier(50.0, 0.0)),
            Some(spawn_arrow(SpawnArrowParams {
                shooter: EntityId::Pc(crate::entity_id::PcId(0)),
                bow_point: WorldPoint3D {
                    x: 0.0,
                    y: 0.0,
                    z: 40.0,
                },
                trajectory_origin: MapPoint { x: 0.0, y: 0.0 },
                target: EntityId::Pc(crate::entity_id::PcId(1)),
                target_pos: MapPoint { x: 50.0, y: 0.0 },
                trajectory: traj,
                damage: 30,
                layer: 0,
                lands_in_hole: false,
                initial_velocity: WorldVec3D {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
            })),
        ]);

        let mut hit = None;
        for _ in 0..20 {
            let results = tick_arrows(
                sim,
                &mut entities,
                crate::sight_obstacle::ObstacleList::empty(),
            );
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
        assert_eq!(
            hit,
            Some(EntityId::Soldier(crate::entity_id::SoldierId(1))),
            "arrow should reach target"
        );
    }

    #[test]
    fn tick_arrows_human_hit_reports_old_position_and_victim_impact_anchor() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let traj = vec![
            TrajectoryPoint {
                position: WorldPoint3D {
                    x: 20.0,
                    y: 0.0,
                    z: 35.0,
                },
                time: 2,
            },
            TrajectoryPoint {
                position: WorldPoint3D {
                    x: 40.0,
                    y: 0.0,
                    z: 30.0,
                },
                time: 2,
            },
            TrajectoryPoint {
                position: WorldPoint3D {
                    x: 50.0,
                    y: 0.0,
                    z: 25.0,
                },
                time: 2,
            },
        ];
        let arrow = spawn_arrow(SpawnArrowParams {
            shooter: EntityId::Pc(crate::entity_id::PcId(0)),
            bow_point: WorldPoint3D {
                x: 0.0,
                y: 0.0,
                z: 40.0,
            },
            trajectory_origin: MapPoint { x: 0.0, y: 0.0 },
            target: EntityId::Soldier(crate::entity_id::SoldierId(1)),
            target_pos: MapPoint { x: 50.0, y: 0.0 },
            trajectory: traj,
            damage: 30,
            layer: 0,
            lands_in_hole: false,
            initial_velocity: WorldVec3D {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        });
        let mut entities = entity_table(vec![
            Some(make_pc(0.0, 0.0)),
            Some(make_soldier(50.0, 0.0)),
            Some(arrow),
        ]);

        let mut hit = None;
        for _ in 0..20 {
            let results = tick_arrows(
                sim,
                &mut entities,
                crate::sight_obstacle::ObstacleList::empty(),
            );
            hit = results.into_iter().find(|result| {
                result.hit_target == Some(EntityId::Soldier(crate::entity_id::SoldierId(1)))
            });
            if hit.is_some() {
                break;
            }
        }

        let hit = hit.expect("arrow should reach human target");
        assert_eq!(hit.impact_pos, MapPoint { x: 50.0, y: 0.0 });
        let old_pos = hit
            .human_hit_old_position
            .expect("human hit should carry previous projectile position");
        assert!(old_pos.x < hit.impact_pos.x);
        assert!((old_pos.y - 0.0).abs() < 0.01);
        assert!(old_pos.z >= 25.0);
    }

    #[test]
    fn tick_arrow_resolves_spawn_primed_segment_only_for_requested_arrow() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let arrow = spawn_arrow(SpawnArrowParams {
            shooter: EntityId::Pc(crate::entity_id::PcId(0)),
            bow_point: WorldPoint3D {
                x: 0.0,
                y: 0.0,
                z: 40.0,
            },
            trajectory_origin: MapPoint { x: 0.0, y: 0.0 },
            target: EntityId::Target(crate::entity_id::TargetId(1)),
            target_pos: MapPoint { x: 50.0, y: 0.0 },
            trajectory: vec![TrajectoryPoint {
                position: WorldPoint3D {
                    x: 50.0,
                    y: 0.0,
                    z: 0.0,
                },
                time: 1,
            }],
            damage: 30,
            layer: 0,
            lands_in_hole: false,
            initial_velocity: WorldVec3D {
                x: 1.0,
                y: 0.0,
                z: -0.25,
            },
        });
        let mut other_arrow = spawn_arrow(SpawnArrowParams {
            shooter: EntityId::Pc(crate::entity_id::PcId(0)),
            bow_point: WorldPoint3D {
                x: 1000.0,
                y: 0.0,
                z: 40.0,
            },
            trajectory_origin: MapPoint { x: 0.0, y: 0.0 },
            target: EntityId::Target(crate::entity_id::TargetId(1)),
            target_pos: MapPoint { x: 50.0, y: 0.0 },
            trajectory: vec![TrajectoryPoint {
                position: WorldPoint3D {
                    x: 1010.0,
                    y: 0.0,
                    z: 40.0,
                },
                time: 1,
            }],
            damage: 30,
            layer: 0,
            lands_in_hole: false,
            initial_velocity: WorldVec3D {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        });
        let Entity::Projectile(p) = &mut other_arrow else {
            panic!("spawn_arrow should create projectile");
        };
        p.projectile.launch_segment_start = Some(WorldPoint3D {
            x: 1000.0,
            y: 0.0,
            z: 40.0,
        });

        let mut entities = entity_table(vec![
            Some(make_pc(0.0, 0.0)),
            Some(make_arrow_target(50.0, 0.0)),
            Some(arrow),
            Some(other_arrow),
        ]);

        let results = tick_arrow(
            sim,
            &mut entities,
            crate::sight_obstacle::ObstacleList::empty(),
            None,
            EntityId::Projectile(crate::entity_id::ProjectileId(2)),
        );

        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].arrow,
            EntityId::Projectile(crate::entity_id::ProjectileId(2))
        );
        assert_eq!(
            results[0].fx_target_hit,
            Some((
                EntityId::Target(crate::entity_id::TargetId(1)),
                Command::ActivateArrow
            ))
        );

        let Some(Entity::Projectile(p)) = entities.get_at_index(3).map(|(_, entity)| entity) else {
            panic!("other arrow should remain present");
        };
        assert!(
            p.projectile.launch_segment_start.is_some(),
            "filtered tick must not consume another projectile's primed segment"
        );
    }

    #[test]
    fn tick_arrows_prefilters_friendly_candidate_before_selecting_victim() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let arrow = spawn_arrow(SpawnArrowParams {
            shooter: EntityId::Soldier(crate::entity_id::SoldierId(0)),
            bow_point: WorldPoint3D {
                x: 0.0,
                y: 0.0,
                z: 25.0,
            },
            trajectory_origin: MapPoint { x: 0.0, y: 0.0 },
            target: EntityId::Soldier(crate::entity_id::SoldierId(2)),
            target_pos: MapPoint { x: 100.0, y: 0.0 },
            trajectory: vec![TrajectoryPoint {
                position: WorldPoint3D {
                    x: 100.0,
                    y: 0.0,
                    z: 25.0,
                },
                time: 1,
            }],
            damage: 30,
            layer: 0,
            lands_in_hole: false,
            initial_velocity: WorldVec3D {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        });
        let mut entities = entity_table(vec![
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
                80.0,
                0.0,
                crate::element::Camp::Lacklandists,
            )),
            Some(arrow),
        ]);

        let results = tick_arrows(
            sim,
            &mut entities,
            crate::sight_obstacle::ObstacleList::empty(),
        );
        assert!(
            results
                .iter()
                .all(|r| r.hit_target != Some(EntityId::Soldier(crate::entity_id::SoldierId(1)))),
            "same-camp soldier must be filtered before hit selection"
        );
        assert!(
            results
                .iter()
                .any(|r| r.hit_target == Some(EntityId::Soldier(crate::entity_id::SoldierId(2)))),
            "arrow should continue to the valid victim behind the filtered candidate"
        );
    }

    #[test]
    fn tick_arrows_stationary_projectile_does_not_hit_human() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut arrow_element = ElementData {
            kind: ElementKind::ObjectProjectile,
            active: true,
            ..ElementData::default()
        };
        arrow_element.set_position_map(MapPoint { x: 50.0, y: -25.0 });
        arrow_element.set_position(WorldPoint3D {
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
                reference: Some(EntityId::Pc(crate::entity_id::PcId(1))),
                ..ObjectData::default()
            },
            projectile: ProjectileData {
                shooter: Some(EntityId::Pc(crate::entity_id::PcId(0))),
                flying: true,
                trajectory: vec![TrajectoryPoint {
                    position: WorldPoint3D {
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
        let mut entities = entity_table(vec![
            Some(make_pc(0.0, 0.0)),
            Some(make_soldier_with_camp(
                50.0,
                0.0,
                crate::element::Camp::Lacklandists,
            )),
            Some(arrow),
        ]);

        let results = tick_arrows(
            sim,
            &mut entities,
            crate::sight_obstacle::ObstacleList::empty(),
        );
        assert!(
            results.iter().all(|r| r.hit_target.is_none()),
            "C++ FindHumanVictim returns no victim when projectile is not moving"
        );
    }

    #[test]
    fn tick_arrows_without_shooter_does_not_hit_human() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut arrow = spawn_arrow(SpawnArrowParams {
            shooter: EntityId::Pc(crate::entity_id::PcId(0)),
            bow_point: WorldPoint3D {
                x: 0.0,
                y: 0.0,
                z: 40.0,
            },
            trajectory_origin: MapPoint { x: 0.0, y: 0.0 },
            target: EntityId::Pc(crate::entity_id::PcId(1)),
            target_pos: MapPoint { x: 50.0, y: 0.0 },
            trajectory: vec![TrajectoryPoint {
                position: WorldPoint3D {
                    x: 50.0,
                    y: 0.0,
                    z: 25.0,
                },
                time: 1,
            }],
            damage: 30,
            layer: 0,
            lands_in_hole: false,
            initial_velocity: WorldVec3D {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        });
        if let Entity::Projectile(proj) = &mut arrow {
            proj.projectile.shooter = None;
        }
        let mut entities = entity_table(vec![None, Some(make_soldier(50.0, 0.0)), Some(arrow)]);

        let results = tick_arrows(
            sim,
            &mut entities,
            crate::sight_obstacle::ObstacleList::empty(),
        );
        assert!(
            results.iter().all(|r| r.hit_target.is_none()),
            "C++ FindHumanVictim returns no victim when projectile has no shooter"
        );
    }

    /// Apple projectile flying through an APPLE-filtered FX target
    /// yields a `Command::ActivateApple` activation on tick.
    #[test]
    fn tick_arrows_apple_projectile_activates_apple_target() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        use crate::element::{ElementKind, ElementTarget, FxData, TargetData, TargetFilter};

        let target_pos = MapPoint { x: 50.0, y: 0.0 };
        let mut target_element = ElementData {
            kind: ElementKind::Target,
            active: true,
            ..ElementData::default()
        };
        target_element.set_position_map(target_pos);
        // `compute_target_center` reads the 3D position; real loaded
        // targets set both, but `ElementData::default()` leaves position
        // at origin so we mirror position_map.
        target_element.set_position(WorldPoint3D {
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
                position: WorldPoint3D {
                    x: 25.0,
                    y: 0.0,
                    z: 10.0,
                },
                time: 2,
            },
            TrajectoryPoint {
                position: WorldPoint3D {
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
        apple_element.set_position_map(MapPoint { x: 0.0, y: 0.0 });
        apple_element.set_position(WorldPoint3D {
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
                reference: Some(EntityId::Target(crate::entity_id::TargetId(0))),
                ..ObjectData::default()
            },
            projectile: ProjectileData {
                shooter: Some(EntityId::Pc(crate::entity_id::PcId(2))),
                flying: true,
                trajectory,
                ..ProjectileData::default()
            },
        });

        let mut entities = entity_table(vec![Some(target), Some(apple), Some(make_pc(0.0, 0.0))]);

        let mut activation = None;
        let mut impact = None;
        for _ in 0..20 {
            for r in tick_arrows(
                sim,
                &mut entities,
                crate::sight_obstacle::ObstacleList::empty(),
            ) {
                if let Some(hit) = r.fx_target_hit {
                    activation = Some(hit);
                    impact = Some((r.impact_fx, r.impact_pos));
                    break;
                }
            }
            if activation.is_some() {
                break;
            }
        }
        assert_eq!(
            activation,
            Some((
                EntityId::Target(crate::entity_id::TargetId(0)),
                Command::ActivateApple
            )),
            "apple projectile should activate APPLE-filter target with ActivateApple"
        );
        assert_eq!(impact, Some((Some(509), target_pos)));
    }

    /// C++ `FindTargetVictim` uses current-position range gating for
    /// FX targets: a target just beyond the old->new segment endpoint
    /// can still be hit when it is within one movement length of the
    /// arrow's current position.  This catches short final segments
    /// that would otherwise land without activating scripted targets.
    #[test]
    fn tick_arrows_arrow_target_uses_current_position_range_gate() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        use crate::element::{ElementKind, ElementTarget, FxData, TargetData, TargetFilter};

        let mut target_element = ElementData {
            kind: ElementKind::Target,
            active: true,
            ..ElementData::default()
        };
        target_element.set_position_map(MapPoint { x: 40.0, y: 0.0 });
        target_element.set_position(WorldPoint3D {
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
        arrow_element.set_position_map(MapPoint { x: 0.0, y: 0.0 });
        arrow_element.set_position(WorldPoint3D {
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
                reference: Some(EntityId::Target(crate::entity_id::TargetId(0))),
                ..ObjectData::default()
            },
            projectile: ProjectileData {
                shooter: Some(EntityId::Pc(crate::entity_id::PcId(2))),
                flying: true,
                trajectory: vec![TrajectoryPoint {
                    position: WorldPoint3D {
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

        let mut entities = entity_table(vec![Some(target), Some(arrow), Some(make_pc(0.0, 0.0))]);
        let results = tick_arrows(
            sim,
            &mut entities,
            crate::sight_obstacle::ObstacleList::empty(),
        );

        assert!(
            results.iter().any(|r| {
                r.fx_target_hit
                    == Some((
                        EntityId::Target(crate::entity_id::TargetId(0)),
                        Command::ActivateArrow,
                    ))
                    && r.despawn
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
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        use crate::element::{ElementKind, ElementTarget, FxData, TargetData, TargetFilter};

        let mut target_element = ElementData {
            kind: ElementKind::Target,
            active: true,
            ..ElementData::default()
        };
        target_element.set_position_map(MapPoint { x: 10.0, y: 0.0 });
        target_element.set_position(WorldPoint3D {
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
        arrow_element.set_position_map(MapPoint { x: 0.0, y: 0.0 });
        arrow_element.set_position(WorldPoint3D {
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
                reference: Some(EntityId::Pc(crate::entity_id::PcId(0))),
                ..ObjectData::default()
            },
            projectile: ProjectileData {
                shooter: Some(EntityId::Pc(crate::entity_id::PcId(2))),
                flying: true,
                trajectory: vec![TrajectoryPoint {
                    position: WorldPoint3D {
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

        let mut entities = entity_table(vec![Some(target), Some(arrow), Some(make_pc(0.0, 0.0))]);
        let results = tick_arrows(
            sim,
            &mut entities,
            crate::sight_obstacle::ObstacleList::empty(),
        );

        assert!(
            results.iter().all(|r| r.fx_target_hit.is_none()),
            "stationary projectile must not activate nearby FX target by radius"
        );
    }

    #[test]
    fn tick_arrows_has_no_artificial_lifetime_timeout() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let trajectory = (1..=320)
            .map(|i| TrajectoryPoint {
                position: WorldPoint3D {
                    x: i as f32 * 10.0,
                    y: 0.0,
                    z: 40.0,
                },
                time: 1,
            })
            .collect();
        let arrow = spawn_arrow(SpawnArrowParams {
            shooter: EntityId::Pc(crate::entity_id::PcId(0)),
            bow_point: WorldPoint3D {
                x: 0.0,
                y: 0.0,
                z: 40.0,
            },
            trajectory_origin: MapPoint { x: 0.0, y: 0.0 },
            target: EntityId::Soldier(crate::entity_id::SoldierId(1)),
            target_pos: MapPoint { x: 3200.0, y: 0.0 },
            trajectory,
            damage: 30,
            layer: 0,
            lands_in_hole: false,
            initial_velocity: WorldVec3D {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        });
        let mut entities = entity_table(vec![Some(make_pc(0.0, -100.0)), Some(arrow)]);

        let mut despawn_frame = None;
        for frame in 0..260 {
            let results = tick_arrows(
                sim,
                &mut entities,
                crate::sight_obstacle::ObstacleList::empty(),
            );
            if results.iter().any(|r| r.despawn) {
                despawn_frame = Some(frame);
                break;
            }
        }

        assert_eq!(
            despawn_frame, None,
            "C++ projectile lifetime is trajectory-driven, not capped at 250 frames"
        );
        match entities.get_at_index(1).map(|(_, entity)| entity).unwrap() {
            Entity::Projectile(p) => assert!(p.projectile.flying),
            _ => panic!("expected projectile"),
        }
    }

    /// Apple projectile flying through a target that does NOT have the
    /// APPLE filter leaves `fx_target_hit` unset — no activation is
    /// launched.
    #[test]
    fn tick_arrows_apple_projectile_ignores_non_apple_target() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        use crate::element::{ElementKind, ElementTarget, FxData, TargetData, TargetFilter};

        let mut target_element = ElementData {
            kind: ElementKind::Target,
            active: true,
            ..ElementData::default()
        };
        target_element.set_position_map(MapPoint { x: 50.0, y: 0.0 });
        target_element.set_position(WorldPoint3D {
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
            position: WorldPoint3D {
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
                shooter: Some(EntityId::Pc(crate::entity_id::PcId(2))),
                flying: true,
                trajectory,
                ..ProjectileData::default()
            },
        });

        let mut entities = entity_table(vec![Some(target), Some(apple)]);
        let results = tick_arrows(
            sim,
            &mut entities,
            crate::sight_obstacle::ObstacleList::empty(),
        );
        assert!(
            results.is_empty(),
            "C++ FindTargetVictim ignores nonmatching target filters before HitTarget can burst"
        );
    }

    /// Apple impact on an FX target sets the burst animation + decay
    /// row and leaves grounded animation/removal to the derived owner path.
    #[test]
    fn tick_arrows_apple_bursts_then_leaves_grounded_tail_to_virtual_owner() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        use crate::element::{ElementKind, ElementTarget, FxData, TargetData, TargetFilter};

        let mut target_element = ElementData {
            kind: ElementKind::Target,
            active: true,
            ..ElementData::default()
        };
        target_element.set_position_map(MapPoint { x: 10.0, y: 0.0 });
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
                shooter: Some(EntityId::Pc(crate::entity_id::PcId(2))),
                flying: true,
                trajectory: vec![TrajectoryPoint {
                    position: WorldPoint3D {
                        x: 10.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    time: 1,
                }],
                ..ProjectileData::default()
            },
        });
        let mut entities = entity_table(vec![Some(target), Some(apple), Some(make_pc(0.0, 0.0))]);

        // First tick: apple reaches target, bursts.
        let impact_results = tick_arrows(
            sim,
            &mut entities,
            crate::sight_obstacle::ObstacleList::empty(),
        );
        assert!(
            impact_results
                .iter()
                .any(|r| r.fx_target_hit.is_some() && !r.despawn),
            "apple must NOT despawn on impact frame — it bursts first"
        );
        let proj_after = entities.get_at_index(1).map(|(_, entity)| entity).unwrap();
        match proj_after {
            Entity::Projectile(p) => {
                assert!(!p.projectile.flying);
                assert_eq!(p.object.animation, Animation::ObjectBursting);
            }
            _ => panic!("expected apple projectile"),
        }

        let grounded_base_results = tick_arrows(
            sim,
            &mut entities,
            crate::sight_obstacle::ObstacleList::empty(),
        );
        assert!(
            grounded_base_results.is_empty(),
            "Projectile::Hourglass must not duplicate the derived landed animation/removal"
        );
    }

    /// Apple impact yields impact FX 509; stone yields 508; arrow hit
    /// without shield yields no impact FX (silent).
    #[test]
    fn tick_arrows_impact_fx_per_projectile_type() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        fn spawn_projectile_at_impact(obj: ObjectType) -> Entity {
            let mut element = ElementData {
                kind: ElementKind::ObjectProjectile,
                active: true,
                ..ElementData::default()
            };
            element.set_position_map(MapPoint { x: 0.0, y: 0.0 });
            element.set_position(WorldPoint3D {
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
                    shooter: Some(EntityId::Pc(crate::entity_id::PcId(1))),
                    flying: true,
                    // Empty trajectory → immediate "trajectory exhausted".
                    trajectory: Vec::new(),
                    ..ProjectileData::default()
                },
            })
        }

        let fx_for = |obj: ObjectType| -> Option<u32> {
            let mut entities = entity_table(vec![
                Some(spawn_projectile_at_impact(obj)),
                Some(make_pc(100.0, 0.0)),
            ]);
            let results = tick_arrows(
                sim,
                &mut entities,
                crate::sight_obstacle::ObstacleList::empty(),
            );
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
        let start = WorldPoint3D {
            x: 0.0,
            y: 0.0,
            z: 40.0,
        };
        let end = WorldPoint3D {
            x: 100.0,
            y: 0.0,
            z: 20.0,
        };
        let apple = spawn_apple(
            EntityId::Pc(crate::entity_id::PcId(0)),
            start,
            end,
            Some(EntityId::Pc(crate::entity_id::PcId(1))),
            None,
            0,
            None,
        );
        match apple {
            Entity::Projectile(p) => {
                assert!(p.projectile.flying);
                assert_eq!(p.object.object_type, ObjectType::Apple);
                assert_eq!(p.object.associated_action, Action::Apple);
                assert_eq!(p.object.animation, Animation::ObjectFlying);
                assert_eq!(
                    p.projectile.shooter,
                    Some(EntityId::Pc(crate::entity_id::PcId(0)))
                );
                assert_eq!(
                    p.object.reference,
                    Some(EntityId::Pc(crate::entity_id::PcId(1)))
                );
                assert!(!p.projectile.trajectory.is_empty());
            }
            _ => panic!("expected apple projectile"),
        }
    }

    #[test]
    fn apply_arrow_hit_wounds_soldier() {
        let mut entities =
            entity_table(vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))]);
        let died = apply_arrow_hit(
            &mut entities,
            EntityId::Soldier(crate::entity_id::SoldierId(1)),
            EntityId::Pc(crate::entity_id::PcId(0)),
            30,
            0,
        );
        assert!(!died, "30 damage shouldn't kill a 100hp soldier");

        let life = match entities.get_at_index(1).map(|(_, entity)| entity).unwrap() {
            Entity::Soldier(s) => s.npc.life_points,
            _ => unreachable!(),
        };
        assert_eq!(life, 70);
    }

    #[test]
    fn apply_arrow_hit_kills_soldier_at_low_hp() {
        let mut entities =
            entity_table(vec![Some(make_pc(0.0, 0.0)), Some(make_soldier(50.0, 0.0))]);
        if let Some((_, Entity::Soldier(s))) = entities.get_mut_at_index(1) {
            s.npc.life_points = 5;
        }
        let died = apply_arrow_hit(
            &mut entities,
            EntityId::Soldier(crate::entity_id::SoldierId(1)),
            EntityId::Pc(crate::entity_id::PcId(0)),
            30,
            0,
        );
        assert!(died);
        let life = match entities.get_at_index(1).map(|(_, entity)| entity).unwrap() {
            Entity::Soldier(s) => s.npc.life_points,
            _ => unreachable!(),
        };
        assert_eq!(life, 0);
    }

    #[test]
    fn build_shoot_bow_element_produces_interaction_element() {
        let elem = build_shoot_bow_element(
            EntityId::Pc(crate::entity_id::PcId(0)),
            EntityId::Pc(crate::entity_id::PcId(1)),
        );
        assert_eq!(elem.command, Command::ShootBow);
        match &elem.data {
            SequenceElementData::Interaction { antagonist } => {
                assert_eq!(*antagonist, Some(EntityId::Pc(crate::entity_id::PcId(1))));
            }
            other => panic!("expected Interaction, got {:?}", other),
        }
    }

    #[test]
    fn hit_chance_bias_scales_with_skill() {
        // The focused fixture supplies an explicit deterministic context.
        crate::sim_rng::with_seed(1, |sim| {
            if let Some(bias) = roll_hit_and_compute_bias(sim, 0, 90) {
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
    fn equip_bow_sets_aiming_on_animation_start() {
        let mut pc = make_pc(0.0, 0.0);
        pc.actor_data_mut().unwrap().action_state = ActionState::Waiting;

        apply_bow_transition_state_side_effect(
            &mut pc,
            OrderType::TransitionEquipBow,
            SpriteMotionState::Start,
        );

        assert_eq!(
            pc.actor_data().unwrap().action_state,
            ActionState::AimingWithBow,
            "C++ TransitionEquipBow sets AimingWithBow on RHMOTION_START"
        );
    }

    #[test]
    fn equip_and_unload_are_active_bow_transition_orders() {
        assert!(is_bow_transition_order(OrderType::TransitionEquipBow));
        assert!(is_bow_transition_order(
            OrderType::TransitionEquipBowAnonymous
        ));
        assert!(is_bow_transition_order(OrderType::TransitionUnloadBow));
        assert!(is_bow_transition_order(
            OrderType::TransitionUnloadBowAnonymous
        ));
    }

    #[test]
    fn unload_bow_sets_waiting_on_animation_start() {
        let mut pc = make_pc(0.0, 0.0);
        pc.actor_data_mut().unwrap().action_state = ActionState::AimingWithBowDown;

        apply_bow_transition_state_side_effect(
            &mut pc,
            OrderType::TransitionUnloadBow,
            SpriteMotionState::Start,
        );

        assert_eq!(
            pc.actor_data().unwrap().action_state,
            ActionState::Waiting,
            "C++ TransitionUnloadBow sets Waiting on RHMOTION_START"
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
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut pc = make_pc(0.0, 0.0);
        pc.element_data_mut().posture = Posture::LeaningOut;
        bind_test_bow_release_rows(&mut pc, OrderType::ShootingWithBowLeaningOut);
        let mut target = make_soldier(50.0, 0.0);
        target.element_data_mut().posture = Posture::LeaningOut;
        let mut entities = entity_table(vec![Some(pc), Some(target)]);
        let target_id = EntityId::Soldier(crate::entity_id::SoldierId(1));
        let (mut sm, seq_id, elem_idx) =
            launch_test_shoot_element(EntityId::Pc(crate::entity_id::PcId(0)), target_id);

        begin_bow_shot(
            &mut entities,
            &mut sm,
            EntityId::Pc(crate::entity_id::PcId(0)),
            target_id,
            seq_id,
            elem_idx,
            false,
            10,
            Some(ShootMode::Down),
            &mut 1u32,
        );

        let mut fired = Vec::new();
        for _ in 0..16 {
            fired.extend(tick_bow_shots(sim, &mut entities, &mut sm).fired);
            if !fired.is_empty() {
                break;
            }
        }
        assert_eq!(fired.len(), 1);
        assert_eq!(
            entities
                .get_at_index(0)
                .map(|(_, entity)| entity)
                .unwrap()
                .element_data()
                .posture,
            Posture::LeaningOut
        );
        assert_eq!(
            entities
                .get_at_index(0)
                .map(|(_, entity)| entity)
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
        let pos = WorldPoint3D {
            x: 10.0,
            y: 20.0,
            z: 0.0,
        };
        let hand = MapPoint::new(pos.x, pos.y);
        let pt = compute_bow_point(pos, ShootMode::Normal, 0, hand);
        assert_eq!(pt.z, BOW_Z_OFFSET_NORMAL);
        assert_eq!(pt.x, 10.0); // no lateral shift for normal

        let pt_long = compute_bow_point(pos, ShootMode::Long, 0, hand);
        assert_eq!(pt_long.z, BOW_Z_OFFSET_LONG);

        // Down shot should shift laterally by 20 units in direction.
        let pt_down = compute_bow_point(pos, ShootMode::Down, 4, hand);
        assert_eq!(pt_down.z, BOW_Z_OFFSET_NORMAL);
        // Sector 4 = east (+x), so x shifts by ~20
        assert!(pt_down.x > pos.x + 15.0, "down-shot should shift x");

        let diagonal = compute_bow_point(pos, ShootMode::Down, 10, hand);
        let [iso_x, iso_y] = crate::position_interface::sector_to_vector_iso(10);
        let (_, unscaled_y) = crate::element::direction_vector_16(10);
        assert_ne!(iso_y, unscaled_y);
        assert_eq!(diagonal.x, hand.x + iso_x * 20.0);
        assert_eq!(diagonal.y, hand.y + iso_y * 20.0);

        // With non-zero elevation, Z should be elevation + offset,
        // and Y should have elevation added (isometric projection
        // adds elevation into the hand Y).
        let elevated_pos = WorldPoint3D {
            x: 10.0,
            y: 50.0,
            z: 30.0,
        };
        let elevated_hand = MapPoint::new(elevated_pos.x, elevated_pos.y);
        let pt_elev = compute_bow_point(elevated_pos, ShootMode::Normal, 0, elevated_hand);
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

    #[test]
    fn arrow_trajectory_retains_exact_terminal_obstacle_identity() {
        let mut obstacle = compute_shield_obstacle(
            MapPoint::new(0.0, 0.0),
            0.0,
            4,
            &ShieldParams {
                pre_offset: 0.0,
                width: 100.0,
                depth: 5.0,
                height: 100.0,
                z_offset: 0.0,
            },
        );
        // The trajectory raycast skips shield obstacles entirely (shield
        // blocking is the per-arrow shield-holder test, not the obstacle
        // grid), so make this wall a plain solid to stay visible to it.
        obstacle.set_flag(crate::sight_obstacle::SIGHTOBSTACLE_SHIELD, false);
        let obstacles = [obstacle];
        // The trajectory raycast forces a bare ground impact for any origin
        // outside the level's map bbox, and a default grid has an empty
        // (hyperspace) bbox — give the flight path an open field instead.
        let mut grid = crate::fast_find_grid::FastFindGrid::default();
        {
            let mut level = (*grid.level).clone();
            level.map_bbox =
                crate::coordinates::MapBBox::from_coords(-10_000.0, -10_000.0, 10_000.0, 10_000.0);
            grid.level = std::sync::Arc::new(level);
        }
        let check = TrajectoryObstacleCheck {
            fast_find_grid: &grid,
            layer: 0,
            sight_obstacles: crate::sight_obstacle::ObstacleList::from_slice_all_active(&obstacles),
            water_zones: None,
        };

        let (trajectory, terminal_obstacle) = compute_trajectory_ballistic_with_terminal_obstacle(
            WorldPoint3D {
                x: 0.0,
                y: 0.0,
                z: 25.0,
            },
            WorldVec3D {
                x: 10.0,
                y: 0.0,
                z: 0.0,
            },
            MASS_ARROW_FLAT,
            false,
            Some(&check),
        );

        assert!(!trajectory.is_empty());
        assert_eq!(terminal_obstacle.map(u16::from), Some(0));
    }

    #[test]
    fn arrow_trajectory_reports_exact_ground_impact_without_an_obstacle() {
        let mut grid = crate::fast_find_grid::FastFindGrid::default();
        {
            let mut level = (*grid.level).clone();
            level.map_bbox =
                crate::coordinates::MapBBox::from_coords(-10_000.0, -10_000.0, 10_000.0, 10_000.0);
            grid.level = std::sync::Arc::new(level);
        }
        let check = TrajectoryObstacleCheck {
            fast_find_grid: &grid,
            layer: 0,
            sight_obstacles: crate::sight_obstacle::ObstacleList::empty(),
            water_zones: None,
        };

        let (trajectory, terminal_obstacle, terminal_impact) =
            compute_trajectory_ballistic_with_terminal_impact(
                WorldPoint3D::new(0.0, 0.0, 25.0),
                WorldVec3D {
                    x: 10.0,
                    y: 0.0,
                    z: 0.0,
                },
                MASS_ARROW_HIGH,
                false,
                Some(&check),
            );

        assert!(terminal_impact);
        assert_eq!(terminal_obstacle, None);
        assert_eq!(trajectory.last().unwrap().position.z, 0.0);
    }

    /// A projectile that passes close to a target on the ground (not
    /// airborne) still misses when the target's posture is one of the
    /// "untargetable" postures.  Spot-check one of them
    /// (`Posture::Lying`) to confirm the filter actually prunes the
    /// snapshot.
    #[test]
    fn tick_arrows_skips_lying_victim() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        use crate::element::Posture;

        let mut soldier = make_soldier(50.0, 0.0);
        soldier.set_posture(Posture::Lying);

        // Arrow trajectory aimed directly at where the belt would be
        // if the soldier were upright — but since it's lying, no hit.
        let trajectory = vec![TrajectoryPoint {
            position: WorldPoint3D {
                x: 50.0,
                y: 0.0,
                z: 25.0,
            },
            time: 2,
        }];
        let arrow = spawn_arrow(SpawnArrowParams {
            shooter: EntityId::Pc(crate::entity_id::PcId(0)),
            bow_point: WorldPoint3D {
                x: 0.0,
                y: 0.0,
                z: 25.0,
            },
            trajectory_origin: MapPoint { x: 0.0, y: 0.0 },
            target: EntityId::Pc(crate::entity_id::PcId(1)),
            target_pos: MapPoint { x: 50.0, y: 0.0 },
            trajectory,
            damage: 30,
            layer: 0,
            lands_in_hole: false,
            initial_velocity: WorldVec3D {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        });

        let mut entities = entity_table(vec![Some(make_pc(0.0, 0.0)), Some(soldier), Some(arrow)]);

        let mut any_hit = None;
        for _ in 0..10 {
            for r in tick_arrows(
                sim,
                &mut entities,
                crate::sight_obstacle::ObstacleList::empty(),
            ) {
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
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        // Arrow stays well above the soldier's belt (Z=25).
        let trajectory = vec![
            TrajectoryPoint {
                position: WorldPoint3D {
                    x: 30.0,
                    y: 0.0,
                    z: 80.0,
                },
                time: 2,
            },
            TrajectoryPoint {
                position: WorldPoint3D {
                    x: 60.0,
                    y: 0.0,
                    z: 78.0,
                },
                time: 2,
            },
            TrajectoryPoint {
                position: WorldPoint3D {
                    x: 90.0,
                    y: 0.0,
                    z: 76.0,
                },
                time: 2,
            },
        ];
        let arrow = spawn_arrow(SpawnArrowParams {
            shooter: EntityId::Pc(crate::entity_id::PcId(0)),
            bow_point: WorldPoint3D {
                x: 0.0,
                y: 0.0,
                z: 82.0,
            },
            trajectory_origin: MapPoint { x: 0.0, y: 0.0 },
            target: EntityId::Pc(crate::entity_id::PcId(1)),
            target_pos: MapPoint { x: 90.0, y: 0.0 },
            trajectory,
            damage: 30,
            layer: 0,
            lands_in_hole: false,
            initial_velocity: WorldVec3D {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        });

        let mut entities = entity_table(vec![
            Some(make_pc(0.0, 0.0)),
            Some(make_soldier(50.0, 0.0)),
            Some(arrow),
        ]);

        let mut any_hit = None;
        for _ in 0..20 {
            for r in tick_arrows(
                sim,
                &mut entities,
                crate::sight_obstacle::ObstacleList::empty(),
            ) {
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
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let trajectory = vec![
            TrajectoryPoint {
                position: WorldPoint3D {
                    x: 50.0,
                    y: 0.0,
                    z: 25.0,
                },
                time: 2,
            },
            TrajectoryPoint {
                position: WorldPoint3D {
                    x: 80.0,
                    y: 0.0,
                    z: 20.0,
                },
                time: 2,
            },
        ];
        let arrow = spawn_arrow(SpawnArrowParams {
            shooter: EntityId::Pc(crate::entity_id::PcId(0)),
            bow_point: WorldPoint3D {
                x: 0.0,
                y: 0.0,
                z: 30.0,
            },
            trajectory_origin: MapPoint { x: 0.0, y: 0.0 },
            target: EntityId::Pc(crate::entity_id::PcId(1)),
            target_pos: MapPoint { x: 80.0, y: 0.0 },
            trajectory,
            damage: 30,
            layer: 0,
            lands_in_hole: false,
            initial_velocity: WorldVec3D {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        });
        let mut entities = entity_table(vec![
            Some(make_pc(0.0, 0.0)),
            Some(make_soldier(20.0, 0.0)),
            Some(arrow),
        ]);
        let mut hit = None;
        for _ in 0..20 {
            for r in tick_arrows(
                sim,
                &mut entities,
                crate::sight_obstacle::ObstacleList::empty(),
            ) {
                if r.hit_target.is_some() {
                    hit = r.hit_target;
                }
            }
            if hit.is_some() {
                break;
            }
        }
        assert_eq!(hit, Some(EntityId::Soldier(crate::entity_id::SoldierId(1))));
    }

    /// Shield intersection flips the projectile into the falling state
    /// and emits a `shield_hit` result.  The projectile keeps flying
    /// on a new deflected trajectory toward the ground — it must not
    /// despawn on the same tick.
    #[test]
    fn tick_arrows_shield_hit_deflects_and_keeps_flying() {
        crate::sim_rng::with_seed(1, |sim| {
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
                let obs = compute_shield_obstacle(MapPoint { x: 50.0, y: 0.0 }, 0.0, 4, &params);
                actor.shield_obstacle = Some(obs);
            }
            shield_holder.element_data_mut().set_direction_instantly(4);

            // Arrow flying from +X toward the shield holder at Z=40 —
            // mid-shield height for `shield_params_for_soldier(20, 40)`
            // which places the quad between Z=30 and Z=50.  The holder
            // stands at ground Y=0, so the arrow shares that ground Y and
            // clears the quad only on height, which the Z extent decides.
            let trajectory = vec![TrajectoryPoint {
                position: WorldPoint3D {
                    x: 50.0,
                    y: 0.0,
                    z: 40.0,
                },
                time: 2,
            }];
            let arrow = spawn_arrow(SpawnArrowParams {
                shooter: EntityId::Pc(crate::entity_id::PcId(0)),
                bow_point: WorldPoint3D {
                    x: 100.0,
                    y: 0.0,
                    z: 40.0,
                },
                trajectory_origin: MapPoint { x: 100.0, y: 0.0 },
                target: EntityId::Soldier(crate::entity_id::SoldierId(1)),
                target_pos: MapPoint { x: 50.0, y: 0.0 },
                trajectory,
                damage: 30,
                layer: 0,
                lands_in_hole: false,
                initial_velocity: WorldVec3D {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
            });

            let mut entities = entity_table(vec![
                Some(make_pc(100.0, 0.0)),
                Some(shield_holder),
                Some(arrow),
            ]);

            // Advance ticks until the shield_hit fires.
            let mut shield_hit = None;
            let mut despawn_seen = false;
            for _ in 0..10 {
                for r in tick_arrows(
                    sim,
                    &mut entities,
                    crate::sight_obstacle::ObstacleList::empty(),
                ) {
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
                Some(EntityId::Soldier(crate::entity_id::SoldierId(1))),
                "arrow must report shield hit on the holder"
            );
            assert!(
                !despawn_seen,
                "shield-hit arrow keeps flying (falling) on same tick"
            );

            // The projectile should be flagged as falling, and the hit
            // check must now skip (falling arrows pass through bodies).
            match entities.get_at_index(2).map(|(_, entity)| entity).unwrap() {
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
                        (WorldPoint3D {
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
        crate::sim_rng::with_seed(1, |sim| {
            // Two waypoints: the spawn primer consumes the first segment, so
            // the ricochet still sees a queued waypoint and derives its fall
            // sector from live flight rather than the orientation cache.
            let trajectory = vec![
                TrajectoryPoint {
                    position: WorldPoint3D {
                        x: 25.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    time: 1,
                },
                TrajectoryPoint {
                    position: WorldPoint3D {
                        x: 50.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    time: 2,
                },
            ];
            let arrow = spawn_arrow(SpawnArrowParams {
                shooter: EntityId::Pc(crate::entity_id::PcId(0)),
                bow_point: WorldPoint3D {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                trajectory_origin: MapPoint { x: 0.0, y: 0.0 },
                target: EntityId::Pc(crate::entity_id::PcId(1)),
                target_pos: MapPoint { x: 50.0, y: 0.0 },
                trajectory,
                damage: 30,
                layer: 0,
                lands_in_hole: false,
                initial_velocity: WorldVec3D {
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

            make_arrow_falling_down(sim, &mut projectile, false, None);

            assert!(projectile.projectile.falling);
            assert!(projectile.projectile.flying);
            assert_eq!(
                projectile.projectile.falling_direction, 12,
                "armor ricochet reverses the flight sector for the fall"
            );
            assert_ne!(
                projectile.element.position(),
                impact_position,
                "C++ MakeFallingDown calls Hourglass for armor ricochets too"
            );

            // The tumble visual is a presentation pass: it renders on the
            // deferred refresh before the next hourglass, not inside
            // MakeFallingDown itself.
            refresh_arrow_after_previous_hourglass(sim, &mut projectile);
            assert_eq!(
                projectile.element.sprite.current_row, 12,
                "impact-frame render uses the first falling sector"
            );
            assert!((3..=5).contains(&projectile.element.sprite.current_frame));
            assert_eq!(
                projectile.projectile.falling_direction, 10,
                "falling refresh rotates the next tumble sector by -2"
            );
        });
    }

    /// An arrow that runs out of trajectory without hitting anything
    /// stops flying on the landing tick and despawns.
    #[test]
    fn tick_arrows_miss_and_land_despawns() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let trajectory = vec![TrajectoryPoint {
            position: WorldPoint3D {
                x: 10.0,
                y: 0.0,
                z: 0.0,
            },
            time: 1,
        }];
        let arrow = spawn_arrow(SpawnArrowParams {
            shooter: EntityId::Pc(crate::entity_id::PcId(0)),
            bow_point: WorldPoint3D {
                x: 0.0,
                y: 0.0,
                z: 5.0,
            },
            trajectory_origin: MapPoint { x: 0.0, y: 0.0 },
            target: EntityId::Pc(crate::entity_id::PcId(0)),
            target_pos: MapPoint { x: 10.0, y: 0.0 },
            trajectory,
            damage: 30,
            layer: 0,
            lands_in_hole: false,
            initial_velocity: WorldVec3D {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        });
        // No other humans in range — arrow will fly out and land.
        let mut entities = entity_table(vec![Some(make_pc(0.0, 0.0)), Some(arrow)]);

        let mut despawn = false;
        for _ in 0..10 {
            for r in tick_arrows(
                sim,
                &mut entities,
                crate::sight_obstacle::ObstacleList::empty(),
            ) {
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

    #[test]
    fn one_waypoint_falling_arrow_into_hole_disappears_without_ground_snap() {
        let endpoint = WorldPoint3D::new(10.0, 0.0, -0.5);
        let endpoint_map = endpoint.to_map();
        let water_zones = crate::water_zones::WaterZones {
            zones: vec![crate::water_zones::WaterZone {
                points: vec![
                    MapPoint::new(0.0, -10.0),
                    MapPoint::new(20.0, -10.0),
                    MapPoint::new(20.0, 10.0),
                    MapPoint::new(0.0, 10.0),
                ],
                bounding_box: crate::coordinates::MapBBox::from_coords(0.0, -10.0, 20.0, 10.0),
                material: crate::sound_cache::Material::Hole,
            }],
        };
        assert!(water_zones.landing_is_in_hole(endpoint_map));

        let Entity::Projectile(mut arrow) = spawn_arrow(SpawnArrowParams {
            shooter: EntityId::Pc(crate::entity_id::PcId(0)),
            bow_point: WorldPoint3D::new(0.0, 0.0, 5.0),
            trajectory_origin: MapPoint::new(0.0, 0.0),
            target: EntityId::Pc(crate::entity_id::PcId(0)),
            target_pos: endpoint_map,
            trajectory: vec![],
            damage: 30,
            layer: 0,
            lands_in_hole: false,
            initial_velocity: WorldVec3D::new(1.0, 0.0, 0.0),
        }) else {
            panic!("spawn_arrow returned a non-projectile entity");
        };
        arrow.projectile.trajectory = vec![TrajectoryPoint {
            position: endpoint,
            time: 1,
        }];
        arrow.projectile.flying = true;
        arrow.projectile.launch_segment_start = None;
        preserve_falling_hole_disappearance(&mut arrow, &water_zones);
        assert!(
            arrow.projectile.disappear,
            "AddTrajectoryFallIntoHole marks even a one-waypoint trajectory"
        );

        arrow.advance_trajectory_one_frame();
        assert_eq!(arrow.element.position().z.to_bits(), endpoint.z.to_bits());
        let mut entities = entity_table(vec![
            Some(make_pc(100.0, 100.0)),
            Some(Entity::Projectile(arrow)),
        ]);
        let results = tick_arrows(
            &crate::sim_rng::test_context(),
            &mut entities,
            crate::sight_obstacle::ObstacleList::empty(),
        );

        assert!(results.iter().any(|result| result.despawn));
        let Entity::Projectile(arrow) = entities.get_at_index(1).unwrap().1 else {
            panic!("falling arrow changed concrete entity kind");
        };
        assert!(!arrow.projectile.flying);
        assert_eq!(
            arrow.element.position().z.to_bits(),
            endpoint.z.to_bits(),
            "mbDisappear returns before HitObstacle's +0.001 elevation snap"
        );
        assert!(!arrow.element.sprite.position_iface.is_moving());
    }

    /// Wasp nest thrown at a ground target bursts (`flying == false`)
    /// once its bounce trajectory is exhausted.  Unlike arrows, the
    /// nest keeps a projectile slot for the post-impact wasp swarm
    /// spawn — here we just assert it stops flying.
    #[test]
    fn spawn_wasp_nest_lands_and_stops_flying() {
        let throw_pos = WorldPoint3D {
            x: 0.0,
            y: 0.0,
            z: 50.0,
        };
        let target_pos = WorldPoint3D {
            x: 80.0,
            y: 0.0,
            z: 0.0,
        };
        let nest = spawn_wasp_nest(
            EntityId::Pc(crate::entity_id::PcId(0)),
            throw_pos,
            target_pos,
            0,
            None,
        );

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

        let mut entities = entity_table(vec![Some(make_pc(0.0, 0.0)), Some(nest)]);
        // Wasp nests are skipped by `tick_arrows` (their impact burst +
        // swarm spawn lives on the engine in `tick_wasp_nests`).  Drive
        // the trajectory directly here via `advance_trajectory_one_frame`;
        // bouncing nests can produce the full 50-waypoint trajectory
        // (~100 ticks at TIME_FLYSEGMENT=2), so 300 iterations is a
        // generous bound.
        for _ in 0..300 {
            if let Some(Entity::Projectile(p)) =
                entities.get_mut_at_index(1).map(|(_, entity)| entity)
            {
                if !p.projectile.flying {
                    break;
                }
                p.advance_trajectory_one_frame();
            }
        }
        let p = match entities.get_at_index(1).map(|(_, entity)| entity).unwrap() {
            Entity::Projectile(p) => p,
            _ => panic!("nest entity lost"),
        };
        assert!(
            !p.projectile.flying,
            "wasp nest must stop flying once its trajectory is exhausted"
        );
    }

    #[test]
    fn every_thrown_object_path_is_primed_exactly_once_by_spawn() {
        let thrower = EntityId::Pc(crate::entity_id::PcId(0));
        let start = WorldPoint3D::new(0.0, 0.0, 20.0);
        let end = WorldPoint3D::new(200.0, 0.0, 0.0);
        let thrown = [
            spawn_net(thrower, start, end, 0, None),
            spawn_wasp_nest(thrower, start, end, 0, None),
            spawn_purse(thrower, start, end, 0, None),
            spawn_apple(thrower, start, end, Some(thrower), None, 0, None),
            spawn_stone(thrower, start, end, Some(thrower), None, 0, None),
            spawn_coin(None, start, end, 0, 0, None, APEX_BEGGAR_COIN, None),
        ];
        for (index, entity) in thrown.into_iter().enumerate() {
            let (position, frame_count) = match entity {
                Entity::Projectile(projectile) => (
                    projectile.element.position(),
                    projectile.projectile.frame_count,
                ),
                Entity::Net(net) => (net.element.position(), net.projectile.frame_count),
                _ => unreachable!(),
            };
            assert_ne!(
                position, start,
                "throw path {index} omitted its explicit primer"
            );
            assert_eq!(
                frame_count, 1,
                "throw path {index} advanced more than once before insertion"
            );
        }
    }

    fn refresh_test_arrow() -> ElementProjectile {
        let mut element = ElementData {
            kind: ElementKind::ObjectProjectile,
            active: true,
            ..Default::default()
        };
        element.sprite.current_row = 9;
        element.sprite.current_frame = 2;
        ElementProjectile {
            element,
            object: ObjectData {
                object_type: ObjectType::Arrow,
                animation: Animation::ObjectFlying,
                ..Default::default()
            },
            projectile: ProjectileData {
                flying: true,
                trajectory: vec![TrajectoryPoint {
                    position: WorldPoint3D::new(10.0, 0.0, 100.0),
                    time: 4,
                }],
                // Deliberately horizontal: Refresh must use the next queued
                // point rather than this current-segment increment.
                velocity_increment: WorldVec3D::new(1.0, 0.0, 0.0),
                ..Default::default()
            },
        }
    }

    #[test]
    fn arrow_refresh_is_deferred_and_uses_next_waypoint_pitch() {
        let mut arrow = refresh_test_arrow();
        assert_eq!(
            (
                arrow.element.sprite.current_row,
                arrow.element.sprite.current_frame
            ),
            (9, 2)
        );

        refresh_arrow_after_previous_hourglass(&crate::sim_rng::test_context(), &mut arrow);

        // The +X ground direction lies in compass sector 4 (the original
        // sector partition puts (0,-1) in sector 0 and (1,0) in sector 4).
        assert_eq!(arrow.projectile.last_orientation_sector, 4);
        assert_eq!(arrow.projectile.last_orientation_azimuth, 60);
        assert_eq!(
            (
                arrow.element.sprite.current_row,
                arrow.element.sprite.current_frame
            ),
            (4, 8)
        );
    }

    #[test]
    fn falling_arrow_refresh_consumes_exactly_one_draw_and_rotates_afterward() {
        let mut arrow = refresh_test_arrow();
        arrow.projectile.falling = true;
        arrow.projectile.falling_direction = 6;

        let (_, draws) = crate::sim_rng::with_draw_trace(|| {
            refresh_arrow_after_previous_hourglass(&crate::sim_rng::test_context(), &mut arrow)
        });

        assert_eq!(draws, vec![crate::sim_rng::RngSite::ArrowFallingFrame]);
        assert_eq!(arrow.element.sprite.current_row, 6);
        assert!((3..=5).contains(&arrow.element.sprite.current_frame));
        assert_eq!(arrow.projectile.falling_direction, 4);
    }

    #[test]
    fn live_flying_arrow_with_world_movement_reuses_orientation_cache() {
        let mut arrow = refresh_test_arrow();
        arrow.projectile.trajectory.clear();
        arrow.projectile.trajectory_frame_count = 0;
        arrow.projectile.last_orientation_sector = 7;
        arrow.projectile.last_orientation_azimuth = -30;
        arrow
            .element
            .sprite
            .position_iface
            .set_old_position(WorldPoint3D::new(-1.0, 0.0, 0.0));

        refresh_arrow_after_previous_hourglass(&crate::sim_rng::test_context(), &mut arrow);

        assert!(arrow.element.active);
        assert_eq!(
            (
                arrow.element.sprite.current_row,
                arrow.element.sprite.current_frame
            ),
            (7, 3)
        );

        // The exhausted trajectory's next Hourglass stops flight and snaps
        // the landing height. Original exposes that movement for one more
        // active snapshot, then retires it on the following Refresh.
        arrow.projectile.flying = false;
        refresh_arrow_after_previous_hourglass(&crate::sim_rng::test_context(), &mut arrow);
        assert!(arrow.element.active);
        assert!(!arrow.element.sprite.position_iface.is_moving());
        refresh_arrow_after_previous_hourglass(&crate::sim_rng::test_context(), &mut arrow);
        assert!(!arrow.element.active);
    }

    #[test]
    fn stopped_empty_trajectory_retires_without_refresh_rng() {
        let mut arrow = refresh_test_arrow();
        arrow.projectile.trajectory.clear();
        arrow.projectile.falling = true;
        arrow.projectile.flying = false;
        arrow.projectile.trajectory_frame_count = 3;
        // Model the terminal landing normalization. It mutates the eager
        // world position after flight has stopped, but is not another flight
        // segment that keeps the arrow alive.
        arrow
            .element
            .sprite
            .position_iface
            .set_old_position(WorldPoint3D::new(-1.0, 0.0, 0.0));

        let (_, draws) = crate::sim_rng::with_draw_trace(|| {
            refresh_arrow_after_previous_hourglass(&crate::sim_rng::test_context(), &mut arrow)
        });

        assert!(!arrow.element.active);
        assert!(draws.is_empty());
    }

    #[test]
    fn flying_endpoint_renders_final_falling_frame_before_retirement() {
        let mut arrow = refresh_test_arrow();
        arrow.projectile.trajectory.clear();
        arrow.projectile.trajectory_frame_count = 0;
        arrow.projectile.falling = true;

        let (_, draws) = crate::sim_rng::with_draw_trace(|| {
            refresh_arrow_after_previous_hourglass(&crate::sim_rng::test_context(), &mut arrow)
        });

        assert!(!arrow.element.active);
        assert_eq!(draws, vec![crate::sim_rng::RngSite::ArrowFallingFrame]);
        assert!(!arrow.element.sprite.position_iface.is_moving());
        assert!(!arrow.element.sprite.position_iface.is_moving_map());
    }

    #[test]
    fn non_falling_flying_endpoint_waits_for_projectile_hourglass() {
        let mut arrow = refresh_test_arrow();
        arrow.projectile.trajectory.clear();
        arrow.projectile.trajectory_frame_count = 0;
        arrow.projectile.falling = false;

        refresh_arrow_after_previous_hourglass(&crate::sim_rng::test_context(), &mut arrow);

        assert!(
            arrow.element.active,
            "ordinary arrows process the empty trajectory in Projectile::Hourglass"
        );
        assert!(arrow.projectile.flying);
    }
}
