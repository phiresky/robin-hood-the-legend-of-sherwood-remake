//! Formation decisions execute against current actors and reciprocal links.

use super::*;
use crate::ai::{AiEntityHandle, Position, Stimulus, StimulusType, Substate};
use crate::ai_enemy::{PrimaryTargetFlags, archer};
use crate::coordinates::MapVec;
use crate::engine::TickCtx;
use crate::position_interface::{ASPECT_RATIO, INVERSE_ASPECT_RATIO};

#[cfg(test)]
mod tests;

fn phalanx_advance_vectors(delta: MapVec) -> (MapVec, MapVec) {
    let normalize = |v: MapVec| {
        let y = v.y / ASPECT_RATIO;
        let norm = (v.x * v.x + y * y).sqrt();
        MapVec::new(v.x / norm, v.y / norm)
    };
    let forward = normalize(delta);
    let forward = MapVec::new(
        forward.x * archer::PHALANX_FORWARD_STEP as f32,
        forward.y * archer::PHALANX_FORWARD_STEP as f32,
    );
    let [x, y] = crate::position_interface::vector_normal_iso(forward.x, forward.y, true);
    let right = normalize(MapVec::new(x, y));
    (
        forward,
        MapVec::new(
            right.x * archer::DISTANCE_SHIELD_BEARER_SHIELD_BEARER as f32,
            right.y * archer::DISTANCE_SHIELD_BEARER_SHIELD_BEARER as f32,
        ),
    )
}

fn shifted(position: Position, vector: MapVec, scale: f32) -> Position {
    Position {
        x: position.x + scale * vector.x,
        y: position.y + scale * vector.y,
        ..position
    }
}

impl EngineInner {
    fn phalanx_neighbour(&self, owner: EntityId, right: bool) -> Option<EntityId> {
        let ai = self.enemy_ai(owner, "phalanx neighbour owner");
        let handle = if right {
            ai.right_combat_neighbour
        } else {
            ai.left_combat_neighbour
        }?;
        let target = self.expect_human_id_for_ai_handle(handle.get(), "phalanx neighbour");
        assert!(
            matches!(target, EntityId::Soldier(_)),
            "phalanx neighbour must be a soldier"
        );
        Some(target)
    }

    fn live_phalanx_protects_archers(&self, owner: EntityId) -> bool {
        let mut member = owner;
        loop {
            if self
                .enemy_ai(member, "phalanx protected archer")
                .archer_behind_me
                .is_some()
            {
                return true;
            }
            let Some(next) = self.phalanx_neighbour(member, true) else {
                return false;
            };
            member = next;
        }
    }

    fn live_phalanx_encircled(&self, owner: EntityId, center: Position, intended: u16) -> bool {
        self.enemy_ai(owner, "phalanx enemy directions")
            .list_them
            .iter()
            .any(|&handle| {
                let enemy = self.expect_human_id_for_ai_handle(handle, "phalanx enemy direction");
                let position = self.live_ai_position(enemy);
                let direction = crate::position_interface::vector_to_sector_0_to_15_iso(
                    position.x - center.x,
                    position.y - center.y,
                ) as u16;
                !matches!(intended.wrapping_sub(direction) & 15, 0..=3 | 13..=15)
            })
    }

    pub(in crate::engine) fn reinitialize_live_phalanx_enemies(
        &mut self,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        let left = self.live_ai_position(owner).map_point();
        let mut merged = Vec::new();
        self.collect_live_phalanx_enemies(assets, owner, left, &mut merged);
    }

    fn collect_live_phalanx_enemies(
        &mut self,
        assets: &LevelAssets,
        owner: EntityId,
        left: MapPoint,
        merged: &mut Vec<u32>,
    ) {
        let mut index = 0;
        loop {
            let Some(&handle) = self
                .enemy_ai(owner, "phalanx retained enemy")
                .list_them
                .get(index)
            else {
                break;
            };
            let target = self.expect_human_id_for_ai_handle(handle, "phalanx retained target");
            let entity = self.expect_entity(target, "phalanx fighting target");
            let able = match entity {
                Entity::Pc(actor) => actor.is_able_to_fight(),
                Entity::Soldier(actor) => actor.is_able_to_fight(),
                Entity::Civilian(actor) => actor.is_able_to_fight(),
                _ => unreachable!("phalanx target must be human"),
            };
            let keep = able
                && self.patrol_member_visible(assets, owner, target)
                && !self.camps_are_allied(
                    self.expect_entity(owner, "phalanx camp").camp(),
                    self.expect_entity(target, "phalanx target camp").camp(),
                );
            if keep {
                if !merged.contains(&handle) {
                    merged.push(handle);
                }
                index += 1;
            } else {
                self.enemy_ai_mut(owner, "phalanx remove enemy")
                    .list_them
                    .remove(index);
            }
        }
        let count = self
            .expect_entity(owner, "phalanx detectable count")
            .ai_actor_data()
            .expect("phalanx owner requires NPC data")
            .detectable_lists[crate::element::DetectableType::Enemy as usize]
            .len();
        for index in 0..count {
            let target = self
                .expect_entity(owner, "phalanx detectable enemy")
                .ai_actor_data()
                .unwrap()
                .detectable_lists[crate::element::DetectableType::Enemy as usize][index]
                .element
                .expect("phalanx detectable requires target");
            if self.live_ai_detects_180(assets, owner, target) {
                let entity = self.expect_entity(target, "phalanx visible enemy");
                if !entity.is_dead()
                    && !entity
                        .human_data()
                        .expect("phalanx visible target requires human")
                        .unconscious
                    && !merged.contains(&target.index())
                {
                    merged.push(target.index());
                }
            }
        }
        if let Some(right) = self.phalanx_neighbour(owner, true) {
            self.collect_live_phalanx_enemies(assets, right, left, merged);
        } else if !merged.is_empty() {
            let right = self.live_ai_position(owner).map_point();
            let center = MapPoint::new(
                left.x + 0.5 * (right.x - left.x),
                left.y + 0.5 * (right.y - left.y),
            );
            let mut nearest = None;
            let mut minimum = 65_432_u16;
            for (index, &handle) in merged.iter().enumerate() {
                let target = self.expect_human_id_for_ai_handle(handle, "phalanx nearest target");
                let position = self.live_ai_position(target);
                let distance = (position.x - center.x)
                    .abs()
                    .max(((position.y - center.y) * INVERSE_ASPECT_RATIO).abs())
                    as u16;
                if distance < minimum {
                    minimum = distance;
                    nearest = Some(index);
                }
            }
            merged.swap(0, nearest.expect("phalanx enemy exceeds maximum distance"));
        }
        let ai = self.enemy_ai_mut(owner, "phalanx shared enemy assignment");
        ai.list_them.clone_from(merged);
        ai.base.primary_target = merged.first().copied().map(AiEntityHandle::new);
    }

    pub(in crate::engine) fn execute_ai_break_phalanx(
        &mut self,
        tcx: TickCtx<'_>,
        owner: EntityId,
        tell_right: bool,
        tell_left: bool,
    ) {
        AiOwnerCtx::new(self, tcx, owner).execute_ai_break_phalanx(tell_right, tell_left)
    }

    fn phalanx_line_accessible(
        &self,
        owner: EntityId,
        from: Position,
        to: Position,
        layer: u16,
    ) -> bool {
        self.world.fast_grid.is_straight_movement_authorized(
            from.map_point(),
            to.map_point(),
            layer,
            self.expect_entity(owner, "phalanx movement bounds")
                .position_iface()
                .get_move_box(),
        )
    }

    pub(in crate::engine) fn instruct_live_phalanx(
        &mut self,
        tcx: TickCtx<'_>,
        members: &[EntityId],
        left: Position,
        right: MapVec,
        direction: u16,
    ) {
        for (index, &member) in members.iter().enumerate() {
            let ai = self.enemy_ai_mut(member, "phalanx instruction recipient");
            if ai.base.current_substate != Substate::AttackingPhalanx {
                continue;
            }
            ai.gather_position = shifted(left, right, index as f32);
            ai.gather_direction = direction;
            ai.gather_position_instructed = true;
            self.execute_ai_callback(tcx, member, &Stimulus::new(StimulusType::CallInstruction));
        }
    }

    pub(in crate::engine) fn reconsider_live_phalanx(
        &mut self,
        tcx: TickCtx<'_>,
        owner: EntityId,
    ) -> bool {
        AiOwnerCtx::new(self, tcx, owner).reconsider_live_phalanx()
    }

    pub(in crate::engine) fn execute_ai_phalanx_timer(
        &mut self,
        tcx: TickCtx<'_>,
        owner: EntityId,
    ) {
        let primary = self.ai(owner, "phalanx timer target").primary_target;
        let action = self
            .expect_entity(owner, "phalanx timer action")
            .actor_data()
            .expect("phalanx member requires actor")
            .action_state;
        if !matches!(
            action,
            crate::element::ActionState::HoldingShield
                | crate::element::ActionState::ParryingShield
        ) && let Some(primary) = primary
        {
            let target = self.expect_human_id_for_ai_handle(primary.get(), "phalanx shield target");
            let point = self
                .expect_entity(target, "phalanx shield danger point")
                .element_data()
                .position();
            self.launch_ai_raise_shield(tcx, owner, point);

            self.world
                .entities
                .expect_ai_controller_mut(owner, format_args!("phalanx shield timer"))
                .launch_timer(20, self.control.frame_counter);
        } else if !self.reconsider_live_phalanx(tcx, owner) {
            if let Some(primary) = self.ai(owner, "phalanx direction target").primary_target {
                let owner_position = self.live_ai_position(owner);
                let target = self.live_ai_position(
                    self.expect_human_id_for_ai_handle(primary.get(), "phalanx facing enemy"),
                );
                let direction = crate::position_interface::vector_to_sector_0_to_15_iso(
                    target.x - owner_position.x,
                    target.y - owner_position.y,
                ) as u16;
                self.execute_ai_direction_goal(owner, direction);

                self.refresh_retained_shield_obstacle(tcx.assets, owner);

                self.world
                    .entities
                    .expect_ai_controller_mut(owner, format_args!("phalanx facing timer"))
                    .launch_timer(20, self.control.frame_counter);
            } else {
                self.execute_ai_get_battle_overview(tcx, owner, 0);
            }
        }
    }
}

impl AiOwnerCtx<'_> {
    pub(in crate::engine) fn execute_ai_phalanx_instruction(&mut self) {
        let ai = self
            .engine
            .enemy_ai_mut(self.owner, "phalanx gather instruction");
        ai.shield_bearer_direction = ai.gather_direction;
        ai.base.seek_position = ai.gather_position;
        self.duty_set_state(
            crate::ai::AiState::Attacking,
            Substate::AttackingRunningToPhalanx,
        );
        let position = self
            .engine
            .ai(self.owner, "phalanx instructed destination")
            .seek_position;
        self.duty_go_to(position, crate::ai::GotoFlags::RUN);
        if let Some(archer) = self
            .engine
            .enemy_ai(self.owner, "phalanx instructed archer")
            .archer_behind_me
        {
            let archer = self
                .engine
                .expect_human_id_for_ai_handle(archer.get(), "protected archer");
            if matches!(
                self.engine
                    .world
                    .entities
                    .expect_ai_controller(archer, format_args!("protected archer state"))
                    .current_substate,
                Substate::AttackingBowShooting
                    | Substate::AttackingBowLoading
                    | Substate::AttackingBowAiming
            ) {
                self.engine.execute_ai_callback(
                    self.tcx,
                    archer,
                    &Stimulus::new(StimulusType::CallCoordinate),
                );
            }
        }
    }

    pub(in crate::engine) fn execute_ai_break_phalanx(
        &mut self,
        tell_right: bool,
        tell_left: bool,
    ) {
        if tell_left {
            if let Some(left) = self.engine.phalanx_neighbour(self.owner, false) {
                self.engine
                    .execute_ai_break_phalanx(self.tcx, left, false, true);
            } else {
                self.engine
                    .reinitialize_live_phalanx_enemies(self.tcx.assets, self.owner);
            }
        }
        if tell_right && let Some(right) = self.engine.phalanx_neighbour(self.owner, true) {
            self.engine
                .execute_ai_break_phalanx(self.tcx, right, true, false);
        }
        let right = self
            .engine
            .enemy_ai(self.owner, "break right link")
            .right_combat_neighbour;
        self.engine
            .apply_update_right_combat_neighbour(self.owner.index(), right, None);
        let left = self
            .engine
            .enemy_ai(self.owner, "break left link")
            .left_combat_neighbour;
        self.engine
            .apply_update_left_combat_neighbour(self.owner.index(), left, None);
        self.engine
            .enemy_ai_mut(self.owner, "abandon phalanx")
            .phalanx_aborted = true;
        self.execute_battle_decisions();
    }

    pub(in crate::engine) fn reconsider_live_phalanx(&mut self) -> bool {
        self.engine
            .ai_mut(self.owner, "phalanx emoticon")
            .clear_emoticon();

        if let Some(target) = self
            .engine
            .select_live_ai_primary_target(self.owner, PrimaryTargetFlags::empty())
        {
            let target = self
                .engine
                .expect_human_id_for_ai_handle(target.get(), "phalanx close threat");
            if self.engine.protection_square_distance(self.owner, target)
                < (archer::PHALANX_ATTACK_DISTANCE as f32).powi(2)
            {
                self.execute_ai_break_phalanx(true, true);
                return true;
            }
        }
        if self.engine.phalanx_neighbour(self.owner, false).is_some() {
            return false;
        }
        self.engine
            .reinitialize_live_phalanx_enemies(self.tcx.assets, self.owner);
        if self
            .engine
            .enemy_ai(self.owner, "phalanx enemies")
            .list_them
            .is_empty()
        {
            self.execute_ai_get_battle_overview(0);
            return true;
        }
        self.engine
            .refresh_retained_shield_obstacle(self.tcx.assets, self.owner);

        // This membership list deliberately survives the callbacks that issue moves.
        let mut members = Vec::new();
        let mut current = self.owner;
        loop {
            members.push(current);
            let primary = self
                .engine
                .ai(self.owner, "phalanx leader target")
                .primary_target;
            let ai = self
                .engine
                .enemy_ai_mut(current, "phalanx member readiness");
            ai.base.primary_target = primary;
            if ai.base.current_substate != Substate::AttackingPhalanx {
                return false;
            }
            let Some(next) = self.engine.phalanx_neighbour(current, true) else {
                break;
            };
            current = next;
        }
        let size = members.len();
        if size == 1 {
            return false;
        }
        let center = self.engine.live_ai_position(members[size / 2]);
        let primary = self
            .engine
            .ai(self.owner, "phalanx geometry target")
            .primary_target
            .expect("phalanx geometry requires target");
        let target = self.engine.live_ai_position(
            self.engine
                .expect_human_id_for_ai_handle(primary.get(), "phalanx geometry enemy"),
        );
        let owner_position = self.engine.live_ai_position(self.owner);
        let last = self.engine.live_ai_position(members[size - 1]);
        let direction = |from: Position, to: Position| {
            crate::position_interface::vector_to_sector_0_to_15_iso(to.x - from.x, to.y - from.y)
                as u16
        };
        let ideal = direction(center, target);
        let real = (direction(last, owner_position) + 4) & 15;
        let difference = ideal.wrapping_sub(real) & 15;
        if matches!(difference, 0 | 1 | 15) {
            if self
                .engine
                .live_phalanx_encircled(self.owner, center, ideal)
            {
                self.execute_ai_break_phalanx(true, true);
                return true;
            }
            if self.engine.live_phalanx_protects_archers(self.owner)
                || crate::sim_rng::u32(self.tcx.sim, crate::sim_rng::RngSite::PhalanxAdvance, 0..3)
                    != 0
            {
                return false;
            }
            let (forward, right) = phalanx_advance_vectors(target.map_point() - center.map_point());
            let new_center = shifted(center, forward, 1.0);
            let new_left = shifted(new_center, right, -((size / 2) as f32));
            let new_right = shifted(new_center, right, (size - 1 - size / 2) as f32);
            if !self
                .engine
                .phalanx_line_accessible(self.owner, new_left, new_right, center.level)
                || !(self.engine.phalanx_line_accessible(
                    self.owner,
                    owner_position,
                    new_left,
                    center.level,
                ) || self.engine.phalanx_line_accessible(
                    self.owner,
                    last,
                    new_right,
                    center.level,
                ) || self.engine.phalanx_line_accessible(
                    self.owner,
                    center,
                    new_center,
                    center.level,
                ))
            {
                return false;
            }
            self.engine
                .instruct_live_phalanx(self.tcx, &members, new_left, right, ideal);
            true
        } else {
            let [x, y] = crate::shadow_polygon::sector_to_direction(((ideal + 4) & 15) as i16);
            let right = MapVec::new(
                x * archer::DISTANCE_SHIELD_BEARER_SHIELD_BEARER as f32,
                (y * ASPECT_RATIO) * archer::DISTANCE_SHIELD_BEARER_SHIELD_BEARER as f32,
            );
            for index in 0..size {
                let pivot = if (2..=8).contains(&difference) {
                    size - index - 1
                } else {
                    index
                };
                let left = shifted(
                    self.engine.live_ai_position(members[pivot]),
                    right,
                    -(pivot as f32),
                );
                let last = shifted(left, right, (size - 1) as f32);
                if self
                    .engine
                    .phalanx_line_accessible(self.owner, left, last, last.level)
                {
                    self.engine
                        .instruct_live_phalanx(self.tcx, &members, left, right, ideal);
                    return true;
                }
            }
            self.execute_ai_break_phalanx(true, true);
            true
        }
    }
}
