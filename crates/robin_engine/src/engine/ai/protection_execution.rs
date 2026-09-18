//! Shield protection evaluates actors and formation links at each call site.

#[cfg(test)]
mod tests;

use super::*;
use crate::ai::{AiEntityHandle, AiState, EmoticonType, GotoFlags, Position, Remark, Substate};
use crate::ai_enemy::{PrimaryTargetFlags, archer};
use crate::engine::TickCtx;

impl EngineInner {
    pub(in crate::engine) fn launch_ai_raise_shield(
        &mut self,
        tcx: TickCtx<'_>,
        owner: EntityId,
        point: crate::coordinates::WorldPoint3D,
    ) {
        let mut element = crate::sequence::SequenceElement::new_generic(
            1,
            crate::element::Command::RaiseShield,
            Some(owner),
        );
        element.set_property(
            crate::sequence::Field::ShieldDangerPoint,
            crate::sequence::FieldValue::Point3D {
                x: point.x,
                y: point.y,
                z: point.z,
            },
        );
        self.launch_element(tcx, element);
    }

    #[cfg(test)]
    pub(in crate::engine) fn execute_ai_shield_expected_event(
        &mut self,
        tcx: TickCtx<'_>,
        owner: EntityId,
        event: crate::ai::StimulusType,
    ) -> bool {
        AiOwnerCtx::new(self, tcx, owner).execute_ai_shield_expected_event(event)
    }

    fn shield_timer(&mut self, owner: EntityId, delay: u32) {
        self.world
            .entities
            .expect_ai_controller_mut(owner, format_args!("shield event timer"))
            .launch_timer(delay, self.control.frame_counter);
    }

    fn shield_primary(&self, owner: EntityId) -> EntityId {
        let target = self
            .ai(owner, "shield primary target")
            .primary_target
            .expect("shield action requires a primary target");
        self.expect_human_id_for_ai_handle(target.get(), "shield primary target")
    }

    fn shield_focus_primary(&mut self, owner: EntityId) {
        let ai = self.ai_mut(owner, "shield focus");
        let target = ai.primary_target;
        self.execute_ai_focus(owner, target);
    }

    fn live_phalanx_neighbour_target(&self, owner: EntityId) -> Option<Option<AiEntityHandle>> {
        let ai = self.enemy_ai(owner, "phalanx neighbour target");
        for neighbour in [ai.left_combat_neighbour, ai.right_combat_neighbour]
            .into_iter()
            .flatten()
        {
            let id = self.expect_human_id_for_ai_handle(neighbour.get(), "phalanx neighbour");
            if matches!(
                self.expect_entity(id, "phalanx neighbour kind"),
                Entity::Soldier(_)
            ) {
                return Some(self.ai(id, "phalanx neighbour primary").primary_target);
            }
        }
        None
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
        self.world
            .soldier_registry
            .camp(camp)
            .iter()
            .map(|&handle| EntityId::Soldier(crate::entity_id::SoldierId(handle)))
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
            let ai = self.enemy_ai(candidate, "free shield bearer");
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
        let ai = self.enemy_ai(bearer, "shield position bearer");
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
            let ai = self.enemy_ai(current, "phalanx end member");
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
            .enemy_ai(owner, "phalanx placement owner")
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
            if actor_queries::is_archer_from_bow(bow)
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
        tcx: TickCtx<'_>,
        owner: EntityId,
        called_from_hourglass: bool,
    ) -> bool {
        AiOwnerCtx::new(self, tcx, owner).refresh_ai_arrow_protection(called_from_hourglass)
    }
}

impl AiOwnerCtx<'_> {
    pub(in crate::engine) fn execute_ai_shield_expected_event(
        &mut self,
        event: crate::ai::StimulusType,
    ) -> bool {
        use crate::ai::StimulusType;
        let substate = self.engine.ai(self.owner, "shield event").current_substate;
        match (substate, event) {
            (Substate::AttackingProtectingWithShield, StimulusType::EventTimer) => {
                self.execute_ai_protecting_shield_timer();
            }
            (Substate::AttackingAdvancingWithShield, StimulusType::EventDone) => {
                let target = self.engine.shield_primary(self.owner);
                let destination = self.engine.live_ai_position(target);
                self.duty_go_near(
                    destination,
                    archer::MIN_PROTECT_ARROW_DISTANCE / 2,
                    GotoFlags::RUN,
                );
                self.engine.shield_timer(self.owner, 10);
            }
            (Substate::AttackingRunningToPhalanx, StimulusType::EventReachPoint) => {
                let direction = self
                    .engine
                    .enemy_ai(self.owner, "phalanx arrival direction")
                    .shield_bearer_direction;
                self.duty_face_direction(direction);
            }
            (Substate::AttackingRunningToPhalanx, StimulusType::EventDone) => {
                self.execute_ai_phalanx_arrival();
            }
            _ => return false,
        }
        true
    }

    fn shield_raise_at_primary(&mut self) {
        let target = self.engine.shield_primary(self.owner);
        let point = self
            .engine
            .expect_entity(target, "shield danger point")
            .element_data()
            .position();
        self.engine
            .launch_ai_raise_shield(TickCtx::new(self.sim, self.assets), self.owner, point);
    }

    fn execute_ai_protecting_shield_timer(&mut self) {
        let action = self
            .engine
            .expect_entity(self.owner, "protecting shield action")
            .actor_data()
            .expect("shield actor")
            .action_state;
        if !matches!(
            action,
            crate::element::ActionState::HoldingShield
                | crate::element::ActionState::ParryingShield
        ) {
            self.shield_raise_at_primary();
            self.engine.shield_timer(self.owner, 20);
            return;
        }
        let ai = self.engine.enemy_ai(self.owner, "shield links");
        if ai.left_combat_neighbour.is_some() || ai.right_combat_neighbour.is_some() {
            self.duty_set_state(AiState::Attacking, Substate::AttackingPhalanx);
            self.engine.shield_focus_primary(self.owner);
            self.engine.shield_timer(self.owner, 5);
            return;
        }
        let archer_behind = ai.archer_behind_me.is_some();
        if ai.base.primary_target.is_none() {
            let target = self
                .engine
                .select_live_ai_primary_target(self.owner, PrimaryTargetFlags::VIPS_ALLOWED);
            self.engine
                .ai_mut(self.owner, "replacement shield target")
                .primary_target = target;
        }
        let target = self
            .engine
            .ai(self.owner, "shield target after selection")
            .primary_target;
        let Some(target) = target else {
            if archer_behind {
                tracing::error!(
                    owner = ?self.owner,
                    "shield bearer protecting an archer has no primary target"
                );
            }
            self.execute_ai_get_battle_overview(0);
            return;
        };
        let target = self
            .engine
            .expect_human_id_for_ai_handle(target.get(), "shield danger target");
        if archer_behind {
            let position = self.engine.live_ai_position(target);
            let origin = self.engine.live_ai_position(self.owner);
            let direction = crate::position_interface::vector_to_sector_0_to_15_iso(
                position.x - origin.x,
                position.y - origin.y,
            ) as u16;
            self.engine.execute_ai_direction_goal(self.owner, direction);

            self.engine
                .refresh_retained_shield_obstacle(self.assets, self.owner);

            self.engine.shield_timer(self.owner, 30);
        } else if self
            .engine
            .expect_entity(target, "shield target action")
            .actor_data()
            .expect("shield target actor")
            .action_state
            .is_bow()
        {
            if crate::sim_rng::u32(self.sim, crate::sim_rng::RngSite::ShieldAdvance, 0..4) == 0 {
                self.duty_set_state(AiState::Attacking, Substate::AttackingAdvancingWithShield);
                self.engine.launch_element(
                    TickCtx::new(self.sim, self.assets),
                    crate::sequence::SequenceElement::new(
                        1,
                        crate::element::Command::LowerShield,
                        Some(self.owner),
                    ),
                );
            } else {
                self.engine.shield_timer(self.owner, 10);
            }
        } else {
            self.execute_ai_get_battle_overview(0);
        }
    }

    fn execute_ai_phalanx_arrival(&mut self) {
        let target = self
            .engine
            .live_phalanx_neighbour_target(self.owner)
            .unwrap_or_else(|| {
                tracing::error!(owner = ?self.owner, "phalanx arrival has no soldier neighbour");
                self.engine
                    .select_live_ai_primary_target(self.owner, PrimaryTargetFlags::empty())
            });
        self.engine
            .ai_mut(self.owner, "phalanx arrival primary")
            .primary_target = target;
        if target.is_none() {
            tracing::error!(owner = ?self.owner, "phalanx arrival has no primary target");
            self.execute_battle_decisions();
            return;
        }
        self.duty_set_state(AiState::Attacking, Substate::AttackingPhalanx);
        self.shield_raise_at_primary();
        self.engine.shield_focus_primary(self.owner);
        self.engine.shield_timer(self.owner, 20);
    }

    pub(in crate::engine) fn execute_ai_advancing_shield_timer(&mut self) {
        if self.refresh_ai_arrow_protection(false) {
            return;
        }
        self.execute_ai_get_battle_overview(1);
    }

    pub(in crate::engine) fn refresh_ai_arrow_protection(
        &mut self,
        called_from_hourglass: bool,
    ) -> bool {
        let substate = self
            .engine
            .enemy_ai(self.owner, "arrow protection owner")
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
        if !self
            .engine
            .live_ai_is_shield_bearer(self.assets, self.owner)
        {
            return false;
        }
        let Some(nearest) = self
            .engine
            .select_live_ai_primary_target(self.owner, PrimaryTargetFlags::VIPS_ALLOWED)
        else {
            return false;
        };
        let nearest_id = self
            .engine
            .expect_human_id_for_ai_handle(nearest.get(), "nearest protection threat");
        if self
            .engine
            .protection_square_distance(self.owner, nearest_id)
            < (archer::PHALANX_ATTACK_DISTANCE as f32).powi(2)
        {
            return false;
        }
        let enemies = &self
            .engine
            .expect_entity(self.owner, "protection detectables")
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
                (self.engine.protection_square_distance(self.owner, target)
                    >= (archer::MIN_PROTECT_ARROW_DISTANCE as f32).powi(2)
                    && self
                        .engine
                        .expect_entity(target, "dangerous archer")
                        .actor_data()
                        .expect("dangerous enemy requires actor")
                        .action_state
                        .is_bow())
                .then_some(target)
            });
        if dangerous.is_none()
            && self
                .engine
                .live_archers_needing_protection(self.assets, self.owner)
                <= 0
        {
            return false;
        }
        let target = dangerous.unwrap_or(nearest_id);
        let handle = AiEntityHandle::new(target.index());
        let ai = self.engine.enemy_ai_mut(self.owner, "protection target");
        ai.base.primary_target = Some(handle);
        self.engine.execute_ai_focus(self.owner, Some(handle));

        if let Some((position, direction, left, right)) =
            self.engine.live_phalanx_place(self.assets, self.owner)
        {
            self.execute_ai_speech(crate::ai::AiSpeechAttempt {
                remark: Remark::ShieldBearersLineFormation,
                flags: 0,
            });
            let ai = self.engine.enemy_ai_mut(self.owner, "formation position");
            ai.base.seek_position = position;
            ai.shield_bearer_direction = direction;
            let old_left = ai.left_combat_neighbour;
            self.engine.apply_update_left_combat_neighbour(
                self.owner.index(),
                old_left,
                left.map(|id| AiEntityHandle::new(id.index())),
            );
            let old_right = self
                .engine
                .enemy_ai(self.owner, "formation right link")
                .right_combat_neighbour;
            self.engine.apply_update_right_combat_neighbour(
                self.owner.index(),
                old_right,
                right.map(|id| AiEntityHandle::new(id.index())),
            );
            self.duty_set_state(AiState::Attacking, Substate::AttackingRunningToPhalanx);
            self.duty_go_to(position, GotoFlags::RUN);
        } else {
            self.stop_ai_owner();
            let target = self
                .engine
                .ai(self.owner, "shield target")
                .primary_target
                .expect("shield raise requires primary target");
            let target = self
                .engine
                .expect_entity_id_for_index(target.get(), "shield danger target");
            let point = self
                .engine
                .expect_entity(target, "shield danger point")
                .element_data()
                .position();
            self.engine.launch_ai_raise_shield(
                TickCtx::new(self.sim, self.assets),
                self.owner,
                point,
            );

            let ai = self
                .engine
                .world
                .entities
                .expect_ai_controller_mut(self.owner, format_args!("shield response"));
            if ai.current_substate == Substate::AttackingAdvancingWithShield || dangerous.is_none()
            {
                ai.clear_emoticon();
            } else {
                ai.set_transient_emoticon(
                    EmoticonType::XMark,
                    30,
                    self.engine.control.frame_counter,
                );
                self.execute_ai_speech(crate::ai::AiSpeechAttempt {
                    remark: Remark::ShieldBearerCovers,
                    flags: 0,
                });
            }

            self.duty_set_state(AiState::Attacking, Substate::AttackingProtectingWithShield);
            self.engine
                .world
                .entities
                .expect_ai_controller_mut(self.owner, format_args!("shield timer"))
                .launch_timer(10, self.engine.control.frame_counter);
        }
        true
    }
}
