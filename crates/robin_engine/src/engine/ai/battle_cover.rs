//! Live shield-cover and ranged-observation decisions.

use super::*;
use crate::ai::{
    AiEntityHandle, AiState, Decision, GotoFlags, HumanHandle, Position, Remark, Substate,
};
use crate::ai_enemy::{PrimaryTargetFlags, archer};
use crate::sim_rng::SimulationContext;
use std::ops::ControlFlow;

#[cfg(test)]
mod tests;

impl EngineInner {
    fn cover_primary(&self, owner: EntityId) -> EntityId {
        let target = self
            .world
            .entities
            .expect_ai_controller(owner, format_args!("cover primary"))
            .primary_target
            .expect("cover decision requires primary target");
        self.expect_human_id_for_ai_handle(target.get(), "cover primary entity")
    }

    fn cover_face_primary(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        let target = self.cover_primary(owner);
        let position = self.live_ai_position(target);
        let elevation = self
            .expect_entity(target, "cover facing elevation")
            .element_data()
            .position()
            .z;
        self.duty_face_position_at_elevation(sim, assets, owner, position, elevation);
    }

    fn cover_focus_primary(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        let ai = self
            .world
            .entities
            .expect_ai_controller_mut(owner, format_args!("cover focus"));
        let target = ai.primary_target;
        self.execute_ai_focus(owner, target);
    }

    fn cover_step_back_goal(
        &self,
        owner: EntityId,
        target: Position,
        good: u16,
        minimum: u16,
    ) -> Option<Position> {
        crate::ai_enemy::propose_good_step_back_goal(
            self.live_ai_position(owner),
            self.expect_entity(owner, "cover retreat move box")
                .position_iface()
                .get_move_box(),
            target,
            good,
            minimum,
            Some(&self.world.fast_grid),
            crate::position_interface::ASPECT_RATIO,
        )
    }

    pub(in crate::engine) fn live_archer_is_too_near(
        &self,
        assets: &LevelAssets,
        owner: EntityId,
        target: Option<AiEntityHandle>,
    ) -> bool {
        let entity = self.expect_entity(owner, "archer proximity owner");
        let ai = entity
            .enemy_ai()
            .expect("archer proximity requires enemy AI");
        if ai.shield_bearer_before_me.is_some()
            || (self.world.weather.is_forest_level
                && self.is_player_aligned_camp(entity.camp())
                && !entity.soldier_data().is_some_and(|s| s.rider))
        {
            return false;
        }
        let target = self.expect_human_id_for_ai_handle(
            target.expect("archer proximity requires target").get(),
            "archer proximity target",
        );
        self.ai_archer_is_too_near_to_enemy(assets, owner, self.live_ai_position(owner), target)
    }

    pub(in crate::engine) fn live_ai_is_too_proud_to_attack(
        &mut self,
        assets: &LevelAssets,
        owner: EntityId,
    ) -> bool {
        let ai = self
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("proud soldier"));
        if ai.profile(&assets.profile_manager).pride == 0 || ai.base.blood_alcohol > 0 {
            return false;
        }
        let target = self.select_live_ai_primary_target(
            owner,
            PrimaryTargetFlags::UNOCCUPIED_STRONGLY_PREFERRED,
        );
        self.world
            .entities
            .expect_ai_controller_mut(owner, format_args!("proud target selection"))
            .primary_target = target;
        let Some(target) = target else {
            return false;
        };
        let target = self.expect_human_id_for_ai_handle(target.get(), "proud target");
        let entity = self.expect_entity(target, "proud target fighting");
        let fighting = !entity
            .human_data()
            .expect("proud target human")
            .opponents
            .is_empty();
        let target_world = entity.element_data().position();
        let me_world = self
            .expect_entity(owner, "proud owner position")
            .element_data()
            .position();
        let distance = (target_world.x - me_world.x)
            .abs()
            .max(
                ((target_world.y - me_world.y) * crate::position_interface::INVERSE_ASPECT_RATIO)
                    .abs(),
            )
            .max((target_world.z - me_world.z).abs());
        let ai = self
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("proud owner range"));
        let range = assets
            .profile_manager
            .get_hth_weapon(ai.hth_weapon_id)
            .expect("proud owner weapon")
            .distance[crate::weapons::WeaponDistance::Maximal as usize];
        if !fighting && distance <= f32::from(range) {
            return false;
        }
        if matches!(
            ai.base.current_substate,
            Substate::AttackingReactiontime | Substate::AttackingOfficerGivingOrdersWaiting
        ) || fighting
        {
            return true;
        }
        let pride = ai.profile(&assets.profile_manager).pride;
        let count = ai.base.list_us.len();
        for index in 0..count {
            let handle = self
                .world
                .entities
                .expect_ai_controller(owner, format_args!("proud ally list"))
                .list_us[index];
            let friend = self.expect_human_id_for_ai_handle(handle, "proud ally");
            if friend == owner {
                continue;
            }
            let Entity::Soldier(soldier) = self.expect_entity(friend, "proud ally kind") else {
                continue;
            };
            if !soldier.is_able_to_fight() {
                continue;
            }
            let ai = soldier
                .npc
                .ai_brain
                .enemy()
                .expect("proud ally requires AI");
            if ai.profile(&assets.profile_manager).pride >= pride {
                continue;
            }
            let state = ai.base.current_substate;
            if state.is_any_swordfight()
                && ai.base.primary_target.map(|p| p.get()) == Some(target.index())
            {
                return true;
            }
            if matches!(
                state,
                Substate::AttackingApproachToObserve
                    | Substate::AttackingObserve
                    | Substate::AttackingObserveAndMove
            ) && self.live_ai_detects_180(assets, friend, target)
            {
                return true;
            }
        }
        false
    }

    pub(in crate::engine) fn execute_ai_battle_too_proud(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        old_substate: Substate,
    ) -> ControlFlow<bool, Decision> {
        let target = self.select_live_ai_primary_target(owner, PrimaryTargetFlags::VIPS_ALLOWED);
        self.world
            .entities
            .expect_ai_controller_mut(owner, format_args!("proud execution target"))
            .primary_target = target;
        let Some(_) = target else {
            tracing::warn!(?owner, "proud observer lost its primary target; reserving");
            return ControlFlow::Continue(Decision::Reserve);
        };
        let position = self.live_ai_position(self.cover_primary(owner));
        let me = self.live_ai_position(owner);
        let dx = position.x - me.x;
        let dy = (position.y - me.y) / crate::position_interface::ASPECT_RATIO;
        let distance = (dx * dx + dy * dy).sqrt();
        if distance < crate::parameters_ai::PROUD_OBSERVER_MIN_DISTANCE as f32 {
            let Some(goal) = self.cover_step_back_goal(
                owner,
                position,
                crate::parameters_ai::PROUD_OBSERVER_GOOD_DISTANCE,
                crate::parameters_ai::PROUD_OBSERVER_MIN_DISTANCE,
            ) else {
                return ControlFlow::Continue(Decision::Fight);
            };
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Attacking,
                Substate::AttackingTooProudToAttackRetire,
            );
            self.duty_go_to(sim, assets, owner, goal, GotoFlags::empty());
        } else if distance > crate::parameters_ai::PROUD_OBSERVER_MAX_DISTANCE as f32 {
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Attacking,
                Substate::AttackingTooProudToAttackApproach,
            );
            let position = self.live_ai_position(self.cover_primary(owner));
            self.duty_go_near(
                sim,
                assets,
                owner,
                position,
                crate::parameters_ai::PROUD_OBSERVER_GOOD_DISTANCE as i32,
                GotoFlags::empty(),
            );
            let ai = self
                .world
                .entities
                .expect_ai_controller_mut(owner, format_args!("proud approach completion"));
            if ai.already_on_point {
                ai.already_on_point = false;
                self.cover_face_primary(sim, assets, owner);
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Attacking,
                    Substate::AttackingTooProudToAttack,
                );
                self.world
                    .entities
                    .expect_ai_controller_mut(owner, format_args!("proud timer"))
                    .launch_timer(20, self.control.frame_counter);
            }
        } else {
            self.cover_face_primary(sim, assets, owner);
            self.cover_focus_primary(sim, assets, owner);
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Attacking,
                Substate::AttackingTooProudToAttack,
            );
            self.world
                .entities
                .expect_ai_controller_mut(owner, format_args!("proud observer timer"))
                .launch_timer(20, self.control.frame_counter);
        }
        if matches!(
            old_substate,
            Substate::AttackingReactiontime | Substate::AttackingReactiontimeRunning
        ) {
            let ai = self
                .world
                .entities
                .expect_enemy_ai_mut(owner, format_args!("proud remark"));
            let remark = if ai.is_vip {
                Remark::VipProudDontFight
            } else {
                Remark::ProudDontFight
            };
            self.execute_ai_speech(
                sim,
                assets,
                owner,
                crate::ai::AiSpeechAttempt { remark, flags: 0 },
            );
        }
        ControlFlow::Break(true)
    }

    pub(in crate::engine) fn execute_ai_battle_archer_step_back(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        _old_substate: Substate,
    ) -> ControlFlow<bool, Decision> {
        let target = self.select_live_ai_primary_target(owner, PrimaryTargetFlags::VIPS_ALLOWED);
        self.world
            .entities
            .expect_ai_controller_mut(owner, format_args!("archer retreat target"))
            .primary_target = target;
        let Some(_) = target else {
            tracing::warn!(
                ?owner,
                "retreating archer lost its primary target; shooting"
            );
            return ControlFlow::Continue(Decision::Shoot);
        };
        let position = self.live_ai_position(self.cover_primary(owner));
        self.world
            .entities
            .expect_ai_controller_mut(owner, format_args!("archer retreat position"))
            .seek_position = position;
        let Some(goal) = self.cover_step_back_goal(
            owner,
            position,
            crate::parameters_ai::ARCHER_GOOD_DISTANCE,
            crate::parameters_ai::ARCHER_MIN_DISTANCE,
        ) else {
            return ControlFlow::Continue(Decision::Shoot);
        };
        self.duty_set_state(
            sim,
            assets,
            owner,
            AiState::Attacking,
            Substate::AttackingArcherRetireFromCombat,
        );
        self.duty_go_to(sim, assets, owner, goal, GotoFlags::RUN);
        ControlFlow::Break(true)
    }

    pub(in crate::engine) fn execute_ai_battle_archer_observe(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) -> ControlFlow<bool, Decision> {
        let target = self.select_live_ai_primary_target(
            owner,
            PrimaryTargetFlags::UNOCCUPIED_PREFERRED | PrimaryTargetFlags::VIPS_ALLOWED,
        );
        self.world
            .entities
            .expect_ai_controller_mut(owner, format_args!("archer observer target"))
            .primary_target = target;
        self.cover_focus_primary(sim, assets, owner);
        if self
            .expect_entity(owner, "archer observer action")
            .actor_data()
            .expect("archer observer actor")
            .action_state
            .is_bow()
        {
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Attacking,
                Substate::AttackingBowObserving,
            );
            self.world
                .entities
                .expect_ai_controller_mut(owner, format_args!("archer observing timer"))
                .launch_timer(50, self.control.frame_counter);
        } else {
            self.stop_ai_owner(sim, assets, owner);
            let ai = self
                .world
                .entities
                .expect_enemy_ai_mut(owner, format_args!("archer observer equip"));
            let command = if ai.enemy_seen_below {
                crate::element::Command::EquipBowDown
            } else {
                crate::element::Command::EquipBow
            };
            self.launch_element(
                sim,
                assets,
                crate::sequence::SequenceElement::new(1, command, Some(owner)),
            );

            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Attacking,
                Substate::AttackingBowObservingLoading,
            );
        }
        ControlFlow::Break(true)
    }

    fn update_live_shield_before_archer(&mut self, owner: EntityId, bearer: Option<EntityId>) {
        let ai = self
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("archer protection link"));
        if !ai.is_archer() {
            return;
        }
        let new = bearer.map(|id| AiEntityHandle::new(id.index()));
        let old = ai.shield_bearer_before_me;
        if old == new {
            return;
        }
        if let Some(old) = old {
            let old = self.expect_human_id_for_ai_handle(old.get(), "old shield bearer");
            self.world
                .entities
                .expect_enemy_ai_mut(old, format_args!("old shield unlink"))
                .archer_behind_me = None;
        }
        self.world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("archer shield assignment"))
            .shield_bearer_before_me = new;
        if let Some(new) = bearer {
            self.world
                .entities
                .expect_enemy_ai_mut(new, format_args!("new shield reciprocal"))
                .archer_behind_me = Some(AiEntityHandle::new(owner.index()));
        }
    }

    pub(in crate::engine) fn execute_ai_battle_cover(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        bearer: HumanHandle,
    ) -> ControlFlow<bool, Decision> {
        let bearer = self.expect_human_id_for_ai_handle(bearer, "battle shield bearer");
        self.update_live_shield_before_archer(owner, Some(bearer));
        let target = self
            .world
            .entities
            .expect_ai_controller(bearer, format_args!("shield bearer target"))
            .primary_target;
        self.world
            .entities
            .expect_ai_controller_mut(owner, format_args!("covered archer target"))
            .primary_target = target;
        if target.is_none() {
            self.update_live_shield_before_archer(owner, None);
            return ControlFlow::Continue(Decision::Shoot);
        }
        let (anchor, direction) = self.live_shield_bearer_position(bearer);
        let [x, y] = crate::shadow_polygon::sector_to_direction(direction as i16);
        let distance = archer::DISTANCE_SHIELD_BEARER_ARCHER as f32;
        let position = Position {
            x: anchor.x - x * distance,
            y: anchor.y - (y * crate::position_interface::ASPECT_RATIO) * distance,
            ..anchor
        };
        self.world
            .entities
            .expect_ai_controller_mut(owner, format_args!("cover candidate output"))
            .seek_position = position;
        let reachable = self.world.fast_grid.is_straight_movement_authorized(
            anchor.map_point(),
            position.map_point(),
            position.level,
            self.expect_entity(owner, "cover move box")
                .position_iface()
                .get_move_box(),
        );
        if !reachable {
            self.update_live_shield_before_archer(owner, None);
            return ControlFlow::Continue(Decision::Shoot);
        }
        let target = self.live_ai_position(self.cover_primary(owner));
        let dx = target.x - position.x;
        let dy = target.y - position.y;
        let radius = f32::from(self.ai.standard_view_polygon_radius);
        if dx * dx + dy * dy >= radius * radius {
            self.update_live_shield_before_archer(owner, None);
            return ControlFlow::Continue(Decision::Shoot);
        }
        self.duty_set_state(
            sim,
            assets,
            owner,
            AiState::Attacking,
            Substate::AttackingBowRunningBehindShieldBearer,
        );
        let position = self
            .world
            .entities
            .expect_ai_controller(owner, format_args!("cover movement position"))
            .seek_position;
        self.duty_go_to(sim, assets, owner, position, GotoFlags::RUN);
        if self
            .world
            .entities
            .expect_ai_controller(owner, format_args!("cover completion"))
            .already_on_point
        {
            let target = self.live_ai_position(self.cover_primary(owner));
            let position = self.live_ai_position(owner);
            let direction = crate::position_interface::vector_to_sector_0_to_15_iso(
                target.x - position.x,
                target.y - position.y,
            ) as i16;
            if self
                .expect_entity(owner, "cover direction")
                .element_data()
                .direction()
                == direction
            {
                self.world
                    .entities
                    .expect_ai_controller_mut(owner, format_args!("cover shoot completion"))
                    .already_on_point = false;
                return ControlFlow::Continue(Decision::Shoot);
            }
        }
        self.execute_ai_speech(
            sim,
            assets,
            bearer,
            crate::ai::AiSpeechAttempt {
                remark: Remark::ArchersBehindShieldBearers,
                flags: 0,
            },
        );
        ControlFlow::Break(true)
    }
}
