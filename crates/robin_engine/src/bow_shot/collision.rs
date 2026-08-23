//! Shield geometry used by projectile collision checks.

use super::*;

// ═══════════════════════════════════════════════════════════════════
//  Shield obstacle — arrow blocking geometry
// ═══════════════════════════════════════════════════════════════════

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

    let (dir_x, dir_y_unscaled) = direction_vector_16(direction_sector);
    // GetDirectionVector constructs the caller's direction with
    // SetSector0to15(..., ASPECT_RATIO). The caller uses that vector for its
    // first offset, then UpdateBox normalizes the already-compressed vector
    // before applying ASPECT_RATIO to its Y components once more.
    let dir_y = dir_y_unscaled * ASPECT_RATIO;

    // Pre-offset: move position forward in the facing direction
    // before constructing the shield box.
    let px = position_ground.x + params.pre_offset * dir_x;
    let py = position_ground.y + params.pre_offset * dir_y;
    let pz = z + params.z_offset;

    // RHSightObstacle::UpdateBox normalizes its compressed input in ordinary
    // Euclidean space before deriving the perpendicular.
    let direction_norm = (dir_x * dir_x + dir_y * dir_y).sqrt();
    assert!(direction_norm > 0.0, "16-sector shield direction is zero");
    let fwd_x = dir_x / direction_norm;
    let fwd_y = dir_y / direction_norm;

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

/// Refresh retained shield geometry at an Original `UpdateShield` call site.
/// Projectile ticks consume this value without deriving it again.
pub(crate) fn refresh_retained_shield_obstacle(entity: &mut Entity, profiles: &ProfileManager) {
    let params = match entity {
        Entity::Pc(pc) => {
            let profile = profiles
                .get_character(pc.pc.profile_index)
                .unwrap_or_else(|| {
                    panic!(
                        "missing character profile {:?} while refreshing shield",
                        pc.pc.profile_index
                    )
                });
            let has_big_shield = profile.has_action(Action::BigShield);
            shield_params_for_pc(has_big_shield)
        }
        Entity::Soldier(soldier) => {
            let profile = profiles
                .get_soldier(soldier.soldier.soldier_profile_index)
                .unwrap_or_else(|| {
                    panic!(
                        "missing soldier profile {:?} while refreshing shield",
                        soldier.soldier.soldier_profile_index
                    )
                });
            let weapon = profiles
                .get_hth_weapon(profile.hth_weapon_id)
                .unwrap_or_else(|| {
                    panic!(
                        "missing HtH weapon profile {:?} while refreshing shield",
                        profile.hth_weapon_id
                    )
                });
            shield_params_for_soldier(weapon.shield_width, weapon.shield_height)
        }
        _ => panic!("shield geometry requested for non-shield-bearing entity"),
    };

    let elem = entity.element_data();
    let elevation = elem.position().z;
    let position_map = elem.position_map();
    let obstacle = compute_shield_obstacle(
        MapPoint {
            x: position_map.x,
            y: position_map.y + elevation,
        },
        elevation,
        elem.direction(),
        &params,
    );
    entity
        .actor_data_mut()
        .expect("shield-bearing entity must have actor data")
        .shield_obstacle = Some(obstacle);
}
