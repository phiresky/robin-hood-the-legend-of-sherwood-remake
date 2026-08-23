//! Ballistic integration, terminal-impact metadata, and bow aiming math.

use super::*;

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
    // Small-throwable owners currently retain only obstacle identity. Their
    // distinct landing lifecycles resolve presentation later; do not smuggle
    // arrow disappearance semantics through this compatibility API.
    let (trajectory, terminal_obstacle, _, _, _, _) = compute_trajectory_ballistic_impl(
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
    bool,
    bool,
) {
    let (trajectory, obstacle, impact, hole, water, _) = compute_trajectory_ballistic_impl(
        start,
        initial_velocity,
        mass,
        flat_shot,
        obstacle_check,
        None,
    );
    (trajectory, obstacle, impact, hole, water)
}

pub(super) fn compute_trajectory_ballistic_with_terminal_metadata(
    start: WorldPoint3D,
    initial_velocity: WorldVec3D,
    mass: f32,
    flat_shot: bool,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
) -> (
    Vec<TrajectoryPoint>,
    Option<crate::position_interface::ObstacleHandle>,
    bool,
    bool,
    bool,
    Option<usize>,
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
    // Net/wasp/coin lifecycles do not consume ProjectileData::disappear.
    // Keep scoped material handling inside integration (so water/hole stops
    // the bounce and exact hole geometry drives extension), while exposing
    // no unused flag until those Original lifecycles are source-proven.
    compute_trajectory_ballistic_bounce_with_terminal(
        start,
        initial_velocity,
        mass,
        flat_shot,
        obstacle_check,
        bounce_factors,
    )
    .0
}

pub(super) fn compute_trajectory_ballistic_bounce_with_terminal(
    start: WorldPoint3D,
    initial_velocity: WorldVec3D,
    mass: f32,
    flat_shot: bool,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
    bounce_factors: (f32, f32),
) -> (
    Vec<TrajectoryPoint>,
    Option<crate::position_interface::ObstacleHandle>,
    bool,
    bool,
    bool,
) {
    let (trajectory, obstacle, impact, hole, water, _) = compute_trajectory_ballistic_impl(
        start,
        initial_velocity,
        mass,
        flat_shot,
        obstacle_check,
        Some(bounce_factors),
    );
    (trajectory, obstacle, impact, hole, water)
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

pub(super) fn projectile_impact_ratio(
    position: WorldPoint3D,
    new_position: WorldPoint3D,
    impact: WorldPoint3D,
) -> f32 {
    let seg_dx = new_position.x - position.x;
    let seg_dy = new_position.y - position.y;
    let seg_dz = new_position.z - position.z;
    let seg_len_sq = seg_dx * seg_dx + seg_dy * seg_dy + seg_dz * seg_dz;
    let ix = impact.x - position.x;
    let iy = impact.y - position.y;
    let iz = impact.z - position.z;
    ((ix * ix + iy * iy + iz * iz) / seg_len_sq).sqrt()
}

pub(super) fn compute_trajectory_ballistic_impl(
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
    bool,
    bool,
    Option<usize>,
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
    let mut terminal_lands_in_hole = false;
    let mut terminal_lands_in_water = false;
    let mut terminal_impact_index = None;
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
            let candidates = check.fast_find_grid.impact_obstacle_candidates(
                crate::coordinates::MapPoint::new(position.x, position.y),
                crate::coordinates::MapPoint::new(new_position.x, new_position.y),
                check.sight_obstacles,
                crate::sight_obstacle::SIGHTOBSTACLE_SOLID,
            );
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
                Some(&candidates),
            );

            // Compute the 3D impact ratio relative to the current
            // segment.  C++ projectile trajectory integration uses
            // `RHFastFindGrid::IsReachableImpact` over sight obstacles;
            // it does not collide with pathfinding/motion-line
            // geometry.  Keep this path sight-obstacle-only.
            // RHElementProjectile::ComputeTrajectory uses the ratio of
            // Euclidean lengths, not a projection of the impact onto the
            // intended segment.  The distinction matters when collision
            // geometry returns a point slightly off the segment: it can
            // change the rounded flight time.
            let ratio_3d = impact_3d.as_ref().map(|r| {
                projectile_impact_ratio(
                    position,
                    new_position,
                    WorldPoint3D::new(r.impact.x, r.impact.y, r.impact.z),
                )
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
            let impact_obstacle = r.obstacle_index.map(|index| {
                let index = u16::try_from(index)
                    .expect("projectile obstacle index does not fit ObstacleHandle");
                crate::position_interface::ObstacleHandle::new(index)
                    .expect("projectile obstacle index used reserved sentinel")
            });
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
            terminal_impact_index = Some(trajectory.len() - 1);

            let water_hole = check.water_zones.and_then(|water_zones| {
                crate::water_zones::determine_water_hole_scoped(
                    water_zones,
                    obstacle,
                    MapPoint::from_world_xyz(impact.x, impact.y, impact.z),
                )
            });
            terminal_lands_in_hole = water_hole.is_some_and(|resolution| {
                matches!(resolution.material, crate::sound_cache::Material::Hole)
            });
            terminal_lands_in_water = water_hole.is_some_and(|resolution| {
                matches!(resolution.material, crate::sound_cache::Material::Water)
            });

            if let Some(proj_bounce) = bounce {
                let info = classify_impact(impact, position, new_position, obstacle);
                let new_vel = apply_bounce_reflection(velocity, new_vz, ratio, info, proj_bounce);

                // Water/hole impacts terminate bounce integration in
                // Original. Hole disappearance is returned to the owner;
                // water presentation remains owned by each projectile's
                // existing landing lifecycle.
                if water_hole.is_some() {
                    terminal_obstacle = impact_obstacle;
                    terminal_impact = true;
                    break;
                }

                velocity = new_vel;
                position = impact;
                // Record the impact kind so the next iteration's
                // termination check can fire.
                last_impact = Some(info);
                continue;
            }
            terminal_obstacle = impact_obstacle;
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
        && terminal_lands_in_hole
        && let Some(water_zones) = check.water_zones
        && trajectory.len() >= 2
    {
        let last = trajectory[trajectory.len() - 1].position;
        let prev = trajectory[trajectory.len() - 2].position;
        let landing_map = MapPoint::from_world_xyz(last.x, last.y, last.z);
        let prev_map = MapPoint::from_world_xyz(prev.x, prev.y, prev.z);
        let obstacle = terminal_obstacle.map(|handle| {
            check
                .sight_obstacles
                .get(usize::from(handle))
                .unwrap_or_else(|| panic!("terminal obstacle {handle} disappeared"))
        });
        let resolution =
            crate::water_zones::determine_water_hole_scoped(water_zones, obstacle, landing_map)
                .expect("terminal hole classification lost its material sector");
        let sector_points = resolution
            .sector_points
            .expect("hole classification did not retain its exact sector polygon");
        if let Some(exit) =
            crate::water_zones::find_hole_far_exit_in_sector(sector_points, prev_map, landing_map)
        {
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

    (
        trajectory,
        terminal_obstacle,
        terminal_impact,
        terminal_lands_in_hole,
        terminal_lands_in_water,
        terminal_impact_index,
    )
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
    // Original uses the raw ULONG capacity here. Normal experience gain caps
    // it at 100, but SetCapacity does not, so preserve negative factors for
    // explicitly authored/scripted capacities above 100.
    let skill_factor = bow_miss_skill_factor(bow_skill_capacity);
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

#[inline]
pub(super) fn bow_miss_skill_factor(bow_skill_capacity: u32) -> f32 {
    1.0 - bow_skill_capacity as f32 / 100.0
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
