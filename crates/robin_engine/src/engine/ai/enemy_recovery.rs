//! Recovery and distraction reactions complete against the current actor.
use super::*;
use crate::ai::{AiState, DutyFlags, EmoticonType, EnemyRecovery, Remark, Substate};
use crate::element::{DetectableType, EyeStatus};
use crate::engine::TickCtx;

impl EngineInner {
    fn recovery_open_eyes(&mut self, owner: EntityId) {
        let radius = self.ai.standard_view_polygon_radius;
        let actor = self.ai_actor_mut(owner, "recovery eyes");
        actor.view_transition = true;
        actor.view_radius = 5;
        actor.view_radius_base = 5;
        actor.view_radius_goal = radius;
        actor.eye_status = EyeStatus::ViewconeGrow;
    }

    fn recovery_blink_enemies(&mut self, owner: EntityId) {
        let actor = self.ai_actor_mut(owner, "recovery enemy blinks");
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
        let actor = self.ai_actor_mut(owner, "recovery view status");
        crate::ai_vision::set_view_status(actor, EyeStatus::LookForward);
    }
}

impl AiOwnerCtx<'_> {
    pub(in crate::engine) fn execute_ai_enemy_recovery(&mut self, operation: EnemyRecovery) {
        match operation {
            EnemyRecovery::FitAgain => {
                if self.engine.observation_ai(self.owner).base.current_substate
                    != Substate::SleepingUnconscious
                {
                    return;
                }
                let knocked_out = self
                    .engine
                    .observation_ai(self.owner)
                    .base
                    .knocked_out_in_money_fight;
                self.engine
                    .restore_detectable_objects_for_npc(self.owner, knocked_out);
                self.engine.broadcast_resurrection(self.owner);
                if self
                    .engine
                    .observation_ai(self.owner)
                    .base
                    .knocked_out_in_money_fight
                {
                    self.engine
                        .observation_ai_mut(self.owner)
                        .base
                        .knocked_out_in_money_fight = false;
                    self.execute_ai_return_to_duty(DutyFlags::empty());
                } else {
                    self.duty_set_state(AiState::Sleeping, Substate::SleepingAwakening);
                    self.engine.observation_timer(
                        self.owner,
                        crate::parameters_ai::AI_WAKEUP_IDLING_TIME as u32,
                    );
                    self.engine.recovery_view_forward(self.owner);
                }
            }
            EnemyRecovery::WaspAway | EnemyRecovery::NetAway => {
                if matches!(operation, EnemyRecovery::WaspAway) {
                    if self.engine.observation_ai(self.owner).base.current_substate
                        != Substate::WonderingWaspInArmour
                    {
                        return;
                    }
                    self.engine.recovery_open_eyes(self.owner);
                } else {
                    self.engine.recovery_view_forward(self.owner);
                }
                self.engine
                    .observation_ai_mut(self.owner)
                    .base
                    .set_emoticon(EmoticonType::QuestionMark);
                self.engine.recovery_blink_enemies(self.owner);
                self.duty_set_state(AiState::Wondering, Substate::WonderingLooking1);
                self.engine.observation_timer(self.owner, 30);
            }
            EnemyRecovery::Stop => {
                let ai = self.engine.observation_ai(self.owner);
                if ai.base.current_state == AiState::Sleeping
                    || ai.base.current_state == AiState::Attacking
                        && ai.base.current_substate.is_real_swordfight()
                {
                    return;
                }
                self.duty_set_state(AiState::Seeking, Substate::SeekingGotStopEvent);
                self.observation_stop();
                self.engine
                    .observation_ai_mut(self.owner)
                    .base
                    .set_emoticon(EmoticonType::QuestionMark);
                self.engine.recovery_blink_enemies(self.owner);
                self.engine.observation_timer(self.owner, 100);
            }
            EnemyRecovery::Apple { position } => {
                let fighting = !self
                    .engine
                    .expect_entity(self.owner, "apple owner")
                    .human_data()
                    .expect("apple owner must be human")
                    .opponents
                    .is_empty();
                let interrupt = self.sim.config().item_gameplay.apple_combat_interrupt;
                if fighting && !interrupt {
                    return;
                }
                self.observation_stop();
                if interrupt
                    && !self
                        .engine
                        .expect_entity(self.owner, "apple interrupt owner")
                        .human_data()
                        .expect("apple owner must be human")
                        .opponents
                        .is_empty()
                {
                    self.engine.launch_element(
                        TickCtx::new(self.sim, self.assets),
                        crate::sequence::SequenceElement::new(
                            1,
                            crate::element::Command::QuitSwordfight,
                            Some(self.owner),
                        ),
                    );
                }
                self.engine
                    .observation_ai_mut(self.owner)
                    .base
                    .seek_position = position;
                self.duty_set_state(AiState::Wondering, Substate::WonderingAppleSauceInTheVisor);
                self.engine.add_weak_stunned(self.owner);
                self.engine.recovery_open_eyes(self.owner);
                self.engine.observation_timer(self.owner, 60);
            }
            EnemyRecovery::Stone { position } => {
                let ai = self.engine.observation_ai_mut(self.owner);
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
                self.observation_stop();
                self.engine
                    .observation_ai_mut(self.owner)
                    .base
                    .seek_position = position;
                self.duty_set_state(AiState::Wondering, Substate::WonderingAppleReactiontime);
                let remark = if self.engine.observation_ai(self.owner).is_vip {
                    Remark::VipAppleNo
                } else {
                    Remark::HitByApple
                };
                self.observation_say(remark);
                let position = self.engine.observation_ai(self.owner).base.seek_position;
                self.duty_face_position_ground(position);
                self.engine
                    .observation_ai_mut(self.owner)
                    .base
                    .set_emoticon(EmoticonType::QuestionMark);
                self.engine.observation_timer(
                    self.owner,
                    crate::ai_enemy::combat::APPLE_REACTIONTIME as u32,
                );
            }
        }
    }
}
