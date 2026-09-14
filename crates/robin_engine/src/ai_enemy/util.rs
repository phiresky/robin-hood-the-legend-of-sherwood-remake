//! Shared helpers for the enemy-AI module: math, snapshots, combat-position
//! evaluation, ambush-point status, and tunable constants.

use bitflags::bitflags;
use serde::{Deserialize, Serialize};

use super::map_vec_ext::AiMapVec;
use crate::ai::*;
use crate::coordinates::MapVec;
use crate::element::EntityId;
use crate::position_interface::INVERSE_ASPECT_RATIO;

// ---------------------------------------------------------------------------
// Task priority constants
// ---------------------------------------------------------------------------

/// Task priorities determine which stimuli can interrupt the current behavior.
/// Higher values = higher priority.
pub mod task_priority {
    pub const NONE: u16 = 0;
    pub const FUNNY_THING: u16 = 1;
    pub const STRANGE_THING: u16 = 2;
    pub const DANGEROUS_THING: u16 = 3;
    pub const MISSED_FRIEND: u16 = 4;
    pub const SEEKING: u16 = 5;
    pub const BODY: u16 = 6;
    pub const FRIEND_IN_TROUBLE: u16 = 7;
    pub const ALERT: u16 = 8;
    pub const COMBAT_NOISE: u16 = 9;
    pub const ENEMY: u16 = 10;
    pub const ALERT_IGNORE_ENEMY: u16 = 11;
}

// ---------------------------------------------------------------------------
// Rank — re-exported from the profile system (`ProfileRank`).
// ---------------------------------------------------------------------------

pub use crate::profiles::ProfileRank;

// ---------------------------------------------------------------------------
// Difficulty constants
// ---------------------------------------------------------------------------

pub mod difficulty {
    pub use crate::player_profile::difficulty_params::*;
}

// ---------------------------------------------------------------------------
// Nearest-target selection flags
// ---------------------------------------------------------------------------

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct GetNearestFlags: u16 {
        const USE_MAXNORM       = 0x0001;
        const DANGEROUS_MENACER = 0x0004;
    }
}

crate::bitcode_adapters::impl_native_bitcode_flags!(GetNearestFlags, u16);

// ---------------------------------------------------------------------------
// Seek flags
// ---------------------------------------------------------------------------

bitflags! {
    /// Flags controlling how a seek operation is performed.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct SeekFlags: u16 {
        const LOCATION_END              = 0x0001;
        const LOCATION_FIRST            = 0x0002;
        const WALKING                   = 0x0004;
        const LOOK_FOR_HELP_AFTER       = 0x0008;
        const REPORT_OFFICER_AFTER      = 0x0010;
        const BODY_SEEK                 = 0x0020;
        const CHARLY_SEEK               = 0x0040;
        const DELAY                     = 0x0080;
        const HOUSE                     = 0x0100;
    }
}

crate::bitcode_adapters::impl_native_bitcode_flags!(SeekFlags, u16);

// ---------------------------------------------------------------------------
// Report update flags
// ---------------------------------------------------------------------------

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct ReportUpdateFlags: u16 {
        const UPDATE_BODIES  = 0x0001;
        const UPDATE_CHARLY  = 0x0002;
        const UPDATE_TYPE    = 0x0004;
    }
}

crate::bitcode_adapters::impl_native_bitcode_flags!(ReportUpdateFlags, u16);

// ---------------------------------------------------------------------------
// Primary target flags
// ---------------------------------------------------------------------------

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct PrimaryTargetFlags: u16 {
        const UNOCCUPIED_PREFERRED          = 0x0001;
        const UNOCCUPIED_STRONGLY_PREFERRED = 0x0002;
        const VIPS_ALLOWED                  = 0x0004;
    }
}

crate::bitcode_adapters::impl_native_bitcode_flags!(PrimaryTargetFlags, u16);

// ---------------------------------------------------------------------------
// Condition flags (internal to expected-event dispatch)
// ---------------------------------------------------------------------------

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct ConditionFlags: u16 {
        const IN_DEFAULT_STATE              = 0x0001;
        const IN_DEFAULT_STATE_OR_LOOKING_BODY = 0x0002;
    }
}

crate::bitcode_adapters::impl_native_bitcode_flags!(ConditionFlags, u16);

// ---------------------------------------------------------------------------
// Combat position
// ---------------------------------------------------------------------------

/// A proposed combat position for swordfight tactics.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct CombatPosition {
    #[serde(default, with = "crate::ai::optional_ai_handle")]
    pub attacker: Option<AiEntityHandle>,
    pub attacker_position: Position,
    #[serde(default, with = "crate::ai::optional_ai_handle")]
    pub target: Option<AiEntityHandle>,
    pub target_position: Position,
    pub target_direction: u16,
    pub change_position: bool,
    pub change_adversary: bool,
    pub bonus: i16,
    pub estimated_damage: i16,
    pub line_position: bool,
    #[serde(default, with = "crate::ai::optional_ai_handle")]
    pub left_neighbour: Option<AiEntityHandle>,
    #[serde(default, with = "crate::ai::optional_ai_handle")]
    pub right_neighbour: Option<AiEntityHandle>,
    /// Jump-line index when the combat position sits across a jump line
    /// (table-swordfight case); `None` otherwise.
    pub line_jump: Option<u32>,
}

impl Default for CombatPosition {
    fn default() -> Self {
        Self {
            attacker: None,
            attacker_position: Position::default(),
            target: None,
            target_position: Position::default(),
            target_direction: 0,
            change_position: false,
            change_adversary: false,
            bonus: 0,
            estimated_damage: NOT_YET_COMPUTED,
            line_position: false,
            left_neighbour: None,
            right_neighbour: None,
            line_jump: None,
        }
    }
}

const NOT_YET_COMPUTED: i16 = 6666;

#[track_caller]
/// The world-space half of all-around detection: the original game passes
/// the upright eye point and detection point straight into the
/// distance test and the opaque ray, so callers holding those stored 3D points
/// must not route them through the map projection and back.
pub(crate) fn soldier_detects_detection_point_360(
    viewer_eye: crate::coordinates::WorldPoint3D,
    viewer_radius: u16,
    viewer_in_building: bool,
    target_detection: crate::coordinates::WorldPoint3D,
    target_in_building: bool,
    obstacles: crate::sight_obstacle::ObstacleList<'_>,
) -> bool {
    detects_360(
        Viewer360 {
            eye: viewer_eye,
            sq_radius: (viewer_radius as f32).powi(2),
            in_building: viewer_in_building,
        },
        Target360 {
            detection: target_detection,
            in_building: target_in_building,
        },
        obstacles,
    )
    .visible
}

/// Map-space form of all-around detection: both points are rebuilt from AI
/// positions plus ground Z (`GroundPoint::from_map_and_z`). That projection
/// round trip is not bit-identical to the stored 3D points used by
/// [`soldier_detects_detection_point_360`], which is why the eye point is an
/// explicit input of the shared [`detects_360`] core rather than recomputed
/// there.
#[track_caller]
pub(crate) fn soldier_detects_target_360(
    viewer_position: Position,
    viewer_ground_z: f32,
    viewer_is_rider: bool,
    viewer_radius: u16,
    viewer_in_building: bool,
    target_position: Position,
    target_ground_z: f32,
    target_posture: crate::element::Posture,
    target_is_rider: bool,
    target_direction: i16,
    target_in_building: bool,
    obstacles: crate::sight_obstacle::ObstacleList<'_>,
) -> bool {
    if viewer_in_building || target_in_building {
        return false;
    }
    let target_xy = crate::stealth::detection_point_xy(
        crate::coordinates::MapPoint::new(target_position.x, target_position.y),
        target_posture,
        target_direction,
    );
    let viewer_z = viewer_ground_z
        + crate::stealth::eye_z_for_posture(crate::element::Posture::Upright, viewer_is_rider);
    let target_z =
        target_ground_z + crate::stealth::detection_z_for_posture(target_posture, target_is_rider);
    let viewer_ground = crate::coordinates::GroundPoint::from_map_and_z(
        crate::coordinates::MapPoint::new(viewer_position.x, viewer_position.y),
        viewer_ground_z,
    );
    let target_ground = crate::coordinates::GroundPoint::from_map_and_z(target_xy, target_ground_z);
    detects_360(
        Viewer360 {
            eye: crate::coordinates::WorldPoint3D::new(viewer_ground.x, viewer_ground.y, viewer_z),
            sq_radius: (viewer_radius as f32).powi(2),
            in_building: viewer_in_building,
        },
        Target360 {
            detection: crate::coordinates::WorldPoint3D::new(
                target_ground.x,
                target_ground.y,
                target_z,
            ),
            in_building: target_in_building,
        },
        obstacles,
    )
    .visible
}

pub fn soldier_is_able_to_help_state(
    is_able_to_fight: bool,
    ai_state: AiState,
    ai_substate: Substate,
) -> bool {
    if !is_able_to_fight {
        return false;
    }

    match ai_state {
        AiState::Sleeping | AiState::Menacing | AiState::Fleeing | AiState::Attacking => false,
        AiState::Default | AiState::Wondering => true,
        AiState::Seeking => matches!(
            ai_substate,
            Substate::SeekingSoldierGiveReportToOfficer
                | Substate::SeekingSoldierGiveAlertingReportToOfficerStart
                | Substate::SeekingSoldierGiveAlertingReportToOfficerPoint
                | Substate::SeekingSoldierGiveAlertingReportToOfficerEnd
                | Substate::SeekingRunningToOfficer
                | Substate::SeekingRunningToOfficerSeen
                | Substate::SeekingHeardstepsReactiontime
                | Substate::SeekingBodyReactiontime
        ),
    }
}

/// Forward-half-plane detection using live position and direction values.
pub(crate) fn detects_position_180_raw(
    viewer_pos: Position,
    viewer_direction: u16,
    target: Position,
    sq_standard_view_radius: f32,
) -> bool {
    let dx = target.x - viewer_pos.x;
    let dy = (target.y - viewer_pos.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
    let sq_distance = dx * dx + dy * dy;
    if sq_distance > sq_standard_view_radius {
        return false;
    }

    match half_plane_180(dx, dy, sq_distance, viewer_direction) {
        HalfPlane180::Beside => true,
        HalfPlane180::NotBeside { forward_dot } => forward_dot >= 0.0,
    }
}

/// Planar outcome of a 180° detection test once the squared distance has
/// passed the view-radius gate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum HalfPlane180 {
    /// Within 50 units and beside the viewer (perpendicular component at
    /// least the forward length): detected without any further test.
    Beside,
    /// Not beside. `forward_dot` is `dot(offset, forward)`; each caller applies
    /// its own comparison (the LOS variant rejects on `< 0.0`, the planar
    /// variant accepts on `>= 0.0`). Those differ for NaN, so this helper
    /// deliberately does not pick one.
    NotBeside { forward_dot: f32 },
}

/// The shared "beside me" / forward half-plane geometry of every 180°
/// detection variant. `(dx, dy)` is the stretched-Y offset from the viewer's
/// eye to the target and `sq_distance` its squared length.
pub(crate) fn half_plane_180(dx: f32, dy: f32, sq_distance: f32, direction: u16) -> HalfPlane180 {
    // The direction vector is built by compressing the sector table's Y by
    // ASPECT_RATIO and then stretching it back by INVERSE_ASPECT_RATIO. The
    // shared Rust table already holds the resulting uncompressed unit vector,
    // so stretching here a second time would narrow the forward half-plane.
    let dir = crate::shadow_polygon::sector_to_direction(direction as i16);
    let fx = dir[0];
    let fy = dir[1];

    if sq_distance < 50.0 * 50.0 {
        let fwd_len = dx * fx + dy * fy;
        let fc_x = fx * fwd_len;
        let fc_y = fy * fwd_len;
        let perp_sq = (dx - fc_x) * (dx - fc_x) + (dy - fc_y) * (dy - fc_y);
        if perp_sq >= fwd_len {
            return HalfPlane180::Beside;
        }
    }

    HalfPlane180::NotBeside {
        forward_dot: dx * fx + dy * fy,
    }
}

// ---------------------------------------------------------------------------
// Combat distance helpers (2-D vector math lives in `super::map_vec_ext`)
// ---------------------------------------------------------------------------

/// The AI's own squared distance metric: a stretched **3D** norm.
///
/// The elements' world-space points are subtracted, the Y component is
/// stretched by `INVERSE_ASPECT_RATIO`, and all three components are
/// squared. Positions in the AI snapshots are map-space, so world Y is
/// recovered as `map_y + elevation`.
///
/// A flat 2D `square_norm` is not a substitute: it both under-reports
/// screen-vertical separation and ignores height, so a soldier on a
/// rampart reads as adjacent to one on the ground below.
pub(super) fn ai_square_distance(
    target: &Position,
    target_elevation: f32,
    me: &Position,
    me_elevation: f32,
) -> f32 {
    let dx = target.x - me.x;
    let dz = target_elevation - me_elevation;
    let dy = ((target.y + target_elevation) - (me.y + me_elevation))
        * crate::position_interface::INVERSE_ASPECT_RATIO;
    dx * dx + dy * dy + dz * dz
}

/// Squared distance over two raw element world points.
///
/// Use this when the original-game path receives an element reference directly.
/// AI-facing `Position()` may snap a door-passing actor to a gate endpoint,
/// whereas squared distance reads the actor body's stored position.
pub(super) fn ai_square_distance_world(
    target: &crate::coordinates::WorldPoint3D,
    me: &crate::coordinates::WorldPoint3D,
) -> f32 {
    let dx = target.x - me.x;
    let dy = (target.y - me.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
    let dz = target.z - me.z;
    dx * dx + dy * dy + dz * dz
}

/// Score a building-door candidate exactly like Original
/// Nearest-door selection narrows maximum norm to 16 bits, then applies both penalties with
/// wrapping 16-bit arithmetic.
pub(crate) fn legacy_nearest_door_distance(
    dx: f32,
    dy: f32,
    sector_changes: bool,
    layer_changes: bool,
) -> u16 {
    let mut distance = dx.abs().max(dy.abs()) as u16;
    if sector_changes {
        distance = distance.wrapping_add(500);
    }
    if layer_changes {
        distance = distance.wrapping_add(300);
    }
    distance
}

#[cfg(test)]
mod raw_element_distance_tests;

/// The AI's stretched **3D** Chebyshev distance.
///
/// The world-space points are subtracted, the Y component is stretched by
/// `INVERSE_ASPECT_RATIO`, and the largest absolute component wins.
/// Snapshot positions are map-space, so world Y is recovered as
/// `map_y + elevation` exactly as [`ai_square_distance`] does.
///
/// A 2D max-norm over raw map coordinates is not a substitute. Map Y
/// already carries the elevation as a projection offset, so a friend one
/// layer up reads as roughly twice their true separation and drops out of
/// every consideration radius that should have contained them.
pub(super) fn ai_max_norm_distance(
    target: &Position,
    target_elevation: f32,
    me: &Position,
    me_elevation: f32,
) -> f32 {
    let dx = (target.x - me.x).abs();
    let dz = (target_elevation - me_elevation).abs();
    let dy = (((target.y + target_elevation) - (me.y + me_elevation))
        * crate::position_interface::INVERSE_ASPECT_RATIO)
        .abs();
    dx.max(dy).max(dz)
}

/// Maximum-norm distance over two already-resolved **world** points.
///
/// AI maximum-norm distance
/// in the original game subtracts the two
/// raw element world points, stretches Y by
/// `INVERSE_ASPECT_RATIO` and takes the 3D Chebyshev norm. Use this variant
/// wherever the raw body points are available: AI `Position()` snaps a
/// door-passing actor to the gate endpoint and is not interchangeable.
pub(super) fn ai_max_norm_distance_world(
    target: &crate::coordinates::WorldPoint3D,
    me: &crate::coordinates::WorldPoint3D,
) -> f32 {
    let dx = (target.x - me.x).abs();
    let dy = ((target.y - me.y) * crate::position_interface::INVERSE_ASPECT_RATIO).abs();
    let dz = (target.z - me.z).abs();
    dx.max(dy).max(dz)
}

/// Convert a raw 2D map-space vector `(target - me)` to a 0–15 sector.
/// Thin alias over [`crate::position_interface::vector_to_sector_0_to_15_iso`].
pub(super) fn vec_to_sector(dx: f32, dy: f32) -> u16 {
    crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy) as u16
}

/// Direction offsets for retreat scanning: 0, 1, -1, 2, -2, 3, -3.
/// Tests center first, then alternating sides outward.
const DIRECTION_SPIRAL: [i16; 7] = [0, 1, -1, 2, -2, 3, -3];

/// Mirror the exact promotions and conversions in Original
/// The step-back proposal's mixed unsigned/signed 16-bit modulo expression.
///
/// Both operands promote to signed `int`, so a negative sum retains a negative
/// remainder. Direction-sector assignment converts that remainder
/// to one byte, then masks the byte with 15.
#[inline]
fn step_back_direction_sector(direction: u16, relative_direction: i16) -> u16 {
    (((direction as i32 + relative_direction as i32) % 15) as u8 & 15) as u16
}

#[cfg(test)]
mod step_back_direction_tests;

/// Compute a retreat position away from `pos_enemy`.
///
/// Tries distances from `good_distance - actual` down to `min_distance`,
/// scanning directions in a spiral (0, +1, -1, +2, -2, +3, -3) around the
/// away-from-enemy direction.  Returns the first position reachable via
/// straight-line movement, or `None` if no valid retreat was found.
pub fn propose_good_step_back_goal(
    pos_me: Position,
    move_box: &crate::coordinates::MoveBox,
    pos_enemy: Position,
    good_distance: u16,
    min_distance: u16,
    grid: Option<&crate::fast_find_grid::FastFindGrid>,
    aspect_ratio: f32,
) -> Option<Position> {
    let v = pos_me.map_point() - pos_enemy.map_point();
    let actual_distance = v.iso_norm(aspect_ratio);

    // Already far enough away.
    if actual_distance >= good_distance as f32 {
        return Some(pos_me);
    }

    let direction = v.sector_with_aspect(aspect_ratio);
    let minimal_run_distance = 10.0f32.max(min_distance as f32 - actual_distance);

    // Try to run away as far as possible, reducing distance by 10 each
    // iteration until we hit the minimum.
    let mut distance = good_distance as f32 - actual_distance;
    while distance > minimal_run_distance {
        for &rel_dir in &DIRECTION_SPIRAL {
            // The `% 15` is a source bug (rather than `% 16`), while the
            // signed-promotion and UBYTE-conversion details are intentional
            // parity requirements. Keep them local to this source call site.
            let sector = step_back_direction_sector(direction, rel_dir);
            let dir_vec = MapVec::from_sector_with_aspect(sector, aspect_ratio);
            let goal = Position {
                x: pos_me.x + dir_vec.x * distance,
                y: pos_me.y + dir_vec.y * distance,
                sector: pos_me.sector,
                level: pos_me.level,
            };

            if let Some(grid) = grid {
                let me_pt = crate::coordinates::MapPoint::new(pos_me.x, pos_me.y);
                let goal_pt = crate::coordinates::MapPoint::new(goal.x, goal.y);
                if grid.is_straight_movement_authorized(me_pt, goal_pt, pos_me.level, move_box) {
                    return Some(goal);
                }
            } else {
                // No grid available — the singleton grid should
                // always be valid. Assert in debug to catch unexpected
                // call-sites; release builds accept the position to
                // preserve the prior contract.
                debug_assert!(
                    grid.is_some(),
                    "propose_good_step_back_goal called without grid"
                );
                return Some(goal);
            }
        }
        distance -= 10.0;
    }

    None
}

/// Check if a fighter's substate is one of the 13 stationary/observing
/// combat substates used by combat-observation step selection.
/// Only friends in these substates contribute to the left/right dispersion
/// calculation.
pub(super) fn is_observing_combat_substate(substate: Substate) -> bool {
    use crate::ai::Substate;
    matches!(
        substate,
        Substate::AttackingObserve
            | Substate::AttackingObserveAndMove
            | Substate::AttackingProtectingWithShield
            | Substate::AttackingAdvancingWithShield
            | Substate::AttackingBowRunningBehindShieldBearer
            | Substate::AttackingBowCorrectingPosition
            | Substate::AttackingPhalanx
            | Substate::AttackingRunningToPhalanx
            | Substate::AttackingBowShooting
            | Substate::AttackingBowLoading
            | Substate::AttackingBowAiming
            | Substate::AttackingBowObserving
            | Substate::AttackingBowObservingLoading
    )
}

/// The three substates the attack-opportunity gate in
/// swordfight observation reconsideration checks: a friend already approaching
/// the same target preempts our opportunistic charge.
pub(super) fn is_walking_running_charging_substate(substate: Substate) -> bool {
    use crate::ai::Substate;
    matches!(
        substate,
        Substate::AttackingWalkingToEnemy
            | Substate::AttackingRunningToEnemy
            | Substate::AttackingChargingEnemy
    )
}

/// Check if straight-line movement is authorized between two positions.
/// Returns `true` if no grid is available (conservative: allow movement).
pub(super) fn check_straight_movement(
    grid: Option<&crate::fast_find_grid::FastFindGrid>,
    from: &Position,
    to: &Position,
    move_box: &crate::coordinates::MoveBox,
) -> bool {
    match grid {
        Some(g) => g.is_straight_movement_authorized(
            crate::coordinates::MapPoint::new(from.x, from.y),
            crate::coordinates::MapPoint::new(to.x, to.y),
            from.level,
            move_box,
        ),
        None => true,
    }
}

// ---------------------------------------------------------------------------
// Door-battle dispersion
// ---------------------------------------------------------------------------

/// A dispersed position pair for a door-exit battle.
#[derive(Debug, Clone)]
pub struct DoorBattlePosition {
    /// Position for the defender (the one fleeing from the building).
    pub defender_pos: Position,
    /// Position for the attacker (the one pursuing).
    pub attacker_pos: Position,
    /// Facing direction (0-15 sector) for both fighters.
    pub direction: u16,
}

// ---------------------------------------------------------------------------
// Standalone combat position evaluation functions
// (Free functions so they don't need &self, avoiding borrow-checker issues
// when called from &mut self methods that also mutate CombatPositions.)
// ---------------------------------------------------------------------------

/// Borrowed combat facts used by the shared candidate damage calculation.
pub(crate) trait CombatFighterAccess: Copy {
    fn position(self, handle: HumanHandle) -> Position;
    fn protection_ground_position(self, handle: HumanHandle) -> crate::coordinates::GroundPoint {
        let position = self.position(handle);
        crate::coordinates::GroundPoint::new(position.x, position.y + self.elevation(handle))
    }
    fn elevation(self, handle: HumanHandle) -> f32;
    fn direction(self, handle: HumanHandle) -> u16;
    fn hth_weapon_id(self, handle: HumanHandle) -> u32;
    fn sword_range_maximal(self, handle: HumanHandle) -> u16;
    fn fighting_ability(self, handle: HumanHandle) -> u16;
    fn rank(self, handle: HumanHandle) -> ProfileRank;
    fn is_pc(self, handle: HumanHandle) -> bool;
    fn is_friendly(self, handle: HumanHandle) -> bool;
}

/// Estimates damage the attacker can deal to the target in the
/// given combat position.
///
/// Iterates all 9 normal sword strikes (A..I, excluding Charge), checks
/// each strike's distance window, computes cutting damage scaled by
/// localised protection and stunning damage scaled by bludgeon protection,
/// then averages and applies the from-behind bonus/malus.
fn estimate_damage(
    evaluator: HumanHandle,
    cp: &mut CombatPosition,
    all_fighters: impl CombatFighterAccess,
    profile_manager: &crate::profiles::ProfileManager,
    iq: u16,
) -> i16 {
    // The combat position caches this value. Combat-position evaluation mutates
    // target directions while walking candidate positions, but damage already
    // estimated for the shared friend/enemy position lists deliberately keeps
    // the first direction's result.
    if cp.estimated_damage != NOT_YET_COMPUTED {
        return cp.estimated_damage;
    }
    let Some(target_handle) = cp.target else {
        cp.estimated_damage = 0;
        return 0;
    };

    // Original-game combat-position evaluation operates on
    // live fighter references and reads both combatants' swords.
    // A selected combat position without either fighter is corrupt input, not
    // a harmless zero-damage position.
    let attacker_handle = cp
        .attacker
        .unwrap_or_else(|| panic!("combat position with target {target_handle} has no attacker"));
    let attacker = attacker_handle.get();
    let target = target_handle.get();

    // Vector from attacker to target.  `dy_iso` applies the isometric
    // Y-stretch for Euclidean distance math; `dy_raw` stays raw for the
    // sector computation below (which applies `ASPECT_RATIO` itself
    // via `vec_to_sector`).
    let dx = cp.target_position.x - cp.attacker_position.x;
    let dy_raw = cp.target_position.y - cp.attacker_position.y;
    let dy_iso = dy_raw * INVERSE_ASPECT_RATIO;
    let sq_dist = dx * dx + dy_iso * dy_iso;

    // Short-circuit: out of maximal range → 0 damage.
    let max_range = all_fighters.sword_range_maximal(attacker) as f32;
    if sq_dist > max_range * max_range {
        cp.estimated_damage = 0;
        return 0;
    }

    let mut overall_damage: i32 = 0;

    // A valid weapon profile and sword are required to obtain the standard range.
    // Do not turn missing required weapon data into fabricated flat damage.
    let att_prof = profile_manager
        .get_hth_weapon(all_fighters.hth_weapon_id(attacker))
        .unwrap_or_else(|| {
            panic!(
                "fighter {} requires missing HtH weapon profile {}",
                attacker,
                all_fighters.hth_weapon_id(attacker)
            )
        });
    let def_prof = profile_manager
        .get_hth_weapon(all_fighters.hth_weapon_id(target))
        .unwrap_or_else(|| {
            panic!(
                "fighter {} requires missing HtH weapon profile {}",
                target,
                all_fighters.hth_weapon_id(target)
            )
        });
    let evaluator_prof = profile_manager
        .get_hth_weapon(all_fighters.hth_weapon_id(evaluator))
        .unwrap_or_else(|| {
            panic!(
                "fighter {} requires missing HtH weapon profile {}",
                evaluator,
                all_fighters.hth_weapon_id(evaluator)
            )
        });
    {
        let is_rank_soldier =
            all_fighters.rank(attacker) == ProfileRank::Soldier && !all_fighters.is_pc(attacker);
        // Protection calculation does not use the proposed
        // combat-position coordinates: it dereferences both actors and
        // computes the defender-to-attacker sector from their *live*
        // ground-space coordinates. Rust stores projected map Y in
        // `position`, so recover the Original world Y by adding elevation.
        // Range still uses the hypothetical coordinates above.
        let attacker_ground = all_fighters.protection_ground_position(attacker);
        let target_ground = all_fighters.protection_ground_position(target);
        let target_to_attacker_sector = vec_to_sector(
            attacker_ground.x - target_ground.x,
            attacker_ground.y - target_ground.y,
        ) as i16;

        use crate::weapons::SwordStrike;
        const NORMAL_STRIKES: [SwordStrike; crate::weapons::NUM_NORMAL_SWORD_STRIKES] = [
            SwordStrike::A,
            SwordStrike::B,
            SwordStrike::C,
            SwordStrike::D,
            SwordStrike::E,
            SwordStrike::F,
            SwordStrike::G,
            SwordStrike::H,
            SwordStrike::I,
        ];
        for strike in NORMAL_STRIKES {
            let strike_idx = strike as usize;
            let thrust = &att_prof.thrusts[strike_idx];

            // Distance window check.
            let min_d = thrust.minimal_distance as f32;
            let max_d = thrust.maximal_distance as f32;
            if sq_dist <= min_d * min_d || sq_dist >= max_d * max_d {
                continue;
            }

            // Cutting damage: scaled by attacker's fighting ability if rank
            // soldier, mitigated by target's localised protection.
            let cutting = crate::combat::get_strike_cutting_effect(
                att_prof,
                strike,
                all_fighters.fighting_ability(attacker),
                is_rank_soldier,
            );
            // Original-game quirk: damage estimation uses the sword (the combat
            // position's attacker) for range/effects, but this actor's sword
            // (the AI currently evaluating all positions) for strike
            // direction when querying the defender's local protection.
            // the combat position's target direction may describe how the target
            // would face this proposed position, but sword protection
            // dereferences the target and reads its live direction.
            let strike_dir = crate::combat::get_strike_direction(evaluator_prof, strike);
            let protection = crate::combat::get_sword_protection(
                def_prof,
                all_fighters.direction(target) as i16,
                target_to_attacker_sector,
                strike_dir,
                all_fighters.elevation(attacker),
                all_fighters.elevation(target),
            );
            let cutting_eff = (cutting as f32 * 0.01 * (100.0 - protection as f32).max(0.0)) as i32;

            // Stunning damage uses the *attacker's* bludgeon_protection
            // here — almost certainly a bug in the original, but
            // preserved for behavioural fidelity.
            let stunning = thrust.stunning;
            let bludgeon_prot = att_prof.bludgeon_protection;
            let stunning_eff =
                (stunning as f32 * 0.01 * (100.0 - bludgeon_prot as f32).max(0.0)) as i32;

            overall_damage += cutting_eff + stunning_eff;
        }

        // Average over all 9 strikes.
        overall_damage /= crate::weapons::NUM_NORMAL_SWORD_STRIKES as i32;
    }

    // From-behind bonus/malus: gated on the *evaluator's* IQ (the
    // AI of `me`, not the attacker). The dot product uses the
    // Y-stretched strike vector; `dy_iso` is that stretched value.
    let target_look = MapVec::from_sector(cp.target_direction);
    let from_behind = target_look.dot(MapVec::new(dx, dy_iso)) > 0.0;
    if from_behind {
        if all_fighters.is_friendly(attacker) {
            if iq > combat::ATTACK_FROM_BEHIND_MIN_IQ {
                overall_damage += combat::ATTACK_FROM_BEHIND_BONUS;
            }
        } else if iq > combat::DONT_GET_ATTACKED_FROM_BEHIND_MIN_IQ {
            overall_damage += combat::GET_ATTACKED_FROM_BEHIND_MALUS;
        }
    }

    cp.estimated_damage = overall_damage as i16;
    cp.estimated_damage
}

/// Evaluates one position by computing damage dealt minus damage
/// received from targeting enemies.
fn evaluate_single_position(
    evaluator: HumanHandle,
    cp: &mut CombatPosition,
    enemy_positions: &mut [CombatPosition],
    all_fighters: impl CombatFighterAccess,
    profile_manager: &crate::profiles::ProfileManager,
    iq: u16,
) -> i32 {
    let mut score: i32 = estimate_damage(evaluator, cp, all_fighters, profile_manager, iq) as i32;

    // Subtract damage from enemies who are targeting me
    for enemy_cp in enemy_positions {
        if enemy_cp.target == cp.attacker {
            score -= estimate_damage(evaluator, enemy_cp, all_fighters, profile_manager, iq) as i32;
        }
    }

    score
}

/// Full evaluation of a combat position considering own damage,
/// friends' damage, and unengaged enemies.
pub(crate) fn evaluate_combat_position_full(
    me_handle: HumanHandle,
    me_pos: &Position,
    them_handles: &[HumanHandle],
    cp: &mut CombatPosition,
    friend_positions: &mut [CombatPosition],
    enemy_positions: &mut [CombatPosition],
    all_fighters: impl CombatFighterAccess,
    profile_manager: &crate::profiles::ProfileManager,
    iq: u16,
) -> i32 {
    // Correct target direction: if the enemy is targeting me, they'll
    // turn to face me — adjust their stored direction accordingly.
    for ep in enemy_positions.iter() {
        if ep.target == cp.attacker && cp.target == ep.attacker {
            cp.target_direction = vec_to_sector(
                cp.attacker_position.x - cp.target_position.x,
                cp.attacker_position.y - cp.target_position.y,
            );
        }
    }

    // Distance penalty for position changes
    let distance: u16 = if cp.change_position {
        // The original game truncates the maximum norm to 16 bits before applying the
        // fractional distance penalty.
        (me_pos.map_point() - cp.attacker_position.map_point()).max_norm() as u16
    } else {
        0
    };

    // My own score: estimated combat value + bonus - distance penalty
    let my_points = (evaluate_single_position(
        me_handle,
        cp,
        enemy_positions,
        all_fighters,
        profile_manager,
        iq,
    ) as f32
        + cp.bonus as f32
        - combat::DISTANCE_MALUS_FACTOR * distance as f32) as i32;

    // Accumulate friends' scores
    let mut friends_points: i32 = 0;
    for fp in friend_positions.iter_mut() {
        if fp.attacker == Some(AiEntityHandle::new(me_handle)) {
            continue;
        }

        // Correct friend's target direction if relevant enemy turns
        for ep in enemy_positions.iter() {
            if ep.target == cp.attacker && fp.target == ep.attacker {
                fp.target_direction = vec_to_sector(
                    cp.attacker_position.x - cp.target_position.x,
                    cp.attacker_position.y - cp.target_position.y,
                );
            }
        }

        let friend_score = evaluate_single_position(
            me_handle,
            fp,
            enemy_positions,
            all_fighters,
            profile_manager,
            iq,
        );
        friends_points += friend_score;

        if friend_score < combat::FRIEND_IN_TROUBLE_LIMIT {
            friends_points -= combat::FRIEND_IN_TROUBLE_MALUS;
        }
    }

    // Penalize unengaged enemies (enemies nobody is targeting)
    let mut general_points: i32 = 0;
    for &enemy_handle in them_handles {
        let enemy = Some(AiEntityHandle::new(enemy_handle));
        let is_engaged = enemy == cp.target || friend_positions.iter().any(|fp| fp.target == enemy);
        if !is_engaged {
            general_points -= combat::NON_ENGAGED_ENEMY_MALUS;
        }
    }

    (combat::EGOISM_FACTOR * my_points as f32 + friends_points as f32 + general_points as f32)
        as i32
}

// ---------------------------------------------------------------------------
// Ambush point status (per-NPC tracking)
// ---------------------------------------------------------------------------

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[repr(u32)]
pub enum AmbushPointStatus {
    Far = 0,
    Near,
    Checked,
}

// ---------------------------------------------------------------------------
// Combat constants
// ---------------------------------------------------------------------------

pub mod combat {
    pub const MIN_DOUBLE_LINE_DISTANCE: i32 = 50;
    pub const MAX_DOUBLE_LINE_DISTANCE: i32 = 200;
    pub const SQR_MAX_CONSIDER_LINE_DISTANCE: i32 = 200 * 200;
    pub const STANDARD_LINE_DISTANCE: i32 = 30;
    pub const SQR_MAX_NEW_POS_DIST: i32 = 200 * 200;
    pub const MIN_ENEMY_DIST: i32 = 20;
    pub const MIN_FRIEND_DIST: i32 = 10;
    pub const EGOISM_FACTOR: f32 = 1.5;

    pub const ATTACK_FROM_BEHIND_BONUS: i32 = 5000;
    pub const GET_ATTACKED_FROM_BEHIND_MALUS: i32 = 3000;
    pub const LINE_FORMATION_BONUS: i32 = 500;
    pub const NON_ENGAGED_ENEMY_MALUS: i32 = 1000;
    pub const BAD_POSITION_MALUS: i32 = 1000;
    pub const ENEMY_NEAR_MALUS: i32 = 20;
    pub const DISTANCE_MALUS_FACTOR: f32 = 0.1;
    pub const FRIEND_IN_TROUBLE_LIMIT: i32 = 0;
    pub const FRIEND_IN_TROUBLE_MALUS: i32 = 50;

    pub const QUIT_FORMATION_MIN_IQ: u16 = 51;
    pub const ATTACK_FROM_BEHIND_MIN_IQ: u16 = 70;
    pub const DONT_GET_ATTACKED_FROM_BEHIND_MIN_IQ: u16 = 30;
    pub const CHARGE_MIN_COURAGE: u16 = 40;
    pub const CHARGE_MIN_DISTANCE: i32 = 100;
    pub const PARADE_MIN_IQ: u16 = 30;
    pub const ALWAYS_PARADE_IQ: u16 = 70;
    pub const HELPING_PROUD_FIGHTER_BONUS: i32 = 6000;

    pub const OFFICER_EXAMINE_BODY_HIMSELF_DISTANCE: i32 = 150;
    pub const OFFICER_EXAMINE_NOISE_HIMSELF_DISTANCE: i32 = 100;

    pub const OBSERVE_SWORDFIGHT_MIN_DISTANCE: i32 = 100;
    pub const OBSERVE_SWORDFIGHT_MAX_DISTANCE: i32 = 200;
    pub const OBSERVE_SWORDFIGHT_SIDE_STEP: i32 = 50;

    pub const STANDARD_TALK_TIME: i32 = 30;
    pub const ALERT_RADIUS: i32 = 500;
    pub const STANDARD_LINE_LENGTH: i32 = 3;

    pub const LOOT_DISTANCE: i32 = 1000;

    pub const PROUD_OBSERVER_MIN_DISTANCE: i32 = 100;
    pub const PROUD_OBSERVER_GOOD_DISTANCE: i32 = 150;
    pub const PROUD_OBSERVER_MAX_DISTANCE: i32 = 200;

    pub const SQR_TOWER_GUARD_ALERT_RADIUS: i32 = 800 * 800;

    pub const OFFICER_ODDS_BONUS: i32 = 30;
    pub const APPLE_REACTIONTIME: i32 = 50;

    pub const MAX_WHISTLE_SEEK_RADIUS: i32 = 400;
    pub const MAX_ALERT_OFFICER_RADIUS: i32 = 1400;

    pub const MIN_SQUARE_RESERVE_DISTANCE: i32 = 22500; // 150 * 150
    pub const MIN_CAPACITY_CHARGE_WEAK_ENEMY: u16 = 60;
}

/// Archer-related constants.
pub mod archer {
    pub const MIN_DISTANCE_ENEMY_HEAD_ON_ATTACK: i32 = 300;
    pub const MIN_DISTANCE_ENEMY_APPROACHING_FAST: i32 = 250;
    pub const MIN_DISTANCE_ENEMY_APPROACHING: i32 = 180;
    pub const MIN_DISTANCE_ENEMY_APPROACHING_SLOWLY: i32 = 150;
    pub const MIN_DISTANCE_ENEMY_PASSING: i32 = 100;
    pub const MIN_DISTANCE_ENEMY_LEAVING: i32 = 80;
    pub const GOOD_DISTANCE: i32 = 250;
    pub const MIN_DISTANCE: i32 = 50;
    pub const SHIELD_BEARER_MIN_DISTANCE: i32 = 500;
    pub const MIN_PROTECT_ARROW_DISTANCE: i32 = 150;
    pub const MIN_TARGET_FRIEND_ANGLE: f32 = 0.25;
    pub const DISTANCE_SHIELD_BEARER_ARCHER: i32 = 30;
    pub const DISTANCE_SHIELD_BEARER_SHIELD_BEARER: i32 = 25;
    /// Tolerance for the "already in cover" check — if the archer's
    /// offset from the ideal cover point is within this maximum norm, they
    /// stay and shoot instead of repositioning.
    pub const COVER_POINT_TOLERANCE: i32 = 25;
    /// Distance used by the nearby-archer protection count to
    /// decide which soldiers are "nearby" in the battle situation.
    pub const CONSIDER_BATTLE_SITUATION_DISTANCE: i32 = 500;

    pub const PHALANX_FORWARD_STEP: i32 = 70;
    pub const PHALANX_ATTACK_DISTANCE: i32 = 100;

    pub const SQR_DISTANCE_OFFICER_HEARS_BRAWL: i32 = 200 * 200;
    pub const SQR_DISTANCE_OFFICER_SEES_BRAWL_180: i32 = 350 * 350;
}

/// Sentinel direction value.
pub const UNDEFINED_DIRECTION: u16 = 666;

/// Euclidean distance between two positions.
pub(super) fn pos_distance(a: Position, b: Position) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    (dx * dx + dy * dy).sqrt()
}

/// Resolve a seek point by ID. Global IDs are indices into
/// `AiGlobalState::seek_points`; 1111 and 2222 are personal sentinels.
pub(super) fn resolve_seek_point_id<'a>(
    id: u16,
    personal1: &'a Option<SeekPoint>,
    personal2: &'a Option<SeekPoint>,
    global: &'a AiGlobalState,
) -> Option<&'a SeekPoint> {
    match id {
        1111 => personal1.as_ref(),
        2222 => personal2.as_ref(),
        _ => global.seek_points.get(id as usize),
    }
}

/// Mutable version of [`resolve_seek_point_id`].
pub(super) fn resolve_seek_point_mut<'a>(
    id: u16,
    personal1: &'a mut Option<SeekPoint>,
    personal2: &'a mut Option<SeekPoint>,
    global: &'a mut AiGlobalState,
) -> Option<&'a mut SeekPoint> {
    match id {
        1111 => personal1.as_mut(),
        2222 => personal2.as_mut(),
        _ => global.seek_points.get_mut(id as usize),
    }
}

#[cfg(test)]
mod required_combat_input_tests;

#[cfg(test)]
mod swordfight_substate_tests;

/// Viewer half of all-around (360°) detection.
///
/// The eye point is an explicit input because callers source it differently
/// (the stored upright eye point, a member's stored world position plus the
/// upright eye height, or a map position rebuilt with ground Z) and those
/// sources are not bit-identical, so the shared core must not recompute it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(crate) struct Viewer360 {
    pub(crate) eye: crate::coordinates::WorldPoint3D,
    pub(crate) sq_radius: f32,
    pub(crate) in_building: bool,
}

/// Target half of all-around detection: the target's detection point.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(crate) struct Target360 {
    pub(crate) detection: crate::coordinates::WorldPoint3D,
    pub(crate) in_building: bool,
}

/// Outcome of [`detects_360`]. `sq_distance` is `None` when the building gate
/// rejected before any geometry ran.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(crate) struct Detection360 {
    pub(crate) sq_distance: Option<f32>,
    pub(crate) visible: bool,
}

/// Stretched-Y 3D squared distance between a viewer eye point and a target
/// detection point, as used by every all-around detection check.
pub(crate) fn sq_distance_360(
    eye: crate::coordinates::WorldPoint3D,
    detection: crate::coordinates::WorldPoint3D,
) -> f32 {
    let dx = detection.x - eye.x;
    let dy = (detection.y - eye.y) * INVERSE_ASPECT_RATIO;
    let dz = detection.z - eye.z;
    dx * dx + dy * dy + dz * dz
}

/// The single all-around detection implementation: building gate, squared
/// 3D distance against the viewer radius, then the opaque 3D sight ray.
///
/// `#[track_caller]` so the recorded visibility query is attributed to the
/// gate that asked for it, not to this shared helper.
#[track_caller]
pub(crate) fn detects_360(
    viewer: Viewer360,
    target: Target360,
    obstacles: crate::sight_obstacle::ObstacleList<'_>,
) -> Detection360 {
    if viewer.in_building || target.in_building {
        return Detection360 {
            sq_distance: None,
            visible: false,
        };
    }
    let sq_distance = sq_distance_360(viewer.eye, target.detection);
    if sq_distance > viewer.sq_radius {
        return Detection360 {
            sq_distance: Some(sq_distance),
            visible: false,
        };
    }
    let visible = crate::sight_obstacle::is_reachable_3d(
        obstacles,
        [viewer.eye.x, viewer.eye.y, viewer.eye.z],
        [target.detection.x, target.detection.y, target.detection.z],
        crate::sight_obstacle::SIGHTOBSTACLE_OPAQUE,
    );
    Detection360 {
        sq_distance: Some(sq_distance),
        visible,
    }
}
