//! Live shot selection and shared human sorting keys.

use super::*;
use crate::ai::{AiEntityHandle, AiState, Decision, HumanHandle, Substate};
use crate::sim_rng::SimulationContext;
use std::ops::ControlFlow;

fn vector_angle(ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let dot = ax * bx + ay * by;
    let det = ax * by - ay * bx;
    if det == 0.0 {
        return if dot > 0.0 { 0.0 } else { std::f32::consts::PI };
    }
    let angle = (det / dot).atan();
    if dot >= 0.0 {
        angle
    } else if det > 0.0 {
        angle + std::f32::consts::PI
    } else {
        angle - std::f32::consts::PI
    }
}

impl EngineInner {
    pub(in crate::engine) fn propose_live_shot_target(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) -> Option<AiEntityHandle> {
        let position = self.live_ai_position(owner);
        let entity = self.expect_entity(owner, "shot selection owner");
        let nose =
            crate::shadow_polygon::sector_to_direction(entity.element_data().direction() as i16);
        let forest = self.world.weather.is_forest_level
            && self.is_player_aligned_camp(entity.camp())
            && !entity.soldier_data().is_some_and(|soldier| soldier.rider);
        let (bow, _) = self
            .bow_profile_and_ability(assets, owner)
            .expect("shot selection requires a bow");
        let bow = assets
            .profile_manager
            .get_bow(bow)
            .expect("shot selection bow profile");
        let range = f32::from(if bow.has_long_shoot {
            bow.long_shoot.range
        } else {
            bow.normal_shoot.range
        });
        let enemies = self
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("shot enemies"))
            .list_them
            .clone();
        for &enemy in &enemies {
            self.ai
                .global
                .primary_target_multiplicity_scratch
                .insert(enemy, 0);
        }
        let friends = self
            .world
            .entities
            .expect_ai_controller(owner, format_args!("shot friends"))
            .list_us
            .clone();
        // Angles are private to this selection; distance keys are shared with
        // nested alert, patrol, and money-victim operations.
        let mut angles = Vec::with_capacity(friends.len());
        for &friend in &friends {
            if friend == owner.index() {
                continue;
            }
            let id = self.expect_human_id_for_ai_handle(friend, "shot friend");
            let point = self.live_ai_position(id);
            let dx = point.x - position.x;
            let dy = (point.y - position.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
            self.world
                .entities
                .expect_entity_mut(id, format_args!("shot friend sorting key"))
                .human_data_mut()
                .expect("shot friend human data")
                .sorting_distance = dx * dx + dy * dy;
            angles.push((id, vector_angle(nose[0], nose[1], dx, dy)));
            if let Some(Entity::Soldier(soldier)) = self.world.entities.get(id) {
                let ai = soldier.npc.ai_brain.enemy().expect("shot friend brain");
                if matches!(
                    ai.base.current_substate,
                    Substate::AttackingBowShooting
                        | Substate::AttackingBowLoading
                        | Substate::AttackingBowAiming
                ) && let Some(target) = ai.base.primary_target
                {
                    *self
                        .ai
                        .global
                        .primary_target_multiplicity_scratch
                        .entry(target.get())
                        .or_default() += 1;
                }
            }
        }
        let mut best = None;
        let mut minimum = u32::MAX as f32;
        for enemy in enemies {
            let id = self.expect_human_id_for_ai_handle(enemy, "shot enemy");
            let point = self.live_ai_position(id);
            let dx = point.x - position.x;
            let dy = (point.y - position.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
            let mut distance = dx * dx + dy * dy;
            if !(distance <= range * range && self.sleeping_enemy_attack_allowed(owner, id)) {
                continue;
            }
            if distance > 100.0 * 100.0 && forest {
                distance +=
                    crate::sim_rng::u32(sim, crate::sim_rng::RngSite::ArcherForestTarget, 0..10000)
                        as f32;
            }
            distance += 10000.0
                * self
                    .ai
                    .global
                    .primary_target_multiplicity_scratch
                    .get(&enemy)
                    .copied()
                    .unwrap_or(0) as f32;
            if !(distance <= minimum && self.sleeping_enemy_attack_allowed(owner, id)) {
                continue;
            }
            let angle = vector_angle(nose[0], nose[1], dx, dy);
            let blocked = angles.iter().any(|&(friend, friend_angle)| {
                let entity = self.expect_entity(friend, "shot blocking friend");
                if !(entity
                    .human_data()
                    .expect("shot friend human data")
                    .sorting_distance
                    <= distance)
                    || (forest && !matches!(entity, Entity::Pc(_)))
                    || entity
                        .actor_data()
                        .expect("shot friend actor data")
                        .action_state
                        .is_shield()
                {
                    return false;
                }
                let mut difference = (angle - friend_angle).abs();
                while difference > std::f32::consts::TAU {
                    difference -= std::f32::consts::TAU;
                }
                difference < crate::ai_enemy::archer::MIN_TARGET_FRIEND_ANGLE
            });
            if !blocked {
                best = Some(AiEntityHandle::new(enemy));
                minimum = distance;
            }
        }
        best
    }

    pub(in crate::engine) fn execute_live_shoot_decision(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) -> ControlFlow<bool, Decision> {
        if self
            .expect_entity(owner, "shot ammunition")
            .ai_actor_data()
            .expect("shot actor")
            .number_of_arrows
            == 0
        {
            return ControlFlow::Continue(Decision::RunForNewArrows);
        }
        let Some(target) = self.propose_live_shot_target(sim, assets, owner) else {
            return ControlFlow::Continue(Decision::ArcherObserve);
        };
        let ai = self
            .world
            .entities
            .expect_ai_controller_mut(owner, format_args!("shot selected target"));
        ai.primary_target = Some(target);
        ai.outbox.actor.set_focus(target.get());
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
        let bow_state = self
            .expect_entity(owner, "shot action")
            .actor_data()
            .expect("shot actor")
            .action_state
            .is_bow();
        if bow_state {
            if self
                .world
                .entities
                .expect_ai_controller(owner, format_args!("shot substate"))
                .current_substate
                == Substate::AttackingBowAiming
            {
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Attacking,
                    Substate::AttackingBowShooting,
                );
                self.world
                    .entities
                    .expect_ai_controller_mut(owner, format_args!("shoot stop"))
                    .stop_all();
                self.drain_direct_ai_owner_boundary(sim, owner, assets);
                let target = self.expect_human_id_for_ai_handle(target.get(), "shot target");
                self.shoot_bow_at(assets, owner, target);
            } else {
                let (_, ability) = self
                    .bow_profile_and_ability(assets, owner)
                    .expect("aiming bow");
                let time = (110_u32.saturating_sub(ability) / 2).max(5);
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Attacking,
                    Substate::AttackingBowAiming,
                );
                let frame = self.control.frame_counter;
                self.world
                    .entities
                    .expect_ai_controller_mut(owner, format_args!("aim timer"))
                    .launch_timer(time, frame);
            }
        } else {
            self.world
                .entities
                .expect_ai_controller_mut(owner, format_args!("equip bow stop"))
                .stop_all();
            self.drain_direct_ai_owner_boundary(sim, owner, assets);
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Attacking,
                Substate::AttackingBowLoading,
            );
            let ai = self
                .world
                .entities
                .expect_enemy_ai_mut(owner, format_args!("equip bow"));
            ai.base
                .outbox
                .actor
                .launch_commands
                .push(if ai.enemy_seen_below {
                    crate::element::Command::EquipBowDown
                } else {
                    crate::element::Command::EquipBow
                });
            self.drain_direct_ai_owner_boundary(sim, owner, assets);
        }
        ControlFlow::Break(true)
    }

    pub(in crate::engine) fn execute_ai_select_shot_target(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        old_substate: Substate,
        cover_shield_bearer: HumanHandle,
    ) {
        match self.execute_live_shoot_decision(sim, assets, owner) {
            ControlFlow::Break(log) => {
                if log {
                    self.world
                        .entities
                        .expect_ai_controller_mut(owner, format_args!("shot decision log"))
                        .register_log_line(
                            crate::ai::LogLineType::BattleDecision,
                            Decision::Shoot as u16,
                        );
                }
            }
            ControlFlow::Continue(decision) => {
                let scratch = self.build_sim_scratch(assets);
                let mut ctx =
                    self.ai_context_for(owner, self.control.frame_counter, &scratch, assets);
                ctx.in_uninterruptible_command = self.is_very_very_busy(owner);
                let tick = self.build_npc_tick_data(sim, owner, assets);
                let mut counts = self.ai.global.primary_target_multiplicity_scratch.clone();
                let result = self
                    .world
                    .entities
                    .expect_enemy_ai_mut(owner, format_args!("shot fallback"))
                    .execute_battle_decision(
                        crate::ai_enemy::ThinkEnv::new(
                            sim,
                            &ctx,
                            &tick,
                            Some(&self.world.fast_grid),
                        ),
                        decision,
                        old_substate,
                        cover_shield_bearer,
                        &mut counts,
                        &mut self.ai.global,
                    );
                match result {
                    Err(call) => {
                        self.execute_ai_duty_call(sim, assets, owner, call);
                    }
                    Ok(Some(decision)) => self
                        .world
                        .entities
                        .expect_ai_controller_mut(owner, format_args!("shot fallback log"))
                        .register_log_line(crate::ai::LogLineType::BattleDecision, decision as u16),
                    Ok(None) => {}
                }
                self.drain_direct_ai_owner_boundary(sim, owner, assets);
            }
        }
    }
}
