//! Shield protection evaluates actors and formation links at each call site.

#[cfg(test)]
mod tests;

use super::*;
use crate::ai::{AiEntityHandle, AiState, EmoticonType, GotoFlags, Position, Remark, Substate};
use crate::ai_enemy::{PrimaryTargetFlags, archer};

impl EngineInner {
    pub(in crate::engine) fn execute_ai_advancing_shield_timer(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        if self.refresh_ai_arrow_protection(sim, assets, owner, false) {
            return;
        }
        self.execute_ai_get_battle_overview(sim, assets, owner, 1);
    }

    pub(in crate::engine) fn live_ai_is_shield_bearer(
        &self,
        assets: &LevelAssets,
        owner: EntityId,
    ) -> bool {
        let Entity::Soldier(soldier) = self.expect_entity(owner, "shield bearer query") else {
            return false;
        };
        let ai = soldier
            .npc
            .ai_brain
            .enemy()
            .expect("shield bearer query requires enemy brain");
        let weapon = assets
            .profile_manager
            .get_hth_weapon(ai.hth_weapon_id)
            .expect("shield bearer requires weapon profile");
        weapon.shield
            && soldier
                .element
                .sprite
                .has_animation(crate::order::OrderType::WaitingShield)
    }

    fn protection_camp_soldiers(&self, owner: EntityId) -> impl Iterator<Item = EntityId> + '_ {
        let camp = self.expect_entity(owner, "protection camp").camp();
        self.ai
            .global
            .all_soldier_handles
            .iter()
            .map(|&handle| EntityId::Soldier(crate::entity_id::SoldierId(handle)))
            .filter(move |&id| self.expect_entity(id, "protection soldier registry").camp() == camp)
    }

    pub(super) fn protection_square_distance(&self, owner: EntityId, target: EntityId) -> f32 {
        let owner = self
            .expect_entity(owner, "protection distance owner")
            .element_data()
            .position();
        let target = self
            .expect_entity(target, "protection distance target")
            .element_data()
            .position();
        let dx = target.x - owner.x;
        let dy = (target.y - owner.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
        let dz = target.z - owner.z;
        dx * dx + dy * dy + dz * dz
    }

    pub(in crate::engine) fn nearest_live_free_shield_bearer(
        &self,
        assets: &LevelAssets,
        owner: EntityId,
    ) -> Option<EntityId> {
        let owner_shield = self.live_ai_is_shield_bearer(assets, owner);
        let position = self
            .expect_entity(owner, "shield distance owner")
            .element_data()
            .position();
        let mut nearest = None;
        let mut distance = archer::SHIELD_BEARER_MIN_DISTANCE as u16;
        for candidate in self.protection_camp_soldiers(owner) {
            if !self.live_ai_is_shield_bearer(assets, candidate) {
                continue;
            }
            let ai = self
                .world
                .entities
                .expect_enemy_ai(candidate, format_args!("free shield bearer"));
            if (!owner_shield && ai.archer_behind_me.is_some())
                || !matches!(
                    ai.base.current_substate,
                    Substate::AttackingProtectingWithShield
                        | Substate::AttackingRunningToPhalanx
                        | Substate::AttackingPhalanx
                )
            {
                continue;
            }
            let target = self
                .expect_entity(candidate, "shield distance target")
                .element_data()
                .position();
            let candidate_distance = (target.x - position.x)
                .abs()
                .max(
                    ((target.y - position.y) * crate::position_interface::INVERSE_ASPECT_RATIO)
                        .abs(),
                )
                .max((target.z - position.z).abs()) as u16;
            if candidate_distance < distance {
                distance = candidate_distance;
                nearest = Some(candidate);
            }
        }
        nearest
    }

    pub(in crate::engine) fn live_shield_bearer_position(
        &self,
        bearer: EntityId,
    ) -> (Position, u16) {
        let ai = self
            .world
            .entities
            .expect_enemy_ai(bearer, format_args!("shield position bearer"));
        if ai.base.current_substate == Substate::AttackingRunningToPhalanx {
            (ai.base.seek_position, ai.shield_bearer_direction)
        } else {
            (
                self.live_ai_position(bearer),
                self.expect_entity(bearer, "shield direction")
                    .element_data()
                    .direction() as u16,
            )
        }
    }

    fn live_phalanx_end(&self, start: EntityId, left: bool) -> EntityId {
        let mut current = start;
        loop {
            let ai = self
                .world
                .entities
                .expect_enemy_ai(current, format_args!("phalanx end member"));
            let next = if left {
                ai.left_combat_neighbour
            } else {
                ai.right_combat_neighbour
            };
            let Some(next) = next else {
                return current;
            };
            current = self.expect_human_id_for_ai_handle(next.get(), "phalanx neighbour");
            assert!(
                matches!(
                    self.expect_entity(current, "phalanx neighbour kind"),
                    Entity::Soldier(_)
                ),
                "phalanx neighbour is not a soldier"
            );
        }
    }

    fn live_phalanx_place(
        &self,
        assets: &LevelAssets,
        owner: EntityId,
    ) -> Option<(Position, u16, Option<EntityId>, Option<EntityId>)> {
        if self
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("phalanx placement owner"))
            .phalanx_aborted
        {
            return None;
        }
        let nearest = self.nearest_live_free_shield_bearer(assets, owner)?;
        let left = self.live_phalanx_end(nearest, true);
        let (left_position, left_direction) = self.live_shield_bearer_position(left);
        let left_forward = crate::shadow_polygon::sector_to_direction(left_direction as i16);
        let left_offset = [
            left_forward[1] * archer::DISTANCE_SHIELD_BEARER_SHIELD_BEARER as f32,
            -left_forward[0]
                * archer::DISTANCE_SHIELD_BEARER_SHIELD_BEARER as f32
                * crate::position_interface::ASPECT_RATIO,
        ];
        let left_slot = Position {
            x: left_position.x + left_offset[0],
            y: left_position.y + left_offset[1],
            ..left_position
        };
        let layer = self
            .expect_entity(left, "left phalanx layer")
            .element_data()
            .layer();
        let bounds = self
            .expect_entity(owner, "phalanx move bounds")
            .position_iface()
            .get_move_box();
        let left_accessible = self.world.fast_grid.is_straight_movement_authorized(
            left_position.map_point(),
            left_slot.map_point(),
            layer,
            bounds,
        );
        // The second probe intentionally uses the left anchor's current layer.
        let right = self.live_phalanx_end(nearest, false);
        let (right_position, right_direction) = self.live_shield_bearer_position(right);
        let right_forward = crate::shadow_polygon::sector_to_direction(right_direction as i16);
        let right_offset = [
            -right_forward[1] * archer::DISTANCE_SHIELD_BEARER_SHIELD_BEARER as f32,
            right_forward[0]
                * archer::DISTANCE_SHIELD_BEARER_SHIELD_BEARER as f32
                * crate::position_interface::ASPECT_RATIO,
        ];
        let right_slot = Position {
            x: right_position.x + right_offset[0],
            y: right_position.y + right_offset[1],
            ..right_position
        };
        let right_accessible = self.world.fast_grid.is_straight_movement_authorized(
            right_position.map_point(),
            right_slot.map_point(),
            layer,
            bounds,
        );
        let position = self.live_ai_position(owner);
        let square_distance = |point: Position| {
            let dx = position.x - point.x;
            let dy = position.y - point.y;
            dx * dx + dy * dy
        };
        if left_accessible
            && (!right_accessible || square_distance(left_slot) < square_distance(right_slot))
        {
            Some((left_slot, left_direction, None, Some(left)))
        } else if right_accessible {
            Some((right_slot, right_direction, Some(right), None))
        } else {
            None
        }
    }

    fn live_archers_needing_protection(&self, assets: &LevelAssets, owner: EntityId) -> i32 {
        let mut count = 0;
        for candidate in self.protection_camp_soldiers(owner) {
            if self.protection_square_distance(owner, candidate)
                >= (archer::CONSIDER_BATTLE_SITUATION_DISTANCE as f32).powi(2)
            {
                continue;
            }
            let Entity::Soldier(soldier) = self.expect_entity(candidate, "protection candidate")
            else {
                unreachable!()
            };
            let ai = soldier
                .npc
                .ai_brain
                .enemy()
                .expect("protection candidate requires brain");
            if !matches!(
                ai.base.current_state,
                AiState::Seeking | AiState::Wondering | AiState::Attacking
            ) {
                continue;
            }
            let (_, _, bow) = self.soldier_profile_facts(assets, soldier, candidate);
            if snapshots::is_archer_from_bow(bow)
                && ai.shield_bearer_before_me.is_none()
                && !ai.tower_guard
            {
                count += 1;
            } else if self.live_ai_is_shield_bearer(assets, candidate)
                && ai.archer_behind_me.is_none()
                && candidate != owner
                && matches!(
                    ai.base.current_substate,
                    Substate::AttackingPhalanx
                        | Substate::AttackingRunningToPhalanx
                        | Substate::AttackingProtectingWithShield
                        | Substate::AttackingAdvancingWithShield
                )
            {
                count -= 1;
            }
        }
        count
    }

    pub(in crate::engine) fn refresh_ai_arrow_protection(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        called_from_hourglass: bool,
    ) -> bool {
        let substate = self
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("arrow protection owner"))
            .base
            .current_substate;
        match substate {
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
            | Substate::AttackingTooProudToAttack => {}
            Substate::AttackingAdvancingWithShield if !called_from_hourglass => {}
            _ => return false,
        }
        if !self.live_ai_is_shield_bearer(assets, owner) {
            return false;
        }
        let Some(nearest) =
            self.select_live_ai_primary_target(owner, PrimaryTargetFlags::VIPS_ALLOWED)
        else {
            return false;
        };
        let nearest_id =
            self.expect_human_id_for_ai_handle(nearest.get(), "nearest protection threat");
        if self.protection_square_distance(owner, nearest_id)
            < (archer::PHALANX_ATTACK_DISTANCE as f32).powi(2)
        {
            return false;
        }
        let enemies = &self
            .expect_entity(owner, "protection detectables")
            .ai_actor_data()
            .expect("protection owner requires NPC data")
            .detectable_lists[crate::element::DetectableType::Enemy as usize];
        let dangerous = enemies
            .iter()
            .filter(|entry| entry.seen_last_frame)
            .find_map(|entry| {
                let target = entry
                    .element
                    .expect("seen enemy detectable requires entity");
                (self.protection_square_distance(owner, target)
                    >= (archer::MIN_PROTECT_ARROW_DISTANCE as f32).powi(2)
                    && self
                        .expect_entity(target, "dangerous archer")
                        .actor_data()
                        .expect("dangerous enemy requires actor")
                        .action_state
                        .is_bow())
                .then_some(target)
            });
        if dangerous.is_none() && self.live_archers_needing_protection(assets, owner) <= 0 {
            return false;
        }
        let target = dangerous.unwrap_or(nearest_id);
        let handle = AiEntityHandle::new(target.index());
        let ai = self
            .world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("protection target"));
        ai.base.primary_target = Some(handle);
        ai.base.outbox.actor.set_focus(Some(handle));
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
        if let Some((position, direction, left, right)) = self.live_phalanx_place(assets, owner) {
            self.world
                .entities
                .expect_ai_controller_mut(owner, format_args!("formation speech"))
                .say(Remark::ShieldBearersLineFormation);
            self.drain_direct_ai_owner_boundary(sim, owner, assets);
            let ai = self
                .world
                .entities
                .expect_enemy_ai_mut(owner, format_args!("formation position"));
            ai.base.seek_position = position;
            ai.shield_bearer_direction = direction;
            let old_left = ai.left_combat_neighbour;
            self.apply_update_left_combat_neighbour(
                owner.index(),
                old_left,
                left.map(|id| AiEntityHandle::new(id.index())),
            );
            let old_right = self
                .world
                .entities
                .expect_enemy_ai(owner, format_args!("formation right link"))
                .right_combat_neighbour;
            self.apply_update_right_combat_neighbour(
                owner.index(),
                old_right,
                right.map(|id| AiEntityHandle::new(id.index())),
            );
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Attacking,
                Substate::AttackingRunningToPhalanx,
            );
            self.duty_go_to(sim, assets, owner, position, GotoFlags::RUN);
        } else {
            self.world
                .entities
                .expect_ai_controller_mut(owner, format_args!("shield stop"))
                .stop_all();
            self.drain_direct_ai_owner_boundary(sim, owner, assets);
            let target = self
                .world
                .entities
                .expect_ai_controller(owner, format_args!("shield target"))
                .primary_target
                .expect("shield raise requires primary target");
            let target = self.expect_entity_id_for_index(target.get(), "shield danger target");
            let point = self
                .expect_entity(target, "shield danger point")
                .element_data()
                .position();
            self.world
                .entities
                .expect_ai_controller_mut(owner, format_args!("shield raise"))
                .raise_shield_world(point);
            self.drain_direct_ai_owner_boundary(sim, owner, assets);
            let ai = self
                .world
                .entities
                .expect_ai_controller_mut(owner, format_args!("shield response"));
            if ai.current_substate == Substate::AttackingAdvancingWithShield || dangerous.is_none()
            {
                ai.clear_emoticon();
            } else {
                ai.set_transient_emoticon(EmoticonType::XMark, 30, self.control.frame_counter);
                ai.say(Remark::ShieldBearerCovers);
            }
            self.drain_direct_ai_owner_boundary(sim, owner, assets);
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Attacking,
                Substate::AttackingProtectingWithShield,
            );
            self.world
                .entities
                .expect_ai_controller_mut(owner, format_args!("shield timer"))
                .launch_timer(10, self.control.frame_counter);
        }
        true
    }
}
