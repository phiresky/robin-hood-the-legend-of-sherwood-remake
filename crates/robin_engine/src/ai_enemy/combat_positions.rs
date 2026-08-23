//! Combat positions, phalanx/shield-bearer formation, archery
//! shooting-point selection, and the swordfight repositioning loop.
//!
//! Owns the helpers used by `propose_good_combat_position`,
//! `reconsider_swordfight`, `reconsider_swordfight_observation`, and
//! `refresh_arrow_protection`. Also exposes `find_fighter`,
//! `is_allowed_to_attack`, and the neighbour predicates.

use crate::ai::*;
use crate::parameters_ai;
use crate::position_interface::{ASPECT_RATIO, INVERSE_ASPECT_RATIO};

use super::util::{
    FighterView, ai_max_norm_distance, ai_square_distance, ai_square_distance_world,
    calculate_opponent_nearest_to_rene, check_straight_movement, det2, dot2,
    evaluate_combat_position_full, get_normal, get_normal_iso, get_normal_right,
    is_any_swordfight_substate, is_observing_combat_substate, is_walking_running_charging_substate,
    iso_norm, iso_normalize, max_norm, pos_diff, sector_to_vector, sector_to_vector_iso,
    square_norm, vec_to_sector, vec_to_sector_ar,
};
use super::{
    CombatPosition, EnemyAi, FighterSnapshot, PrimaryTargetFlags, ProfileRank, Question, SeekFlags,
    UNDEFINED_DIRECTION, archer, combat, propose_good_step_back_goal,
};

fn reconsider_observation_debug_matches(frame: u32, owner: u32) -> bool {
    if std::env::var_os("PARITY_DEBUG_RECONSIDER_OBSERVATION").is_none() {
        return false;
    }
    let parse_filter = |name: &str| {
        std::env::var(name).ok().map(|value| {
            value.parse::<u32>().unwrap_or_else(|error| {
                panic!("invalid {name}={value:?} for RECONSIDER diagnostic: {error}")
            })
        })
    };
    parse_filter("PARITY_DEBUG_RECONSIDER_OBSERVATION_FRAME")
        .is_none_or(|expected| frame == expected)
        && parse_filter("PARITY_DEBUG_RECONSIDER_OBSERVATION_OWNER_HANDLE")
            .is_none_or(|expected| owner == expected)
}

/// Opt-in trace for the event-driven swordfight reposition decision. Keep the
/// gate process-local and evaluate it before touching any proposal data so the
/// disabled path cannot add lookups, RNG draws, or simulation state.
pub(super) fn reconsider_position_debug_matches(
    frame: impl FnOnce() -> u32,
    creation_order: impl FnOnce() -> Option<u32>,
    owner: impl FnOnce() -> u32,
) -> bool {
    if std::env::var_os("PARITY_DEBUG_RECONSIDER_POSITION").is_none() {
        return false;
    }
    let parse_filter = |name: &str| {
        std::env::var(name).ok().map(|value| {
            value.parse::<u32>().unwrap_or_else(|error| {
                panic!("invalid {name}={value:?} for RECONSIDER_POSITION diagnostic: {error}")
            })
        })
    };
    let frame = frame();
    let creation_order = creation_order();
    let owner = owner();
    parse_filter("PARITY_DEBUG_RECONSIDER_POSITION_FRAME").is_none_or(|expected| frame == expected)
        && parse_filter("PARITY_DEBUG_RECONSIDER_POSITION_CREATION_ORDER")
            .is_none_or(|expected| creation_order == Some(expected))
        && parse_filter("PARITY_DEBUG_RECONSIDER_POSITION_OWNER_HANDLE")
            .is_none_or(|expected| owner == expected)
}

/// Opt-in trace for the periodic shield-protection decision. The master gate
/// is checked before either closure is evaluated so a disabled diagnostic
/// performs no additional simulation-state reads.
fn arrow_protection_debug_matches(
    frame: impl FnOnce() -> u32,
    owner: impl FnOnce() -> u32,
) -> bool {
    if std::env::var_os("PARITY_DEBUG_ARROW_PROTECTION").is_none() {
        return false;
    }
    let parse_filter = |name: &str| {
        std::env::var(name).ok().map(|value| {
            value.parse::<u32>().unwrap_or_else(|error| {
                panic!("invalid {name}={value:?} for ARROW_PROTECTION diagnostic: {error}")
            })
        })
    };
    let frame = frame();
    let owner = owner();
    parse_filter("PARITY_DEBUG_ARROW_PROTECTION_FRAME").is_none_or(|expected| frame == expected)
        && parse_filter("PARITY_DEBUG_ARROW_PROTECTION_OWNER_HANDLE")
            .is_none_or(|expected| owner == expected)
}

/// Resolve the point stored in `RHFIELD_SHIELD_DANGER_POINT`.
///
/// The Original reads `RHElement::GetPosition()` here.  This deliberately
/// bypasses the AI-facing position, which may be a door endpoint while the
/// target is passing through a gate.
fn shield_danger_point(
    fighter: Option<&FighterSnapshot>,
    view: Option<&crate::ai_entity_view::AiEntityView>,
) -> Option<(Position, f32)> {
    fighter
        .map(|fighter| (fighter.raw_position, fighter.elevation))
        .or_else(|| {
            view.map(|view| {
                let mut raw_position = view.position;
                raw_position.x = view.detection_position.x;
                raw_position.y = view.detection_position.y;
                (raw_position, view.detection_position_world.z)
            })
        })
}

fn original_uword_norm(delta: (f32, f32)) -> u16 {
    (delta.0 * delta.0 + delta.1 * delta.1).sqrt() as u16
}

fn nearest_phalanx_enemy_index(distances: impl IntoIterator<Item = (usize, f32)>) -> Option<usize> {
    // Original narrows each MaxNorm result to UWORD before comparing it.
    // Sub-unit differences are therefore ties and preserve InsertLast order.
    let mut nearest = None;
    let mut minimum = 65_432_u16;
    for (index, distance) in distances {
        let distance = distance as u16;
        if distance < minimum {
            minimum = distance;
            nearest = Some(index);
        }
    }
    nearest
}

/// The forward and rightward steps used by Original's `ReconsiderPhalanx`.
///
/// Both operations are aspect-aware `SBGeoVector2D` operations. In
/// particular, `GetNormal(true, ASPECT_RATIO)` is not a plain screen-space
/// clockwise rotation: it rotates in stretched world space, then projects
/// the result back to map space.
fn phalanx_advance_vectors(to_target: (f32, f32)) -> ((f32, f32), (f32, f32)) {
    let forward = iso_normalize(to_target, ASPECT_RATIO);
    let forward_step = (
        forward.0 * archer::PHALANX_FORWARD_STEP as f32,
        forward.1 * archer::PHALANX_FORWARD_STEP as f32,
    );

    let right = iso_normalize(
        get_normal_iso(forward_step, true, ASPECT_RATIO),
        ASPECT_RATIO,
    );
    let right_step = (
        right.0 * archer::DISTANCE_SHIELD_BEARER_SHIELD_BEARER as f32,
        right.1 * archer::DISTANCE_SHIELD_BEARER_SHIELD_BEARER as f32,
    );

    (forward_step, right_step)
}

/// Compare the exact sector identities inherited by an
/// `RHposition +/- vector` result. `Position` copies its `SectorHandle`, whose
/// optional arena companion is the live analogue of Original's `RHSector*`.
fn inherited_position_crosses_sector_identity(current: &Position, anchor: &Position) -> bool {
    let (Some(current_sector), Some(anchor_sector)) = (current.sector, anchor.sector) else {
        return false;
    };
    match (current_sector.arena_index(), anchor_sector.arena_index()) {
        (Some(current), Some(anchor)) => current != anchor,
        // Compatibility for old Rust snapshots which predate arena identity.
        // Never infer an identity from coordinates: only the public topology
        // remains available on this path.
        _ => current_sector != anchor_sector || current.level != anchor.level,
    }
}

fn is_facing_swordfight_target(
    me_position: &Position,
    me_elevation: f32,
    me_direction: u16,
    target_position: &Position,
    target_elevation: f32,
) -> bool {
    // Original compares `GetPositionGround()` values here. `Position`
    // stores projected map Y, so reconstruct ground/world Y by adding
    // elevation before applying `GetSector0to15(ASPECT_RATIO)`.
    let to_target = (
        target_position.x - me_position.x,
        (target_position.y + target_elevation) - (me_position.y + me_elevation),
    );
    let target_sector = vec_to_sector(to_target.0, to_target.1);
    let facing_delta = (me_direction as i32 + 16 - target_sector as i32).rem_euclid(16);
    matches!(facing_delta, 15 | 0 | 1)
}

fn swordfight_facing_target_position<'a>(
    primary: &'a FighterSnapshot,
    tick: &'a AiPerTickData,
) -> &'a Position {
    // ReconsiderSwordfight uses GetPositionGround() for this guard, not the
    // AI Position() helper used by the fighter snapshots. The distinction is
    // observable while the opponent passes a door: Position() forecasts the
    // committed gate side, whereas GetPositionGround() remains at the live
    // actor position until movement advances it.
    // The target can change synchronously after this tick snapshot was built
    // (notably when the principal opponent is refreshed above). Never pair a
    // replacement target with the preceding target's literal position.
    if tick.primary_target_snapshot_handle == primary.handle {
        tick.primary_target_live_position
            .as_ref()
            .unwrap_or(&primary.position)
    } else {
        &primary.position
    }
}

#[track_caller]
fn phalanx_member_detects_360(
    member: &PhalanxMemberThemList,
    target: &PhalanxEnemySnapshot,
    obstacles: crate::sight_obstacle::ObstacleList<'_>,
) -> bool {
    if !member.active || member.in_building || !target.active || target.in_building {
        return false;
    }

    let viewer_eye_z = member.elevation
        + crate::stealth::eye_z_for_posture(crate::element::Posture::Upright, member.is_rider);
    let target_xy = crate::stealth::detection_point_xy(
        crate::coordinates::MapPoint::new(target.position.x, target.position.y),
        target.posture,
        target.direction as i16,
    );
    let target_z =
        target.elevation + crate::stealth::detection_z_for_posture(target.posture, target.is_rider);
    let viewer_ground = crate::coordinates::GroundPoint::from_map_and_z(
        crate::coordinates::MapPoint::new(member.position.x, member.position.y),
        member.elevation,
    );
    let target_ground =
        crate::coordinates::GroundPoint::from_map_and_z(target_xy, target.elevation);
    let dx = target_ground.x - viewer_ground.x;
    let dy = (target_ground.y - viewer_ground.y) * INVERSE_ASPECT_RATIO;
    let dz = target_z - viewer_eye_z;
    if dx * dx + dy * dy + dz * dz > member.sq_view_radius {
        return false;
    }

    crate::sight_obstacle::is_reachable_3d(
        obstacles,
        [viewer_ground.x, viewer_ground.y, viewer_eye_z],
        [target_ground.x, target_ground.y, target_z],
        crate::sight_obstacle::SIGHTOBSTACLE_OPAQUE,
    )
}

#[track_caller]
fn phalanx_member_detects_180(
    member: &PhalanxMemberThemList,
    target: &PhalanxEnemySnapshot,
    ctx: &AiContext,
) -> bool {
    if !member.active || member.in_building || !target.active {
        return false;
    }

    let viewer_xy = crate::stealth::eye_point_xy(
        crate::coordinates::MapPoint::new(member.position.x, member.position.y),
        member.posture,
        member.direction as i16,
        false,
    );
    let viewer_z =
        member.elevation + crate::stealth::eye_z_for_posture(member.posture, member.is_rider);
    let target_xy = crate::stealth::detection_point_xy(
        crate::coordinates::MapPoint::new(target.position.x, target.position.y),
        target.posture,
        target.direction as i16,
    );
    let target_z =
        target.elevation + crate::stealth::detection_z_for_posture(target.posture, target.is_rider);
    let viewer_ground =
        crate::coordinates::GroundPoint::from_map_and_z(viewer_xy, member.elevation);
    let target_ground =
        crate::coordinates::GroundPoint::from_map_and_z(target_xy, target.elevation);
    let dx = target_ground.x - viewer_ground.x;
    let dy = (target_ground.y - viewer_ground.y) * INVERSE_ASPECT_RATIO;
    let sq_distance = dx * dx + dy * dy;
    if sq_distance > member.sq_view_radius {
        return false;
    }

    // The direction vector this test compares against is built in map
    // space, where Y is already compressed by `ASPECT_RATIO`, and is then
    // expanded back into the stretched frame the offsets above use. The
    // shared table is the expanded unit vector already, so stretching it
    // a second time would narrow the forward half-plane and reject
    // enemies that are genuinely in front.
    let direction = crate::shadow_polygon::sector_to_direction(member.direction as i16);
    let forward_x = direction[0];
    let forward_y = direction[1];
    if sq_distance < 50.0 * 50.0 {
        let forward_length = dx * forward_x + dy * forward_y;
        let projected_x = forward_x * forward_length;
        let projected_y = forward_y * forward_length;
        let perpendicular_sq =
            (dx - projected_x) * (dx - projected_x) + (dy - projected_y) * (dy - projected_y);
        if perpendicular_sq >= forward_length {
            return true;
        }
    }
    if dx * forward_x + dy * forward_y < 0.0 {
        return false;
    }

    // Original calls ComputeViewRadius here, before the final target LOS.
    // At night/fog that call emits the ordered light-sector barycentre rays;
    // its per-surface memo is observable because later detection calls can
    // reuse the radius without repeating those rays.
    let obstacles = ctx.obstacle_list();
    let target_obstacle = target.obstacle.map(|handle| {
        obstacles.get(usize::from(handle)).unwrap_or_else(|| {
            panic!(
                "phalanx detection target {} requires missing sight obstacle {}",
                target.handle,
                u16::from(handle)
            )
        })
    });
    let effective_view_radius =
        ctx.compute_view_radius_cached(member.entity, target.obstacle, || {
            crate::ai_vision::compute_view_radius(
                crate::coordinates::WorldPoint3D::new(viewer_ground.x, viewer_ground.y, viewer_z),
                member.view_radius,
                (member.view_direction[0], member.view_direction[1]),
                member.real_half_aperture,
                ctx.is_night_or_fog,
                &ctx.fast_grid,
                obstacles,
                target_obstacle,
            )
        });
    if sq_distance > effective_view_radius * effective_view_radius {
        return false;
    }

    crate::sight_obstacle::is_reachable_3d(
        obstacles,
        [viewer_ground.x, viewer_ground.y, viewer_z],
        [target_ground.x, target_ground.y, target_z],
        crate::sight_obstacle::SIGHTOBSTACLE_OPAQUE,
    )
}

fn append_phalanx_member_enemies(
    merged: &mut Vec<HumanHandle>,
    member: &PhalanxMemberThemList,
    kept: &[&PhalanxEnemySnapshot],
    ctx: &AiContext,
) {
    let obstacles = ctx.obstacle_list();
    tracing::trace!(
        target: "robin_engine::ai_enemy::phalanx",
        member = member.handle,
        sq_view_radius = member.sq_view_radius,
        kept = ?kept.iter().map(|t| t.handle).collect::<Vec<_>>(),
        detectable = ?member.detectable_enemies.iter().map(|t| t.handle).collect::<Vec<_>>(),
        "phalanx them-list: member inputs"
    );
    for target in kept {
        // Short-circuit order matters: an entry that can no longer fight
        // is dropped without ever running a line-of-sight query.
        let keep = target.able_to_fight
            && phalanx_member_detects_360(member, target, obstacles)
            && !target.friend;
        tracing::trace!(
            target: "robin_engine::ai_enemy::phalanx",
            member = member.handle,
            enemy = target.handle,
            able = target.able_to_fight,
            friend = target.friend,
            keep,
            "phalanx them-list: keep check"
        );
        if !keep {
            continue;
        }
        if !merged.contains(&target.handle) {
            merged.push(target.handle);
        }
    }

    for target in &member.detectable_enemies {
        // The detection test runs before the dead/unconscious filter, so
        // it queries line of sight even for targets about to be rejected.
        let detects = phalanx_member_detects_180(member, target, ctx);
        tracing::trace!(
            target: "robin_engine::ai_enemy::phalanx",
            member = member.handle,
            enemy = target.handle,
            dead = target.dead,
            unconscious = target.unconscious,
            detects_180 = detects,
            "phalanx them-list: add check"
        );
        if !detects || target.dead || target.unconscious {
            continue;
        }
        if !merged.contains(&target.handle) {
            merged.push(target.handle);
        }
    }
}

impl EnemyAi {
    /// Original `ReconsiderSwordfightObservation` uses `Panic` when its
    /// defensive step-back has no goal. This is intentionally not `Flee`,
    /// whose first statement is `Say(REMARK_PANIC)`.
    fn panic_after_failed_observation_step_back(
        &mut self,
        enemy_pos: Position,
    ) -> crate::ai::PanicRequest {
        self.panic_from_position(enemy_pos, parameters_ai::AI_STANDARD_PANIC_RUNS as u8);
        // `Panic` is synchronous in Original, but Rust drains this actor work
        // after the AI borrow is released. Hold it until the rest of
        // ReconsiderSwordfightObservation has had a chance to override the
        // panic with AttackEnemy/observe work from later statements.
        self.base
            .outbox
            .actor
            .begin_panic
            .take()
            .expect("panic_from_position must stage BeginPanic")
    }

    // -----------------------------------------------------------------------
    // Combat-position helpers (used by ProposeGoodCombatPosition)
    // -----------------------------------------------------------------------

    /// Look up a fighter snapshot by handle in the engine-provided cache.
    pub(super) fn find_fighter<'a>(
        &self,
        handle: HumanHandle,
        tick: &'a AiPerTickData,
    ) -> Option<&'a FighterSnapshot> {
        tick.nearby_fighters
            .iter()
            .find(|f| f.handle == handle)
            .or_else(|| tick.fighter_registry.iter().find(|f| f.handle == handle))
    }

    /// IsAllowedToAttack — VIP / mission rules.
    ///
    /// Pure VIP/Robin gate. Does NOT filter on friendliness or
    /// `is_able_to_fight` — those are caller responsibilities (the
    /// reference dereferences the caller-supplied pointer without those
    /// guards). Resolves the target via the broader `entity_view` map
    /// first so callers passing a handle outside the 500px
    /// `nearby_fighters` snapshot still get a meaningful answer.
    pub(super) fn is_allowed_to_attack(
        &self,
        target: HumanHandle,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        // Prefer entity_view (broader population) for the VIP/Robin/PC
        // properties; fall back to the fighter snapshot if absent.
        let (target_is_pc, target_is_robin, target_is_vip) =
            if let Some(view) = ctx.entity_view(target) {
                (view.is_pc, view.is_robin, view.is_vip)
            } else if let Some(adversary) = self.find_fighter(target, tick) {
                (adversary.is_pc, adversary.is_robin, adversary.is_vip)
            } else {
                // No info available — the reference would dereference
                // the pointer (no guard) and assume the target is valid;
                // match that.
                tracing::warn!(
                    me = self.base.me,
                    target,
                    "is_allowed_to_attack: target not in entity_view or fighter snapshot"
                );
                return true;
            };

        // Rule 1: VIPs can only begin combat with Robin.
        if self.is_vip && (!target_is_pc || !target_is_robin) {
            return false;
        }

        // Rule 2: Soldiers cannot begin combat with VIP NPCs.
        if !target_is_pc && target_is_vip {
            return false;
        }

        true
    }

    /// CanBeLeftNeighbour: only soldiers count, must look the same way
    /// as me, and must lie to my left when projected through my facing.
    fn can_be_left_neighbour(
        &self,
        neighbour: &FighterSnapshot,
        ctx: &AiContext,
        _tick: &AiPerTickData,
    ) -> bool {
        if neighbour.is_pc || neighbour.rank != ProfileRank::Soldier {
            return false;
        }
        let my_nose = sector_to_vector(ctx.direction);
        let his_nose = sector_to_vector(neighbour.direction);
        if dot2(my_nose, his_nose) < 0.0 {
            return false;
        }
        let mut to_friend = pos_diff(&neighbour.position, &ctx.position);
        to_friend.1 *= INVERSE_ASPECT_RATIO;
        det2(my_nose, to_friend) < 0.0
    }

    /// CanBeRightNeighbour.
    fn can_be_right_neighbour(
        &self,
        neighbour: &FighterSnapshot,
        ctx: &AiContext,
        _tick: &AiPerTickData,
    ) -> bool {
        if neighbour.is_pc || neighbour.rank != ProfileRank::Soldier {
            return false;
        }
        let my_nose = sector_to_vector(ctx.direction);
        let his_nose = sector_to_vector(neighbour.direction);
        if dot2(my_nose, his_nose) < 0.0 {
            return false;
        }
        let mut to_friend = pos_diff(&neighbour.position, &ctx.position);
        to_friend.1 *= INVERSE_ASPECT_RATIO;
        det2(my_nose, to_friend) > 0.0
    }

    /// ProposeLeftAndRightNeighbour. Picks the nearest friendly soldier
    /// on each side that I can fall in formation with, preferring the
    /// already-cached neighbour if it's still valid.
    fn propose_left_and_right_neighbour(
        &self,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> (HumanHandle, HumanHandle) {
        let me_pos = ctx.position;

        let pick_neighbour = |cached: HumanHandle, side_left: bool| -> HumanHandle {
            if cached != 0
                && let Some(snap) = self.find_fighter(cached, tick)
            {
                let ok = if side_left {
                    self.can_be_left_neighbour(snap, ctx, tick)
                } else {
                    self.can_be_right_neighbour(snap, ctx, tick)
                };
                if ok {
                    return cached;
                }
            }
            let mut best: HumanHandle = 0;
            let mut best_sq = f32::MAX;
            for handle in &self.base.list_us {
                if *handle == self.base.me {
                    continue;
                }
                let Some(snap) = self.find_fighter(*handle, tick) else {
                    continue;
                };
                let ok = if side_left {
                    self.can_be_left_neighbour(snap, ctx, tick)
                } else {
                    self.can_be_right_neighbour(snap, ctx, tick)
                };
                if !ok {
                    continue;
                }
                // RHArtificialIntelligence::SquareDistance receives the
                // element pointer and reads RHElement::GetPosition().  Keep
                // that literal body point for ranking neighbours; `position`
                // is the door-aware AI Position() and may already be snapped
                // to a gate endpoint.
                let sq = ai_square_distance(
                    &snap.raw_position,
                    snap.elevation as f32,
                    &me_pos,
                    ctx.elevation,
                );
                if sq < best_sq {
                    best_sq = sq;
                    best = *handle;
                }
            }
            best
        };

        let left = pick_neighbour(self.left_combat_neighbour, true);
        let right = pick_neighbour(self.right_combat_neighbour, false);
        (left, right)
    }

    /// ProposeLinePositionsThere. Drops a line-formation candidate at
    /// `there` facing `direction`, for every them-list enemy reachable
    /// from it.
    #[allow(clippy::too_many_arguments)]
    fn propose_line_positions_there(
        &self,
        list: &mut Vec<CombatPosition>,
        there: Position,
        direction: (f32, f32),
        left_neighbour: HumanHandle,
        right_neighbour: HumanHandle,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) {
        // Early return if the proposed line position is not reachable in
        // a straight line.
        if let Some(grid) = grid {
            let me_pt = crate::coordinates::MapPoint::new(ctx.position.x, ctx.position.y);
            let there_pt = crate::coordinates::MapPoint::new(there.x, there.y);
            if !grid.is_straight_movement_authorized(me_pt, there_pt, there.level, &ctx.move_box) {
                return;
            }
        }

        let weapon_distance = self
            .find_fighter(self.base.me, tick)
            .map(|f| f.sword_range_uber)
            .unwrap_or(self.sword_range) as f32;
        let weapon_sq = weapon_distance * weapon_distance;

        for enemy_handle in &self.list_them {
            let Some(enemy) = self.find_fighter(*enemy_handle, tick) else {
                continue;
            };
            let v = pos_diff(&enemy.position, &there);
            if max_norm(v) >= weapon_distance {
                continue;
            }
            if square_norm(v) >= weapon_sq {
                continue;
            }
            if dot2(v, direction) <= 0.0 {
                continue;
            }
            let me_pos = ctx.position;
            let cp = CombatPosition {
                attacker: self.base.me,
                attacker_position: there,
                target: *enemy_handle,
                target_position: enemy.position,
                target_direction: enemy.direction,
                change_position: max_norm(pos_diff(&there, &me_pos)) > 3.0,
                line_position: true,
                left_neighbour,
                right_neighbour,
                bonus: combat::LINE_FORMATION_BONUS as i16,
                ..CombatPosition::default()
            };
            list.push(cp);
        }
    }

    /// ProposeCombatPositionsLeftOf.
    fn propose_combat_positions_left_of(
        &self,
        list: &mut Vec<CombatPosition>,
        right_neighbour_handle: HumanHandle,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) {
        let Some(right) = self.find_fighter(right_neighbour_handle, tick) else {
            return;
        };
        let me_pos = ctx.position;
        if right.position.sector != me_pos.sector {
            return;
        }

        let mut nose_friend = sector_to_vector(right.direction);
        let sidewards_raw = get_normal(nose_friend);
        let mut sidewards = (
            sidewards_raw.0 * combat::STANDARD_LINE_DISTANCE as f32,
            sidewards_raw.1 * combat::STANDARD_LINE_DISTANCE as f32,
        );
        nose_friend.1 *= ASPECT_RATIO;
        sidewards.1 *= ASPECT_RATIO;

        let new_pos = Position {
            x: right.position.x - sidewards.0,
            y: right.position.y - sidewards.1,
            ..right.position
        };
        self.propose_line_positions_there(
            list,
            new_pos,
            nose_friend,
            0,
            right_neighbour_handle,
            ctx,
            tick,
            grid,
        );
    }

    /// ProposeCombatPositionsRightOf.
    fn propose_combat_positions_right_of(
        &self,
        list: &mut Vec<CombatPosition>,
        left_neighbour_handle: HumanHandle,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) {
        let Some(left) = self.find_fighter(left_neighbour_handle, tick) else {
            return;
        };
        let me_pos = ctx.position;
        if left.position.sector != me_pos.sector {
            return;
        }

        let mut nose_friend = sector_to_vector(left.direction);
        let sidewards_raw = get_normal(nose_friend);
        let mut sidewards = (
            sidewards_raw.0 * combat::STANDARD_LINE_DISTANCE as f32,
            sidewards_raw.1 * combat::STANDARD_LINE_DISTANCE as f32,
        );
        nose_friend.1 *= ASPECT_RATIO;
        sidewards.1 *= ASPECT_RATIO;

        let new_pos = Position {
            x: left.position.x + sidewards.0,
            y: left.position.y + sidewards.1,
            ..left.position
        };
        self.propose_line_positions_there(
            list,
            new_pos,
            nose_friend,
            left_neighbour_handle,
            0,
            ctx,
            tick,
            grid,
        );
    }

    /// ProposeCombatPositionsBetween.
    fn propose_combat_positions_between(
        &self,
        list: &mut Vec<CombatPosition>,
        left_handle: HumanHandle,
        right_handle: HumanHandle,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) {
        let Some(left) = self.find_fighter(left_handle, tick) else {
            return;
        };
        let Some(right) = self.find_fighter(right_handle, tick) else {
            return;
        };
        let me_pos = ctx.position;
        if left.position.sector != me_pos.sector || right.position.sector != me_pos.sector {
            return;
        }

        let sidewards = pos_diff(&right.position, &left.position);
        let new_pos = Position {
            x: left.position.x + 0.5 * sidewards.0,
            y: left.position.y + 0.5 * sidewards.1,
            ..left.position
        };
        // GetNormal(false) — clockwise normal — for the facing.
        let direction = get_normal_right(sidewards);
        self.propose_line_positions_there(
            list,
            new_pos,
            direction,
            left_handle,
            right_handle,
            ctx,
            tick,
            grid,
        );
    }

    /// ProposeCombatPositionsAround — propose 16 positions ringed around
    /// `enemy_handle`, or, if the enemy is already targeting me, just
    /// one "change adversary" entry.
    fn propose_combat_positions_around(
        &self,
        list: &mut Vec<CombatPosition>,
        enemy_handle: HumanHandle,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) {
        let Some(enemy) = self.find_fighter(enemy_handle, tick) else {
            return;
        };
        let me_pos = ctx.position;
        let sword_distance = self.sword_range as f32;
        debug_assert!(sword_distance > combat::MIN_ENEMY_DIST as f32);

        let propose_change_direct;
        let propose_around;
        let forbidden_direction: u16;

        if enemy.has_as_opponent(self.base.me) {
            if enemy_handle != self.base.primary_target {
                propose_change_direct = true;
                propose_around = false;
                forbidden_direction = UNDEFINED_DIRECTION;
            } else {
                propose_change_direct = false;
                propose_around = self.my_line_jump.is_none();
                forbidden_direction = ctx.direction;
            }
        } else {
            propose_change_direct = false;
            propose_around = true;
            forbidden_direction = UNDEFINED_DIRECTION;
        }

        if propose_change_direct {
            let cp = CombatPosition {
                attacker: self.base.me,
                attacker_position: me_pos,
                target: enemy_handle,
                target_position: enemy.position,
                target_direction: enemy.direction,
                change_position: false,
                ..CombatPosition::default()
            };
            list.push(cp);
            return;
        }

        if !propose_around {
            return;
        }

        // Table-swordfight branch: when the primary target sits across a
        // jump line, replace the 16-direction surround with a single
        // position standing on the aggressor's side of the line, aimed
        // at the nearest point to the victim.
        let my_max_range = self
            .find_fighter(self.base.me, tick)
            .map(|f| f.sword_range_maximal)
            .unwrap_or(self.sword_range);
        // The "SCOTCHED" workaround gates this branch on
        // `my_line_jump.is_some()` to avoid a crash. Mirror it: only
        // enter the table-swordfight branch when the aggressor is already
        // standing on a line jump.
        if self.my_line_jump.is_some()
            && let Some(grid) = grid
            && let Some(aggressor_line_idx) = crate::engine::melee::table_swordfight_jump_line(
                grid,
                ctx.position.sector.map(i16::from).unwrap_or(-1),
                enemy.position.sector.map(i16::from).unwrap_or(-1),
                crate::coordinates::MapPoint::new(enemy.position.x, enemy.position.y),
                my_max_range as f32,
            )
            && let Some(aggressor_line) = grid.level.jump_lines.get(aggressor_line_idx as usize)
            && let Some(victim_line_idx) = aggressor_line.associated_line_index
            && let Some(victim_line) = grid.level.jump_lines.get(victim_line_idx as usize)
        {
            // Project victim onto its own line, then mirror that offset
            // back along the aggressor line (from B toward A).
            let t_victim = victim_line.compute_nearest_point_param(
                crate::coordinates::MapPoint::new(enemy.position.x, enemy.position.y),
            );
            let f_coeff = t_victim * victim_line.norm();
            let aggressor_norm = aggressor_line.norm().max(f32::EPSILON);
            let inv_norm = 1.0 / aggressor_norm;
            let aggressor_vec = aggressor_line.vector();
            let pt_on_line_x = aggressor_line.point_b.x - f_coeff * aggressor_vec.x * inv_norm;
            let pt_on_line_y = aggressor_line.point_b.y - f_coeff * aggressor_vec.y * inv_norm;

            let new_pos = Position {
                x: pt_on_line_x,
                y: pt_on_line_y,
                level: aggressor_line.layer,
                sector: aggressor_line
                    .sector_index
                    .and_then(|index| {
                        grid.level
                            .sectors
                            .get(usize::from(index))
                            .and_then(|sector| SectorHandle::new(u16::from(sector.sector_number)))
                            .map(|sector| sector.with_arena_index(index))
                    })
                    .or(ctx.position.sector),
            };
            let cp = CombatPosition {
                attacker: self.base.me,
                attacker_position: new_pos,
                target: enemy_handle,
                target_position: enemy.position,
                target_direction: enemy.direction,
                change_position: true,
                line_jump: Some(aggressor_line_idx),
                ..CombatPosition::default()
            };
            list.push(cp);
            return;
        }

        for direction_index in 0..16u16 {
            if direction_index == forbidden_direction {
                continue;
            }
            let mut vec_enemy = sector_to_vector(direction_index);
            vec_enemy.0 *= sword_distance;
            vec_enemy.1 *= sword_distance;
            vec_enemy.1 *= ASPECT_RATIO;

            let new_pos = Position {
                x: enemy.position.x - vec_enemy.0,
                y: enemy.position.y - vec_enemy.1,
                ..enemy.position
            };

            // Skip unreachable positions.
            if let Some(grid) = grid {
                let me_pt = crate::coordinates::MapPoint::new(me_pos.x, me_pos.y);
                let new_pt = crate::coordinates::MapPoint::new(new_pos.x, new_pos.y);
                if !grid.is_straight_movement_authorized(
                    me_pt,
                    new_pt,
                    new_pos.level,
                    &ctx.move_box,
                ) {
                    continue;
                }
            }

            let cp = CombatPosition {
                attacker: self.base.me,
                attacker_position: new_pos,
                target: enemy_handle,
                target_position: enemy.position,
                target_direction: enemy.direction,
                change_position: true,
                ..CombatPosition::default()
            };
            list.push(cp);
        }
    }

    // -----------------------------------------------------------------------
    // Phalanx / shield-bearer formation helpers
    // -----------------------------------------------------------------------

    /// GetNearestFreeShieldBearer. Scans the complete friendly-soldier
    /// registry for the nearest shield bearer already in (or heading into)
    /// a shield-bearer substate. Original does not apply IsAbleToFight here:
    /// an inactive or script-locked bearer remains a valid formation anchor.
    /// If the caller is a shield bearer any protecting shield bearer will do;
    /// if the caller is an archer we only accept shield bearers that don't yet
    /// have an archer behind them.
    pub(super) fn get_nearest_free_shield_bearer(
        &self,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> Option<HumanHandle> {
        let me_snap = self.find_fighter(self.base.me, tick)?;
        let i_am_shield_bearer = me_snap.is_shield_bearer;

        let shield_running = crate::ai::Substate::AttackingRunningToPhalanx as u32;
        let shield_phalanx = crate::ai::Substate::AttackingPhalanx as u32;
        let shield_protecting = crate::ai::Substate::AttackingProtectingWithShield as u32;

        let min_distance = archer::SHIELD_BEARER_MIN_DISTANCE as f32;
        let mut best: HumanHandle = 0;
        let mut best_distance = min_distance;

        for f in &tick.fighter_registry {
            if f.handle == self.base.me || !f.is_friendly || !f.is_shield_bearer {
                continue;
            }
            // If we're an archer, the shield bearer must not already
            // have someone hiding behind them.
            if !i_am_shield_bearer && f.archer_behind_me != 0 {
                // This shield bearer already has an archer — skip.
                continue;
            }

            if f.current_substate != shield_running
                && f.current_substate != shield_phalanx
                && f.current_substate != shield_protecting
            {
                continue;
            }
            // Original `MaxNormDistance(pSoldier)` subtracts the two raw
            // `RHElement::GetPosition()` values. It does not call AI
            // `Position()`, which may snap a door-passing bearer to the
            // committed gate endpoint. Keep that accessor distinction here;
            // slot construction below still intentionally uses the bearer's
            // AI-facing position/seek position.
            let dist = ai_max_norm_distance(
                &f.raw_position,
                f.elevation,
                &me_snap.raw_position,
                me_snap.elevation,
            ) as u16;
            if crate::ai_enemy::battle_decision_debug_enabled() {
                eprintln!(
                    "SHIELD_BEARER_CANDIDATE frame={} me={} candidate={} substate={} archer_behind={} dist={dist} min={min_distance}",
                    ctx.frame, self.base.me, f.handle, f.current_substate, f.archer_behind_me,
                );
            }
            if f32::from(dist) < best_distance {
                best_distance = f32::from(dist);
                best = f.handle;
            }
        }

        if crate::ai_enemy::battle_decision_debug_enabled() {
            let shield_bearers = tick
                .fighter_registry
                .iter()
                .filter(|f| f.is_shield_bearer)
                .map(|f| {
                    (
                        f.handle,
                        f.is_friendly,
                        f.current_substate,
                        f.archer_behind_me,
                    )
                })
                .collect::<Vec<_>>();
            eprintln!(
                "SHIELD_BEARER_RESULT frame={} me={} best={best} registry={} bearers={shield_bearers:?}",
                ctx.frame,
                self.base.me,
                tick.fighter_registry.len(),
            );
        }
        if best == 0 { None } else { Some(best) }
    }

    /// ChooseGoodShootingPoint. Searches archery sectors for one that
    /// contains the primary target
    /// and isn't full, then finds the nearest free shooting point and
    /// nearest entry point. Sets up `my_archery_*` fields for the path.
    /// Returns `true` if a good shooting point was found.
    pub(super) fn choose_good_shooting_point(
        &mut self,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        // (0) Clear current shooting point. SetMyShootingPoint(NULL) also
        // releases the prior point's owner.
        self.set_my_shooting_point(global, None);

        // The reference implicitly requires a non-null primary target —
        // it would crash otherwise. Rather than falling back to
        // ctx.position (which meaninglessly tests "a point inside my own
        // sector"), bail out cleanly so the caller treats this as "no
        // good shooting point".
        let Some(primary) = self.find_fighter(self.base.primary_target, tick) else {
            tracing::trace!(
                me = self.base.me,
                primary_target = self.base.primary_target,
                "choose_good_shooting_point: primary target not visible; bailing"
            );
            return false;
        };
        let primary_pos = primary.position;

        // (1) Search for an archery sector containing the enemy.
        // `RHSectorArchery::IsInside( posEnemy )` (RHsector.cpp:2330-2340)
        // rejects the sector when its own layer differs from
        // `posEnemy.uwLevel` — the layer travels with the enemy position the
        // caller passed in (`ChooseGoodShootingPoint( Position( mpPrimaryTarget ) )`,
        // RHartificialmalignity.cpp:7627), not with the archer.
        let mut found_sector: Option<usize> = None;
        for (i, sector) in global.archery_sectors.iter().enumerate() {
            if !sector.is_full() && sector.is_inside(&primary_pos, primary_pos.level) {
                found_sector = Some(i);
                break;
            }
        }
        let sector_idx = match found_sector {
            Some(i) => i,
            None => return false,
        };

        // (2) Find nearest entry point and nearest free shooting point
        let my_sector = ctx.position.sector;

        let mut nearest_entry: Option<(usize, f32)> = None; // (index, sq_dist)
        let mut nearest_shooting: Option<(usize, f32)> = None;

        let sector = &global.archery_sectors[sector_idx];
        let primary_handle = self.base.primary_target;
        for (i, pt) in sector.points.iter().enumerate() {
            // Probe each path point with the full
            // ArcherIsToNearToEnemy predicate (per-enemy, sector- and
            // action-state-dependent threshold) — if the path passes
            // dangerously close to the primary target, abandon the
            // whole search.
            if self.archer_is_too_near_to_enemy(&pt.position, primary_handle, ctx, tick) {
                return false;
            }

            let d_to_me = pos_diff(&pt.position, &ctx.position);
            let mut sq_dist = square_norm(d_to_me);
            // Penalty for sector changes.
            let pt_sector =
                crate::position_interface::SectorHandle::new(u16::from(pt.sector_index));
            if pt_sector != my_sector {
                sq_dist += 10000.0;
            }

            if !pt.is_shooting_point {
                if nearest_entry.is_none_or(|(_, best)| sq_dist < best) {
                    nearest_entry = Some((i, sq_dist));
                }
            } else if pt.owner.is_none() && nearest_shooting.is_none_or(|(_, best)| sq_dist < best)
            {
                nearest_shooting = Some((i, sq_dist));
            }
        }

        let (shooting_idx, _) = match nearest_shooting {
            Some(v) => v,
            None => return false, // no free shooting point
        };

        // (3) Set up archery path variables
        self.my_archery_sector_index = sector_idx as u16;
        // Fall back to the original sentinels when no shooting point
        // range was recorded, preserving the "always near head"
        // behavior in that degenerate case.
        let first_sp = sector
            .index_first_shooting_point
            .map_or(u16::MAX, u16::from);
        let last_sp = sector.index_last_shooting_point.map_or(0, u16::from);

        if let Some((entry_idx, _)) = nearest_entry {
            if (entry_idx as u16) < first_sp {
                // Near the head — run forward
                self.my_archery_point_index = crate::sector::ArcheryPointIdx(entry_idx as u16);
                self.my_archery_point_increment = 1;
            } else if (entry_idx as u16) > last_sp {
                // Near the tail — run backward
                self.my_archery_point_index = crate::sector::ArcheryPointIdx(entry_idx as u16);
                self.my_archery_point_increment = -1;
            } else {
                // Between head and tail — run directly toward shooting point
                if entry_idx < shooting_idx {
                    self.my_archery_point_index =
                        crate::sector::ArcheryPointIdx(shooting_idx.saturating_sub(1) as u16);
                    self.my_archery_point_increment = 1;
                } else {
                    self.my_archery_point_index = crate::sector::ArcheryPointIdx(
                        (shooting_idx + 1).min(sector.points.len() - 1) as u16,
                    );
                    self.my_archery_point_increment = -1;
                }
                // Already reserve this shooting point.
                self.set_my_shooting_point(global, Some((sector_idx as u16, shooting_idx as u16)));
            }
        } else {
            // No entry point — go directly to shooting point
            self.my_archery_point_index = crate::sector::ArcheryPointIdx(shooting_idx as u16);
            self.my_archery_point_increment = 1;
            self.set_my_shooting_point(global, Some((sector_idx as u16, shooting_idx as u16)));
        }

        self.set_my_archery_sector(global, Some(sector_idx as u16));
        true
    }

    /// SetMyShootingPoint. Three-step contract: (1) clear `owner` on the
    /// previously held
    /// shooting point, (2) overwrite `my_shooting_point`, (3) write
    /// `owner` on the new shooting point.  `new` is `(sector_idx,
    /// point_idx)` into `AiGlobalState::archery_sectors`.  The
    /// sector-level `num_owners` counter is independent and is managed
    /// by `set_my_archery_sector`.
    pub(super) fn set_my_shooting_point(
        &mut self,
        global: &mut AiGlobalState,
        new: Option<(u16, u16)>,
    ) {
        if let Some((old_sec, old_pt)) = self.my_shooting_point
            && let Some(sector) = global.archery_sectors.get_mut(old_sec as usize)
            && let Some(pt) = sector.points.get_mut(old_pt as usize)
        {
            pt.owner = None;
        }
        self.my_shooting_point = new;
        if let Some((new_sec, new_pt)) = new
            && let Some(sector) = global.archery_sectors.get_mut(new_sec as usize)
            && let Some(pt) = sector.points.get_mut(new_pt as usize)
        {
            pt.owner = Some(crate::entity_id::EntityId::Soldier(
                crate::entity_id::SoldierId(self.base.me),
            ));
        }
    }

    /// SetMyArcherySector. Updates `my_archery_sector` and keeps the
    /// owner counter on the
    /// old/new archery sector in sync. Counter drives `is_full`, which
    /// gates sector selection in `choose_good_shooting_point`.
    fn set_my_archery_sector(&mut self, global: &mut AiGlobalState, new_sector: Option<u16>) {
        if let Some(old) = self.my_archery_sector
            && let Some(sector) = global.archery_sectors.get_mut(old as usize)
        {
            sector.decrement_owner_counter();
        }
        self.my_archery_sector = new_sector;
        if let Some(new) = new_sector
            && let Some(sector) = global.archery_sectors.get_mut(new as usize)
        {
            sector.increment_owner_counter();
        }
    }

    /// ArcheryPathGetWaypoint. Pure read: returns the current waypoint
    /// on the archery path, or
    /// `None` if the cursor is past either end.  The caller is
    /// responsible for advancing via `archery_path_increment_waypoint`.
    pub(super) fn archery_path_get_waypoint(&self, global: &AiGlobalState) -> Option<PointArchery> {
        let sector = global
            .archery_sectors
            .get(self.my_archery_sector? as usize)?;
        let idx = usize::from(self.my_archery_point_index);
        sector.points.get(idx).cloned()
    }

    /// ArcheryPathIncrementWaypoint. One-liner:
    /// `my_archery_point_index += my_archery_point_increment;` with
    /// UWORD wrapping on overflow/underflow. After stepping off the end
    /// in either direction, the next `archery_path_get_waypoint` will
    /// see an out-of-bounds index and return `None`, matching the
    /// reference's null-sentinel check.
    pub(super) fn archery_path_increment_waypoint(&mut self) {
        let cur = u16::from(self.my_archery_point_index);
        let inc = i16::from(self.my_archery_point_increment);
        self.my_archery_point_index = crate::sector::ArcheryPointIdx(cur.wrapping_add_signed(inc));
    }

    /// UpdateShieldBearerBeforeMe. Updates the archer's own
    /// `shield_bearer_before_me` link and the shield bearer's reciprocal
    /// `archer_behind_me` link.
    pub(super) fn update_shield_bearer_before_me(&mut self, new_sb: HumanHandle) {
        if !self.is_archer() {
            return;
        }
        if new_sb == self.shield_bearer_before_me {
            return;
        }
        let old_sb = self.shield_bearer_before_me;
        if old_sb != 0 {
            self.base
                .outbox
                .reentrant
                .cross_npc_actions
                .push(CrossNpcAction::SetArcherBehindMe {
                    target: old_sb,
                    archer: 0,
                });
        }
        self.shield_bearer_before_me = new_sb;
        if new_sb != 0 {
            self.base
                .outbox
                .reentrant
                .cross_npc_actions
                .push(CrossNpcAction::SetArcherBehindMe {
                    target: new_sb,
                    archer: self.base.me,
                });
        }
    }

    /// Walk the cached neighbour chain from `start` via left/right links,
    /// returning the last fighter encountered on that side. Used by
    /// `find_phalanx_place` to locate the ends of an existing phalanx.
    /// Capped at 16 iterations so a corrupted chain can't spin forever.
    fn walk_phalanx_end(
        &self,
        start: HumanHandle,
        go_left: bool,
        _ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> HumanHandle {
        let mut current = start;
        // Cap at 16 to guard against a cyclic chain; a healthy phalanx
        // has at most a handful of links.
        for _ in 0..16 {
            let Some(snap) = self.find_fighter(current, tick) else {
                return current;
            };
            let next = if go_left {
                snap.left_combat_neighbour
            } else {
                snap.right_combat_neighbour
            };
            if next == 0 || next == current {
                return current;
            }
            current = next;
        }
        current
    }

    /// FindPhalanxPlace. Tries to find a free slot beside an existing
    /// shield-bearer phalanx — either just left of the leftmost member or
    /// just right of the rightmost — and returns the chosen position,
    /// facing, and the new neighbour pair.
    ///
    /// Returns `None` if we've already bailed on a phalanx in this fight
    /// (`phalanx_aborted`), or if no nearby shield bearer is available, or
    /// if neither side has room.
    ///
    /// Port of the legacy phalanx placement search.
    fn find_phalanx_place(
        &self,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> Option<(Position, u16, HumanHandle, HumanHandle, bool)> {
        if self.phalanx_aborted {
            return None;
        }
        let Some(nearest) = self.get_nearest_free_shield_bearer(ctx, tick) else {
            return None;
        };

        // Walk left/right to find the end-of-phalanx anchors.
        let left_guy = self.walk_phalanx_end(nearest, true, ctx, tick);
        let right_guy = self.walk_phalanx_end(nearest, false, ctx, tick);

        // Use GetShieldBearerPosition semantics: when the anchor is running
        // to a phalanx slot, use their future seek position + shield bearing
        // direction; when in position, use their current pose.
        let shield_running = Substate::AttackingRunningToPhalanx as u32;
        // Both straight-movement probes below are made on the left
        // anchor's own layer, taken from where it currently stands rather
        // than from the slot it is heading for.
        let left_layer = self.find_fighter(left_guy, tick)?.position.level;
        let (left_pos, left_dir) = {
            let snap = self.find_fighter(left_guy, tick)?;
            if snap.current_substate == shield_running {
                (
                    snap.shield_bearer_seek_position,
                    snap.shield_bearer_direction,
                )
            } else {
                (snap.position, snap.direction)
            }
        };
        let (right_pos, right_dir) = {
            let snap = self.find_fighter(right_guy, tick)?;
            if snap.current_substate == shield_running {
                (
                    snap.shield_bearer_seek_position,
                    snap.shield_bearer_direction,
                )
            } else {
                (snap.position, snap.direction)
            }
        };

        let distance = archer::DISTANCE_SHIELD_BEARER_SHIELD_BEARER as f32;

        // Left slot: anchor's forward vector, clockwise normal.
        let left_forward = sector_to_vector(left_dir);
        let mut left_side = get_normal_right(left_forward);
        left_side.0 *= distance;
        left_side.1 *= distance;
        left_side.1 *= ASPECT_RATIO;
        let pos_left = Position {
            x: left_pos.x + left_side.0,
            y: left_pos.y + left_side.1,
            ..left_pos
        };

        // Right slot: anchor's forward vector, counter-clockwise normal.
        let right_forward = sector_to_vector(right_dir);
        let mut right_side = get_normal(right_forward);
        right_side.0 *= distance;
        right_side.1 *= distance;
        right_side.1 *= ASPECT_RATIO;
        let pos_right = Position {
            x: right_pos.x + right_side.0,
            y: right_pos.y + right_side.1,
            ..right_pos
        };

        // Check each slot for straight-line reachability from the
        // anchor soldier.
        let left_accessible = grid.is_none_or(|g| {
            let anchor_pt = crate::coordinates::MapPoint::new(left_pos.x, left_pos.y);
            let slot_pt = crate::coordinates::MapPoint::new(pos_left.x, pos_left.y);
            g.is_straight_movement_authorized(anchor_pt, slot_pt, left_layer, &ctx.move_box)
        });
        // Both reachability probes are made on the *left* anchor's layer,
        // including the right-hand one. It reads like a slip, but the
        // formation geometry it produces is the behaviour being matched.
        let right_accessible = grid.is_none_or(|g| {
            let anchor_pt = crate::coordinates::MapPoint::new(right_pos.x, right_pos.y);
            let slot_pt = crate::coordinates::MapPoint::new(pos_right.x, pos_right.y);
            g.is_straight_movement_authorized(anchor_pt, slot_pt, left_layer, &ctx.move_box)
        });

        let me_pos = ctx.position;
        let sq_left = square_norm(pos_diff(&pos_left, &me_pos));
        let sq_right = square_norm(pos_diff(&pos_right, &me_pos));
        let left_crosses_identity = inherited_position_crosses_sector_identity(&me_pos, &left_pos);
        let right_crosses_identity =
            inherited_position_crosses_sector_identity(&me_pos, &right_pos);

        match (left_accessible, right_accessible) {
            (true, true) => {
                // Strict `<` — ties go to right slot.
                if sq_left < sq_right {
                    Some((pos_left, left_dir, 0, left_guy, left_crosses_identity))
                } else {
                    Some((pos_right, right_dir, right_guy, 0, right_crosses_identity))
                }
            }
            (true, false) => Some((pos_left, left_dir, 0, left_guy, left_crosses_identity)),
            (false, true) => Some((pos_right, right_dir, right_guy, 0, right_crosses_identity)),
            (false, false) => None,
        }
    }

    /// Calculate the ideal position behind a linked shield bearer without
    /// testing whether the archer can move there.
    ///
    /// Original's already-in-cover check performs only this position
    /// calculation. `ComputePositionBehindMyShieldBearer` adds the movement
    /// authorization check, but BattleDecisions calls that method only when
    /// the archer actually needs to reposition.
    pub(super) fn shield_bearer_cover_position(
        &self,
        shield_bearer: HumanHandle,
        tick: &AiPerTickData,
    ) -> Option<Position> {
        let snap = self.find_fighter(shield_bearer, tick)?;
        // Read the bearer's "shield bearer position" — when running to
        // a phalanx slot, that's the future seek pose; once in position,
        // the current pose.
        let shield_running = Substate::AttackingRunningToPhalanx as u32;
        let (bearer_pos, bearer_dir) = if snap.current_substate == shield_running {
            (
                snap.shield_bearer_seek_position,
                snap.shield_bearer_direction,
            )
        } else {
            (snap.position, snap.direction)
        };
        let forward = sector_to_vector(bearer_dir);
        let distance = archer::DISTANCE_SHIELD_BEARER_ARCHER as f32;
        // Original first authors the aspect-corrected vector through
        // `SetSector0to15(direction, ASPECT_RATIO)` and only then applies
        // `operator*=(DISTANCE_SHIELD_BEARER_ARCHER)`. Keep those two f32
        // roundings in that order: reassociating this as
        // `(forward.y * distance) * ASPECT_RATIO` changes the cover point by
        // one ULP for diagonal sectors.
        let vertical_offset = (forward.1 * ASPECT_RATIO) * distance;
        Some(Position {
            x: bearer_pos.x - forward.0 * distance,
            y: bearer_pos.y - vertical_offset,
            ..bearer_pos
        })
    }

    /// ComputePositionBehindMyShieldBearer. Given an archer caller with
    /// a linked shield bearer, compute the cover position
    /// `DISTANCE_SHIELD_BEARER_ARCHER` behind that shield bearer along
    /// their facing.
    ///
    /// Called from the `CoverBehindShieldBearer` decision after the
    /// unchecked already-in-cover calculation has established that the
    /// archer really needs to move.
    ///
    /// When the shield bearer is `AttackingRunningToPhalanx`, projects
    /// the cover point behind their *future* slot (seek position +
    /// shield-bearer direction) rather than their current pose, matching
    /// the GetShieldBearerPosition behavior. Returns `None` if the
    /// cover line crosses geometry (IsStraightMovementAutorized
    /// failure).
    pub fn compute_position_behind_shield_bearer(
        &self,
        shield_bearer: HumanHandle,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> Option<Position> {
        let snap = self.find_fighter(shield_bearer, tick)?;
        let bearer_pos = if snap.current_substate == Substate::AttackingRunningToPhalanx as u32 {
            snap.shield_bearer_seek_position
        } else {
            snap.position
        };
        let behind = self.shield_bearer_cover_position(shield_bearer, tick)?;
        // Cover line must be unobstructed from the bearer.
        if let Some(g) = grid {
            let bearer_pt = crate::coordinates::MapPoint::new(bearer_pos.x, bearer_pos.y);
            let cover_pt = crate::coordinates::MapPoint::new(behind.x, behind.y);
            let ok =
                g.is_straight_movement_authorized(bearer_pt, cover_pt, behind.level, &ctx.move_box);
            if crate::ai_enemy::battle_decision_debug_enabled() {
                eprintln!(
                    "COVER_POS frame={} me={} bearer={} sub={} bearer_pos={:?} bearer_dir={} bearer_raw={:?} behind={:?} straight_ok={ok}",
                    ctx.frame,
                    self.base.me,
                    shield_bearer,
                    snap.current_substate,
                    bearer_pos,
                    snap.shield_bearer_direction,
                    snap.position,
                    behind,
                );
            }
            if !ok {
                return None;
            }
        }
        Some(behind)
    }

    // -----------------------------------------------------------------------
    // Phalanx substate helpers
    // -----------------------------------------------------------------------

    /// NumberOfNearbyArchersWhoNeedProtection. Scans nearby friendly
    /// soldiers for archers without a shield bearer and shield bearers
    /// without an archer, returning the net count. Positive = archers
    /// that need protection.
    fn number_of_nearby_archers_who_need_protection(
        &self,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> i32 {
        let consider_sq = (archer::SHIELD_BEARER_MIN_DISTANCE as f32)
            * (archer::SHIELD_BEARER_MIN_DISTANCE as f32);

        let shield_running = Substate::AttackingRunningToPhalanx as u32;
        let shield_phalanx = Substate::AttackingPhalanx as u32;
        let shield_protecting = Substate::AttackingProtectingWithShield as u32;
        let shield_advancing = Substate::AttackingAdvancingWithShield as u32;

        let mut count: i32 = 0;
        let debug = arrow_protection_debug_matches(|| ctx.frame, || self.base.me);
        // `SquareDistance` compares `pSoldier->GetPosition()` against
        // `mpMe->GetPosition()` (`RHartificialintelligence.cpp:6919-6922`),
        // i.e. both raw element positions. `AiContext::position` and
        // `FighterSnapshot::position` carry the AI `Position()` result
        // instead, which snaps an actor in door transit onto the gate
        // endpoint; measuring from there moved a door-passing orphan archer
        // out of the 500-unit radius and silenced the shield-bearer
        // reaction.
        let me_raw = self
            .find_fighter(self.base.me, tick)
            .unwrap_or_else(|| {
                panic!(
                    "NumberOfNearbyArchersWhoNeedProtection owner {} is absent from its own fighter snapshots",
                    self.base.me
                )
            });
        let (me_position, me_elevation) = (me_raw.raw_position, me_raw.elevation as f32);
        // Original walks the complete camp soldier registry. In particular it
        // does not apply IsAbleToFight/Active before counting an orphan
        // archer: an inactive Seeking soldier still owns its AI state and
        // shield-bearer link and therefore remains a protection candidate.
        // `nearby_fighters` applies that unrelated able-to-fight filter;
        // `fighter_registry` preserves the complete Original scan domain and
        // ordering, while the distance/state gates below do the actual
        // admission work.
        for f in &tick.fighter_registry {
            if !f.is_friendly {
                if debug {
                    eprintln!(
                        "ARCHER_PROTECTION_SCAN frame={} owner={} cand={} skip=not_friendly",
                        ctx.frame, self.base.me, f.handle
                    );
                }
                continue;
            }
            let sq = ai_square_distance(
                &f.raw_position,
                f.elevation as f32,
                &me_position,
                me_elevation,
            );
            if debug {
                eprintln!(
                    "ARCHER_PROTECTION_SCAN frame={} owner={} cand={} sq={} state={:?} sub={} archer={} tower={} sbb={} own_abm={} abm={} shield={} pos={:?} elev={}",
                    ctx.frame,
                    self.base.me,
                    f.handle,
                    sq,
                    f.ai_state,
                    f.current_substate,
                    f.is_archer_unit,
                    f.is_tower_guard,
                    f.shield_bearer_before_me,
                    self.archer_behind_me,
                    f.archer_behind_me,
                    f.is_shield_bearer,
                    f.position,
                    f.elevation
                );
            }
            if sq >= consider_sq {
                continue;
            }
            // Cross-NPC relationship writes are drained after the current
            // Think call, but Original's UpdateArcherBehindMe /
            // UpdateShieldBearerBeforeMe update both objects synchronously.
            // Overlay the ordered writes already emitted by this Think so a
            // later tactical scan observes the same reciprocal relationship.
            let effective_shield_bearer_before_me = self
                .base
                .outbox
                .reentrant
                .cross_npc_actions
                .iter()
                .rev()
                .find_map(|action| match action {
                    CrossNpcAction::SetShieldBearerBeforeMe {
                        target,
                        shield_bearer,
                    } if *target == f.handle => Some(*shield_bearer),
                    _ => None,
                })
                .unwrap_or(f.shield_bearer_before_me);
            let effective_archer_behind_me = self
                .base
                .outbox
                .reentrant
                .cross_npc_actions
                .iter()
                .rev()
                .find_map(|action| match action {
                    CrossNpcAction::SetArcherBehindMe { target, archer } if *target == f.handle => {
                        Some(*archer)
                    }
                    _ => None,
                })
                .unwrap_or(f.archer_behind_me);
            // Filter on AI state ∈ {Seeking, Wondering, Attacking}.
            match f.ai_state {
                AiState::Seeking | AiState::Wondering | AiState::Attacking => {
                    if f.is_archer_unit
                        && effective_shield_bearer_before_me == 0
                        && !f.is_tower_guard
                    {
                        // Orphan archer. No `soldier != me` guard here —
                        // an orphan-archer self counts itself.
                        count += 1;
                    } else if f.is_shield_bearer
                        && effective_archer_behind_me == 0
                        && f.handle != self.base.me
                    {
                        // Explicitly exclude self in the shield-bearer
                        // branch so a shield bearer can't become its own
                        // protector.
                        match f.current_substate {
                            s if s == shield_phalanx
                                || s == shield_running
                                || s == shield_protecting
                                || s == shield_advancing =>
                            {
                                count -= 1;
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        count
    }

    /// PhalanxIsEncercledByEnemies. Check whether enemies are attacking
    /// from behind the phalanx (direction difference > ±3 sectors from
    /// the intended facing).
    fn phalanx_is_encircled_by_enemies(
        &self,
        center: &Position,
        intended_direction: u16,
        tick: &AiPerTickData,
    ) -> bool {
        for &enemy_handle in &self.list_them {
            let Some(enemy) = self.find_fighter(enemy_handle, tick) else {
                continue;
            };
            let dx = enemy.position.x - center.x;
            let dy = enemy.position.y - center.y;
            let enemy_sector = vec_to_sector(dx, dy);
            let diff = (intended_direction.wrapping_sub(enemy_sector)) & 15;
            match diff {
                0 | 1 | 2 | 3 | 13 | 14 | 15 => {
                    // Within front tolerance
                }
                _ => return true,
            }
        }
        false
    }

    /// PhalanxIsProtectingArchers. Walk the right-neighbour chain
    /// checking if any member has an archer hiding behind them.
    fn phalanx_is_protecting_archers(&self, tick: &AiPerTickData) -> bool {
        // Check self first
        if self.archer_behind_me != 0 {
            return true;
        }
        // Walk right chain via snapshots
        let mut current = self.right_combat_neighbour;
        for _ in 0..16 {
            if current == 0 {
                return false;
            }
            let Some(snap) = self.find_fighter(current, tick) else {
                return false;
            };
            if snap.archer_behind_me != 0 {
                return true;
            }
            let next = snap.right_combat_neighbour;
            if next == 0 || next == current {
                return false;
            }
            current = next;
        }
        false
    }

    /// BreakPhalanx. Propagate break through the neighbour chain, clear
    /// our own links, set `phalanx_aborted`, and fall back to
    /// `BattleDecisions`.
    ///
    /// Since we can't modify other NPCs directly, we emit
    /// `CrossNpcAction::BreakPhalanx` for each neighbour. The engine
    /// processes these after our think() returns.
    fn break_phalanx(
        &mut self,
        _sim: &crate::sim_rng::SimulationContext,
        _global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        _grid: Option<&crate::fast_find_grid::FastFindGrid>,
        carried_them_list: Option<&[HumanHandle]>,
    ) {
        // Original recursively descends all the way left and runs each
        // member's BattleDecisions while unwinding, then does the same on the
        // right, and only finally handles the initiating member. Queue that
        // exact depth-first tail order; the engine drains these actions before
        // the current owner boundary closes.
        let mut left = Vec::new();
        if self.left_combat_neighbour != 0 {
            let mut current = self.left_combat_neighbour;
            for _ in 0..16 {
                if current == 0 {
                    break;
                }
                left.push(current);
                let Some(snap) = self.find_fighter(current, tick) else {
                    break;
                };
                let next = snap.left_combat_neighbour;
                if next == 0 || next == current {
                    break;
                }
                current = next;
            }
        }
        let has_left_neighbour = !left.is_empty();
        for (index, target) in left.into_iter().rev().enumerate() {
            self.base
                .outbox
                .reentrant
                .cross_npc_actions
                .push(CrossNpcAction::BreakPhalanx {
                    target,
                    refresh_them_list: index == 0,
                });
        }
        if !has_left_neighbour {
            // We are already the leftmost member. Original refreshes the
            // shared list before recursing to the right.
            self.phalanx_reinitialize_them_list(&ctx.position, ctx, tick, carried_them_list);
        }

        let mut right = Vec::new();
        if self.right_combat_neighbour != 0 {
            let mut current = self.right_combat_neighbour;
            for _ in 0..16 {
                if current == 0 {
                    break;
                }
                right.push(current);
                let Some(snap) = self.find_fighter(current, tick) else {
                    break;
                };
                let next = snap.right_combat_neighbour;
                if next == 0 || next == current {
                    break;
                }
                current = next;
            }
        }
        for target in right.into_iter().rev() {
            self.base
                .outbox
                .reentrant
                .cross_npc_actions
                .push(CrossNpcAction::BreakPhalanx {
                    target,
                    refresh_them_list: false,
                });
        }
        self.base
            .outbox
            .reentrant
            .cross_npc_actions
            .push(CrossNpcAction::BreakPhalanx {
                target: self.base.me,
                refresh_them_list: false,
            });
    }

    /// Apply the local half of a neighbouring member's recursive
    /// `BreakPhalanx` call.
    ///
    /// Original `RHArtificialMalignity::BreakPhalanx` walks the neighbour
    /// chain recursively. Every visited member clears both links, marks the
    /// formation abandoned, and calls `BattleDecisions` before returning.
    /// The originating Rust member has already enumerated the chain into
    /// `CrossNpcAction::BreakPhalanx`, so this entry point must not propagate
    /// again; it performs exactly the per-member tail of that recursion.
    pub(crate) fn break_phalanx_from_neighbour(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
        refresh_them_list: bool,
    ) {
        if refresh_them_list {
            self.phalanx_reinitialize_them_list(&ctx.position, ctx, tick, None);
        }
        self.clear_combat_neighbours();
        self.phalanx_aborted = true;
        self.battle_decisions(sim, global, ctx, tick, grid);
    }

    /// PhalanxReinitializeThemList. Rebuild the shared enemy list for
    /// the whole phalanx. The reference is recursive through the
    /// right-neighbour chain; here we build from our own detectable
    /// enemies plus those visible to right neighbours via snapshots,
    /// then set primary_target to the nearest enemy.
    ///
    /// `carried` is the list a previous rebuild in this same think already
    /// installed on every member. The member snapshots are taken once at
    /// the start of the think, so without it a second rebuild would clean
    /// each member's pre-rebuild list instead of the shared one, and would
    /// re-derive a list the formation has already moved past.
    fn phalanx_reinitialize_them_list(
        &mut self,
        phalanx_left_pos: &Position,
        ctx: &AiContext,
        tick: &AiPerTickData,
        carried: Option<&[HumanHandle]>,
    ) -> Vec<HumanHandle> {
        // (1..3) Replay the original recursion over the up-front member
        // snapshots. Each member cleans its persistent list with its own
        // current 360° radius+LOS, then scans its live detectable-enemy
        // list with its own current 180° radius+LOS.
        let mut snapshots: std::collections::HashMap<HumanHandle, &PhalanxEnemySnapshot> =
            std::collections::HashMap::new();
        for member in &tick.phalanx_member_them_lists {
            for target in member
                .current_them_list
                .iter()
                .chain(member.detectable_enemies.iter())
            {
                snapshots.entry(target.handle).or_insert(target);
            }
        }

        self.list_them.clear();
        for member in &tick.phalanx_member_them_lists {
            let kept: Vec<&PhalanxEnemySnapshot> = match carried {
                Some(handles) => handles
                    .iter()
                    .filter_map(|handle| snapshots.get(handle).copied())
                    .collect(),
                None => member.current_them_list.iter().collect(),
            };
            append_phalanx_member_enemies(&mut self.list_them, member, &kept, ctx);
        }

        // (4) Find nearest enemy to phalanx center and make it primary
        if !self.list_them.is_empty() {
            // Phalanx center = midpoint between left end and rightmost member.
            // Walk the right-chain to find the rightmost position.
            let rightmost_pos = {
                let mut pos = ctx.position;
                let mut cur = self.right_combat_neighbour;
                for _ in 0..16 {
                    if cur == 0 {
                        break;
                    }
                    if let Some(snap) = self.find_fighter(cur, tick) {
                        pos = snap.position;
                        let next = snap.right_combat_neighbour;
                        if next == 0 || next == cur {
                            break;
                        }
                        cur = next;
                    } else {
                        break;
                    }
                }
                pos
            };
            let center_x = phalanx_left_pos.x + 0.5 * (rightmost_pos.x - phalanx_left_pos.x);
            let center_y = phalanx_left_pos.y + 0.5 * (rightmost_pos.y - phalanx_left_pos.y);

            let nearest_index = nearest_phalanx_enemy_index(
                self.list_them.iter().enumerate().filter_map(|(index, &h)| {
                    self.find_fighter(h, tick).map(|snap| {
                        let dx = snap.position.x - center_x;
                        let dy = (snap.position.y - center_y)
                            * crate::position_interface::INVERSE_ASPECT_RATIO;
                        // MaxNorm(ASPECT_RATIO) — Chebyshev (L∞) — not
                        // Euclidean. Original then narrows that norm to UWORD
                        // before comparing candidates.
                        (index, dx.abs().max(dy.abs()))
                    })
                }),
            )
            .expect("phalanx Them-list entries must have fighter snapshots");
            let best_handle = self.list_them[nearest_index];
            // Swap nearest to front
            self.list_them.swap(0, nearest_index);
            self.base.primary_target = best_handle;
        } else {
            self.base.primary_target = 0;
        }

        // Steps (5) and (6) run once per member as the recursion unwinds,
        // so the completed shared list and its head become every member's
        // them-list and primary target — not only the caller's.
        for member in &tick.phalanx_member_them_lists {
            if member.handle == self.base.me {
                continue;
            }
            self.base
                .outbox
                .reentrant
                .cross_npc_actions
                .push(CrossNpcAction::SetPhalanxThemList {
                    target: member.handle,
                    them: self.list_them.clone(),
                    primary_target: self.base.primary_target,
                });
        }

        self.list_them.clone()
    }

    /// ReconsiderPhalanx. Called by the leftmost phalanx member on timer
    /// to re-evaluate formation: pivot when enemies attack from the
    /// side, advance when enemies are dead-ahead, break when encircled.
    /// Returns `true` if the substate was changed.
    pub(super) fn reconsider_phalanx(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        tracing::trace!(
            target: "robin_engine::ai_enemy::phalanx",
            me = self.base.me,
            frame = ctx.frame,
            substate = ?self.base.current_substate,
            left = self.left_combat_neighbour,
            right = self.right_combat_neighbour,
            "reconsider_phalanx: enter"
        );
        self.base.clear_emoticon();

        // Check PHALANX_ATTACK_DISTANCE gate
        let nearest = self.get_new_primary_target(PrimaryTargetFlags::empty(), ctx, tick);
        if nearest != 0
            && let Some(snap) = self.find_fighter(nearest, tick)
        {
            let atk_dist = archer::PHALANX_ATTACK_DISTANCE as f32;
            let sq = ai_square_distance(
                &snap.position,
                snap.elevation as f32,
                &ctx.position,
                ctx.elevation,
            );
            tracing::trace!(
                target: "robin_engine::ai_enemy::phalanx",
                me = self.base.me,
                frame = ctx.frame,
                nearest,
                square_distance = sq,
                threshold = atk_dist * atk_dist,
                "reconsider_phalanx: attack-distance gate"
            );
            if crate::ai_enemy::battle_decision_debug_enabled() {
                eprintln!(
                    "RECONSIDER_PHALANX frame={} me={} nearest={} sq={} thr={} left={} right={}",
                    ctx.frame,
                    self.base.me,
                    nearest,
                    sq,
                    atk_dist * atk_dist,
                    self.left_combat_neighbour,
                    self.right_combat_neighbour
                );
            }
            if sq < atk_dist * atk_dist {
                self.break_phalanx(sim, global, ctx, tick, grid, None);
                return true;
            }
        }

        // Only the leftmost guy has the right to modify the phalanx
        if self.left_combat_neighbour != 0 {
            return false;
        }

        // Reinitialize them lists
        tracing::trace!(
            target: "robin_engine::ai_enemy::phalanx",
            me = self.base.me,
            frame = ctx.frame,
            "reconsider_phalanx: reinitializing them lists"
        );
        let merged_them_list = self.phalanx_reinitialize_them_list(&ctx.position, ctx, tick, None);

        if self.list_them.is_empty() {
            // Pass no flags (uwFlags = 0, default per
            // ), so the FAST_OVERVIEW branch
            // that pulls in FillListWithAllNearFighters is deliberately
            // skipped.
            self.get_battle_overview(0, ctx, tick);
            return true;
        }

        // Original explicitly refreshes the retained shield box after
        // recalculating the phalanx formation.
        self.base.outbox.actor.refresh_shield = true;

        // Build phalanx member list by walking right chain. The per-guy
        // loop starts at `pGuy = me`, so the substate-check /
        // SetPrimaryTarget side-effect apply to self too. If we
        // ourselves aren't still in AttackingPhalanx, bail like the
        // reference does for any non-positioned member.
        if self.base.current_substate != Substate::AttackingPhalanx {
            return false;
        }
        let mut phalanx_members: Vec<HumanHandle> = Vec::new();
        phalanx_members.push(self.base.me);
        let mut current = self.right_combat_neighbour;
        for _ in 0..16 {
            if current == 0 {
                break;
            }
            let Some(snap) = self.find_fighter(current, tick) else {
                break;
            };
            // Propagate primary_target to all phalanx members.
            self.base
                .outbox
                .reentrant
                .cross_npc_actions
                .push(CrossNpcAction::SetPrimaryTarget {
                    target: current,
                    primary_target: self.base.primary_target,
                });
            // If any member isn't yet in position, don't reconsider
            if snap.current_substate != Substate::AttackingPhalanx as u32 {
                return false;
            }
            phalanx_members.push(current);
            let next = snap.right_combat_neighbour;
            if next == 0 || next == current {
                break;
            }
            current = next;
        }

        let phalanx_size = phalanx_members.len();
        if phalanx_size <= 1 {
            return false;
        }

        // Compute ideal and real direction
        let last_guy = *phalanx_members.last().unwrap();
        let last_pos = self
            .find_fighter(last_guy, tick)
            .map(|f| f.position)
            .unwrap_or(ctx.position);
        // Use the middle member's actual position for the center.
        let mid_idx = phalanx_size / 2;
        let phalanx_center = self
            .find_fighter(phalanx_members[mid_idx], tick)
            .map(|f| f.position)
            .unwrap_or(ctx.position);

        let primary_pos = self
            .find_fighter(self.base.primary_target, tick)
            .map(|f| f.position)
            .unwrap_or(ctx.position);
        let ideal_direction = {
            let dx = primary_pos.x - phalanx_center.x;
            let dy = primary_pos.y - phalanx_center.y;
            vec_to_sector(dx, dy)
        };
        let real_direction = {
            // Direction perpendicular to phalanx line (left→right +
            // 4 sectors).
            let dx = ctx.position.x - last_pos.x;
            let dy = ctx.position.y - last_pos.y;
            (vec_to_sector(dx, dy) + 4) & 15
        };

        let dir_diff = (ideal_direction.wrapping_sub(real_direction)) & 15;

        tracing::trace!(
            target: "robin_engine::ai_enemy::phalanx",
            me = self.base.me,
            frame = ctx.frame,
            members = ?phalanx_members,
            them = ?self.list_them,
            primary = self.base.primary_target,
            ideal_direction,
            real_direction,
            dir_diff,
            "reconsider_phalanx: geometry"
        );

        let (enemies_in_front, enemy_on_right_side) = match dir_diff {
            0 | 1 | 15 => {
                // Within tolerance
                if self.phalanx_is_encircled_by_enemies(&phalanx_center, ideal_direction, tick) {
                    tracing::trace!(
                        target: "robin_engine::ai_enemy::phalanx",
                        me = self.base.me,
                        frame = ctx.frame,
                        "reconsider_phalanx: encircled, breaking phalanx"
                    );
                    self.break_phalanx(sim, global, ctx, tick, grid, Some(&merged_them_list));
                    return true;
                }
                (true, false) // unused when enemies_in_front
            }
            2..=8 => (false, true),
            _ => (false, false),
        };

        let distance_sb = archer::DISTANCE_SHIELD_BEARER_SHIELD_BEARER as f32;

        if enemies_in_front {
            // Try to advance with the whole phalanx
            if !self.phalanx_is_protecting_archers(tick)
                && (crate::sim_rng::u32(sim, crate::sim_rng::RngSite::PhalanxAdvance, 0..3) == 0)
            {
                let half = phalanx_size / 2;

                // Original normalizes both the forward vector and its
                // `GetNormal(true, ASPECT_RATIO)` result in isometric space.
                let (fwd_step, right_scaled) =
                    phalanx_advance_vectors(pos_diff(&primary_pos, &phalanx_center));

                let new_center = Position {
                    x: phalanx_center.x + fwd_step.0,
                    y: phalanx_center.y + fwd_step.1,
                    ..phalanx_center
                };
                let new_left = Position {
                    x: new_center.x - half as f32 * right_scaled.0,
                    y: new_center.y - half as f32 * right_scaled.1,
                    ..new_center
                };
                let new_right = Position {
                    x: new_left.x + (phalanx_size - 1) as f32 * right_scaled.0,
                    y: new_left.y + (phalanx_size - 1) as f32 * right_scaled.1,
                    ..new_left
                };

                // Check that the new phalanx line is free of obstacles
                // AND at least one path from old to new is clear.
                let reachable = if let Some(grid) = grid {
                    let nl = crate::coordinates::MapPoint::new(new_left.x, new_left.y);
                    let nr = crate::coordinates::MapPoint::new(new_right.x, new_right.y);
                    let nc = crate::coordinates::MapPoint::new(new_center.x, new_center.y);
                    let ol = crate::coordinates::MapPoint::new(ctx.position.x, ctx.position.y);
                    let or_ = crate::coordinates::MapPoint::new(last_pos.x, last_pos.y);
                    let oc = crate::coordinates::MapPoint::new(phalanx_center.x, phalanx_center.y);
                    let lvl = new_left.level;
                    let mb = &ctx.move_box;
                    grid.is_straight_movement_authorized(nl, nr, lvl, mb)
                        && (grid.is_straight_movement_authorized(ol, nl, lvl, mb)
                            || grid.is_straight_movement_authorized(or_, nr, lvl, mb)
                            || grid.is_straight_movement_authorized(oc, nc, lvl, mb))
                } else {
                    true
                };

                if !reachable {
                    // Can't advance — do nothing this tick
                    return false;
                }

                // Every phalanx member (including self) gets
                // InstructGatherPosition + Think(CALL_INSTRUCTION). We
                // route self through the same
                // `CrossNpcAction::InstructGatherPosition` queue used
                // for peers so the engine's CallInstruction dispatch
                // (which invokes the AttackingPhalanx CallInstruction
                // handler — including the archer-behind-me notify) runs
                // uniformly for all members.
                for (i, &guy) in phalanx_members.iter().enumerate() {
                    let new_pos = Position {
                        x: new_left.x + i as f32 * right_scaled.0,
                        y: new_left.y + i as f32 * right_scaled.1,
                        ..new_left
                    };
                    self.base.outbox.reentrant.cross_npc_actions.push(
                        CrossNpcAction::InstructGatherPosition {
                            target: guy,
                            position: new_pos,
                            direction: ideal_direction,
                            call_instruction: true,
                        },
                    );
                }
                return true;
            }
            // Nothing instructed
            false
        } else {
            // Must realign: pivot the phalanx to face the new direction

            // Compute right vector for the ideal direction
            let ideal_right_sector = (ideal_direction + 4) & 15;
            let right_vec = sector_to_vector(ideal_right_sector);
            let right_scaled = (
                right_vec.0 * distance_sb,
                right_vec.1 * distance_sb * ASPECT_RATIO,
            );

            // Try all phalanx members as pivot points, starting from the
            // side nearest the enemy. Accept the first whose resulting
            // phalanx line passes the reachability check.
            let mut found_pivot = false;
            let mut new_left = Position::default();
            for i in 0..phalanx_size {
                let k = if enemy_on_right_side {
                    phalanx_size - 1 - i
                } else {
                    i
                };
                let pivot_pos = self
                    .find_fighter(phalanx_members[k], tick)
                    .map(|f| f.position)
                    .unwrap_or(ctx.position);
                let candidate_left = Position {
                    x: pivot_pos.x - k as f32 * right_scaled.0,
                    y: pivot_pos.y - k as f32 * right_scaled.1,
                    ..pivot_pos
                };
                let candidate_right = Position {
                    x: candidate_left.x + (phalanx_size - 1) as f32 * right_scaled.0,
                    y: candidate_left.y + (phalanx_size - 1) as f32 * right_scaled.1,
                    ..candidate_left
                };

                // Check if the new phalanx line is free of obstacles
                if let Some(grid) = grid {
                    let left_pt =
                        crate::coordinates::MapPoint::new(candidate_left.x, candidate_left.y);
                    let right_pt =
                        crate::coordinates::MapPoint::new(candidate_right.x, candidate_right.y);
                    if grid.is_straight_movement_authorized(
                        left_pt,
                        right_pt,
                        candidate_left.level,
                        &ctx.move_box,
                    ) {
                        new_left = candidate_left;
                        found_pivot = true;
                        break;
                    }
                } else {
                    // No grid available — accept the first candidate
                    new_left = candidate_left;
                    found_pivot = true;
                    break;
                }
            }

            if !found_pivot {
                // Not enough space to hold the phalanx — break formation
                self.break_phalanx(sim, global, ctx, tick, grid, Some(&merged_them_list));
                return true;
            }

            // Instruct all guys (self included) via the same uniform
            // path. See the equivalent comment in the advance branch
            // above.
            for (i, &guy) in phalanx_members.iter().enumerate() {
                let new_pos = Position {
                    x: new_left.x + i as f32 * right_scaled.0,
                    y: new_left.y + i as f32 * right_scaled.1,
                    ..new_left
                };
                self.base.outbox.reentrant.cross_npc_actions.push(
                    CrossNpcAction::InstructGatherPosition {
                        target: guy,
                        position: new_pos,
                        direction: ideal_direction,
                        call_instruction: true,
                    },
                );
            }
            true
        }
    }

    /// RefreshArrowProtection / ConsiderShieldBearerAttack. The decision
    /// that wakes up a shield bearer when a dangerous archer or
    /// unprotected friendly archer is nearby. Focuses the threat and
    /// either runs to a phalanx slot or raises shield in place.
    ///
    /// Returns `true` if a shield-bearing action was taken.
    pub fn refresh_arrow_protection(
        &mut self,
        called_from_hourglass: bool,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        let debug = arrow_protection_debug_matches(|| ctx.frame, || self.base.me);
        // Check if we're in the right substate
        match self.base.current_substate {
            Substate::AttackingReactiontimeTurning
            | Substate::AttackingReactiontime
            | Substate::AttackingReactiontimeRunning
            | Substate::AttackingRunningToEnemy
            | Substate::AttackingWalkingToEnemy
            | Substate::AttackingChargingEnemy
            | Substate::AttackingOverviewLookLeft
            | Substate::AttackingOverviewLookRight
            | Substate::AttackingReserve
            | Substate::AttackingLastReserve
            | Substate::AttackingReserveOverview
            | Substate::AttackingApproachToObserve
            | Substate::AttackingObserve
            | Substate::AttackingObserveAndMove
            | Substate::AttackingTooProudToAttack => {
                // OK, can pass
            }
            Substate::AttackingAdvancingWithShield => {
                if called_from_hourglass {
                    if debug {
                        eprintln!(
                            "ARROW_PROTECTION frame={} owner={} result=reject reason=advancing_hourglass substate={:?}",
                            ctx.frame, self.base.me, self.base.current_substate
                        );
                    }
                    return false;
                }
            }
            _ => {
                if debug {
                    eprintln!(
                        "ARROW_PROTECTION frame={} owner={} result=reject reason=substate substate={:?} from_hourglass={}",
                        ctx.frame, self.base.me, self.base.current_substate, called_from_hourglass
                    );
                }
                return false;
            }
        }

        // Shield bearers only
        let me_snap = self.find_fighter(self.base.me, tick);
        if !me_snap.map(|f| f.is_shield_bearer).unwrap_or(false) {
            if debug {
                eprintln!(
                    "ARROW_PROTECTION frame={} owner={} result=reject reason=shield_bearer owner_present={} is_shield_bearer={:?}",
                    ctx.frame,
                    self.base.me,
                    me_snap.is_some(),
                    me_snap.map(|fighter| fighter.is_shield_bearer)
                );
            }
            return false;
        }
        tracing::trace!(
            target: "robin_engine::ai_enemy::shield",
            frame = ctx.frame,
            me = self.base.me,
            substate = ?self.base.current_substate,
            from_hourglass = called_from_hourglass,
            "RefreshArrowProtection: shield bearer passed substate gate"
        );

        // Get nearest enemy
        let nearest_enemy =
            self.get_new_primary_target(PrimaryTargetFlags::VIPS_ALLOWED, ctx, tick);
        if nearest_enemy == 0 {
            if debug {
                eprintln!(
                    "ARROW_PROTECTION frame={} owner={} result=reject reason=no_nearest_enemy list_them={:?}",
                    ctx.frame, self.base.me, self.list_them
                );
            }
            return false;
        }

        // Both range gates below call `SquareDistance(element)`. That helper
        // reads each actor's literal `RHElement::GetPosition()`, not the
        // door-aware AI `Position()` stored in fighter snapshots.
        let square_distance_to = |handle: HumanHandle| -> f32 {
            let target = ctx
                .expect_entity_view(handle, "RefreshArrowProtection SquareDistance target")
                .detection_position_world;
            ai_square_distance_world(&target, &ctx.self_body_position_world)
        };

        // Are we already near enough to fight? (PHALANX_ATTACK_DISTANCE gate)
        let atk_dist = archer::PHALANX_ATTACK_DISTANCE as f32;
        let enemy_square_distance = square_distance_to(nearest_enemy);
        if enemy_square_distance < atk_dist * atk_dist {
            if debug {
                eprintln!(
                    "ARROW_PROTECTION frame={} owner={} result=reject reason=enemy_near nearest={} square_distance_bits={} threshold={}",
                    ctx.frame,
                    self.base.me,
                    nearest_enemy,
                    enemy_square_distance.to_bits(),
                    atk_dist * atk_dist
                );
            }
            return false;
        }

        // Scan all visible enemies for a dangerous one (using a bow).
        //
        // This walks the soldier's own enemy detectable list in list
        // order, gated on the `seen_last_frame` flag: only enemies the
        // soldier currently sees (or saw last frame) count as
        // dangerous, so a bow-armed enemy who is occluded or has
        // slipped out of the cone of vision can't trip a phalanx /
        // shield-raise. `tick.seen_last_frame_enemies` mirrors that
        // flag from this NPC's own detectable list.
        //
        // Candidates are resolved through the unfiltered entity-view
        // map rather than `nearby_fighters`: the latter is a
        // swordfight-oriented snapshot capped at a 500px radius that
        // also drops anyone who is not `is_able_to_fight`. Both
        // exclusions are wrong here — an archer must be at least
        // MIN_PROTECT_ARROW_DISTANCE away to qualify, and a PC
        // shooting from a tree is exactly the threat this reaction
        // exists to answer even though such a posture reads as
        // "unable to fight".
        let mut dangerous_enemy: HumanHandle = 0;
        if debug {
            eprintln!(
                "ARROW_PROTECTION_SEEN frame={} owner={} seen={:?}",
                ctx.frame, self.base.me, tick.seen_last_frame_enemies
            );
        }
        for &handle in &tick.seen_last_frame_enemies {
            let Some(view) = ctx.entity_view(handle) else {
                tracing::warn!(
                    target: "robin_engine::ai_enemy::shield",
                    frame = ctx.frame,
                    me = self.base.me,
                    handle,
                    "RefreshArrowProtection: enemy detectable has no entity view"
                );
                continue;
            };
            let min_dist = archer::MIN_PROTECT_ARROW_DISTANCE as f32;
            if debug {
                eprintln!(
                    "ARROW_PROTECTION_ENEMY frame={} owner={} cand={} sq={} action={:?} bow={}",
                    ctx.frame,
                    self.base.me,
                    handle,
                    square_distance_to(handle),
                    view.action_state,
                    view.action_state.is_bow()
                );
            }
            if square_distance_to(handle) < min_dist * min_dist {
                continue;
            }
            if view.action_state.is_bow() {
                dangerous_enemy = handle;
                break;
            }
        }
        tracing::trace!(
            target: "robin_engine::ai_enemy::shield",
            frame = ctx.frame,
            me = self.base.me,
            seen = ?tick.seen_last_frame_enemies,
            candidates = ?tick
                .seen_last_frame_enemies
                .iter()
                .map(|&h| {
                    ctx.entity_view(h)
                        .map(|v| {
                            (
                                h,
                                v.action_state,
                                square_distance_to(h),
                            )
                        })
                })
                .collect::<Vec<_>>(),
            dangerous_enemy,
            "RefreshArrowProtection: dangerous-enemy scan"
        );

        if dangerous_enemy == 0 {
            // No dangerous archer — check if friendly archers need protection
            let protectable = self.number_of_nearby_archers_who_need_protection(ctx, tick);
            tracing::trace!(
                target: "robin_engine::ai_enemy::shield",
                frame = ctx.frame,
                me = self.base.me,
                protectable,
                "RefreshArrowProtection: no dangerous archer"
            );
            if protectable <= 0 {
                if debug {
                    eprintln!(
                        "ARROW_PROTECTION frame={} owner={} result=reject reason=no_protection_target nearest={} dangerous=0 protectable={} nearby={:?}",
                        ctx.frame,
                        self.base.me,
                        nearest_enemy,
                        protectable,
                        tick.nearby_fighters
                            .iter()
                            .map(|fighter| (
                                fighter.handle,
                                fighter.is_friendly,
                                fighter.ai_state,
                                fighter.current_substate,
                                fighter.is_archer_unit,
                                fighter.is_shield_bearer,
                                fighter.shield_bearer_before_me,
                                fighter.archer_behind_me,
                                fighter.position,
                                fighter.elevation,
                            ))
                            .collect::<Vec<_>>()
                    );
                }
                return false;
            }
            self.base.primary_target = nearest_enemy;
        } else {
            self.base.primary_target = dangerous_enemy;
        }

        // Focus primary target. Original Focus updates the NPC's view target;
        // it does not turn the actor's body or overwrite direction_goal.
        // RHArtificialMalignity::RefreshArrowProtection stores the target's
        // raw RHElement::GetPosition(), not RHArtificialIntelligence::Position()
        // (which snaps an actor in door transit to a gate endpoint). Fall
        // back to the entity-view map when the target is outside the
        // `nearby_fighters` snapshot — a dangerous archer is by definition
        // at least MIN_PROTECT_ARROW_DISTANCE away and may be posturing in
        // a way that keeps it out of that list entirely.
        let (target_pos, target_elevation) = shield_danger_point(
            self.find_fighter(self.base.primary_target, tick),
            ctx.entity_view(self.base.primary_target),
        )
        .unwrap_or_else(|| {
            tracing::warn!(
                target: "robin_engine::ai_enemy::shield",
                frame = ctx.frame,
                me = self.base.me,
                target = self.base.primary_target,
                "RefreshArrowProtection: primary target has no position snapshot"
            );
            (ctx.position, ctx.elevation)
        });
        self.base.outbox.actor.set_focus(self.base.primary_target);

        // Try to join a phalanx
        if let Some((
            run_pos,
            direction,
            left_neighbour,
            right_neighbour,
            inherited_sector_identity_differs,
        )) = self.find_phalanx_place(ctx, tick, grid)
        {
            if debug {
                eprintln!(
                    "ARROW_PROTECTION frame={} owner={} result=run_to_phalanx nearest={} dangerous={} target={} run_pos={:?} direction={} left={} right={} inherited_sector_identity_differs={}",
                    ctx.frame,
                    self.base.me,
                    nearest_enemy,
                    dangerous_enemy,
                    self.base.primary_target,
                    run_pos,
                    direction,
                    left_neighbour,
                    right_neighbour,
                    inherited_sector_identity_differs
                );
            }
            self.base.say(Remark::ShieldBearersLineFormation);
            self.base.seek_position = run_pos;
            self.shield_bearer_direction = direction;

            // Update combat neighbour links. Eager direct writes give
            // other code in this tick the new values; the queued
            // cross-NPC `Update*` actions perform the full reciprocal
            // cleanup at drain time (UpdateLeftCombatNeighbour /
            // UpdateRightCombatNeighbour), which includes clearing my
            // old neighbours' back-pointers, scrubbing the new
            // neighbours' stale right/left chains, and wiring the new
            // neighbours' back-pointers to me.
            let old_left = self.left_combat_neighbour;
            let old_right = self.right_combat_neighbour;
            self.left_combat_neighbour = left_neighbour;
            self.right_combat_neighbour = right_neighbour;
            self.base.outbox.reentrant.cross_npc_actions.push(
                CrossNpcAction::UpdateLeftCombatNeighbour {
                    target: self.base.me,
                    old_left,
                    new_left: left_neighbour,
                },
            );
            self.base.outbox.reentrant.cross_npc_actions.push(
                CrossNpcAction::UpdateRightCombatNeighbour {
                    target: self.base.me,
                    old_right,
                    new_right: right_neighbour,
                },
            );

            self.go_to(
                AiState::Attacking,
                Substate::AttackingRunningToPhalanx,
                run_pos,
                GotoFlags::RUN,
                ctx,
            );
            if inherited_sector_identity_differs {
                let order = self.base.outbox.actor.orders.last_mut().unwrap_or_else(|| {
                    panic!(
                        "phalanx placement GoTo for {} did not emit its required movement intent",
                        self.base.me
                    )
                });
                order.source_target_sector_identity_differs = true;
            }
        } else {
            if debug {
                eprintln!(
                    "ARROW_PROTECTION frame={} owner={} result=raise_shield nearest={} dangerous={} target={} target_pos={:?} target_elevation_bits={}",
                    ctx.frame,
                    self.base.me,
                    nearest_enemy,
                    dangerous_enemy,
                    self.base.primary_target,
                    target_pos,
                    target_elevation.to_bits()
                );
            }
            // No phalanx slot — raise shield in place
            tracing::trace!(
                target: "robin_engine::ai_enemy::shield",
                frame = ctx.frame,
                me = self.base.me,
                dangerous_enemy,
                substate = ?self.base.current_substate,
                "RefreshArrowProtection: raising shield in place"
            );
            self.base.stop_all();
            self.base.raise_shield(target_pos, target_elevation);

            // Emoticon: first time gets the X mark, advancing gets nothing
            if self.base.current_substate == Substate::AttackingAdvancingWithShield
                || dangerous_enemy == 0
            {
                self.base.clear_emoticon();
            } else {
                self.base
                    .set_transient_emoticon(EmoticonType::XMark, 30, ctx.frame);
                self.base.say(Remark::ShieldBearerCovers);
            }

            self.set_state(AiState::Attacking, Substate::AttackingProtectingWithShield);
            self.base.launch_timer(10, ctx.frame);
        }

        true
    }

    /// ProposeCombatPositions.
    fn propose_combat_positions(
        &self,
        list: &mut Vec<CombatPosition>,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) {
        if self.base.blood_alcohol > 0 {
            return;
        }

        let mut try_to_surround = true;

        let me_snap = self.find_fighter(self.base.me, tick);
        let i_am_a_formation_soldier = self.get_rank() == ProfileRank::Soldier
            && self.base.list_us.len() > 2
            && me_snap.map(|f| f.has_formation).unwrap_or(false);
        if i_am_a_formation_soldier {
            let (left, right) = self.propose_left_and_right_neighbour(ctx, tick);
            if left != 0 && right != 0 {
                self.propose_combat_positions_between(list, left, right, ctx, tick, grid);
                try_to_surround = false;
            } else if left != 0 {
                self.propose_combat_positions_right_of(list, left, ctx, tick, grid);
                try_to_surround = false;
            } else if right != 0 {
                self.propose_combat_positions_left_of(list, right, ctx, tick, grid);
                try_to_surround = false;
            }
        }

        if try_to_surround {
            // The reference also has a commented-out "help proud guy"
            // path; skip.
            for enemy_handle in self.list_them.clone() {
                self.propose_combat_positions_around(list, enemy_handle, ctx, tick, grid);
            }
        }

        self.clean_up_list_of_combat_positions(list, ctx, tick);
    }

    /// CleanUpListOfCombatPositions. The 0th entry is the current
    /// position and is never removed (only penalised).
    fn clean_up_list_of_combat_positions(
        &self,
        list: &mut Vec<CombatPosition>,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        let me_pos = ctx.position;

        // "Enemy who attacks only me" — if my principal opponent is engaged
        // with nobody else, I can't leave him.
        let lock_to_target: HumanHandle = self
            .find_fighter(self.base.primary_target, tick)
            .filter(|f| f.number_of_opponents <= 1)
            .map(|f| f.handle)
            .unwrap_or(0);

        let mut i = 0usize;
        while i < list.len() {
            let mut clean_me = false;
            // Update the change-adversary flag based on the current primary.
            list[i].change_adversary = list[i].target != self.base.primary_target;

            let cant_leave_target = lock_to_target != 0 && list[i].target != lock_to_target;
            let too_far = square_norm(pos_diff(&list[i].attacker_position, &me_pos))
                > combat::SQR_MAX_NEW_POS_DIST as f32;
            // The reference uses pure VIP rules. Bake the "still
            // engageable" checks (friendly / down) in here explicitly so
            // a downed or side-switched former target gets cleaned up —
            // the reference relied on the surrounding state machine to
            // clear these, but the Rust engine does not currently have
            // an equivalent sweep.
            let illegal_target = list[i].change_adversary
                && (!self.is_allowed_to_attack(list[i].target, ctx, tick)
                    || self
                        .find_fighter(list[i].target, tick)
                        .is_some_and(|f| f.is_friendly || !f.is_able_to_fight));

            if cant_leave_target || too_far || illegal_target {
                clean_me = true;
            } else {
                // Penalise / cull based on nearby fighters.
                for enemy_handle in &self.list_them {
                    let Some(enemy) = self.find_fighter(*enemy_handle, tick) else {
                        continue;
                    };
                    let dist = max_norm(pos_diff(&enemy.position, &list[i].attacker_position));
                    if dist < combat::MIN_ENEMY_DIST as f32 {
                        clean_me = true;
                        break;
                    }
                    if *enemy_handle != list[i].target && dist < enemy.sword_range_maximal as f32 {
                        list[i].bonus = list[i]
                            .bonus
                            .saturating_sub(combat::ENEMY_NEAR_MALUS as i16);
                    }
                }

                if !clean_me && list[i].line_jump.is_none() {
                    for friend_handle in &self.base.list_us {
                        if *friend_handle == self.base.me {
                            continue;
                        }
                        let Some(friend) = self.find_fighter(*friend_handle, tick) else {
                            continue;
                        };
                        if max_norm(pos_diff(&friend.position, &list[i].attacker_position))
                            < combat::MIN_FRIEND_DIST as f32
                        {
                            clean_me = true;
                            break;
                        }
                    }
                }
            }

            if clean_me {
                if i == 0 {
                    // Never drop the actual position — penalise it instead.
                    list[i].bonus = list[i]
                        .bonus
                        .saturating_sub(combat::BAD_POSITION_MALUS as i16);
                    i += 1;
                } else {
                    // Original `SBList::Delete(i)` preserves proposal order.
                    // Equal scores keep the first candidate, so a swap-remove
                    // can authoritatively select a different combat position.
                    list.remove(i);
                }
            } else {
                i += 1;
            }
        }
    }

    // -----------------------------------------------------------------------
    // ReconsiderSwordfight
    // -----------------------------------------------------------------------

    pub fn reconsider_swordfight(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        enemy_weak: bool,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) {
        let reconsider_debug = reconsider_position_debug_matches(
            || ctx.frame,
            || ctx.original_creation_order,
            || self.base.me,
        );
        if reconsider_debug {
            let rng_cursor = crate::sim_rng::original_replay_cursor(sim);
            eprintln!(
                "[RECONSIDER_ENTRY] frame={} owner={} creation_order={:?} phase=entry rng={:?} substate={:?} primary={} swordfighting={} enter_pending={} position=({:?},{:?},{:?}) direction={} blood={} cheat={} trainer={}",
                ctx.frame,
                self.base.me,
                ctx.original_creation_order,
                rng_cursor,
                self.base.current_substate,
                self.base.primary_target,
                ctx.is_swordfighting,
                ctx.enter_swordfight_pending,
                ctx.position.x.to_bits(),
                ctx.position.y.to_bits(),
                ctx.elevation.to_bits(),
                ctx.direction,
                self.base.blood_alcohol,
                global.stupid_soldiers_cheat,
                self.combat_trainer,
            );
        }
        // Keep ourselves on a heartbeat while in swordfight.
        if self.base.current_substate == Substate::AttackingSwordfight {
            self.base.launch_timer(20, ctx.frame);
        }

        // Bail out if an ENTER_SWORDFIGHT sequence element is already
        // queued — the entity isn't ready to fight yet. The engine sets
        // `enter_swordfight_pending` on AiContext when there's a pending
        // ENTER_SWORDFIGHT in the sequence manager.
        if ctx.enter_swordfight_pending {
            if reconsider_debug {
                let rng_cursor = crate::sim_rng::original_replay_cursor(sim);
                eprintln!(
                    "[RECONSIDER_ENTRY] frame={} owner={} phase=return reason=enter_pending rng={:?}",
                    ctx.frame, self.base.me, rng_cursor,
                );
            }
            return;
        }

        // Are we still swordfighting at all? Route through
        // Think(EVENT_QUIT_SWORDFIGHT) so the unexpected-event handler
        // fires. Cascade caveat: skips engine FilterAIEvent gate — see
        // end_think comment.
        if !ctx.is_swordfighting {
            if reconsider_debug {
                let rng_cursor = crate::sim_rng::original_replay_cursor(sim);
                eprintln!(
                    "[RECONSIDER_ENTRY] frame={} owner={} phase=return reason=not_swordfighting rng={:?}",
                    ctx.frame, self.base.me, rng_cursor,
                );
            }
            let quit_stimulus = Stimulus::new(StimulusType::EventQuitSwordfight);
            if self.base.has_script_filter_override {
                tracing::warn!(
                    target: "filter_ai_event_divergence",
                    handle = self.base.me as i32,
                    stimulus_type = ?quit_stimulus.stimulus_type,
                    "cascade think() skipped FilterAIEvent gate (reconsider_swordfight \
                     quit) — would re-filter; scripted actor may see divergent \
                     behavior"
                );
            }
            self.think(sim, &quit_stimulus, global, ctx, tick, grid);
            return;
        }

        // Refresh principal opponent from the snapshot.
        if let Some(me) = self.find_fighter(self.base.me, tick) {
            self.base.primary_target = me.principal_opponent;
        }
        if reconsider_debug {
            let rng_cursor = crate::sim_rng::original_replay_cursor(sim);
            eprintln!(
                "[RECONSIDER_ENTRY] frame={} owner={} phase=principal primary={} rng={:?}",
                ctx.frame, self.base.me, self.base.primary_target, rng_cursor,
            );
        }

        // Scotch: if we somehow ended up with a friendly target, bail
        // out cleanly. Use the live entity-view snapshot rather than
        // `nearby_fighters`: the latter only contains detected fighters
        // and can momentarily omit the principal opponent during sword
        // animations.
        let primary_is_friend = ctx
            .entity_view(self.base.primary_target)
            .map(|v| v.camp == ctx.camp)
            .unwrap_or_else(|| {
                tracing::warn!(
                    handle = self.base.me,
                    primary_target = self.base.primary_target,
                    "reconsider_swordfight: missing primary target entity view for friend check"
                );
                false
            });
        if primary_is_friend {
            if reconsider_debug {
                let rng_cursor = crate::sim_rng::original_replay_cursor(sim);
                eprintln!(
                    "[RECONSIDER_ENTRY] frame={} owner={} phase=return reason=primary_friend primary={} rng={:?}",
                    ctx.frame, self.base.me, self.base.primary_target, rng_cursor,
                );
            }
            self.end_swordfight(ctx, tick);
            // Original uses the reciprocal Update* setters here
            // (RHartificialmalignity.cpp:13709-13710).
            self.clear_combat_neighbours();
            self.set_state(AiState::Attacking, Substate::AttackingQuittingSwordfight);
            self.base.launch_timer(3, ctx.frame);
            return;
        }

        // Sight check. This must call the real 360° detection
        // equivalent. `nearby_fighters` is only populated by the primary
        // AI detection pass and can be missing during swordfight
        // animation transitions; treating that absence as lost sight
        // made soldiers quit combat and walk into battle-overview
        // positions while still engaged.
        let detects_primary = self.is_detecting_360_degrees(self.base.primary_target, ctx);
        if reconsider_debug {
            let rng_cursor = crate::sim_rng::original_replay_cursor(sim);
            eprintln!(
                "[RECONSIDER_ENTRY] frame={} owner={} phase=detection primary={} detected={} rng={:?}",
                ctx.frame, self.base.me, self.base.primary_target, detects_primary, rng_cursor,
            );
        }
        if !detects_primary {
            // Lost sight: forecast their direction and abandon the fight.
            // `primary_target` may just have changed to the actor's principal
            // opponent. Never apply the tick's old primary-target forecast to
            // that replacement: Original refreshes mpPrimaryTarget first and
            // calls ForecastDestinationForIA on the refreshed pointer.
            let prepared = tick
                .enemy_detectable_forecasts
                .iter()
                .find_map(|(handle, forecast)| {
                    (*handle == self.base.primary_target).then_some(forecast)
                })
                .or_else(|| {
                    (tick.primary_target_snapshot_handle == self.base.primary_target)
                        .then_some(tick.primary_target_forecast.as_ref())
                        .flatten()
                })
                .unwrap_or_else(|| {
                    panic!(
                        "ReconsiderSwordfight refreshed primary target {} without a matching destination forecast",
                        self.base.primary_target
                    )
                });
            let forecast =
                prepared.resolve_retaining_direction(sim, self.pc_gone_away_in_this_direction);
            self.base.seek_position = forecast.position;
            self.pc_gone_away_in_this_direction = forecast.direction;
            self.missed_pc = self.base.primary_target;
            self.pc_missed = true;
            self.end_swordfight(ctx, tick);

            // Focus(NULL) — clear target lock.
            self.base.outbox.actor.set_unfocus();

            // Chase or overview depending on target type and personality.
            let missed_is_pc = ctx
                .entity_view(self.missed_pc)
                .unwrap_or_else(|| {
                    panic!(
                        "ReconsiderSwordfight lost refreshed target {} without an entity view",
                        self.missed_pc
                    )
                })
                .is_pc;
            if missed_is_pc && self.answer_question(Question::ShallIFollowLostEnemy, ctx) {
                self.base.say(Remark::HuntsEnemy);
                self.seek_area(
                    sim,
                    self.base.seek_position,
                    parameters_ai::AI_LOST_ENEMY_SEEK_RADIUS as u16,
                    SeekFlags::LOCATION_FIRST | SeekFlags::HOUSE,
                    self.pc_gone_away_in_this_direction,
                    global,
                    ctx,
                    tick,
                );
            } else {
                // ForecastDestinationForIA above only populates the retained
                // seek center. Original aims this immediate snap at the
                // missed actor's current `Position`, not that forecast.
                let missed_position = ctx
                    .entity_view(self.missed_pc)
                    .unwrap_or_else(|| {
                        panic!(
                            "lost-enemy overview owner {} is missing current position for target {}",
                            self.base.me, self.missed_pc
                        )
                    })
                    .position;
                let dx = missed_position.x - ctx.position.x;
                let dy = missed_position.y - ctx.position.y;
                let dir = vec_to_sector(dx, dy);
                self.base.outbox.actor.set_direction_instantly = Some(dir as i16);
                self.get_battle_overview(0, ctx, tick);
            }
            return;
        }

        let primary_snapshot = self.find_fighter(self.base.primary_target, tick).cloned();
        let Some(primary) = primary_snapshot else {
            // We still detect the opponent by the 360° predicate, so do
            // not run the lost-enemy branch. Missing data here means the
            // per-tick fighter cache is incomplete for this frame.
            tracing::warn!(
                handle = self.base.me,
                primary_target = self.base.primary_target,
                "reconsider_swordfight: detected primary target is absent from nearby_fighters"
            );
            if reconsider_debug {
                let rng_cursor = crate::sim_rng::original_replay_cursor(sim);
                eprintln!(
                    "[RECONSIDER_ENTRY] frame={} owner={} phase=return reason=missing_primary_snapshot primary={} rng={:?}",
                    ctx.frame, self.base.me, self.base.primary_target, rng_cursor,
                );
            }
            return;
        };

        // Are we facing the primary opponent?
        let facing_target_position = swordfight_facing_target_position(&primary, tick);
        let facing_primary = is_facing_swordfight_target(
            &ctx.position,
            ctx.elevation,
            ctx.direction,
            facing_target_position,
            primary.elevation,
        );
        if reconsider_debug {
            let rng_cursor = crate::sim_rng::original_replay_cursor(sim);
            eprintln!(
                "[RECONSIDER_ENTRY] frame={} owner={} phase=facing primary={} target=({:?},{:?},{:?}) direction={} facing={} rng={:?}",
                ctx.frame,
                self.base.me,
                self.base.primary_target,
                facing_target_position.x.to_bits(),
                facing_target_position.y.to_bits(),
                primary.elevation.to_bits(),
                ctx.direction,
                facing_primary,
                rng_cursor,
            );
        }
        if !facing_primary {
            // Need to turn first; the engine will rotate us, then call back.
            return;
        }

        // -----------------------------------------------------------------
        // Build the us / them lists from the cached snapshot.
        // -----------------------------------------------------------------
        let me_pos = ctx.position;

        self.base.list_us.clear();
        self.base.list_us.push(self.base.me);
        self.list_them.clear();
        let mut nearest_friend_solo: HumanHandle = 0;
        let mut nearest_friend_solo_dist = f32::MAX;
        let mut number_of_swordfighting_enemies: u16 = 0;

        // First pass: us list (only friends actively swordfighting). The
        // engine builds this dedicated snapshot from the complete friendly
        // fighter registry using Original's 3D-world `MaxNormDistance`.
        for f in &tick.reconsider_swordfight_friends {
            self.base.list_us.push(f.handle);
            let dist = f.max_norm_distance as f32;
            if f.number_of_opponents > 1 && dist < nearest_friend_solo_dist {
                nearest_friend_solo = f.handle;
                nearest_friend_solo_dist = dist;
            }
        }

        // Second pass: them list (any able-to-fight enemy that we can see).
        for f in &tick.reconsider_swordfight_enemies {
            if !f.is_able_to_fight {
                continue;
            }
            if !self.is_detecting_360_degrees(f.handle, ctx) {
                continue;
            }
            self.list_them.push(f.handle);
            if f.is_swordfighting {
                number_of_swordfighting_enemies += 1;
            }
        }
        let number_of_friends = self.base.list_us.len() as u16;
        if reconsider_debug {
            let rng_cursor = crate::sim_rng::original_replay_cursor(sim);
            eprintln!(
                "[RECONSIDER_ENTRY] frame={} owner={} phase=lists us={:?} them={:?} swordfighting_enemies={} nearest_friend_solo={} rng={:?}",
                ctx.frame,
                self.base.me,
                self.base.list_us,
                self.list_them,
                number_of_swordfighting_enemies,
                nearest_friend_solo,
                rng_cursor,
            );
        }

        // Merry men with bow flee!
        if self.is_merry_man_forest(ctx)
            && self.is_archer()
            && self.merry_man_forest_cassos(ctx, global)
        {
            // Flee!
            if reconsider_debug {
                let rng_cursor = crate::sim_rng::original_replay_cursor(sim);
                eprintln!(
                    "[RECONSIDER_ENTRY] frame={} owner={} phase=return reason=merry_archer_flee rng={:?}",
                    ctx.frame, self.base.me, rng_cursor,
                );
            }
            return;
        }

        // Imbalanced situation rebalance — if I'm dogpiling someone with
        // help while a friend is fighting solo, swap to the solo fighter's
        // nearest enemy.
        let primary_outnumbered = primary.number_of_opponents > 1;
        if reconsider_debug {
            eprintln!(
                "[RECONSIDER_ENTRY] frame={} owner={} phase=rebalance_gate primary={} primary_opponents={} outnumbered={} nearest_friend_solo={}",
                ctx.frame,
                self.base.me,
                self.base.primary_target,
                primary.number_of_opponents,
                primary_outnumbered,
                nearest_friend_solo,
            );
        }
        if primary_outnumbered && nearest_friend_solo != 0 {
            let nearest_enemy_of_solo = calculate_opponent_nearest_to_rene(
                |handle| self.find_fighter(handle, tick),
                nearest_friend_solo,
                &me_pos,
            );
            if reconsider_debug {
                let maurice = tick
                    .nearby_fighters
                    .iter()
                    .find(|f| f.handle == nearest_friend_solo);
                eprintln!(
                    "[RECONSIDER_ENTRY] frame={} owner={} phase=rebalance_maurice maurice={} present={} opponents={:?} nearby={:?} nearest_enemy_of_solo={}",
                    ctx.frame,
                    self.base.me,
                    nearest_friend_solo,
                    maurice.is_some(),
                    maurice.map(|m| m.opponent_handles.clone()),
                    tick.nearby_fighters
                        .iter()
                        .map(|f| f.handle)
                        .collect::<Vec<_>>(),
                    nearest_enemy_of_solo,
                );
            }
            if nearest_enemy_of_solo != 0 {
                let nearest_to_that_enemy = calculate_opponent_nearest_to_rene(
                    |handle| self.find_fighter(handle, tick),
                    self.base.primary_target,
                    self.find_fighter(nearest_enemy_of_solo, tick)
                        .map(|f| &f.position)
                        .unwrap_or(&me_pos),
                );
                let i_should_take_him = nearest_to_that_enemy == self.base.me;
                if reconsider_debug {
                    eprintln!(
                        "[RECONSIDER_ENTRY] frame={} owner={} phase=rebalance_pick nearest_enemy_of_solo={} nearest_to_that_enemy={} i_should_take_him={}",
                        ctx.frame,
                        self.base.me,
                        nearest_enemy_of_solo,
                        nearest_to_that_enemy,
                        i_should_take_him,
                    );
                }
                if i_should_take_him {
                    // Original calls RHElementActorHuman::EnterSwordFight
                    // directly here. It does not launch another
                    // ENTER_SWORDFIGHT command: doing so would make that
                    // command's EventDone re-enter this branch recursively.
                    self.base.outbox.actor.enter_swordfight =
                        Some(EnterSwordfightRequest::Rebalance(nearest_enemy_of_solo));
                    if reconsider_debug {
                        let rng_cursor = crate::sim_rng::original_replay_cursor(sim);
                        eprintln!(
                            "[RECONSIDER_ENTRY] frame={} owner={} phase=return reason=rebalance target={} rng={:?}",
                            ctx.frame, self.base.me, nearest_enemy_of_solo, rng_cursor,
                        );
                    }
                    return;
                }
            }
            // Re-confirm primary target from snapshot now that we kept it.
            if let Some(me) = self.find_fighter(self.base.me, tick) {
                self.base.primary_target = me.principal_opponent;
            }
        }

        // Stupid-soldiers cheat short circuit.
        if global.stupid_soldiers_cheat {
            if reconsider_debug {
                let rng_cursor = crate::sim_rng::original_replay_cursor(sim);
                eprintln!(
                    "[RECONSIDER_ENTRY] frame={} owner={} phase=return reason=stupid_soldiers_cheat rng={:?}",
                    ctx.frame, self.base.me, rng_cursor,
                );
            }
            return;
        }

        // Original: both gates are evaluated even for a sober soldier.
        // Preserve the `||` short-circuit: a zero roll at blood alcohol 0
        // consumes only the first draw and still freezes the soldier.
        if drunk_combat_freezes(sim, self.base.blood_alcohol) {
            if reconsider_debug {
                let rng_cursor = crate::sim_rng::original_replay_cursor(sim);
                eprintln!(
                    "[RECONSIDER_ENTRY] frame={} owner={} phase=return reason=drunk_freeze rng={:?}",
                    ctx.frame, self.base.me, rng_cursor,
                );
            }
            return;
        }
        if reconsider_debug {
            let rng_cursor = crate::sim_rng::original_replay_cursor(sim);
            eprintln!(
                "[RECONSIDER_ENTRY] frame={} owner={} phase=after_drunk blood={} rng={:?}",
                ctx.frame, self.base.me, self.base.blood_alcohol, rng_cursor,
            );
        }

        // Refresh primary snapshot in case it changed above.
        let Some(primary) = self.find_fighter(self.base.primary_target, tick).cloned() else {
            if reconsider_debug {
                let rng_cursor = crate::sim_rng::original_replay_cursor(sim);
                eprintln!(
                    "[RECONSIDER_ENTRY] frame={} owner={} phase=return reason=missing_refreshed_primary primary={} rng={:?}",
                    ctx.frame, self.base.me, self.base.primary_target, rng_cursor,
                );
            }
            return;
        };
        let to_target = pos_diff(&primary.position, &ctx.position);
        // Original stores Norm() in a UWORD before every following range
        // comparison. Preserve that truncation: a target at 90.7 units is
        // compared as 90, and is therefore still within a 90-unit maximal
        // range. Keeping the fractional f32 here can spuriously relaunch an
        // approach instead of reaching ProposeGoodSwordStrike.
        let dist_to_target = original_uword_norm(to_target);

        // Weak-enemy charge: a soldier sprints in if the foe is out of
        // his max range and he has the capacity to charge.
        //
        // This gate is spelled `Distance( mpPrimaryTarget )`
        // (`RHartificialmalignity.cpp:13756`), which is
        // `RHArtificialIntelligence::Distance`
        // (`RHartificialintelligence.cpp:6935`):
        // `( target->GetPosition() - me->GetPosition() )
        //      .StretchY( INVERSE_ASPECT_RATIO ).Norm()` — the stretched
        // **3D world** norm. That is a different metric from the flat
        // map-space `( Position( target ) - Position( me ) ).Norm()` the
        // "too far to adversary" test further down uses
        // (`RHartificialmalignity.cpp:13846`; `RHposition` is 2D). Screen-
        // vertical separation stretches by ~1.74, so conflating the two
        // under-reports this distance badly and skips charges the Original
        // commits to — after which the Original returns, leaving the
        // reposition roll and the strike proposal undrawn.
        //
        // The comparison is also a plain FLOAT one: `GetRange( MAXIMAL )`
        // promotes to float, so there is no UWORD truncation here (unlike
        // the `uwDistance` test below).
        let weak_charge_distance = ai_square_distance(
            &primary.position,
            primary.elevation,
            &ctx.position,
            ctx.elevation,
        )
        .sqrt();
        let my_max_range = self
            .find_fighter(self.base.me, tick)
            .map(|f| f.sword_range_maximal)
            .unwrap_or(self.sword_range) as f32;
        let my_fighting_ability = self
            .find_fighter(self.base.me, tick)
            .map(|f| f.fighting_ability)
            .unwrap_or(0);
        if reconsider_debug {
            eprintln!(
                "[RECONSIDER_ENTRY] frame={} owner={} phase=weak_charge_gate enemy_weak={} rank={:?} charge_dist={} flat_dist={} max_range={} ability={}",
                ctx.frame,
                self.base.me,
                enemy_weak,
                self.get_rank(),
                weak_charge_distance,
                dist_to_target,
                my_max_range,
                my_fighting_ability,
            );
        }
        if enemy_weak
            && self.get_rank() == ProfileRank::Soldier
            && weak_charge_distance > my_max_range
            && my_fighting_ability >= combat::MIN_CAPACITY_CHARGE_WEAK_ENEMY
        {
            let target_pos = primary.position;
            if reconsider_debug {
                let rng_cursor = crate::sim_rng::original_replay_cursor(sim);
                eprintln!(
                    "[RECONSIDER_ENTRY] frame={} owner={} phase=return reason=weak_enemy_charge target={} distance={} max_range={:?} ability={} rng={:?}",
                    ctx.frame,
                    self.base.me,
                    self.base.primary_target,
                    dist_to_target,
                    my_max_range,
                    my_fighting_ability,
                    rng_cursor,
                );
            }
            self.go_near(
                AiState::Attacking,
                Substate::AttackingMovingAroundOldEnemy,
                target_pos,
                self.sword_range as i32,
                GotoFlags::RUN | GotoFlags::SWORD,
                ctx,
            );
            return;
        }

        // Re-evaluate combat position 1 in 3 ticks (skip in pure 1v1
        // fights and combat-trainer mode).
        let reposition_debug = reconsider_debug;
        let reposition_eligible = !self.combat_trainer
            && (number_of_friends != 1 || number_of_swordfighting_enemies != 1);
        // Preserve Original's short-circuit: pure 1v1 and trainer fights do
        // not consume the one-in-three reposition draw.
        let reposition_roll = reposition_eligible
            .then(|| crate::sim_rng::u32(sim, crate::sim_rng::RngSite::CombatReposition, 0..3));
        let do_reposition = reposition_roll == Some(0);
        if reposition_debug {
            eprintln!(
                "[RECONSIDER_POSITION] frame={} owner={} creation_order={:?} friends={} swordfighting_enemies={} combat_trainer={} eligible={} roll={:?} do_reposition={}",
                ctx.frame,
                self.base.me,
                ctx.original_creation_order,
                number_of_friends,
                number_of_swordfighting_enemies,
                self.combat_trainer,
                reposition_eligible,
                reposition_roll,
                do_reposition,
            );
        }

        if do_reposition {
            let new_combat_position =
                self.propose_good_combat_position_inner(global, ctx, tick, grid, reposition_debug);
            self.base.seek_position = new_combat_position.attacker_position;
            self.my_line_jump = new_combat_position.line_jump;

            if new_combat_position.change_adversary {
                self.base.primary_target = new_combat_position.target;
                if new_combat_position.change_position {
                    self.set_state(AiState::Attacking, Substate::AttackingApproachingNewEnemy);
                    if new_combat_position.line_jump.is_some() {
                        self.go_near(
                            self.base.current_state,
                            self.base.current_substate,
                            new_combat_position.attacker_position,
                            30,
                            GotoFlags::SWORD,
                            ctx,
                        );
                    } else {
                        self.go_to(
                            self.base.current_state,
                            self.base.current_substate,
                            new_combat_position.attacker_position,
                            GotoFlags::SWORD,
                            ctx,
                        );
                    }
                    return;
                } else {
                    // Just turn to the new opponent, no position change.
                    // SetAsNewPrincipalOpponent.
                    self.set_state(AiState::Attacking, Substate::AttackingSwordfight);
                    debug_assert!(self.is_allowed_to_attack(self.base.primary_target, ctx, tick));
                    self.base.outbox.actor.set_principal = Some(self.base.primary_target);
                    self.base.launch_timer(20, ctx.frame);
                    return;
                }
            } else if new_combat_position.change_position {
                self.set_state(AiState::Attacking, Substate::AttackingMovingAroundOldEnemy);
                self.base
                    .go_to(new_combat_position.attacker_position, GotoFlags::SWORD, ctx);
                return;
            }
        }

        // Too far? Step in. Uses Norm (Euclidean) instead of squared
        // distance.
        let primary_max_range = primary.sword_range_maximal;
        if dist_to_target > my_max_range as u16
            && dist_to_target > primary_max_range
            && self.my_line_jump.is_none()
            && !self.combat_trainer
        {
            let target_pos = primary.position;
            self.set_state(AiState::Attacking, Substate::AttackingMovingAroundOldEnemy);
            self.base
                .go_near(target_pos, self.sword_range as i32, GotoFlags::SWORD, ctx);
            return;
        }

        // Combat-trainer recall to post.
        if self.combat_trainer {
            let initial = self.base.initial_position;
            if max_norm(pos_diff(&initial, &ctx.position)) > 20.0 {
                self.go_to(
                    AiState::Attacking,
                    Substate::AttackingMovingAroundOldEnemy,
                    initial,
                    GotoFlags::SWORD,
                    ctx,
                );
                return;
            }
        }

        // Honour: don't hit a downed enemy. Only attack while target is
        // in a sword action state, then propose a strike. Both checks
        // live engine-side in `tick_enemy_sword_attacks` (melee.rs): it
        // gates on the target's `action_state.is_sword()`, calls
        // `propose_good_sword_strike`, and handles the PC-specific
        // hulk/delay preamble + sequence launching.
        //
        // This is a one-shot authorization. The engine consumes it even
        // when one of those downstream checks rejects the proposal, matching
        // the Original's single event-driven call to ReconsiderSwordfight.
        self.pending_sword_strike_consideration = true;
    }

    // -----------------------------------------------------------------------
    // ProposeGoodStepBackGoal
    // -----------------------------------------------------------------------

    /// Compute a retreat position away from `pos_enemy`.
    /// Delegates to the free function [`propose_good_step_back_goal`].
    pub fn propose_good_step_back_goal(
        &self,
        pos_enemy: Position,
        good_distance: u16,
        min_distance: u16,
        ctx: &AiContext,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
        aspect_ratio: f32,
    ) -> Option<Position> {
        propose_good_step_back_goal(
            ctx.position,
            &ctx.move_box,
            pos_enemy,
            good_distance,
            min_distance,
            grid,
            aspect_ratio,
        )
    }

    // -----------------------------------------------------------------------
    // ProposeStepDirectionWhileObservingCombat
    // -----------------------------------------------------------------------

    /// Decide whether it's better to step left or right while observing combat.
    /// Returns `true` when the preferred direction is the left normal
    /// (i.e. friends are more crowded to the right, so step left to spread out).
    fn propose_step_direction_while_observing_combat(
        &self,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        // Right perpendicular of our facing direction in isometric space.
        // RHElement::GetDirectionVector builds its vector with
        // SetSector0to15(direction, ASPECT_RATIO).  The aspect-scaled
        // components are observable here because the following scalar
        // products are truncated to SWORD before left/right scoring.
        let dir_vec = sector_to_vector_iso(ctx.direction, ASPECT_RATIO);
        let right_vec = get_normal_iso(dir_vec, false, ASPECT_RATIO);

        let mut points_for_right: i32 = 0;

        // ReconsiderSwordfightObservation rebuilds list_us with the
        // IsAbleToFight + MAX_SWORDFIGHT_CONSIDERATION_RADIUS gates.
        // Reading from `self.base.list_us` keeps the right/left score
        // honest — a downed or out-of-radius friend should not
        // contribute.
        for handle in &self.base.list_us {
            if *handle == self.base.me {
                continue;
            }
            let Some(f) = self.find_fighter(*handle, tick) else {
                continue;
            };
            if !f.is_friendly || !f.is_soldier {
                continue;
            }
            // Only count friends in observing/stationary combat
            // substates (not moving/fleeing/etc).
            if !is_observing_combat_substate(f.current_substate) {
                continue;
            }
            let v = pos_diff(&f.position, &ctx.position);
            let scalar = dot2(right_vec, v) as i32;

            // `scalar > 0` takes the right-bonus branch, otherwise
            // (including `scalar == 0`) takes the right-malus branch.
            // Mirror the inclusive zero on the negative side.
            if (1..=200).contains(&scalar) {
                points_for_right += 200 - scalar;
            } else if (-200..=0).contains(&scalar) {
                points_for_right -= 200 + scalar;
            }
        }

        points_for_right > 0
    }

    // -----------------------------------------------------------------------
    // ReconsiderSwordfightObservation
    // -----------------------------------------------------------------------

    /// EVENT_TIMER handler for `Substate::AttackingObserve`. Runs its
    /// own decision body literally rather than dispatching through
    /// `battle_decisions` (which has a different decision tree). Walks
    /// these steps:
    ///   1. RefreshArrowProtection guard
    ///   2. rebuild list_them with `IsAbleToFight + MaxNorm <
    ///      MAX_SWORDFIGHT_CONSIDERATION_RADIUS + IsDetecting180Degrees`
    ///   3. rebuild list_us and bump local primary-target multiplicity for
    ///      same-camp soldiers in any swordfight substate
    ///   4. `GetNewPrimaryTarget(PRIMARY_TARGET_UNOCCUPIED_STRONGLY_PREFERED)`
    ///      with the local multiplicity override
    ///   5. Focus(primary)
    ///   6. null primary → `GetBattleOverview` and bail
    ///   7. combat_trainer → SetDirection + Observe + LaunchTimer(20) and bail
    ///   8. defensive predecision → step-back goal or directed panic
    ///   9. attack-opportunity block (back-to-me / not-swordfighting /
    ///      principal opponent dogpiled / very close) gated on no friend
    ///      already approaching the same target
    ///   10. fall through to `observe_and_step` for repositioning
    pub(super) fn reconsider_swordfight_observation(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) {
        let mut deferred_defensive_panic = None;

        // (1) Arrow protection guard.
        if self.refresh_arrow_protection(false, ctx, tick, grid) {
            return;
        }

        // (2) Rebuild list_them with the three filters and reset both the
        //     selector-local view and Original's owner-ordered shared UWORD
        //     scratch. The reference's count is restricted to fighters within
        //     `MAX_SWORDFIGHT_CONSIDERATION_RADIUS`.
        self.list_them.clear();
        let max_radius = parameters_ai::MAX_SWORDFIGHT_CONSIDERATION_RADIUS as f32;
        let me_pos = ctx.position;
        let me_world = tick
            .reconsider_swordfight_observation_fighters
            .iter()
            .find(|fighter| fighter.handle == self.base.me)
            .unwrap_or_else(|| {
                panic!(
                    "ReconsiderSwordfightObservation owner {} is absent from its fighter registry",
                    self.base.me
                )
            })
            .raw_world_position;
        let raw_max_norm_distance =
            |fighter: &crate::ai::ReconsiderSwordfightObservationFighter| {
                let dx = fighter.raw_world_position.x - me_world.x;
                let dy = (fighter.raw_world_position.y - me_world.y)
                    * crate::position_interface::INVERSE_ASPECT_RATIO;
                let dz = fighter.raw_world_position.z - me_world.z;
                dx.abs().max(dy.abs()).max(dz.abs())
            };
        let reconsider_debug = reconsider_observation_debug_matches(ctx.frame, self.base.me);
        if reconsider_debug {
            eprintln!(
                "RECONSIDER {{\"event\":\"invoke\",\"frame\":{},\"owner\":{},\"state\":{:?},\"substate\":{:?},\"fighters\":{}}}",
                ctx.frame,
                self.base.me,
                self.base.current_state,
                self.base.current_substate,
                tick.reconsider_swordfight_observation_fighters.len(),
            );
        }
        let mut local_mult: std::collections::BTreeMap<HumanHandle, u32> =
            std::collections::BTreeMap::new();
        for f in &tick.reconsider_swordfight_observation_fighters {
            if f.is_friendly || !f.is_able_to_fight {
                if reconsider_debug {
                    let d = raw_max_norm_distance(f);
                    let rejection = if f.is_friendly { "friendly" } else { "unable" };
                    eprintln!(
                        "RECONSIDER {{\"event\":\"them_candidate\",\"frame\":{},\"owner\":{},\"fighter\":{},\"friendly\":{},\"able\":{},\"distance\":{},\"result\":{:?}}}",
                        ctx.frame,
                        self.base.me,
                        f.handle,
                        f.is_friendly,
                        f.is_able_to_fight,
                        d,
                        rejection,
                    );
                }
                continue;
            }
            let d = raw_max_norm_distance(f);
            if d >= max_radius {
                if reconsider_debug {
                    eprintln!(
                        "RECONSIDER {{\"event\":\"them_candidate\",\"frame\":{},\"owner\":{},\"fighter\":{},\"friendly\":{},\"able\":{},\"distance\":{},\"result\":\"radius\"}}",
                        ctx.frame, self.base.me, f.handle, f.is_friendly, f.is_able_to_fight, d,
                    );
                }
                continue;
            }
            let detected = self.is_detecting_180_degrees(f.handle, ctx);
            if reconsider_debug {
                let result = if detected {
                    "accepted"
                } else {
                    "not_detected_180"
                };
                eprintln!(
                    "RECONSIDER {{\"event\":\"them_candidate\",\"frame\":{},\"owner\":{},\"fighter\":{},\"friendly\":{},\"able\":{},\"distance\":{},\"result\":{:?}}}",
                    ctx.frame, self.base.me, f.handle, f.is_friendly, f.is_able_to_fight, d, result,
                );
            }
            if !detected {
                continue;
            }
            self.list_them.push(f.handle);
            local_mult.insert(f.handle, 0);
            global
                .primary_target_multiplicity_scratch
                .insert(f.handle, 0);
        }
        // (3) Rebuild list_us (self first), bump
        //     multiplicity for same-camp soldiers actively in any swordfight
        //     substate against a primary target.
        self.base.list_us.clear();
        self.base.list_us.push(self.base.me);
        for f in &tick.reconsider_swordfight_observation_fighters {
            if !f.is_friendly || f.handle == self.base.me || !f.is_able_to_fight {
                if reconsider_debug {
                    let d = raw_max_norm_distance(f) as u16;
                    let rejection = if !f.is_friendly {
                        "enemy"
                    } else if f.handle == self.base.me {
                        "self"
                    } else {
                        "unable"
                    };
                    eprintln!(
                        "RECONSIDER {{\"event\":\"us_candidate\",\"frame\":{},\"owner\":{},\"fighter\":{},\"friendly\":{},\"able\":{},\"distance_uword\":{},\"result\":{:?}}}",
                        ctx.frame,
                        self.base.me,
                        f.handle,
                        f.is_friendly,
                        f.is_able_to_fight,
                        d,
                        rejection,
                    );
                }
                continue;
            }
            // Original truncates `MaxNormDistance` to UWORD on the us-list
            // side before the radius comparison.
            let d = raw_max_norm_distance(f) as u16;
            if reconsider_debug {
                let result = if f32::from(d) >= max_radius {
                    "radius"
                } else {
                    "accepted"
                };
                eprintln!(
                    "RECONSIDER {{\"event\":\"us_candidate\",\"frame\":{},\"owner\":{},\"fighter\":{},\"friendly\":{},\"able\":{},\"distance_uword\":{},\"result\":{:?}}}",
                    ctx.frame, self.base.me, f.handle, f.is_friendly, f.is_able_to_fight, d, result,
                );
            }
            if f32::from(d) >= max_radius {
                continue;
            }
            self.base.list_us.push(f.handle);
            if f.is_soldier
                && f.primary_target != 0
                && is_any_swordfight_substate(f.current_substate)
            {
                let local = local_mult.entry(f.primary_target).or_insert(0);
                *local = u32::from((*local as u16).wrapping_add(1));
                let shared = global
                    .primary_target_multiplicity_scratch
                    .entry(f.primary_target)
                    .or_insert(0);
                *shared = u32::from((*shared as u16).wrapping_add(1));
            }
        }

        tracing::trace!(
            frame = ctx.frame,
            me = self.base.me,
            list_us = ?self.base.list_us,
            list_them = ?self.list_them,
            observation_fighters = tick.reconsider_swordfight_observation_fighters.len(),
            "ReconsiderSwordfightObservation rebuilt the us/them lists"
        );

        // (4) Pick new primary target with the local multiplicity
        //     override.
        let new_primary = self.get_new_primary_target_with_mult_override(
            PrimaryTargetFlags::UNOCCUPIED_STRONGLY_PREFERRED,
            ctx,
            tick,
            Some(&local_mult),
        );
        self.base.primary_target = new_primary;

        // (5) Focus(primary). With null primary the focus is cleared,
        //     matching the engine's drain behaviour.
        if new_primary != 0 {
            self.base.outbox.actor.set_focus(new_primary);
        } else {
            self.base.outbox.actor.set_unfocus();
        }

        // (6) No target → battle overview.
        if new_primary == 0 {
            self.get_battle_overview(0, ctx, tick);
            return;
        }

        // (7) Combat trainer special path: progressively face the target,
        //     stop, set Observe, launch 20-tick timer. The Original calls
        //     `SetDirection`, which changes the goal only; the actor's normal
        //     turn step advances the current direction on the following frame.
        if self.combat_trainer {
            if let Some(primary) = self.find_fighter(new_primary, tick) {
                let v = pos_diff(&primary.position, &me_pos);
                let dir = vec_to_sector_ar(v.0, v.1, ASPECT_RATIO);
                self.base.set_direction_goal(dir as u16);
            }
            self.base.outbox.actor.set_focus(new_primary);
            self.base.stop_all();
            self.set_state(AiState::Attacking, Substate::AttackingObserve);
            self.base.launch_timer(20, ctx.frame);
            return;
        }

        // (8) Defensive predecision: controlled step back, otherwise
        //     panic flee. The reference deliberately does not return
        //     here; the attack-opportunity and observe-step blocks below
        //     may immediately override the defensive move.
        if self.make_battle_predecisions(sim, ctx, tick) == Decision::PredecisionDefensive {
            let enemy_pos = self
                .find_fighter(new_primary, tick)
                .map(|f| f.position)
                .unwrap_or(ctx.position);
            self.base.seek_position = enemy_pos;
            if let Some(goal) = self.propose_good_step_back_goal(
                enemy_pos,
                parameters_ai::ARCHER_GOOD_DISTANCE,
                parameters_ai::ARCHER_MIN_DISTANCE,
                ctx,
                grid,
                ASPECT_RATIO,
            ) {
                self.go_to(
                    AiState::Fleeing,
                    Substate::FleeingRetireFromCombat,
                    goal,
                    GotoFlags::RUN,
                    ctx,
                );
            } else {
                deferred_defensive_panic =
                    Some(self.panic_after_failed_observation_step_back(enemy_pos));
            }
        }

        // (9) Attack-opportunity block. Trigger an
        //     AttackEnemy if any of:
        //       - target's facing dotted with (target → me) > 0 (back to me)
        //       - target is not swordfighting
        //       - target's principal opponent has >= 3 opponents
        //       - distance < 30
        //     gated by no same-camp soldier already approaching this target
        //     in WALKING/RUNNING/CHARGING.
        if let Some(primary) = self.find_fighter(new_primary, tick).cloned() {
            let pos_fighter = primary.position;
            let v_to_me = pos_diff(&me_pos, &pos_fighter);
            let distance = iso_norm(v_to_me, ASPECT_RATIO) as u16;

            // `RHElement::GetDirectionVector` constructs this vector with
            // `SetSector0to15(direction, ASPECT_RATIO)`.  The isometric Y
            // compression is significant near the behind/in-front boundary.
            let target_dir = sector_to_vector_iso(primary.direction, ASPECT_RATIO);
            // Reference condition:
            //   primary.direction_vector * (pos_fighter - pos_me) > 0
            // This means the target is looking away from the observer,
            // exposing their back. Using (me - fighter) instead makes
            // observers attack when the target faces them.
            let v_observer_to_target = pos_diff(&pos_fighter, &me_pos);
            let back_to_me = dot2(target_dir, v_observer_to_target) > 0.0;

            let principal_opponents_count = if primary.is_swordfighting {
                self.find_fighter(primary.principal_opponent, tick)
                    .map(|p| p.number_of_opponents)
                    .unwrap_or(0)
            } else {
                0
            };

            let attack_opportunity = back_to_me
                || !primary.is_swordfighting
                || principal_opponents_count >= 3
                || distance < 30;

            if attack_opportunity {
                let mut nobody_else_does = true;
                for handle in &self.base.list_us {
                    if *handle == self.base.me {
                        continue;
                    }
                    let Some(friend) = self.find_fighter(*handle, tick) else {
                        continue;
                    };
                    if !friend.is_soldier {
                        continue;
                    }
                    if friend.primary_target != new_primary {
                        // legacy implementation gates this on the friend's GetPrimaryTarget(),
                        // not their principal swordfight opponent.
                        continue;
                    }
                    if is_walking_running_charging_substate(friend.current_substate) {
                        nobody_else_does = false;
                        break;
                    }
                }
                if nobody_else_does {
                    // Original already ran Panic synchronously, then
                    // AttackEnemy's later StopAll/state change superseded it.
                    // Do not replay the deferred panic after AttackEnemy.
                    self.attack_enemy(new_primary, Some(&mut *global), ctx, tick, grid);
                    return;
                }
            }
        }

        // (10) Repositioning + fallback stay-in-place.
        self.observe_and_step(sim, ctx, tick, grid);
        if self.base.current_state == AiState::Fleeing
            && self.base.current_substate == Substate::FleeingPanic
            && let Some(request) = deferred_defensive_panic
        {
            debug_assert!(self.base.outbox.actor.begin_panic.is_none());
            self.base.outbox.actor.begin_panic = Some(request);
        }
    }

    // -----------------------------------------------------------------------
    // Observe-and-step movement
    // Repositioning logic from ReconsiderSwordfightObservation.
    // -----------------------------------------------------------------------

    /// Reposition while observing a swordfight: step forward, back, or sideways
    /// to maintain an ideal distance from the fight. Called when
    /// `battle_decisions` didn't produce a state change.
    fn observe_and_step(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) {
        let Some(primary) = self.find_fighter(self.base.primary_target, tick).cloned() else {
            return;
        };

        let pos_me = ctx.position;
        let mut pos_fighter = primary.position;

        // Ideal distance interpolated from courage.
        let ideal_distance = AiController::value_between(
            parameters_ai::OBSERVE_SWORDFIGHT_MAX_DISTANCE,
            parameters_ai::OBSERVE_SWORDFIGHT_MIN_DISTANCE,
            self.get_courage() as u8,
        );

        let v_to_fighter = pos_diff(&pos_me, &pos_fighter);
        let mut distance = iso_norm(v_to_fighter, ASPECT_RATIO) as u16;

        // If the primary target is swordfighting someone else who is
        // closer, use that person as the reference distance.
        if primary.is_swordfighting
            && primary.principal_opponent != self.base.me
            && let Some(friend) = self.find_fighter(primary.principal_opponent, tick)
        {
            let friend_v = pos_diff(&pos_me, &friend.position);
            let friend_dist = iso_norm(friend_v, ASPECT_RATIO) as u16;
            if friend_dist < distance {
                distance = friend_dist;
                pos_fighter = friend.position;
            }
        }

        let mut b_move = false;
        let mut pos_destination = pos_me;

        if distance + 50 < ideal_distance {
            // Too near — step back.
            let v = pos_diff(&pos_me, &pos_fighter);
            let n = iso_normalize(v, ASPECT_RATIO);
            let step = (ideal_distance - distance) as f32;
            pos_destination = Position {
                x: pos_me.x + n.0 * step,
                y: pos_me.y + n.1 * step,
                sector: pos_me.sector,
                level: pos_me.level,
            };
            b_move = check_straight_movement(grid, &pos_me, &pos_destination, &ctx.move_box);
        } else if distance > ideal_distance + 50 {
            // Too far — step forward.
            let v = pos_diff(&pos_fighter, &pos_me);
            let n = iso_normalize(v, ASPECT_RATIO);
            let step = (distance - ideal_distance) as f32;
            pos_destination = Position {
                x: pos_me.x + n.0 * step,
                y: pos_me.y + n.1 * step,
                sector: pos_me.sector,
                level: pos_me.level,
            };
            b_move = check_straight_movement(grid, &pos_me, &pos_destination, &ctx.move_box);
        }

        // Distance is OK — maybe a step sideways?
        // Source is `rand() % 2 == 0`. `sim_rng::bool` is true for the
        // opposite (odd) half of the stream, so keep the legacy residue
        // predicate explicit.
        if !b_move
            && crate::sim_rng::u32(sim, crate::sim_rng::RngSite::CombatObserveSideStep, 0..2) == 0
        {
            let prefer_left = self.propose_step_direction_while_observing_combat(ctx, tick);

            for i in 0..2u8 {
                // First try preferred direction, then the other.
                let direct = (i == 0) == prefer_left;
                let to_fighter = pos_diff(&pos_fighter, &pos_me);
                let normal = get_normal_iso(to_fighter, direct, ASPECT_RATIO);
                let n = iso_normalize(normal, ASPECT_RATIO);
                let step = parameters_ai::OBSERVE_SWORDFIGHT_SIDE_STEP;
                let candidate = Position {
                    x: pos_me.x + n.0 * step,
                    y: pos_me.y + n.1 * step,
                    sector: pos_me.sector,
                    level: pos_me.level,
                };

                // Don't walk into an obstacle.
                if !check_straight_movement(grid, &pos_me, &candidate, &ctx.move_box) {
                    continue;
                }

                // If we can currently see the fighter, make sure we can
                // still see them from the new position.
                if check_straight_movement(grid, &pos_me, &pos_fighter, &ctx.move_box)
                    && !check_straight_movement(grid, &candidate, &pos_fighter, &ctx.move_box)
                {
                    continue;
                }

                b_move = true;
                pos_destination = candidate;
                break;
            }
        }

        if b_move {
            // Go to the new position.
            self.base.outbox.actor.set_focus(self.base.primary_target);
            self.go_to(
                AiState::Attacking,
                Substate::AttackingObserveAndMove,
                pos_destination,
                GotoFlags::SWORD,
                ctx,
            );
        } else {
            // Stay in place, face primary target.
            let to_target = pos_diff(&primary.position, &ctx.position);
            let dir = vec_to_sector(to_target.0, to_target.1);
            self.base.set_direction_goal(dir as u16);
            self.base.outbox.actor.set_focus(self.base.primary_target);
            self.base.stop_all();
            self.set_state(AiState::Attacking, Substate::AttackingObserve);
            self.base.launch_timer(20, ctx.frame);
        }
    }

    // -----------------------------------------------------------------------
    // ProposeGoodCombatPosition
    // -----------------------------------------------------------------------

    pub fn propose_good_combat_position(
        &mut self,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> CombatPosition {
        self.propose_good_combat_position_inner(global, ctx, tick, grid, false)
    }

    fn propose_good_combat_position_inner(
        &mut self,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
        debug_reposition: bool,
    ) -> CombatPosition {
        debug_assert!(ctx.is_swordfighting);

        // Re-anchor primary target on the snapshot.
        if let Some(me) = self.find_fighter(self.base.me, tick) {
            self.base.primary_target = me.principal_opponent;
        }

        let me_pos = ctx.position;
        // Set `primary_target` from `me.principal_opponent` and then
        // assert `!is_friend(primary_target)`, which implicitly requires
        // a non-null primary. Mirror that as a panic — the project's
        // "no fake data" rule forbids silently substituting a default
        // position when the precondition is violated.
        let primary = self
            .find_fighter(self.base.primary_target, tick)
            .unwrap_or_else(|| {
                panic!(
                    "propose_good_combat_position: primary_target ({:?}) not found in snapshot \
                     (caller must ensure mpMe->GetPrincipalOpponent() resolves to a live fighter)",
                    self.base.primary_target
                )
            })
            .clone();
        assert!(
            !primary.is_friendly,
            "propose_good_combat_position: primary_target {:?} is a friend (asserted away)",
            self.base.primary_target,
        );

        // 0th entry: keep the current position.
        let mut possible: Vec<CombatPosition> = Vec::new();
        possible.push(CombatPosition {
            attacker: self.base.me,
            attacker_position: me_pos,
            target: self.base.primary_target,
            target_position: primary.position,
            target_direction: primary.direction,
            ..CombatPosition::default()
        });
        // Add alternatives.
        self.propose_combat_positions(&mut possible, ctx, tick, grid);

        // Build the enemies' positions list.
        let mut enemies_positions: Vec<CombatPosition> = Vec::new();
        for handle in &self.list_them {
            global
                .primary_target_multiplicity_scratch
                .insert(*handle, 0);
            let Some(f) = self.find_fighter(*handle, tick) else {
                continue;
            };
            let mut cp = CombatPosition {
                attacker: *handle,
                attacker_position: f.position,
                ..CombatPosition::default()
            };
            if f.is_swordfighting
                && let Some(opp) = self.find_fighter(f.principal_opponent, tick)
            {
                cp.target = opp.handle;
                cp.target_position = opp.position;
                cp.target_direction = opp.direction;
            }
            enemies_positions.push(cp);
        }

        // Build the friends' positions list (excluding me).
        let mut friends_positions: Vec<CombatPosition> = Vec::new();
        for handle in &self.base.list_us {
            if *handle == self.base.me {
                continue;
            }
            let Some(friend) = self.find_fighter(*handle, tick) else {
                continue;
            };
            // Split the friend record by class:
            //   - soldier friends → GetCombatPosition (intended
            //     position): live when in APPROACHING_NEW_ENEMY /
            //     MOVING_AROUND_OLD_ENEMY, `seek_position` otherwise.
            //   - non-soldier swordfighters → live `position` with their
            //     principal opponent.
            //   - non-swordfighter non-soldiers → live `position` with
            //     no target.
            // Only the soldier arm uses the seek/approach dichotomy.
            let (attacker_position, target_handle) = if friend.is_soldier {
                let approaching = friend.current_substate
                    == Substate::AttackingApproachingNewEnemy as u32
                    || friend.current_substate == Substate::AttackingMovingAroundOldEnemy as u32;
                if approaching {
                    (friend.position, friend.principal_opponent)
                } else {
                    (friend.seek_position, friend.primary_target)
                }
            } else if friend.is_swordfighting {
                (friend.position, friend.principal_opponent)
            } else {
                (friend.position, 0)
            };
            let mut cp = CombatPosition {
                attacker: *handle,
                attacker_position,
                ..CombatPosition::default()
            };
            if target_handle != 0
                && let Some(opp) = self.find_fighter(target_handle, tick)
            {
                cp.target = opp.handle;
                cp.target_position = opp.position;
                cp.target_direction = opp.direction;
                let count = global
                    .primary_target_multiplicity_scratch
                    .entry(opp.handle)
                    .or_insert(0);
                *count = u32::from((*count as u16).wrapping_add(1));
            }
            friends_positions.push(cp);
        }

        // Evaluate every candidate and keep the best.
        let me_handle = self.base.me;
        let them_handles = self.list_them.clone();
        let iq = self.get_iq(ctx);

        let mut best_index: usize = 0;
        let mut best_score: i32 = i32::MIN;
        for (idx, cp) in possible.iter_mut().enumerate() {
            let input = debug_reposition.then(|| {
                (
                    cp.attacker,
                    cp.attacker_position,
                    cp.target,
                    cp.target_position,
                    cp.target_direction,
                    cp.change_adversary,
                    cp.change_position,
                    cp.line_position,
                    cp.left_neighbour,
                    cp.right_neighbour,
                    cp.bonus,
                    cp.line_jump,
                )
            });
            let score = evaluate_combat_position_full(
                me_handle,
                &me_pos,
                &them_handles,
                cp,
                &mut friends_positions,
                &mut enemies_positions,
                FighterView {
                    near: &tick.nearby_fighters,
                    registry: &tick.fighter_registry,
                },
                tick.required_profile_manager(),
                iq,
            );
            if debug_reposition {
                eprintln!(
                    "[RECONSIDER_POSITION] frame={} owner={} candidate={} input={:?} evaluated=(attacker={} attacker_position={:?} target={} target_position={:?} target_direction={} change_adversary={} change_position={} line_position={} left={} right={} bonus={} line_jump={:?}) score={}",
                    ctx.frame,
                    self.base.me,
                    idx,
                    input.expect("enabled reposition diagnostic captured candidate input"),
                    cp.attacker,
                    cp.attacker_position,
                    cp.target,
                    cp.target_position,
                    cp.target_direction,
                    cp.change_adversary,
                    cp.change_position,
                    cp.line_position,
                    cp.left_neighbour,
                    cp.right_neighbour,
                    cp.bonus,
                    cp.line_jump,
                    score,
                );
            }
            if score > best_score {
                best_score = score;
                best_index = idx;
            }
        }

        let mut best = possible.swap_remove(best_index);
        // If the winning candidate doesn't already carry a line jump,
        // ask `IsTableSwordfightNeeded` whether crossing a table/jump
        // line is required to reach the target. Without this, the
        // caller's cached `my_line_jump` stays None when the best
        // candidate is the initial "stay put" entry, and the
        // table-approach path is skipped even when needed.
        //
        // The reference asks about the WINNING CANDIDATE'S target, not
        // about the primary target:
        // `pBestCombatPosition->pLineJump = mpMe->IsTableSwordfightNeeded(
        //  pBestCombatPosition->pTarget )`
        // (`original-code/RHartificialmalignity.cpp:15326`). The two differ
        // whenever the winner is a `bChangeAdversary` candidate, which is
        // exactly the case that installs a cross-sector line jump: asking
        // about the (same-sector) primary target answers `NULL` and leaves
        // `mpMyLineJump` unset for every later frame, so
        // `ProposeCombatPositionsAround` then generates the 16-direction
        // surround ring the Original replaces with its jump-line branches.
        // `target_position` is the candidate's `posTargetPosition`, i.e.
        // `Position(pTarget)` captured in this same call.
        if best.line_jump.is_none()
            && best.target != 0
            && let Some(grid) = grid
        {
            let my_max_range = self
                .find_fighter(self.base.me, tick)
                .map(|f| f.sword_range_maximal)
                .unwrap_or(self.sword_range);
            let victim = best.target_position;
            best.line_jump = crate::engine::melee::table_swordfight_jump_line(
                grid,
                ctx.position.sector.map(i16::from).unwrap_or(-1),
                victim.sector.map(i16::from).unwrap_or(-1),
                crate::coordinates::MapPoint::new(victim.x, victim.y),
                my_max_range as f32,
            );
        }
        if debug_reposition {
            eprintln!(
                "[RECONSIDER_POSITION] frame={} owner={} chosen={} score={} result=(attacker={} attacker_position={:?} target={} target_position={:?} target_direction={} change_adversary={} change_position={} line_position={} left={} right={} bonus={} line_jump={:?})",
                ctx.frame,
                self.base.me,
                best_index,
                best_score,
                best.attacker,
                best.attacker_position,
                best.target,
                best.target_position,
                best.target_direction,
                best.change_adversary,
                best.change_position,
                best.line_position,
                best.left_neighbour,
                best.right_neighbour,
                best.bonus,
                best.line_jump,
            );
        }

        // Update neighbour cache from the chosen position. Eager direct
        // writes give other code in this tick the new values; the queued
        // cross-NPC actions perform the full reciprocal cleanup at drain
        // time (UpdateLeftCombatNeighbour / UpdateRightCombatNeighbour),
        // including clearing the old neighbours' back-pointers and the
        // new neighbours' stale right/left chains.
        let old_left = self.left_combat_neighbour;
        let old_right = self.right_combat_neighbour;
        self.left_combat_neighbour = best.left_neighbour;
        self.right_combat_neighbour = best.right_neighbour;
        self.base.outbox.reentrant.cross_npc_actions.push(
            CrossNpcAction::UpdateLeftCombatNeighbour {
                target: self.base.me,
                old_left,
                new_left: best.left_neighbour,
            },
        );
        self.base.outbox.reentrant.cross_npc_actions.push(
            CrossNpcAction::UpdateRightCombatNeighbour {
                target: self.base.me,
                old_right,
                new_right: best.right_neighbour,
            },
        );

        best
    }
}

fn drunk_combat_freezes(sim: &crate::sim_rng::SimulationContext, blood_alcohol: u8) -> bool {
    crate::sim_rng::u16(sim, crate::sim_rng::RngSite::DrunkCombatFreeze, 0..100)
        <= blood_alcohol as u16
        || crate::sim_rng::u16(sim, crate::sim_rng::RngSite::DrunkCombatFreeze, 0..100)
            <= blood_alcohol as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{EntityId, Posture, SoldierId};
    use crate::sight_obstacle::{ObstaclePoint, SharedSightObstacles, SightObstacle};
    use crate::sim_rng::{RngSite, SimulationContext, with_draw_trace};

    fn position(x: f32, y: f32) -> Position {
        Position {
            x,
            y,
            sector: None,
            level: 0,
        }
    }

    #[test]
    fn vector_derived_phalanx_slot_retains_anchor_sector_identity() {
        use crate::fast_find_grid::SectorIndex;

        let current = Position {
            x: 20.0,
            y: 20.0,
            sector: crate::position_interface::SectorHandle::new(18)
                .map(|sector| sector.with_arena_index(SectorIndex::new(40).unwrap())),
            level: 0,
        };
        let anchor = Position {
            x: 80.0,
            y: 20.0,
            sector: crate::position_interface::SectorHandle::new(18)
                .map(|sector| sector.with_arena_index(SectorIndex::new(41).unwrap())),
            level: 0,
        };
        let derived = Position {
            x: anchor.x - 20.0,
            y: anchor.y,
            ..anchor
        };

        assert_eq!(derived.sector.unwrap().arena_index(), SectorIndex::new(41));
        assert!(inherited_position_crosses_sector_identity(
            &current, &derived
        ));
        assert!(!inherited_position_crosses_sector_identity(
            &anchor, &derived
        ));
    }

    #[test]
    fn nescafe_phalanx_uses_raw_body_distance_then_ai_facing_chain_anchors() {
        use crate::position_interface::SectorHandle;

        // Schema-16 seed 1,000,000, Nescafe Restart, frame 1187. Original
        // Soldier co160 (Rust 129) is passing door 95. AI `Position()` snaps
        // it to the sector-0 endpoint (1307,2245), but its raw body is still
        // near (1306.8123,2262.1873). Original MaxNormDistance reads the raw
        // body and gets 43, so co160 beats co163/Rust132 at distance 58.
        // Measuring the snapped point changes the ordering. After selection,
        // FindPhalanxPlace deliberately returns to AI-facing current/seek
        // positions and derives the slot from the sector-0 chain end.
        let sector_18 = SectorHandle::new(18).unwrap();
        let sector_0 = SectorHandle::new(0).unwrap();
        let mut ai = EnemyAi::new(128);
        ai.base.me = 128;
        let mut tick = AiPerTickData::stub();
        tick.fighter_registry.push(FighterSnapshot {
            handle: 128,
            is_friendly: true,
            is_shield_bearer: true,
            position: Position {
                x: 1263.1832,
                y: 2281.7712,
                sector: Some(sector_18),
                level: 0,
            },
            raw_position: Position {
                x: 1263.1832,
                y: 2281.7712,
                sector: Some(sector_18),
                level: 0,
            },
            ..FighterSnapshot::default()
        });
        tick.fighter_registry.push(FighterSnapshot {
            handle: 132,
            is_friendly: true,
            is_shield_bearer: true,
            current_substate: Substate::AttackingProtectingWithShield as u32,
            left_combat_neighbour: 130,
            right_combat_neighbour: 129,
            position: Position {
                x: 1322.0,
                y: 2276.0,
                sector: Some(sector_18),
                level: 0,
            },
            raw_position: Position {
                x: 1322.0,
                y: 2276.0,
                sector: Some(sector_18),
                level: 0,
            },
            ..FighterSnapshot::default()
        });
        tick.fighter_registry.extend([
            FighterSnapshot {
                handle: 129,
                is_friendly: true,
                is_shield_bearer: true,
                current_substate: Substate::AttackingRunningToPhalanx as u32,
                left_combat_neighbour: 133,
                shield_bearer_direction: 8,
                position: Position {
                    x: 1307.0,
                    y: 2245.0,
                    sector: Some(sector_0),
                    level: 0,
                },
                raw_position: Position {
                    x: 1306.8123,
                    y: 2262.1873,
                    sector: Some(sector_18),
                    level: 0,
                },
                shield_bearer_seek_position: Position {
                    x: 1310.3472,
                    y: 2209.295,
                    sector: Some(sector_0),
                    level: 0,
                },
                ..FighterSnapshot::default()
            },
            FighterSnapshot {
                handle: 133,
                is_friendly: true,
                is_shield_bearer: true,
                current_substate: Substate::AttackingPhalanx as u32,
                left_combat_neighbour: 130,
                right_combat_neighbour: 129,
                position: Position {
                    x: 1337.8904,
                    y: 2213.9985,
                    sector: Some(sector_0),
                    level: 0,
                },
                raw_position: Position {
                    x: 1337.8904,
                    y: 2213.9985,
                    sector: Some(sector_0),
                    level: 0,
                },
                direction: 10,
                ..FighterSnapshot::default()
            },
            FighterSnapshot {
                handle: 130,
                is_friendly: true,
                is_shield_bearer: true,
                current_substate: Substate::AttackingPhalanx as u32,
                right_combat_neighbour: 133,
                position: Position {
                    x: 1359.2433,
                    y: 2211.426,
                    sector: Some(sector_0),
                    level: 0,
                },
                raw_position: Position {
                    x: 1359.2433,
                    y: 2211.426,
                    sector: Some(sector_0),
                    level: 0,
                },
                direction: 10,
                ..FighterSnapshot::default()
            },
        ]);
        let ctx = AiContext {
            position: tick.fighter_registry[0].position,
            ..AiContext::default()
        };

        assert_eq!(ai.get_nearest_free_shield_bearer(&ctx, &tick), Some(129));
        let (slot, _, left, right, crosses_sector) = ai
            .find_phalanx_place(&ctx, &tick, None)
            .expect("nearby protecting shield bearer provides a slot");

        assert_eq!(slot.sector, Some(sector_0));
        assert!(crosses_sector);
        // With no grid fixture both slots are authorized, so this focused
        // control chooses the closer right slot. Crucially it is derived from
        // co160's future seek anchor, not its raw body or door endpoint.
        assert_eq!((left, right), (129, 0));
        assert_eq!(slot.x.to_bits(), 1285.3472_f32.to_bits());
        assert_eq!(slot.y.to_bits(), 2209.295_f32.to_bits());
    }

    #[test]
    fn shield_danger_point_uses_raw_target_position_during_door_pass() {
        // Schema-14 seed 1000000, SuN1Sh1nE Profile_004/Savegame_013
        // replay-008 frame 1173. PC 171's AI Position() is the door endpoint,
        // but RefreshArrowProtection stores the PC element's raw position in
        // RHFIELD_SHIELD_DANGER_POINT. The two points face different sectors.
        let target = FighterSnapshot {
            handle: 171,
            position: position(572.0, 2360.0),
            raw_position: position(578.74, 2388.01),
            elevation: 85.44939,
            ..FighterSnapshot::default()
        };

        let (danger_position, danger_elevation) =
            shield_danger_point(Some(&target), None).expect("fighter has a raw position");

        assert_eq!(danger_position, target.raw_position);
        assert_eq!(danger_elevation, target.elevation);
        assert_ne!(danger_position, target.position);
    }

    #[test]
    fn combat_neighbour_ranking_uses_literal_body_position_during_door_pass() {
        // Schema-16 seed 2,000,000, linux2/Profile_002/Savegame_003,
        // replay-037 frame 15008. Soldier 130's AI Position() is the nearby
        // door endpoint (1173, 1849), while its literal body remains at
        // (1159.50, 1829.36). RHArtificialIntelligence::SquareDistance uses
        // the latter, making Soldier 178 the nearest right neighbour.
        let mut ai = EnemyAi::new(186);
        ai.base.list_us = vec![186, 130, 178];

        let mut tick = AiPerTickData::stub();
        tick.fighter_registry.extend([
            FighterSnapshot {
                handle: 130,
                position: position(1173.0, 1849.0),
                raw_position: position(1159.4979, 1829.3608),
                elevation: 1.4621211,
                direction: 7,
                is_soldier: true,
                rank: ProfileRank::Soldier,
                ..FighterSnapshot::default()
            },
            FighterSnapshot {
                handle: 178,
                position: position(1220.2673, 1886.527),
                raw_position: position(1220.2673, 1886.527),
                direction: 8,
                is_soldier: true,
                rank: ProfileRank::Soldier,
                ..FighterSnapshot::default()
            },
        ]);
        let ctx = AiContext {
            position: position(1231.5779, 1845.2806),
            direction: 8,
            ..AiContext::default()
        };

        assert_eq!(ai.propose_left_and_right_neighbour(&ctx, &tick), (0, 178));
    }

    #[test]
    fn phalanx_nearest_enemy_truncates_distance_before_tie_breaking() {
        assert_eq!(
            nearest_phalanx_enemy_index([(0, 120.9), (1, 120.1), (2, 121.0)]),
            Some(0),
            "Original UWORD narrowing keeps the first enemy within a shared integer bucket"
        );
        assert_eq!(
            nearest_phalanx_enemy_index([(0, 120.9), (1, 119.9)]),
            Some(1)
        );
    }

    #[test]
    fn already_in_cover_position_does_not_require_reachability() {
        // nicouzouf Savegame_010 replay-012 frame 515: the archer is already
        // behind Soldier 58. The direct cover corridor is obstructed, but
        // Original only compares this ideal offset with the archer's current
        // position and therefore keeps the relationship while shooting.
        let mut tick = AiPerTickData::stub();
        tick.fighter_registry.push(FighterSnapshot {
            handle: 58,
            position: position(1144.9557, 408.22668),
            direction: 7,
            current_substate: Substate::AttackingProtectingWithShield as u32,
            ..FighterSnapshot::default()
        });
        let archer_position = position(1123.7424, 396.0593);
        let cover = EnemyAi::default()
            .shield_bearer_cover_position(58, &tick)
            .expect("linked shield bearer has an ideal cover position");

        assert!(
            max_norm(pos_diff(&archer_position, &cover)) < archer::COVER_POINT_TOLERANCE as f32
        );
    }

    #[test]
    fn shield_bearer_cover_preserves_original_aspect_then_distance_rounding() {
        // Schema-16 seed 2,000,000, linux3/Profile_003/Savegame_029,
        // replay-017 frame 12826. Original SetSector0to15 first multiplies
        // sector 10's Y component by ASPECT_RATIO, then operator*= applies
        // distance 30. Reassociating those products raises the destination Y
        // from bits 0x4268_b9b6 to 0x4268_b9b7 and eventually changes a
        // bit-exact visibility endpoint by two ULPs.
        let mut tick = AiPerTickData::stub();
        tick.fighter_registry.push(FighterSnapshot {
            handle: 82,
            position: position(1072.624755859375, 70.3487548828125),
            direction: 10,
            current_substate: Substate::AttackingProtectingWithShield as u32,
            ..FighterSnapshot::default()
        });

        let cover = EnemyAi::default()
            .shield_bearer_cover_position(82, &tick)
            .expect("protecting shield bearer has a cover position");

        assert_eq!(cover.x.to_bits(), 0x4488_bad1);
        assert_eq!(cover.y.to_bits(), 0x4268_b9b6);
        assert_eq!(cover.x, 1093.8380126953125);
        assert_eq!(cover.y, 58.181358337402344);
    }

    #[test]
    fn nearest_shield_bearer_includes_inactive_running_to_phalanx_soldier() {
        // nicouzouf Profile_001 Savegame_045 replay-014 frame 1054. Soldier
        // 62 is inactive/script-locked while running to its phalanx slot, but
        // Original's global soldier scan still chooses it over Soldier 60.
        let ai = EnemyAi::new(66);
        let mut tick = AiPerTickData::stub();
        tick.fighter_registry.push(FighterSnapshot {
            handle: 66,
            is_friendly: true,
            is_archer_unit: true,
            ..FighterSnapshot::default()
        });
        let active_bearer = FighterSnapshot {
            handle: 60,
            position: position(669.8923, 746.43475),
            is_friendly: true,
            is_able_to_fight: true,
            is_shield_bearer: true,
            current_substate: Substate::AttackingPhalanx as u32,
            ..FighterSnapshot::default()
        };
        let inactive_bearer = FighterSnapshot {
            handle: 62,
            position: position(484.0, 701.0),
            is_friendly: true,
            is_able_to_fight: false,
            is_shield_bearer: true,
            current_substate: Substate::AttackingRunningToPhalanx as u32,
            ..FighterSnapshot::default()
        };
        tick.fighter_registry.push(active_bearer.clone());
        tick.fighter_registry.push(inactive_bearer);
        // The radius-limited/able-only list omits Soldier 62, which is why it
        // cannot be the backing collection for this Original global scan.
        tick.nearby_fighters.push(active_bearer);
        let ctx = AiContext {
            position: position(549.00867, 517.99506),
            ..AiContext::default()
        };

        assert_eq!(ai.get_nearest_free_shield_bearer(&ctx, &tick), Some(62));
    }

    #[test]
    fn arrow_protection_counts_inactive_seeking_orphan_from_complete_registry() {
        // linux3 Profile_003 Savegame_071 replay-012 frame 5208: Soldiers
        // 250..255 are inactive but remain Seeking orphan archers within the
        // Original 500-unit camp-soldier scan. The swordfight-oriented nearby
        // cache excludes them through IsAbleToFight and must not define this
        // decision's candidate domain.
        let ai = EnemyAi::new(219);
        let ctx = AiContext {
            position: position(1_520.0, 900.0),
            ..AiContext::default()
        };
        let orphan = FighterSnapshot {
            handle: 250,
            position: position(1_328.0, 1_033.0),
            raw_position: position(1_328.0, 1_033.0),
            elevation: 0.0,
            is_friendly: true,
            is_able_to_fight: false,
            is_archer_unit: true,
            ai_state: AiState::Seeking,
            shield_bearer_before_me: 0,
            is_tower_guard: false,
            ..FighterSnapshot::default()
        };
        let mut tick = AiPerTickData::stub();
        // The registry always leads with the scanning soldier's own entry;
        // `SquareDistance` measures from its raw element position.
        tick.fighter_registry.push(FighterSnapshot {
            handle: 219,
            position: position(1_520.0, 900.0),
            raw_position: position(1_520.0, 900.0),
            elevation: 0.0,
            is_friendly: true,
            ai_state: AiState::Attacking,
            ..FighterSnapshot::default()
        });
        tick.fighter_registry.push(orphan.clone());
        assert_eq!(
            ai.number_of_nearby_archers_who_need_protection(&ctx, &tick),
            1,
            "inactive is not an Original admission gate"
        );

        tick.fighter_registry[1].ai_state = AiState::Default;
        assert_eq!(
            ai.number_of_nearby_archers_who_need_protection(&ctx, &tick),
            0,
            "the explicit AI-state gate still rejects an inactive Default archer"
        );

        tick.fighter_registry[1] = FighterSnapshot {
            position: position(2_100.0, 900.0),
            raw_position: position(2_100.0, 900.0),
            ..orphan
        };
        assert_eq!(
            ai.number_of_nearby_archers_who_need_protection(&ctx, &tick),
            0,
            "the complete registry must still obey the strict 500-unit radius"
        );
    }

    #[test]
    fn arrow_protection_sees_reciprocal_unlink_emitted_by_same_think() {
        // Schema-12 SuN1Sh1nE Savegame_013 replay-005 frame 2165. Breaking
        // the phalanx makes Soldier 81 choose Observe. Original SetState
        // synchronously unlinks archer 86 before ReconsiderEnemyApproach
        // scans for archers needing protection, so 86 is already orphaned.
        // Rust's reciprocal write is queued until the owner-boundary drain;
        // this tactical scan must overlay that ordered write.
        let mut ai = EnemyAi::new(81);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingPhalanx;
        ai.archer_behind_me = 86;
        ai.set_state(AiState::Attacking, Substate::AttackingApproachToObserve);

        assert_eq!(ai.archer_behind_me, 0);
        assert!(
            ai.base
                .outbox
                .reentrant
                .cross_npc_actions
                .iter()
                .any(|action| matches!(
                    action,
                    CrossNpcAction::SetShieldBearerBeforeMe {
                        target: 86,
                        shield_bearer: 0
                    }
                ))
        );

        let owner_position = position(900.0, 2500.0);
        let mut tick = AiPerTickData::stub();
        tick.fighter_registry.push(FighterSnapshot {
            handle: 81,
            position: owner_position,
            raw_position: owner_position,
            is_friendly: true,
            is_shield_bearer: true,
            ai_state: AiState::Attacking,
            current_substate: Substate::AttackingPhalanx as u32,
            archer_behind_me: 86,
            ..FighterSnapshot::default()
        });
        tick.fighter_registry.push(FighterSnapshot {
            handle: 86,
            position: position(920.0, 2510.0),
            raw_position: position(920.0, 2510.0),
            is_friendly: true,
            is_archer_unit: true,
            ai_state: AiState::Attacking,
            shield_bearer_before_me: 81,
            ..FighterSnapshot::default()
        });
        let ctx = AiContext {
            position: owner_position,
            ..AiContext::default()
        };

        assert_eq!(
            ai.number_of_nearby_archers_who_need_protection(&ctx, &tick),
            1
        );
    }

    #[test]
    fn phalanx_advance_uses_original_aspect_aware_normalization() {
        // Schema-14 Savegame_034/replay-009, frame 34182. Soldier 52 is
        // the center of the three-man phalanx and PC 167 is its target.
        // These are the literal results of SBGeoVector2D::Normalize and
        // GetNormal(true, ASPECT_RATIO) in Original.
        let center = position(720.15155, 2198.4492);
        let target = position(967.95605, 2068.5835);
        let (forward, right) = phalanx_advance_vectors(pos_diff(&target, &center));

        assert!((forward.0 - 51.67761).abs() < 0.0001);
        assert!((forward.1 - -27.082436).abs() < 0.0001);
        assert!((right.0 - 16.863138).abs() < 0.0001);
        assert!((right.1 - 10.586092).abs() < 0.0001);

        let new_center = (center.x + forward.0, center.y + forward.1);
        let left_slot = (new_center.0 - right.0, new_center.1 - right.1);
        let right_slot = (new_center.0 + right.0, new_center.1 + right.1);
        assert!((left_slot.0 - 754.966).abs() < 0.001);
        assert!((left_slot.1 - 2160.7808).abs() < 0.001);
        assert!((right_slot.0 - 788.6923).abs() < 0.001);
        assert!((right_slot.1 - 2181.953).abs() < 0.001);
    }

    fn enemy(handle: HumanHandle, x: f32) -> PhalanxEnemySnapshot {
        PhalanxEnemySnapshot {
            handle,
            position: position(x, 0.0),
            direction: 4,
            posture: Posture::Upright,
            elevation: 0.0,
            is_rider: false,
            active: true,
            able_to_fight: true,
            dead: false,
            unconscious: false,
            friend: false,
            in_building: false,
            obstacle: None,
        }
    }

    fn member(
        handle: HumanHandle,
        radius: f32,
        current_them_list: Vec<PhalanxEnemySnapshot>,
        detectable_enemies: Vec<PhalanxEnemySnapshot>,
    ) -> PhalanxMemberThemList {
        PhalanxMemberThemList {
            handle,
            entity: EntityId::Soldier(SoldierId(handle)),
            current_them_list,
            detectable_enemies,
            position: position(0.0, 0.0),
            direction: 4,
            posture: Posture::Upright,
            elevation: 0.0,
            is_rider: false,
            active: true,
            in_building: false,
            view_radius: radius as u16,
            view_direction: [1.0, 0.0],
            real_half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
            sq_view_radius: radius * radius,
        }
    }

    fn opaque_wall() -> SightObstacle {
        let mut wall = SightObstacle::new_default(0);
        wall.obstacle_points = vec![
            ObstaclePoint {
                x: 95.0,
                y: -10.0,
                z_top: 80.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 105.0,
                y: -10.0,
                z_top: 80.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 105.0,
                y: 10.0,
                z_top: 80.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 95.0,
                y: 10.0,
                z_top: 80.0,
                z_bottom: 0.0,
            },
        ];
        wall.top_plane_points = [
            [95.0, -10.0, 80.0],
            [105.0, -10.0, 80.0],
            [95.0, 10.0, 80.0],
        ];
        wall.bottom_plane_points = [[95.0, -10.0, 0.0], [105.0, -10.0, 0.0], [95.0, 10.0, 0.0]];
        wall.rebuild_geometry();
        wall
    }

    #[test]
    fn phalanx_uses_each_members_heterogeneous_view_radius() {
        let target = enemy(9, 150.0);
        let leftmost = member(1, 300.0, Vec::new(), Vec::new());
        let right = member(2, 100.0, vec![target], Vec::new());
        let mut merged = Vec::new();

        let leftmost_kept: Vec<&PhalanxEnemySnapshot> = leftmost.current_them_list.iter().collect();
        let right_kept: Vec<&PhalanxEnemySnapshot> = right.current_them_list.iter().collect();
        let ctx = AiContext::default();
        append_phalanx_member_enemies(&mut merged, &leftmost, &leftmost_kept, &ctx);
        append_phalanx_member_enemies(&mut merged, &right, &right_kept, &ctx);

        assert!(merged.is_empty());
    }

    #[test]
    fn inactive_phalanx_member_is_traversed_without_detecting_enemies() {
        let target = enemy(9, 40.0);
        let mut inactive = member(2, 300.0, vec![target.clone()], vec![target]);
        inactive.active = false;
        let kept: Vec<&PhalanxEnemySnapshot> = inactive.current_them_list.iter().collect();
        let mut merged = Vec::new();

        append_phalanx_member_enemies(&mut merged, &inactive, &kept, &AiContext::default());

        assert!(
            merged.is_empty(),
            "Original follows the inactive neighbour link but both detection variants reject its viewer before LOS"
        );
    }

    #[test]
    fn phalanx_rejects_occluded_persistent_and_detectable_entries() {
        let target = enemy(9, 200.0);
        let member = member(2, 300.0, vec![target.clone()], vec![target]);

        let mut clear_merged = Vec::new();
        let kept: Vec<&PhalanxEnemySnapshot> = member.current_them_list.iter().collect();
        let clear_ctx = AiContext::default();
        append_phalanx_member_enemies(&mut clear_merged, &member, &kept, &clear_ctx);
        assert_eq!(clear_merged, vec![9]);

        let obstacles = vec![opaque_wall()];
        let active = vec![true];
        let blocked_ctx = AiContext {
            sight_obstacles: SharedSightObstacles {
                static_obstacles: std::sync::Arc::new(obstacles),
                dynamic_obstacles: std::sync::Arc::new(Vec::new()),
                static_active: std::sync::Arc::new(active),
            },
            ..AiContext::default()
        };
        let mut blocked_merged = Vec::new();
        append_phalanx_member_enemies(&mut blocked_merged, &member, &kept, &blocked_ctx);
        assert!(blocked_merged.is_empty());
    }

    #[test]
    fn phalanx_night_detection_orders_light_rays_before_target_los() {
        let mut target = enemy(9, 600.0);
        target.position.y = 500.0;
        let mut member = member(2, 500.0, Vec::new(), vec![target]);
        member.position = position(500.0, 500.0);

        let mut ctx = AiContext {
            is_night_or_fog: true,
            ..AiContext::default()
        };
        let fast_grid = std::sync::Arc::make_mut(&mut ctx.fast_grid);
        fast_grid.size_map(20, 20);
        fast_grid.allocate_layers(1);
        let barycentres = [(750.0, 500.0), (760.0, 510.0), (770.0, 490.0)];
        for (index, &(x, y)) in barycentres.iter().enumerate() {
            let points = vec![
                crate::coordinates::MapPoint::new(x - 4.0, y - 4.0),
                crate::coordinates::MapPoint::new(x + 4.0, y - 4.0),
                crate::coordinates::MapPoint::new(x + 4.0, y + 4.0),
                crate::coordinates::MapPoint::new(x - 4.0, y + 4.0),
            ];
            let mut bounding_box = crate::coordinates::MapBBox::new();
            for &point in &points {
                bounding_box.expand_point(point);
            }
            fast_grid.add_sector(
                crate::fast_find_grid::GridSector {
                    points,
                    bounding_box,
                    sector_type: crate::sector::SectorType::SHADOW,
                    layer: 0,
                    sector_number: crate::sector::SectorNumber::new(index as i16 + 1),
                    door_index: None,
                    lift_type: None,
                    lift_direction: 0,
                    force_crouched: false,
                    building_index: None,
                    low_exit_point: None,
                    high_exit_point: None,
                    lowest_door_index: None,
                    jump_line_indices: Vec::new(),
                    gate_indices: Vec::new(),
                    underlying_sector: None,
                },
                0,
            );
            std::sync::Arc::make_mut(&mut fast_grid.level)
                .shadow_data
                .insert(
                    index as u32,
                    crate::sector::ShadowData {
                        barycentre_2d: crate::coordinates::MapPoint::new(x, y),
                        barycentre_3d_x: x,
                        barycentre_3d_y: y,
                        barycentre_3d_z: 45.0,
                        radius: 4.0,
                    },
                );
        }

        crate::sight_obstacle::begin_parity_visibility_capture();
        let mut merged = Vec::new();
        append_phalanx_member_enemies(&mut merged, &member, &[], &ctx);
        let queries = crate::sight_obstacle::take_parity_visibility_capture();

        assert_eq!(merged, vec![9]);
        assert_eq!(queries.len(), 4);
        assert_eq!(
            queries
                .iter()
                .map(|query| query.destination)
                .collect::<Vec<_>>(),
            vec![
                [750.0, 500.0, 45.0],
                [760.0, 510.0, 45.0],
                [770.0, 490.0, 45.0],
                [600.0, 500.0, 45.0],
            ]
        );
        assert!(queries.iter().all(|query| query.result));
        let cached_radius = ctx.compute_view_radius_cached(member.entity, None, || {
            panic!("the phalanx member's ground radius should remain cached for its caller")
        });
        assert!(cached_radius > 0.0);
    }

    #[test]
    fn sober_drunk_combat_gate_preserves_original_draws_and_short_circuit() {
        let two_draw_seed = (0..10_000)
            .find(|seed| {
                let sim = SimulationContext::with_seed(*seed);
                crate::sim_rng::u16(&sim, RngSite::DrunkCombatFreeze, 0..100) != 0
                    && crate::sim_rng::u16(&sim, RngSite::DrunkCombatFreeze, 0..100) != 0
            })
            .expect("find a seed whose first two drunk gates do not freeze a sober soldier");
        let sim = SimulationContext::with_seed(two_draw_seed);
        let (freezes, trace) = with_draw_trace(|| drunk_combat_freezes(&sim, 0));
        assert!(!freezes);
        assert_eq!(
            trace,
            vec![RngSite::DrunkCombatFreeze, RngSite::DrunkCombatFreeze],
            "a sober soldier must still consume both Original drunk gates"
        );

        let short_circuit_seed = (0..10_000)
            .find(|seed| {
                let sim = SimulationContext::with_seed(*seed);
                crate::sim_rng::u16(&sim, RngSite::DrunkCombatFreeze, 0..100) == 0
            })
            .expect("find a seed whose first drunk gate freezes a sober soldier");
        let sim = SimulationContext::with_seed(short_circuit_seed);
        let (freezes, trace) = with_draw_trace(|| drunk_combat_freezes(&sim, 0));
        assert!(freezes);
        assert_eq!(
            trace,
            vec![RngSite::DrunkCombatFreeze],
            "a successful first gate must preserve Original || short-circuiting"
        );
    }

    #[test]
    fn swordfight_range_checks_use_original_uword_truncation() {
        assert_eq!(original_uword_norm((90.7, 0.0)), 90);
        assert_eq!(original_uword_norm((91.0, 0.0)), 91);
        assert!(original_uword_norm((90.7, 0.0)) <= 90);
    }

    #[test]
    fn swordfight_facing_guard_uses_ground_positions_before_rng() {
        // Schema-14 task 168 frame 2155: projected map positions misleadingly
        // put PC252 in Soldier137's facing sector because their elevations
        // differ. Original GetPositionGround() puts the PC to the east, so
        // ReconsiderSwordfight returns before its combat RNG gates.
        let soldier = position(1720.6782, 1984.8649);
        let pc = position(1749.6063, 1954.4141);
        let soldier_elevation = 17.4139;
        let pc_elevation = 45.0639;

        let projected_sector = vec_to_sector(pc.x - soldier.x, pc.y - soldier.y);
        assert_eq!(projected_sector, 1);
        assert_eq!((1_i32 + 16 - projected_sector as i32) % 16, 0);
        let ground_sector = vec_to_sector(
            pc.x - soldier.x,
            (pc.y + pc_elevation) - (soldier.y + soldier_elevation),
        );
        assert_eq!(ground_sector, 4);
        assert_eq!((1_i32 + 16 - ground_sector as i32) % 16, 13);
        assert!(!is_facing_swordfight_target(
            &soldier,
            soldier_elevation,
            1,
            &pc,
            pc_elevation,
        ));

        let sim = SimulationContext::with_seed(0);
        let (_, trace) = with_draw_trace(|| {
            if is_facing_swordfight_target(&soldier, soldier_elevation, 1, &pc, pc_elevation) {
                let _ = drunk_combat_freezes(&sim, 0);
            }
        });
        assert!(
            trace.is_empty(),
            "the facing return must precede combat RNG"
        );
    }

    #[test]
    fn swordfight_facing_guard_uses_live_position_during_door_pass() {
        // Schema-14 Nescafe Profile_003/Savegame_001 replay-012 frame 1448:
        // Position(PC252) forecasts the far side of door 95, but Original's
        // facing guard reads GetPositionGround() and still sees the live PC.
        let soldier = position(1355.0133, 2248.718);
        let live_pc = position(1307.6046, 2248.1819);
        let forecast_pc = position(1304.0, 2276.0);
        let elevation = 45.0;
        let primary = FighterSnapshot {
            handle: 252,
            position: forecast_pc,
            elevation,
            ..FighterSnapshot::default()
        };
        let mut tick = AiPerTickData::stub();
        tick.primary_target_snapshot_handle = 252;
        tick.primary_target_live_position = Some(live_pc);

        assert!(!is_facing_swordfight_target(
            &soldier,
            elevation,
            12,
            &primary.position,
            primary.elevation,
        ));
        assert!(is_facing_swordfight_target(
            &soldier,
            elevation,
            12,
            swordfight_facing_target_position(&primary, &tick),
            primary.elevation,
        ));
    }

    #[test]
    fn swordfight_facing_guard_rejects_stale_live_position_after_target_refresh() {
        let refreshed_primary = FighterSnapshot {
            handle: 131,
            position: position(1400.0, 2100.0),
            ..FighterSnapshot::default()
        };
        let mut tick = AiPerTickData::stub();
        tick.primary_target_snapshot_handle = 252;
        tick.primary_target_live_position = Some(position(1307.6046, 2248.1819));

        assert_eq!(
            swordfight_facing_target_position(&refreshed_primary, &tick),
            &refreshed_primary.position,
            "a refreshed principal opponent must not inherit the old target's live geometry"
        );
    }

    fn lost_enemy_reconsider_fixture(company_number: u16) -> (EnemyAi, AiContext, AiPerTickData) {
        const OWNER: u32 = 21;
        const TARGET: u32 = 20;

        let target = crate::element::Entity::Pc(crate::element::ActorPc {
            element: crate::element::ElementData {
                kind: crate::element::ElementKind::ActorPc,
                posture: crate::element::Posture::Upright,
                ..Default::default()
            },
            actor: Default::default(),
            human: Default::default(),
            pc: Default::default(),
        });
        let mut target_view = crate::ai_entity_view::entity_view_from_entity(
            &target,
            51,
            false,
            None,
            None,
            crate::order::OrderType::NonanimationEnd,
        );
        target_view.position = position(100.0, 0.0);
        target_view.camp = crate::element::Camp::Royalists;
        target_view.active = false;

        let mut views = crate::ai_entity_view::AiEntityViewMap::new();
        views.insert(TARGET, target_view);
        let ctx = AiContext {
            position: position(0.0, 0.0),
            self_is_active: true,
            camp: crate::element::Camp::Lacklandists,
            is_swordfighting: true,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };
        let mut tick = AiPerTickData::stub();
        tick.fighter_registry.push(FighterSnapshot {
            handle: OWNER,
            principal_opponent: TARGET,
            is_friendly: true,
            ..FighterSnapshot::default()
        });
        tick.primary_target_snapshot_handle = TARGET;
        tick.primary_target_is_pc = true;
        tick.primary_target_forecast = Some(crate::ai::PreparedForecastDestination::fixed(
            position(0.0, 100.0),
            4,
        ));

        let mut ai = EnemyAi::new(OWNER);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingSwordfight;
        ai.base.primary_target = TARGET;
        ai.company_number = company_number;
        (ai, ctx, tick)
    }

    #[test]
    fn lost_enemy_overview_faces_live_target_not_forecast_destination() {
        // randomguy Profile_004/Savegame_030 replay-014 frame 3456:
        // Original forecasts the missed PC for possible pursuit, but the
        // no-follow branch snaps Soldier 21 toward the PC's current Position.
        let (mut ai, ctx, tick) = lost_enemy_reconsider_fixture(100);
        let sim = SimulationContext::with_seed(0);
        ai.reconsider_swordfight(
            &sim,
            false,
            &mut AiGlobalState::default(),
            &ctx,
            &tick,
            None,
        );

        assert_eq!(ai.base.seek_position, position(0.0, 100.0));
        assert_eq!(vec_to_sector(100.0, 0.0), 4);
        assert_eq!(vec_to_sector(0.0, 100.0), 8);
        let direction_prefixes: Vec<_> = ai
            .base
            .outbox
            .reentrant
            .owner_work
            .iter()
            .filter_map(|work| match work {
                crate::ai::AiOwnerWork::ActorEffects(effects) => effects.set_direction_instantly,
                crate::ai::AiOwnerWork::StateChange(change) => change
                    .actor_effects_before_callback
                    .as_ref()
                    .and_then(|effects| effects.set_direction_instantly),
                _ => None,
            })
            .collect();
        assert_eq!(
            direction_prefixes,
            vec![4],
            "the live-target snap must cross exactly one pre-StopAll actor boundary"
        );
        assert_eq!(ai.base.outbox.actor.set_direction_instantly, None);
        assert_eq!(
            ai.base.current_substate,
            Substate::AttackingOverviewLookLeft
        );
    }

    #[test]
    fn lost_enemy_follow_path_keeps_forecast_as_seek_center_without_direction_snap() {
        let (mut ai, ctx, tick) = lost_enemy_reconsider_fixture(0);
        let sim = SimulationContext::with_seed(0);
        ai.reconsider_swordfight(
            &sim,
            false,
            &mut AiGlobalState::default(),
            &ctx,
            &tick,
            None,
        );

        assert_eq!(ai.base.outbox.actor.set_direction_instantly, None);
        assert_eq!(ai.seek_center, position(0.0, 100.0));
        assert_eq!(ai.base.current_state, AiState::Seeking);
    }

    #[test]
    fn lost_enemy_refreshes_forecast_with_swordfight_principal() {
        // Savegame_029 replay-032 frame 5296: the AI member still named PC
        // 168, while GetPrincipalOpponent returned PC 169. The old forecast
        // centered SeekArea on 168 and changed its RNG draw count.
        const OLD_TARGET: u32 = 20;
        const NEW_PRINCIPAL: u32 = 22;
        let (mut ai, mut ctx, mut tick) = lost_enemy_reconsider_fixture(0);
        debug_assert_eq!(ai.base.primary_target, OLD_TARGET);

        let old_view = ctx.entity_view(OLD_TARGET).unwrap().clone();
        let mut new_view = old_view.clone();
        new_view.position = position(300.0, 400.0);
        let mut views = crate::ai_entity_view::AiEntityViewMap::new();
        views.insert(OLD_TARGET, old_view);
        views.insert(NEW_PRINCIPAL, new_view);
        ctx.entity_views = crate::ai_entity_view::shared_entity_views(views);

        tick.fighter_registry[0].principal_opponent = NEW_PRINCIPAL;
        let refreshed_forecast = position(500.0, 600.0);
        tick.enemy_detectable_forecasts.push((
            NEW_PRINCIPAL,
            crate::ai::PreparedForecastDestination::fixed(refreshed_forecast, 7),
        ));

        let sim = SimulationContext::with_seed(0);
        ai.reconsider_swordfight(
            &sim,
            false,
            &mut AiGlobalState::default(),
            &ctx,
            &tick,
            None,
        );

        assert_eq!(ai.missed_pc, NEW_PRINCIPAL);
        assert_eq!(ai.seek_center, refreshed_forecast);
        assert_ne!(ai.seek_center, position(0.0, 100.0));
    }

    #[test]
    fn direct_fighter_lookup_reaches_beyond_nearby_radius_snapshot() {
        let ai = EnemyAi::default();
        let mut tick = AiPerTickData::stub();
        tick.nearby_fighters.push(FighterSnapshot {
            handle: 1,
            ..FighterSnapshot::default()
        });
        tick.fighter_registry.push(FighterSnapshot {
            handle: 2,
            ..FighterSnapshot::default()
        });

        assert_eq!(
            ai.find_fighter(1, &tick).map(|fighter| fighter.handle),
            Some(1)
        );
        assert_eq!(
            ai.find_fighter(2, &tick).map(|fighter| fighter.handle),
            Some(2)
        );
        assert!(ai.find_fighter(3, &tick).is_none());
    }

    #[test]
    fn failed_observation_step_back_panics_without_speaking() {
        let mut ai = EnemyAi::new(91);
        let enemy_pos = position(663.922_5, 2096.012);

        let request = ai.panic_after_failed_observation_step_back(enemy_pos);

        assert_eq!(ai.base.current_state, AiState::Fleeing);
        assert_eq!(ai.base.current_substate, Substate::FleeingPanic);
        assert!(ai.base.outbox.actor.begin_panic.is_none());
        assert_eq!(request.center, Some(enemy_pos));
        assert_eq!(request.runs, parameters_ai::AI_STANDARD_PANIC_RUNS as u8);
        assert!(
            ai.base
                .outbox
                .reentrant
                .owner_work
                .iter()
                .all(|work| !matches!(work, AiOwnerWork::Speech(_))),
            "Original's Panic fallback does not call Flee's Say(REMARK_PANIC)"
        );
    }
}
