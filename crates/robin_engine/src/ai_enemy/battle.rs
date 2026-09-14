//! Combat decision inputs, geometry helpers, and live-operation adapters.

use crate::ai::*;
use crate::fast_find_grid::FastFindGrid;
use crate::parameters_ai;
use crate::position_interface::{ASPECT_RATIO, INVERSE_ASPECT_RATIO};
use crate::sim_rng::SimulationContext;

use super::map_vec_ext::AiMapVec;
use super::util::vec_to_sector;
use super::{
    EnemyAi, PrimaryTargetFlags, ProfileRank, SeekFlags, UNDEFINED_DIRECTION, archer, combat,
};
use crate::coordinates::MapVec;

/// Enemy-approach reconsideration uses raw saved-position map coordinates and stores
/// their Euclidean norm in an unsigned 16-bit value. This is deliberately different from the
/// game's general aspect-corrected distance helpers.
fn reconsider_approach_distance(a: Position, b: Position) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    let truncated = (dx * dx + dy * dy).sqrt() as u16;
    f32::from(truncated)
}

fn enough_nearer_friends_to_observe(
    nearer_friends: u16,
    visible_enemies: usize,
    courage: u16,
) -> bool {
    let visible_enemies = visible_enemies as f32;
    // Preserve Original's floating comparison and operation grouping:
    //   friends >= enemies + enemies * (0.045f * courage)
    // Truncating the courage bonus first lets a soldier observe with too few
    // friends at every non-integral threshold.
    f32::from(nearer_friends)
        >= visible_enemies + visible_enemies * (0.045_f32 * f32::from(courage))
}

/// Derive the original game's nearby-alerting-soldier state from the friends already admitted
/// to the ally list. The admission walk has performed the authoritative 360-degree
/// detection query; querying the camp again here changes both call order and
/// the opaque-visibility cache.
fn has_nearby_alerting_soldier(
    owner: NpcHandle,
    admitted_friends: &[HumanHandle],
    candidates: impl IntoIterator<Item = (NpcHandle, Substate)>,
) -> bool {
    candidates.into_iter().any(|(handle, substate)| {
        handle != owner
            && admitted_friends.contains(&handle)
            && substate == Substate::SeekingRunningToOfficer
    })
}

/// The original game compares the actors' literal squared distance
/// 3D sprite positions, stretches world Y, includes Z, and then truncates the
/// single-precision result to an unsigned 32-bit value before comparing friend distances.
pub(crate) fn battle_owner_target_square_distance(
    owner: crate::coordinates::WorldPoint3D,
    target: crate::coordinates::WorldPoint3D,
) -> u32 {
    let dx = target.x - owner.x;
    let dy = (target.y - owner.y) * INVERSE_ASPECT_RATIO;
    let dz = target.z - owner.z;
    (dx * dx + dy * dy + dz * dz) as u32
}

pub(crate) fn battle_friend_is_nearer(
    friend: Position,
    target: Position,
    owner_target_square_distance: u32,
) -> bool {
    let dx = friend.x - target.x;
    let dy = friend.y - target.y;
    dx * dx + dy * dy < owner_target_square_distance as f32
}

/// Increment primary-target multiplicity: every nearby friend in the
/// broad swordfight family adds another `UNOCCUPIED_PREFERRED` penalty.
pub(crate) fn increment_battle_target_multiplicity(
    multiplicity: &mut std::collections::BTreeMap<HumanHandle, u32>,
    target: HumanHandle,
) {
    let count = multiplicity.entry(target).or_insert(0);
    // The original game stores this counter in an unsigned 16-bit value.
    *count = u32::from((*count as u16).wrapping_add(1));
}

/// Preserve the shared counter for a target appended after battle planning's
/// reset pass. Original resets multiplicity only for the enemies already in
/// enemy list; a nearby friend's previously unseen target retains its live
/// global value when it is inserted later in the same decision.
pub(crate) fn seed_appended_battle_target_multiplicity(
    multiplicity: &mut std::collections::BTreeMap<HumanHandle, u32>,
    target: HumanHandle,
    shared_multiplicity: &std::collections::BTreeMap<HumanHandle, u32>,
) {
    multiplicity
        .entry(target)
        .or_insert_with(|| shared_multiplicity.get(&target).copied().unwrap_or(0));
}

/// Preserve shot-target selection's use of the actors' shared multiplicity scratch:
/// clear all current enemies, then count only friends actively using a bow.
/// A failed shot proposal can immediately fall through to another battle
/// decision, so that later selector must observe this rebuilt state.
fn rebuild_battle_target_multiplicity_for_shot(
    multiplicity: &mut std::collections::BTreeMap<HumanHandle, u32>,
    enemies: &[HumanHandle],
    bow_targets: impl IntoIterator<Item = HumanHandle>,
) {
    multiplicity.clear();
    for &enemy in enemies {
        multiplicity.insert(enemy, 0);
    }
    for target in bow_targets {
        increment_battle_target_multiplicity(multiplicity, target);
    }
}

/// Return the live primary-target claim used by battle decisions' friend
/// scan. This is deliberately independent of both the friend's swordfight
/// opponent list and any earlier enemy-attack target recorded during the
/// same owner pass: later AI work can retarget the primary target while the
/// melee opponent remains unchanged.
fn battle_friend_primary_target(
    state: AiState,
    primary_target: Option<AiEntityHandle>,
) -> Option<HumanHandle> {
    (state == AiState::Attacking)
        .then_some(primary_target)
        .flatten()
        .map(AiEntityHandle::get)
}

impl EnemyAi {
    pub(crate) fn battle_predecision_from_points(
        &self,
        sim: &SimulationContext,
        us_points: u16,
        enemies: u16,
        there_is_an_officer: bool,
        life_points: i16,
        max_life_points: i16,
    ) -> Decision {
        let them_points = enemies.wrapping_mul(100).wrapping_add(1);
        let relation = (u32::from(us_points) * 100 / u32::from(them_points)) as u16;
        let mut odds = if relation >= 100 {
            let raw =
                (50 + 50 * (i32::from(relation) - 100)
                    / parameters_ai::AI_BEST_BATTLE_RELATION_MINUS_100) as i16;
            raw.min(100)
        } else {
            let raw = (50 * (i32::from(relation) - parameters_ai::AI_WORST_BATTLE_RELATION)
                / parameters_ai::AI_100_MINUS_WORST_BATTLE_RELATION) as i16;
            raw.max(0)
        };
        if life_points < max_life_points {
            odds = (i32::from(odds) * i32::from(life_points) / i32::from(max_life_points)) as i16;
        }
        if self.get_rank() == ProfileRank::Soldier && there_is_an_officer {
            odds = (i32::from(odds) * combat::OFFICER_ODDS_BONUS) as i16;
        }
        let courage = self.get_courage();
        if i32::from(odds) < (50 - i32::from(courage) / 2)
            && crate::sim_rng::u16(sim, crate::sim_rng::RngSite::BattleCourage, 0..100) > courage
        {
            Decision::PredecisionDefensive
        } else {
            Decision::PredecisionOffensive
        }
    }

    /// Compute the approach point on `line_idx` closest to the victim.
    /// Returns the point on the aggressor's jump-line B-end mirrored
    /// from the victim's nearest-point projection on the paired line.
    pub(crate) fn compute_jump_line_target(
        &self,
        grid: &FastFindGrid,
        line_idx: u32,
        victim_pos: crate::ai::Position,
    ) -> Option<crate::ai::Position> {
        let aggressor_line = grid.level.jump_lines.get(line_idx as usize)?;
        let victim_line_idx = aggressor_line.associated_line_index?;
        let victim_line = grid.level.jump_lines.get(victim_line_idx as usize)?;
        let t_victim = victim_line.compute_nearest_point_param(crate::coordinates::MapPoint::new(
            victim_pos.x,
            victim_pos.y,
        ));
        let coeff = t_victim * victim_line.norm();
        let aggressor_vec = aggressor_line.vector();
        let aggressor_len = aggressor_line.norm().max(f32::EPSILON);
        let inv_len = 1.0 / aggressor_len;
        Some(crate::ai::Position {
            x: aggressor_line.point_b.x - coeff * aggressor_vec.x * inv_len,
            y: aggressor_line.point_b.y - coeff * aggressor_vec.y * inv_len,
            sector: aggressor_line
                .sector_index
                .and_then(|s| SectorHandle::new(u32::from(s) as u16))
                .or(victim_pos.sector),
            level: aggressor_line.layer,
        })
    }

    // -----------------------------------------------------------------------
    // Rider combat — charge attack logic
    // -----------------------------------------------------------------------

    // Rider charge constants.
    const RIDER_CHARGE_LATERAL_DISTANCE: f32 = 40.0;
    const RIDER_CHARGE_SQR_LATERAL_DISTANCE: f32 = 1600.0;
    const RIDER_CHARGE_LOOP_DISTANCE: f32 = 80.0;
}

/// Accepted output of [`rider_charge_goal_geometry`].
pub(crate) struct RiderChargeGeometry {
    pub forward_dot: f32,
    pub sq_norm: f32,
    pub cos_alpha: f32,
    /// `vMeToHitPoint` — map-space vector from the rider to the hit point.
    pub me_to_hit: (f32, f32),
    /// `vMeToHitPointNormalized` — `me_to_hit / hit_norm_len`.
    pub hit_dir: (f32, f32),
    /// `fMeToHitPointNorm`.
    pub hit_norm_len: f32,
    /// `ptGoal` — charge destination past the hit point.
    pub goal: (f32, f32),
}

/// Rejection reasons, carrying the value each debug print reports.
pub(crate) enum RiderChargeReject {
    Behind { forward_dot: f32 },
    TooNear { norm: f32, sq_norm: f32 },
    ZeroOrthogonal { ortho_len: f32 },
    ZeroHitVector { hp_len: f32 },
    ZeroHitNorm { hit_norm_len: f32 },
}

/// Pure geometry core of rider attack destination selection.
/// during rider-charge setup.
///
/// The charge goal feeds the movement order verbatim, so this math is
/// save-observable to the last bit. Two shapes are easy to get wrong:
///
/// * The nose vector is computed from the facing sector with the
///   **default** aspect ratio `1.0` — the raw
///   stretched-space table entry. Applying `ASPECT_RATIO` and then
///   unapplying `INVERSE_ASPECT_RATIO` lands an ULP off and can flip the
///   forward half-plane test for boundary vectors.
/// * Both vector-scaling sites round their
///   scalar **once** before touching the components (`Set(k*mX, k*mY)`,
///   vector operation): `k1 = RIDER_CHARGE_LATERAL_DISTANCE /
///   fCosAlpha` and `k2 = fCosAlpha *
/// the normalized enemy vector. Distributing the multiply per component
///   (`n.x * 40.0 / cos`) double-rounds differently; nicouzouf
///   Savegame_047 Soldier51's frame-563 charge goal came out one ULP low
///   in Y that way, which shifted the spliced running-order goal, its
///   normalized increment, and every subsequent walk step.
///
/// The `f32::EPSILON` degenerate-input rejections have no Original
/// counterpart (it would divide by zero and assert in debug); they are
/// unreachable for the finite, >= 40-unit vectors that pass the earlier
/// gates.
pub(crate) fn rider_charge_goal_geometry(
    my_pos: (f32, f32),
    my_dir: u16,
    enemy_pos: (f32, f32),
) -> Result<RiderChargeGeometry, RiderChargeReject> {
    // vMeToEnemyStretchedY = ptEnemy - ptMe;  .mY *= INVERSE_ASPECT_RATIO
    let me_to_enemy_sy = (
        enemy_pos.0 - my_pos.0,
        (enemy_pos.1 - my_pos.1) * INVERSE_ASPECT_RATIO,
    );

    // Facing-sector vector — default aspect 1.0.
    let nose_sy = MapVec::from_sector_with_aspect(my_dir, 1.0);

    // Is the enemy before me?
    let forward_dot = nose_sy.dot(MapVec::new(me_to_enemy_sy.0, me_to_enemy_sy.1));
    if forward_dot < 0.0 {
        return Err(RiderChargeReject::Behind { forward_dot });
    }

    // fMeToEnemySquareNorm / fMeToEnemyNorm.
    let sq_norm = me_to_enemy_sy.0 * me_to_enemy_sy.0 + me_to_enemy_sy.1 * me_to_enemy_sy.1;
    let norm = sq_norm.sqrt();
    if norm < EnemyAi::RIDER_CHARGE_LATERAL_DISTANCE {
        return Err(RiderChargeReject::TooNear { norm, sq_norm });
    }

    // fCosAlpha = sqrt( 1.0f - RIDER_CHARGE_SQR_LATERAL_DISTANCE / fMeToEnemySquareNorm )
    let cos_alpha = (1.0 - EnemyAi::RIDER_CHARGE_SQR_LATERAL_DISTANCE / sq_norm).sqrt();

    // The clockwise normal with aspect 1.0 is (y, -x); normalize it next.
    let ortho = (me_to_enemy_sy.1, -me_to_enemy_sy.0);
    let ortho_len = (ortho.0 * ortho.0 + ortho.1 * ortho.1).sqrt();
    if ortho_len < f32::EPSILON {
        return Err(RiderChargeReject::ZeroOrthogonal { ortho_len });
    }
    let ortho_norm = (ortho.0 / ortho_len, ortho.1 / ortho_len);
    // operator*=: one rounded scalar, then k1 * component.
    let k1 = EnemyAi::RIDER_CHARGE_LATERAL_DISTANCE / cos_alpha;
    let ortho_scaled = (k1 * ortho_norm.0, k1 * ortho_norm.1);

    // vMeToHitPointStretchedY = vMeToEnemyStretchedY + orthogonal; Normalize().
    let hit_point_sy = (
        me_to_enemy_sy.0 + ortho_scaled.0,
        me_to_enemy_sy.1 + ortho_scaled.1,
    );
    let hp_len = (hit_point_sy.0 * hit_point_sy.0 + hit_point_sy.1 * hit_point_sy.1).sqrt();
    if hp_len < f32::EPSILON {
        return Err(RiderChargeReject::ZeroHitVector { hp_len });
    }
    let hp_norm = (hit_point_sy.0 / hp_len, hit_point_sy.1 / hp_len);
    // operator*=: one rounded scalar, then k2 * component.
    let k2 = cos_alpha * norm;
    let hp_scaled = (k2 * hp_norm.0, k2 * hp_norm.1);

    // vMeToHitPoint — reapply the aspect ratio to Y.
    let me_to_hit = (hp_scaled.0, hp_scaled.1 * ASPECT_RATIO);

    // Scale the normalized hit-point-to-goal vector by the charge-loop distance.
    // ptGoal = ptMe + vMeToHitPoint + vHitPointToGoal.
    let hit_norm_len = (me_to_hit.0 * me_to_hit.0 + me_to_hit.1 * me_to_hit.1).sqrt();
    if hit_norm_len < f32::EPSILON {
        return Err(RiderChargeReject::ZeroHitNorm { hit_norm_len });
    }
    let hit_dir = (me_to_hit.0 / hit_norm_len, me_to_hit.1 / hit_norm_len);
    let goal = (
        my_pos.0 + me_to_hit.0 + hit_dir.0 * EnemyAi::RIDER_CHARGE_LOOP_DISTANCE,
        my_pos.1 + me_to_hit.1 + hit_dir.1 * EnemyAi::RIDER_CHARGE_LOOP_DISTANCE,
    );

    Ok(RiderChargeGeometry {
        forward_dot,
        sq_norm,
        cos_alpha,
        me_to_hit,
        hit_dir,
        hit_norm_len,
        goal,
    })
}

#[cfg(test)]
mod tests;
#[test]
fn battle_target_multiplicity_stacks_duplicate_friend_claims_as_uword() {
    let mut multiplicity = std::collections::BTreeMap::from([(174, 0)]);

    increment_battle_target_multiplicity(&mut multiplicity, 174);
    increment_battle_target_multiplicity(&mut multiplicity, 174);

    assert_eq!(multiplicity[&174], 2);

    multiplicity.insert(174, u32::from(u16::MAX));
    increment_battle_target_multiplicity(&mut multiplicity, 174);

    assert_eq!(multiplicity[&174], 0);
}

#[test]
fn appended_battle_target_retains_global_multiplicity_after_personal_reset() {
    // Battle planning resets only the target already in its personal enemy
    // list. A target appended by a nearby friend keeps the shared counter.
    let mut decision = std::collections::BTreeMap::from([(343, 0)]);
    let global = std::collections::BTreeMap::from([(343, 4), (345, 1)]);

    seed_appended_battle_target_multiplicity(&mut decision, 343, &global);
    seed_appended_battle_target_multiplicity(&mut decision, 345, &global);

    assert_eq!(decision[&343], 0, "personal target stays reset");
    assert_eq!(decision[&345], 1, "appended target retains shared count");

    increment_battle_target_multiplicity(&mut decision, 345);
    assert_eq!(decision[&345], 2, "a live friend claim still stacks");
}

#[test]
fn appended_battle_target_observes_an_earlier_owners_serial_reset() {
    // Task #146: S131 resets PC101's shared 16-bit value before S178 appends that
    // target in a later owner slot. Re-deriving occupancy from live fighter
    // states would resurrect the stale claim and count it twice.
    let mut shared = std::collections::BTreeMap::from([(101, 1)]);
    shared.insert(101, 0);

    let mut later_decision = std::collections::BTreeMap::from([(100, 0)]);
    seed_appended_battle_target_multiplicity(&mut later_decision, 101, &shared);
    increment_battle_target_multiplicity(&mut later_decision, 101);
    increment_battle_target_multiplicity(&mut shared, 101);

    assert_eq!(later_decision[&101], 1);
    assert_eq!(shared[&101], 1, "later owners retain the serial mutation");
}

#[test]
fn failed_shot_proposal_resets_melee_multiplicity_before_observe_fallback() {
    // Task #134: two swordfighters claimed target 174 during
    // battle planning, but the archer's shot-target proposal reset the shared
    // counters before returning no shot. The ensuing Observe selector must
    // therefore see zero melee claims (plus only any live bow claims).
    let mut decision = std::collections::BTreeMap::from([(172, 1), (171, 0), (174, 2)]);

    rebuild_battle_target_multiplicity_for_shot(&mut decision, &[172, 171, 174], []);

    assert_eq!(
        decision,
        std::collections::BTreeMap::from([(171, 0), (172, 0), (174, 0)])
    );

    rebuild_battle_target_multiplicity_for_shot(&mut decision, &[172, 171, 174], [171, 171]);
    assert_eq!(decision[&171], 2, "bow claims are rebuilt after the reset");
    assert_eq!(decision[&174], 0, "stale melee claims remain cleared");
}

#[test]
fn battle_friend_claim_uses_primary_target_not_swordfight_opponent() {
    // Task #61/#134 control: both friends still had PC174 in their melee
    // opponent lists, while their live AI primary target had retargeted to
    // PC173. An earlier same-frame attack on enemy 174 must not
    // overwrite the later primary-target value used by battle decisions.
    let swordfight_opponent = 174;
    let stale_attack_enemy_claim = swordfight_opponent;
    let live_primary_target = 173;
    let mut multiplicity =
        std::collections::BTreeMap::from([(live_primary_target, 0), (swordfight_opponent, 0)]);

    for _ in 0..2 {
        let target = battle_friend_primary_target(
            AiState::Attacking,
            Some(AiEntityHandle::new(live_primary_target)),
        )
        .expect("attacking friend has a live primary target");
        assert_ne!(target, stale_attack_enemy_claim);
        increment_battle_target_multiplicity(&mut multiplicity, target);
    }

    assert_eq!(multiplicity[&live_primary_target], 2);
    assert_eq!(multiplicity[&swordfight_opponent], 0);
}

impl EnemyAi {}

#[derive(serde::Serialize, serde::Deserialize)]
/// Decision-local aggregates handed from `battle_decisions` to the
/// decision-tree pieces.
#[derive(Clone, Copy)]
pub(crate) struct BattleDecisionInputs {
    pub(crate) friends_lower_company: u16,
    pub(crate) soldiers_lower_pride: bool,
    pub(crate) simple_soldiers_near: bool,
    pub(crate) alerting_soldier_near: bool,
    pub(crate) min_square_enemy_distance: u32,
    pub(crate) num_enemies_i_can_see: usize,
    pub(crate) friends_nearer_to_enemy: u16,
}
