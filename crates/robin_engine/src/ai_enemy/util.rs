//! Shared helpers for the enemy-AI module: math, snapshots, combat-position
//! evaluation, ambush-point status, and tunable constants.

use bitflags::bitflags;
use serde::{Deserialize, Serialize};

use crate::ai::*;
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
// GetNearest flags
// ---------------------------------------------------------------------------

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct GetNearestFlags: u16 {
        const USE_MAXNORM       = 0x0001;
        const DANGEROUS_MENACER = 0x0004;
    }
}

// ---------------------------------------------------------------------------
// Seek flags
// ---------------------------------------------------------------------------

bitflags! {
    /// Flags controlling how a seek operation is performed.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

// ---------------------------------------------------------------------------
// Condition flags (internal to ThinkExpectedEvent)
// ---------------------------------------------------------------------------

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct ConditionFlags: u16 {
        const IN_DEFAULT_STATE              = 0x0001;
        const IN_DEFAULT_STATE_OR_LOOKING_BODY = 0x0002;
    }
}

// ---------------------------------------------------------------------------
// Combat position
// ---------------------------------------------------------------------------

/// A proposed combat position for swordfight tactics.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct CombatPosition {
    pub attacker: HumanHandle,
    pub attacker_position: Position,
    pub target: HumanHandle,
    pub target_position: Position,
    pub target_direction: u16,
    pub change_position: bool,
    pub change_adversary: bool,
    pub bonus: i16,
    pub estimated_damage: i16,
    pub line_position: bool,
    pub left_neighbour: HumanHandle,
    pub right_neighbour: HumanHandle,
    /// Jump-line index when the combat position sits across a jump line
    /// (table-swordfight case); `None` otherwise.
    pub line_jump: Option<u32>,
}

impl Default for CombatPosition {
    fn default() -> Self {
        Self {
            attacker: 0,
            attacker_position: Position::default(),
            target: 0,
            target_position: Position::default(),
            target_direction: 0,
            change_position: false,
            change_adversary: false,
            bonus: 0,
            estimated_damage: NOT_YET_COMPUTED,
            line_position: false,
            left_neighbour: 0,
            right_neighbour: 0,
            line_jump: None,
        }
    }
}

const NOT_YET_COMPUTED: i16 = 6666;

// ---------------------------------------------------------------------------
// FighterSnapshot — engine-provided state for combat position evaluation
// ---------------------------------------------------------------------------

/// Snapshot of a fighter's state for combat AI evaluation.
/// The engine populates these before think() for NPCs in swordfight states.
/// Lightweight snapshot of a same-camp soldier used by alert functions
/// (`alert_officer`, `alert_soldiers`).  Populated by the engine each tick
/// for all soldiers in the same camp, regardless of combat state.
#[derive(Debug, Clone)]
pub struct CampSoldierInfo {
    pub handle: NpcHandle,
    pub position: Position,
    pub direction: u16,
    pub rank: ProfileRank,
    pub ai_state: AiState,
    pub ai_substate: Substate,
    pub is_able_to_fight: bool,
    /// Live primary target used when BattleDecisions merges an attacking
    /// friend's target into its persistent Them list.
    pub primary_target: HumanHandle,
    /// Soldier-profile pride used by BattleDecisions' side-strength and
    /// too-proud-to-attack calculations.
    pub pride: u16,
    /// Narrower than `is_able_to_fight`: sleeping / attacking /
    /// menacing / fleeing soldiers cannot be pulled into officer
    /// coordination, while default, wondering, and specific
    /// report-to-officer seeking substates can.
    pub is_able_to_help: bool,
    pub script_locked: bool,
    /// Dynamic `AILOCK_FREEZE` snapshot used by non-result-bearing candidate
    /// checks. Original direct `Think` retains a locked stimulus and returns
    /// true, so bool-consuming call sites must dispatch instead of predicting.
    pub ai_lock_frozen: bool,
    pub layer: u16,
    /// Reconnaissance report type (for `GetReportFromSoldier` checks).
    pub report_type: ReportType,
    /// Seek position from the soldier's reconnaissance report.
    pub report_seek_position: Position,
    /// Seen bodies from the soldier's reconnaissance report.
    /// Used by `ConsiderReport` for body/charly list merging.
    pub report_seen_bodies: Vec<HumanHandle>,
    /// Charly (missing friend) handle from the soldier's report.
    pub report_charly: NpcHandle,
    /// The soldier's alert-soldiers point.
    pub alert_soldiers_point: Position,
    /// This soldier's patrol chief.
    /// Used by `CanCallThisSoldier` and the patrol-chief fallback in the
    /// AlertSoldiers eligibility predicate.
    pub patrol_chief: Option<EntityId>,
    /// This soldier's current antagonist.
    /// Used by `CanCallThisSoldier` to reject a soldier already in a
    /// conversation with someone other than the calling officer.
    pub antagonist: NpcHandle,
    /// Whether this soldier has the duty soldier-profile flag set.
    /// Combined with `is_tower_guard` and `company_number == 100` it drives
    /// `Q_SHALL_I_STAY_ON_MY_POST` / `IsAllowedToLeaveHisPost`.
    pub duty_flag: bool,
    /// Whether this soldier is a tower guard.  Feeds
    /// `IsAllowedToLeaveHisPost` (outdoor branch).
    pub is_tower_guard: bool,
    /// Soldier's company number — company 100 stays on post.
    pub company_number: u16,
    /// Whether this soldier is currently inside a building sector.
    /// Used by `AlertOfficer` to gate the layer-change penalty.
    pub in_building: bool,
    /// Forecasted destination for this soldier. Used by
    /// `AlertOfficer` so the alerting soldier homes on where the
    /// officer will be, not where the officer currently is.
    /// Present only when the caller explicitly evaluated
    /// `ForecastDestinationForIA`; owner-local brackets may omit it to avoid
    /// drawing unrelated building-exit RNG.
    pub forecast_destination: Option<crate::ai::PreparedForecastDestination>,
    /// Body handles still on this soldier's detectable-body list —
    /// i.e. corpses they have *not yet* reacted to.  An officer
    /// who has *already* processed a body has dropped it from this
    /// list, gating `NearOfficerWhoIsInformedAboutThisBody`.  Live
    /// data, refreshed every tick from
    /// `Soldier::detectable_lists[DetectableType::Body]`.
    pub detectable_bodies: Vec<HumanHandle>,
    /// Soldier's own seek position (live AI field, not the
    /// reconnaissance-report seek position).  Used by
    /// `NearOfficerWhoIsWonderningAboutTheSameNoise` to identify an
    /// officer actively heading to the same noise.  Distinct from
    /// [`Self::report_seek_position`], which only updates on report
    /// merges.
    pub seek_position: Position,
    /// Live current task priority from the soldier's AI brain.  Used
    /// by the officer's AlertSoldiers gate to predict whether the
    /// soldier's `Think(CALL_ALERT)` would have returned true (the
    /// `Q_HAS_THE_NEW_TASK_PRIORITY` arm).
    pub current_task_priority: u16,
    /// Live minimal task priority from the soldier's AI brain — the
    /// floor under which `Q_HAS_THE_NEW_TASK_PRIORITY` may admit a
    /// downgrade in the non-Seeking/non-Wondering state arm.
    pub minimal_task_priority: u16,
    /// View direction post-`RefreshView` — the unit forward
    /// vector after head-turn / stare / lean modifiers. Used by
    /// `MaybeOfficerSeesMeFighting`'s ≥350² band to evaluate the
    /// officer's cone (the triangle test in `ComputeVisibility`)
    /// without re-borrowing the engine for a `VisibilityQuery`.
    pub view_direction: [f32; 2],
    /// View radius post-`RefreshView` — radius gate for the
    /// cone+LOS detection.
    pub view_radius: u16,
    /// Half-aperture post-`RefreshView` — half-angle of the vision
    /// cone after stare / drunk / lean-out modifiers.
    pub real_half_aperture: f32,
    /// Whether the eyes are blind (closed / dying / unconscious), so
    /// the cone+LOS gate skips blind officers.
    pub eye_blind: bool,
}

#[allow(clippy::too_many_arguments)]
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
    let dx = target_ground.x - viewer_ground.x;
    let dy = (target_ground.y - viewer_ground.y) * INVERSE_ASPECT_RATIO;
    let dz = target_z - viewer_z;
    if dx * dx + dy * dy + dz * dz > (viewer_radius as f32).powi(2) {
        return false;
    }
    crate::sight_obstacle::is_reachable_3d(
        obstacles,
        [viewer_ground.x, viewer_ground.y, viewer_z],
        [target_ground.x, target_ground.y, target_z],
        crate::sight_obstacle::SIGHTOBSTACLE_OPAQUE,
    )
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

/// 180-degree detection check evaluated from the perspective of an
/// arbitrary [`FighterSnapshot`] (not necessarily `self`).  Used by
/// `is_too_proud_to_attack` to ask whether a lower-pride ally is
/// observing our primary target.
pub(super) fn fighter_detects_position_180(
    viewer: &FighterSnapshot,
    target: Position,
    sq_standard_view_radius: f32,
) -> bool {
    if !viewer.is_able_to_fight {
        return false;
    }

    let dx = target.x - viewer.position.x;
    let dy = (target.y - viewer.position.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
    let sq_distance = dx * dx + dy * dy;
    if sq_distance > sq_standard_view_radius {
        return false;
    }

    let dir = crate::shadow_polygon::sector_to_direction(viewer.direction as i16);
    let fx = dir[0];
    let fy = dir[1] * crate::position_interface::INVERSE_ASPECT_RATIO;

    if sq_distance < 50.0 * 50.0 {
        let fwd_len = dx * fx + dy * fy;
        let fc_x = fx * fwd_len;
        let fc_y = fy * fwd_len;
        let perp_sq = (dx - fc_x) * (dx - fc_x) + (dy - fc_y) * (dy - fc_y);
        if perp_sq >= fwd_len {
            return true;
        }
    }

    dx * fx + dy * fy >= 0.0
}

pub(super) fn soldier_detects_position_180(
    viewer: &CampSoldierInfo,
    target: Position,
    sq_standard_view_radius: f32,
) -> bool {
    if viewer.in_building || !viewer.is_able_to_fight {
        return false;
    }
    detects_position_180_raw(
        viewer.position,
        viewer.direction,
        target,
        sq_standard_view_radius,
    )
}

/// Free-function form of [`soldier_detects_position_180`] taking raw
/// position/direction inputs.  Used by the engine-side
/// `UnalertAllNearCharlySeekers` sweep, which has access to the entity
/// store but doesn't construct a [`CampSoldierInfo`] for the candidates.
///
/// 180° forward cone on the viewer's direction, bounded by the standard
/// view radius squared.
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

    let dir = crate::shadow_polygon::sector_to_direction(viewer_direction as i16);
    let fx = dir[0];
    let fy = dir[1] * crate::position_interface::INVERSE_ASPECT_RATIO;

    if sq_distance < 50.0 * 50.0 {
        let fwd_len = dx * fx + dy * fy;
        let fc_x = fx * fwd_len;
        let fc_y = fy * fwd_len;
        let perp_sq = (dx - fc_x) * (dx - fc_x) + (dy - fc_y) * (dy - fc_y);
        if perp_sq >= fwd_len {
            return true;
        }
    }

    dx * fx + dy * fy >= 0.0
}

/// Snapshot of entity-level data (position, direction, sword range,
/// opponents, etc.) read by combat AI evaluation.
#[derive(Debug, Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct FighterSnapshot {
    pub handle: HumanHandle,
    pub position: Position,
    pub direction: u16,
    /// True if this fighter is on the same side as the evaluating AI.
    pub is_friendly: bool,
    pub is_swordfighting: bool,
    pub is_able_to_fight: bool,
    /// True when the fighter's posture is `Tied` — they cannot be
    /// targeted by rider charges or counted as a friend in
    /// `IsAnyFriendInThisPolygon`.
    pub is_tied: bool,
    /// True when the fighter is unconscious. Distinct from
    /// `is_able_to_fight`, which folds inactivity, life-points, and
    /// disguise postures into one bit.
    pub is_unconscious: bool,
    /// True when the fighter is dead (`life_points <= 0` or
    /// `posture == Dead/DeadBack`).
    pub is_dead: bool,
    /// True when the fighter is being carried (e.g. PC slung over
    /// another character's shoulders).
    pub is_carried: bool,
    pub is_pc: bool,
    pub is_soldier: bool,
    pub rank: ProfileRank,
    /// Soldier AI `primary_target` / PC melee target. For soldiers this is the
    /// reference `GetPrimaryTarget()` value, which differs from the principal
    /// swordfight opponent while approaching a target.
    pub primary_target: HumanHandle,
    pub principal_opponent: HumanHandle,
    pub number_of_opponents: u16,
    /// All handles this fighter is currently engaged with.
    pub opponent_handles: Vec<HumanHandle>,
    pub sword_range_default: u16,
    pub sword_range_maximal: u16,
    /// Extended weapon range used for line-position enemy reachability.
    pub sword_range_uber: u16,
    pub fighting_ability: u16,
    /// Whether this fighter's soldier profile has formation enabled.
    pub has_formation: bool,
    /// True if this fighter's HtH weapon is a shield weapon. Strictly
    /// this also requires a `RHANIMATION_WAITING_SHIELD` animation,
    /// but for now we key off the weapon profile alone (the animation
    /// presence is a property of the soldier sprite set, not tracked
    /// in the Rust port yet).
    pub is_shield_bearer: bool,
    /// Whether this fighter is an archer unit (has a bow).
    pub is_archer_unit: bool,
    /// Whether this fighter is a tower guard. Used by
    /// `number_of_nearby_archers_who_need_protection` to exclude tower
    /// guards from the orphan-archer count.
    pub is_tower_guard: bool,
    /// Whether this fighter is a VIP (important character) — from
    /// `CharacterProfile::vip` for PCs, `SoldierProfile::vip` for soldiers.
    pub is_vip: bool,
    /// Soldier profile pride value. PCs get 0 (they contribute a flat 100
    /// to us-points in `MakeBattlePredecisions`).
    pub soldier_profile_pride: u16,
    /// Whether this fighter is Robin Hood (the main hero). Only true for PCs.
    pub is_robin: bool,
    /// This fighter's cached left combat neighbour (for phalanx chain
    /// walking).
    pub left_combat_neighbour: HumanHandle,
    /// This fighter's cached right combat neighbour.
    pub right_combat_neighbour: HumanHandle,
    /// True if in a recovery animation (being hit, dying, unconscious, etc.).
    pub is_in_recovery_animation: bool,
    /// True if in a valid sword combat action state.
    pub in_sword_action_state: bool,
    /// Fighter's ground-plane elevation (world Z). Used by the
    /// archer run-to-archery-point path to remember the enemy's Z
    /// when picking a bow posture.
    pub elevation: u16,
    /// Position the fighter is moving to (or current position for stationary
    /// fighters). Used by `propose_good_combat_position` to score friends at
    /// their *intended* combat position rather than their current pose.
    pub seek_position: Position,
    /// Substate snapshot — used by `GetCombatPosition` semantics: when this is
    /// `AttackingApproachingNewEnemy` or `AttackingMovingAroundOldEnemy`, the
    /// scorer treats the fighter as moving toward `position`; otherwise it
    /// scores at `seek_position`.
    pub current_substate: u32,
    /// The handle of the archer hiding behind this shield bearer, or 0.
    /// Derived during snapshot building from the reverse
    /// `shield_bearer_before_me` link so archers can't double-claim a
    /// shield bearer.
    pub archer_behind_me: HumanHandle,
    /// The AI state of this fighter (Seeking, Attacking, etc.).
    /// Used by `number_of_nearby_archers_who_need_protection` to filter
    /// fighters in specific states.
    pub ai_state: AiState,
    /// Handle of the shield bearer this archer is hiding behind (0 = none).
    /// Used to identify "orphan" archers who need protection.
    pub shield_bearer_before_me: u32,
    /// Snapshots keep the stable 1-based weapon profile id and resolve
    /// it through the tick's shared `ProfileManager` when strike
    /// damage is evaluated, avoiding per-fighter `HtHWeaponProfile`
    /// clones.
    pub hth_weapon_id: u32,
    /// The fighter's current action state — used by phalanx tick handlers
    /// to check `HoldingShield`/`ParryingShield` and bow states.
    pub action_state: crate::element::ActionState,
    // (archer_behind_me field is above, derived from reverse shield_bearer_before_me scan)
    /// This fighter's shield-bearer direction (seek direction when running
    /// to phalanx). Used by `get_shield_bearer_position`.
    pub shield_bearer_direction: u16,
    /// This fighter's seek position (destination when running to phalanx).
    pub shield_bearer_seek_position: Position,
    /// Bow max range. 0 for non-archers.
    pub bow_max_range: u16,
}

impl FighterSnapshot {
    /// Checks if this fighter lists `handle` as an opponent.
    pub fn has_as_opponent(&self, handle: HumanHandle) -> bool {
        self.opponent_handles.contains(&handle)
    }
}

// ---------------------------------------------------------------------------
// Combat vector math helpers
// ---------------------------------------------------------------------------

/// Convert a 0–15 compass sector to a unit direction vector.
/// Sector 0 = north (0, -1), increasing clockwise.
pub(super) fn sector_to_vector(sector: u16) -> (f32, f32) {
    let angle = (sector as f32) * std::f32::consts::PI / 8.0;
    (angle.sin(), -angle.cos())
}

/// Dot product of two 2D vectors.
pub(super) fn dot2(a: (f32, f32), b: (f32, f32)) -> f32 {
    a.0 * b.0 + a.1 * b.1
}

/// 2D determinant (cross product Z component): positive if b is to the left of a.
pub(super) fn det2(a: (f32, f32), b: (f32, f32)) -> f32 {
    a.0 * b.1 - a.1 * b.0
}

/// Max-norm (Chebyshev distance) of a 2D vector.
pub(super) fn max_norm(v: (f32, f32)) -> f32 {
    v.0.abs().max(v.1.abs())
}

/// Squared Euclidean norm of a 2D vector.
pub(super) fn square_norm(v: (f32, f32)) -> f32 {
    v.0 * v.0 + v.1 * v.1
}

/// Perpendicular vector (90° counter-clockwise rotation, left normal).
pub(super) fn get_normal(v: (f32, f32)) -> (f32, f32) {
    (-v.1, v.0)
}

/// Perpendicular vector (90° clockwise rotation, right normal).
pub(super) fn get_normal_right(v: (f32, f32)) -> (f32, f32) {
    (v.1, -v.0)
}

/// Position difference as a 2D vector.
pub(super) fn pos_diff(a: &Position, b: &Position) -> (f32, f32) {
    (a.x - b.x, a.y - b.y)
}

/// Convert a raw 2D map-space vector `(target - me)` to a 0–15 sector.
/// Thin alias over [`crate::position_interface::vector_to_sector_0_to_15_iso`].
pub(super) fn vec_to_sector(dx: f32, dy: f32) -> u16 {
    crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy) as u16
}

/// Isometric Euclidean norm.  Thin alias over
/// [`crate::position_interface::vector_norm_iso`].  Every caller passes
/// `aspect_ratio = ASPECT_RATIO`; the argument is retained for signature
/// stability.
pub(super) fn iso_norm(v: (f32, f32), _aspect_ratio: f32) -> f32 {
    crate::position_interface::vector_norm_iso(v.0, v.1)
}

/// Isometric normalize.  Thin alias over
/// [`crate::position_interface::vector_normalize_iso`].
pub(super) fn iso_normalize(v: (f32, f32), _aspect_ratio: f32) -> (f32, f32) {
    let [x, y] = crate::position_interface::vector_normalize_iso(v.0, v.1);
    (x, y)
}

/// Sector to unit vector with isometric Y scaling.  Thin alias over
/// [`crate::position_interface::sector_to_vector_iso`].
pub(super) fn sector_to_vector_iso(sector: u16, _aspect_ratio: f32) -> (f32, f32) {
    let [x, y] = crate::position_interface::sector_to_vector_iso(sector as i16);
    (x, y)
}

/// Vector to sector.  Thin alias over
/// [`crate::position_interface::vector_to_sector_0_to_15_iso`].
pub(super) fn vec_to_sector_ar(dx: f32, dy: f32, _aspect_ratio: f32) -> u16 {
    crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy) as u16
}

/// Perpendicular vector in isometric space.  Thin alias over
/// [`crate::position_interface::vector_normal_iso`].
pub(super) fn get_normal_iso(v: (f32, f32), direct: bool, _aspect_ratio: f32) -> (f32, f32) {
    let [x, y] = crate::position_interface::vector_normal_iso(v.0, v.1, direct);
    (x, y)
}

/// Direction offsets for retreat scanning: 0, 1, -1, 2, -2, 3, -3.
/// Tests center first, then alternating sides outward.
const DIRECTION_SPIRAL: [i16; 7] = [0, 1, -1, 2, -2, 3, -3];

/// Returns `true` iff the enemy is sufficiently below the viewer
/// that an archer should bend down to bow-down posture.  The inputs
/// are the viewer's [`AiContext`] and the target's `(position,
/// elevation)` — or `None` when the target has no view snapshot
/// (defensive: always return `false`).
pub(super) fn enemy_is_below_me(
    ctx: &AiContext,
    target: Option<(crate::ai::Position, f32)>,
) -> bool {
    // Leaning out = enemy is downstairs.
    if ctx.posture == crate::element::Posture::LeaningOut {
        return true;
    }

    let Some((target_pos, target_elevation)) = target else {
        return false;
    };

    let height_diff = target_elevation - ctx.elevation;
    if height_diff >= 0.0 {
        // Same height or above — not below.
        return false;
    }
    if height_diff < -50.0 {
        // Sufficiently below.
        return true;
    }

    // Horizontal vs vertical distance comparison (stretched Y to
    // cancel the isometric aspect ratio).
    let dx = target_pos.x - ctx.position.x;
    let dy = (target_pos.y - ctx.position.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
    (dx * dx + dy * dy) <= height_diff * height_diff
}

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
    let v = pos_diff(&pos_me, &pos_enemy);
    let actual_distance = iso_norm(v, aspect_ratio);

    // Already far enough away.
    if actual_distance >= good_distance as f32 {
        return Some(pos_me);
    }

    let direction = vec_to_sector_ar(v.0, v.1, aspect_ratio);
    let minimal_run_distance = 10.0f32.max(min_distance as f32 - actual_distance);

    // Try to run away as far as possible, reducing distance by 10 each
    // iteration until we hit the minimum.
    let mut distance = good_distance as f32 - actual_distance;
    while distance > minimal_run_distance {
        for &rel_dir in &DIRECTION_SPIRAL {
            // `(direction + relative_direction) % 15`. The `% 15`
            // is a latent bug (should be `% 16` for 16 sectors), but
            // with unsigned 16-bit wraparound `(0 + -1) = 65535` and
            // `65535 % 15 = 0`, so direction=0, rel=-1 maps to
            // sector 0 instead of 15; direction=15, rel=+1 maps to 1
            // instead of 0; etc.  Replicate the unsigned 16-bit
            // wrap, modulo 15 for bug-for-bug parity.
            let sum = (direction as i32 + rel_dir as i32).rem_euclid(65536) as u16;
            let sector = sum % 15;
            let dir_vec = sector_to_vector_iso(sector, aspect_ratio);
            let goal = Position {
                x: pos_me.x + dir_vec.0 * distance,
                y: pos_me.y + dir_vec.1 * distance,
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

/// Check if a fighter's substate (as u32) is one of the 13 stationary/observing
/// combat substates used by `ProposeStepDirectionWhileObservingCombat`.
/// Only friends in these substates contribute to the left/right dispersion
/// calculation.
pub(super) fn is_observing_combat_substate(substate: u32) -> bool {
    use crate::ai::Substate;
    matches!(
        substate,
        s if s == Substate::AttackingObserve as u32
            || s == Substate::AttackingObserveAndMove as u32
            || s == Substate::AttackingProtectingWithShield as u32
            || s == Substate::AttackingAdvancingWithShield as u32
            || s == Substate::AttackingBowRunningBehindShieldBearer as u32
            || s == Substate::AttackingBowCorrectingPosition as u32
            || s == Substate::AttackingPhalanx as u32
            || s == Substate::AttackingRunningToPhalanx as u32
            || s == Substate::AttackingBowShooting as u32
            || s == Substate::AttackingBowLoading as u32
            || s == Substate::AttackingBowAiming as u32
            || s == Substate::AttackingBowObserving as u32
            || s == Substate::AttackingBowObservingLoading as u32
    )
}

/// u32-keyed mirror of `Substate::is_any_swordfight`. Used by
/// `ReconsiderSwordfightObservation` to bump multiplicity for allies
/// actively committed to a swordfight against their primary target.
pub(crate) fn is_any_swordfight_substate(substate: u32) -> bool {
    use crate::ai::Substate;
    matches!(
        substate,
        s if s == Substate::AttackingRunningToEnemy as u32
            || s == Substate::AttackingWalkingToEnemy as u32
            || s == Substate::AttackingChargingEnemy as u32
            || s == Substate::AttackingSwordfight as u32
            || s == Substate::AttackingSwordfightSpecialStrike as u32
            || s == Substate::AttackingSwordfightParade as u32
            || s == Substate::AttackingApproachingNewEnemy as u32
            || s == Substate::AttackingSwordfightStepBack as u32
            || s == Substate::AttackingMovingAroundOldEnemy as u32
    )
}

/// The three substates the attack-opportunity gate in
/// `ReconsiderSwordfightObservation` checks: a friend already approaching
/// the same target preempts our opportunistic charge.
pub(super) fn is_walking_running_charging_substate(substate: u32) -> bool {
    use crate::ai::Substate;
    matches!(
        substate,
        s if s == Substate::AttackingWalkingToEnemy as u32
            || s == Substate::AttackingRunningToEnemy as u32
            || s == Substate::AttackingChargingEnemy as u32
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
    all_fighters: &[FighterSnapshot],
    profile_manager: &crate::profiles::ProfileManager,
    iq: u16,
) -> i16 {
    // RHcombatPosition caches this value. EvaluateCombatPosition mutates
    // target directions while walking candidate positions, but damage already
    // estimated for the shared friend/enemy position lists deliberately keeps
    // the first direction's result.
    if cp.estimated_damage != NOT_YET_COMPUTED {
        return cp.estimated_damage;
    }
    if cp.target == 0 {
        cp.estimated_damage = 0;
        return 0;
    }

    // Original: `RHArtificialMalignity::EvaluateCombatPosition` operates on
    // live fighter pointers and dereferences both combatants' `mpSword`.
    // A selected combat position without either fighter is corrupt input, not
    // a harmless zero-damage position.
    let attacker = all_fighters
        .iter()
        .find(|f| f.handle == cp.attacker)
        .unwrap_or_else(|| {
            panic!(
                "combat position attacker {} is absent from the per-tick fighter view",
                cp.attacker
            )
        });
    let target = all_fighters
        .iter()
        .find(|f| f.handle == cp.target)
        .unwrap_or_else(|| {
            panic!(
                "combat position target {} is absent from the per-tick fighter view",
                cp.target
            )
        });
    let evaluator = all_fighters
        .iter()
        .find(|f| f.handle == evaluator)
        .unwrap_or_else(|| {
            panic!(
                "combat position evaluator {} is absent from the per-tick fighter view",
                evaluator
            )
        });

    // Vector from attacker to target.  `dy_iso` applies the isometric
    // Y-stretch for Euclidean distance math; `dy_raw` stays raw for the
    // sector computation below (which applies `ASPECT_RATIO` itself
    // via `vec_to_sector`).
    let dx = cp.target_position.x - cp.attacker_position.x;
    let dy_raw = cp.target_position.y - cp.attacker_position.y;
    let dy_iso = dy_raw * INVERSE_ASPECT_RATIO;
    let sq_dist = dx * dx + dy_iso * dy_iso;

    // Short-circuit: out of maximal range → 0 damage.
    let max_range = attacker.sword_range_maximal as f32;
    if sq_dist > max_range * max_range {
        cp.estimated_damage = 0;
        return 0;
    }

    let mut overall_damage: i32 = 0;

    // Original: `RHProfileManager.h::GetHandToHandProfile` asserts bounds and
    // `RHelementactorhuman.h::GetStandardRangeSword` asserts `mpSword != 0`.
    // Do not turn missing required weapon data into fabricated flat damage.
    let att_prof = profile_manager
        .get_hth_weapon(attacker.hth_weapon_id)
        .unwrap_or_else(|| {
            panic!(
                "fighter {} requires missing HtH weapon profile {}",
                attacker.handle, attacker.hth_weapon_id
            )
        });
    let def_prof = profile_manager
        .get_hth_weapon(target.hth_weapon_id)
        .unwrap_or_else(|| {
            panic!(
                "fighter {} requires missing HtH weapon profile {}",
                target.handle, target.hth_weapon_id
            )
        });
    let evaluator_prof = profile_manager
        .get_hth_weapon(evaluator.hth_weapon_id)
        .unwrap_or_else(|| {
            panic!(
                "fighter {} requires missing HtH weapon profile {}",
                evaluator.handle, evaluator.hth_weapon_id
            )
        });
    {
        let is_rank_soldier = attacker.rank == ProfileRank::Soldier && !attacker.is_pc;
        // `GetProtection` computes `me_to_him_direction = (him - me).sector_0_to_15()`
        // where `me` is the defender — i.e. (attacker - target), the sector
        // from defender to attacker.
        let target_to_attacker_sector = vec_to_sector(-dx, -dy_raw) as i16;

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
                attacker.fighting_ability,
                is_rank_soldier,
            );
            // Original quirk: EstimateDamage uses `pSword` (the combat
            // position's attacker) for range/effects, but `mpMe->GetSword()`
            // (the AI currently evaluating all positions) for strike
            // direction when querying the defender's local protection.
            let strike_dir = crate::combat::get_strike_direction(evaluator_prof, strike);
            let protection = crate::combat::get_sword_protection(
                def_prof,
                cp.target_direction as i16,
                target_to_attacker_sector,
                strike_dir,
                attacker.elevation as f32,
                target.elevation as f32,
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
    let target_look = sector_to_vector(cp.target_direction);
    let from_behind = dot2(target_look, (dx, dy_iso)) > 0.0;
    if from_behind {
        if attacker.is_friendly {
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
    all_fighters: &[FighterSnapshot],
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
#[allow(clippy::too_many_arguments)]
pub(super) fn evaluate_combat_position_full(
    me_handle: HumanHandle,
    me_pos: &Position,
    them_handles: &[HumanHandle],
    cp: &mut CombatPosition,
    friend_positions: &mut [CombatPosition],
    enemy_positions: &mut [CombatPosition],
    all_fighters: &[FighterSnapshot],
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
        // Original truncates MaxNorm into UWORD before applying the
        // fractional distance penalty.
        max_norm(pos_diff(me_pos, &cp.attacker_position)) as u16
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
        if fp.attacker == me_handle {
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
        let is_engaged = enemy_handle == cp.target
            || friend_positions.iter().any(|fp| fp.target == enemy_handle);
        if !is_engaged {
            general_points -= combat::NON_ENGAGED_ENEMY_MALUS;
        }
    }

    (combat::EGOISM_FACTOR * my_points as f32 + friends_points as f32 + general_points as f32)
        as i32
}

/// Finds the opponent of `maurice` who is nearest (MaxNorm) to `rene_pos`.
pub(super) fn calculate_opponent_nearest_to_rene(
    all_fighters: &[FighterSnapshot],
    maurice_handle: HumanHandle,
    rene_pos: &Position,
) -> HumanHandle {
    let maurice = match all_fighters.iter().find(|f| f.handle == maurice_handle) {
        Some(m) => m,
        None => return 0,
    };

    let mut nearest: HumanHandle = 0;
    let mut min_dist = f32::MAX;

    for &opp_handle in &maurice.opponent_handles {
        if let Some(opp) = all_fighters.iter().find(|f| f.handle == opp_handle) {
            let dist = max_norm(pos_diff(rene_pos, &opp.position));
            if dist < min_dist {
                min_dist = dist;
                nearest = opp_handle;
            }
        }
    }

    nearest
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
    /// offset from the ideal cover point is within this MaxNorm, they
    /// stay and shoot instead of repositioning.
    pub const COVER_POINT_TOLERANCE: i32 = 25;
    /// Distance used by `NumberOfNearbyArchersWhoNeedProtection` to
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
mod required_combat_input_tests {
    use super::*;

    fn combat_position() -> CombatPosition {
        CombatPosition {
            attacker: 1,
            target: 2,
            attacker_position: Position::default(),
            target_position: Position {
                x: 10.0,
                ..Position::default()
            },
            ..CombatPosition::default()
        }
    }

    fn fighter(handle: HumanHandle) -> FighterSnapshot {
        FighterSnapshot {
            handle,
            sword_range_maximal: 100,
            hth_weapon_id: 1,
            ..FighterSnapshot::default()
        }
    }

    #[test]
    #[should_panic(expected = "combat position target 2 is absent")]
    fn damage_evaluation_rejects_a_missing_selected_target() {
        let fighters = [fighter(1)];
        let mut position = combat_position();
        estimate_damage(
            1,
            &mut position,
            &fighters,
            &crate::profiles::ProfileManager::new(),
            50,
        );
    }

    #[test]
    #[should_panic(expected = "fighter 1 requires missing HtH weapon profile 1")]
    fn damage_evaluation_rejects_a_missing_required_weapon() {
        let fighters = [fighter(1), fighter(2)];
        let mut position = combat_position();
        estimate_damage(
            1,
            &mut position,
            &fighters,
            &crate::profiles::ProfileManager::new(),
            50,
        );
    }

    #[test]
    fn damage_evaluation_reuses_the_combat_position_cache() {
        let mut position = combat_position();
        position.estimated_damage = 123;
        assert_eq!(
            estimate_damage(
                1,
                &mut position,
                &[],
                &crate::profiles::ProfileManager::new(),
                50,
            ),
            123
        );
    }

    #[test]
    fn combat_position_score_truncates_distance_before_fractional_penalty() {
        let mut position = CombatPosition {
            attacker_position: Position {
                x: 52.9,
                ..Position::default()
            },
            change_position: true,
            ..CombatPosition::default()
        };
        let mut friends = [];
        let mut enemies = [];
        assert_eq!(
            evaluate_combat_position_full(
                1,
                &Position::default(),
                &[],
                &mut position,
                &mut friends,
                &mut enemies,
                &[],
                &crate::profiles::ProfileManager::new(),
                50,
            ),
            -7
        );
    }
}

#[cfg(test)]
mod swordfight_substate_tests {
    use super::*;

    #[test]
    fn broad_swordfight_family_includes_approach_and_special_strike() {
        assert!(is_any_swordfight_substate(
            crate::ai::Substate::AttackingRunningToEnemy as u32
        ));
        assert!(is_any_swordfight_substate(
            crate::ai::Substate::AttackingSwordfightSpecialStrike as u32
        ));
        assert!(!is_any_swordfight_substate(
            crate::ai::Substate::AttackingTooProudToAttack as u32
        ));
    }
}
