//! Live shot selection and shared human sorting keys.

use super::*;
use crate::ai::{AiEntityHandle, AiState, Decision, Substate};
use crate::engine::TickCtx;
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

#[cfg(test)]
mod angle_tests {
    use super::vector_angle;

    #[test]
    fn coincident_friend_has_opposite_angle_to_forward_target() {
        let friend = vector_angle(1.0, 0.0, 0.0, 0.0);
        let target = vector_angle(1.0, 0.0, 100.0, -3.0);
        assert_eq!(friend, std::f32::consts::PI);
        assert!((target - friend).abs() > std::f32::consts::FRAC_PI_2);
    }
}

impl EngineInner {
    pub(in crate::engine) fn propose_live_shot_target(
        &mut self,
        tcx: TickCtx<'_>,
        owner: EntityId,
    ) -> Option<AiEntityHandle> {
        let position = self.live_ai_position(owner);
        let entity = self.expect_entity(owner, "shot selection owner");
        let nose =
            crate::shadow_polygon::sector_to_direction(entity.element_data().direction() as i16);
        let forest = self.world.weather.is_forest_level
            && self.is_player_aligned_camp(entity.camp())
            && !entity.soldier_data().is_some_and(|soldier| soldier.rider);
        let enemy_count = self.enemy_ai(owner, "shot enemies").list_them.len();
        for index in 0..enemy_count {
            let enemy = self.enemy_ai(owner, "shot multiplicity reset").list_them[index];
            self.ai
                .global
                .primary_target_multiplicity_scratch
                .insert(enemy, 0);
        }
        let friend_count = self.ai(owner, "shot friends").list_us.len();
        // Angles are private to this selection; distance keys are shared with
        // nested alert, patrol, and money-victim operations.
        let mut angles = Vec::with_capacity(friend_count);
        for index in 0..friend_count {
            let friend = self.ai(owner, "shot friend angle").list_us[index];
            if friend == owner.index() {
                angles.push(0.0);
                continue;
            }
            let id = self.expect_human_id_for_ai_handle(friend, "shot friend");
            let point = self.live_ai_position(id);
            let dx = point.x - position.x;
            let dy = (point.y - position.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
            self.entities_mut()
                .expect_entity_mut(id, format_args!("shot friend sorting key"))
                .human_data_mut()
                .expect("shot friend human data")
                .sorting_distance = dx * dx + dy * dy;
            angles.push(vector_angle(nose[0], nose[1], dx, dy));
            if let Some(Entity::Soldier(soldier)) = self.entities().get(id) {
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
        let enemy_count = self.enemy_ai(owner, "shot candidate count").list_them.len();
        let (bow, _) = self
            .bow_profile_and_ability(tcx.assets, owner)
            .expect("shot selection requires a bow");
        let bow = tcx
            .assets
            .profile_manager
            .get_bow(bow)
            .expect("shot selection bow profile");
        let range = f32::from(if bow.has_long_shoot {
            bow.long_shoot.range
        } else {
            bow.normal_shoot.range
        });
        let mut best = None;
        let mut minimum = u32::MAX as f32;
        for index in 0..enemy_count {
            let enemy = self.enemy_ai(owner, "shot candidate").list_them[index];
            let id = self.expect_human_id_for_ai_handle(enemy, "shot enemy");
            let point = self.live_ai_position(id);
            let dx = point.x - position.x;
            let dy = (point.y - position.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
            let mut distance = dx * dx + dy * dy;
            let angle = vector_angle(nose[0], nose[1], dx, dy);
            if !(distance <= range * range && self.sleeping_enemy_attack_allowed(owner, id)) {
                continue;
            }
            if distance > 100.0 * 100.0 && forest {
                distance += crate::sim_rng::u32(
                    tcx.sim,
                    crate::sim_rng::RngSite::ArcherForestTarget,
                    0..10000,
                ) as f32;
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
            let blocked = (0..friend_count).any(|index| {
                let friend = self.ai(owner, "shot blocking friend index").list_us[index];
                if friend == owner.index() {
                    return false;
                }
                let friend = self.expect_human_id_for_ai_handle(friend, "shot blocking friend");
                let friend_angle = angles[index];
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
}

impl AiOwnerCtx<'_> {
    pub(in crate::engine) fn execute_live_shoot_decision(&mut self) -> ControlFlow<bool, Decision> {
        if self
            .engine
            .expect_entity(self.owner, "shot ammunition")
            .ai_actor_data()
            .expect("shot actor")
            .number_of_arrows
            == 0
        {
            return ControlFlow::Continue(Decision::RunForNewArrows);
        }
        let Some(target) = self
            .engine
            .propose_live_shot_target(TickCtx::new(self.sim, self.assets), self.owner)
        else {
            return ControlFlow::Continue(Decision::ArcherObserve);
        };
        let ai = self.engine.ai_mut(self.owner, "shot selected target");
        ai.primary_target = Some(target);
        self.engine.execute_ai_focus(self.owner, Some(target));

        let bow_state = self
            .engine
            .expect_entity(self.owner, "shot action")
            .actor_data()
            .expect("shot actor")
            .action_state
            .is_bow();
        if bow_state {
            if self.engine.ai(self.owner, "shot substate").current_substate
                == Substate::AttackingBowAiming
            {
                self.duty_set_state(AiState::Attacking, Substate::AttackingBowShooting);
                let target = self
                    .engine
                    .ai(self.owner, "shot target after state callback")
                    .primary_target
                    .expect("shooting requires target");
                self.stop_ai_owner();
                let target = self
                    .engine
                    .expect_human_id_for_ai_handle(target.get(), "shot target");
                self.engine
                    .shoot_bow_at(TickCtx::new(self.sim, self.assets), self.owner, target);
            } else {
                self.duty_set_state(AiState::Attacking, Substate::AttackingBowAiming);
                let (_, ability) = self
                    .engine
                    .bow_profile_and_ability(self.assets, self.owner)
                    .expect("aiming bow");
                let time = ((110 - i32::from(ability as u16)) / 2) as u32;
                let frame = self.engine.control.frame_counter;
                self.engine
                    .ai_mut(self.owner, "aim timer")
                    .launch_timer(time, frame);
            }
        } else {
            self.stop_ai_owner();
            self.duty_set_state(AiState::Attacking, Substate::AttackingBowLoading);
            let ai = self.engine.enemy_ai_mut(self.owner, "equip bow");
            let command = if ai.enemy_seen_below {
                crate::element::Command::EquipBowDown
            } else {
                crate::element::Command::EquipBow
            };
            self.engine.launch_element(
                TickCtx::new(self.sim, self.assets),
                crate::sequence::SequenceElement::new(1, command, Some(self.owner)),
            );
        }
        ControlFlow::Break(true)
    }
}
