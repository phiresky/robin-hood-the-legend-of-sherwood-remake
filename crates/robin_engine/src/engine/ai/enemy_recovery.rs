//! Recovery and distraction reactions complete against the current actor.
use super::*;
use crate::ai::{AiState, DutyFlags, EmoticonType, EnemyRecovery, Remark, Substate};
use crate::element::{DetectableType, EyeStatus};
use crate::sim_rng::SimulationContext;

impl EngineInner {
    fn recovery_open_eyes(&mut self, owner: EntityId) {
        let radius = self.ai.standard_view_polygon_radius;
        let actor = self
            .world
            .entities
            .expect_ai_actor_data_mut(owner, format_args!("recovery eyes"));
        actor.view_transition = true;
        actor.view_radius = 5;
        actor.view_radius_base = 5;
        actor.view_radius_goal = radius;
        actor.eye_status = EyeStatus::ViewconeGrow;
    }

    fn recovery_blink_enemies(&mut self, owner: EntityId) {
        let actor = self
            .world
            .entities
            .expect_ai_actor_data_mut(owner, format_args!("recovery enemy blinks"));
        let enemies = actor
            .detectable_lists
            .get_mut(DetectableType::Enemy as usize)
            .expect("recovery owner requires enemy detectable list");
        for enemy in enemies {
            enemy.seen_now = false;
            enemy.seen_last_frame = false;
        }
    }

    fn recovery_view_forward(&mut self, owner: EntityId) {
        let actor = self
            .world
            .entities
            .expect_ai_actor_data_mut(owner, format_args!("recovery view status"));
        crate::ai_vision::set_view_status(actor, EyeStatus::LookForward);
    }

    pub(in crate::engine) fn execute_ai_enemy_recovery(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        operation: EnemyRecovery,
    ) {
        match operation {
            EnemyRecovery::FitAgain => {
                if self.observation_ai(owner).base.current_substate != Substate::SleepingUnconscious
                {
                    return;
                }
                let knocked_out = self.observation_ai(owner).base.knocked_out_in_money_fight;
                self.restore_detectable_objects_for_npc(owner, knocked_out);
                self.broadcast_resurrection(owner);
                if self.observation_ai(owner).base.knocked_out_in_money_fight {
                    self.observation_ai_mut(owner)
                        .base
                        .knocked_out_in_money_fight = false;
                    self.execute_ai_return_to_duty(sim, assets, owner, DutyFlags::empty());
                } else {
                    self.duty_set_state(
                        sim,
                        assets,
                        owner,
                        AiState::Sleeping,
                        Substate::SleepingAwakening,
                    );
                    self.observation_timer(
                        owner,
                        crate::parameters_ai::AI_WAKEUP_IDLING_TIME as u32,
                    );
                    self.recovery_view_forward(owner);
                }
            }
            EnemyRecovery::WaspAway | EnemyRecovery::NetAway => {
                if matches!(operation, EnemyRecovery::WaspAway) {
                    if self.observation_ai(owner).base.current_substate
                        != Substate::WonderingWaspInArmour
                    {
                        return;
                    }
                    self.recovery_open_eyes(owner);
                } else {
                    self.recovery_view_forward(owner);
                }
                self.observation_ai_mut(owner)
                    .base
                    .set_emoticon(EmoticonType::QuestionMark);
                self.recovery_blink_enemies(owner);
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Wondering,
                    Substate::WonderingLooking1,
                );
                self.observation_timer(owner, 30);
            }
            EnemyRecovery::Stop => {
                let ai = self.observation_ai(owner);
                if ai.base.current_state == AiState::Sleeping
                    || ai.base.current_state == AiState::Attacking
                        && ai.base.current_substate.is_real_swordfight()
                {
                    return;
                }
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Seeking,
                    Substate::SeekingGotStopEvent,
                );
                self.observation_stop(sim, assets, owner);
                self.observation_ai_mut(owner)
                    .base
                    .set_emoticon(EmoticonType::QuestionMark);
                self.recovery_blink_enemies(owner);
                self.observation_timer(owner, 100);
            }
            EnemyRecovery::Apple { position } => {
                let fighting = !self
                    .expect_entity(owner, "apple owner")
                    .human_data()
                    .expect("apple owner must be human")
                    .opponents
                    .is_empty();
                let interrupt = sim.config().item_gameplay.apple_combat_interrupt;
                if fighting && !interrupt {
                    return;
                }
                self.observation_stop(sim, assets, owner);
                if interrupt
                    && !self
                        .expect_entity(owner, "apple interrupt owner")
                        .human_data()
                        .expect("apple owner must be human")
                        .opponents
                        .is_empty()
                {
                    self.launch_element(crate::sequence::SequenceElement::new(
                        1,
                        crate::element::Command::QuitSwordfight,
                        Some(owner),
                    ));
                }
                self.observation_ai_mut(owner).base.seek_position = position;
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Wondering,
                    Substate::WonderingAppleSauceInTheVisor,
                );
                self.add_weak_stunned(owner);
                self.recovery_open_eyes(owner);
                self.observation_timer(owner, 60);
            }
            EnemyRecovery::Stone { position } => {
                let ai = self.observation_ai_mut(owner);
                if !matches!(
                    ai.base.current_state,
                    AiState::Sleeping | AiState::Default | AiState::Wondering
                ) || i32::from(ai.base.blood_alcohol)
                    > crate::parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT
                    || !ai.has_the_new_task_priority()
                {
                    return;
                }
                ai.current_task_priority = ai.new_task_priority;
                if let Some(object) = ai.base.object_of_desire.take() {
                    ai.base.forgotten_objects.push(object.get());
                }
                self.observation_stop(sim, assets, owner);
                self.observation_ai_mut(owner).base.seek_position = position;
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Wondering,
                    Substate::WonderingAppleReactiontime,
                );
                let remark = if self.observation_ai(owner).is_vip {
                    Remark::VipAppleNo
                } else {
                    Remark::HitByApple
                };
                self.observation_say(sim, assets, owner, remark);
                let position = self.observation_ai(owner).base.seek_position;
                self.duty_face_position_ground(sim, assets, owner, position);
                self.observation_ai_mut(owner)
                    .base
                    .set_emoticon(EmoticonType::QuestionMark);
                self.observation_timer(owner, crate::ai_enemy::combat::APPLE_REACTIONTIME as u32);
            }
        }
    }
}
