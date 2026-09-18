use super::*;
use crate::ai::{
    AiState, AlertFlags, AlertLevel, EmoticonType, LogLineType, Stimulus, StimulusType, Substate,
};
use crate::element::{EyeStatus, Posture};
use crate::sim_rng::SimulationContext;

#[cfg(test)]
mod tests;

impl EngineInner {
    pub(super) fn admit_ai_think_live(&mut self, owner: EntityId, stimulus: &Stimulus) -> bool {
        let frozen = self.ai.global.freeze;
        let entity = self
            .entities_mut()
            .expect_entity_mut(owner, format_args!("live Think admission"));
        let unconscious = entity.is_unconscious();
        let dead = entity.is_dead();
        let posture = entity.element_data().posture();
        let ai = entity
            .ai_controller_mut()
            .expect("Think admission owner requires AI");
        let event = stimulus.stimulus_type;
        ai.couldnt_reachpoint = false;
        ai.already_on_point = false;
        ai.already_turned = false;
        if frozen {
            return false;
        }
        if ai.script_locked {
            if ai.remember_events
                && !matches!(
                    event,
                    StimulusType::EventDone | StimulusType::EventReachPoint
                )
            {
                ai.stimulus_queue.push(*stimulus);
            }
            ai.register_log_line(LogLineType::EventRefused, 2);
            return false;
        }
        if !ai.locks_flag_field.is_empty() {
            ai.stimulus_queue.push(*stimulus);
            ai.register_log_line(LogLineType::EventRefused, 3);
            return false;
        }
        let refusal = match ai.current_substate {
            Substate::WonderingWaspInArmour
                if !matches!(
                    event,
                    StimulusType::EventLoseConsciousness | StimulusType::EventWaspAway
                ) =>
            {
                Some(4)
            }
            Substate::WonderingUnderNet
                if !matches!(
                    event,
                    StimulusType::EventLoseConsciousness | StimulusType::EventNetAway
                ) =>
            {
                Some(5)
            }
            Substate::FleeingMerryManLeaveMap if event != StimulusType::EventReachPoint => Some(6),
            _ => None,
        };
        if let Some(refusal) = refusal {
            ai.register_log_line(LogLineType::EventRefused, refusal);
            return false;
        }
        if unconscious {
            let refusal = match event {
                StimulusType::EventLoseConsciousness => None,
                StimulusType::EventFitAgain if posture != Posture::Carried => None,
                StimulusType::EventFitAgain => Some(7),
                _ => Some(8),
            };
            if let Some(refusal) = refusal {
                ai.register_log_line(LogLineType::EventRefused, refusal);
                return false;
            }
        }
        ai.standing_around_timer = 0;
        if ai.timer_is_running {
            if ai.current_substate != ai.substate_at_last_timer_launch {
                ai.timer_is_running = false;
            }
        } else if event == StimulusType::EventTimer
            && ai.current_substate != ai.substate_at_last_timer_launch
        {
            ai.register_log_line(LogLineType::EventRefused, 9);
            return false;
        }
        let refusal = if dead {
            Some(10)
        } else if ai.current_substate == Substate::SleepingUnconscious
            && event != StimulusType::EventFitAgain
        {
            Some(11)
        } else if event == StimulusType::EventFitAgain
            && !matches!(
                ai.current_substate,
                Substate::SleepingUnconscious | Substate::SleepingNapping
            )
        {
            Some(12)
        } else {
            None
        };
        if let Some(refusal) = refusal {
            ai.register_log_line(LogLineType::EventRefused, refusal);
            return false;
        }
        true
    }

    pub(in crate::engine) fn begin_ai_special_strike(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        self.observation_ai_mut(owner).pending_special_strike = true;
        self.duty_set_state(
            sim,
            assets,
            owner,
            AiState::Attacking,
            Substate::AttackingSwordfightSpecialStrike,
        );
    }

    pub(in crate::engine) fn reconcile_ai_special_strike(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        has_active: bool,
    ) {
        let ai = self.observation_ai_mut(owner);
        if !ai.pending_special_strike || ai.base.ai_is_locked() {
            return;
        }
        if !matches!(
            ai.base.current_substate,
            Substate::AttackingSwordfight | Substate::AttackingSwordfightSpecialStrike
        ) {
            ai.pending_special_strike = false;
            return;
        }
        if has_active {
            return;
        }
        ai.pending_special_strike = false;
        if ai.base.current_substate == Substate::AttackingSwordfightSpecialStrike {
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Attacking,
                Substate::AttackingSwordfight,
            );
            let frame = self.control.frame_counter;
            let ai = self.observation_ai_mut(owner);
            ai.base.launch_timer(20, frame);
            ai.next_sword_strike_frame = frame + 20;
        }
    }

    pub(in crate::engine) fn begin_enemy_think(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: &Stimulus,
    ) -> bool {
        let frame = self.control.frame_counter;
        let original_creation_order = Some(self.world.original_creation_order(owner));
        let ai = self.observation_ai_mut(owner);
        ai.base.cached_frame = frame;
        ai.base.debug_macro_lifecycle_at(
            frame,
            original_creation_order,
            "think_enter",
            stimulus.stimulus_type,
        );
        if !self.admit_ai_think_live(owner, stimulus) {
            return false;
        }
        let ai = self.observation_ai_mut(owner);
        let (state, substate, eyes, refused) = match stimulus.stimulus_type {
            StimulusType::EventLoseConsciousness => (
                AiState::Sleeping,
                Substate::SleepingUnconscious,
                EyeStatus::DieOrGetUnconscious,
                13,
            ),
            StimulusType::EventWasp => (
                AiState::Wondering,
                Substate::WonderingWaspInArmour,
                EyeStatus::Closed,
                14,
            ),
            StimulusType::EventNet => (
                AiState::Wondering,
                Substate::WonderingUnderNet,
                EyeStatus::Closed,
                15,
            ),
            _ => {
                ai.update_new_task_priority(stimulus);
                return true;
            }
        };
        self.execute_ai_break_macro(owner);

        if stimulus.stimulus_type == StimulusType::EventLoseConsciousness {
            let ai = self.observation_ai_mut(owner);
            ai.base.clear_emoticon();
            if ai.base.current_substate.is_take_money()
                || ai.base.current_substate.is_fight_for_money()
            {
                self.forget_ai_nearby_coins_live(owner);
            }
        } else if stimulus.stimulus_type == StimulusType::EventWasp {
            self.observation_ai_mut(owner)
                .base
                .set_emoticon(EmoticonType::Thunderstorm);
        }

        self.duty_set_state(sim, assets, owner, state, substate);
        let actor = self.ai_actor_mut(owner, "enemy admission eyes");
        crate::ai_vision::set_view_status(actor, eyes);
        if stimulus.stimulus_type == StimulusType::EventLoseConsciousness {
            self.execute_ai_set_alert_status(
                assets,
                owner,
                AlertLevel::Green,
                AlertFlags::INSTANT_MUSIC_CHANGE,
            );
        }
        let ai = self.observation_ai_mut(owner);
        ai.base.sorrow_level = 0;
        ai.attentive = false;
        ai.will_be_attentive = false;
        ai.base
            .register_log_line(LogLineType::EventRefused, refused);
        false
    }
}
