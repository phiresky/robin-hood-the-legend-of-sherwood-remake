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
    calculate_opponent_nearest_to_rene, check_straight_movement, det2, dot2,
    evaluate_combat_position_full, get_normal, get_normal_iso, get_normal_right,
    is_any_swordfight_substate, is_observing_combat_substate, is_walking_running_charging_substate,
    iso_norm, iso_normalize, max_norm, pos_diff, sector_to_vector, square_norm, vec_to_sector,
    vec_to_sector_ar,
};
use super::{
    CombatPosition, EnemyAi, FighterSnapshot, PrimaryTargetFlags, ProfileRank, Question, SeekFlags,
    UNDEFINED_DIRECTION, archer, combat, propose_good_step_back_goal,
};

fn phalanx_member_detects_360(
    member: &PhalanxMemberThemList,
    target: &PhalanxEnemySnapshot,
    obstacles: crate::sight_obstacle::ObstacleList<'_>,
) -> bool {
    if member.in_building || !target.active || target.in_building {
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

fn phalanx_member_detects_180(
    member: &PhalanxMemberThemList,
    target: &PhalanxEnemySnapshot,
    obstacles: crate::sight_obstacle::ObstacleList<'_>,
) -> bool {
    if member.in_building || !target.active {
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

    let direction = crate::shadow_polygon::sector_to_direction(member.direction as i16);
    let forward_x = direction[0];
    let forward_y = direction[1] * INVERSE_ASPECT_RATIO;
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
    obstacles: crate::sight_obstacle::ObstacleList<'_>,
) {
    for target in &member.current_them_list {
        if !target.able_to_fight
            || target.friend
            || !phalanx_member_detects_360(member, target, obstacles)
        {
            continue;
        }
        if !merged.contains(&target.handle) {
            merged.push(target.handle);
        }
    }

    for target in &member.detectable_enemies {
        if target.dead
            || target.unconscious
            || !phalanx_member_detects_180(member, target, obstacles)
        {
            continue;
        }
        if !merged.contains(&target.handle) {
            merged.push(target.handle);
        }
    }
}

impl EnemyAi {
    // -----------------------------------------------------------------------
    // Combat-position helpers (used by ProposeGoodCombatPosition)
    // -----------------------------------------------------------------------

    /// Look up a fighter snapshot by handle in the engine-provided cache.
    pub(super) fn find_fighter<'a>(
        &self,
        handle: HumanHandle,
        tick: &'a AiPerTickData,
    ) -> Option<&'a FighterSnapshot> {
        tick.nearby_fighters.iter().find(|f| f.handle == handle)
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
                // SquareDistance(pFriend) stretches Y by
                // INVERSE_ASPECT_RATIO before squaring — isometric
                // squared distance, not Euclidean.
                let v = pos_diff(&snap.position, &me_pos);
                let v_iso = (v.0, v.1 * INVERSE_ASPECT_RATIO);
                let sq = square_norm(v_iso);
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
                    .and_then(|s| grid.level.sectors.get(usize::from(s)))
                    .and_then(|s| SectorHandle::new(u16::from(s.sector_number)))
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

    /// GetNearestFreeShieldBearer. Scans friendly soldiers in the
    /// snapshot for the nearest shield bearer already in (or heading
    /// into) a shield-bearer substate. If the caller is a shield bearer
    /// any protecting shield bearer will do; if the caller is an archer
    /// we only accept shield bearers that don't yet have an archer
    /// behind them.
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

        let me_pos = ctx.position;
        let min_distance = archer::SHIELD_BEARER_MIN_DISTANCE as f32;
        let mut best: HumanHandle = 0;
        let mut best_distance = min_distance;

        for f in &tick.nearby_fighters {
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
            let dist = max_norm(pos_diff(&f.position, &me_pos));
            if dist < best_distance {
                best_distance = dist;
                best = f.handle;
            }
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

        // (1) Search for an archery sector containing the enemy
        let mut found_sector: Option<usize> = None;
        for (i, sector) in global.archery_sectors.iter().enumerate() {
            if !sector.is_full() && sector.is_inside(&primary_pos, ctx.position.level) {
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
    /// `shield_bearer_before_me` link. The reverse link on the shield
    /// bearer (`archer_behind_me`) is reconciled by the engine's
    /// snapshot-building pass so we don't need cross-entity mutation
    /// here.
    pub(super) fn update_shield_bearer_before_me(&mut self, new_sb: HumanHandle) {
        if !self.is_archer() {
            return;
        }
        if new_sb == self.shield_bearer_before_me {
            return;
        }
        self.shield_bearer_before_me = new_sb;
    }

    // UpdateArcherBehindMe is not ported: the reverse link is
    // maintained by the engine's snapshot-building pass (see
    // `archer_behind_me` derivation in `engine/ai.rs`), so no direct
    // setter is required. Mirror of `update_shield_bearer_before_me`
    // above if a future caller needs it.

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
    ) -> Option<(Position, u16, HumanHandle, HumanHandle)> {
        if self.phalanx_aborted {
            return None;
        }
        let nearest = self.get_nearest_free_shield_bearer(ctx, tick)?;

        // Walk left/right to find the end-of-phalanx anchors.
        let left_guy = self.walk_phalanx_end(nearest, true, ctx, tick);
        let right_guy = self.walk_phalanx_end(nearest, false, ctx, tick);

        // Use GetShieldBearerPosition semantics: when the anchor is running
        // to a phalanx slot, use their future seek position + shield bearing
        // direction; when in position, use their current pose.
        let shield_running = Substate::AttackingRunningToPhalanx as u32;
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

        // Left slot: anchor's forward vector, counter-clockwise normal.
        let left_forward = sector_to_vector(left_dir);
        let mut left_side = get_normal(left_forward);
        left_side.0 *= distance;
        left_side.1 *= distance;
        left_side.1 *= ASPECT_RATIO;
        let pos_left = Position {
            x: left_pos.x + left_side.0,
            y: left_pos.y + left_side.1,
            ..left_pos
        };

        // Right slot: anchor's forward vector, clockwise normal.
        let right_forward = sector_to_vector(right_dir);
        let mut right_side = get_normal_right(right_forward);
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
            g.is_straight_movement_authorized(anchor_pt, slot_pt, left_pos.level, &ctx.move_box)
        });
        // The reference passes `left_guy.layer` here (a copy-paste bug);
        // we use `right_pos.level` so the right-side check matches the
        // right anchor when phalanx ends straddle stairs/ramps.
        let right_accessible = grid.is_none_or(|g| {
            let anchor_pt = crate::coordinates::MapPoint::new(right_pos.x, right_pos.y);
            let slot_pt = crate::coordinates::MapPoint::new(pos_right.x, pos_right.y);
            g.is_straight_movement_authorized(anchor_pt, slot_pt, right_pos.level, &ctx.move_box)
        });

        let me_pos = ctx.position;
        let sq_left = square_norm(pos_diff(&pos_left, &me_pos));
        let sq_right = square_norm(pos_diff(&pos_right, &me_pos));

        match (left_accessible, right_accessible) {
            (true, true) => {
                // Strict `<` — ties go to right slot.
                if sq_left < sq_right {
                    Some((pos_left, left_dir, 0, left_guy))
                } else {
                    Some((pos_right, right_dir, right_guy, 0))
                }
            }
            (true, false) => Some((pos_left, left_dir, 0, left_guy)),
            (false, true) => Some((pos_right, right_dir, right_guy, 0)),
            (false, false) => None,
        }
    }

    /// ComputePositionBehindMyShieldBearer. Given an archer caller with
    /// a linked shield bearer, compute the cover position
    /// `DISTANCE_SHIELD_BEARER_ARCHER` behind that shield bearer along
    /// their facing.
    ///
    /// Currently exposed as a helper for the archer
    /// cover-behind-shield-bearer decision path; called from the
    /// `CoverBehindShieldBearer` decision and the "already in cover"
    /// check in `battle_decisions`.
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
        // Step backwards from the shield bearer along their facing.
        let distance = archer::DISTANCE_SHIELD_BEARER_ARCHER as f32;
        let behind = Position {
            x: bearer_pos.x - forward.0 * distance,
            y: bearer_pos.y - forward.1 * distance * ASPECT_RATIO,
            ..bearer_pos
        };
        // Cover line must be unobstructed from the bearer.
        if let Some(g) = grid {
            let bearer_pt = crate::coordinates::MapPoint::new(bearer_pos.x, bearer_pos.y);
            let cover_pt = crate::coordinates::MapPoint::new(behind.x, behind.y);
            if !g.is_straight_movement_authorized(bearer_pt, cover_pt, behind.level, &ctx.move_box)
            {
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
        for f in &tick.nearby_fighters {
            if !f.is_friendly {
                continue;
            }
            let d = pos_diff(&f.position, &ctx.position);
            if square_norm(d) >= consider_sq {
                continue;
            }
            // Filter on AI state ∈ {Seeking, Wondering, Attacking}.
            match f.ai_state {
                AiState::Seeking | AiState::Wondering | AiState::Attacking => {
                    if f.is_archer_unit && f.shield_bearer_before_me == 0 && !f.is_tower_guard {
                        // Orphan archer. No `soldier != me` guard here —
                        // an orphan-archer self counts itself.
                        count += 1;
                    } else if f.is_shield_bearer
                        && f.archer_behind_me == 0
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
        sim: &crate::sim_rng::SimulationContext,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) {
        // Leftmost guy reinitializes the them-list one last time before
        // breaking, so all members have fresh enemy data for solo AI.
        if self.left_combat_neighbour == 0 {
            self.phalanx_reinitialize_them_list(&ctx.position, ctx, tick);
        }

        // Propagate left
        if self.left_combat_neighbour != 0 {
            // Walk left to leftmost, telling each to break
            let mut current = self.left_combat_neighbour;
            for _ in 0..16 {
                if current == 0 {
                    break;
                }
                self.base
                    .outbox
                    .reentrant
                    .cross_npc_actions
                    .push(CrossNpcAction::BreakPhalanx { target: current });
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

        // Propagate right
        if self.right_combat_neighbour != 0 {
            let mut current = self.right_combat_neighbour;
            for _ in 0..16 {
                if current == 0 {
                    break;
                }
                self.base
                    .outbox
                    .reentrant
                    .cross_npc_actions
                    .push(CrossNpcAction::BreakPhalanx { target: current });
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

        // Clear our own links
        self.left_combat_neighbour = 0;
        self.right_combat_neighbour = 0;
        self.phalanx_aborted = true;

        // Go on with single-fighter AI
        self.battle_decisions(sim, global, ctx, tick, grid);
    }

    /// PhalanxReinitializeThemList. Rebuild the shared enemy list for
    /// the whole phalanx. The reference is recursive through the
    /// right-neighbour chain; here we build from our own detectable
    /// enemies plus those visible to right neighbours via snapshots,
    /// then set primary_target to the nearest enemy.
    fn phalanx_reinitialize_them_list(
        &mut self,
        phalanx_left_pos: &Position,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        // (1..3) Replay the original recursion over the up-front member
        // snapshots. Each member cleans its persistent list with its own
        // current 360° radius+LOS, then scans its live detectable-enemy
        // list with its own current 180° radius+LOS.
        self.list_them.clear();
        let obstacles = ctx.obstacle_list();
        for member in &tick.phalanx_member_them_lists {
            append_phalanx_member_enemies(&mut self.list_them, member, obstacles);
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

            let mut best_handle = self.list_them[0];
            let mut best_dist = f32::MAX;
            for &h in &self.list_them {
                if let Some(snap) = self.find_fighter(h, tick) {
                    let dx = snap.position.x - center_x;
                    let dy = (snap.position.y - center_y)
                        * crate::position_interface::INVERSE_ASPECT_RATIO;
                    // MaxNorm(ASPECT_RATIO) — Chebyshev (L∞) — not
                    // Euclidean. L2 would pick a different "nearest"
                    // enemy whenever dx/dy are asymmetric.
                    let d = dx.abs().max(dy.abs());
                    if d < best_dist {
                        best_dist = d;
                        best_handle = h;
                    }
                }
            }
            // Swap nearest to front
            if let Some(idx) = self.list_them.iter().position(|&h| h == best_handle) {
                self.list_them.swap(0, idx);
            }
            self.base.primary_target = best_handle;
        } else {
            self.base.primary_target = 0;
        }
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
        self.base.clear_emoticon();

        // Check PHALANX_ATTACK_DISTANCE gate
        let nearest = self.get_new_primary_target(PrimaryTargetFlags::empty(), ctx, tick);
        if nearest != 0
            && let Some(snap) = self.find_fighter(nearest, tick)
        {
            let d = pos_diff(&snap.position, &ctx.position);
            let atk_dist = archer::PHALANX_ATTACK_DISTANCE as f32;
            if square_norm(d) < atk_dist * atk_dist {
                self.break_phalanx(sim, global, ctx, tick, grid);
                return true;
            }
        }

        // Only the leftmost guy has the right to modify the phalanx
        if self.left_combat_neighbour != 0 {
            return false;
        }

        // Reinitialize them lists
        self.phalanx_reinitialize_them_list(&ctx.position, ctx, tick);

        if self.list_them.is_empty() {
            // Pass no flags (uwFlags = 0, default per
            // ), so the FAST_OVERVIEW branch
            // that pulls in FillListWithAllNearFighters is deliberately
            // skipped.
            self.get_battle_overview(0, ctx, tick);
            return true;
        }

        // The engine's `update_shield_obstacles()` runs every frame, so
        // the shield box is always current — no explicit refresh needed.

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

        let (enemies_in_front, enemy_on_right_side) = match dir_diff {
            0 | 1 | 15 => {
                // Within tolerance
                if self.phalanx_is_encircled_by_enemies(&phalanx_center, ideal_direction, tick) {
                    self.break_phalanx(sim, global, ctx, tick, grid);
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

                // Compute forward vector
                let mut forward = pos_diff(&primary_pos, &phalanx_center);
                let fwd_len = max_norm(forward);
                if fwd_len > 0.001 {
                    forward.0 /= fwd_len;
                    forward.1 /= fwd_len;
                }
                let step = archer::PHALANX_FORWARD_STEP as f32;
                let fwd_step = (forward.0 * step, forward.1 * step);

                // Compute rightward vector. Scale by
                // DISTANCE_SHIELD_BEARER_SHIELD_BEARER directly with no
                // extra aspect-ratio factor on the Y component (after
                // the preceding Normalize(ASPECT_RATIO)); the stray
                // `* ASPECT_RATIO` that used to be on `right.1` squared
                // the aspect ratio and was a port bug.
                let right = get_normal_right(forward);
                let right_scaled = (right.0 * distance_sb, right.1 * distance_sb);

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
                self.break_phalanx(sim, global, ctx, tick, grid);
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
                    return false;
                }
            }
            _ => return false,
        }

        // Shield bearers only
        let me_snap = self.find_fighter(self.base.me, tick);
        if !me_snap.map(|f| f.is_shield_bearer).unwrap_or(false) {
            return false;
        }

        // Get nearest enemy
        let nearest_enemy =
            self.get_new_primary_target(PrimaryTargetFlags::VIPS_ALLOWED, ctx, tick);
        if nearest_enemy == 0 {
            return false;
        }

        // Are we already near enough to fight? (PHALANX_ATTACK_DISTANCE gate)
        if let Some(enemy_snap) = self.find_fighter(nearest_enemy, tick) {
            let d = pos_diff(&enemy_snap.position, &ctx.position);
            let atk_dist = archer::PHALANX_ATTACK_DISTANCE as f32;
            if square_norm(d) < atk_dist * atk_dist {
                return false;
            }
        }

        // Scan all visible enemies for a dangerous one (using a bow).
        // The reference walks the enemy detectables list
        // and additionally gates each candidate on
        // `pDetectable->IsSeenLastFrame() == true` — only enemies the
        // soldier currently sees (or saw last frame) count as
        // dangerous, so a bow-armed enemy who is occluded or has
        // slipped out of the cone of vision can't trip a phalanx /
        // shield-raise.  `tick.seen_last_frame_enemies` mirrors that
        // flag from this NPC's own detectable list.
        let mut dangerous_enemy: HumanHandle = 0;
        for f in &tick.nearby_fighters {
            if f.is_friendly || !f.is_able_to_fight {
                continue;
            }
            if !tick.seen_last_frame_enemies.contains(&f.handle) {
                continue;
            }
            let d = pos_diff(&f.position, &ctx.position);
            let min_dist = archer::MIN_PROTECT_ARROW_DISTANCE as f32;
            if square_norm(d) < min_dist * min_dist {
                continue;
            }
            if f.action_state.is_bow() {
                dangerous_enemy = f.handle;
                break;
            }
        }

        if dangerous_enemy == 0 {
            // No dangerous archer — check if friendly archers need protection
            if self.number_of_nearby_archers_who_need_protection(ctx, tick) <= 0 {
                return false;
            }
            self.base.primary_target = nearest_enemy;
        } else {
            self.base.primary_target = dangerous_enemy;
        }

        // Focus primary target. Original Focus updates the NPC's view target;
        // it does not turn the actor's body or overwrite direction_goal.
        let target_pos = self
            .find_fighter(self.base.primary_target, tick)
            .map(|f| f.position)
            .unwrap_or(ctx.position);
        self.base.outbox.actor.set_focus(self.base.primary_target);

        // Try to join a phalanx
        if let Some((run_pos, direction, left_neighbour, right_neighbour)) =
            self.find_phalanx_place(ctx, tick, grid)
        {
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
        } else {
            // No phalanx slot — raise shield in place
            self.base.stop_all();
            self.base.raise_shield(target_pos);

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
                    list.swap_remove(i);
                    // Note: cleanup is commutative for our scoring
                    // purposes — swap_remove is fine even though the
                    // reference's Delete(i)/i-- preserves order.
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
        // Keep ourselves on a heartbeat while in swordfight.
        if self.base.current_substate == Substate::AttackingSwordfight {
            self.base.launch_timer(20, ctx.frame);
        }

        // Bail out if an ENTER_SWORDFIGHT sequence element is already
        // queued — the entity isn't ready to fight yet. The engine sets
        // `enter_swordfight_pending` on AiContext when there's a pending
        // ENTER_SWORDFIGHT in the sequence manager.
        if ctx.enter_swordfight_pending {
            return;
        }

        // Are we still swordfighting at all? Route through
        // Think(EVENT_QUIT_SWORDFIGHT) so the unexpected-event handler
        // fires. Cascade caveat: skips engine FilterAIEvent gate — see
        // end_think comment.
        if !ctx.is_swordfighting {
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
            self.end_swordfight(ctx, tick);
            self.left_combat_neighbour = 0;
            self.right_combat_neighbour = 0;
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
        if !self.is_detecting_360_degrees(self.base.primary_target, ctx) {
            // Lost sight: forecast their direction and abandon the fight.
            if let Some(prepared) = &tick.primary_target_forecast {
                let forecast = prepared.resolve(sim);
                self.base.seek_position = forecast.position;
                self.pc_gone_away_in_this_direction = forecast.direction;
            }
            self.missed_pc = self.base.primary_target;
            self.pc_missed = true;
            self.end_swordfight(ctx, tick);

            // Focus(NULL) — clear target lock.
            self.base.outbox.actor.set_unfocus();

            // Chase or overview depending on target type and personality.
            if tick.primary_target_is_pc
                && self.answer_question(Question::ShallIFollowLostEnemy, ctx)
            {
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
                // SetDirectionInstantly to the missed-PC direction with
                // the isometric Y-stretch via `vec_to_sector` so the
                // snapped direction matches the AI's face target.
                let dx = self.base.seek_position.x - ctx.position.x;
                let dy = self.base.seek_position.y - ctx.position.y;
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
            return;
        };

        // Are we facing the primary opponent?
        let to_target = pos_diff(&primary.position, &ctx.position);
        let target_sector = vec_to_sector(to_target.0, to_target.1);
        let facing_delta = (ctx.direction as i32 + 16 - target_sector as i32).rem_euclid(16);
        if !matches!(facing_delta, 15 | 0 | 1) {
            // Need to turn first; the engine will rotate us, then call back.
            return;
        }

        // -----------------------------------------------------------------
        // Build the us / them lists from the cached snapshot.
        // -----------------------------------------------------------------
        let me_pos = ctx.position;
        let max_radius = parameters_ai::MAX_SWORDFIGHT_CONSIDERATION_RADIUS as f32;

        self.base.list_us.clear();
        self.base.list_us.push(self.base.me);
        self.list_them.clear();
        let mut nearest_friend_solo: HumanHandle = 0;
        let mut nearest_friend_solo_dist = f32::MAX;
        let mut number_of_swordfighting_enemies: u16 = 0;

        // First pass: us list (only friends actively swordfighting).
        // MaxNormDistance (Chebyshev) for this radius check.
        for f in &tick.nearby_fighters {
            if f.handle == self.base.me || !f.is_friendly {
                continue;
            }
            if !f.is_swordfighting {
                continue;
            }
            let dv = pos_diff(&f.position, &me_pos);
            let dist = max_norm(dv);
            if dist < max_radius {
                self.base.list_us.push(f.handle);
                if f.number_of_opponents > 1 && dist < nearest_friend_solo_dist {
                    nearest_friend_solo = f.handle;
                    nearest_friend_solo_dist = dist;
                }
            }
        }

        // Second pass: them list (any able-to-fight enemy that we can see).
        for f in &tick.nearby_fighters {
            if f.is_friendly || !f.is_able_to_fight {
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

        // Merry men with bow flee!
        if self.is_merry_man_forest(ctx)
            && self.is_archer()
            && self.merry_man_forest_cassos(ctx, global)
        {
            // Flee!
            return;
        }

        // Imbalanced situation rebalance — if I'm dogpiling someone with
        // help while a friend is fighting solo, swap to the solo fighter's
        // nearest enemy.
        let primary_outnumbered = primary.number_of_opponents > 1;
        if primary_outnumbered && nearest_friend_solo != 0 {
            let nearest_enemy_of_solo = calculate_opponent_nearest_to_rene(
                &tick.nearby_fighters,
                nearest_friend_solo,
                &me_pos,
            );
            if nearest_enemy_of_solo != 0 {
                let i_should_take_him = calculate_opponent_nearest_to_rene(
                    &tick.nearby_fighters,
                    self.base.primary_target,
                    self.find_fighter(nearest_enemy_of_solo, tick)
                        .map(|f| &f.position)
                        .unwrap_or(&me_pos),
                ) == self.base.me;
                if i_should_take_him {
                    // Request the engine to enter swordfight with the new
                    // target. The engine picks this up after the AI
                    // tick.
                    self.base.outbox.actor.enter_swordfight =
                        Some(EnterSwordfightRequest::Engage(nearest_enemy_of_solo));
                    self.base.primary_target = nearest_enemy_of_solo;
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
            return;
        }

        // Original: both gates are evaluated even for a sober soldier.
        // Preserve the `||` short-circuit: a zero roll at blood alcohol 0
        // consumes only the first draw and still freezes the soldier.
        if drunk_combat_freezes(sim, self.base.blood_alcohol) {
            return;
        }

        // Refresh primary snapshot in case it changed above.
        let Some(primary) = self.find_fighter(self.base.primary_target, tick).cloned() else {
            return;
        };
        let to_target = pos_diff(&primary.position, &ctx.position);
        let dist_to_target = (to_target.0 * to_target.0 + to_target.1 * to_target.1).sqrt();

        // Weak-enemy charge: a soldier sprints in if the foe is out of
        // his max range and he has the capacity to charge.
        let my_max_range = self
            .find_fighter(self.base.me, tick)
            .map(|f| f.sword_range_maximal)
            .unwrap_or(self.sword_range) as f32;
        let my_fighting_ability = self
            .find_fighter(self.base.me, tick)
            .map(|f| f.fighting_ability)
            .unwrap_or(0);
        if enemy_weak
            && self.get_rank() == ProfileRank::Soldier
            && dist_to_target > my_max_range
            && my_fighting_ability >= combat::MIN_CAPACITY_CHARGE_WEAK_ENEMY
        {
            let target_pos = primary.position;
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
        let do_reposition = !self.combat_trainer
            && (number_of_friends != 1 || number_of_swordfighting_enemies != 1)
            && crate::sim_rng::u32(sim, crate::sim_rng::RngSite::CombatReposition, 0..3) == 0;

        if do_reposition {
            let new_combat_position = self.propose_good_combat_position(ctx, tick, grid);
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
        let primary_max_range = primary.sword_range_maximal as f32;
        if dist_to_target > my_max_range
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
        let dir_vec = sector_to_vector(ctx.direction);
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
    ///   8. defensive predecision → step-back goal or panic flee and bail
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
        // (1) Arrow protection guard.
        if self.refresh_arrow_protection(false, ctx, tick, grid) {
            return;
        }

        // (2) Rebuild list_them with the three filters and reset
        //     multiplicity. Multiplicity is tracked locally in
        //     `local_mult` since the global engine snapshot
        //     `primary_target_multiplicity` is not radius-filtered; the
        //     reference's count is restricted to fighters within
        //     `MAX_SWORDFIGHT_CONSIDERATION_RADIUS`.
        let saved_primary = self.base.primary_target;
        self.list_them.clear();
        let max_radius = parameters_ai::MAX_SWORDFIGHT_CONSIDERATION_RADIUS as f32;
        let me_pos = ctx.position;
        let mut local_mult: std::collections::BTreeMap<HumanHandle, u32> =
            std::collections::BTreeMap::new();
        for f in &tick.nearby_fighters {
            if f.is_friendly || !f.is_able_to_fight {
                continue;
            }
            let d = max_norm(pos_diff(&f.position, &me_pos));
            if d >= max_radius {
                continue;
            }
            if !self.is_detecting_180_degrees(f.handle, ctx) {
                continue;
            }
            self.list_them.push(f.handle);
            local_mult.insert(f.handle, 0);
        }
        // Carry across the previous primary target if the snapshot dropped
        // it for a frame (mirrors the comment on `reinitialize_them_list`).
        if saved_primary != 0 && !self.list_them.contains(&saved_primary) {
            self.list_them.push(saved_primary);
            local_mult.entry(saved_primary).or_insert(0);
        }

        // (3) Rebuild list_us (self first), bump
        //     multiplicity for same-camp soldiers actively in any swordfight
        //     substate against a primary target.
        self.base.list_us.clear();
        self.base.list_us.push(self.base.me);
        for f in &tick.nearby_fighters {
            if !f.is_friendly || f.handle == self.base.me || !f.is_able_to_fight {
                continue;
            }
            let d = max_norm(pos_diff(&f.position, &me_pos));
            if d >= max_radius {
                continue;
            }
            self.base.list_us.push(f.handle);
            if f.is_soldier
                && f.primary_target != 0
                && is_any_swordfight_substate(f.current_substate)
            {
                *local_mult.entry(f.primary_target).or_insert(0) += 1;
            }
        }

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

        // (7) Combat trainer special path: snap to face the target,
        //     stop, set Observe, launch 20-tick timer.
        if self.combat_trainer {
            if let Some(primary) = self.find_fighter(new_primary, tick) {
                let v = pos_diff(&primary.position, &me_pos);
                let dir = vec_to_sector_ar(v.0, v.1, ASPECT_RATIO);
                self.base.outbox.actor.set_direction_instantly = Some(dir as i16);
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
                self.flee(&enemy_pos, ctx, tick, global);
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

            let target_dir = sector_to_vector(primary.direction);
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
                    self.attack_enemy(new_primary, Some(&mut *global), ctx, tick, grid);
                    return;
                }
            }
        }

        // (10) Repositioning + fallback stay-in-place.
        self.observe_and_step(sim, ctx, tick, grid);
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
            self.base.outbox.actor.set_direction_instantly = Some(dir as i16);
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
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
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
            let score = evaluate_combat_position_full(
                me_handle,
                &me_pos,
                &them_handles,
                cp,
                &mut friends_positions,
                &mut enemies_positions,
                &tick.nearby_fighters,
                tick.required_profile_manager(),
                iq,
            );
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
        if best.line_jump.is_none()
            && let Some(grid) = grid
        {
            let my_max_range = self
                .find_fighter(self.base.me, tick)
                .map(|f| f.sword_range_maximal)
                .unwrap_or(self.sword_range);
            best.line_jump = crate::engine::melee::table_swordfight_jump_line(
                grid,
                ctx.position.sector.map(i16::from).unwrap_or(-1),
                primary.position.sector.map(i16::from).unwrap_or(-1),
                crate::coordinates::MapPoint::new(primary.position.x, primary.position.y),
                my_max_range as f32,
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
    use crate::element::Posture;
    use crate::sight_obstacle::{ObstacleList, ObstaclePoint, SightObstacle};
    use crate::sim_rng::{RngSite, SimulationContext, with_draw_trace};

    fn position(x: f32, y: f32) -> Position {
        Position {
            x,
            y,
            sector: None,
            level: 0,
        }
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
            current_them_list,
            detectable_enemies,
            position: position(0.0, 0.0),
            direction: 4,
            posture: Posture::Upright,
            elevation: 0.0,
            is_rider: false,
            in_building: false,
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

        append_phalanx_member_enemies(&mut merged, &leftmost, ObstacleList::empty());
        append_phalanx_member_enemies(&mut merged, &right, ObstacleList::empty());

        assert!(merged.is_empty());
    }

    #[test]
    fn phalanx_rejects_occluded_persistent_and_detectable_entries() {
        let target = enemy(9, 200.0);
        let member = member(2, 300.0, vec![target.clone()], vec![target]);

        let mut clear_merged = Vec::new();
        append_phalanx_member_enemies(&mut clear_merged, &member, ObstacleList::empty());
        assert_eq!(clear_merged, vec![9]);

        let obstacles = vec![opaque_wall()];
        let active = vec![true];
        let blocked = ObstacleList {
            static_obstacles: &obstacles,
            dynamic_obstacles: &[],
            static_active: &active,
        };
        let mut blocked_merged = Vec::new();
        append_phalanx_member_enemies(&mut blocked_merged, &member, blocked);
        assert!(blocked_merged.is_empty());
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
}
